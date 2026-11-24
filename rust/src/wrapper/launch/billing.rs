//! API key billing for a Claude Code headless run (Issue #14), checked in the
//! wrapper after the Hub registered the session and before the provider starts.
//!
//! Every check fails closed with a fixed message. Nothing here runs the
//! person's key command (the recipe), reads a key, or formats a path, command,
//! value or provider output into an error. The only process started is
//! `claude --version` (no inference, no credential use).
//!
//! The plan login (the default) gets one narrow step, [`plan_login_override`]:
//! a key the environment or the Claude settings would bill the run to is
//! switched off for that run only (removed from the provider's environment,
//! blanked in the run's own `--settings`). The person's files are not changed.
//! Nothing else about a plan run changes.
use super::LaunchPlan;
use crate::{
    config::{
        RuntimePaths,
        headless_billing::{self, BillingError, PrivateRecipe, ResolvedBilling},
    },
    files::safe_fs::Dir,
    process::{self, Cancellation, ExitOutcome, ProcessPlan, SpawnOptions},
    proto::Message,
};
use std::{
    collections::BTreeSet,
    io,
    path::{Path, PathBuf},
    time::Duration,
};

pub struct BillingContext<'a> {
    pub paths: &'a RuntimePaths,
    pub home_dir: &'a Path,
    pub cwd: &'a Path,
    /// The wrapper's environment: what the provider inherits.
    pub environment: &'a [String],
    pub launch: &'a LaunchPlan,
    pub registered: &'a Message,
}

/// Flags that would read settings, credentials or a session from somewhere
/// these checks did not inspect, run elsewhere, or write raw provider logs.
/// Both `--flag value` and `--flag=value` forms are matched by name.
const REFUSED_FLAGS: &[&str] = &[
    "--settings",
    "--setting-sources",
    "--managed-settings",
    "--project-config-root",
    "--client-data-url",
    "--worktree",
    "--deep-link-cwd-b64",
    "--desktop",
    "--sdk-url",
    "--teleport",
    "--cloud",
    "--environment",
    "--remote",
    "--attach-serve",
    "--remote-control",
    "--rc",
    "--bg",
    "--background",
    "--tmux",
    "--resume",
    "--continue",
    "--fork-session",
    "--from-pr",
    "--api-key",
    "--apiKeyHelper",
    "--max-budget-usd",
    "--bare",
    "--debug",
    "--debug-file",
    "--debug-to-stderr",
    "--session-id",
    "--add-dir",
    "--mcp-config",
    "--plugin-dir",
    "--plugin-url",
];

/// The provider arguments the Hub composed (model, effort, permission and
/// tool flags). Our own `--session-id` is removed by the caller first.
pub fn validate_provider_args(args: &[String], api: bool) -> io::Result<()> {
    if !api {
        return Ok(());
    }
    if args.iter().any(|arg| {
        let name = arg.split('=').next().unwrap_or(arg);
        REFUSED_FLAGS.contains(&name) || conflicting_short_option(arg)
    }) {
        return Err(BillingError::AuthConflict.into());
    }
    Ok(())
}

/// Commander accepts attached short values and clusters of boolean options.
/// Recognize the reviewed short grammar (`-c -d -h -n -p -r -v -w`); stop at a
/// value-taking safe option so a label like `-nwork` is not read as `-w`.
fn conflicting_short_option(arg: &str) -> bool {
    let Some(short) = arg
        .strip_prefix('-')
        .filter(|short| !short.starts_with('-'))
    else {
        return false;
    };
    for option in short.chars() {
        match option {
            'd' | 'w' | 'r' | 'c' => return true,
            'n' => return false,
            'p' | 'v' | 'V' | 'h' => {}
            _ => return false,
        }
    }
    false
}

