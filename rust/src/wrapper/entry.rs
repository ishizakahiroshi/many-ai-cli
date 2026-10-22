//! Binary integration seam. The caller supplies its already-loaded configuration,
//! explicit runtime root, environment snapshot and effective Hub endpoint.
use super::{
    hooks::SessionHooks,
    launch::{self, LaunchHook, LaunchPlan, PreparedLaunchPrompt, WrapperArgs},
    runtime::{self, RawLogPolicy, WrapperOptions, WrapperResult},
    transport::{HubConnector, LoopbackConnector, register},
};
use crate::{
    config::{Config, RuntimePaths},
    orchestration::headless,
    process::{
        Cancellation, ProcessPlan,
        pty::{NativePtyFactory, PtyExit},
    },
    proto::{
        Message,
        core::{SPAWN_PROOF_ENV, TerminalSize},
    },
};
use std::{collections::BTreeMap, io, path::Path, time::Duration};

mod command;
use command::CommandResolver;

pub struct WrapperContext<'a> {
    pub config: &'a Config,
    pub paths: &'a RuntimePaths,
    pub cwd: &'a Path,
    pub executable: &'a Path,
    /// Explicit snapshot, never write process-global environment variables.
    pub environment: &'a [String],
    pub shell: &'a str,
    pub terminal_size: TerminalSize,
    /// Already selected by the caller; trial never discovers the actual home.
    pub home_dir: &'a Path,
}
fn env<'a>(entries: &'a [String], key: &str) -> Option<&'a str> {
    entries.iter().rev().find_map(|entry| {
        entry
            .split_once('=')
            .filter(|(name, _)| name.eq_ignore_ascii_case(key))
            .map(|(_, value)| value)
    })
}
fn overrides(
    base: &[String],
    child: &[String],
) -> BTreeMap<std::ffi::OsString, Option<std::ffi::OsString>> {
    let mut values = BTreeMap::<String, (String, Option<String>)>::new();
    let key = |name: &str| {
        if cfg!(windows) {
            name.to_uppercase()
        } else {
            name.to_owned()
        }
    };
    // Preserve child-slice last-value precedence before BTreeMap sorts keys;
    // Windows environment names compare case-insensitively.
    for entry in base {
        if let Some((name, _)) = entry.split_once('=') {
            values.insert(key(name), (name.into(), None));
        }
    }
    for entry in child {
        if let Some((name, value)) = entry.split_once('=') {
            values.insert(key(name), (name.into(), Some(value.into())));
        }
    }
    values.insert(key(SPAWN_PROOF_ENV), (SPAWN_PROOF_ENV.into(), None));
    values
        .into_values()
        .map(|(name, value)| (name.into(), value.map(Into::into)))
        .collect()
}
fn prompt_path(context: &WrapperContext<'_>, raw: &str) -> std::path::PathBuf {
    let path = Path::new(raw.trim());
    if path.as_os_str().is_empty() || path.is_absolute() {
        path.into()
    } else {
        context.cwd.join(path)
    }
}

