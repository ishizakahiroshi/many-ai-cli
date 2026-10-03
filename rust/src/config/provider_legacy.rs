//! Pure adapter for existing config.yaml custom providers.
//!
//! Source: `internal/config/provider_legacy.go` at Go baseline 21d0bc7. Reading
//! existing entries never invokes a command, mutates the configuration, writes
//! files, or applies the stricter new-provider manifest validator.

use super::{CustomProvider, effective_custom_providers, validate_headless_def};
use crate::proto::provider::{
    CURRENT_SCHEMA_VERSION, Definition, Diagnostic, HeadlessDefinition, LaunchDefinition,
    ORIGIN_LEGACY, SEVERITY_WARNING, SourceRef,
};

/// Convert effective historical entries, preserving input and diagnostic order.
/// Empty diagnostics use Rust's empty Vec; callers exposing a required Go slice
/// must encode that no-diagnostic result as null, as they do for other nil slices.
pub fn legacy_provider_definitions(raw: &[CustomProvider]) -> (Vec<Definition>, Vec<Diagnostic>) {
    let effective = effective_custom_providers(raw);
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