/// Environment switches that pick another hosted auth source or make the
/// provider write its own debug output. Names only.
fn conflicting_launch_environment(environment: &[String]) -> bool {
    environment
        .iter()
        .filter_map(|entry| entry.split_once('='))
        .any(|(key, _)| {
            matches!(
                key.to_ascii_uppercase().as_str(),
                "CLAUDE_CODE_CLIENT_DATA_URL"
                    | "DEBUG"
                    | "DEBUG_SDK"
                    | "DEBUG_AUTH"
                    | "DEBUG_CLAUDE_AGENT_SDK"
                    | "CLAUDE_DEBUG"
                    | "NODE_DEBUG"
                    | "CLAUDE_CODE_DEBUG_LOG_LEVEL"
                    | "CLAUDE_CODE_DEBUG_LOGS_DIR"
            )
        })
}

fn env<'a>(entries: &'a [String], name: &str) -> Option<&'a str> {
    entries.iter().rev().find_map(|entry| {
        entry
            .split_once('=')
            .filter(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value)
    })
}

fn conflict() -> io::Error {
    BillingError::AuthConflict.into()
}

fn inspect_file(paths: &RuntimePaths, path: &Path, managed: bool) -> io::Result<()> {
    if paths.is_trial() {
        paths
            .relative_to_selected_root(path)
            .map_err(|_| conflict())?;
    }
    let parent = path.parent().ok_or_else(conflict)?;
    let name = path
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or_else(conflict)?;
    let directory = match Dir::open(parent) {
        Ok(dir) => dir,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(conflict()),
    };
    let bytes = match directory.read(name, 128 * 1024) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(conflict()),
    };
    // A managed (policy) file can hold policy outside the public auth schema.
    // Never guess which of those wins over the selected helper.
    if managed {
        return Err(conflict());
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| conflict())?;
    if headless_billing::auth_settings_conflict(&value)
        || value
            .get("env")
            .and_then(serde_json::Value::as_object)
            .is_some_and(|env| {
                // The process's own config root was selected and inspected. A
                // root loaded from settings would move later reads past these
                // checks, so it is refused too.
                env.keys()
                    .any(|key| key.eq_ignore_ascii_case("CLAUDE_CONFIG_DIR"))
                    || conflicting_launch_environment(
                        &env.keys().map(|key| format!("{key}=")).collect::<Vec<_>>(),
                    )
            })
    {
        return Err(conflict());
    }
    Ok(())
}

fn inspect_managed_directory(paths: &RuntimePaths, root: &Path) -> io::Result<()> {
    inspect_file(paths, &root.join("managed-settings.json"), true)?;
    let drop_ins = root.join("managed-settings.d");
    if paths.is_trial() {
        paths
            .relative_to_selected_root(&drop_ins)
            .map_err(|_| conflict())?;
    }
    match Dir::open(&drop_ins) {
        Ok(directory) if directory.entries().map_err(|_| conflict())?.is_empty() => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        _ => Err(conflict()),
    }
}

