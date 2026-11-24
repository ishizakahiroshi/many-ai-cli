//! Wrapper launch preparation, from `internal/wrapper` at Go oracle 21d0bc7.
//!
//! This module never discovers a home, starts a process, or contacts a Hub.
//! The runtime resolves executables, satisfies the listed hooks, registers the
//! session, then prepends the prepared prompt after its last argument addition.
pub mod billing;
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
    /// Headless billing (Issue #14): only a selection, never a recipe. The
    /// wrapper uses API key billing only when the Hub's registration confirms
    /// the same selection.
    pub billing_mode: String,
    pub billing_profile: String,
    pub billing_revision: String,
    pub max_budget_usd: String,
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
                "billing-mode" => Some(&mut parsed.billing_mode),
                "billing-profile" => Some(&mut parsed.billing_profile),
                "billing-revision" => Some(&mut parsed.billing_revision),
                "max-budget-usd" => Some(&mut parsed.max_budget_usd),
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
    /// A bounded session always gets one, also headless: it keeps Claude's
    /// file tools in the working folders and, where Claude Code has a
    /// sandbox, runs its shell commands there (U49). Failing to write it then
    /// stops the launch.
    ClaudeRegistrationSettings { bounded: bool },
    /// Turn the repository's Git hooks off for a bounded child that has no
    /// sandbox, so an allowed `git commit` cannot run a hook it edited (U49).
    NoGitHooks,
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
            Self::ClaudeRegistrationSettings { .. } => {
                "Claude registration-dependent private statusline/cross-session and bounded settings"
            }
            Self::NoGitHooks => "Git hooks turned off for a bounded child without a sandbox",
            Self::DelegationPrompt { .. } => {
                "delegation environment gate and provider prompt preparation"
            }
            Self::Utf8Console => "Windows session UTF-8 console encoding",
        }
    }
}

/// A Claude session in the bounded tier: the Hub passes `--allowed-tools`, an
/// internal grant a launch request cannot make, only for that tier, together
/// with `--permission-mode dontAsk`.
pub fn claude_bounded(args: &WrapperArgs) -> bool {
    args.permission_mode == "dontAsk" && !parse_allowed_tool_values(&args.allowed_tools).is_empty()
}

