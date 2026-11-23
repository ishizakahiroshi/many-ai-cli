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
use std::sync::{Arc, OnceLock, RwLock};

/// Where the running process reads the current provider registry from. The Hub
/// and the wrapper install one at startup; until then (and in unit tests) the
/// built-in declarations alone answer.
pub type RegistrySource =
    Arc<dyn Fn() -> Option<Arc<crate::profile::registry::Registry>> + Send + Sync>;
static SOURCE: RwLock<Option<RegistrySource>> = RwLock::new(None);

pub fn install_registry_source(source: RegistrySource) {
    *SOURCE.write().unwrap_or_else(|e| e.into_inner()) = Some(source);
}

#[cfg(test)]
thread_local! {
    static TEST_REGISTRY: std::cell::RefCell<Option<Arc<crate::profile::registry::Registry>>> =
        const { std::cell::RefCell::new(None) };
}

/// Test only: lookups on this thread see the installed registry until the guard
/// drops (the process-wide source is shared by parallel tests, so it is not used).
#[cfg(test)]
pub struct TestRegistryGuard;
#[cfg(test)]
impl Drop for TestRegistryGuard {
    fn drop(&mut self) {
        TEST_REGISTRY.with(|r| *r.borrow_mut() = None);
    }
}
#[cfg(test)]
pub fn install_test_registry(registry: crate::profile::registry::Registry) -> TestRegistryGuard {
    TEST_REGISTRY.with(|r| *r.borrow_mut() = Some(Arc::new(registry)));
    TestRegistryGuard
}

/// Why an automated path may not use a provider reached over ACP.
pub const ACP_HUMAN_ONLY: &str = "this AI is reached over ACP and is driven by a person only; an automated path (child, relay, routine, board, automatic approval or retry, usage polling, another session) cannot use it";

/// Whether `id` is declared `connection: acp` in the running registry.
pub fn is_acp(id: &str) -> bool {
    with_declared(id, |d| !automation_allowed(d)).unwrap_or(false)
}

/// `Some(reason)` when an automated path must refuse `id`.
pub fn automation_refusal(id: &str) -> Option<&'static str> {
    is_acp(id).then_some(ACP_HUMAN_ONLY)
}

fn current_registry() -> Option<Arc<crate::profile::registry::Registry>> {
    #[cfg(test)]
    if let Some(r) = TEST_REGISTRY.with(|r| r.borrow().clone()) {
        return Some(r);
    }
    let source = SOURCE.read().unwrap_or_else(|e| e.into_inner()).clone()?;
    source()
}

/// The seven built-ins as the registry would resolve them (manifest plus
/// declaration), for a process that has no registry installed yet.
fn builtin_cached() -> &'static BTreeMap<String, Definition> {
    static CACHE: OnceLock<BTreeMap<String, Definition>> = OnceLock::new();
    CACHE.get_or_init(|| {
        let declared = builtin_declarations();
        crate::profile::registry::embedded_definitions()
            .unwrap_or_default()
            .iter()
            .map(|manifest| {
                (
                    manifest.id.clone(),
                    resolve(manifest, declared.get(&manifest.id)),
                )
            })
            .collect()
    })
}

/// The resolved declarations of a built-in provider, whatever the running
/// registry holds. For code that must give the same answer in every process.
pub fn builtin(id: &str) -> Option<Definition> {
    builtin_cached().get(id).cloned()
}

/// The declarations that apply to a launch of `provider`: its own definition
/// (when one is at hand) with the built-in declaration filling what it leaves
/// out, or the built-in declaration alone.
pub fn resolved_for(provider: &str, definition: Option<&Definition>) -> Option<Definition> {
    let builtin = builtin(provider);
    match definition {
        Some(def) => Some(resolve(def, builtin.as_ref())),
        None => builtin,
    }
}

/// Launch arguments for a permission mode, from `permissions.modes`: the entry
/// of that mode, else the `*` entry (with `{mode}` replaced). The second value
/// is the mode that was applied (empty when no argument was added).
pub fn permission_args(def: Option<&Definition>, mode: &str) -> (Vec<String>, String) {
    let modes = def.and_then(|d| d.permissions.as_ref()).map(|p| &p.modes);
    let values: Vec<String> = match modes {
        Some(modes) if !mode.is_empty() => modes
            .get(mode)
            .or_else(|| modes.get("*"))
            .map(|args| args.iter().map(|a| a.replace("{mode}", mode)).collect())
            .unwrap_or_default(),
        _ => vec![],
    };
    let applied = if values.is_empty() {
        String::new()
    } else {
        mode.into()
    };
    (values, applied)
}