/// Refuses settings, managed policy and environment that could pick another
/// credential source than the selected helper. Nothing is written or fixed.
pub fn check_auth_files(
    paths: &RuntimePaths,
    home: &Path,
    cwd: &Path,
    environment: &[String],
) -> io::Result<()> {
    if environment
        .iter()
        .any(|entry| entry.starts_with("WSL_DISTRO_NAME=") || entry.starts_with("WSL_INTEROP="))
    {
        // WSL can inherit Windows managed policy that these local checks
        // cannot see.
        return Err(conflict());
    }
    if headless_billing::auth_environment_conflicts(environment)
        || conflicting_launch_environment(environment)
    {
        return Err(conflict());
    }
    let config = match env(environment, "CLAUDE_CONFIG_DIR") {
        Some(path) if Path::new(path).is_absolute() => PathBuf::from(path),
        Some(_) => return Err(conflict()),
        None => home.join(".claude"),
    };
    for name in ["settings.json", "settings.local.json"] {
        inspect_file(paths, &config.join(name), false)?;
    }
    inspect_managed_directory(paths, &config)?;
    for name in [
        "managed-settings.json",
        "managed-settings-cache.json",
        "remote-settings.json",
    ] {
        inspect_file(paths, &config.join(name), true)?;
    }
    // The CLI can find a project above cwd; inspect each settings scope up to
    // the home (or the trial root). Nothing is overwritten.
    for ancestor in cwd.ancestors() {
        // A trial never reads above its selected root (in any spelling).
        let at_root = if paths.is_trial() {
            match paths.relative_to_selected_root(ancestor) {
                Ok(relative) => relative.as_os_str().is_empty(),
                Err(_) => break,
            }
        } else {
            ancestor == paths.root()
        };
        for name in ["settings.json", "settings.local.json"] {
            inspect_file(paths, &ancestor.join(".claude").join(name), false)?;
        }
        if ancestor == home || at_root {
            break;
        }
    }
    if !paths.is_trial() {
        #[cfg(target_os = "linux")]
        inspect_managed_directory(paths, Path::new("/etc/claude-code"))?;
        #[cfg(target_os = "macos")]
        {
            inspect_managed_directory(paths, Path::new("/Library/Application Support/ClaudeCode"))?;
            inspect_file(
                paths,
                Path::new("/Library/Managed Preferences/com.anthropic.claudecode.plist"),
                true,
            )?;
            // Per-user MDM files: refuse any without guessing the account name.
            let directory = match Dir::open(Path::new("/Library/Managed Preferences")) {
                Ok(directory) => Some(directory),
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(_) => return Err(conflict()),
            };
            if let Some(directory) = directory {
                for entry in directory.entries().map_err(|_| conflict())? {
                    if directory.metadata(&entry).map_err(|_| conflict())?.is_dir() {
                        inspect_file(
                            paths,
                            &directory
                                .path()
                                .join(entry)
                                .join("com.anthropic.claudecode.plist"),
                            true,
                        )?;
                    }
                }
            }
        }
        #[cfg(windows)]
        {
            inspect_managed_directory(paths, Path::new(r"C:\Program Files\ClaudeCode"))?;
            if managed_registry_present()? {
                return Err(conflict());
            }
        }
    }
    Ok(())
}

/// Environment variables through which Claude Code takes an API key instead
/// of the plan login: the key itself, and a file descriptor it reads a key
/// from. Both are reported as the `ANTHROPIC_API_KEY` source by Claude Code.
const PLAN_LOGIN_KEY_ENVIRONMENT: &[&str] =
    &["ANTHROPIC_API_KEY", "CLAUDE_CODE_API_KEY_FILE_DESCRIPTOR"];

/// What a plan login run (the default) switches off so it is not billed to
/// an API key the person did not select for it. Nothing here is written to
/// the person's files: the environment entries are removed from the
/// provider's environment only, and the settings entries are blanked in the
/// run's own session settings (`--settings`), which Claude Code ranks above
/// the user, project and local settings. Names only; no value is kept.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlanLoginOverride {
    /// Spellings found in the wrapper's environment with a non-empty value.
    environment: BTreeSet<String>,
    /// A user or project settings file sets `apiKeyHelper`.
    settings_helper: bool,
    /// Spellings of `env.ANTHROPIC_API_KEY` found in those settings.
    settings_environment: BTreeSet<String>,
}
impl PlanLoginOverride {
    /// Whether the run needs its own settings to switch a key off.
    pub fn needs_settings(&self) -> bool {
        self.settings_helper || !self.settings_environment.is_empty()
    }
    /// Removes the key variables from what the provider inherits.
    pub fn remove_environment(
        &self,
        env: &mut std::collections::BTreeMap<std::ffi::OsString, Option<std::ffi::OsString>>,
    ) {
        for name in &self.environment {
            env.insert(name.into(), None);
        }
    }
    /// Blank values: Claude Code treats an empty `apiKeyHelper` and an empty
    /// `ANTHROPIC_API_KEY` as not set, so the run falls back to the login.
    fn insert_settings(&self, settings: &mut serde_json::Map<String, serde_json::Value>) {
        if self.settings_helper {
            settings.insert("apiKeyHelper".into(), String::new().into());
        }
        if !self.settings_environment.is_empty() {
            let env = settings
                .entry("env")
                .or_insert_with(|| serde_json::Value::Object(Default::default()));
            if let Some(env) = env.as_object_mut() {
                for name in self
                    .settings_environment
                    .iter()
                    .map(String::as_str)
                    .chain(["ANTHROPIC_API_KEY"])
                {
                    env.insert(name.into(), String::new().into());
                }
            }
        }
    }
}

