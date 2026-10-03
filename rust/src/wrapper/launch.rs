//! Wrapper launch preparation, from `internal/wrapper` at Go oracle 21d0bc7.
//!
//! This module never discovers a home, starts a process, or contacts a Hub.
//! The runtime resolves executables, satisfies the listed hooks, registers the
//! session, then prepends the prepared prompt after its last argument addition.
use crate::config::{self, Config, HeadlessDef, Resource, RuntimePaths};
use crate::files::safe_fs::Dir;
use crate::profile::registry::{self, Registry};
use crate::proto::provider::{EffectiveDefinition, LaunchRequest, Layers};
use std::{
    fmt, io,
    io::Read,
    path::{Component, Path, PathBuf},
};

pub const HUB_TOKEN_ENV: &str = "MANY_AI_CLI_HUB_TOKEN";
pub const DELEGATION_ENV: &str = "MANY_AI_CLI_DELEGATION";
pub const SESSION_ID_PLACEHOLDER: &str = "{{many-ai-cli:session-id}}";
pub const LAUNCH_PROMPT_MAX_ARG_COST: usize = 24_000;

/// Go's flag package stops at its first positional or parse error. Run ignores
/// that error, so callers must not reject this structure merely for parse_error.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct WrapperArgs {
    pub label: String,
    pub model: String,
    pub permission_mode: String,
    pub effort: String,
    pub allowed_tools: String,
    pub sandbox: String,
    pub ask_for_approval: String,
    pub codex_oss: bool,
    pub utf8: bool,
    pub subscription_login: bool,
    pub headless: bool,
    pub prompt_file: String,
    pub provider_args: Vec<String>,
    pub parse_error: Option<String>,
}
impl WrapperArgs {
    pub fn parse(args: &[String]) -> Self {
        let mut parsed = Self::default();
        let mut index = 0;
        while let Some(arg) = args.get(index) {
            if arg == "--" {
                index += 1;
                break;
            }
            if arg.len() < 2 || !arg.starts_with('-') {
                break;
            }
            let name_value = arg.strip_prefix("--").unwrap_or(&arg[1..]);
            // Go leaves malformed flags in Args(), unlike unknown flags.
            if name_value.is_empty() || name_value.starts_with(['-', '=']) {
                parsed.parse_error = Some(format!("bad flag syntax: {arg}"));
                break;
            }
            index += 1;
            let (name, value) = name_value
                .split_once('=')
                .map_or((name_value, None), |(n, v)| (n, Some(v)));
            let bool_target = match name {
                "codex-oss" => Some(&mut parsed.codex_oss),
                "utf8" => Some(&mut parsed.utf8),
                "subscription-login" => Some(&mut parsed.subscription_login),
                "headless" => Some(&mut parsed.headless),
                _ => None,
            };
            if let Some(target) = bool_target {
                let value = value.unwrap_or("true");
                match value {
                    "1" | "t" | "T" | "TRUE" | "true" | "True" => *target = true,
                    "0" | "f" | "F" | "FALSE" | "false" | "False" => *target = false,
                    _ => {
                        // flag.boolValue.Set assigns strconv.ParseBool's false even on error.
                        *target = false;
                        parsed.parse_error = Some(format!(
                            "invalid boolean value {value:?} for -{name}: parse error"
                        ));
                        break;
                    }
                }
                continue;
            }
            let target = match name {
                "label" => Some(&mut parsed.label),
                "model" => Some(&mut parsed.model),
                "permission-mode" => Some(&mut parsed.permission_mode),
                "effort" => Some(&mut parsed.effort),
                "allowed-tools" => Some(&mut parsed.allowed_tools),
                "sandbox" => Some(&mut parsed.sandbox),
                "ask-for-approval" => Some(&mut parsed.ask_for_approval),
                "prompt-file" => Some(&mut parsed.prompt_file),
                _ => None,
            };
            let Some(target) = target else {
                parsed.parse_error = Some(if matches!(name, "help" | "h") {
                    "flag: help requested".into()
                } else {
                    format!("flag provided but not defined: -{name}")
                });
                break;
            };
            let value = match value {
                Some(value) => value,
                None => match args.get(index) {
                    Some(value) => {
                        index += 1;
                        value
                    }
                    None => {
                        parsed.parse_error = Some(format!("flag needs an argument: -{name}"));
                        break;
                    }
                },
            };
            *target = value.into();
        }
        parsed.provider_args = args[index..].to_vec();
        parsed
    }
}

#[derive(Debug)]
pub struct LaunchError(pub String);
impl fmt::Display for LaunchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for LaunchError {}

