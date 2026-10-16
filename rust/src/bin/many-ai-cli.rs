use many_ai_cli::{
    application::{launcher_program, main_program, runtime_context},
    cli::{self, Command},
    config::{ConfigError, ConfigStore},
    launcher,
    process::{self, Cancellation},
};
fn main() {
    // Adopt an inherited startup Job before any provider or registration work.
    // Its lifetime remains outside Tokio so runtime cleanup happens first.
    let startup = match many_ai_cli::wrapper::startup::StartupLifetime::adopt_from_environment() {
        Ok(startup) => startup,
        Err(error) => {
            eprintln!("wrapper startup: {error}");
            std::process::exit(1);
        }
    };
    let result = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())
        .and_then(|runtime| runtime.block_on(run()));
    let code = match result {
        Ok(()) => 0,
        // An empty message means the command already printed everything it
        // had to say (for example `config check` listing mistakes).
        Err(error) if error.is_empty() => 1,
        Err(error) => {
            eprintln!("{error}");
            1
        }
    };
    startup.exit(code);
}
async fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (trial, command_args) = cli::split_trial_options(&args)?;
    if command_args
        .first()
        .is_some_and(|command| command == "usage-relay")
    {
        let environment: Vec<String> = std::env::vars()
            .map(|(key, value)| format!("{key}={value}"))
            .collect();
        let paths = trial
            .as_ref()
            .map(|options| {
                runtime_context::user_home()
                    .and_then(|home| runtime_context::runtime_paths(Some(options), &home))
            })
            .transpose()
            .map_err(|error| error.to_string())?;
        return many_ai_cli::application::usage_relay::run_native(
            &command_args[1..],
            &environment,
            paths,
        )
        .await
        .map_err(|error| error.to_string());
    }
    // Source custom aliases are resolved only after the known command lookup.
    let loaded = if command_args.first().is_some_and(|command| {
        !cli::BUILTIN_PROVIDERS.contains(&command.as_str())
            && !matches!(
                command.as_str(),
                "version"
                    | "--version"
                    | "-v"
                    | "help"
                    | "--help"
                    | "-h"
                    | "config"
                    | "serve"
                    | "connect"
                    | "setup"
                    | "doctor"
                    | "issue"
                    | "provider"
                    | "wrap"
                    | "shell-init"
                    | "stop"
                    | "status"
                    | "tray"
                    | "profile-export"
                    | "log-clean"
                    | "uninstall"
                    | "usage-relay"
                    | "orchestrate"
            )
    }) {
        Some(main_program::MainContext::load(trial.as_ref()).map_err(|error| error.to_string())?)
    } else {
        None
    };
    let custom = loaded
        .as_ref()
        .map(|context| {
            context.config.snapshot().map(|snapshot| {
                many_ai_cli::config::effective_custom_providers(&snapshot.config.custom_providers)
                    .iter()
                    .map(|provider| provider.id.clone())
                    .collect::<Vec<_>>()
            })
        })
        .transpose()
        .map_err(|error| error.to_string())?
        .unwrap_or_default();
    // A custom provider id is compared trimmed and lower-cased by the refusal,
    // the registry and the launch alike; turn what was typed into that one form
    // here, once, so all of them see the same value (`Twin` and ` twin `
    // are `twin`).
    let mut parse_args = args.clone();
    if let Some(context) = &loaded
        && let Some(position) = args.len().checked_sub(command_args.len())
        && let Some(first) = parse_args.get_mut(position)
    {
        *first =
            many_ai_cli::application::left_out_provider::canonical_provider(&context.config, first);
    }
    let invocation = match cli::parse(&parse_args, &custom) {
        Ok(invocation) => invocation,
        // `many-ai-cli <custom id>` where that id's setting was left out: say why,
        // instead of "unknown command".
        Err(error) => {
            return Err(loaded
                .as_ref()
                .zip(command_args.first())
                .and_then(|(context, command)| {
                    many_ai_cli::application::left_out_provider::wrap_refusal(
                        &context.config,
                        command,
                    )
                })
                .unwrap_or(error));
        }
    };
    match invocation.command {
        Command::Default => {
            let context = main_program::MainContext::load(invocation.trial.as_ref())
                .map_err(|error| error.to_string())?;
            let shutdown = Cancellation::default();
            let _signals = runtime_context::ShutdownSignals::install(shutdown.clone())
                .map_err(|error| error.to_string())?;
            if let Some(port) = main_program::running_port(&context)
                .await
                .map_err(|error| error.to_string())?
            {
                let token = context
                    .config
                    .snapshot()
                    .map_err(|error| error.to_string())?
                    .config
                    .token;
                main_program::open_existing_hub(
                    &context.paths,
                    port,
                    &token,
                    launcher::open_browser,
                )
                .map_err(|error| error.to_string())?;
            } else {
                main_program::HubComposition::new(
                    context,
                    main_program::ServeOptions {
                        open: true,
                        ..Default::default()
                    },
                )
                .await
                .map_err(|error| error.to_string())?
                .run(&shutdown)
                .await
                .map_err(|error| error.to_string())?;
            }
        }
        Command::Version => println!("{}", env!("MANY_AI_BUILD_VERSION")),
        Command::Help => println!("{}\n{}", cli::USAGE, cli::EXTRA_USAGE),
        Command::Config(args) => {
            // Only the runtime paths: this must work when config.yaml cannot be read.
            let home = runtime_context::user_home().map_err(|error| error.to_string())?;
            let paths = runtime_context::runtime_paths(invocation.trial.as_ref(), &home)
                .map_err(|error| error.to_string())?;
            let code =
                many_ai_cli::application::config_cli::run(&paths, &args, &mut std::io::stdout())?;
            if code != 0 {
                return Err(String::new());
            }
        }
        Command::Serve(args) => {
            let options = main_program::ServeOptions::parse(&args)?;
            let context = main_program::MainContext::load(invocation.trial.as_ref())
                .map_err(|error| error.to_string())?;
            let shutdown = Cancellation::default();
            let _signals = runtime_context::ShutdownSignals::install(shutdown.clone())
                .map_err(|error| error.to_string())?;
            main_program::HubComposition::new(context, options)
                .await
                .map_err(|error| error.to_string())?
                .run(&shutdown)
                .await
                .map_err(|error| error.to_string())?;
        }
        Command::Wrap { provider, args } => {
            let mut context = match loaded {
                Some(context) => context,
                None => main_program::MainContext::load(invocation.trial.as_ref())
                    .map_err(|error| error.to_string())?,
            };
            let provider = many_ai_cli::application::left_out_provider::canonical_provider(
                &context.config,
                &provider,
            );
            // A custom provider whose setting was left out says why, before a Hub is started for it.
            if let Some(reason) = many_ai_cli::application::left_out_provider::wrap_refusal(
                &context.config,
                &provider,
            ) {
                return Err(reason);
            }
            let cancel = Cancellation::default();
            let _signals = runtime_context::ShutdownSignals::install(cancel.clone())
                .map_err(|error| error.to_string())?;
            main_program::ensure_hub(&mut context, &cancel)
                .await
                .map_err(|error| error.to_string())?;
            let result = main_program::run_wrap(&context, &provider, &args, &cancel)
                .await
                .map_err(|error| error.to_string())?;
            if result.exit.code != 0 || !result.exit.signal.is_empty() {
                return Err(format!(
                    "wrapper provider exited: code={} signal={}",
                    result.exit.code, result.exit.signal
                ));
            }
        }
        Command::Connect(args) => {
            let home = runtime_context::user_home().map_err(|error| error.to_string())?;
            let paths = runtime_context::runtime_paths(invocation.trial.as_ref(), &home)
                .map_err(|error| error.to_string())?;
            // The fixed main application loads configuration before connect
            // parsing, unlike the independent launcher's help-only path.
            let _config = ConfigStore::load_or_create(paths.clone(), || {
                process::random_token().map_err(ConfigError::from)
            })
            .map_err(|error| error.to_string())?;
            let cwd = std::env::current_dir().map_err(|error| error.to_string())?;
            let connector = launcher::ConnectorConfig::new(paths, cwd).with_console();
            let cancel = Cancellation::default();
            let _signals = runtime_context::ShutdownSignals::install(cancel.clone())
                .map_err(|error| error.to_string())?;
            launcher_program::run_connect(&args, connector, &cancel, &launcher::open_browser)
                .await?;
        }
        Command::ShellInit => {
            let home = runtime_context::user_home().map_err(|error| error.to_string())?;
            let paths = runtime_context::runtime_paths(invocation.trial.as_ref(), &home)
                .map_err(|error| error.to_string())?;
            let config = ConfigStore::load_or_create(paths, || {
                process::random_token().map_err(ConfigError::from)
            })
            .map_err(|error| error.to_string())?;
            let snapshot = config.snapshot().map_err(|error| error.to_string())?;
            print!(
                "{}",
                many_ai_cli::wrapper::shell::init_script_for_config(&snapshot.config)
            );
        }
        Command::ProfileExport(args) => {
            many_ai_cli::application::profile_export::write(
                &args,
                &launcher::ExportIdentity::local(),
                &mut std::io::stdout(),
                &mut std::io::stderr(),
            )?;
        }
        Command::Status => {
            let context = main_program::MainContext::load(invocation.trial.as_ref())
                .map_err(|error| error.to_string())?;
            let config = context
                .config
                .snapshot()
                .map_err(|error| error.to_string())?
                .config;
            many_ai_cli::application::hub_status::write(
                &config,
                &context.paths,
                &mut std::io::stdout(),
            )
            .await
            .map_err(|error| error.to_string())?;
        }
        Command::Doctor(args) => {
            let context = match main_program::MainContext::load(invocation.trial.as_ref()) {
                Ok(context) => context,
                // doctor is how an unreadable settings file gets diagnosed:
                // report that instead of stopping with the same message.
                Err(error) => match many_ai_cli::config::unreadable_from_io(&error) {
                    Some(unreadable) => {
                        many_ai_cli::application::diagnostic_cli::run_unreadable_config(
                            &args,
                            unreadable,
                            &mut std::io::stdout(),
                            &mut std::io::stderr(),
                        )?;
                        return Ok(());
                    }
                    None => return Err(error.to_string()),
                },
            };
            let cancel = Cancellation::default();
            let _signals = runtime_context::ShutdownSignals::install(cancel.clone())
                .map_err(|error| error.to_string())?;
            many_ai_cli::application::diagnostic_cli::run(
                &context,
                &args,
                &cancel,
                &mut std::io::stdout(),
                &mut std::io::stderr(),
            )
            .await?;
        }
        Command::Issue(args) => {
            use many_ai_cli::application::{
                diagnostics::NativeDiagnosticIo,
                issue_cli::{IssueCli, IssueDependencies, NativeIssueIo},
            };
            let context = main_program::MainContext::load(invocation.trial.as_ref())
                .map_err(|e| e.to_string())?;
            let cancel = Cancellation::default();
            let _signals = runtime_context::ShutdownSignals::install(cancel.clone())
                .map_err(|e| e.to_string())?;
            let owner = IssueCli::new(IssueDependencies {
                config: context.config.clone(),
                paths: context.paths.clone(),
                environment: context.environment.clone(),
                version: env!("MANY_AI_BUILD_VERSION").into(),
                platform: if cfg!(windows) {
                    "windows"
                } else if cfg!(target_os = "macos") {
                    "darwin"
                } else {
                    "linux"
                }
                .into(),
                arch: if cfg!(target_arch = "x86_64") {
                    "amd64"
                } else if cfg!(target_arch = "aarch64") {
                    "arm64"
                } else {
                    std::env::consts::ARCH
                }
                .into(),
                runtime_version: "Rust (compiler version unavailable)".into(),
                io: std::sync::Arc::new(NativeIssueIo {
                    actor: NativeDiagnosticIo {
                        paths: context.paths.clone(),
                        cwd: context.cwd.clone(),
                        environment: context.environment.clone(),
                    },
                }),
            });
            owner
                .run(
                    &args,
                    &mut std::io::stdin().lock(),
                    &mut std::io::stdout(),
                    &mut std::io::stderr(),
                    &cancel,
                )
                .await
                .map_err(|e| e.to_string())?;
        }
        Command::Orchestrate(args) => {
            use many_ai_cli::application::orchestrate_cli::{NativeOrchestrateIo, OrchestrateCli};
            let context = main_program::MainContext::load(invocation.trial.as_ref())
                .map_err(|e| e.to_string())?;
            let cancel = Cancellation::default();
            let _signals = runtime_context::ShutdownSignals::install(cancel.clone())
                .map_err(|e| e.to_string())?;
            let owner = OrchestrateCli {
                paths: context.paths.clone(),
                cwd: context.cwd,
                environment: context.environment,
                io: std::sync::Arc::new(NativeOrchestrateIo {
                    paths: context.paths,
                }),
            };
            print!(
                "{}",
                owner.run(&args, &cancel).await.map_err(|e| e.to_string())?
            );
        }
        Command::Stop => {
            let context = main_program::MainContext::load(invocation.trial.as_ref())
                .map_err(|e| e.to_string())?;
            let cancel = Cancellation::default();
            let _signals = runtime_context::ShutdownSignals::install(cancel.clone())
                .map_err(|e| e.to_string())?;
            many_ai_cli::application::maintenance_entry::stop(&context, &cancel)
                .await
                .map_err(|e| e.to_string())?;
        }
        Command::Setup(args) => {
            let flags = many_ai_cli::application::issue_cli::flags::parse(&args, &[])?;
            if flags.help {
                return Ok(());
            }
            {
                let context = main_program::MainContext::load(invocation.trial.as_ref())
                    .map_err(|e| e.to_string())?;
                let cancel = Cancellation::default();
                let _signals = runtime_context::ShutdownSignals::install(cancel.clone())
                    .map_err(|e| e.to_string())?;
                let native = std::sync::Arc::new(
                    many_ai_cli::application::platform_cli::native::NativePlatformIo {
                        actor: many_ai_cli::application::diagnostics::NativeDiagnosticIo {
                            paths: context.paths.clone(),
                            cwd: context.cwd.clone(),
                            environment: context.environment.clone(),
                        },
                    },
                );
                let locations = native
                    .locations(context.executable, context.vendor_home, &cancel)
                    .await
                    .map_err(|e| e.to_string())?;
                let owner = many_ai_cli::application::platform_cli::PlatformCli {
                    paths: context.paths,
                    locations,
                    io: native,
                };
                owner
                    .setup(&mut std::io::stdout(), &cancel)
                    .await
                    .map_err(|e| e.to_string())?;
            }
        }
        Command::Uninstall(args) => {
            use many_ai_cli::application::issue_cli::flags::{self, Kind};
            let flags = flags::parse(&args, &[("purge", Kind::Bool)])?;
            if flags.help {
                return Ok(());
            }
            // Uninstalling needs only the paths and the executable, not the
            // settings, so it also runs when config.yaml cannot be read.
            let (context, unreadable) =
                main_program::MainContext::load_tolerating_unreadable_config(
                    invocation.trial.as_ref(),
                )
                .map_err(|e| e.to_string())?;
            let cancel = Cancellation::default();
            let _signals = runtime_context::ShutdownSignals::install(cancel.clone())
                .map_err(|e| e.to_string())?;
            let record_unreadable =
                many_ai_cli::application::maintenance_entry::hub_record_unreadable(&context)
                    .map_err(|e| e.to_string())?;
            if let Some(unreadable) = &unreadable {
                // Without the settings there is no Hub token, so the Hub cannot
                // be asked to stop, and killing a pid from the ledger could hit
                // an unrelated process. Refuse only if a Hub looks alive.
                if let Some(pid) =
                    many_ai_cli::application::maintenance_entry::live_hub_pid(&context)
                        .map_err(|e| e.to_string())?
                {
                    return Err(format!(
                        "A Hub appears to be running (pid {pid}), but the settings file cannot be read ({}), so it cannot be asked to stop. Stop that process yourself, or fix the file (`many-ai-cli config check`), then run uninstall again.",
                        unreadable.summary_en()
                    ));
                }
                if record_unreadable {
                    println!(
                        "The settings file cannot be read ({}), and the Hub's record (hub-runtime.json) is damaged, so it cannot be told whether a Hub is running.",
                        unreadable.summary_en()
                    );
                } else {
                    println!(
                        "The settings file cannot be read ({}); continuing without it. No Hub is running.",
                        unreadable.summary_en()
                    );
                }
            } else if record_unreadable {
                println!(
                    "The Hub's record (hub-runtime.json) is damaged, so it cannot be told whether a Hub is running."
                );
            } else if main_program::running_port(&context)
                .await
                .map_err(|e| e.to_string())?
                .is_some()
            {
                println!("Hub を停止中...");
                let _ = many_ai_cli::application::maintenance_entry::stop(&context, &cancel).await;
            }
            // Nothing says whether a Hub is running, and killing a process on
            // a guess could hit an unrelated one: leave the stopping to the
            // person and make them say it is done.
            if record_unreadable
                && !many_ai_cli::application::maintenance_entry::confirm_hub_stopped(
                    &mut std::io::stdin().lock(),
                    &mut std::io::stdout(),
                )
                .map_err(|e| e.to_string())?
            {
                println!(
                    "\nUninstall was cancelled. Stop the Hub yourself (end its many-ai-cli process), then run uninstall again."
                );
                return Ok(());
            }
            let native = std::sync::Arc::new(
                many_ai_cli::application::platform_cli::native::NativePlatformIo {
                    actor: many_ai_cli::application::diagnostics::NativeDiagnosticIo {
                        paths: context.paths.clone(),
                        cwd: context.cwd.clone(),
                        environment: context.environment.clone(),
                    },
                },
            );
            let locations = native
                .locations(
                    context.executable.clone(),
                    context.vendor_home.clone(),
                    &cancel,
                )
                .await
                .map_err(|e| e.to_string())?;
            let owner = many_ai_cli::application::platform_cli::PlatformCli {
                paths: context.paths.clone(),
                locations,
                io: native,
            };
            // Release ConfigStore's held Windows root before removing the data
            // directory. The immutable actor retains no directory capability.
            drop(context);
            owner
                .uninstall(
                    flags.boolean("purge"),
                    &mut std::io::stdin().lock(),
                    &mut std::io::stdout(),
                    &cancel,
                )
                .await
                .map_err(|e| e.to_string())?;
        }
        Command::Tray => {
            let context = main_program::MainContext::load(invocation.trial.as_ref())
                .map_err(|e| e.to_string())?;
            let stop = std::sync::Arc::new(
                many_ai_cli::application::maintenance_entry::stop_command(&context)
                    .map_err(|e| e.to_string())?,
            );
            let trial = context.paths.is_trial();
            let hooks =
                std::sync::Arc::new(many_ai_cli::application::tray::native::NativeTrayHooks {
                    paths: context.paths,
                    config: context.config,
                    executable: context.executable,
                    cwd: context.cwd,
                    environment: context.environment,
                    stop,
                });
            many_ai_cli::application::tray::TrayOwner::new(hooks)
                .run(trial)
                .await
                .map_err(|e| e.to_string())?;
        }
        Command::LogClean(args) => {
            let context = main_program::MainContext::load(invocation.trial.as_ref())
                .map_err(|e| e.to_string())?;
            println!(
                "{}",
                many_ai_cli::application::maintenance_entry::log_clean(&context, &args)?.display()
            );
        }
        Command::Provider(args) => {
            let context = main_program::MainContext::load(invocation.trial.as_ref())
                .map_err(|e| e.to_string())?;
            print!(
                "{}",
                many_ai_cli::application::maintenance_entry::provider(&context, &args)
                    .map_err(|e| e.to_string())?
            );
        }
        _ => {
            return Err(
                "Rust migration candidate: this command is not integrated yet; see PROGRESS.md"
                    .into(),
            );
        }
    }
    Ok(())
}