/// The credential part of a run's session-owned Claude settings.
pub enum SessionCredential<'a> {
    /// API key billing: the confirmed key command as `apiKeyHelper`.
    Helper(&'a PrivateRecipe),
    /// Plan login: blank out a key the person's own settings provide.
    PlanLogin(&'a PlanLoginOverride),
}
impl SessionCredential<'_> {
    pub(crate) fn insert_settings(
        &self,
        settings: &mut serde_json::Map<String, serde_json::Value>,
    ) {
        match self {
            Self::Helper(recipe) => recipe.insert_settings(settings),
            Self::PlanLogin(plan) => plan.insert_settings(settings),
        }
    }
}

/// A plan login run whose key could not be switched off for this run: the
/// provider arguments already name a settings file of their own, so which of
/// the two Claude Code reads last is not something this wrapper decides.
pub fn plan_login_override_conflicts(override_: &PlanLoginOverride, args: &[String]) -> bool {
    override_.needs_settings()
        && args
            .iter()
            .any(|arg| arg == "--settings" || arg.starts_with("--settings="))
}

/// Plan login (the default) for a provider that supports API key billing:
/// finds what would bill the run to an API key the person did not select for
/// it, so the launch can switch exactly that off for this run instead of
/// refusing to start. Only sources that make Claude Code use a key are looked
/// at: a non-empty key variable in the environment, and `apiKeyHelper` or an
/// `env.ANTHROPIC_API_KEY` in the user or project settings. Names and
/// emptiness only; no value is read. A route to a local model clears the key
/// to an empty value and sets its own endpoint; its settings are its own. A
/// settings file that cannot be read or parsed is left to Claude Code as
/// before (this never makes a plan run depend on files it did not need).
pub fn plan_login_override(
    paths: &RuntimePaths,
    home: &Path,
    cwd: &Path,
    environment: &[String],
) -> PlanLoginOverride {
    let mut found = PlanLoginOverride {
        environment: environment
            .iter()
            .filter_map(|entry| entry.split_once('='))
            .filter(|(key, value)| {
                !value.is_empty()
                    && PLAN_LOGIN_KEY_ENVIRONMENT
                        .iter()
                        .any(|name| key.eq_ignore_ascii_case(name))
            })
            .map(|(key, _)| key.to_owned())
            .collect(),
        ..Default::default()
    };
    // A run sent to another endpoint (a local model route) is not billed to
    // the Anthropic plan or a key either way; its settings are its own.
    if env(environment, "ANTHROPIC_BASE_URL").is_some_and(|value| !value.is_empty()) {
        return found;
    }
    let config = match env(environment, "CLAUDE_CONFIG_DIR") {
        Some(path) if Path::new(path).is_absolute() => PathBuf::from(path),
        Some(_) => return found,
        None => home.join(".claude"),
    };
    let mut files: Vec<PathBuf> = ["settings.json", "settings.local.json"]
        .iter()
        .map(|name| config.join(name))
        .collect();
    for ancestor in cwd.ancestors() {
        let at_root = if paths.is_trial() {
            match paths.relative_to_selected_root(ancestor) {
                Ok(relative) => relative.as_os_str().is_empty(),
                Err(_) => break,
            }
        } else {
            ancestor == paths.root()
        };
        for name in ["settings.json", "settings.local.json"] {
            files.push(ancestor.join(".claude").join(name));
        }
        if ancestor == home || at_root {
            break;
        }
    }
    for path in &files {
        settings_provide_a_key(paths, path, &mut found);
    }
    found
}