/// Runtime work which this planner must not pretend to have performed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaunchHook {
    /// Apply/restore opencode.json permission rules, including bounded bash deny.
    OpenCodePermissions {
        permission: String,
        bash_deny: Vec<String>,
    },
    /// Inspect register_ack token_statusbar, orchestration_id and auto, then
    /// create one private --settings file if either Claude setting is enabled.
    ClaudeRegistrationSettings,
    /// Evaluate MANY_AI_CLI_DELEGATION's 1/0 override before the config default.
    DelegationPrompt { config_default: bool },
    /// Windows-only console encoding side effect; Unix explicitly needs no work.
    Utf8Console,
}
impl LaunchHook {
    pub fn description(&self) -> &'static str {
        match self {
            Self::OpenCodePermissions { .. } => {
                "OpenCode temporary permission configuration and restoration"
            }
            Self::ClaudeRegistrationSettings => {
                "Claude registration-dependent private statusline/cross-session settings"
            }
            Self::DelegationPrompt { .. } => {
                "delegation environment gate and provider prompt preparation"
            }
            Self::Utf8Console => "Windows session UTF-8 console encoding",
        }
    }
}

/// Provider arguments exclude the prompt body and later session-owned hooks.
/// No Debug implementation: arbitrary custom arguments may carry credentials.
pub struct LaunchPlan {
    pub provider: String,
    pub args: WrapperArgs,
    pub provider_args: Vec<String>,
    /// Custom argv takes precedence over every built-in/provider-name fallback.
    pub custom_argv: Option<Vec<String>>,
    /// Hints for the shared resolver. It owns shell and copilot/gh fallback rules.
    pub executable_candidates: Vec<String>,
    pub display: String,
    pub label: String,
    /// Go registers the requested model even when custom-provider flags skip it.
    pub model: String,
    pub effort: String,
    pub permission_mode: String,
    pub execution_mode: String,
    pub agent_session_id: String,
    pub login_mode: bool,
    pub headless: bool,
    pub headless_definition: Option<HeadlessDef>,
    pub required_hooks: Vec<LaunchHook>,
    /// Reasons only, never a prompt, environment, token, or command body.
    pub diagnostics: Vec<String>,
}

pub fn provider_registry_for_config(cfg: &Config) -> Result<Registry, LaunchError> {
    let embedded = registry::embedded_definitions().map_err(|e| LaunchError(e.to_string()))?;
    let (legacy, _) = config::legacy_provider_definitions(&cfg.custom_providers);
    Ok(Registry::build(
        Layers {
            embedded: Some(embedded),
            legacy: Some(legacy),
            ..Default::default()
        },
        &registry::default_adapters(),
    ))
}

pub fn prepare_launch(
    cfg: &Config,
    provider: &str,
    args: WrapperArgs,
) -> Result<LaunchPlan, LaunchError> {
    let registry = provider_registry_for_config(cfg)?;
    prepare_launch_with_definition(cfg, provider, args, registry.lookup(provider).as_ref())
}

