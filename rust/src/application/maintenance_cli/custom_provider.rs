//! Custom provider commands. Validation finishes before any private file is written.
use crate::{
    application::approval_patterns::{ApprovalPatterns, custom_pattern_path, encode_pattern_file},
    config::{self, ConfigStore, CustomProvider, HeadlessDef},
    process::execpath::{NativeFs, Platform, Resolver},
};
use serde_json::{Value, json};
use std::{io, sync::{Arc, Weak}};

pub const HELP: &str = "\
Usage:
  many-ai-cli provider add --id <id> --label <label> --command <command>
      [--headless-args <argument>]... [--headless-format text]
      [--prompt-via arg|stdin] [--approval-option <exact prompt text>]...
  many-ai-cli provider list [--json]
  many-ai-cli provider remove <id>
  many-ai-cli provider backup|reset|recover ...   (distributed registry history)

Before adding a CLI, look up its official documentation: the non-interactive
(headless) command or flags, prompt delivery (argument or stdin), exact approval
option text, model flag, and instruction files it reads. Show the user which
facts are documented and which are not documented BEFORE running provider add.

Example, Muse Code (official documentation: https://dev.meta.ai/docs/muse-code):
  many-ai-cli provider add --id muse --label \"Muse Code\" --command muse --headless-args exec --headless-format text --prompt-via arg --approval-option \"Allow once\" --approval-option \"Always allow in this workspace\" --approval-option \"Reject\"
Muse documents `muse exec <prompt>`, --model, and the first found of AGENTS.md,
CLAUDE.md, .agents/AGENTS.md, .claude/CLAUDE.md. A settings-directory override
and session-history location are not documented. Verify the installed CLI's
current documentation before choosing these options.

IDs are lowercase and exact. Built-in, duplicate, and top-level command IDs
are refused. --command is split as a command line, not evaluated by a shell;
use double quotes around words containing spaces. Repeat --headless-args for
each argument (use --headless-args=--flag for a leading dash). Headless format
is text; --prompt-via is arg or stdin. Approval options are literal phrases,
matched case-insensitively, not regular expressions. Empty options are refused.
Every successful add/remove backs up an existing config.yaml exactly once.
Comments and layout are not kept by the rewrite. No backup is needed when
there is no config.yaml. Restart a running Hub to load changed providers.
Remove keeps local approval patterns so adding the same id restores them.
";

pub fn handles(args: &[String]) -> bool {
    args.first().is_none_or(|s| matches!(s.as_str(), "add" | "list" | "remove" | "help" | "--help" | "-h"))
}

pub fn run(store: &Arc<ConfigStore>, args: &[String]) -> io::Result<String> {
    let environment: Vec<String> = std::env::vars_os()
        .filter_map(|(key, value)| key.into_string().ok().map(|key| format!("{key}={}", value.to_string_lossy())))
        .collect();
    let cwd = std::env::current_dir()?;
    let resolver = Resolver::new(Platform::native(), &environment, &cwd, &NativeFs);
    run_with(store, args, &|name| resolver.look_path(name))
}

fn run_with(
    store: &Arc<ConfigStore>,
    args: &[String],
    look_path: &dyn Fn(&str) -> io::Result<String>,
) -> io::Result<String> {
    let snapshot = store.snapshot().map_err(io::Error::other)?;
    let token = snapshot.config.token.clone();
    let result = match args.first().map(String::as_str) {
        None | Some("help" | "--help" | "-h") => Ok(HELP.into()),
        Some("add") if args[1..].iter().any(|s| matches!(s.as_str(), "--help" | "-h")) => Ok(HELP.into()),
        Some("add") => add(store, snapshot, &args[1..], look_path),
        Some("list") => list(store, &snapshot.config, &args[1..]),
        Some("remove") => remove(store, snapshot, &args[1..]),
        _ => Err(io::Error::other("unknown provider command; run many-ai-cli provider --help")),
    };
    // Even a token accidentally copied into an allowed display field stays private.
    let redact = |text: String| {
        if token.is_empty() {
            return text;
        }
        let encoded = serde_json::to_string(&token).expect("string serialization");
        text.replace(&token, "[redacted]")
            .replace(&encoded[1..encoded.len() - 1], "[redacted]")
    };
    // List redacts data before encoding; editing encoded JSON could replace
    // its quotes or newlines when an existing token contains those characters.
    result.map(|text| {
        if args.first().is_some_and(|arg| arg == "list") { text } else { redact(text) }
    }).map_err(|e| io::Error::new(e.kind(), redact(e.to_string())))
}

#[derive(Default)]
struct AddFlags {
    id: String,
    label: String,
    command: String,
    headless: Option<HeadlessDef>,
    approval: Vec<String>,
}
fn parse_add(args: &[String]) -> (AddFlags, Vec<String>) {
    let mut flags = AddFlags::default();
    let mut errors = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        let (name, inline) = arg.split_once('=').map_or((arg.as_str(), None), |(n, v)| (n, Some(v)));
        let known = matches!(name, "--id" | "--label" | "--command" | "--headless-args" | "--headless-format" | "--prompt-via" | "--approval-option");
        if !known {
            errors.push("unrecognized argument: use the flags shown by provider add --help".into());
            index += 1;
            continue;
        }
        let value = match inline {
            Some(value) => value.to_owned(),
            None if args.get(index + 1).is_some_and(|value| !value.starts_with("--")) => {
                index += 1;
                args[index].clone()
            }
            None => {
                errors.push(format!("{name}: a value is required; use {name}=<value> for a leading dash"));
                index += 1;
                continue;
            }
        };
        if !matches!(name, "--headless-args" | "--approval-option") && !seen.insert(name.to_owned()) {
            errors.push(format!("{name}: given more than once; supply it once"));
        }
        match name {
            "--id" => flags.id = value,
            "--label" => flags.label = value,
            "--command" => flags.command = value,
            "--approval-option" => flags.approval.push(value),
            _ => {
                let def = flags.headless.get_or_insert_with(HeadlessDef::default);
                match name {
                    "--headless-args" => def.args.push(value),
                    "--headless-format" => def.format = value,
                    "--prompt-via" => def.prompt_via = value,
                    _ => unreachable!(),
                }
            }
        }
        index += 1;
    }
    (flags, errors)
}
fn add(
    store: &Arc<ConfigStore>,
    snapshot: config::ConfigSnapshot,
    args: &[String],
    look_path: &dyn Fn(&str) -> io::Result<String>,
) -> io::Result<String> {
    let (flags, mut errors) = parse_add(args);
    if let Err(error) = config::validate_custom_provider_id(&flags.id) {
        errors.push(format!("--id: {error}; choose a non-reserved id of 1–64 lowercase letters, digits, dot, underscore or hyphen, starting with a letter or digit"));
    }
    if flags.id != config::normalize_subscription_id(&flags.id) {
        errors.push("--id: use lowercase with no surrounding whitespace".into());
    }
    // Only this command rejects CLI names: existing settings keep their old semantics.
    if crate::cli::parse(&[flags.id.clone()], &[]).is_ok()
        || matches!(flags.id.as_str(), "wrap" | "orchestrate")
    {
        errors.push("--id: a top-level CLI command already uses this name; choose another id".into());
    }
    if store.contains_custom_provider_id(&flags.id).map_err(io::Error::other)?
    {
        errors.push("--id: already present in custom_providers (possibly dropped); remove that id first or choose another".into());
    }
    match config::split_command_line(&flags.command) {
        Ok(argv) if argv.first().is_some_and(|s| !s.trim().is_empty()) => {
            if look_path(&argv[0]).is_err() {
                errors.push("--command: executable was not found on PATH; install it or supply its executable path (Windows .cmd launchers are supported)".into());
            }
        }
        Ok(_) => errors.push("--command: the executable is empty; supply a program name or path".into()),
        Err(error) => errors.push(format!("--command: {error}; supply a nonempty command with balanced double quotes and no control characters")),
    }
    if let Some(def) = &flags.headless {
        // Validate independently so a bad format cannot hide a bad prompt or argument.
        if def.format != config::HEADLESS_FORMAT_TEXT {
            errors.push("--headless-format: use text when providing headless flags".into());
        }
        if !matches!(def.prompt_via.as_str(), "arg" | "stdin") {
            errors.push("--prompt-via: specify arg or stdin when providing headless flags".into());
        }
        let mut check = def.clone();
        check.format = config::HEADLESS_FORMAT_TEXT.into();
        check.prompt_via = config::HEADLESS_PROMPT_VIA_ARG.into();
        if let Err(error) = config::validate_headless_def(&check) {
            errors.push(format!("--headless-args: {error}; use nonempty arguments without control characters within the documented size limits"));
        }
        if config::validate_headless_def(def).is_err() && errors.is_empty() {
            errors.push("--headless-format / --prompt-via: invalid definition; use text and arg or stdin".into());
        }
    }
    if flags.approval.iter().any(|s| s.trim().is_empty() || s.chars().any(char::is_control)) {
        errors.push("--approval-option: use a nonempty literal prompt phrase without control characters".into());
    }
    if encode_pattern_file(&flags.approval).is_err() {
        errors.push("--approval-option: combined phrases exceed the 2 MiB pattern-file limit; shorten them".into());
    }
    if flags.id == "common" && !flags.approval.is_empty() {
        errors.push("--approval-option: common names the shared detector phrases; choose a different --id for provider-specific options".into());
    }
    if !errors.is_empty() {
        return Err(io::Error::other(errors.join("\n")));
    }
    let entry = CustomProvider {
        id: flags.id,
        label: flags.label,
        command: flags.command,
        approval_pattern_source: String::new(),
        headless: flags.headless,
    };
    let mut next = snapshot.config;
    next.custom_providers.push(entry.clone());
    let (_, backup, comments) = store.persist_with_backup(snapshot.revision, next, || {
        crate::process::random_token().map_err(config::ConfigError::from)
    }).map_err(io::Error::other)?;
    let mut out = format!("Added {}\n", row(&entry, false, false, Vec::new()));
    out.push_str(&saved_notice(backup.as_ref(), comments));
    if !flags.approval.is_empty() {
        let write = ApprovalPatterns::new(store.paths(), store.clone(), Weak::new(), Arc::new(|_| {}))
            .and_then(|patterns| patterns.write_custom(&entry.id, &flags.approval));
        if write.is_err() {
            return Err(io::Error::other(format!("Provider added, but approval patterns could not be saved. {out}The provider remains registered. Fix access to the approval-patterns directory, then remove this id and add it again with the options; alternatively restore the backup if one is listed.")));
        }
    }
    out.push_str(&format!("Next: restart a running Hub, then run many-ai-cli {} or choose it in the Hub.\n", entry.id));
    Ok(out)
}
fn saved_notice(backup: Option<&config::BackupRecord>, comments: bool) -> String {
    let mut out = match backup {
        Some(backup) => format!("Backup: {} (restore with many-ai-cli config backup restore {})\n", backup.id, backup.id),
        None => "No previous config.yaml existed; no backup was needed.\n".into(),
    };
    if comments && let Some(backup) = backup {
        out.push_str(&format!("Comments were not kept; the previous file is in backup {}.\n", backup.id));
    }
    out
}
fn row(entry: &CustomProvider, builtin: bool, dropped: bool, reasons: Vec<String>) -> Value {
    json!({"id": entry.id, "label": entry.label, "command": entry.command,
        "builtin": builtin, "headless": entry.headless.is_some(),
        "status": if dropped { "dropped" } else { "in-use" }, "reasons": reasons})
}
fn list(store: &ConfigStore, cfg: &config::Config, args: &[String]) -> io::Result<String> {
    if !args.is_empty() && args != ["--json"] {
        return Err(io::Error::other("provider list takes only --json"));
    }
    let mut rows = Vec::new();
    for definition in crate::profile::registry::embedded_definitions().map_err(io::Error::other)? {
        if !crate::cli::BUILTIN_PROVIDERS.contains(&definition.id.as_str()) {
            continue;
        }
        let command = definition.launch.as_ref().map(|launch| {
            std::iter::once(launch.executable.as_str())
                .chain(launch.args.iter().map(String::as_str))
                .map(|word| {
                    if word.contains([' ', '\t', '"']) {
                        format!("\"{}\"", word.replace('"', "\"\""))
                    } else {
                        word.to_owned()
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        }).unwrap_or_default();
        rows.push(row(&CustomProvider {
            id: definition.id.clone(), label: definition.display_name,
            command, headless: config::headless_def_for(&definition.id, Some(cfg)),
            ..Default::default()
        }, true, false, Vec::new()));
    }
    rows.extend(store.provider_listing().map_err(io::Error::other)?);
    fn redact_fields(value: &mut Value, token: &str) {
        match value {
            Value::String(text) => *text = text.replace(token, "[redacted]"),
            Value::Array(items) => items.iter_mut().for_each(|item| redact_fields(item, token)),
            Value::Object(fields) => fields.values_mut().for_each(|field| redact_fields(field, token)),
            _ => {}
        }
    }
    if !cfg.token.is_empty() {
        rows.iter_mut().for_each(|row| redact_fields(row, &cfg.token));
    }
    if args.is_empty() {
        Ok(rows.iter().map(|r| format!("{r}\n")).collect())
    } else {
        serde_json::to_string_pretty(&rows).map(|s| format!("{s}\n")).map_err(io::Error::other)
    }
}
fn remove(store: &Arc<ConfigStore>, snapshot: config::ConfigSnapshot, args: &[String]) -> io::Result<String> {
    if args.len() != 1 {
        return Err(io::Error::other("provider remove <id>"));
    }
    let id = config::normalize_subscription_id(&args[0]);
    if config::is_builtin_provider_id(&id) || id == "shell" {
        return Err(io::Error::other("built-in providers cannot be removed; choose a custom provider id from provider list"));
    }
    let mut next = snapshot.config;
    let before = next.custom_providers.len();
    next.custom_providers.retain(|p| config::normalize_subscription_id(&p.id) != id);
    let dropped = store.contains_custom_provider_id(&id).map_err(io::Error::other)?;
    if before == next.custom_providers.len() && !dropped {
        return Err(io::Error::other("custom provider id does not exist; run provider list"));
    }
    let (_, backup, comments) = store.persist_with_backup(snapshot.revision, next, || {
        crate::process::random_token().map_err(config::ConfigError::from)
    }).map_err(io::Error::other)?;
    let mut out = format!("Removed custom provider {}.\n", serde_json::to_string(&id).expect("string serialization"));
    out.push_str(&saved_notice(backup.as_ref(), comments));
    out.push_str(&format!("Approval patterns kept if present: {:?}\nLegacy patterns, if present, are also kept in approval-patterns.\nRestart a running Hub to load this change.\n", custom_pattern_path(store.paths(), &id)));
    Ok(out)
}

#[cfg(test)]
mod tests;