fn settings_provide_a_key(paths: &RuntimePaths, path: &Path, found: &mut PlanLoginOverride) {
    if paths.is_trial() && paths.relative_to_selected_root(path).is_err() {
        return;
    }
    let (Some(parent), Some(name)) = (path.parent(), path.file_name().and_then(|v| v.to_str()))
    else {
        return;
    };
    let Ok(bytes) = Dir::open(parent).and_then(|directory| directory.read(name, 128 * 1024)) else {
        return;
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return;
    };
    if value
        .get("apiKeyHelper")
        .is_some_and(|helper| !helper.is_null())
    {
        found.settings_helper = true;
    }
    if let Some(env) = value.get("env").and_then(serde_json::Value::as_object) {
        found.settings_environment.extend(
            env.iter()
                .filter(|(key, value)| {
                    key.eq_ignore_ascii_case("ANTHROPIC_API_KEY")
                        && value.as_str().is_none_or(|value| !value.is_empty())
                })
                .map(|(key, _)| key.clone()),
        );
    }
}

#[cfg(windows)]
fn managed_registry_present() -> io::Result<bool> {
    use windows_sys::Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
        System::Registry::{
            HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY, RegCloseKey,
            RegOpenKeyExW,
        },
    };
    let key: Vec<u16> = "SOFTWARE\\Policies\\ClaudeCode"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    for root in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        let mut handle = std::ptr::null_mut();
        // SAFETY: the key name is NUL-terminated and outlives the call; a
        // successfully opened handle is closed right away.
        let result = unsafe {
            RegOpenKeyExW(
                root,
                key.as_ptr(),
                0,
                KEY_READ | KEY_WOW64_64KEY,
                &mut handle,
            )
        };
        if result == ERROR_SUCCESS {
            // SAFETY: the handle came from the successful open above.
            unsafe { RegCloseKey(handle) };
            return Ok(true);
        }
        if result != ERROR_FILE_NOT_FOUND {
            return Err(conflict());
        }
    }
    Ok(false)
}

/// Runs `claude --version`: stdout only (bounded), stderr discarded, a fixed
/// error for anything but a clean exit.
async fn probe(plan: &ProcessPlan, cancel: &Cancellation) -> io::Result<Vec<u8>> {
    let mut plan = plan.clone();
    plan.stdin.clear();
    plan.timeout = Duration::from_secs(10);
    plan.output_cap = 16 * 1024;
    plan.pipe_drain_timeout = Duration::from_secs(1);
    let output = process::run_capped_with_options(
        &plan,
        cancel,
        SpawnOptions {
            stderr_null: true,
            ..Default::default()
        },
    )
    .await
    .map_err(|_| io::Error::from(BillingError::UnsupportedVersion))?;
    if output.outcome
        != (ExitOutcome::Exited {
            code: Some(0),
            signal: None,
        })
        || output.stdout_truncated
        || output.pipes_forced_closed
    {
        return Err(BillingError::UnsupportedVersion.into());
    }
    Ok(output.stdout)
}

/// The Claude Code versions whose `--bare` (auth strictly from `apiKeyHelper`
/// via `--settings`), `--max-budget-usd`, settings sources and short-option
/// grammar were reviewed (2.1.295; 2.1.330 re-read on 2026-11-24). Only an
/// exact match is accepted: a later version may read settings or credentials
/// from somewhere these checks do not inspect, so it waits until reviewed and
/// added here.
const VERIFIED_VERSIONS: &[&str] = &["2.1.295", "2.1.330"];
pub fn verified_version(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    text.trim()
        .strip_suffix(" (Claude Code)")
        .is_some_and(|version| VERIFIED_VERSIONS.contains(&version))
}

