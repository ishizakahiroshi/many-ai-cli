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
    collections::BTreeMap,
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
) -> io::Result<()> {
    for entry in entries {
        let Some((key, value)) = entry.split_once('=') else {
            return Err(invalid("invalid wrapper environment entry"));
        };
        if key.is_empty() || key.contains('\0') || value.contains('\0') {
            return Err(invalid("invalid wrapper environment entry"));
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
    let dir = Dir::open_or_create_private(&paths.resource(Resource::Temporary))?;
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
) -> io::Result<PreparedSpawn> {
    if !spec.cwd.is_absolute() {
        return Err(invalid("invalid wrapper working directory"));
    }
    // Profile selection happens before model/label validation in Go. The real
    // policy repeats those checks after selection and before route/key work;
    // this caller does not silently skip auto-selection's source mutation.
    let resolved = policy.resolve(spec, &options.environment)?;
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
) -> io::Result<PreparedSpawn> {
    prepare_resolved(options, policy, spec, resolved, Some(kind), warn)
}
fn prepare_resolved(
    options: &WrapperSpawnOptions,
    policy: &dyn SpawnLaunchPolicy,
    spec: &WrappedSpawnSpec,
    resolved: &ResolvedSpawnPolicy,
    kind: Option<WrappedStartKind>,
    warn: &(dyn Fn(&'static str) + Send + Sync),
) -> io::Result<PreparedSpawn> {
    if !spec.cwd.is_absolute() {
        return Err(invalid("invalid wrapper working directory"));
    }
    let bare = matches!(kind, Some(WrappedStartKind::Bare | WrappedStartKind::Grid));
    if (!bare && !valid_model_label(&spec.model)) || !valid_model_label(&spec.label) {
        return Err(invalid("invalid wrapper model or label"));
    }
    if !bare && risk_confirmation(spec, resolved, kind.is_some()) {
        return Err(invalid("risk confirmation required"));
    }
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
            args.extend(resolved.ordinary_model_args.iter().cloned().map(Into::into));
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
        args.extend(resolved.effort_args.iter().cloned().map(Into::into));
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
        prompt = self::prompt(&options.paths, &spec.initial_prompt)?;
        if let Some(prompt) = &prompt {
            args.extend(["--prompt-file".into(), prompt.path().into_os_string()]);
        }
        if spec.provider == "codex" && local_route(&resolved.effective_route) {
            args.push("--codex-oss".into());
        }
        if spec.utf8_session {
            args.push("--utf8".into());
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
    if let Some(WrappedStartKind::OrdinaryAi { delegation }) = kind {
        add_environment(
            &mut environment,
            &[format!("MANY_AI_CLI_DELEGATION={}", u8::from(delegation))],
        )?;
    }
    // Internal proof/Job cannot be inherited from another wrapper or replaced by
    // a profile/route. Bootstrap receives only the proof which core just issued.
    environment.retain(|_, (key, _)| {
        !key.eq_ignore_ascii_case(SPAWN_PROOF_ENV) && !key.eq_ignore_ascii_case(STARTUP_JOB_ENV)
    });
    let proof = spec
        .registration_proof
        .as_ref()
        .ok_or_else(|| invalid("core registration proof is required"))?;
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
    crate::files::safe_fs::basename(&spec.provider)?;
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
            std::fs::OpenOptions::new().write(true).open(null)?
        }
        Err(error) => return Err(error),
    };
    let stderr = stdout.try_clone()?;
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