/// Launch arguments for an allowed tool list, from
/// `permissions.allowed_tools_args` (see [`PermissionsDefinition`]).
pub fn allowed_tool_args(def: Option<&Definition>, values: &[String]) -> Vec<String> {
    let Some(template) = def
        .and_then(|d| d.permissions.as_ref())
        .map(|p| &p.allowed_tools_args)
        .filter(|t| !t.is_empty())
    else {
        return vec![];
    };
    if values.is_empty() {
        return vec![];
    }
    let mut out = vec![];
    for entry in template {
        if entry == "{tools}" {
            out.extend(values.iter().cloned());
        } else if entry.contains("{tool}") {
            out.extend(values.iter().map(|v| entry.replace("{tool}", v)));
        } else {
            out.push(entry.clone());
        }
    }
    out
}

/// The environment variable that moves a built-in provider's config directory.
/// Subscription profiles and folder trust stay limited to the built-ins, so this
/// never reads a registered provider's declaration.
pub fn builtin_config_dir_env(provider: &str) -> Option<String> {
    builtin(provider)
        .and_then(|d| d.launch)
        .map(|l| l.config_dir_env)
        .filter(|e| !e.is_empty())
}

/// The slash command source a built-in provider ships with.
pub fn builtin_slash_source(id: &str) -> String {
    builtin(id)
        .and_then(|d| d.slash_commands)
        .map(|s| s.source)
        .unwrap_or_default()
}

/// The resolved declarations of `id` from the running registry, or (before a
/// registry is installed, or when it does not know `id`) from the built-in
/// declarations. `None` for an id nobody declares.
pub fn declared(id: &str) -> Option<Definition> {
    if let Some(def) = current_registry().and_then(|r| r.declared(id)) {
        return Some(def);
    }
    builtin_cached().get(id).cloned()
}

/// Reads the declarations of `id` without copying them (a hot path such as a
/// keystroke). `None` for an id nobody declares.
pub fn with_declared<R>(id: &str, read: impl FnOnce(&Definition) -> R) -> Option<R> {
    if let Some(registry) = current_registry()
        && let Some(def) = registry.declared_ref(id)
    {
        return Some(read(def));
    }
    builtin_cached().get(id).map(read)
}

/// Whether the CLI drops a quickly pasted body, so the Enter waits longer
/// (`input.slow_paste`).
pub fn slow_paste(id: &str) -> bool {
    with_declared(id, |d| d.input.as_ref().is_some_and(|i| i.slow_paste)).unwrap_or(false)
}

/// The project-root file the Hub keeps its marker rules in while a session of
/// `id` runs (`instructions.rules_file`). `None` when the provider declares
/// none, when the name is not a plain Markdown file name, or when the provider
/// is reached over ACP (its sessions have no terminal that shows a marker).
pub fn project_rules_file(id: &str) -> Option<String> {
    with_declared(id, |d| {
        let name = d.instructions.as_ref()?.rules_file.as_str();
        (automation_allowed(d) && crate::profile::validation::rules_file_name(name))
            .then(|| name.to_owned())
    })
    .flatten()
}

/// Every provider the running registry knows, built-ins first; the seven
/// built-ins alone when no registry is installed.
pub fn declared_all() -> Vec<Definition> {
    if let Some(registry) = current_registry() {
        let list = registry.declared_list();
        if !list.is_empty() {
            return list;
        }
    }
    BUILTIN_PROVIDER_IDS
        .iter()
        .filter_map(|id| builtin_cached().get(*id).cloned())
        .collect()
}

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

/// `launch.permission_args` (a registered AI CLI's approval-mode map, shipped
/// in 0.11.1) is the older spelling of `permissions.modes`. Fold it in, so
/// every reader sees one permission declaration: a mode it maps becomes a
/// `permissions.modes` entry (an entry the definition already writes wins),
/// and without `permissions.choices` the picker offers `default` and the mapped
/// modes in their usual order.
fn fold_launch_permission_args(out: &mut Definition) {
    let Some(map) = out
        .launch
        .as_ref()
        .map(|launch| launch.permission_args.clone())
        .filter(|map| !map.is_empty())
    else {
        return;
    };
    let permissions = out.permissions.get_or_insert_with(Default::default);
    for (mode, args) in map {
        permissions.modes.entry(mode).or_insert(args);
    }
    if permissions.choices.is_empty() {
        permissions.choices = std::iter::once("default")
            .chain(
                crate::profile::validation::PERMISSION_ARG_MODES
                    .iter()
                    .copied()
                    .filter(|mode| permissions.modes.contains_key(*mode)),
            )
            .map(str::to_owned)
            .collect();
    }
}