#[derive(serde::Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct ConfirmedBilling {
    mode: String,
    profile_id: String,
    revision: String,
    max_budget_usd: Option<f64>,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistrationData {
    #[serde(default)]
    #[allow(dead_code)]
    headless_run_id: Option<String>,
    #[serde(default)]
    headless_billing: Option<ConfirmedBilling>,
}

/// Whether the Hub's registration ACK carries exactly this API key billing.
/// The Hub sends it only for a spawn a person approved in the confirmation
/// dialog; a manual `wrap` or any other registration never gets it, so the
/// wrapper's own flags alone can never start a paid run.
pub fn hub_confirmed(registered: &Message, billing: &ResolvedBilling) -> bool {
    if !billing.is_api() || registered.data.is_empty() || registered.data.len() > 2048 {
        return false;
    }
    let Ok(data) = serde_json::from_slice::<RegistrationData>(&registered.data) else {
        return false;
    };
    data.headless_billing.is_some_and(|confirmed| {
        confirmed
            == ConfirmedBilling {
                mode: billing.mode.clone(),
                profile_id: billing.profile_id.clone(),
                revision: billing.revision.clone(),
                max_budget_usd: billing.max_budget_usd,
            }
    })
}

/// Checks an API key billing launch and returns the confirmed recipe. The
/// caller merges it into the session-owned settings and then calls
/// [`finish_args`]. `version_probe` is `claude --version` on the same
/// executable and environment as the launch.
pub async fn prepare(
    context: BillingContext<'_>,
    version_probe: &ProcessPlan,
    cancel: &Cancellation,
) -> io::Result<PrivateRecipe> {
    let billing = &context.launch.billing;
    if !billing.is_api() || !headless_billing::api_key_supported(&context.launch.provider) {
        return Err(BillingError::InvalidSelection.into());
    }
    if !hub_confirmed(context.registered, billing) {
        return Err(BillingError::HumanConfirmationRequired.into());
    }
    if context.launch.custom_argv.is_some() {
        return Err(conflict());
    }
    // The launch's own Claude session ID is the only `--session-id` allowed.
    let mut checked = context.launch.provider_args.clone();
    if !context.launch.agent_session_id.is_empty()
        && let Some(index) = checked.windows(2).position(|pair| {
            pair[0] == "--session-id" && pair[1] == context.launch.agent_session_id
        })
    {
        checked.drain(index..index + 2);
    }
    validate_provider_args(&checked, true)?;
    check_auth_files(
        context.paths,
        context.home_dir,
        context.cwd,
        context.environment,
    )?;
    let version = probe(version_probe, cancel).await?;
    if !verified_version(&version) {
        return Err(BillingError::UnsupportedVersion.into());
    }
    Ok(headless_billing::recipe(context.paths, billing)?)
}

/// After the settings hook ran: exactly one `--settings` (ours, carrying the
/// helper), then `--bare` (auth strictly from that helper; no OAuth, keychain,
/// CLAUDE.md, skills or MCP discovery) and the budget.
pub fn finish_args(args: &mut Vec<String>, billing: &ResolvedBilling) -> io::Result<()> {
    if args.iter().filter(|arg| *arg == "--settings").count() != 1
        || args.iter().any(|arg| arg.starts_with("--settings="))
    {
        return Err(conflict());
    }
    let budget = billing
        .max_budget_usd
        .filter(|value| value.is_finite() && *value > 0.0)
        .ok_or(BillingError::InvalidSelection)?;
    args.extend([
        "--bare".into(),
        "--max-budget-usd".into(),
        budget.to_string(),
    ]);
    Ok(())
}

#[cfg(test)]
mod tests;