/// Explicit definition injection is useful to registry callers and synthetic
/// tests; ordinary wrap uses exactly embedded+legacy layers, like Go Run.
pub fn prepare_launch_with_definition(
    cfg: &Config,
    provider: &str,
    args: WrapperArgs,
    definition: Option<&EffectiveDefinition>,
) -> Result<LaunchPlan, LaunchError> {
    let custom = config::effective_custom_providers(&cfg.custom_providers)
        .into_iter()
        .find(|p| p.id == provider);
    let login_mode = args.subscription_login;
    if login_mode && args.headless {
        return Err(LaunchError(
            "--headless cannot be combined with --subscription-login".into(),
        ));
    }
    if login_mode && !args.prompt_file.trim().is_empty() {
        return Err(LaunchError(
            "--prompt-file cannot be combined with --subscription-login".into(),
        ));
    }
    let mut provider_args = if login_mode {
        subscription_login_args(provider).ok_or_else(|| {
            LaunchError(format!(
                "provider {provider:?} does not support subscription profiles"
            ))
        })?
    } else {
        args.provider_args.clone()
    };
    let mut effort = String::new();
    let mut permission_mode = String::new();
    let mut diagnostics = Vec::new();
    if !login_mode {
        let mut extra = Vec::new();
        match model_args(&args.model, custom.is_some(), definition) {
            Ok(model) => extra.extend(model),
            Err(reason) => diagnostics.push(format!("model ignored: {reason}")),
        }
        let effort_result = if custom.is_some() {
            match definition {
                Some(def) => registry::effort_args(def, &args.effort).and_then(|v| {
                    if v.is_empty() && !args.effort.is_empty() {
                        Err("provider definition has no effort mapping".into())
                    } else {
                        Ok(v)
                    }
                }),
                None if args.effort.is_empty() => Ok(vec![]),
                None => Err("custom provider takes no built-in flags".into()),
            }
        } else {
            config::validate_effort(provider, &args.effort)
                .map(|()| config::effort_args(provider, &args.effort))
                .map_err(|e| e.to_string())
        };
        match effort_result {
            Ok(values) => {
                if !values.is_empty() {
                    effort = args.effort.clone();
                }
                extra.extend(values);
            }
            Err(reason) => diagnostics.push(format!("effort ignored: {reason}")),
        }
        if provider == "codex" {
            if args.codex_oss {
                extra.push("--oss".into());
            }
            if !args.sandbox.is_empty() {
                extra.extend(["--sandbox".into(), args.sandbox.clone()]);
            }
            if !args.ask_for_approval.is_empty() {
                extra.extend(["--ask-for-approval".into(), args.ask_for_approval.clone()]);
            }
        }
        let (permission_args, declared) =
            permission_args_for_provider(provider, &args.permission_mode);
        extra.extend(permission_args);
        permission_mode = declared;
        extra.extend(allowed_tool_args(
            provider,
            &parse_allowed_tool_values(&args.allowed_tools),
        ));
        extra.extend(provider_args);
        provider_args = extra;
    }
    let agent_session_id = if login_mode {
        String::new()
    } else {
        let (prepared, id) = prepare_claude_session_args(provider, provider_args)
            .map_err(|e| LaunchError(format!("generate Claude session id: {e}")))?;
        provider_args = prepared;
        id
    };
    let custom_argv = if let Some(custom) = &custom {
        Some(
            if let Some(def) = definition.filter(|d| d.definition.launch.is_some()) {
                let resolved = registry::resolve_launch(def, &LaunchRequest::default())
                    .map_err(LaunchError)?;
                let mut values = resolved
                    .executable_candidates
                    .unwrap_or_default()
                    .into_iter()
                    .take(1)
                    .collect::<Vec<_>>();
                values.extend(resolved.args.unwrap_or_default());
                values
            } else {
                custom.argv().map_err(|e| LaunchError(e.to_string()))?
            },
        )
    } else {
        None
    };
    let executable_candidates = match &custom_argv {
        Some(argv) => argv.first().cloned().into_iter().collect(),
        None if provider == "copilot" => vec!["copilot".into(), "gh".into()],
        None => vec![provider.into()],
    };
    let display = match provider {
        "claude" => "Claude",
        "codex" => "Codex",
        "copilot" => "GitHub Copilot",
        "cursor-agent" => "Cursor Agent",
        "opencode" => "OpenCode",
        "grok" => "Grok Build",
        "command-code" => "Command Code",
        "shell" => "Shell",
        _ => "",
    };
    let display = if display.is_empty() {
        custom
            .as_ref()
            .map(|p| p.effective_label().to_owned())
            .unwrap_or_default()
    } else {
        display.into()
    };
    let headless_definition = definition
        .and_then(|d| d.definition.launch.as_ref())
        .and_then(|l| l.headless.as_ref())
        .map(|h| HeadlessDef {
            args: h.args.clone(),
            format: h.format.clone(),
            prompt_via: h.prompt_via.clone(),
        })
        .or_else(|| config::headless_def_for(provider, Some(cfg)));
    let mut required_hooks = Vec::new();
    if !login_mode {
        if provider == "opencode" {
            let permission = if matches!(
                args.permission_mode.as_str(),
                "bounded" | "bypassPermissions"
            ) {
                "allow"
            } else {
                "ask"
            };
            let bash_deny = if args.permission_mode == config::PERMISSION_MODE_BOUNDED {
                strings(&["git push*", "git reset*", "git clean*", "rm*", "sudo*"])
            } else {
                vec![]
            };
            required_hooks.push(LaunchHook::OpenCodePermissions {
                permission: permission.into(),
                bash_deny,
            });
        }
        if !args.headless {
            if provider == "claude" {
                required_hooks.push(LaunchHook::ClaudeRegistrationSettings);
            }
            // Go's argument hook currently supports these two providers. Other
            // providers' project-file delegation injection belongs to the Hub.
            if matches!(provider, "claude" | "grok") {
                required_hooks.push(LaunchHook::DelegationPrompt {
                    config_default: cfg.user_prefs.spawn.delegation_auto,
                });
            }
        }
    }
    if args.utf8 && !args.headless {
        required_hooks.push(LaunchHook::Utf8Console);
    }
    Ok(LaunchPlan {
        provider: provider.into(),
        label: args.label.clone(),
        model: args.model.clone(),
        effort,
        permission_mode,
        execution_mode: if args.headless {
            config::EXECUTION_MODE_HEADLESS.into()
        } else {
            String::new()
        },
        agent_session_id,
        login_mode,
        headless: args.headless,
        headless_definition,
        args,
        provider_args,
        custom_argv,
        executable_candidates,
        display,
        required_hooks,
        diagnostics,
    })
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| (*v).into()).collect()
}
pub fn subscription_login_args(provider: &str) -> Option<Vec<String>> {
    match provider {
        "claude" => Some(strings(&["auth", "login"])),
        "codex" | "grok" => Some(strings(&["login"])),
        "opencode" => Some(strings(&["providers", "login"])),
        _ => None,
    }
}
pub fn model_args(
    model: &str,
    is_custom: bool,
    definition: Option<&EffectiveDefinition>,
) -> Result<Vec<String>, String> {
    if model.is_empty() {
        return Ok(vec![]);
    }
    if !is_custom {
        return Ok(vec!["--model".into(), model.into()]);
    }
    match definition.filter(|d| {
        d.definition
            .launch
            .as_ref()
            .is_some_and(|l| !l.model_args.is_empty())
    }) {
        Some(def) => registry::model_args(def, model),
        None => Err("custom provider takes no built-in flags".into()),
    }
}
pub fn permission_args_for_provider(provider: &str, mode: &str) -> (Vec<String>, String) {
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
pub fn parse_allowed_tool_values(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|v| config::valid_allowed_tool_value(v))
        .map(Into::into)
        .collect()
}
pub fn allowed_tool_args(provider: &str, values: &[String]) -> Vec<String> {
    if values.is_empty() {
        return vec![];
    }
    match provider {
        "claude" => std::iter::once("--allowedTools".into())
            .chain(values.iter().cloned())
            .collect(),
        "copilot" => values.iter().map(|v| format!("--allow-tool={v}")).collect(),
        _ => vec![],
    }
}
pub fn claude_session_flag_present(args: &[String]) -> bool {
    args.iter().any(|arg| {
        matches!(
            arg.as_str(),
            "--session-id" | "--resume" | "-r" | "--continue" | "--fork-session"
        ) || arg.starts_with("--session-id=")
            || arg.starts_with("--resume=")
    })
}
pub fn prepare_claude_session_args(
    provider: &str,
    args: Vec<String>,
) -> io::Result<(Vec<String>, String)> {
    if provider != "claude" || claude_session_flag_present(&args) {
        return Ok((args, String::new()));
    }
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|_| io::Error::other("operating-system random source failed"))?;
    bytes[6] = (bytes[6] & 15) | 64;
    bytes[8] = (bytes[8] & 63) | 128;
    let hex = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let id = format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    );
    let mut prepared = vec!["--session-id".into(), id.clone()];
    prepared.extend(args);
    Ok((prepared, id))
}

