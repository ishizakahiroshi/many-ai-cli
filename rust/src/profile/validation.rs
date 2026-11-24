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
/// Whether a provider name a wrapper reports to the Hub (when it registers or
/// reattaches) has the shape the registration check allows: what `valid_id`
/// accepts, or the shell launch identity a plain shell session uses. The Hub
/// refuses any other name before it reaches log file names, history or the
/// screen.
pub fn valid_session_provider(name: &str) -> bool {
    name == "shell" || valid_id(name).is_ok()
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
/// Approval modes a registered provider may map to its own flags.
pub const PERMISSION_ARG_MODES: [&str; 4] = ["plan", "acceptEdits", "auto", "bypassPermissions"];
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
    for (mode, values) in &launch.permission_args {
        let field = format!("launch.permission_args.{mode}");
        if !PERMISSION_ARG_MODES.contains(&mode.as_str()) {
            out.push(diag(
                "invalid_permission_mode",
                "error",
                &field,
                "permission_args keys must be plan, acceptEdits, auto or bypassPermissions",
            ));
            continue;
        }
        out.extend(args(&field, values, false));
    }
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
/// `screen` markers are compared with the screen after whitespace is removed,
/// so a marker that is only whitespace would match every screen.
fn screen(screen: &ScreenDefinition) -> Vec<Diagnostic> {
    let mut out = vec![];
    if screen.composer_prefix.trim() != screen.composer_prefix
        || screen.composer_prefix.chars().count() > 8
        || control(&screen.composer_prefix)
    {
        out.push(diag(
            "invalid_screen",
            "error",
            "screen.composer_prefix",
            "screen.composer_prefix must be at most 8 characters without surrounding whitespace or control characters",
        ));
    }
    for (field, markers) in [
        ("screen.ready_markers", &screen.ready_markers),
        ("screen.startup_blockers", &screen.startup_blockers),
        ("screen.composer_end_markers", &screen.composer_end_markers),
    ] {
        if markers.len() > 32 {
            out.push(diag("invalid_screen", "error", field, "too many markers"));
            continue;
        }
        for (i, marker) in markers.iter().enumerate() {
            if marker.trim().is_empty() || marker.len() > 128 || control(marker) {
                out.push(diag(
                    "invalid_screen",
                    "error",
                    &format!("{field}[{i}]"),
                    "a screen marker is empty, too long, or contains a control character",
                ));
            }
        }
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
    let letter = if valid_icon(&p.letter) {
        p.letter.clone()
    } else {
        String::new()
    };
    let order = p.order.max(0);
    let handoff_order = p.handoff_order.max(0);
    let label = if valid_label(&p.label) {
        p.label.clone()
    } else {
        String::new()
    };
    let long_label = if valid_label(&p.long_label) {
        p.long_label.clone()
    } else {
        String::new()
    };
    if icon_text.is_empty()
        && color.is_empty()
        && letter.is_empty()
        && order == 0
        && handoff_order == 0
        && label.is_empty()
        && long_label.is_empty()
    {
        None
    } else {
        Some(PresentationDefinition {
            icon_text,
            color,
            letter,
            order,
            handoff_order,
            label,
            long_label,
        })
    }
}

/// A name shown on screen: short, printable, no surrounding whitespace.
fn valid_label(value: &str) -> bool {
    !value.is_empty()
        && value.chars().count() <= 40
        && value.trim() == value
        && !value.chars().any(char::is_control)
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
            "agents_md_instructions",
            "slash_suggestion_popup",
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
    if let Some(v) = &def.screen {
        out.extend(screen(v));
    }
    out.extend(declared_sections(&def));
    sort_diagnostics(&mut out);
    Ok((def, out))
}

fn https_url(value: &str) -> bool {
    value.len() <= 512
        && !control(value)
        && value
            .strip_prefix("https://")
            .is_some_and(|rest| !rest.is_empty() && !rest.starts_with('/') && !rest.contains(' '))
}
fn one_of(value: &str, field: &str, allowed: &[&str], out: &mut Vec<Diagnostic>) {
    if !value.is_empty() && !allowed.contains(&value) {
        out.push(diag(
            "invalid_declaration",
            "error",
            field,
            &format!("{field} must be one of: {}", allowed.join(", ")),
        ));
    }
}
fn env_name(value: &str) -> bool {
    let mut chars = value.chars();
    value.len() <= 64
        && chars
            .next()
            .is_some_and(|c| c.is_ascii_uppercase() || c == '_')
        && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// A file the Hub may write its rules block to: one plain Markdown file name,
/// never a path. The Hub joins it to the project root, so a separator, a drive
/// or `..` would let a definition point the write somewhere else.
pub fn rules_file_name(value: &str) -> bool {
    let Some(stem) = value
        .len()
        .checked_sub(3)
        .filter(|at| value.is_char_boundary(*at) && value[*at..].eq_ignore_ascii_case(".md"))
        .map(|at| &value[..at])
    else {
        return false;
    };
    value.len() <= 64
        && !stem.is_empty()
        && stem.chars().any(|c| c.is_ascii_alphanumeric())
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        && !value.contains("..")
}

/// The Rust-only declarative sections (connection, slash commands, permissions,
/// login, links, input, routines and the extra launch / models /
/// presentation fields; `screen` has its own check above). Every problem is an error: a definition with a bad
/// declaration is refused for new input instead of being half applied.
fn declared_sections(def: &Definition) -> Vec<Diagnostic> {
    let mut out = vec![];
    one_of(&def.connection, "connection", &["pty", "acp"], &mut out);
    if let Some(s) = &def.slash_commands {
        if !s.source.is_empty() && !https_url(&s.source) {
            out.push(diag(
                "invalid_declaration",
                "error",
                "slash_commands.source",
                "slash_commands.source must be an https URL",
            ));
        }
        if s.items.len() > 2000 {
            out.push(diag(
                "invalid_declaration",
                "error",
                "slash_commands.items",
                "too many slash command items",
            ));
        }
        for (i, item) in s.items.iter().enumerate() {
            let field = format!("slash_commands.items[{i}]");
            let first = item.cmd.chars().next();
            if !matches!(first, Some('/') | Some('$'))
                || item.cmd.len() > 64
                || item.cmd.contains(char::is_whitespace)
                || control(&item.cmd)
                || item.desc.len() > 256
                || control(&item.desc)
            {
                out.push(diag(
                    "invalid_declaration",
                    "error",
                    &field,
                    "a slash command starts with / or $, has no whitespace, and has a short description",
                ));
            }
        }
        if !s.skill_prefix.is_empty() && !matches!(s.skill_prefix.as_str(), "/" | "$") {
            out.push(diag(
                "invalid_declaration",
                "error",
                "slash_commands.skill_prefix",
                "slash_commands.skill_prefix must be / or $",
            ));
        }
    }
    if let Some(p) = &def.permissions {
        if p.modes.len() > 32 {
            out.push(diag(
                "invalid_declaration",
                "error",
                "permissions.modes",
                "too many permission modes",
            ));
        }
        for (name, values) in &p.modes {
            let field = format!("permissions.modes.{name}");
            if name.is_empty() || name.len() > 64 || control(name) {
                out.push(diag(
                    "invalid_declaration",
                    "error",
                    &field,
                    "permission mode name is empty, too long, or contains a control character",
                ));
            }
            let values: Vec<String> = values.iter().map(|v| v.replace("{mode}", "x")).collect();
            out.extend(args(&field, &values, false));
        }
        if p.choices.len() > 32
            || p.choices
                .iter()
                .any(|c| c.is_empty() || c.len() > 64 || control(c) || c == "*")
        {
            out.push(diag(
                "invalid_declaration",
                "error",
                "permissions.choices",
                "permissions.choices holds at most 32 mode names without control characters",
            ));
        }
        let tools: Vec<String> = p
            .allowed_tools_args
            .iter()
            .map(|v| v.replace("{tools}", "x").replace("{tool}", "x"))
            .collect();
        out.extend(args("permissions.allowed_tools_args", &tools, false));
    }
    if let Some(l) = &def.login {
        out.extend(args("login.args", &l.args, false));
    }
    if let Some(l) = &def.links
        && !l.usage_url.is_empty()
        && !https_url(&l.usage_url)
    {
        out.push(diag(
            "invalid_declaration",
            "error",
            "links.usage_url",
            "links.usage_url must be an https URL",
        ));
    }
    if let Some(i) = &def.input {
        one_of(
            &i.interrupt_key,
            "input.interrupt_key",
            &["esc", "ctrl-c"],
            &mut out,
        );
        one_of(
            &i.attach_format,
            "input.attach_format",
            &["path", "at-path", "at-path-slash"],
            &mut out,
        );
        one_of(
            &i.slash_submit,
            "input.slash_submit",
            &["once", "paste"],
            &mut out,
        );
    }
    if let Some(r) = &def.routines {
        one_of(
            &r.completion_mode,
            "routines.completion_mode",
            &["marker", "native_or_marker"],
            &mut out,
        );
    }
    if let Some(i) = &def.instructions
        && !i.rules_file.is_empty()
        && !rules_file_name(&i.rules_file)
    {
        out.push(diag(
            "invalid_declaration",
            "error",
            "instructions.rules_file",
            "instructions.rules_file must be the name of a Markdown file in the project root (letters, digits, '.', '-' and '_', ending in .md)",
        ));
    }
    if let Some(a) = &def.approval {
        for (field, message) in approval_problems(a) {
            out.push(diag("invalid_declaration", "error", &field, &message));
        }
    }
    if let Some(t) = &def.transcript {
        for (field, message) in transcript_problems(t) {
            out.push(diag("invalid_declaration", "error", &field, &message));
        }
    }
    if let Some(l) = &def.launch
        && !l.config_dir_env.is_empty()
        && !env_name(&l.config_dir_env)
    {
        out.push(diag(
            "invalid_declaration",
            "error",
            "launch.config_dir_env",
            "launch.config_dir_env must be an upper case environment variable name",
        ));
    }
    if let Some(m) = &def.models {
        if m.context_limit < 0 {
            out.push(diag(
                "invalid_declaration",
                "error",
                "models.context_limit",
                "models.context_limit must not be negative",
            ));
        }
        if !m.change_pattern.is_empty() {
            let one_group = m.change_pattern.len() <= 200
                && regex::Regex::new(&m.change_pattern).is_ok_and(|re| re.captures_len() == 2);
            if !one_group {
                out.push(diag(
                    "invalid_declaration",
                    "error",
                    "models.change_pattern",
                    "models.change_pattern must be a regular expression with exactly one capture group",
                ));
            }
        }
        if let Some(list) = &m.list {
            out.extend(args("models.list.args", &list.args, false));
            out.extend(args("models.list.refresh_args", &list.refresh_args, false));
            if !(0..=600).contains(&list.timeout_seconds) {
                out.push(diag(
                    "invalid_declaration",
                    "error",
                    "models.list.timeout_seconds",
                    "models.list.timeout_seconds must be between 0 and 600",
                ));
            }
        }
    }
    if let Some(p) = &def.presentation {
        if !p.letter.is_empty() && !valid_icon(&p.letter) {
            out.push(diag(
                "invalid_presentation",
                "error",
                "presentation.letter",
                "presentation.letter must be 1 to 2 visible characters without surrounding whitespace",
            ));
        }
        for (field, value) in [("label", &p.label), ("long_label", &p.long_label)] {
            if !value.is_empty() && !valid_label(value) {
                out.push(diag(
                    "invalid_presentation",
                    "error",
                    &format!("presentation.{field}"),
                    "a presentation label is 1 to 40 characters without control characters",
                ));
            }
        }
        if p.order < 0 || p.handoff_order < 0 {
            out.push(diag(
                "invalid_presentation",
                "error",
                "presentation.order",
                "presentation.order and presentation.handoff_order must not be negative",
            ));
        }
    }
    out
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

/// A path pattern of a transcript declaration: it starts at `{home}` or
/// `{env:NAME}`, may use `{cwd_slug:alnum}` / `{cwd_slug:path}`, and contains
/// no `..`, no control character and no other `{...}`.
pub fn transcript_pattern_ok(pattern: &str) -> bool {
    if pattern.is_empty() || pattern.len() > 300 || control(pattern) || pattern.contains("..") {
        return false;
    }
    let mut rest = pattern;
    let mut first = true;
    while let Some(open) = rest.find('{') {
        if first && open != 0 {
            return false;
        }
        let Some(close) = rest[open..].find('}') else {
            return false;
        };
        let token = &rest[open + 1..open + close];
        let known = token == "home"
            || token == "cwd_slug:alnum"
            || token == "cwd_slug:path"
            || token.strip_prefix("env:").is_some_and(env_name);
        if !known || (first && token != "home" && !token.starts_with("env:")) {
            return false;
        }
        if first {
            // The first path segment is the token alone.
            let after = &rest[open + close + 1..];
            if !after.is_empty() && !after.starts_with(['/', '\\']) {
                return false;
            }
        }
        first = false;
        rest = &rest[open + close + 1..];
    }
    // A closing brace with no opening one is a typo too.
    !first && !rest.contains('}')
}

fn sql_name(value: &str) -> bool {
    let mut chars = value.chars();
    value.len() <= 64
        && chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn dotted_key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value
            .split('.')
            .all(|part| !part.is_empty() && !control(part))
}

/// Problems of a `transcript` declaration as (field, message).
fn transcript_problems(t: &TranscriptDefinition) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = vec![];
    let mut bad = |field: &str, message: &str| out.push((field.to_owned(), message.to_owned()));
    let shape_ok = (!t.root.is_empty() && t.file.is_empty() && !t.find.is_empty())
        || (t.root.is_empty() && !t.file.is_empty());
    if !shape_ok {
        // Either a root with find steps, or a single shared file.
        bad(
            "transcript",
            "transcript needs a root with find steps, or a file",
        );
    }
    for (name, pattern) in [("root", &t.root), ("file", &t.file)] {
        if !pattern.is_empty() && !transcript_pattern_ok(pattern) {
            bad(
                &format!("transcript.{name}"),
                "the path pattern must start with {home} or {env:NAME} and use only known {...} tokens",
            );
        }
    }
    let extension_ok = t.extension.is_empty()
        || (t.extension.len() <= 12 && t.extension.chars().all(|c| c.is_ascii_alphanumeric()));
    if !extension_ok {
        bad(
            "transcript.extension",
            "transcript.extension must be letters and digits without the dot",
        );
    }
    if !t.id_format.is_empty() && t.id_format != "uuid" {
        bad("transcript.id_format", "transcript.id_format must be uuid");
    }
    for (name, text) in [
        ("label", &t.label),
        ("dir_label", &t.dir_label),
        ("unavailable_reason", &t.unavailable_reason),
        ("not_found_reason", &t.not_found_reason),
    ] {
        if text.len() > 200 || control(text) {
            bad(
                &format!("transcript.{name}"),
                "text is too long or contains a control character",
            );
        }
    }
    for step in &t.find {
        match step.as_str() {
            "session_id" => {}
            "meta" => match &t.meta {
                Some(m) if dotted_key(&m.cwd_key) && dotted_key(&m.time_key) => {}
                _ => bad(
                    "transcript.meta",
                    "find: meta needs transcript.meta.cwd_key and time_key",
                ),
            },
            "index" => match &t.index {
                Some(i)
                    if transcript_pattern_ok(&i.file)
                        && [&i.table, &i.id_column, &i.cwd_column, &i.time_column]
                            .iter()
                            .all(|v| sql_name(v))
                        && matches!(i.time_unit.as_str(), "" | "s" | "ms") => {}
                _ => bad(
                    "transcript.index",
                    "find: index needs file, table and column names (letters, digits, _) and time_unit s or ms",
                ),
            },
            _ => bad(
                "transcript.find",
                "transcript.find entries must be session_id, meta or index",
            ),
        }
    }
    if let Some(m) = &t.meta
        && (m.kind_key.is_empty() != m.kind_value.is_empty()
            || (!m.kind_key.is_empty() && !dotted_key(&m.kind_key)))
    {
        bad(
            "transcript.meta",
            "kind_key and kind_value must be given together",
        );
    }
    if let Some(r) = &t.reader {
        if !matches!(r.format.as_str(), "json" | "jsonl") {
            bad("transcript.reader.format", "format must be json or jsonl");
        }
        if r.format == "json" && !dotted_key(&r.messages) {
            bad(
                "transcript.reader.messages",
                "format json needs the dotted path of the message array",
            );
        }
        if !dotted_key(&r.role_key) || !dotted_key(&r.text_key) {
            bad("transcript.reader", "reader needs role_key and text_key");
        }
        if !r.time_key.is_empty() && !dotted_key(&r.time_key) {
            bad(
                "transcript.reader.time_key",
                "time_key is not a valid key path",
            );
        }
        if r.roles.is_empty() || r.roles.values().any(|v| v != "user" && v != "assistant") {
            bad(
                "transcript.reader.roles",
                "roles must map at least one value to user or assistant",
            );
        }
        if t.root.is_empty() {
            bad(
                "transcript.reader",
                "a reader needs a root (a shared file is never read as a chat)",
            );
        }
    }
    out
}

/// Problems of an `approval` declaration as (field, message).
fn approval_problems(a: &ApprovalDefinition) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = vec![];
    let mut bad = |field: &str, message: &str| out.push((field.to_owned(), message.to_owned()));
    if !matches!(a.style.as_str(), "" | "numbered" | "buttons") {
        bad("approval.style", "style must be numbered or buttons");
    }
    for extra in &a.line_extras {
        if !matches!(
            extra.as_str(),
            "paren_marker" | "bracket_keys" | "key_lines"
        ) {
            bad(
                "approval.line_extras",
                "line_extras entries must be paren_marker, bracket_keys or key_lines",
            );
        }
    }
    let phrase = |p: &String| !p.is_empty() && p.len() <= 100 && !control(p);
    for (name, list) in [
        ("hint_phrases", &a.hint_phrases),
        ("not_approval_phrases", &a.not_approval_phrases),
    ] {
        if list.len() > 64 || !list.iter().all(phrase) {
            bad(
                &format!("approval.{name}"),
                "at most 64 phrases of 1 to 100 characters without control characters",
            );
        }
    }
    for (name, table) in [("keys", &a.keys), ("bracket_keys", &a.bracket_keys)] {
        if table.len() > 32 {
            bad(&format!("approval.{name}"), "too many entries");
        }
        for (i, k) in table.iter().enumerate() {
            let field = format!("approval.{name}[{i}]");
            if k.keys.is_empty() || !k.keys.iter().all(|s| phrase(s) && !s.contains(['(', ')'])) {
                bad(
                    &field,
                    "keys must be 1 to 100 character spellings without parentheses",
                );
            }
            if !(0..=20).contains(&k.num) {
                bad(&field, "num must be between 0 and 20");
            }
            if k.send.len() > 16 {
                bad(&field, "send is too long");
            }
        }
    }
    if a.style == "buttons" && a.buttons.is_empty() {
        bad("approval.buttons", "style buttons needs buttons");
    }
    if a.buttons.len() > 8 {
        bad("approval.buttons", "too many button rows");
    }
    for (i, b) in a.buttons.iter().enumerate() {
        let field = format!("approval.buttons[{i}]");
        if b.line_all.is_empty() || !b.line_all.iter().all(phrase) {
            bad(&field, "line_all needs 1 to 100 character words");
        }
        if !b.start_line.is_empty() && !phrase(&b.start_line) {
            bad(&field, "start_line is too long or has a control character");
        }
        if b.labels.is_empty() || b.labels.len() > 8 || !b.labels.iter().all(phrase) {
            bad(&field, "labels needs 1 to 8 entries");
        }
    }
    if a.suppress_screens.len() > 8 {
        bad("approval.suppress_screens", "too many entries");
    }
    for (i, s) in a.suppress_screens.iter().enumerate() {
        if !s.all.iter().chain(&s.any).all(phrase) || (s.all.is_empty() && s.any.is_empty()) {
            bad(
                &format!("approval.suppress_screens[{i}]"),
                "all and any need phrases (at least one list is not empty)",
            );
        }
    }
    out
}
