//! Pure adapter for existing config.yaml custom providers.
//!
//! Source: `internal/config/provider_legacy.go` at Go baseline 21d0bc7. Reading
//! existing entries never invokes a command, mutates the configuration, writes
//! files, or applies the stricter new-provider manifest validator.

use super::{CustomProvider, model::legacy_custom_providers, validate_headless_def};
use crate::proto::provider::{
    CURRENT_SCHEMA_VERSION, Definition, Diagnostic, HeadlessDefinition, LaunchDefinition,
    ORIGIN_LEGACY, SEVERITY_WARNING, SourceRef,
};

/// What the running program launches: the same entries the spawn list offers
/// (`effective_custom_providers`), converted by `legacy_provider_definitions`.
///
/// The Go adapter alone lets the first entry of a repeated id win even when its
/// `command` cannot be split, which makes the id unlaunchable; the spawn list
/// and `wrap` skip such an entry and use the next usable one, so the registry
/// has to be built from the same list or the three would disagree about which
/// entry an id means. The Go behaviour stays in `legacy_provider_definitions`
/// (fixture-pinned); this is the one place the program chooses.
pub fn provider_definitions_for_app(raw: &[CustomProvider]) -> (Vec<Definition>, Vec<Diagnostic>) {
    let usable = super::model::effective_custom_providers(raw);
    let (definitions, mut diagnostics) = legacy_provider_definitions(&usable);
    // Entries left out are still reported, unless another entry with their id
    // is the one in use (the report would then say the id does not work).
    let (_, historical) = legacy_provider_definitions(raw);
    for diagnostic in historical {
        let id = diagnostic.field.split('.').next().unwrap_or_default();
        let in_use = usable.iter().any(|provider| provider.id == id);
        let known = diagnostics
            .iter()
            .any(|seen| seen.code == diagnostic.code && seen.field == diagnostic.field);
        if !in_use && !known {
            diagnostics.push(diagnostic);
        }
    }
    (definitions, diagnostics)
}

/// Convert effective historical entries, preserving input and diagnostic order.
/// Empty diagnostics use Rust's empty Vec; callers exposing a required Go slice
/// must encode that no-diagnostic result as null, as they do for other nil slices.
pub fn legacy_provider_definitions(raw: &[CustomProvider]) -> (Vec<Definition>, Vec<Diagnostic>) {
    let effective = legacy_custom_providers(raw);
    let mut definitions = Vec::with_capacity(effective.len());
    let mut diagnostics = Vec::new();
    for legacy in effective {
        let argv = match legacy.argv() {
            Ok(argv) if !argv.is_empty() => argv,
            result => {
                let mut message = "legacy custom provider command cannot be split".to_owned();
                if let Err(error) = result {
                    message.push_str(": ");
                    message.push_str(&error.to_string());
                }
                diagnostics.push(Diagnostic {
                    code: "legacy_command_invalid".into(),
                    severity: SEVERITY_WARNING.into(),
                    field: format!("{}.command", legacy.id),
                    message,
                });
                continue;
            }
        };
        let mut launch = LaunchDefinition {
            executable: argv[0].clone(),
            args: argv[1..].to_vec(),
            ..Default::default()
        };
        if let Some(headless) = legacy.headless.as_ref() {
            if validate_headless_def(headless).is_err() {
                diagnostics.push(Diagnostic {
                    code: "legacy_headless_invalid".into(),
                    severity: SEVERITY_WARNING.into(),
                    field: format!("{}.headless", legacy.id),
                    message: "invalid headless definition was ignored".into(),
                });
            } else {
                launch.headless = Some(HeadlessDefinition {
                    args: headless.args.clone(),
                    format: headless.format.trim().to_owned(),
                    prompt_via: headless.prompt_via.trim().to_owned(),
                });
            }
        }
        definitions.push(Definition {
            schema_version: CURRENT_SCHEMA_VERSION,
            id: legacy.id.clone(),
            display_name: legacy.effective_label().to_owned(),
            approval_pattern_source: legacy.approval_pattern_source,
            source: SourceRef {
                origin: ORIGIN_LEGACY.into(),
                ..Default::default()
            },
            launch: Some(launch),
            ..Default::default()
        });
    }
    (definitions, diagnostics)
}