/// Environment ordering is intentional: the process adapter must implement
/// last-value-wins semantics. Proof credentials are removed even from extras.
pub fn child_env(base: &[String], terminal_color: &str, extra: &[String]) -> Vec<String> {
    let color = config::normalize_terminal_color(terminal_color);
    let excluded = match color {
        "force" => Some("NO_COLOR"),
        "off" => Some("CLICOLOR_FORCE"),
        _ => None,
    };
    let mut result = env_without(base, &excluded.into_iter().collect::<Vec<_>>());
    result.extend(strings(&[
        "TERM=xterm-256color",
        "COLORTERM=truecolor",
        "MANY_AI_CLI=1",
    ]));
    match color {
        "force" => result.extend(strings(&["FORCE_COLOR=3", "CLICOLOR_FORCE=1"])),
        "off" => result.extend(strings(&["NO_COLOR=1", "FORCE_COLOR=0"])),
        _ => {}
    }
    result.extend_from_slice(extra);
    env_without(&result, &[crate::proto::core::SPAWN_PROOF_ENV])
}
pub fn env_without(env: &[String], names: &[&str]) -> Vec<String> {
    env.iter()
        .filter(|kv| {
            !kv.split_once('=')
                .is_some_and(|(name, _)| names.iter().any(|want| name.eq_ignore_ascii_case(want)))
        })
        .cloned()
        .collect()
}
pub fn delegation_prompt_enabled(config_default: bool, env: &[String]) -> bool {
    match env
        .iter()
        .rev()
        .find_map(|v| v.strip_prefix("MANY_AI_CLI_DELEGATION="))
    {
        Some("1") => true,
        Some("0") => false,
        _ => config_default,
    }
}