/// `capabilities.slash_suggestion_popup` (a CLI whose "/" popup takes the first
/// Enter) asks for the same submit as `input.slash_submit: paste`; fold it in so
/// the screen reads one declaration. A written `slash_submit` wins.
fn fold_slash_suggestion_popup(out: &mut Definition) {
    if !out
        .capabilities
        .get("slash_suggestion_popup")
        .copied()
        .unwrap_or(false)
    {
        return;
    }
    let input = out.input.get_or_insert_with(Default::default);
    if input.slash_submit.is_empty() {
        input.slash_submit = "paste".into();
    }
}

fn pick<T: Clone>(own: &Option<T>, fallback: &Option<T>) -> Option<T> {
    own.clone().or_else(|| fallback.clone())
}

/// `def` with every declarative section that it leaves empty filled from
/// `builtin`. The result is what the rest of the Hub reads; `def` itself (and
/// therefore the registry revision) is never changed.
pub fn resolve(def: &Definition, builtin: Option<&Definition>) -> Definition {
    let mut out = def.clone();
    fold_launch_permission_args(&mut out);
    fold_slash_suggestion_popup(&mut out);
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
    out.transcript = pick(&def.transcript, &b.transcript);
    out.approval = pick(&def.approval, &b.approval);
    out.instructions = pick(&def.instructions, &b.instructions);
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
            if models.change_pattern.is_empty() {
                models.change_pattern = bm.change_pattern.clone();
                models.change_effort_suffix = bm.change_effort_suffix;
            }
        }
        (None, Some(bm))
            if bm.list.is_some() || bm.context_limit != 0 || !bm.change_pattern.is_empty() =>
        {
            out.models = Some(ModelsDefinition {
                list: bm.list.clone(),
                context_limit: bm.context_limit,
                change_pattern: bm.change_pattern.clone(),
                change_effort_suffix: bm.change_effort_suffix,
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
            if p.handoff_order == 0 {
                p.handoff_order = bp.handoff_order;
            }
            if p.label.is_empty() {
                p.label = bp.label.clone();
            }
            if p.long_label.is_empty() {
                p.long_label = bp.long_label.clone();
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

/// The providers a session can be handed to: enabled PTY providers with a
/// `launch` definition, ordered by `presentation.handoff_order` (0 = after the
/// others, registry order kept among equals).
pub fn handoff_candidates_of(defs: &[Definition]) -> Vec<String> {
    let mut list: Vec<(usize, &Definition)> = defs
        .iter()
        .enumerate()
        .filter(|(_, d)| d.enabled.unwrap_or(true) && d.launch.is_some() && automation_allowed(d))
        .collect();
    let rank = |d: &Definition| {
        d.presentation
            .as_ref()
            .map(|p| p.handoff_order)
            .filter(|o| *o > 0)
            .unwrap_or(i64::MAX)
    };
    list.sort_by_key(|(index, d)| (rank(d), *index));
    list.into_iter().map(|(_, d)| d.id.clone()).collect()
}

pub fn handoff_candidates() -> Vec<String> {
    handoff_candidates_of(&declared_all())
}

/// Whether a routine (scheduled run) may use this provider: an enabled PTY
/// provider that has a `launch` definition and does not opt out.
pub fn routines_allowed(def: &Definition) -> bool {
    automation_allowed(def)
        && def.enabled.unwrap_or(true)
        && def.launch.is_some()
        && def
            .routines
            .as_ref()
            .and_then(|r| r.enabled)
            .unwrap_or(true)
}

/// How a routine run is judged complete: `marker` (default) or `native_or_marker`.
pub fn completion_mode(def: &Definition) -> &str {
    match def.routines.as_ref().map(|r| r.completion_mode.as_str()) {
        Some(mode) if !mode.is_empty() => mode,
        _ => "marker",
    }
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
    fn a_transcript_declaration_is_checked_before_a_definition_is_accepted() {
        let errors = |transcript: &str| -> Vec<String> {
            let raw = format!(
                r#"{{"schema_version":1,"id":"synthetic-tool","display_name":"Synthetic","launch":{{"executable":"synthetic"}},"transcript":{transcript}}}"#
            );
            let (_, diagnostics) =
                validate_definition(raw.as_bytes(), &default_adapters()).unwrap();
            diagnostics
                .iter()
                .filter(|d| d.severity.as_str() == "error")
                .map(|d| d.field.clone())
                .collect()
        };
        let good = r#"{"root":"{env:SYNTH_APPDATA}/t","find":["index"],"index":{"file":"{env:SYNTH_APPDATA}/s.db","table":"sessions","id_column":"id","cwd_column":"cwd","time_column":"at"},"reader":{"format":"json","messages":"steps","role_key":"source","text_key":"message","roles":{"user":"user"}}}"#;
        assert!(errors(good).is_empty(), "{:?}", errors(good));
        assert!(!errors(r#"{"root":"/absolute/t","find":["session_id"]}"#).is_empty());
        assert!(!errors(r#"{"root":"{home}/t"}"#).is_empty());
        assert!(!errors(r#"{"root":"{home}/t","find":["guess"]}"#).is_empty());
        assert!(!errors(r#"{"root":"{home}/t","find":["index"],"index":{"file":"{home}/s","table":"a b","id_column":"id","cwd_column":"c","time_column":"t"}}"#).is_empty());
        assert!(!errors(r#"{"root":"{home}/t","find":["session_id"],"reader":{"format":"json","messages":"m","role_key":"r","text_key":"t","roles":{"x":"system"}}}"#).is_empty());
        assert!(!errors(r#"{"file":"{home}/a.db","reader":{"format":"jsonl","role_key":"r","text_key":"t","roles":{"u":"user"}}}"#).is_empty());
    }

    #[test]
    fn an_approval_declaration_is_checked_before_a_definition_is_accepted() {
        let errors = |approval: &str| -> Vec<String> {
            let raw = format!(
                r#"{{"schema_version":1,"id":"synthetic-tool","display_name":"Synthetic","launch":{{"executable":"synthetic"}},"approval":{approval}}}"#
            );
            let (_, diagnostics) =
                validate_definition(raw.as_bytes(), &default_adapters()).unwrap();
            diagnostics
                .iter()
                .filter(|d| d.severity.as_str() == "error")
                .map(|d| d.field.clone())
                .collect()
        };
        let good = r#"{"line_extras":["key_lines"],"keys":[{"keys":["y"],"num":1},{"keys":["esc"],"num":2,"send":"\u001b"}],"hint_phrases":["needs approval"],"shortcut_kind":true}"#;
        assert!(errors(good).is_empty(), "{:?}", errors(good));
        assert!(!errors(r#"{"style":"wizard"}"#).is_empty());
        assert!(!errors(r#"{"style":"buttons"}"#).is_empty());
        assert!(!errors(r#"{"line_extras":["regex"]}"#).is_empty());
        assert!(!errors(r#"{"keys":[{"keys":[],"num":1}]}"#).is_empty());
        assert!(!errors(r#"{"keys":[{"keys":["(y)"],"num":1}]}"#).is_empty());
        assert!(!errors(r#"{"keys":[{"keys":["y"],"num":99}]}"#).is_empty());
        assert!(!errors(r#"{"buttons":[{"line_all":[],"labels":["a"]}]}"#).is_empty());
        assert!(!errors(r#"{"suppress_screens":[{}]}"#).is_empty());
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
                "transcript",
                "approval",
                "instructions",
            ] {
                assert!(!object.contains_key(key), "{}: {key}", manifest.id);
            }
        }
    }

    /// The older spellings a registered definition may still carry (0.11.1's
    /// `launch.permission_args` and `capabilities.slash_suggestion_popup`) read
    /// as the same declarations as `permissions` and `input.slash_submit`.
    #[test]
    fn older_spellings_fold_into_the_declarations() {
        let raw = br#"{"schema_version":1,"id":"demo","display_name":"Demo","launch":{"executable":"demo","permission_args":{"bypassPermissions":["--yolo"],"acceptEdits":["--edits"]}},"capabilities":{"slash_suggestion_popup":true}}"#;
        let (def, diagnostics) = validate_definition(raw, &default_adapters()).unwrap();
        assert!(
            diagnostics.iter().all(|d| d.severity.as_str() != "error"),
            "{:?}",
            diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
        let resolved = resolve(&def, None);
        let permissions = resolved.permissions.clone().unwrap();
        assert_eq!(
            permissions.choices,
            ["default", "acceptEdits", "bypassPermissions"]
        );
        assert_eq!(
            permission_args(Some(&resolved), "bypassPermissions"),
            (vec!["--yolo".to_owned()], "bypassPermissions".to_owned())
        );
        assert!(permission_args(Some(&resolved), "plan").0.is_empty());
        assert_eq!(resolved.input.unwrap().slash_submit, "paste");
        // A written declaration wins over the older spelling.
        let raw = br#"{"schema_version":1,"id":"demo","display_name":"Demo","launch":{"executable":"demo","permission_args":{"auto":["--old"]}},"permissions":{"modes":{"auto":["--new"]},"choices":["default","auto"]},"input":{"slash_submit":"once"},"capabilities":{"slash_suggestion_popup":true}}"#;
        let (def, _) = validate_definition(raw, &default_adapters()).unwrap();
        let resolved = resolve(&def, None);
        assert_eq!(
            permission_args(Some(&resolved), "auto").0,
            ["--new".to_owned()]
        );
        assert_eq!(resolved.input.unwrap().slash_submit, "once");
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
            r#","instructions":{"rules_file":"../AGENTS.md"}"#,
            r#","instructions":{"rules_file":"docs/AGENTS.md"}"#,
            r#","instructions":{"rules_file":"C:AGENTS.md"}"#,
            r#","instructions":{"rules_file":"AGENTS.txt"}"#,
            r#","instructions":{"rules_file":".md"}"#,
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
            r#","connection":"acp","links":{"usage_url":"https://example.com/usage"},"slash_commands":{"items":[{"cmd":"/help","desc":"Help"}]},"input":{"interrupt_key":"ctrl-c"},"permissions":{"modes":{"auto":["--auto"]},"allowed_tools_args":["--allow","{tools}"]},"routines":{"enabled":false},"instructions":{"rules_file":"AGENTS.md"}}"#
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

    /// The sample ACP definition users add with `provider add --definition`.
    /// It goes through the same validation as a user-supplied file, is an ACP
    /// provider, and is never reachable from an automated path.
    #[test]
    fn the_acp_sample_definition_validates_and_is_person_only() {
        let raw = include_str!("../../../docs/samples/acp-agent-sample.json");
        let (def, diagnostics) = validate_definition(raw.as_bytes(), &default_adapters()).unwrap();
        assert!(
            diagnostics.iter().all(|d| d.severity.as_str() != "error"),
            "{:?}",
            diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
        assert_eq!(def.connection, "acp");
        let launch = def.launch.as_ref().expect("launch");
        assert_eq!(launch.executable, "agy");
        assert_eq!(launch.args, vec!["acp".to_string()]);
        assert!(
            launch.headless.is_none(),
            "an ACP sample has no headless mode"
        );
        assert!(!automation_allowed(&def));
        assert!(!BUILTIN_PROVIDER_IDS.contains(&def.id.as_str()));
    }

    /// The values of the per-provider tables that C2 removed from the code,
    /// copied here as they were.
    #[test]
    fn declared_values_equal_the_tables_the_code_used_to_hard_code() {
        // routine/validation.rs: only codex used `native_or_marker`.
        for id in BUILTIN_PROVIDER_IDS {
            let def = builtin(id).unwrap();
            assert!(routines_allowed(&def), "{id} must allow routines");
            let expected = if *id == "codex" {
                "native_or_marker"
            } else {
                "marker"
            };
            assert_eq!(completion_mode(&def), expected, "{id}");
        }
        // application/instruction_rules.rs: these four got the rules block in
        // the project's AGENTS.md; claude and codex keep their own files in
        // their config directory, and command-code got nothing.
        for id in BUILTIN_PROVIDER_IDS {
            let expected = matches!(*id, "copilot" | "cursor-agent" | "grok" | "opencode")
                .then(|| "AGENTS.md".to_string());
            assert_eq!(project_rules_file(id), expected, "{id}");
        }
        // slash_commands settings: one raw.githubusercontent URL per provider.
        for id in BUILTIN_PROVIDER_IDS {
            assert_eq!(
                builtin_slash_source(id),
                format!(
                    "https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/slash-commands/{id}.md"
                )
            );
        }
        assert_eq!(
            crate::config::SlashCmdSources::defaults().claude,
            builtin_slash_source("claude")
        );
        // slash_commands/skills.rs: codex skills start with `$`, the others `/`.
        for id in BUILTIN_PROVIDER_IDS {
            let prefix = builtin(id)
                .and_then(|d| d.slash_commands)
                .map(|s| s.skill_prefix)
                .unwrap_or_default();
            assert_eq!(prefix, if *id == "codex" { "$" } else { "" }, "{id}");
        }
        // hub/handoff_routes.rs: the order of the candidate list.
        assert_eq!(
            handoff_candidates_of(&declared_all()),
            [
                "claude",
                "codex",
                "grok",
                "copilot",
                "cursor-agent",
                "opencode",
                "command-code"
            ]
        );
    }

    #[test]
    fn a_registered_provider_is_routable_through_the_same_functions() {
        let mut def = Definition {
            id: "synthetic-cli".into(),
            launch: Some(LaunchDefinition::default()),
            ..Default::default()
        };
        assert!(routines_allowed(&def));
        assert_eq!(completion_mode(&def), "marker");
        def.routines = Some(RoutinesDefinition {
            enabled: None,
            completion_mode: "native_or_marker".into(),
        });
        assert_eq!(completion_mode(&def), "native_or_marker");
        let mut off = def.clone();
        off.routines = Some(RoutinesDefinition {
            enabled: Some(false),
            completion_mode: String::new(),
        });
        assert!(!routines_allowed(&off));
        let mut acp = def.clone();
        acp.connection = "acp".into();
        assert!(!routines_allowed(&acp));
        let mut disabled = def.clone();
        disabled.enabled = Some(false);
        assert!(!routines_allowed(&disabled));
        assert!(!routines_allowed(&Definition::default()), "no launch");
        // The handoff list: built-ins in their order, then the registered one;
        // an ACP provider is never offered.
        let mut all = declared_all();
        all.push(def);
        all.push(acp);
        let list = handoff_candidates_of(&all);
        assert_eq!(list.len(), 8);
        assert_eq!(list.last().unwrap(), "synthetic-cli");
    }

    /// The permission table of `wrapper/launch.rs` before C3, as it was.
    fn old_permission_args(provider: &str, mode: &str) -> (Vec<String>, String) {
        let strings = |v: &[&str]| -> Vec<String> { v.iter().map(|s| (*s).into()).collect() };
        let values = match (provider, mode) {
            ("claude" | "grok", "" | "default" | "bounded") => vec![],
            ("claude" | "grok", mode) => vec!["--permission-mode".into(), mode.into()],
            ("copilot", "bypassPermissions") => strings(&["--allow-all"]),
            ("copilot", "bounded") => strings(&["--no-ask-user"]),
            ("copilot", "auto") => strings(&["--autopilot"]),
            ("cursor-agent", "bypassPermissions") => strings(&["--force"]),
            ("cursor-agent", "auto") => strings(&["--auto-review"]),
            ("opencode", "bounded" | "bypassPermissions") => strings(&["--auto"]),
            ("command-code", "plan") => strings(&["--permission-mode", "plan"]),
            ("command-code", "acceptEdits" | "auto") => strings(&["--auto-accept"]),
            ("command-code", "bypassPermissions") => strings(&["--yolo"]),
            _ => vec![],
        };
        let declared = if values.is_empty() {
            String::new()
        } else {
            mode.into()
        };
        (values, declared)
    }

    #[test]
    fn launch_declarations_equal_the_tables_the_code_used_to_hard_code() {
        let modes = [
            "",
            "default",
            "bounded",
            "auto",
            "plan",
            "acceptEdits",
            "bypassPermissions",
            "dontAsk",
            "weird",
        ];
        for provider in BUILTIN_PROVIDER_IDS
            .iter()
            .chain(["shell", "synthetic"].iter())
        {
            for mode in modes {
                assert_eq!(
                    permission_args(builtin(provider).as_ref(), mode),
                    old_permission_args(provider, mode),
                    "{provider} {mode}"
                );
            }
        }
        // Allowed tools: claude `--allowedTools a b`, copilot `--allow-tool=a` each.
        let tools = vec!["Read".to_owned(), "Bash(git log *)".to_owned()];
        assert_eq!(
            allowed_tool_args(builtin("claude").as_ref(), &tools),
            ["--allowedTools", "Read", "Bash(git log *)"]
        );
        assert_eq!(
            allowed_tool_args(builtin("copilot").as_ref(), &tools),
            ["--allow-tool=Read", "--allow-tool=Bash(git log *)"]
        );
        for provider in ["codex", "grok", "opencode", "cursor-agent", "command-code"] {
            assert!(allowed_tool_args(builtin(provider).as_ref(), &tools).is_empty());
        }
        assert!(allowed_tool_args(builtin("claude").as_ref(), &[]).is_empty());
        // Login arguments and the config directory variable.
        let login = |id: &str| builtin(id).and_then(|d| d.login).map(|l| l.args);
        assert_eq!(login("claude"), Some(vec!["auth".into(), "login".into()]));
        assert_eq!(login("codex"), Some(vec!["login".into()]));
        assert_eq!(login("grok"), Some(vec!["login".into()]));
        assert_eq!(
            login("opencode"),
            Some(vec!["providers".into(), "login".into()])
        );
        for id in ["copilot", "cursor-agent", "command-code"] {
            assert_eq!(login(id), None, "{id}");
        }
        for (id, env) in [
            ("claude", Some("CLAUDE_CONFIG_DIR")),
            ("codex", Some("CODEX_HOME")),
            ("opencode", Some("XDG_DATA_HOME")),
            ("grok", Some("GROK_HOME")),
            ("copilot", None),
            ("cursor-agent", None),
            ("command-code", None),
        ] {
            assert_eq!(builtin_config_dir_env(id).as_deref(), env, "{id}");
        }
        // Model list commands (and their timeouts) of the native catalog.
        let list = |id: &str| builtin(id).and_then(|d| d.models).and_then(|m| m.list);
        let cursor = list("cursor-agent").unwrap();
        assert_eq!(
            (cursor.args, cursor.timeout_seconds),
            (vec!["--list-models".to_owned()], 15)
        );
        let grok = list("grok").unwrap();
        assert_eq!(
            (grok.args, grok.timeout_seconds),
            (vec!["models".to_owned()], 15)
        );
        let opencode = list("opencode").unwrap();
        assert_eq!(opencode.args, ["models", "opencode", "--verbose"]);
        assert_eq!(
            (opencode.timeout_seconds, opencode.refresh_args),
            (30, vec!["--refresh".to_owned()])
        );
        for id in ["claude", "codex", "copilot", "command-code"] {
            assert!(list(id).is_none(), "{id}");
        }
        // Input: only codex and opencode need the slow paste.
        for id in BUILTIN_PROVIDER_IDS {
            assert_eq!(slow_paste(id), matches!(*id, "codex" | "opencode"), "{id}");
        }
        assert!(!slow_paste("synthetic"));
        // web `shouldBypassCursorHideFilterForProvider`: grok and codex.
        // web `stopKeyForActiveSession`: grok is interrupted with Ctrl+C.
        for id in BUILTIN_PROVIDER_IDS {
            let input = builtin(id).and_then(|d| d.input).unwrap_or_default();
            assert_eq!(
                input.full_frame_cursor,
                matches!(*id, "grok" | "codex"),
                "{id}"
            );
            let expected = if *id == "grok" { "ctrl-c" } else { "" };
            assert_eq!(input.interrupt_key, expected, "{id}");
            // web `shouldSkipClearPrefix`: codex (and the shell, which is not a provider).
            assert_eq!(input.no_line_clear, *id == "codex", "{id}");
        }
        // Ready screens and the model change line.
        let marker = |id: &str| {
            builtin(id)
                .and_then(|d| d.screen)
                .map(|s| s.ready_markers)
                .unwrap_or_default()
        };
        assert_eq!(marker("claude"), ["shift+tab to cycle"]);
        assert_eq!(marker("codex"), ["Ask Codex to do anything"]);
        assert!(marker("grok").is_empty());
        let change = |id: &str| {
            builtin(id)
                .and_then(|d| d.models)
                .map(|m| (m.change_pattern, m.change_effort_suffix))
                .filter(|(pattern, _)| !pattern.is_empty())
        };
        assert_eq!(
            change("claude"),
            Some((r"Set model to ([^\r\n]+)".to_owned(), true))
        );
        assert_eq!(
            change("codex"),
            Some((r"Model changed to ([^\r\n]+)".to_owned(), false))
        );
        assert_eq!(change("opencode"), None);
        // Attachments: claude `@path`, codex `@path` with slashes, the rest bare.
        let attach = |id: &str| {
            builtin(id)
                .and_then(|d| d.input)
                .map(|i| i.attach_format)
                .unwrap_or_default()
        };
        assert_eq!(attach("claude"), "at-path");
        assert_eq!(attach("codex"), "at-path-slash");
        for id in [
            "copilot",
            "cursor-agent",
            "opencode",
            "grok",
            "command-code",
        ] {
            assert_eq!(attach(id), "", "{id}");
        }
        // Display names come from the manifests (the launch used a fixed table).
        for (id, name) in [
            ("claude", "Claude"),
            ("codex", "Codex"),
            ("copilot", "GitHub Copilot"),
            ("cursor-agent", "Cursor Agent"),
            ("opencode", "OpenCode"),
            ("grok", "Grok Build"),
            ("command-code", "Command Code"),
        ] {
            assert_eq!(builtin(id).unwrap().display_name, name, "{id}");
        }
    }

    #[test]
    fn a_registered_provider_gets_its_launch_arguments_from_its_own_declaration() {
        let mut def = Definition {
            id: "synthetic-cli".into(),
            launch: Some(LaunchDefinition::default()),
            ..Default::default()
        };
        def.permissions = Some(PermissionsDefinition {
            modes: BTreeMap::from([
                ("auto".to_owned(), vec!["--yolo".to_owned()]),
                ("*".to_owned(), vec!["--mode={mode}".to_owned()]),
            ]),
            allowed_tools_args: vec!["--allow".into(), "{tools}".into(), "--each={tool}".into()],
            choices: vec![],
        });
        assert_eq!(
            permission_args(Some(&def), "auto"),
            (vec!["--yolo".to_owned()], "auto".to_owned())
        );
        assert_eq!(
            permission_args(Some(&def), "plan"),
            (vec!["--mode=plan".to_owned()], "plan".to_owned())
        );
        assert_eq!(permission_args(Some(&def), ""), (vec![], String::new()));
        assert_eq!(
            allowed_tool_args(Some(&def), &["a".into(), "b".into()]),
            ["--allow", "a", "b", "--each=a", "--each=b"]
        );
        // Nothing declared: no argument is invented.
        assert_eq!(permission_args(None, "auto"), (vec![], String::new()));
        assert_eq!(
            permission_args(Some(&Definition::default()), "auto"),
            (vec![], String::new())
        );
        // A launch resolves the definition's own section; an id with no built-in
        // declaration and no definition resolves to nothing.
        let resolved = resolved_for("synthetic-cli", Some(&def)).unwrap();
        assert!(resolved.permissions.is_some());
        assert!(resolved_for("synthetic-cli", None).is_none());
        assert!(builtin_config_dir_env("synthetic-cli").is_none());
    }

    /// The lists the screen used to hard-code (`web/src`), copied as they were.
    #[test]
    fn presentation_and_permission_choices_equal_what_the_screen_used_to_hard_code() {
        // spawn-approval-memory.ts PERMISSION_MODE_VALUES
        let choices = |id: &str| {
            builtin(id)
                .and_then(|d| d.permissions)
                .map(|p| p.choices)
                .unwrap_or_default()
        };
        let full = [
            "default",
            "plan",
            "acceptEdits",
            "auto",
            "bypassPermissions",
        ];
        assert_eq!(choices("claude"), full);
        assert_eq!(choices("grok"), full);
        assert_eq!(choices("copilot"), ["default", "auto", "bypassPermissions"]);
        assert_eq!(
            choices("cursor-agent"),
            ["default", "auto", "bypassPermissions"]
        );
        assert_eq!(
            choices("command-code"),
            ["default", "plan", "acceptEdits", "bypassPermissions"]
        );
        assert!(choices("codex").is_empty() && choices("opencode").is_empty());
        // provider-icon.ts letters (opencode `O` in every screen but one) and
        // session-list.ts order (ollama and lm-studio are screen-only entries at
        // 2 and 3); orchestration-roles.ts / chat-history.ts names.
        let rows = [
            ("claude", "C", 1, "Claude", "Claude Code"),
            ("codex", "X", 4, "Codex", "Codex CLI"),
            ("copilot", "P", 5, "Copilot", "GitHub Copilot"),
            ("opencode", "O", 6, "OpenCode", "OpenCode"),
            ("cursor-agent", "r", 7, "Cursor Agent", "Cursor Agent"),
            ("grok", "G", 8, "Grok Build", "Grok Build"),
            ("command-code", "M", 9, "Command Code", "Command Code"),
        ];
        for (id, letter, order, label, long_label) in rows {
            let p = builtin(id).unwrap().presentation.unwrap();
            assert_eq!(
                (
                    p.letter.as_str(),
                    p.order,
                    p.label.as_str(),
                    p.long_label.as_str()
                ),
                (letter, order, label, long_label),
                "{id}"
            );
        }
    }
}