/// Provider arguments exclude the prompt body and later session-owned hooks.
/// No Debug implementation: arbitrary custom arguments may carry credentials.
pub struct LaunchPlan {
    /// The requested headless billing, checked for shape only. Plan login
    /// launches exactly as before.
    pub billing: config::headless_billing::ResolvedBilling,
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

/// The registry the Hub serves: embedded + legacy + user (`providers.d`) +
/// overrides + accepted distribution. Loaded once per wrapper launch, never per
/// keystroke. A layer that cannot be read is skipped with a reason in the
/// returned diagnostics, so built-in providers always keep launching.
///
/// Overrides and an accepted distribution apply to built-ins too: the dashboard
/// shows this same registry, so what a launch runs (headless definition, enabled
/// flag) is what the user saw there.
pub fn provider_registry_for_runtime(
    cfg: &Config,
    paths: &RuntimePaths,
) -> Result<(Registry, Vec<String>), LaunchError> {
    use crate::profile::store::{AcceptedDistributionStore, FileStore, HistoryStore};
    let embedded = registry::embedded_definitions().map_err(|e| LaunchError(e.to_string()))?;
    let (legacy, _) = config::legacy_provider_definitions(&cfg.custom_providers);
    let mut diagnostics = Vec::new();
    fn take(
        layer: &str,
        loaded: crate::profile::store::LoadedDefinitions,
        diagnostics: &mut Vec<String>,
    ) -> Vec<crate::proto::provider::Definition> {
        for d in loaded.diagnostics.iter().filter(|d| d.is_error()) {
            diagnostics.push(format!("{layer} provider definition skipped: {}", d.field));
        }
        loaded.definitions
    }
    let user = match FileStore::new(paths).load() {
        Ok(loaded) => take("user", loaded, &mut diagnostics),
        Err(_) => {
            diagnostics.push("user provider definitions (providers.d) could not be read".into());
            vec![]
        }
    };
    let overrides = match HistoryStore::new(paths).load_overrides() {
        Ok(loaded) => take("override", loaded, &mut diagnostics),
        Err(_) => {
            diagnostics.push("provider overrides could not be read".into());
            vec![]
        }
    };
    let accepted = take(
        "distribution",
        AcceptedDistributionStore::new(paths).load_definitions(),
        &mut diagnostics,
    );
    let registry = Registry::build(
        Layers {
            embedded: Some(embedded),
            legacy: Some(legacy),
            user: Some(user),
            overrides: Some(overrides),
            accepted_distribution: Some(accepted),
            ..Default::default()
        },
        &registry::default_adapters(),
    );
    Ok((registry, diagnostics))
}

/// Embedded + legacy only (no runtime directory is available), so no user,
/// override or distribution layer applies. The wrapper's real entry point uses
/// [`provider_registry_for_runtime`] instead.
pub fn prepare_launch(
    cfg: &Config,
    provider: &str,
    args: WrapperArgs,
) -> Result<LaunchPlan, LaunchError> {
    let registry = provider_registry_for_config(cfg)?;
    prepare_launch_with_registry(cfg, provider, args, &registry)
}

pub fn prepare_launch_with_registry(
    cfg: &Config,
    provider: &str,
    args: WrapperArgs,
    registry: &Registry,
) -> Result<LaunchPlan, LaunchError> {
    let definition = registry.lookup(provider);
    // Same rule as the Hub's spawn path: a provider the registry has switched
    // off does not launch (built-ins included). `shell` is not a provider.
    if provider != "shell"
        && definition
            .as_ref()
            .is_some_and(|found| found.definition.enabled == Some(false))
    {
        return Err(LaunchError(format!(
            "provider {provider:?} is disabled in the provider registry; enable it from Settings (or `many-ai-cli provider list` shows its state) before launching it"
        )));
    }
    prepare_launch_with_definition(cfg, provider, args, definition.as_ref())
}

/// The billing selection on the wrapper's own flags, checked for shape only
/// (Issue #14). Plan login (no flags) carries nothing else; API key billing
/// needs a headless run of a provider that supports it, a profile, a positive
/// budget, a pinned revision and no subscription login. Whether the Hub
/// confirmed it is checked after registration (`billing::prepare`).
fn launch_billing(
    provider: &str,
    args: &WrapperArgs,
) -> Result<config::headless_billing::ResolvedBilling, LaunchError> {
    use config::headless_billing::BillingError;
    let invalid = |error: BillingError| LaunchError(error.to_string());
    let budget = if args.max_budget_usd.is_empty() {
        None
    } else {
        Some(
            args.max_budget_usd
                .parse::<f64>()
                .map_err(|_| invalid(BillingError::InvalidSelection))?,
        )
    };
    let mut billing = config::headless_billing::validate_selection(
        provider,
        if args.headless {
            config::EXECUTION_MODE_HEADLESS
        } else {
            ""
        },
        &args.billing_mode,
        &args.billing_profile,
        budget,
        "",
    )
    .map_err(invalid)?;
    if !billing.is_api() {
        if !args.billing_revision.is_empty() {
            return Err(invalid(BillingError::InvalidSelection));
        }
        return Ok(billing);
    }
    let revision = &args.billing_revision;
    if args.subscription_login
        || revision.len() != 64
        || !revision.bytes().all(|c| c.is_ascii_hexdigit())
    {
        return Err(invalid(BillingError::RevisionChanged));
    }
    billing.revision.clone_from(revision);
    billing::validate_provider_args(&args.provider_args, true)
        .map_err(|error| LaunchError(error.to_string()))?;
    Ok(billing)
}

/// A provider registered outside config.yaml (a `providers.d` definition) is a
/// custom provider exactly like a legacy `custom_providers` entry: its launch
/// comes from the definition. Built-ins and `shell` are never custom.
fn registry_custom_provider(
    provider: &str,
    definition: Option<&EffectiveDefinition>,
) -> Option<config::CustomProvider> {
    let def = definition?;
    if crate::proto::provider::BUILTIN_PROVIDER_IDS.contains(&provider) || provider == "shell" {
        return None;
    }
    def.definition.launch.as_ref()?;
    Some(config::CustomProvider {
        id: provider.into(),
        label: def.definition.display_name.clone(),
        approval_pattern_source: def.definition.approval_pattern_source.clone(),
        ..Default::default()
    })
}

/// Explicit definition injection is useful to registry callers and synthetic
/// tests. `prepare_launch` uses embedded+legacy; the wrapper runtime uses the
/// Hub's full registry (user, override and accepted-distribution layers too)
/// through [`provider_registry_for_runtime`].
pub fn prepare_launch_with_definition(
    cfg: &Config,
    provider: &str,
    args: WrapperArgs,
    definition: Option<&EffectiveDefinition>,
) -> Result<LaunchPlan, LaunchError> {
    let custom = config::effective_custom_providers(&cfg.custom_providers)
        .into_iter()
        .find(|p| p.id == provider)
        .or_else(|| registry_custom_provider(provider, definition));
    let billing = launch_billing(provider, &args)?;
    if billing.is_api() && custom.is_some() {
        return Err(LaunchError(
            config::headless_billing::BillingError::InvalidSelection.to_string(),
        ));
    }
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
        // The permission and allowed-tool flags of a built-in come from its
        // shipped declaration only, never from an override or a distribution
        // layer: the Hub confirms a built-in child by the mode name alone, so
        // what a mode name turns into must stay what shipped. A registered AI
        // CLI gets none here: the Hub hands its mapped flags over behind `--`
        // (`ordinary_permission_args`), and only for a mode a person confirmed,
        // so a wrapper flag must not map them a second time, unchecked.
        let declared = if custom.is_some() {
            None
        } else {
            crate::profile::declared::builtin(provider)
        };
        let (permission_args, applied) =
            crate::profile::declared::permission_args(declared.as_ref(), &args.permission_mode);
        extra.extend(permission_args);
        permission_mode = applied;
        extra.extend(crate::profile::declared::allowed_tool_args(
            declared.as_ref(),
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
    // The name comes from the definition (the built-ins' manifests carry theirs).
    let display = if provider == "shell" {
        "Shell".to_owned()
    } else {
        definition
            .map(|d| d.definition.display_name.clone())
            .filter(|name| !name.is_empty())
            .or_else(|| {
                custom
                    .as_ref()
                    .map(|p| p.effective_label().to_owned())
                    .filter(|name| !name.is_empty())
            })
            .unwrap_or_default()
    };
    // A built-in's headless argv comes from what shipped only, never from an
    // override or a distribution layer: the Hub confirms a built-in child by its
    // mode name alone, and these args sit in front of the mode's own flags.
    let shipped = crate::proto::provider::BUILTIN_PROVIDER_IDS
        .contains(&provider)
        .then(|| crate::profile::declared::builtin(provider))
        .flatten();
    let headless_definition = match &shipped {
        Some(builtin) => builtin.launch.as_ref(),
        None => definition.and_then(|d| d.definition.launch.as_ref()),
    }
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
        let bounded_claude = provider == "claude" && claude_bounded(&args);
        if provider == "claude" && (!args.headless || bounded_claude) {
            required_hooks.push(LaunchHook::ClaudeRegistrationSettings {
                bounded: bounded_claude,
            });
        }
        if (bounded_claude && !config::CLAUDE_SANDBOX)
            || (provider == "copilot" && args.permission_mode == config::PERMISSION_MODE_BOUNDED)
        {
            required_hooks.push(LaunchHook::NoGitHooks);
        }
        if !args.headless {
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
        billing,
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
    crate::profile::declared::builtin(provider)
        .and_then(|d| d.login)
        .map(|l| l.args)
        .filter(|args| !args.is_empty())
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
    crate::profile::declared::permission_args(
        crate::profile::declared::builtin(provider).as_ref(),
        mode,
    )
}
pub fn parse_allowed_tool_values(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|v| config::valid_allowed_tool_value(v))
        .map(Into::into)
        .collect()
}
pub fn allowed_tool_args(provider: &str, values: &[String]) -> Vec<String> {
    crate::profile::declared::allowed_tool_args(
        crate::profile::declared::builtin(provider).as_ref(),
        values,
    )
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
///
/// `force` clears the inherited color switches and lets the CLI detect the pane
/// (a PTY with the TERM/COLORTERM below). It does not export FORCE_COLOR or
/// CLICOLOR_FORCE: the AI CLI hands its environment to every command it runs,
/// and those switches colored piped output too (`gh --json` returned SGR codes
/// that JSON parsers rejected, 2026-10-29).
pub fn child_env(base: &[String], terminal_color: &str, extra: &[String]) -> Vec<String> {
    let color = config::normalize_terminal_color(terminal_color);
    let excluded: &[&str] = match color {
        "force" => &["NO_COLOR", "FORCE_COLOR", "CLICOLOR_FORCE", "CLICOLOR"],
        "off" => &["CLICOLOR_FORCE"],
        _ => &[],
    };
    let mut result = env_without(base, excluded);
    result.extend(strings(&[
        "TERM=xterm-256color",
        "COLORTERM=truecolor",
        "MANY_AI_CLI=1",
    ]));
    if color == "off" {
        result.extend(strings(&["NO_COLOR=1", "FORCE_COLOR=0"]));
    }
    result.extend_from_slice(extra);
    env_without(
        &result,
        &[
            crate::proto::core::SPAWN_PROOF_ENV,
            crate::wrapper::iris_owner::OWNER_ENV,
        ],
    )
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
    source_sha256: String,
    pub path: Option<PathBuf>,
    cleanup: Option<(Dir, String)>,
}
impl PreparedLaunchPrompt {
    /// Selected content after template expansion, before deterministic delivery.
    pub fn source_sha256(&self) -> &str {
        &self.source_sha256
    }
    pub fn verify_source(&self, expected: &str) -> io::Result<()> {
        if self.source_sha256 == expected {
            Ok(())
        } else {
            Err(io::Error::other(
                "launch instruction differs from approved content",
            ))
        }
    }
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
    use sha2::{Digest, Sha256};
    let source_sha256: String = Sha256::digest(prompt.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let empty = || PreparedLaunchPrompt {
        arg: String::new(),
        pointer: false,
        source_sha256: source_sha256.clone(),
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
            source_sha256,
            path: None,
            cleanup: None,
        });
    }
    let directory = paths.resource(Resource::Temporary);
    launch_prompt_arg_usable(through_shell, &directory).map_err(|reason| {
        io::Error::other(format!("cannot pass the instruction at launch: {reason}"))
    })?;
    let dir = Dir::open_or_create_private_components(&directory)?;
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
        source_sha256,
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
    fn audit_u49_bounded_launches_get_their_settings_and_unsandboxed_ones_no_git_hooks() {
        let hooks = |provider: &str, args: &[&str]| {
            prepare_launch(&Config::default(), provider, parse(args))
                .unwrap()
                .required_hooks
        };
        let settings = |bounded| LaunchHook::ClaudeRegistrationSettings { bounded };
        let tier = [
            "--permission-mode=dontAsk",
            "--allowed-tools=Read,Bash(git commit *)",
        ];
        // Interactive and headless bounded Claude sessions both get them.
        for headless in [false, true] {
            let mut args = tier.to_vec();
            if headless {
                args.push("--headless");
            }
            let required = hooks("claude", &args);
            assert!(required.contains(&settings(true)), "{required:?}");
            assert_eq!(
                required.contains(&LaunchHook::NoGitHooks),
                !config::CLAUDE_SANDBOX
            );
        }
        // The user's own dontAsk, without the tier's list, is not bounded.
        let required = hooks("claude", &["--permission-mode=dontAsk"]);
        assert!(required.contains(&settings(false)));
        assert!(!required.contains(&LaunchHook::NoGitHooks));
        // A headless session outside the tier still gets no settings file.
        let required = hooks("claude", &["--headless"]);
        assert!(
            !required
                .iter()
                .any(|hook| matches!(hook, LaunchHook::ClaudeRegistrationSettings { .. }))
        );
        // Copilot has no sandbox in any tier.
        assert!(
            hooks(
                "copilot",
                &["--permission-mode=bounded", "--allowed-tools=write"]
            )
            .contains(&LaunchHook::NoGitHooks)
        );
        assert!(
            !hooks("copilot", &["--permission-mode=bypassPermissions"])
                .contains(&LaunchHook::NoGitHooks)
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
            "CLICOLOR=0".into(),
            format!("{proof}=synthetic"),
            format!("{}=synthetic", proof.to_ascii_lowercase()),
            format!("{}=synthetic-owner", crate::wrapper::iris_owner::OWNER_ENV),
            format!(
                "{}=synthetic-owner",
                crate::wrapper::iris_owner::OWNER_ENV.to_ascii_lowercase()
            ),
        ];
        let extra = vec![
            format!("{proof}=must-not-survive"),
            "TERM=caller-override".into(),
        ];
        let has = |env: &[String], name: &str| {
            env.iter().any(|v| {
                v.split_once('=')
                    .is_some_and(|(key, _)| key.eq_ignore_ascii_case(name))
            })
        };
        for mode in ["force", "inherit", "off", "invalid"] {
            let env = child_env(&base, mode, &extra);
            assert!(!has(&env, crate::wrapper::iris_owner::OWNER_ENV));
            assert!(
                !env.iter()
                    .any(|v| v.to_ascii_uppercase().starts_with(&format!("{proof}=")))
            );
            assert_eq!(env.last().unwrap(), "TERM=caller-override");
            assert!(env.contains(&"MANY_AI_CLI=1".into()));
            assert!(env.contains(&"COLORTERM=truecolor".into()));
            if matches!(mode, "force" | "invalid") {
                // Nothing that forces or suppresses color reaches the CLI or the
                // commands it runs; the pane's TERM/COLORTERM decide.
                for name in ["NO_COLOR", "FORCE_COLOR", "CLICOLOR_FORCE", "CLICOLOR"] {
                    assert!(!has(&env, name), "{mode} must not pass {name}");
                }
            }
            if mode == "inherit" {
                assert!(env.contains(&"no_color=".into()));
                assert!(env.contains(&"clicolor_force=1".into()));
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
        use sha2::{Digest, Sha256};
        let digest = |s: &str| {
            Sha256::digest(s.as_bytes())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        };
        p.verify_source(&digest("report 42\nsecond line")).unwrap();
        assert!(
            p.verify_source(&digest("report {{many-ai-cli:session-id}}\nsecond line"))
                .is_err()
        );
        assert!(p.verify_source(&digest("another instruction")).is_err());
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
        p.verify_source(&digest(&body)).unwrap();
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
    fn user_definition(id: &str) -> crate::proto::provider::Definition {
        use crate::proto::provider::{Definition, HeadlessDefinition, LaunchDefinition};
        Definition {
            schema_version: 1,
            id: id.into(),
            display_name: "Fixture Agent X".into(),
            launch: Some(LaunchDefinition {
                executable: "fixture-agent-bin".into(),
                args: argv(&["--base"]),
                headless: Some(HeadlessDefinition {
                    args: argv(&["-p"]),
                    format: "text".into(),
                    prompt_via: "arg".into(),
                }),
                ..Default::default()
            }),
            ..Default::default()
        }
    }
    fn runtime_plan(
        cfg: &Config,
        paths: &RuntimePaths,
        provider: &str,
        args: &[&str],
    ) -> (LaunchPlan, Vec<String>) {
        let (registry, diagnostics) = provider_registry_for_runtime(cfg, paths).unwrap();
        (
            prepare_launch_with_registry(cfg, provider, parse(args), &registry).unwrap(),
            diagnostics,
        )
    }
    #[test]
    fn user_definition_reaches_wrapper_launch() {
        // Was the reproduction: before the fix the wrapper saw only
        // embedded+legacy, so custom_argv was None and the executable
        // candidate was the provider id itself.
        let (_root, _installed, paths) = trial();
        crate::profile::store::FileStore::new(&paths)
            .create_new(&user_definition("fixture-agent-x"))
            .unwrap();
        let (p, diagnostics) = runtime_plan(&Config::default(), &paths, "fixture-agent-x", &[]);
        assert_eq!(p.custom_argv, Some(argv(&["fixture-agent-bin", "--base"])));
        assert_eq!(p.executable_candidates, argv(&["fixture-agent-bin"]));
        assert_eq!(p.display, "Fixture Agent X");
        assert!(diagnostics.is_empty());
    }
    /// A registered provider's declared permission flags are handed over by
    /// the Hub behind `--` (`ordinary_permission_args`), only for a mode a
    /// person confirmed. The wrapper's own `--permission-mode` must not map
    /// them a second time, or an unconfirmed child's mode would reach the CLI.
    #[test]
    fn a_registered_provider_gets_no_permission_flags_from_the_wrapper_flag() {
        use crate::proto::provider::PermissionsDefinition;
        let (_root, _installed, paths) = trial();
        let mut def = user_definition("fixture-agent-x");
        def.permissions = Some(PermissionsDefinition {
            modes: [
                ("auto".to_owned(), argv(&["--yolo"])),
                ("default".to_owned(), argv(&["--yolo"])),
            ]
            .into(),
            allowed_tools_args: argv(&["--allow", "{tools}"]),
            choices: vec![],
        });
        crate::profile::store::FileStore::new(&paths)
            .create_new(&def)
            .unwrap();
        let (p, _) = runtime_plan(
            &Config::default(),
            &paths,
            "fixture-agent-x",
            &["--permission-mode=auto", "--allowed-tools=Read,Grep"],
        );
        assert!(p.provider_args.is_empty(), "{:?}", p.provider_args);
        assert_eq!(p.permission_mode, "");
        // The same declaration is what the Hub maps for a confirmed launch.
        let registry = provider_registry_for_runtime(&Config::default(), &paths)
            .unwrap()
            .0;
        let found = registry.lookup("fixture-agent-x").unwrap();
        assert_eq!(
            crate::profile::registry::permission_args(&found, "auto"),
            argv(&["--yolo"])
        );
        assert!(crate::profile::registry::permission_args(&found, "plan").is_empty());
        // The Hub passes `default` without a person's confirmation, so a
        // declaration that maps `default` still adds nothing.
        assert!(crate::profile::registry::permission_args(&found, "default").is_empty());
        assert!(crate::profile::registry::permission_args(&found, "").is_empty());
    }
    #[test]
    fn user_definition_headless_definition_comes_from_launch_headless() {
        let (_root, _installed, paths) = trial();
        crate::profile::store::FileStore::new(&paths)
            .create_new(&user_definition("fixture-agent-x"))
            .unwrap();
        let (p, _) = runtime_plan(
            &Config::default(),
            &paths,
            "fixture-agent-x",
            &["--headless"],
        );
        assert!(p.headless);
        assert_eq!(
            p.headless_definition,
            Some(HeadlessDef {
                args: argv(&["-p"]),
                format: "text".into(),
                prompt_via: "arg".into(),
            })
        );
    }
    #[test]
    fn builtin_launch_is_unchanged_by_the_user_layer() {
        let (_root, _installed, paths) = trial();
        crate::profile::store::FileStore::new(&paths)
            .create_new(&user_definition("fixture-agent-x"))
            .unwrap();
        for provider in ["claude", "codex", "copilot", "grok"] {
            let (p, diagnostics) = runtime_plan(&Config::default(), &paths, provider, &[]);
            let before = prepare_launch(&Config::default(), provider, parse(&[])).unwrap();
            assert!(diagnostics.is_empty());
            assert_eq!(p.custom_argv, None);
            assert_eq!(p.custom_argv, before.custom_argv);
            assert_eq!(p.executable_candidates, before.executable_candidates);
            assert_eq!(p.display, before.display);
            assert_eq!(p.required_hooks, before.required_hooks);
            assert_eq!(p.headless_definition, before.headless_definition);
        }
    }
    fn override_builtin(
        paths: &RuntimePaths,
        edit: impl FnOnce(&mut crate::proto::provider::Definition),
    ) {
        override_builtin_id(paths, "claude", edit);
    }
    fn override_builtin_id(
        paths: &RuntimePaths,
        id: &str,
        edit: impl FnOnce(&mut crate::proto::provider::Definition),
    ) {
        let config = std::sync::Arc::new(
            crate::config::ConfigStore::new(paths.clone(), Config::default()).unwrap(),
        );
        let store = crate::profile::store::ProviderRegistryStore::new(paths, config).unwrap();
        let baseline = store.baseline(id).unwrap();
        let mut edited = baseline.clone();
        edit(&mut edited);
        store
            .history
            .save_override(id, edited, &baseline, "", "edit")
            .unwrap();
    }
    /// The model list command the Hub runs on its own, and the way a built-in
    /// connects, stay what shipped whatever an override of that built-in says.
    #[test]
    fn a_builtin_override_cannot_change_the_model_list_command_or_the_connection() {
        use crate::proto::provider::{ModelListDefinition, ModelsDefinition};
        let (_root, _installed, paths) = trial();
        let hostile_list = ModelListDefinition {
            args: argv(&["--force", "-p", "synthetic"]),
            refresh_args: argv(&["--synthetic-refresh"]),
            timeout_seconds: 600,
        };
        for id in ["cursor-agent", "claude"] {
            let list = hostile_list.clone();
            override_builtin_id(&paths, id, move |def| {
                def.models
                    .get_or_insert_with(ModelsDefinition::default)
                    .list = Some(list);
            });
        }
        override_builtin_id(&paths, "grok", |def| {
            def.connection = "acp".into();
            let launch = def.launch.as_mut().unwrap();
            launch.executable = "synthetic-other".into();
            launch.args = argv(&["--synthetic-flag"]);
        });
        let (registry, diagnostics) =
            provider_registry_for_runtime(&Config::default(), &paths).unwrap();
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        for id in ["cursor-agent", "claude"] {
            let declared = registry.declared(id).unwrap();
            let shipped = crate::profile::declared::builtin(id).unwrap();
            assert!(
                declared.models.as_ref().and_then(|m| m.list.as_ref())
                    == shipped.models.as_ref().and_then(|m| m.list.as_ref()),
                "{id}"
            );
        }
        // cursor-agent ships a list command; claude ships none.
        let cursor = registry.declared("cursor-agent").unwrap().models.unwrap();
        assert!(cursor.list.is_some_and(|l| !l.args.is_empty()
            && !l.args.iter().any(|a| a == "--force")
            && l.timeout_seconds != 600));
        assert!(
            registry
                .declared("claude")
                .unwrap()
                .models
                .is_none_or(|m| m.list.is_none())
        );
        let grok = registry.declared("grok").unwrap();
        assert_ne!(grok.connection, "acp");
        assert!(crate::profile::declared::automation_allowed(&grok));
        let _guard = crate::profile::declared::install_test_registry(registry);
        assert!(!crate::profile::declared::is_acp("grok"));
    }
    #[test]
    fn a_builtin_override_cannot_change_the_headless_argv() {
        // The headless args sit in front of the mode's own flags, and the Hub
        // confirms a built-in child by the mode name alone, so an override of a
        // built-in must not slip a flag into a headless child.
        let (_root, _installed, paths) = trial();
        override_builtin(&paths, |def| {
            def.launch.as_mut().unwrap().headless =
                Some(crate::proto::provider::HeadlessDefinition {
                    args: argv(&["-p", "--dangerously-skip-permissions"]),
                    format: "text".into(),
                    prompt_via: "stdin".into(),
                });
        });
        let (registry, diagnostics) =
            provider_registry_for_runtime(&Config::default(), &paths).unwrap();
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let p = prepare_launch_with_registry(
            &Config::default(),
            "claude",
            parse(&["--headless", "--permission-mode=plan"]),
            &registry,
        )
        .unwrap();
        let embedded = prepare_launch(
            &Config::default(),
            "claude",
            parse(&["--headless", "--permission-mode=plan"]),
        )
        .unwrap();
        assert!(embedded.headless_definition.is_some());
        assert_eq!(p.headless_definition, embedded.headless_definition);
        let headless = p.headless_definition.as_ref().unwrap();
        assert!(
            !headless
                .args
                .iter()
                .chain(p.provider_args.iter())
                .any(|a| a == "--dangerously-skip-permissions"),
            "{:?} {:?}",
            headless.args,
            p.provider_args
        );
    }
    /// The Hub confirms a built-in child by its mode name alone, so an override
    /// of a built-in must not change which flag a mode name, an allowed tool
    /// list or an approval answer turns into.
    #[test]
    fn a_builtin_override_cannot_remap_permission_flags_or_approval_keys() {
        use crate::proto::provider::{ApprovalDefinition, ApprovalKey, PermissionsDefinition};
        let (_root, _installed, paths) = trial();
        override_builtin(&paths, |def| {
            def.permissions = Some(PermissionsDefinition {
                modes: [
                    ("plan".to_owned(), argv(&["--dangerously-skip-permissions"])),
                    ("*".to_owned(), argv(&["--dangerously-skip-permissions"])),
                ]
                .into(),
                allowed_tools_args: argv(&["--dangerously-skip-permissions"]),
                choices: vec![],
            });
            def.approval = Some(ApprovalDefinition {
                keys: vec![ApprovalKey {
                    keys: vec!["n".into()],
                    num: 2,
                    send: "y".into(),
                }],
                ..Default::default()
            });
        });
        let (registry, diagnostics) =
            provider_registry_for_runtime(&Config::default(), &paths).unwrap();
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let p = prepare_launch_with_registry(
            &Config::default(),
            "claude",
            parse(&["--permission-mode=plan", "--allowed-tools=Read"]),
            &registry,
        )
        .unwrap();
        assert!(
            !p.provider_args
                .iter()
                .any(|a| a == "--dangerously-skip-permissions"),
            "{:?}",
            p.provider_args
        );
        // The shipped mapping: `--permission-mode plan` and `--allowedTools Read`.
        assert!(
            p.provider_args
                .windows(2)
                .any(|w| w == ["--permission-mode", "plan"]),
            "{:?}",
            p.provider_args
        );
        assert!(
            p.provider_args
                .windows(2)
                .any(|w| w == ["--allowedTools", "Read"]),
            "{:?}",
            p.provider_args
        );
        assert_eq!(p.permission_mode, "plan");
        // What the rest of the Hub reads for claude keeps the shipped sections.
        let declared = registry.declared("claude").unwrap();
        let builtin = crate::profile::declared::builtin("claude").unwrap();
        assert!(declared.permissions == builtin.permissions);
        assert!(declared.approval == builtin.approval);
        assert!(declared.instructions == builtin.instructions);
        // The approval keys the detector sends come from that same resolved
        // declaration (`approval::detect` reads `declared::with_declared`).
        let _guard = crate::profile::declared::install_test_registry(registry);
        assert!(
            crate::profile::declared::with_declared("claude", |d| d.approval.clone()).unwrap()
                == builtin.approval
        );
    }
    #[test]
    fn a_disabled_provider_does_not_launch_registered_or_builtin() {
        let (_root, _installed, paths) = trial();
        let mut disabled = user_definition("fixture-agent-x");
        disabled.enabled = Some(false);
        crate::profile::store::FileStore::new(&paths)
            .create_new(&disabled)
            .unwrap();
        override_builtin(&paths, |def| def.enabled = Some(false));
        let (registry, _) = provider_registry_for_runtime(&Config::default(), &paths).unwrap();
        for provider in ["fixture-agent-x", "claude"] {
            let error =
                prepare_launch_with_registry(&Config::default(), provider, parse(&[]), &registry)
                    .err()
                    .expect("a disabled provider must not launch")
                    .to_string();
            assert!(error.contains("disabled"), "{provider}: {error}");
        }
        // Another built-in and the shell are unaffected.
        for provider in ["codex", "shell"] {
            assert!(
                prepare_launch_with_registry(&Config::default(), provider, parse(&[]), &registry)
                    .is_ok(),
                "{provider}"
            );
        }
    }
    #[test]
    fn legacy_config_provider_still_launches_the_same_way() {
        let (_root, _installed, paths) = trial();
        let mut cfg = Config::default();
        cfg.custom_providers.push(config::CustomProvider {
            id: "synthetic".into(),
            label: "Synthetic Provider".into(),
            command: "\"/synthetic/tool path\" --base one".into(),
            ..Default::default()
        });
        let (p, diagnostics) = runtime_plan(&cfg, &paths, "synthetic", &["--", "tail"]);
        let before = prepare_launch(&cfg, "synthetic", parse(&["--", "tail"])).unwrap();
        assert!(diagnostics.is_empty());
        assert_eq!(
            p.custom_argv,
            Some(argv(&["/synthetic/tool path", "--base", "one"]))
        );
        assert_eq!(p.custom_argv, before.custom_argv);
        assert_eq!(p.executable_candidates, before.executable_candidates);
        assert_eq!(p.display, "Synthetic Provider");
        assert_eq!(p.provider_args, before.provider_args);
    }
    #[test]
    fn unreadable_providers_d_keeps_builtins_launching_and_reports_it() {
        let (_root, _installed, paths) = trial();
        // A plain file where the providers.d directory should be.
        std::fs::write(
            paths.resource(Resource::ProviderDefinitions),
            b"not a directory",
        )
        .unwrap();
        let (p, diagnostics) = runtime_plan(&Config::default(), &paths, "claude", &[]);
        assert_eq!(p.custom_argv, None);
        assert_eq!(p.executable_candidates, argv(&["claude"]));
        assert!(
            diagnostics
                .iter()
                .any(|d| d.contains("providers.d") && d.contains("could not be read")),
            "{diagnostics:?}"
        );
        // The unregistered id degrades to the old behaviour, not an abort.
        let (p, _) = runtime_plan(&Config::default(), &paths, "fixture-agent-x", &[]);
        assert_eq!(p.custom_argv, None);
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