/// Pointer cleanup is tied to the held directory capability, so a rename cannot
/// redirect cleanup to a different tree. Debug intentionally omits the prompt.
pub struct PreparedLaunchPrompt {
    pub arg: String,
    pub pointer: bool,
    pub path: Option<PathBuf>,
    cleanup: Option<(Dir, String)>,
}
impl PreparedLaunchPrompt {
    pub fn cleanup(&mut self) -> io::Result<()> {
        let Some((dir, name)) = self.cleanup.as_ref() else {
            return Ok(());
        };
        match dir.remove_file(name) {
            Ok(()) => {
                self.cleanup = None;
                Ok(())
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.cleanup = None;
                Ok(())
            }
            Err(error) => Err(error),
        }
    }
}
impl Drop for PreparedLaunchPrompt {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

/// Explicitly supplied prompt paths are accepted in production. Trial paths
/// must stay under RuntimePaths, including symlink-safe directory/file opens.
pub fn take_prompt_file(paths: &RuntimePaths, prompt_path: &Path) -> io::Result<String> {
    if prompt_path.as_os_str().is_empty() {
        return Ok(String::new());
    }
    let absolute = if prompt_path.is_absolute() {
        prompt_path.to_owned()
    } else {
        std::env::current_dir()?.join(prompt_path)
    };
    if absolute
        .components()
        .any(|c| matches!(c, Component::ParentDir))
        || (paths.is_trial() && !absolute.starts_with(paths.root()))
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "prompt path escapes selected runtime root",
        ));
    }
    let parent = absolute
        .parent()
        .ok_or_else(|| io::Error::other("prompt file has no parent"))?;
    let name = absolute
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or_else(|| io::Error::other("prompt file has no UTF-8 basename"))?;
    let dir = Dir::open(parent)?;
    let read = (|| {
        let mut bytes = Vec::new();
        dir.open_file(name, false)?.read_to_end(&mut bytes)?;
        Ok::<_, io::Error>(bytes)
    })();
    // Go defers removal even after a read failure. The capability never follows
    // a symlink while reading and removal can remove only its directory entry.
    let _ = dir.remove_file(name);
    let bytes = read.map_err(|e| io::Error::new(e.kind(), format!("read prompt file: {e}")))?;
    String::from_utf8(bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "prompt file is not UTF-8"))
}
pub fn expand_prompt(prompt: &str, session_id: i64) -> String {
    prompt.replace(SESSION_ID_PLACEHOLDER, &session_id.to_string())
}
pub fn launch_prompt_arg(prompt: &str) -> String {
    if prompt.starts_with('-') || !prompt.chars().any(char::is_whitespace) {
        format!(" {prompt}")
    } else {
        prompt.into()
    }
}
pub fn launch_prompt_arg_cost(prompt: &str) -> usize {
    prompt.encode_utf16().count() + prompt.bytes().filter(|b| matches!(b, b'"' | b'\\')).count() + 2
}
/// through_shell is the actual resolved cmd.exe shim path, never a guess from
/// the provider name. None means direct execution on either supported OS.
pub fn launch_prompt_arg_usable(
    through_shell: Option<&str>,
    dir: &Path,
) -> Result<(), &'static str> {
    let Some(shim) = through_shell else {
        return Ok(());
    };
    if shim.contains([' ', '\t']) {
        return Err("the launch goes through cmd.exe and the CLI's shim path contains whitespace");
    }
    if dir
        .to_string_lossy()
        .contains(['%', '!', '^', '&', '|', '<', '>', '"'])
    {
        return Err(
            "the launch goes through cmd.exe and the temp directory path contains a character cmd.exe interprets",
        );
    }
    Ok(())
}
pub fn prepare_launch_prompt(
    paths: &RuntimePaths,
    prompt_path: &Path,
    session_id: i64,
    through_shell: Option<&str>,
    pid: u32,
) -> io::Result<PreparedLaunchPrompt> {
    let prompt = expand_prompt(&take_prompt_file(paths, prompt_path)?, session_id);
    let empty = || PreparedLaunchPrompt {
        arg: String::new(),
        pointer: false,
        path: None,
        cleanup: None,
    };
    if prompt.trim().is_empty() {
        return Ok(empty());
    }
    if through_shell.is_none() && launch_prompt_arg_cost(&prompt) <= LAUNCH_PROMPT_MAX_ARG_COST {
        return Ok(PreparedLaunchPrompt {
            arg: launch_prompt_arg(&prompt),
            pointer: false,
            path: None,
            cleanup: None,
        });
    }
    let directory = paths.resource(Resource::Temporary);
    launch_prompt_arg_usable(through_shell, &directory).map_err(|reason| {
        io::Error::other(format!("cannot pass the instruction at launch: {reason}"))
    })?;
    let dir = Dir::open_or_create_private(&directory)?;
    let mut random = [0u8; 16];
    getrandom::fill(&mut random)
        .map_err(|_| io::Error::other("launch prompt random source failed"))?;
    let suffix = random
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let name = format!("launch-{pid}-{suffix}.md");
    dir.create_new(&name, prompt.as_bytes(), 0o600)?;
    let path = directory.join(&name);
    let arg = format!(
        "Your instructions are in the file {}. Read that file now and follow it exactly.",
        path.display()
    );
    Ok(PreparedLaunchPrompt {
        arg,
        pointer: true,
        path: Some(path),
        cleanup: Some((dir, name)),
    })
}
pub fn with_launch_prompt(provider_args: &[String], arg: &str) -> Vec<String> {
    std::iter::once(arg.to_owned())
        .chain(provider_args.iter().cloned())
        .collect()
}
pub fn launch_prompt_file_owner(name: &str) -> Option<i64> {
    let rest = name.strip_prefix("launch-")?.strip_suffix(".md")?;
    let (pid, _) = rest.split_once('-')?;
    let pid = pid.parse::<i64>().ok()?;
    (pid > 0).then_some(pid)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn argv(values: &[&str]) -> Vec<String> {
        strings(values)
    }
    fn parse(values: &[&str]) -> WrapperArgs {
        WrapperArgs::parse(&argv(values))
    }
    fn trial() -> (tempfile::TempDir, tempfile::TempDir, RuntimePaths) {
        let root = tempfile::tempdir().unwrap();
        let installed = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::trial(root.path(), 49071, installed.path()).unwrap();
        (root, installed, paths)
    }
    #[test]
    fn wrapper_flags_match_go_stop_and_consumption_rules() {
        let p = parse(&["-model=a", "--headless", "false", "--effort", "high"]);
        assert_eq!(p.model, "a");
        assert!(p.headless);
        assert_eq!(p.provider_args, argv(&["false", "--effort", "high"]));
        let p = parse(&["--model", "--literal", "--unknown=x", "--effort", "high"]);
        assert_eq!(p.model, "--literal");
        assert_eq!(p.provider_args, argv(&["--effort", "high"]));
        assert_eq!(
            p.parse_error.as_deref(),
            Some("flag provided but not defined: -unknown")
        );
        let p = parse(&["--headless", "--headless=bad", "next"]);
        assert!(!p.headless);
        assert!(p.parse_error.is_some());
        assert_eq!(p.provider_args, argv(&["next"]));
        for malformed in ["---bad", "-=x", "--=x"] {
            assert_eq!(
                parse(&[malformed, "next"]).provider_args,
                argv(&[malformed, "next"])
            );
        }
        assert_eq!(
            parse(&["--", "--model", "m"]).provider_args,
            argv(&["--model", "m"])
        );
        assert_eq!(
            parse(&["-", "--model", "m"]).provider_args,
            argv(&["-", "--model", "m"])
        );
        assert_eq!(
            parse(&["--model"]).parse_error.as_deref(),
            Some("flag needs an argument: -model")
        );
        assert_eq!(parse(&["--help", "tail"]).provider_args, argv(&["tail"]));
        for value in ["1", "t", "T", "TRUE", "true", "True"] {
            assert!(parse(&[&format!("--utf8={value}")]).utf8);
        }
        for value in ["0", "f", "F", "FALSE", "false", "False"] {
            assert!(!parse(&[&format!("--utf8={value}")]).utf8);
        }
    }
    #[test]
    fn codex_flags_and_declared_metadata_follow_actual_mapping() {
        let p = prepare_launch(
            &Config::default(),
            "codex",
            parse(&[
                "--model",
                "m",
                "--effort=high",
                "--codex-oss",
                "--sandbox=workspace-write",
                "--ask-for-approval=never",
                "--permission-mode=bypassPermissions",
                "--",
                "tail",
            ]),
        )
        .unwrap();
        assert_eq!(
            p.provider_args,
            argv(&[
                "--model",
                "m",
                "-c",
                "model_reasoning_effort=\"high\"",
                "--oss",
                "--sandbox",
                "workspace-write",
                "--ask-for-approval",
                "never",
                "tail"
            ])
        );
        assert_eq!(p.model, "m");
        assert_eq!(p.effort, "high");
        assert_eq!(p.permission_mode, "");
        assert_eq!(p.execution_mode, "");
        assert_eq!(p.agent_session_id, "");
        let p = prepare_launch(
            &Config::default(),
            "codex",
            parse(&["--effort=wrong", "--headless"]),
        )
        .unwrap();
        assert!(p.provider_args.is_empty());
        assert_eq!(p.effort, "");
        assert_eq!(p.execution_mode, "headless");
        assert_eq!(p.diagnostics.len(), 1);
    }
    #[test]
    fn custom_command_precedence_suppresses_builtin_flags() {
        let mut cfg = Config::default();
        cfg.custom_providers.push(config::CustomProvider {
            id: "synthetic".into(),
            label: "Synthetic Provider".into(),
            command: "\"/synthetic/tool path\" --base one".into(),
            ..Default::default()
        });
        let p = prepare_launch(
            &cfg,
            "synthetic",
            parse(&[
                "--model=m",
                "--effort=high",
                "--permission-mode=bypassPermissions",
                "--",
                "tail",
            ]),
        )
        .unwrap();
        assert_eq!(
            p.custom_argv,
            Some(argv(&["/synthetic/tool path", "--base", "one"]))
        );
        assert_eq!(p.executable_candidates, argv(&["/synthetic/tool path"]));
        assert_eq!(p.provider_args, argv(&["tail"]));
        assert_eq!(p.model, "m");
        assert_eq!(p.effort, "");
        assert_eq!(p.permission_mode, "");
        assert_eq!(p.display, "Synthetic Provider");
        assert_eq!(p.diagnostics.len(), 2);
    }
    #[test]
    fn custom_definition_explicit_model_and_effort_mappings_are_used() {
        let mut cfg = Config::default();
        cfg.custom_providers.push(config::CustomProvider {
            id: "synthetic".into(),
            command: "ignored-command".into(),
            ..Default::default()
        });
        let mut def = EffectiveDefinition::default();
        def.definition.id = "synthetic".into();
        def.definition.launch = Some(crate::proto::provider::LaunchDefinition {
            executable: "declared-command".into(),
            args: argv(&["base"]),
            model_args: argv(&["--engine", "{model}"]),
            effort_args: argv(&["--budget={effort}"]),
            effort_levels: argv(&["high"]),
            ..Default::default()
        });
        let p = prepare_launch_with_definition(
            &cfg,
            "synthetic",
            parse(&["--model=m", "--effort=high"]),
            Some(&def),
        )
        .unwrap();
        assert_eq!(p.provider_args, argv(&["--engine", "m", "--budget=high"]));
        assert_eq!(p.custom_argv, Some(argv(&["declared-command", "base"])));
        assert_eq!(p.effort, "high");
    }
    #[test]
    fn login_mode_replaces_tail_and_rejects_conflicts() {
        for (provider, expected) in [
            ("claude", argv(&["auth", "login"])),
            ("codex", argv(&["login"])),
            ("grok", argv(&["login"])),
            ("opencode", argv(&["providers", "login"])),
        ] {
            let p = prepare_launch(
                &Config::default(),
                provider,
                parse(&[
                    "--subscription-login",
                    "--model=m",
                    "--effort=high",
                    "--permission-mode=auto",
                    "--",
                    "discard",
                ]),
            )
            .unwrap();
            assert_eq!(p.provider_args, expected);
            assert!(p.login_mode);
            assert!(p.effort.is_empty());
            assert!(p.permission_mode.is_empty());
            assert!(p.agent_session_id.is_empty());
            assert!(p.required_hooks.is_empty());
        }
        assert!(
            prepare_launch(
                &Config::default(),
                "claude",
                parse(&["--subscription-login", "--headless"])
            )
            .is_err()
        );
        assert!(
            prepare_launch(
                &Config::default(),
                "claude",
                parse(&["--subscription-login", "--prompt-file=p"])
            )
            .is_err()
        );
        assert!(
            prepare_launch(
                &Config::default(),
                "copilot",
                parse(&["--subscription-login"])
            )
            .is_err()
        );
    }
    #[test]
    fn permission_flags_and_declarations_are_one_contract() {
        for (provider, mode, expected) in [
            ("claude", "dontAsk", argv(&["--permission-mode", "dontAsk"])),
            ("grok", "auto", argv(&["--permission-mode", "auto"])),
            ("copilot", "bounded", argv(&["--no-ask-user"])),
            ("copilot", "bypassPermissions", argv(&["--allow-all"])),
            ("cursor-agent", "auto", argv(&["--auto-review"])),
            ("opencode", "bounded", argv(&["--auto"])),
            ("command-code", "plan", argv(&["--permission-mode", "plan"])),
            ("command-code", "auto", argv(&["--auto-accept"])),
            ("command-code", "bypassPermissions", argv(&["--yolo"])),
        ] {
            assert_eq!(
                permission_args_for_provider(provider, mode),
                (expected, mode.into())
            );
        }
        for provider in [
            "claude",
            "grok",
            "codex",
            "cursor-agent",
            "command-code",
            "shell",
            "synthetic",
        ] {
            assert_eq!(
                permission_args_for_provider(provider, "bounded"),
                (vec![], String::new())
            );
        }
        let values = parse_allowed_tool_values("Read, Bash(git log *) ,--oops,rm -rf / ; echo x,");
        assert_eq!(values, argv(&["Read", "Bash(git log *)"]));
        assert_eq!(
            allowed_tool_args("claude", &values),
            argv(&["--allowedTools", "Read", "Bash(git log *)"])
        );
        assert_eq!(
            allowed_tool_args("copilot", &values),
            argv(&["--allow-tool=Read", "--allow-tool=Bash(git log *)"])
        );
        let p = prepare_launch(
            &Config::default(),
            "opencode",
            parse(&["--permission-mode=bounded"]),
        )
        .unwrap();
        assert_eq!(
            p.required_hooks,
            vec![LaunchHook::OpenCodePermissions {
                permission: "allow".into(),
                bash_deny: argv(&["git push*", "git reset*", "git clean*", "rm*", "sudo*"])
            }]
        );
    }
    #[test]
    fn claude_fresh_uuid_and_all_resume_variants() {
        let (args, id) = prepare_claude_session_args("claude", argv(&["--verbose"])).unwrap();
        assert_eq!(
            args,
            vec!["--session-id".to_owned(), id.clone(), "--verbose".into()]
        );
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
        assert!(matches!(&id[19..20], "8" | "9" | "a" | "b"));
        for flag in [
            "--session-id",
            "--session-id=old",
            "--resume",
            "--resume=old",
            "-r",
            "--continue",
            "--fork-session",
        ] {
            let original = argv(&[flag, "old"]);
            let (prepared, id) = prepare_claude_session_args("claude", original.clone()).unwrap();
            assert_eq!(prepared, original);
            assert!(id.is_empty());
        }
        let (args, id) = prepare_claude_session_args("codex", argv(&["tail"])).unwrap();
        assert_eq!(args, argv(&["tail"]));
        assert!(id.is_empty());
    }
    #[test]
    fn color_precedence_and_case_insensitive_spawn_proof_removal() {
        let proof = crate::proto::core::SPAWN_PROOF_ENV;
        let base = vec![
            "PATH=/synthetic".into(),
            "no_color=".into(),
            "FORCE_COLOR=0".into(),
            "clicolor_force=1".into(),
            format!("{proof}=synthetic"),
            format!("{}=synthetic", proof.to_ascii_lowercase()),
        ];
        let extra = vec![
            format!("{proof}=must-not-survive"),
            "TERM=caller-override".into(),
        ];
        for mode in ["force", "inherit", "off", "invalid"] {
            let env = child_env(&base, mode, &extra);
            assert!(
                !env.iter()
                    .any(|v| v.to_ascii_uppercase().starts_with(&format!("{proof}=")))
            );
            assert_eq!(env.last().unwrap(), "TERM=caller-override");
            assert!(env.contains(&"MANY_AI_CLI=1".into()));
            if matches!(mode, "force" | "invalid") {
                assert!(!env.iter().any(|v| v.eq_ignore_ascii_case("NO_COLOR=")));
                assert!(env.contains(&"FORCE_COLOR=3".into()));
            }
            if mode == "inherit" {
                assert!(env.contains(&"no_color=".into()));
                assert!(!env.contains(&"FORCE_COLOR=3".into()));
            }
            if mode == "off" {
                assert!(
                    !env.iter()
                        .any(|v| v.eq_ignore_ascii_case("CLICOLOR_FORCE=1"))
                );
                assert!(env.contains(&"NO_COLOR=1".into()));
            }
        }
        assert!(delegation_prompt_enabled(
            false,
            &argv(&["MANY_AI_CLI_DELEGATION=1"])
        ));
        assert!(!delegation_prompt_enabled(
            true,
            &argv(&["MANY_AI_CLI_DELEGATION=0"])
        ));
        assert!(delegation_prompt_enabled(
            true,
            &argv(&["MANY_AI_CLI_DELEGATION=other"])
        ));
    }
    #[test]
    fn prompt_consumption_expansion_argument_order_and_pointer_cleanup() {
        let (_root, _installed, paths) = trial();
        let source = paths.root().join("prompt.md");
        std::fs::write(&source, "report {{many-ai-cli:session-id}}\nsecond line").unwrap();
        let p = prepare_launch_prompt(&paths, &source, 42, None, 77).unwrap();
        assert!(!source.exists());
        assert!(!p.pointer);
        assert_eq!(p.arg, "report 42\nsecond line");
        assert_eq!(
            with_launch_prompt(
                &argv(&["--allowedTools", "Read", "--settings", "p"]),
                &p.arg
            )[0],
            p.arg
        );
        let body = "a".repeat(LAUNCH_PROMPT_MAX_ARG_COST);
        std::fs::write(&source, &body).unwrap();
        let mut p = prepare_launch_prompt(&paths, &source, 42, None, 77).unwrap();
        let path = p.path.clone().unwrap();
        assert!(p.pointer);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), body);
        assert_eq!(
            launch_prompt_file_owner(path.file_name().unwrap().to_str().unwrap()),
            Some(77)
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        p.cleanup().unwrap();
        p.cleanup().unwrap();
        assert!(!path.exists());
        std::fs::write(&source, "short\nsecond line").unwrap();
        let p =
            prepare_launch_prompt(&paths, &source, 42, Some(r"C:\bin\synthetic.cmd"), 77).unwrap();
        let path = p.path.clone().unwrap();
        assert!(p.pointer);
        drop(p);
        assert!(!path.exists());
    }
    #[test]
    fn prompt_cost_subcommand_guard_and_shell_safety() {
        assert_eq!(launch_prompt_arg_cost("😀\"\\"), 8);
        for value in ["--help me", "update", "resume", "login"] {
            assert_eq!(launch_prompt_arg(value), format!(" {value}"));
        }
        assert_eq!(launch_prompt_arg("update the readme"), "update the readme");
        assert_eq!(launch_prompt_arg("word\u{a0}word"), "word\u{a0}word");
        assert!(
            launch_prompt_arg_usable(Some("C:\\has space\\x.cmd"), Path::new("/tmp/plain"))
                .is_err()
        );
        assert!(
            launch_prompt_arg_usable(Some("C:\\plain\\x.cmd"), Path::new("/tmp/100%")).is_err()
        );
        assert!(launch_prompt_arg_usable(None, Path::new("/tmp/100%")).is_ok());
        assert_eq!(launch_prompt_file_owner("launch-+42-any.md"), Some(42));
        for name in [
            "launch-0-a.md",
            "launch--1-a.md",
            "launch-1.md",
            "launch-1-a.txt",
            "other-1-a.md",
        ] {
            assert!(launch_prompt_file_owner(name).is_none());
        }
    }
    #[test]
    fn trial_prompt_outside_root_is_neither_read_nor_deleted() {
        let (_root, installed, paths) = trial();
        let source = installed.path().join("prompt.md");
        std::fs::write(&source, "synthetic").unwrap();
        assert!(take_prompt_file(&paths, &source).is_err());
        assert!(source.exists());
    }
    #[cfg(unix)]
    #[test]
    fn trial_prompt_symlink_does_not_read_external_target() {
        let (_root, installed, paths) = trial();
        let target = installed.path().join("prompt.md");
        std::fs::write(&target, "synthetic").unwrap();
        let source = paths.root().join("prompt.md");
        std::os::unix::fs::symlink(&target, &source).unwrap();
        assert!(take_prompt_file(&paths, &source).is_err());
        assert!(target.exists());
        assert!(!source.exists());
    }
}
