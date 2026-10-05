//! New-input provider validation. Historical definitions are retained with
//! diagnostics; registry reads never erase them by applying these diagnostics.
use super::registry::{diag, sort_diagnostics};
use crate::proto::provider::*;
use std::{collections::BTreeSet, sync::OnceLock};

pub const DEFINITION_LIMIT: usize = 256 * 1024;
fn control(s: &str) -> bool {
    s.chars().any(|c| c < '\u{20}' || c == '\u{7f}')
}
pub fn valid_id(id: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err("id is required".into());
    }
    let mut bytes = id.bytes();
    if id.len() > 64
        || !bytes
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        || !bytes.all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'.' | b'_' | b'-')
        })
    {
        return Err("id must be a lowercase slug of at most 64 characters".into());
    }
    if id == "shell" {
        return Err("shell is a reserved launch identity".into());
    }
    Ok(())
}
fn text(value: &str, field: &str, required: bool) -> Result<(), String> {
    if required && value.trim().is_empty() {
        return Err(format!("{field} is required"));
    }
    if value.len() > 512 || control(value) {
        return Err(format!(
            "{field} is too long or contains a control character"
        ));
    }
    Ok(())
}
fn argument(value: &str, field: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 1024 || control(value) {
        return Err(format!(
            "{field} is empty, too long, or contains a control character"
        ));
    }
    if value.contains(['&', '|', '<', '>', '^', '%', ';', '`'])
        || value.contains("$(")
        || value.contains("${")
    {
        return Err(format!("{field} contains shell expansion syntax"));
    }
    Ok(())
}
fn args(field: &str, values: &[String], placeholders: bool) -> Vec<Diagnostic> {
    if values.len() > 256 {
        return vec![diag(
            "too_many_args",
            "error",
            field,
            "too many argv entries",
        )];
    }
    let mut out = vec![];
    for (i, value) in values.iter().enumerate() {
        let field = format!("{field}[{i}]");
        if let Err(e) = argument(value, &field) {
            out.push(diag("invalid_arg", "error", &field, &e));
            continue;
        }
        let mut value = value.clone();
        if placeholders {
            for token in ["{model}", "{effort}", "{session_id}", "{prompt}"] {
                value = value.replace(token, "");
            }
        }
        if value.contains(['{', '}']) {
            out.push(diag(
                "unknown_placeholder",
                "error",
                &field,
                "argv contains an unknown placeholder",
            ));
        }
    }
    out
}
fn launch(launch: &LaunchDefinition) -> Vec<Diagnostic> {
    let mut out = vec![];
    let candidates: Vec<_> = std::iter::once(&launch.executable)
        .filter(|s| !s.is_empty())
        .chain(launch.executable_candidates.iter())
        .collect();
    if candidates.is_empty() {
        out.push(diag(
            "missing_executable",
            "error",
            "launch.executable",
            "launch.executable or launch.executable_candidates is required",
        ));
    }
    if candidates.len() > 256 {
        out.push(diag(
            "too_many_executables",
            "error",
            "launch.executable_candidates",
            "too many executable candidates",
        ));
    }
    for (i, value) in candidates.iter().enumerate() {
        let field = format!("launch.executable[{i}]");
        if let Err(e) = argument(value, &field) {
            out.push(diag("invalid_executable", "error", &field, &e));
        }
    }
    out.extend(args("launch.args", &launch.args, false));
    out.extend(args("launch.model_args", &launch.model_args, true));
    out.extend(args("launch.effort_args", &launch.effort_args, true));
    if launch.effort_levels.len() > 256 {
        out.push(diag(
            "too_many_effort_levels",
            "error",
            "launch.effort_levels",
            "too many effort levels",
        ));
    }
    for (i, value) in launch.effort_levels.iter().enumerate() {
        let field = format!("launch.effort_levels[{i}]");
        if let Err(e) = argument(value, &field) {
            out.push(diag("invalid_effort_level", "error", &field, &e));
        }
    }
    if launch.allowed_env.len() > 256 {
        out.push(diag(
            "too_many_env_names",
            "error",
            "launch.allowed_env",
            "too many environment variable names",
        ));
    }
    for (i, value) in launch.allowed_env.iter().enumerate() {
        let mut bytes = value.bytes();
        if !bytes
            .next()
            .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
            || !bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            out.push(diag(
                "invalid_env_name",
                "error",
                &format!("launch.allowed_env[{i}]"),
                "environment entries must be variable names, not values",
            ));
        }
    }
    if let Some(h) = &launch.headless {
        if h.format.is_empty() {
            out.push(diag(
                "missing_headless_format",
                "error",
                "launch.headless.format",
                "headless format is required",
            ));
        }
        if !matches!(h.prompt_via.as_str(), "" | "stdin" | "arg") {
            out.push(diag(
                "invalid_headless_prompt",
                "error",
                "launch.headless.prompt_via",
                "prompt_via must be stdin or arg",
            ));
        }
        out.extend(args("launch.headless.args", &h.args, false));
    }
    out
}
fn update(update: &UpdateDefinition) -> Vec<Diagnostic> {
    let mut out = vec![];
    if update.enabled == Some(true) && update.args.is_empty() {
        out.push(diag(
            "update_enabled_without_args",
            "error",
            "update.enabled",
            "update.enabled is true but update.args is empty",
        ));
    }
    out.extend(args("update.args", &update.args, false));
    out.extend(args("update.version_args", &update.version_args, false));
    if !update.executable.is_empty()
        && let Err(e) = argument(&update.executable, "update.executable")
    {
        out.push(diag(
            "invalid_update_executable",
            "error",
            "update.executable",
            &e,
        ));
    }
    if !(0..=3600).contains(&update.timeout_seconds) {
        out.push(diag(
            "invalid_update_timeout",
            "error",
            "update.timeout_seconds",
            "update.timeout_seconds must be between 0 and 3600",
        ));
    }
    out
}
fn models(models: &ModelsDefinition) -> Vec<Diagnostic> {
    let mut out = vec![];
    if models.items.len() > 256 {
        out.push(diag(
            "too_many_models",
            "error",
            "models.items",
            "too many model entries",
        ));
    }
    let mut seen = BTreeSet::new();
    for (i, m) in models.items.iter().enumerate() {
        let field = format!("models.items[{i}]");
        if let Err(e) = valid_id(&m.id) {
            out.push(diag(
                "invalid_model_id",
                "error",
                &format!("{field}.id"),
                &e,
            ));
        }
        if !seen.insert(&m.id) {
            out.push(diag(
                "duplicate_model_id",
                "error",
                &format!("{field}.id"),
                "model id is duplicated",
            ));
        }
        if let Err(e) = text(&m.label, &format!("{field}.label"), true) {
            out.push(diag(
                "invalid_model_label",
                "error",
                &format!("{field}.label"),
                &e,
            ));
        }
    }
    out
}
fn valid_color(color: &str) -> bool {
    color.is_empty()
        || (color.len() == 7
            && color.starts_with('#')
            && color.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit))
}
fn icon_count(value: &str) -> usize {
    static MARK: OnceLock<regex::Regex> = OnceLock::new();
    let mark = MARK.get_or_init(|| regex::Regex::new(r"\A\p{M}\z").unwrap());
    let (mut count, mut joined, mut regional) = (0, false, 0);
    for c in value.chars() {
        if c == '\u{200d}' {
            joined = true;
        } else if mark.is_match(c.encode_utf8(&mut [0; 4]))
            || ('\u{1f3fb}'..='\u{1f3ff}').contains(&c)
            || ('\u{e0020}'..='\u{e007f}').contains(&c)
        {
        } else if joined {
            joined = false;
        } else if ('\u{1f1e6}'..='\u{1f1ff}').contains(&c) {
            regional += 1;
            if regional % 2 == 1 {
                count += 1;
            }
        } else {
            regional = 0;
            count += 1;
        }
    }
    count
}
fn valid_icon(value: &str) -> bool {
    value.is_empty()
        || (value.len() <= 512
            && value.trim() == value
            && !value.chars().any(char::is_control)
            && (1..=2).contains(&icon_count(value)))
}
pub fn sanitize_presentation(p: Option<&PresentationDefinition>) -> Option<PresentationDefinition> {
    let p = p?;
    let icon_text = if valid_icon(&p.icon_text) {
        p.icon_text.clone()
    } else {
        String::new()
    };
    let color = if valid_color(&p.color) {
        p.color.clone()
    } else {
        String::new()
    };
    if icon_text.is_empty() && color.is_empty() {
        None
    } else {
        Some(PresentationDefinition { icon_text, color })
    }
}

