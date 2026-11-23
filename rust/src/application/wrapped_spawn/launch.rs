//! Go spawn-handler argument/environment boundary. All profile/route decisions
//! stay with the application policy, so auto profile selection runs only once.
use crate::{
    config::{self, Resource, RuntimePaths},
    files::safe_fs::Dir,
    process::{ProcessPlan, random_token, wrapper_startup::STARTUP_JOB_ENV},
    proto::core::{SPAWN_PROOF_ENV, WrappedSpawnSpec, WrappedStartKind},
    wrapper::launch::launch_prompt_file_owner,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs::File,
    io,
    path::PathBuf,
    time::{Duration, SystemTime},
};

#[derive(Clone)]
pub struct WrapperSpawnOptions {
    pub paths: RuntimePaths,
    /// Exact executable running this Hub, never a PATH rediscovery.
    pub executable: PathBuf,
    /// Only the many-ai-cli child receives this HOME. Its provider is sanitized
    /// again by wrapper::entry, retaining the trial vendor/profile directories.
    pub application_home: PathBuf,
    /// Explicit inherited snapshot; no process-global environment mutation.
    pub environment: Vec<String>,
    pub hub_port: u16,
    pub parent_shell: String,
    pub pending_capacity: usize,
    pub reap_timeout: Duration,
}
impl WrapperSpawnOptions {
    pub(super) fn validate(&self) -> io::Result<()> {
        if !self.executable.is_absolute()
            || self.hub_port == 0
            || self.pending_capacity == 0
            || self.reap_timeout.is_zero()
            || self.reap_timeout > Duration::from_secs(60)
            || (self.paths.is_trial() && self.paths.port() != self.hub_port)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid wrapper spawn runtime options",
            ));
        }
        if self.paths.is_trial() {
            if !self.application_home.is_absolute() {
                return Err(invalid("trial application home must be absolute"));
            }
            // Preserve the same disjoint-root validation performed by main.
            RuntimePaths::trial(
                self.paths.root(),
                self.hub_port,
                &self.application_home.join(".many-ai-cli"),
            )?;
        }
        Ok(())
    }
}
pub use crate::proto::core::ResolvedSpawnPolicy;
pub trait SpawnLaunchPolicy: Send + Sync {
    /// Must validate provider, profile enablement/existence, route and configured
    /// model/effort support. Resolve auto subscription exactly once. No fallback
    /// on failure: the process must not run using another account or route.
    fn resolve(
        &self,
        spec: &WrappedSpawnSpec,
        base_environment: &[String],
    ) -> io::Result<ResolvedSpawnPolicy>;
    /// Invoked only for the core's explicit trust grant, once after env is final.
    /// Source treats failure as warning and continues launching the same account.
    fn folder_trust(&self, spec: &WrappedSpawnSpec, exact_environment: &[String])
    -> io::Result<()>;
    /// Source records nonlocal, nonempty models after registration for child/login
    /// callers, or after native Start for ordinary AI callers.
    fn registered_model(&self, provider: &str, model: &str) -> io::Result<()>;
}
pub(super) struct PromptFile {
    dir: Dir,
    name: String,
    remove: bool,
}
impl PromptFile {
    pub(super) fn path(&self) -> PathBuf {
        self.dir.path().join(&self.name)
    }
    pub(super) fn release_to_wrapper(mut self) {
        self.remove = false;
    }
}
impl Drop for PromptFile {
    fn drop(&mut self) {
        if self.remove {
            let _ = self.dir.remove_file(&self.name);
        }
    }
}
pub(super) struct PreparedSpawn {
    pub process: ProcessPlan,
    pub stdout: File,
    pub stderr: File,
    pub prompt: Option<PromptFile>,
    pub record_model: Option<String>,
}
fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
/// Which step stopped a wrapper launch. Every step has a fixed text, so the
/// failure a caller receives (a child's `spawn error: ...` 500) and the Hub log
/// say which step failed without carrying an OS message, a path or a profile.
/// Before this, every step was the same "wrapper process launch failed", and a
/// refused risk confirmation could not be told from a wrapper that never ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LaunchFailure {
    /// Working directory, model, label or provider name the launch cannot use.
    InvalidRequest,
    /// A high-risk launch (bypass, model change) nobody confirmed.
    RiskConfirmation,
    /// The provider is absent, disabled or has no launch definition, or its
    /// model/label were refused by the launch policy.
    ProviderPolicy,
    SubscriptionProfile,
    Route,
    /// Environment entries, protected-name list or the registration proof.
    Environment,
    IrisOwner,
    PromptFile,
    SpawnLog,
    /// The wrapper process itself could not be created.
    NativeStart,
    /// Hub stop or task cancellation closed the startup first.
    Closed,
}
impl LaunchFailure {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::InvalidRequest => {
                "wrapper process launch failed: invalid working directory, model, label or provider"
            }
            Self::RiskConfirmation => "wrapper process launch failed: risk confirmation required",
            Self::ProviderPolicy => {
                "wrapper process launch failed: the provider is unavailable or refused this launch"
            }
            Self::SubscriptionProfile => {
                "wrapper process launch failed: subscription profile error"
            }
            Self::Route => "wrapper process launch failed: route error",
            Self::Environment => "wrapper process launch failed: invalid launch environment",
            Self::IrisOwner => "wrapper process launch failed: invalid Iris owner bootstrap",
            Self::PromptFile => {
                "wrapper process launch failed: the initial prompt file could not be written"
            }
            Self::SpawnLog => "wrapper process launch failed: the spawn log could not be opened",
            Self::NativeStart => {
                "wrapper process launch failed: the wrapper process could not be started"
            }
            Self::Closed => "wrapper process launch failed: wrapper startup closed",
        }
    }
    fn policy(error: &io::Error) -> Self {
        use crate::application::spawn_policy::{PolicyFailureStage, failure_stage};
        match failure_stage(error) {
            Some(PolicyFailureStage::Route) => Self::Route,
            Some(PolicyFailureStage::Subscription) => Self::SubscriptionProfile,
            None => Self::ProviderPolicy,
        }
    }
}
impl std::fmt::Display for LaunchFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message())
    }
}
fn valid_model_label(value: &str) -> bool {
    !value.starts_with('-') && crate::profile::validation::valid_spawn_model_label(value)
}
fn local_route(route: &str) -> bool {
    matches!(route, "ollama" | "lm-studio")
}
fn risk_confirmation(
    spec: &WrappedSpawnSpec,
    policy: &ResolvedSpawnPolicy,
    ordinary: bool,
) -> bool {
    if spec.usage_probe || spec.subscription_login || spec.risk_confirmed {
        return false;
    }
    let model_changed = !policy.resolved_model.trim().is_empty()
        && policy.resolved_model.trim() != policy.current_model.trim();
    match spec.provider.as_str() {
        "claude" => {
            spec.model_selection == "required"
                || model_changed
                || spec.permission_mode == "bypassPermissions"
        }
        "codex" => {
            spec.model_selection == "required"
                || model_changed
                || spec.sandbox == "danger-full-access"
                || spec.ask_for_approval == "never"
        }
        "shell" => false,
        "opencode" | "command-code" => spec.permission_mode == "bypassPermissions",
        _ if ordinary => false,
        _ => spec.permission_mode == "bypassPermissions",
    }
}
fn environment_key(name: &str) -> String {
    if cfg!(windows) {
        name.to_ascii_uppercase()
    } else {
        name.into()
    }
}
fn add_environment(
    environment: &mut BTreeMap<String, (String, String)>,
    entries: &[String],
) -> Result<(), LaunchFailure> {
    for entry in entries {
        let Some((key, value)) = entry.split_once('=') else {
            return Err(LaunchFailure::Environment);
        };
        if key.is_empty() || key.contains('\0') || value.contains('\0') {
            return Err(LaunchFailure::Environment);
        }
        let value = if key.eq_ignore_ascii_case("PATH") {
            value
                .split(if cfg!(windows) { ';' } else { ':' })
                .filter(|entry| !entry.trim().is_empty())
                .collect::<Vec<_>>()
                .join(if cfg!(windows) { ";" } else { ":" })
        } else {
            value.into()
        };
        environment.insert(environment_key(key), (key.into(), value));
    }
    Ok(())
}
/// Bounded single-directory collection using held capabilities; unrelated names
/// and still-live wrapper-owned pointer files are never removed.
fn sweep_prompts(dir: &Dir) {
    let Ok(names) = dir.entries() else {
        return;
    };
    for name in names {
        if let Some(pid) = launch_prompt_file_owner(&name) {
            if crate::process::pid_alive(pid) {
                continue;
            }
        } else if !name.starts_with("prompt-") || !name.ends_with(".md") {
            continue;
        }
        let Ok(metadata) = dir.metadata(&name) else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        if metadata
            .modified()
            .ok()
            .and_then(|time| SystemTime::now().duration_since(time).ok())
            .is_some_and(|age| age > Duration::from_secs(3600))
        {
            let _ = dir.remove_file(&name);
        }
    }
}
fn prompt(paths: &RuntimePaths, body: &str) -> io::Result<Option<PromptFile>> {
    if body.trim().is_empty() {
        return Ok(None);
    }
    let dir = Dir::open_or_create_private_components(&paths.resource(Resource::Temporary))?;
    sweep_prompts(&dir);
    let name = format!("prompt-{}.md", &random_token()?[..32]);
    dir.create_new(&name, body.as_bytes(), 0o600)?;
    Ok(Some(PromptFile {
        dir,
        name,
        remove: true,
    }))
}
pub(super) fn prepare(
    options: &WrapperSpawnOptions,
    policy: &dyn SpawnLaunchPolicy,
    spec: &WrappedSpawnSpec,
    warn: &(dyn Fn(&'static str) + Send + Sync),
) -> Result<PreparedSpawn, LaunchFailure> {
    if !spec.cwd.is_absolute() {
        return Err(LaunchFailure::InvalidRequest);
    }
    // Profile selection happens before model/label validation in Go. The real
    // policy repeats those checks after selection and before route/key work;
    // this caller does not silently skip auto-selection's source mutation.
    let resolved = policy
        .resolve(spec, &options.environment)
        .map_err(|error| LaunchFailure::policy(&error))?;
    prepare_resolved(options, policy, spec, &resolved, None, warn)
}
/// The ordinary caller already selected its profile/route/model. Never resolve
/// again here: selection may mutate round-robin state or choose another account.
pub(super) fn prepare_start(
    options: &WrapperSpawnOptions,
    policy: &dyn SpawnLaunchPolicy,
    spec: &WrappedSpawnSpec,
    resolved: &ResolvedSpawnPolicy,
    kind: WrappedStartKind,
    warn: &(dyn Fn(&'static str) + Send + Sync),
) -> Result<PreparedSpawn, LaunchFailure> {
    prepare_resolved(options, policy, spec, resolved, Some(kind), warn)
}
fn prepare_resolved(
    options: &WrapperSpawnOptions,
    policy: &dyn SpawnLaunchPolicy,
    spec: &WrappedSpawnSpec,
    resolved: &ResolvedSpawnPolicy,
    kind: Option<WrappedStartKind>,
    warn: &(dyn Fn(&'static str) + Send + Sync),
) -> Result<PreparedSpawn, LaunchFailure> {
    use LaunchFailure::{Environment, InvalidRequest, IrisOwner, PromptFile, SpawnLog};
    if !spec.cwd.is_absolute() {
        return Err(InvalidRequest);
    }
    let bare = matches!(kind, Some(WrappedStartKind::Bare | WrappedStartKind::Grid));
    if (!bare && !valid_model_label(&spec.model)) || !valid_model_label(&spec.label) {
        return Err(InvalidRequest);
    }
    if !bare && risk_confirmation(spec, resolved, kind.is_some()) {
        return Err(LaunchFailure::RiskConfirmation);
    }
    let registered = !crate::proto::provider::BUILTIN_PROVIDER_IDS
        .contains(&spec.provider.as_str())
        && spec.provider != "shell";
    let mut args = Vec::<OsString>::new();
    if options.paths.is_trial() {
        args.extend([
            "--trial-root".into(),
            options.paths.root().as_os_str().into(),
            "--trial-port".into(),
            options.hub_port.to_string().into(),
        ]);
    }
    args.extend(["wrap".into(), spec.provider.clone().into()]);
    if !spec.label.is_empty() {
        args.push(format!("--label={}", spec.label).into());
    }
    let mut prompt = None;
    // What a model template or an effort mapping expands to, when the wrapper
    // cannot take it as its own `--model` / `--effort`: handed over behind
    // `--` after every wrapper flag (audit N19).
    let mut provider_tail: Vec<OsString> = Vec::new();
    if bare {
        if kind == Some(WrappedStartKind::Bare) && spec.provider == "shell" && spec.utf8_session {
            args.push("--utf8".into());
        }
    } else if kind.is_none() && spec.subscription_login {
        args.push("--subscription-login".into());
    } else {
        if kind.is_some()
            && !crate::proto::provider::BUILTIN_PROVIDER_IDS.contains(&spec.provider.as_str())
        {
            // The wrapper reads its flags the way Go's flag package does: it
            // stops at the first one it does not know and drops it. Put
            // first, a template such as `--engine <model>` therefore ended
            // the wrapper's own flags (the first prompt, the allowed tools)
            // and lost its flag name. Only a plain `--model <name>` is the
            // wrapper's own flag and stays in front.
            let model = &resolved.ordinary_model_args;
            let own = match model.as_slice() {
                [flag, _] => flag == "--model",
                [one] => one.starts_with("--model="),
                _ => false,
            };
            if own {
                args.extend(model.iter().cloned().map(Into::into));
            } else {
                provider_tail.extend(model.iter().cloned().map(Into::into));
            }
        } else if !resolved.resolved_model.is_empty() {
            args.extend(["--model".into(), resolved.resolved_model.clone().into()]);
        }
        match spec.provider.as_str() {
            "codex" => {
                if !spec.sandbox.is_empty() {
                    args.extend(["--sandbox".into(), spec.sandbox.clone().into()]);
                }
                if !spec.ask_for_approval.is_empty() {
                    args.extend([
                        "--ask-for-approval".into(),
                        spec.ask_for_approval.clone().into(),
                    ]);
                }
            }
            "shell" => {}
            _ if (kind.is_none()
                || matches!(
                    spec.provider.as_str(),
                    "claude" | "opencode" | "command-code"
                ))
                && !spec.permission_mode.is_empty()
                && spec.permission_mode != "default" =>
            {
                args.extend([
                    "--permission-mode".into(),
                    spec.permission_mode.clone().into(),
                ]);
            }
            _ => {}
        }
        // A registered provider's approval-mode flags are its own spelling too.
        // An orchestrate child (no start kind) needs them as much as a new
        // session: the wrapper's own `--permission-mode` maps only built-ins,
        // so without this the mode a human confirmed never reached the CLI.
        // Only a confirmed mode, though: `risk_confirmation` refuses just
        // bypassPermissions, yet a definition maps plan/acceptEdits/auto to
        // flags of its own choosing, so an unconfirmed (autonomous) child's
        // mode would otherwise reach the CLI with nobody having seen it.
        if registered
            && (kind.is_some()
                || spec.risk_confirmed
                || matches!(spec.permission_mode.trim(), "" | "default"))
        {
            provider_tail.extend(
                resolved
                    .ordinary_permission_args
                    .iter()
                    .cloned()
                    .map(Into::into),
            );
        }
        // An effort mapping is the same: `--effort <level>` is the wrapper's
        // own flag, and a provider's own spelling (codex:
        // `-c model_reasoning_effort="high"`) is not, so it follows `--` too.
        let effort = &resolved.effort_args;
        let own = match effort.as_slice() {
            [flag, _] => flag == "--effort",
            [one] => one.starts_with("--effort="),
            _ => false,
        };
        if own {
            args.extend(effort.iter().cloned().map(Into::into));
        } else {
            provider_tail.extend(effort.iter().cloned().map(Into::into));
        }
        let tools = spec
            .grants
            .allowed_tools()
            .iter()
            .map(|v| v.trim())
            .filter(|v| config::valid_allowed_tool_value(v))
            .collect::<Vec<_>>();
        if !tools.is_empty() {
            args.extend(["--allowed-tools".into(), tools.join(",").into()]);
        }
        if config::is_headless_execution_mode(&spec.execution_mode) {
            args.push("--headless".into());
        }
        prompt = self::prompt(&options.paths, &spec.initial_prompt).map_err(|_| PromptFile)?;
        if let Some(prompt) = &prompt {
            args.extend(["--prompt-file".into(), prompt.path().into_os_string()]);
        }
        if spec.provider == "codex" && local_route(&resolved.effective_route) {
            args.push("--codex-oss".into());
        }
        if spec.utf8_session {
            args.push("--utf8".into());
        }
        if !provider_tail.is_empty() {
            args.push("--".into());
            args.append(&mut provider_tail);
        }
    }
    let mut environment = BTreeMap::new();
    add_environment(&mut environment, &resolved.base_environment)?;
    add_environment(
        &mut environment,
        &[
            "MANY_AI_CLI=1".into(),
            format!("MANY_AI_CLI_HUB_PORT={}", options.hub_port),
            format!("MANY_AI_CLI_BIN={}", options.executable.to_string_lossy()),
        ],
    )?;
    if kind.is_none() {
        add_environment(
            &mut environment,
            &[
                format!("MANY_AI_CLI_USAGE_PROBE={}", u8::from(spec.usage_probe)),
                format!(
                    "MANY_AI_CLI_SUBSCRIPTION_LOGIN={}",
                    u8::from(spec.subscription_login)
                ),
            ],
        )?;
    }
    if !options.parent_shell.is_empty() {
        add_environment(
            &mut environment,
            &[format!("MANY_AI_CLI_PARENT_SHELL={}", options.parent_shell)],
        )?;
    }
    add_environment(&mut environment, &resolved.route_environment)?;
    add_environment(&mut environment, &resolved.subscription_environment)?;
    // The wrapper may explicitly load a PowerShell profile before it starts
    // the provider. Carry only selected environment names (never their values)
    // so that route/account keys outside standard vendor namespaces can also
    // be restored after profile initialization.
    let protected_names = resolved
        .route_environment
        .iter()
        .chain(&resolved.subscription_environment)
        .filter_map(|entry| entry.split_once('=').map(|(name, _)| name.to_owned()))
        .collect::<BTreeSet<_>>();
    let protected_name_key =
        environment_key(crate::wrapper::launch_payload::PROTECTED_ENV_NAMES_ENV);
    environment.remove(&protected_name_key);
    if !protected_names.is_empty() {
        let serialized = serde_json::to_string(&protected_names.into_iter().collect::<Vec<_>>())
            .map_err(|_| Environment)?;
        environment.insert(
            protected_name_key,
            (
                crate::wrapper::launch_payload::PROTECTED_ENV_NAMES_ENV.into(),
                serialized,
            ),
        );
    }
    if let Some(WrappedStartKind::OrdinaryAi { delegation }) = kind {
        add_environment(
            &mut environment,
            &[format!("MANY_AI_CLI_DELEGATION={}", u8::from(delegation))],
        )?;
    }
    // Internal proof/Job cannot be inherited from another wrapper or replaced by
    // a profile/route. Bootstrap receives only the proof which core just issued.
    environment.retain(|_, (key, _)| {
        !key.eq_ignore_ascii_case(SPAWN_PROOF_ENV)
            && !key.eq_ignore_ascii_case(STARTUP_JOB_ENV)
            && !key.eq_ignore_ascii_case(crate::wrapper::iris_owner::OWNER_ENV)
    });
    if !spec.registration_metadata.iris_owner_context.is_empty() {
        let value = crate::hub::iris_execution_mcp::parse_json(
            spec.registration_metadata.iris_owner_context.as_bytes(),
        )
        .map_err(|_| IrisOwner)?;
        let ticket: crate::hub::iris_execution_owner::OwnerTicket =
            serde_json::from_value(value).map_err(|_| IrisOwner)?;
        if ticket.binding.provider != spec.provider
            || ticket.binding.profile_id
                != if spec.subscription_profile_id.is_empty() {
                    "default"
                } else {
                    &spec.subscription_profile_id
                }
        {
            return Err(IrisOwner);
        }
        environment.insert(
            environment_key(crate::wrapper::iris_owner::OWNER_ENV),
            (
                crate::wrapper::iris_owner::OWNER_ENV.into(),
                spec.registration_metadata.iris_owner_context.clone(),
            ),
        );
    }
    let proof = spec.registration_proof.as_ref().ok_or(Environment)?;
    environment.insert(
        environment_key(SPAWN_PROOF_ENV),
        (SPAWN_PROOF_ENV.into(), proof.as_header_value().into()),
    );
    let exact_environment = environment
        .values()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>();
    let mut overrides = BTreeMap::<OsString, Option<OsString>>::new();
    for entry in &options.environment {
        if let Some((key, _)) = entry.split_once('=')
            && !environment.contains_key(&environment_key(key))
        {
            overrides.insert(key.into(), None);
        }
    }
    overrides.insert(STARTUP_JOB_ENV.into(), None);
    overrides.insert(SPAWN_PROOF_ENV.into(), None);
    for (key, value) in environment.into_values() {
        overrides.insert(key.into(), Some(value.into()));
    }
    if options.paths.is_trial() {
        // Apply after capturing the provider environment for folder trust.
        // This override belongs solely to the exact Hub-owned self executable;
        // provider subprocesses go through trial_environment::prepare again.
        for key in ["HOME", "USERPROFILE"] {
            overrides.insert(
                key.into(),
                Some(options.application_home.as_os_str().into()),
            );
        }
    }
    // Preserve Go's provider + local-time append log name. A provider accepted
    // by application policy must still be one basename at this filesystem edge.
    crate::files::safe_fs::basename(&spec.provider).map_err(|_| InvalidRequest)?;
    let filename = format!(
        "{}-{}.log",
        spec.provider,
        chrono::Local::now().format("%Y%m%d-%H%M%S%.3f")
    );
    let log = || {
        Dir::open_or_create_private_components(&options.paths.resource(Resource::Logs))?
            .child_dir("spawn", true)?
            .open_append(&filename)
    };
    let stdout = match log() {
        Ok(file) => file,
        Err(_) if kind == Some(WrappedStartKind::Grid) => {
            // Go leaves nil stdout/stderr when optional grid logging fails;
            // exec maps nil output to the native null device.
            #[cfg(windows)]
            let null = "NUL";
            #[cfg(not(windows))]
            let null = "/dev/null";
            std::fs::OpenOptions::new()
                .write(true)
                .open(null)
                .map_err(|_| SpawnLog)?
        }
        Err(_) => return Err(SpawnLog),
    };
    let stderr = stdout.try_clone().map_err(|_| SpawnLog)?;
    if !bare
        && spec.grants.grant_folder_trust()
        && policy.folder_trust(spec, &exact_environment).is_err()
    {
        warn("wrapper folder trust could not be written");
    }
    let record_model = (!bare
        && (kind.is_some() || (!spec.subscription_login && !spec.usage_probe))
        && !resolved.resolved_model.is_empty()
        && !local_route(&resolved.effective_route))
    .then(|| resolved.resolved_model.clone());
    Ok(PreparedSpawn {
        process: ProcessPlan {
            executable: options.executable.clone(),
            args,
            cwd: spec.cwd.clone(),
            env: overrides,
            stdin: vec![],
            timeout: Duration::ZERO,
            output_cap: 0,
            pipe_drain_timeout: options.reap_timeout,
        },
        stdout,
        stderr,
        prompt,
        record_model,
    })
}