fn effective_port(context: &WrapperContext<'_>) -> io::Result<u16> {
    if context.paths.is_trial() {
        return Ok(context.paths.port());
    }
    env(context.environment, "MANY_AI_CLI_HUB_PORT")
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|p| *p > 0)
        .or_else(|| {
            u16::try_from(context.config.hub.port)
                .ok()
                .filter(|p| *p > 0)
        })
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid wrapper Hub port"))
}
fn registration(
    context: &WrapperContext<'_>,
    launch: &LaunchPlan,
    session_shell: &str,
) -> io::Result<Message> {
    let skill_env = |key| {
        if context.paths.is_trial() {
            String::new()
        } else {
            env(context.environment, key).unwrap_or("").into()
        }
    };
    Ok(Message {
        r#type: "register".into(),
        role: "wrapper".into(),
        provider: launch.provider.clone(),
        provider_revision: launch::provider_registry_for_config(context.config)
            .map_err(io::Error::other)?
            .revision()
            .into(),
        display_name: launch.display.clone(),
        cwd: context.cwd.to_string_lossy().into_owned(),
        label: launch.label.clone(),
        model: launch.model.clone(),
        effort: launch.effort.clone(),
        permission_mode: launch.permission_mode.clone(),
        execution_mode: launch.execution_mode.clone(),
        agent_session_id: launch.agent_session_id.clone(),
        pid: i64::from(std::process::id()),
        shell: session_shell.into(),
        token: context.config.token.clone(),
        home_dir: if context.paths.is_trial() {
            context.paths.root()
        } else {
            context.home_dir
        }
        .to_string_lossy()
        .into_owned(),
        codex_home: skill_env("CODEX_HOME"),
        claude_dir: skill_env("CLAUDE_CONFIG_DIR"),
        grok_home: skill_env("GROK_HOME"),
        subscription_id: env(context.environment, "MANY_AI_CLI_SUBSCRIPTION_ID")
            .unwrap_or("")
            .trim()
            .into(),
        usage_probe: env(context.environment, "MANY_AI_CLI_USAGE_PROBE") == Some("1"),
        subscription_login: env(context.environment, "MANY_AI_CLI_SUBSCRIPTION_LOGIN") == Some("1"),
        cols: context.terminal_size.cols,
        rows: context.terminal_size.rows,
        ..Default::default()
    })
}
fn select_launch_shell(
    context: &WrapperContext<'_>,
    launch: &LaunchPlan,
    provider: &str,
    resolver: &CommandResolver<'_>,
) -> io::Result<Option<(String, String)>> {
    if !launch_shell_eligible(
        provider,
        launch.headless,
        launch.login_mode,
        context.environment,
    ) {
        return Ok(None);
    }
    match context.config.hub.ai_launch_shell.as_str() {
        "auto" => Ok(None),
        mode @ ("pwsh" | "cmd") => resolver
            .resolve_ai_launch_shell(mode)
            .map(|path| Some((mode.to_owned(), path))),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "hub.ai_launch_shell must be auto, pwsh, or cmd",
        )),
    }
}
fn launch_shell_eligible(
    provider: &str,
    headless: bool,
    login_mode: bool,
    environment: &[String],
) -> bool {
    !headless
        && !login_mode
        && provider != "shell"
        && env(environment, "MANY_AI_CLI_USAGE_PROBE") != Some("1")
        && env(environment, "MANY_AI_CLI_SUBSCRIPTION_LOGIN") != Some("1")
}
fn registration_shell(
    provider: &str,
    selected_shell: Option<&(String, String)>,
    command: &crate::process::execpath::ResolvedCommand,
) -> String {
    if let Some((_, path)) = selected_shell {
        path.clone()
    } else if provider == "shell" || command.shell_shim.is_some() {
        command.executable.clone()
    } else {
        "direct".into()
    }
}
fn install_shell_bootstrap(
    process: &mut ProcessPlan,
    context: &WrapperContext<'_>,
    mode: &str,
    shell_executable: &str,
    payload: &super::launch_payload::PreparedLaunchPayload,
) -> io::Result<()> {
    let bootstrap = super::launch_payload::build_bootstrap_command(
        mode,
        Path::new(shell_executable),
        context.executable,
        payload.path(),
    )?;
    process.executable = bootstrap.executable.into();
    process.args = bootstrap.args;
    process
        .env
        .insert(super::launch_payload::RUNNER_ENV.into(), None);
    process
        .env
        .insert(super::launch_payload::PAYLOAD_ENV.into(), None);
    for (name, value) in bootstrap.environment {
        process.env.insert(name, Some(value));
    }
    Ok(())
}
fn clear_inherited_raw_cmd_bootstrap(process: &mut ProcessPlan) {
    let marker_keys = process
        .env
        .keys()
        .filter(|key| {
            key.to_string_lossy()
                .eq_ignore_ascii_case(super::launch_payload::CMD_RAW_BOOTSTRAP_ENV)
        })
        .cloned()
        .collect::<Vec<_>>();
    for key in marker_keys {
        process.env.remove(&key);
    }
    process
        .env
        .insert(super::launch_payload::CMD_RAW_BOOTSTRAP_ENV.into(), None);
}
fn apply_hooks(
    context: &WrapperContext<'_>,
    launch: &LaunchPlan,
    registered: &Message,
    port: u16,
    hooks: &mut SessionHooks,
    args: &mut Vec<String>,
) -> io::Result<()> {
    for hook in &launch.required_hooks {
        match hook {
            LaunchHook::ClaudeRegistrationSettings => match hooks.claude_settings(
                context.paths,
                registered,
                context.executable,
                port,
                &context.config.token,
                cfg!(windows),
            ) {
                Ok(extra) => args.extend(extra),
                Err(_) => eprintln!("wrapper: Claude session settings setup failed"),
            },
            LaunchHook::DelegationPrompt { config_default } => {
                if launch::delegation_prompt_enabled(*config_default, context.environment) {
                    match hooks.delegation(context.paths, &launch.provider, registered.session_id) {
                        Ok(extra) => args.extend(extra),
                        Err(_) => eprintln!("wrapper: delegation prompt setup failed"),
                    }
                }
            }
            LaunchHook::OpenCodePermissions {
                permission,
                bash_deny,
            } => {
                if let Err(error) = hooks.opencode(context.cwd, permission, bash_deny) {
                    // Never silently widen a confirmed bounded tier into full
                    // --auto when its deny configuration was not applied.
                    if !bash_deny.is_empty() {
                        return Err(error);
                    }
                    eprintln!("wrapper: OpenCode permission configuration was not applied");
                }
            }
            LaunchHook::Utf8Console => {
                #[cfg(windows)]
                unsafe {
                    // Source deliberately tolerates a launcher without a console.
                    windows_sys::Win32::System::Console::SetConsoleCP(65001);
                    windows_sys::Win32::System::Console::SetConsoleOutputCP(65001);
                }
            }
        }
    }
    Ok(())
}
/// The effective Hub must already be running. Startup/discovery is owned by the
/// application caller; the wrapper never silently revives it during reconnect.
pub async fn run_cli(
    context: WrapperContext<'_>,
    provider: &str,
    args: &[String],
    cancel: &Cancellation,
) -> io::Result<WrapperResult> {
    let launch = launch::prepare_launch(context.config, provider, WrapperArgs::parse(args))
        .map_err(io::Error::other)?;
    let port = effective_port(&context)?;
    let write_timeout = Duration::from_secs(
        u64::try_from(context.config.hub.wrapper_send_write_timeout_sec)
            .ok()
            .filter(|v| *v > 0)
            .unwrap_or(5),
    );
    let connector = LoopbackConnector::new(port, context.config.token.clone(), write_timeout)?;
    let resolver = CommandResolver::new(context.paths, context.environment, context.cwd);
    let command = resolver.resolve_provider(
        provider,
        launch.custom_argv.as_deref(),
        &launch.provider_args,
    )?;
    let selected_shell = select_launch_shell(&context, &launch, provider, &resolver)?;
    let color = if launch.headless {
        "off"
    } else {
        &context.config.hub.terminal_color
    };
    let child = launch::child_env(context.environment, color, &[]);
    let mut process = ProcessPlan {
        executable: command.executable.clone().into(),
        args: command.args.iter().cloned().map(Into::into).collect(),
        cwd: context.cwd.into(),
        env: overrides(context.environment, &child),
        stdin: vec![],
        timeout: Duration::ZERO,
        output_cap: 16 * 1024 * 1024,
        pipe_drain_timeout: Duration::from_secs(2),
    };
    crate::logging::spawn_probe::mark("wrapper_prepared", Some(std::process::id()), None);
    super::startup::strip_provider_environment(&mut process);
    clear_inherited_raw_cmd_bootstrap(&mut process);
    let internal_launch_env = [
        super::launch_payload::RUNNER_ENV,
        super::launch_payload::PAYLOAD_ENV,
        super::launch_payload::CMD_RAW_BOOTSTRAP_ENV,
    ];
    for name in internal_launch_env {
        process.env.insert(name.into(), None);
    }
    let mut prepared_payload = None;
    if let Some((mode, shell_executable)) = &selected_shell {
        super::launch_payload::validate_inner_cmd(&command)?;
        let payload = super::launch_payload::PreparedLaunchPayload::create(
            context.paths,
            &command,
            context.cwd,
            launch.required_hooks.contains(&LaunchHook::Utf8Console),
            &child,
        )?;
        install_shell_bootstrap(&mut process, &context, mode, shell_executable, &payload)?;
        prepared_payload = Some(payload);
    }
    let session_shell = registration_shell(provider, selected_shell.as_ref(), &command);
    let registration = registration(&context, &launch, &session_shell)?;
    let proof = env(context.environment, SPAWN_PROOF_ENV).map(str::to_owned);
    if launch.headless {
        return run_headless(
            context,
            launch,
            registration,
            proof,
            process,
            port,
            write_timeout,
            &connector,
            cancel,
        )
        .await;
    }
    let mut hooks = SessionHooks::default();
    let mut prompt: Option<PreparedLaunchPrompt> = None;
    let mut options = WrapperOptions::new(registration, process);
    options.initial_proof = proof;
    options.login_mode = launch.login_mode;
    if context.config.log.session_enabled {
        options.raw_log = Some(RawLogPolicy {
            paths: context.paths.clone(),
            max_bytes: context
                .config
                .log
                .session_max_size_mb
                .saturating_mul(1024 * 1024),
        });
    }
    options.auto_shutdown = context.config.hub.auto_shutdown;
    options.write_timeout = write_timeout;
    options.reconnect_grace =
        Duration::from_secs(context.config.hub.wrapper_reconnect_grace_sec.max(0) as u64);
    let result = runtime::run(
        options,
        &connector,
        &NativePtyFactory,
        cancel,
        |registered, process| {
            let mut args = launch.provider_args.clone();
            apply_hooks(&context, &launch, registered, port, &mut hooks, &mut args)?;
            if !launch.args.prompt_file.trim().is_empty() {
                let shim = resolver.launch_shell_shim(provider, launch.custom_argv.as_deref())?;
                let prepared = launch::prepare_launch_prompt(
                    context.paths,
                    &prompt_path(&context, &launch.args.prompt_file),
                    registered.session_id,
                    shim.as_deref(),
                    std::process::id(),
                )?;
                if !prepared.arg.is_empty() {
                    args = launch::with_launch_prompt(&args, &prepared.arg);
                }
                prompt = Some(prepared);
            }
            let command =
                resolver.resolve_provider(provider, launch.custom_argv.as_deref(), &args)?;
            if let Some((mode, shell_executable)) = &selected_shell {
                super::launch_payload::validate_inner_cmd(&command)?;
                let payload = super::launch_payload::PreparedLaunchPayload::create(
                    context.paths,
                    &command,
                    context.cwd,
                    launch.required_hooks.contains(&LaunchHook::Utf8Console),
                    &child,
                )?;
                install_shell_bootstrap(process, &context, mode, shell_executable, &payload)?;
                prepared_payload = Some(payload);
            } else {
                process.executable = command.executable.into();
                process.args = command.args.into_iter().map(Into::into).collect();
            }
            Ok(())
        },
    )
    .await;
    let cleanup = hooks.finish();
    drop(prompt);
    drop(prepared_payload);
    match result {
        Ok(result) => {
            cleanup?;
            Ok(result)
        }
        Err(error) => Err(error),
    }
}
#[allow(clippy::too_many_arguments)]
async fn run_headless(
    context: WrapperContext<'_>,
    launch: LaunchPlan,
    registration: Message,
    mut proof: Option<String>,
    mut process: ProcessPlan,
    port: u16,
    timeout: Duration,
    connector: &dyn HubConnector,
    cancel: &Cancellation,
) -> io::Result<WrapperResult> {
    let def = launch.headless_definition.as_ref().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "provider has no headless definition",
        )
    })?;
    let resolver = CommandResolver::new(context.paths, context.environment, context.cwd);
    let argument_prompt = crate::config::normalize_headless_prompt_via(&def.prompt_via) == "arg";
    let unsafe_prompt_transport = || {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "headless argument prompts cannot run through a Windows cmd shim; use a native executable or a provider with headless prompt_via=stdin",
        )
    };
    // Refuse before registration: cmd.exe reparses CRT-escaped arguments as
    // shell source. The interactive pointer transport is not a headless API.
    if argument_prompt
        && resolver
            .launch_shell_shim(&launch.provider, launch.custom_argv.as_deref())?
            .is_some()
    {
        return Err(unsafe_prompt_transport());
    }
    let (socket, registered) =
        register(connector, &registration, proof.as_deref(), timeout, cancel).await?;
    proof.take();
    let mut hooks = SessionHooks::default();
    let mut args = launch.provider_args.clone();
    apply_hooks(&context, &launch, &registered, port, &mut hooks, &mut args)?;
    let prompt = launch::expand_prompt(
        &launch::take_prompt_file(context.paths, Path::new(launch.args.prompt_file.trim()))?,
        registered.session_id,
    );
    args = headless::build_argv(def, &args, &prompt);
    let command =
        resolver.resolve_provider(&launch.provider, launch.custom_argv.as_deref(), &args)?;
    // Recheck the actual resolved command in case the shim changed while the
    // registration/prompt preparation above was awaiting work.
    if argument_prompt && command.shell_shim.is_some() {
        return Err(unsafe_prompt_transport());
    }
    process.executable = command.executable.into();
    process.args = command.args.into_iter().map(Into::into).collect();
    process.env.insert(
        "MANY_AI_CLI_SESSION_ID".into(),
        Some(registered.session_id.to_string().into()),
    );
    process.env.insert(
        "MANY_AI_CLI_HUB_TOKEN".into(),
        Some(context.config.token.clone().into()),
    );
    let child_cancel = Cancellation::default();
    let output_cancel = child_cancel.clone();
    let (tx, mut outputs) = tokio::sync::mpsc::channel::<Vec<u8>>(1024);
    let spec = headless::Spec {
        process,
        prompt,
        prompt_via: def.prompt_via.clone(),
        format: def.format.clone(),
        raw_logs: context
            .config
            .log
            .session_enabled
            .then(|| headless::RawLogs {
                paths: context.paths.clone(),
                prefix: std::path::PathBuf::from("headless").join(format!(
                    "{}_{}",
                    launch.provider.replace(['/', '\\'], "_"),
                    chrono::Local::now().format("%Y%m%d-%H%M%S.%3f")
                )),
            }),
        event_capacity: 1024,
    };
    let execution = headless::run(spec, &child_cancel, move |bytes| {
        if tx.try_send(bytes.to_vec()).is_err() {
            output_cancel.cancel();
        }
    });
    tokio::pin!(execution);
    let mut socket = Some(socket);
    let mut total = 0i64;
    let mut cancel_seen = false;
    let mut output_open = true;
    let result = loop {
        enum Event {
            Output(Option<Vec<u8>>),
            Frame(io::Result<Box<Message>>),
            Done(io::Result<headless::RunResult>),
            Cancel,
        }
        let event = if let Some(socket) = socket.as_mut() {
            tokio::select! {
                result=&mut execution=>Event::Done(result),_ = cancel.cancelled(), if !cancel_seen =>Event::Cancel,
                bytes=outputs.recv(), if output_open =>Event::Output(bytes),frame=socket.receive()=>Event::Frame(frame.map(Box::new)),
            }
        } else {
            tokio::select! {result=&mut execution=>Event::Done(result),_=cancel.cancelled(), if !cancel_seen =>Event::Cancel,bytes=outputs.recv(), if output_open =>Event::Output(bytes)}
        };
        match event {
            Event::Done(result) => match result {
                Ok(result) => break result,
                Err(error) => {
                    if let Some(socket) = socket.as_mut() {
                        let frame = Message {
                            r#type: "session_end".into(),
                            session_id: registered.session_id,
                            state: "error".into(),
                            exit_code: 1,
                            reason: if error.kind() == io::ErrorKind::NotFound {
                                "exec_not_found".into()
                            } else {
                                String::new()
                            },
                            ..Default::default()
                        };
                        let _ = tokio::time::timeout(timeout, socket.send(&frame)).await;
                    }
                    return Err(error);
                }
            },
            Event::Cancel => {
                cancel_seen = true;
                child_cancel.cancel();
            }
            Event::Frame(Ok(frame)) if frame.r#type == "session_dismissed" => child_cancel.cancel(),
            Event::Frame(Ok(frame)) if frame.r#type == "hub_shutdown" => {
                socket.take();
            }
            Event::Frame(Err(_)) => {
                socket.take();
            }
            Event::Frame(Ok(_)) => {}
            Event::Output(Some(bytes)) => {
                total += bytes.len() as i64;
                if let Some(current) = socket.as_mut() {
                    let frame = Message {
                        r#type: "pty_data".into(),
                        session_id: registered.session_id,
                        data: bytes,
                        ..Default::default()
                    };
                    if tokio::time::timeout(timeout, current.send(&frame))
                        .await
                        .map_or(true, |r| r.is_err())
                    {
                        socket.take();
                    }
                }
            }
            Event::Output(None) => {
                output_open = false;
            }
        }
    };
    if let Some(mut socket) = socket {
        while let Ok(bytes) = outputs.try_recv() {
            total += bytes.len() as i64;
            let _ = tokio::time::timeout(
                timeout,
                socket.send(&Message {
                    r#type: "pty_data".into(),
                    session_id: registered.session_id,
                    data: bytes,
                    ..Default::default()
                }),
            )
            .await;
        }
        let final_text = if result.timed_out {
            "[error] run timed out\r\n".into()
        } else if result.canceled {
            "[error] run was stopped\r\n".into()
        } else {
            format!("[result] exit={}\r\n", result.exit_code)
        };
        let final_frame = Message {
            r#type: "pty_data".into(),
            session_id: registered.session_id,
            data: final_text.as_bytes().to_vec(),
            ..Default::default()
        };
        total += final_frame.data.len() as i64;
        let _ = tokio::time::timeout(timeout, socket.send(&final_frame)).await;
        let frame = Message {
            r#type: "session_end".into(),
            session_id: registered.session_id,
            state: result.state.clone(),
            exit_code: i64::from(result.exit_code),
            ..Default::default()
        };
        let _ = tokio::time::timeout(timeout, socket.send(&frame)).await;
    }
    hooks.finish()?;
    Ok(WrapperResult {
        session_id: registered.session_id,
        exit: PtyExit {
            code: i64::from(result.exit_code),
            signal: String::new(),
            forced: result.canceled || result.timed_out,
        },
        reconnects: 0,
        output_bytes: total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        process::execpath::ResolvedCommand,
        wrapper::launch_payload::{PAYLOAD_ENV, PreparedLaunchPayload},
    };

    #[test]
    fn registration_shell_describes_the_actual_launch_path() {
        let direct = ResolvedCommand {
            executable: r"C:\synthetic\codex.exe".into(),
            args: vec![],
            shell_shim: None,
        };
        assert_eq!(registration_shell("codex", None, &direct), "direct");
        let explicit = ("pwsh".into(), r"C:\PowerShell\pwsh.exe".into());
        assert_eq!(
            registration_shell("codex", Some(&explicit), &direct),
            r"C:\PowerShell\pwsh.exe"
        );
        let shim = ResolvedCommand {
            executable: r"C:\Windows\System32\cmd.exe".into(),
            args: vec!["/c".into(), r"C:\synthetic\codex.cmd".into()],
            shell_shim: Some(r"C:\synthetic\codex.cmd".into()),
        };
        assert_eq!(
            registration_shell("codex", None, &shim),
            r"C:\Windows\System32\cmd.exe"
        );
        assert_eq!(
            registration_shell("shell", None, &direct),
            r"C:\synthetic\codex.exe"
        );
    }

    #[test]
    fn shell_mode_bypasses_shell_headless_login_usage_and_subscription_auxiliary_flows() {
        assert!(launch_shell_eligible("codex", false, false, &[]));
        assert!(!launch_shell_eligible("shell", false, false, &[]));
        assert!(!launch_shell_eligible("codex", true, false, &[]));
        assert!(!launch_shell_eligible("codex", false, true, &[]));
        assert!(!launch_shell_eligible(
            "codex",
            false,
            false,
            &["MANY_AI_CLI_USAGE_PROBE=1".into()]
        ));
        assert!(!launch_shell_eligible(
            "codex",
            false,
            false,
            &["MANY_AI_CLI_SUBSCRIPTION_LOGIN=1".into()]
        ));
    }

    #[test]
    fn inherited_raw_cmd_marker_is_cleared_before_auto_resolution() {
        let marker = crate::wrapper::launch_payload::CMD_RAW_BOOTSTRAP_ENV;
        let mut process = ProcessPlan {
            executable: "synthetic-provider".into(),
            args: vec![],
            cwd: std::path::PathBuf::from("C:/synthetic"),
            env: BTreeMap::from([(
                "many_ai_cli_internal_cmd_raw_bootstrap".into(),
                Some("v1".into()),
            )]),
            stdin: vec![],
            timeout: Duration::ZERO,
            output_cap: 1024,
            pipe_drain_timeout: Duration::from_secs(1),
        };
        clear_inherited_raw_cmd_bootstrap(&mut process);
        assert_eq!(process.env.get(std::ffi::OsStr::new(marker)), Some(&None));
        assert!(process.env.keys().all(|key| {
            !key.to_string_lossy().eq_ignore_ascii_case(marker)
                || key == std::ffi::OsStr::new(marker)
        }));
    }

    fn shell_test_context<'a>(
        config: &'a Config,
        paths: &'a RuntimePaths,
        cwd: &'a Path,
        executable: &'a Path,
    ) -> WrapperContext<'a> {
        WrapperContext {
            config,
            paths,
            cwd,
            executable,
            environment: &[],
            shell: "synthetic-shell",
            terminal_size: crate::proto::core::TerminalSize { cols: 80, rows: 24 },
            home_dir: cwd,
        }
    }

    #[test]
    fn post_hook_payload_replaces_pre_registration_payload_and_failed_setup_cleans_up() {
        let temp = tempfile::tempdir().unwrap();
        let runtime = temp.path().join("runtime");
        std::fs::create_dir(&runtime).unwrap();
        let installed = temp.path().join("installed");
        std::fs::create_dir(&installed).unwrap();
        let paths = RuntimePaths::trial(&runtime, 49440, &installed).unwrap();
        let config = Config::default();
        let executable = temp.path().join("synthetic-runner.exe");
        let context = shell_test_context(&config, &paths, temp.path(), &executable);
        let command = |args: &[&str]| ResolvedCommand {
            executable: "synthetic-provider".into(),
            args: args.iter().map(|arg| (*arg).into()).collect(),
            shell_shim: None,
        };
        let mut process = ProcessPlan {
            executable: "synthetic-provider".into(),
            args: vec!["before-hooks".into()],
            cwd: temp.path().into(),
            env: BTreeMap::new(),
            stdin: vec![],
            timeout: Duration::ZERO,
            output_cap: 1024,
            pipe_drain_timeout: Duration::from_secs(1),
        };
        let initial = PreparedLaunchPayload::create(
            &paths,
            &command(&["before-hooks"]),
            temp.path(),
            false,
            &[],
        )
        .unwrap();
        let initial_path = initial.path().to_owned();
        install_shell_bootstrap(
            &mut process,
            &context,
            "pwsh",
            r"C:\synthetic\pwsh.exe",
            &initial,
        )
        .unwrap();
        let after_hooks = PreparedLaunchPayload::create(
            &paths,
            &command(&["after", "hook", "and prompt"]),
            temp.path(),
            true,
            &[],
        )
        .unwrap();
        let after_hooks_path = after_hooks.path().to_owned();
        install_shell_bootstrap(
            &mut process,
            &context,
            "pwsh",
            r"C:\synthetic\pwsh.exe",
            &after_hooks,
        )
        .unwrap();
        drop(initial);
        assert!(!initial_path.exists());
        assert!(after_hooks_path.exists());
        let active_path = process
            .env
            .get(std::ffi::OsStr::new(PAYLOAD_ENV))
            .and_then(Option::as_deref)
            .unwrap();
        assert_eq!(active_path, after_hooks_path.as_os_str());
        drop(after_hooks);
        assert!(!after_hooks_path.exists());

        let failed_path = {
            let payload = PreparedLaunchPayload::create(
                &paths,
                &command(&["synthetic"]),
                temp.path(),
                false,
                &[],
            )
            .unwrap();
            let path = payload.path().to_owned();
            let invalid_runner = Path::new(r"C:\synthetic\100%\runner.exe");
            let failed_context = shell_test_context(&config, &paths, temp.path(), invalid_runner);
            assert!(
                install_shell_bootstrap(
                    &mut process,
                    &failed_context,
                    "cmd",
                    r"C:\Windows\System32\cmd.exe",
                    &payload,
                )
                .is_err()
            );
            path
        };
        assert!(!failed_path.exists());
        assert!(process.env.contains_key(std::ffi::OsStr::new(PAYLOAD_ENV)));
    }
}