pub fn validate_definition(
    raw: &[u8],
    adapters: &AdapterCatalog,
) -> Result<(Definition, Vec<Diagnostic>), String> {
    if raw.len() > DEFINITION_LIMIT {
        return Err(format!(
            "provider definition exceeds {DEFINITION_LIMIT} bytes"
        ));
    }
    let object = crate::proto::wire::decode_go_json_object(raw)
        .map_err(|e| format!("decode provider definition: {e}"))?;
    let def = crate::proto::decode_wire::<Definition>(raw)
        .map_err(|e| format!("decode provider definition fields: {e}"))?;
    let mut out = vec![];
    if let Some(object) = &object {
        for field in object.keys() {
            if ![
                "schema_version",
                "id",
                "display_name",
                "description",
                "enabled",
                "launch",
                "models",
                "capabilities",
                "adapters",
                "presentation",
                "approval_pattern_source",
                "source",
                "update",
            ]
            .contains(&field.as_str())
            {
                out.push(diag(
                    "unknown_field",
                    "warning",
                    field,
                    "unknown field is ignored",
                ));
            }
        }
    }
    if def.schema_version != CURRENT_SCHEMA_VERSION {
        out.push(diag(
            "unsupported_schema_version",
            "error",
            "schema_version",
            "schema version must be 1",
        ));
    }
    if let Err(e) = valid_id(&def.id) {
        out.push(diag("invalid_id", "error", "id", &e));
    }
    if let Err(e) = text(&def.display_name, "display_name", true) {
        out.push(diag("invalid_display_name", "error", "display_name", &e));
    }
    if def.description.len() > 512 || control(&def.description) {
        out.push(diag(
            "invalid_description",
            "error",
            "description",
            "description is too long or contains a control character",
        ));
    }
    if def.approval_pattern_source.len() > 512 || control(&def.approval_pattern_source) {
        out.push(diag(
            "invalid_approval_pattern_source",
            "error",
            "approval_pattern_source",
            "approval pattern source is too long or contains a control character",
        ));
    }
    if let Some(v) = &def.launch {
        out.extend(launch(v));
    } else {
        out.push(diag(
            "missing_launch",
            "error",
            "launch",
            "launch is required",
        ));
    }
    if let Some(v) = &def.models {
        out.extend(models(v));
    }
    for (field, key) in [
        ("launch", &def.adapters.launch),
        ("approval", &def.adapters.approval),
        ("transcript", &def.adapters.transcript),
        ("usage", &def.adapters.usage),
        ("subscription", &def.adapters.subscription),
        ("permissions", &def.adapters.permissions),
        ("subagents", &def.adapters.subagents),
    ] {
        if !matches!(key.as_str(), "" | "none" | "unsupported")
            && !adapters
                .keys
                .as_ref()
                .is_some_and(|keys| keys.contains_key(key))
        {
            out.push(diag(
                "unknown_adapter",
                "error",
                &format!("adapters.{field}"),
                "adapter key is not registered",
            ));
        }
    }
    for key in def.capabilities.keys() {
        if ![
            "models",
            "effort",
            "headless",
            "approval",
            "transcript",
            "usage",
            "subscription",
            "permissions",
            "subagents",
        ]
        .contains(&key.as_str())
        {
            out.push(diag(
                "unknown_capability",
                "warning",
                &format!("capabilities.{key}"),
                "unknown capability is ignored",
            ));
        }
    }
    if let Some(p) = &def.presentation {
        if let Err(e) = text(&p.icon_text, "presentation.icon_text", false) {
            out.push(diag(
                "invalid_presentation",
                "error",
                "presentation.icon_text",
                &e,
            ));
        } else if !valid_icon(&p.icon_text) {
            out.push(diag("invalid_presentation","error","presentation.icon_text","presentation.icon_text must be 1 to 2 visible characters without surrounding whitespace"));
        }
        if !valid_color(&p.color) {
            out.push(diag(
                "invalid_presentation",
                "error",
                "presentation.color",
                "presentation.color must be a #RRGGBB hex color",
            ));
        }
    }
    if let Some(v) = &def.update {
        out.extend(update(v));
    }
    sort_diagnostics(&mut out);
    Ok((def, out))
}

/// Fixed Go spawnValidModelLabel, shared by child and ordinary launch paths.
/// Leading dashes are rejected separately where the value could become a flag.
pub fn valid_spawn_model_label(value: &str) -> bool {
    !value.chars().any(|c| {
        let n = c as u32;
        n < 0x20
            || n == 0x7f
            || [32, 34, 39, 124, 38, 62, 60, 94, 37, 40, 41, 59, 96, 36].contains(&n)
    })
}
