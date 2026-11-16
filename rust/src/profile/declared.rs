//! Declarative values of the seven built-in providers and the rule that merges
//! them with a provider's own definition.
//!
//! The embedded manifests (`assets/provider/manifests`) are fixed by the Go
//! oracle: their canonical hash feeds the registry revision, so they must not
//! grow. The values that the code used to hard-code per provider (usage page,
//! login arguments, config directory variable, ...) therefore live in separate
//! files (`assets/provider/declarations`) that have the same shape as a
//! definition but carry only the declarative sections. A user definition wins
//! field by field; a section it does not write falls back to the built-in one.
use crate::proto::provider::*;
use std::collections::BTreeMap;

/// One JSON object: provider id -> a partial definition that carries only the
/// declarative sections.
const BUILTIN_DECLARATIONS: &[u8] = include_bytes!("../../assets/provider/declarations.json");

/// The declarations of the built-in providers, keyed by provider id. An entry
/// that does not parse is skipped (the unit test below makes that a failure at
/// test time, never at run time).
pub fn builtin_declarations() -> BTreeMap<String, Definition> {
    let Ok(serde_json::Value::Object(entries)) =
        serde_json::from_slice::<serde_json::Value>(BUILTIN_DECLARATIONS)
    else {
        return BTreeMap::new();
    };
    entries
        .into_iter()
        .filter_map(|(id, value)| {
            let bytes = serde_json::to_vec(&value).ok()?;
            crate::proto::decode_wire::<Definition>(&bytes)
                .ok()
                .map(|def| (id, def))
        })
        .collect()
}

fn pick<T: Clone>(own: &Option<T>, fallback: &Option<T>) -> Option<T> {
    own.clone().or_else(|| fallback.clone())
}

/// `def` with every declarative section that it leaves empty filled from
/// `builtin`. The result is what the rest of the Hub reads; `def` itself (and
/// therefore the registry revision) is never changed.
pub fn resolve(def: &Definition, builtin: Option<&Definition>) -> Definition {
    let mut out = def.clone();
    let Some(b) = builtin else {
        return out;
    };
    if out.connection.is_empty() {
        out.connection = b.connection.clone();
    }
    out.slash_commands = pick(&def.slash_commands, &b.slash_commands);
    out.permissions = pick(&def.permissions, &b.permissions);
    out.login = pick(&def.login, &b.login);
    out.links = pick(&def.links, &b.links);
    out.input = pick(&def.input, &b.input);
    out.routines = pick(&def.routines, &b.routines);
    out.screen = pick(&def.screen, &b.screen);
    if let (Some(launch), Some(bl)) = (out.launch.as_mut(), b.launch.as_ref())
        && launch.config_dir_env.is_empty()
    {
        launch.config_dir_env = bl.config_dir_env.clone();
    }
    match (out.models.as_mut(), b.models.as_ref()) {
        (Some(models), Some(bm)) => {
            if models.list.is_none() {
                models.list = bm.list.clone();
            }
            if models.context_limit == 0 {
                models.context_limit = bm.context_limit;
            }
        }
        (None, Some(bm)) if bm.list.is_some() || bm.context_limit != 0 => {
            out.models = Some(ModelsDefinition {
                list: bm.list.clone(),
                context_limit: bm.context_limit,
                ..Default::default()
            });
        }
        _ => {}
    }
    match (out.presentation.as_mut(), b.presentation.as_ref()) {
        (Some(p), Some(bp)) => {
            if p.icon_text.is_empty() {
                p.icon_text = bp.icon_text.clone();
            }
            if p.color.is_empty() {
                p.color = bp.color.clone();
            }
            if p.letter.is_empty() {
                p.letter = bp.letter.clone();
            }
            if p.order == 0 {
                p.order = bp.order;
            }
        }
        (None, Some(bp)) => out.presentation = Some(bp.clone()),
        _ => {}
    }
    out
}

/// Whether a provider may be driven by an automated path (a child, a relay, a
/// routine, a board, an automatic approval or retry). Only a PTY provider may.
pub fn automation_allowed(def: &Definition) -> bool {
    !matches!(def.connection.as_str(), "acp")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::registry::{default_adapters, embedded_definitions};
    use crate::profile::validation::validate_definition;

    #[test]
    fn every_builtin_has_a_declaration_file_that_parses_and_validates() {
        let declared = builtin_declarations();
        for id in BUILTIN_PROVIDER_IDS {
            let def = declared
                .get(*id)
                .unwrap_or_else(|| panic!("declaration file of {id} missing or unparsable"));
            assert_eq!(def.id, *id);
        }
        assert_eq!(declared.len(), BUILTIN_PROVIDER_IDS.len());
    }

    #[test]
    fn a_declaration_merged_into_its_manifest_has_no_validation_error() {
        let declared = builtin_declarations();
        for manifest in embedded_definitions().unwrap() {
            let merged = resolve(&manifest, declared.get(&manifest.id));
            let raw = serde_json::to_vec(&merged).unwrap();
            let (_, diagnostics) = validate_definition(&raw, &default_adapters()).unwrap();
            let errors: Vec<_> = diagnostics
                .iter()
                .filter(|d| d.severity.as_str() == "error")
                .map(|d| format!("{}: {} {}", merged.id, d.field, d.message))
                .collect();
            assert!(errors.is_empty(), "{errors:?}");
        }
    }

    #[test]
    fn declarations_never_touch_the_fixed_manifest_fields() {
        // The oracle revision hashes the manifests; a declaration must only add
        // the declarative sections and must not redefine the launch contract.
        for (id, def) in builtin_declarations() {
            assert!(
                def.launch.as_ref().is_none_or(|l| {
                    l.executable.is_empty()
                        && l.args.is_empty()
                        && l.model_args.is_empty()
                        && l.headless.is_none()
                }),
                "{id}: launch contract belongs to the manifest"
            );
            assert!(
                def.adapters == AdapterRefs::default(),
                "{id}: adapters belong to the manifest"
            );
        }
    }

    #[test]
    fn a_provider_own_section_wins_and_an_empty_one_falls_back() {
        let mut builtin = Definition {
            id: "x".into(),
            ..Default::default()
        };
        builtin.links = Some(LinksDefinition {
            usage_url: "https://example.com/builtin".into(),
        });
        builtin.login = Some(LoginDefinition {
            args: vec!["login".into()],
        });
        let mut own = Definition {
            id: "x".into(),
            ..Default::default()
        };
        own.links = Some(LinksDefinition {
            usage_url: "https://example.com/own".into(),
        });
        let merged = resolve(&own, Some(&builtin));
        assert_eq!(merged.links.unwrap().usage_url, "https://example.com/own");
        assert_eq!(merged.login.unwrap().args, vec!["login".to_owned()]);
        // The merge never edits the input, so the registry revision is stable.
        assert!(own.login.is_none());
    }

    #[test]
    fn only_a_pty_provider_may_be_automated() {
        let mut def = Definition::default();
        assert!(automation_allowed(&def));
        def.connection = "pty".into();
        assert!(automation_allowed(&def));
        def.connection = "acp".into();
        assert!(!automation_allowed(&def));
    }

    #[test]
    fn a_definition_without_declarations_serializes_as_before() {
        // No new key may appear for a definition that does not use them, or the
        // Go oracle hash of every existing definition would change.
        for manifest in embedded_definitions().unwrap() {
            let value = serde_json::to_value(&manifest).unwrap();
            let object = value.as_object().unwrap();
            for key in [
                "connection",
                "slash_commands",
                "permissions",
                "login",
                "links",
                "input",
                "routines",
                "screen",
            ] {
                assert!(!object.contains_key(key), "{}: {key}", manifest.id);
            }
        }
    }

    #[test]
    fn bad_declarations_are_refused_for_new_input() {
        let adapters = default_adapters();
        let base = r#"{"schema_version":1,"id":"demo","display_name":"Demo","launch":{"executable":"demo"}"#;
        let bad = [
            r#","connection":"web""#,
            r#","links":{"usage_url":"http://example.com"}"#,
            r#","slash_commands":{"items":[{"cmd":"help"}]}"#,
            r#","input":{"interrupt_key":"f5"}"#,
            r#","permissions":{"modes":{"auto":["--a;b"]}}"#,
            r#","screen":{"ready_markers":[""]}"#,
            r#","routines":{"completion_mode":"x"}"#,
        ];
        for fragment in bad {
            let raw = format!("{base}{fragment}}}");
            let (_, diagnostics) = validate_definition(raw.as_bytes(), &adapters).unwrap();
            assert!(
                diagnostics.iter().any(|d| d.severity.as_str() == "error"),
                "accepted: {fragment}"
            );
        }
        let good = format!(
            "{base}{}",
            r#","connection":"acp","links":{"usage_url":"https://example.com/usage"},"slash_commands":{"items":[{"cmd":"/help","desc":"Help"}]},"input":{"interrupt_key":"ctrl-c"},"permissions":{"modes":{"auto":["--auto"]},"allowed_tools_args":["--allow","{tools}"]},"routines":{"enabled":false}}"#
        );
        let (def, diagnostics) = validate_definition(good.as_bytes(), &adapters).unwrap();
        assert!(
            diagnostics.iter().all(|d| d.severity.as_str() != "error"),
            "{:?}",
            diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
        assert_eq!(def.connection, "acp");
        assert!(!automation_allowed(&def));
    }
}
