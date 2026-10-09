use super::{MainContext, ServeOptions, model_cache::LocalModelCache};
use crate::{
    application::{
        event_observer::{ApplicationEventObserver, EventWarning},
        hub_runtime::RuntimeLedger,
        ordinary_spawn::{OrdinarySpawnDependencies, OrdinarySpawnHooks, OrdinarySpawnService},
        session_workers::SessionWorkers,
        spawn_policy::{
            ConfigSpawnLaunchPolicy, NativePathEnvironment, PathEnvironment, RegistrySnapshot,
            SpawnPolicyDependencies,
        },
        wrapped_spawn::{ProcessWrappedSpawner, WrapperSpawnOptions},
    },
    config::{ConfigStore, Resource},
    files::{FilesService, safe_fs::Dir},
    hub::{
        ServiceRouter,
        sockets::{EffectDriver, SocketRegistry},
        task_owner::{HubTaskHandle, HubTaskOwner},
        websocket::WebSocketService,
    },
    orchestration::handoff::HandoffStore,
    process::Cancellation,
    profile::store::ProviderRegistryStore,
    proto::{core::*, time::Timestamp},
    routine::memo::MemoManager,
    storage::SqliteSessionStorage,
    terminal::{
        events::CoreEventBus,
        journal::{JournalOptions, SessionJournal},
        session::{EngineOptions, SessionEngine},
    },
};
use std::{
    io,
    sync::{Arc, OnceLock, Weak},
    time::Duration,
};
#[path = "idle.rs"]
mod idle;
#[path = "info.rs"]
mod info;
#[path = "subscriptions.rs"]
mod subscriptions;
#[path = "workspace.rs"]
mod workspace;

/// Actual constructed owners for further endpoint integrations. A caller can
/// add routes with these same owners; it cannot substitute a second core/store.
pub struct HubCompositionDependencies {
    pub config: Arc<ConfigStore>,
    pub core: Arc<SessionEngine>,
    pub effects: Arc<dyn CoreEffectSink>,
    pub files: Arc<FilesService>,
    pub journal: Arc<SessionJournal>,
    pub registry: Arc<ProviderRegistryStore>,
    pub spawn_policy: Arc<ConfigSpawnLaunchPolicy>,
    pub local_models: Arc<LocalModelCache>,
    pub models: Arc<super::model_catalog::ModelsCatalog>,
    pub workers: Arc<SessionWorkers>,
    pub usage_hooks: Arc<crate::application::usage_hooks::UsageHooks>,
    pub tasks: HubTaskHandle,
    pub ordinary: Arc<OrdinarySpawnService>,
    pub subscriptions: Arc<crate::application::subscriptions::SubscriptionService>,
    pub session_usage: Arc<crate::application::session_usage::SessionUsage>,
    pub cli_updates: Arc<crate::application::cli_updates::CliUpdates>,
    pub notifications: Arc<crate::notify::Manager>,
    pub push: Option<Arc<crate::application::push::PushManager>>,
    pub security: Arc<crate::application::push::security::SecurityNotifications>,
    pub one_tap: Arc<crate::approval::token::OneTapManager>,
    pub approval_rules: Arc<crate::application::approval_rules::ApprovalRules>,
    pub approval_patterns: Arc<crate::application::approval_patterns::ApprovalPatterns>,
    pub instruction_rules: Arc<crate::application::instruction_rules::InstructionRules>,
    pub approval_pattern_io: Arc<dyn crate::application::slash_commands::SlashIo>,
    pub orchestration: Arc<crate::application::orchestration_program::OrchestrationProgram>,
    pub relay: Arc<crate::application::relay_program::RelayProgram>,
    pub routines: Arc<crate::routine::runner::RoutineRunner>,
    pub routine_store: Arc<crate::routine::store::RoutineStore>,
    pub servers: Arc<crate::launcher::ConnectionManager>,
    pub whisper: Arc<crate::application::whisper::WhisperManager>,
}
pub struct HubComposition {
    pub dependencies: HubCompositionDependencies,
    services: ServiceRouter,
    sockets: Arc<SocketRegistry>,
    spawner: Arc<ProcessWrappedSpawner>,
    storage: Option<Arc<SqliteSessionStorage>>,
    owner: HubTaskOwner,
    ledger: RuntimeLedger,
    listener: tokio::net::TcpListener,
    service_cancel: Cancellation,
    open: bool,
    paths: crate::config::RuntimePaths,
    warning: EventWarning,
    logger: Arc<super::logger::HubLogger>,
    shutdown_owner: Arc<crate::hub::lifecycle::ShutdownOwner>,
    routine_cancel: TaskCancellation,
    // Kept after all database-bearing owners in drop order.
    _database_lease: Option<super::ownership::OwnerLease>,
    _runtime_lease: super::ownership::OwnerLease,
}
struct TrialPathEnvironment;
impl PathEnvironment for TrialPathEnvironment {
    fn sanitize(&self, environment: &[String]) -> io::Result<Vec<String>> {
        // Trial PATH is the selected explicit bootstrap input. Unlike native
        // Windows expansion this does not read operator registry/user settings.
        Ok(environment
            .iter()
            .map(|entry| {
                if let Some((key, value)) = entry.split_once('=')
                    && key.eq_ignore_ascii_case("PATH")
                {
                    return format!(
                        "{key}={}",
                        value
                            .split(if cfg!(windows) { ';' } else { ':' })
                            .filter(|part| !part.trim().is_empty())
                            .collect::<Vec<_>>()
                            .join(if cfg!(windows) { ";" } else { ":" })
                    );
                }
                entry.clone()
            })
            .collect())
    }
}
impl HubComposition {
    pub async fn new(mut context: MainContext, options: ServeOptions) -> io::Result<Self> {
        let mut config = context.config.snapshot().map_err(io::Error::other)?.config;
        if let Some(port) = options.port {
            if context.paths.is_trial() && port != context.paths.port() {
                return Err(io::Error::other(
                    "serve port override does not match the trial port",
                ));
            }
            config.hub.port = i64::from(port);
            // Go's serve override is in memory for this process, not saved to
            // config.yaml. Rebuild before owners/readers are published.
            context.config = Arc::new(
                ConfigStore::new(context.paths.clone(), config.clone())
                    .map_err(io::Error::other)?,
            );
        }
        let runtime_lease = super::ownership::OwnerLease::runtime(&context.paths)?;
        let requested = if context.paths.is_trial() {
            context.paths.port()
        } else {
            u16::try_from(config.hub.port)
                .ok()
                .filter(|port| *port > 0)
                .ok_or_else(|| io::Error::other("invalid configured Hub port"))?
        };
        let listener = bind_listener(requested, context.paths.is_trial()).await?;
        let bound_port = listener.local_addr()?.port();
        let paths = context
            .paths
            .clone()
            .with_log_dir_at(std::path::Path::new(&config.hub.log_dir), &context.cwd)?;
        // A configured log directory can point two distinct runtime roots at
        // one physical database. Acquire that directory's lifetime lock before
        // reset/schema/stale-session writes. Lock contention is never degraded.
        let database = paths.resource(Resource::Database);
        let database_lease = if config.hub.log_dir.is_empty() {
            None
        } else {
            match super::ownership::OwnerLease::acquire(
                database
                    .parent()
                    .ok_or_else(|| io::Error::other("database parent unavailable"))?,
                "rust-database-owner.lock",
            ) {
                Ok(lease) => Some(lease),
                Err(error) if error.kind() == io::ErrorKind::AddrInUse => return Err(error),
                Err(_) => None,
            }
        };
        let storage = database_lease.as_ref().and_then(|_| {
            SqliteSessionStorage::open(
                &paths,
                StorageOptions::baseline(paths.resource(Resource::Logs)),
            )
            .ok()
            .map(Arc::new)
        });
        // Fixed Go keeps the Hub available if history initialization fails.
        // Never reset or replace a corrupt database to manufacture success.
        let stale_error = storage.as_ref().and_then(|storage| {
            storage
                .close_stale_sessions(Timestamp::now(), "hub_restart")
                .err()
        });
        let logger = super::logger::HubLogger::new(
            paths.resource(Resource::Logs).as_path(),
            config.log.clone(),
            options.debug,
        )?;
        if storage.is_none() {
            logger.write(
                "WARN",
                "session history disabled",
                "database initialization unavailable",
            );
        } else if stale_error.is_some() {
            logger.write(
                "WARN",
                "stale history cleanup unavailable",
                "history remains open",
            );
        }
        let warning_logger = logger.clone();
        let warning: EventWarning = Arc::new(move |operation, error| {
            warning_logger.write("WARN", operation, &format!("{error:?}"))
        });
        logger.write(
            "DEBUG",
            "Hub composition",
            &format!("port={bound_port} dev={}", options.dev),
        );
        let journal = Arc::new(SessionJournal::new(
            paths.clone(),
            storage
                .clone()
                .map(|storage| storage as Arc<dyn SessionStorage>),
            JournalOptions {
                session_enabled: config.log.session_enabled,
                max_bytes: config.log.session_max_size_mb.saturating_mul(1024 * 1024),
            },
        ));
        let warning_copy = warning.clone();
        journal.set_warning_handler(Arc::new(move |_, error| {
            warning_copy("session journal", error)
        }));
        let files = Arc::new(
            FilesService::new(context.cwd.clone(), paths.clone()).with_history(journal.clone()),
        );
        let registry = Arc::new(
            ProviderRegistryStore::new(&paths, context.config.clone()).map_err(io::Error::other)?,
        );
        let registry_copy = registry.clone();
        let registry_snapshot: RegistrySnapshot = Arc::new(move || {
            registry_copy
                .snapshot()
                .map(|snapshot| snapshot.registry.as_ref().clone())
                .map_err(io::Error::other)
        });
        let local_models = Arc::new(LocalModelCache::default());
        let catalog_io = Arc::new(super::model_catalog::native::NativeCatalogIo::new(
            paths.clone(),
            context.environment.clone(),
            context.cwd.clone(),
            local_models.clone(),
        ));
        let models = Arc::new(super::model_catalog::ModelsCatalog::new(
            context.config.clone(),
            catalog_io.clone(),
        ));
        let model_owner = local_models.clone();
        let path_environment: Arc<dyn PathEnvironment> = if paths.is_trial() {
            Arc::new(TrialPathEnvironment)
        } else {
            Arc::new(NativePathEnvironment)
        };
        let spawn_policy = Arc::new(ConfigSpawnLaunchPolicy::new(
            paths.clone(),
            context.config.clone(),
            SpawnPolicyDependencies {
                registry: registry_snapshot.clone(),
                local_models: Arc::new(move || model_owner.snapshot()),
                path_environment: path_environment.clone(),
                vendor_home: context.vendor_home.clone(),
                hub_cwd: context.cwd.clone(),
                seeded: {
                    let logger = logger.clone();
                    Arc::new(move |_| {
                        logger.write("INFO", "subscription profile seed completed", "")
                    })
                },
                trusted: {
                    let logger = logger.clone();
                    Arc::new(move |_, result| {
                        if result.is_err() {
                            logger.write("WARN", "folder trust was not written", "");
                        }
                    })
                },
            },
        )?);
        let owner = HubTaskOwner::new(tokio::runtime::Handle::current());
        let tasks = owner.handle();
        let push = match crate::application::push::PushManager::new(
            &paths,
            tasks.clone(),
            Arc::new(crate::application::push::native::NativePushHttpClient::new(
                &paths,
            )),
            {
                let logger = logger.clone();
                Arc::new(move |operation, hash| logger.write("WARN", operation, hash))
            },
        ) {
            Ok(manager) => Some(manager),
            Err(_) => {
                logger.write(
                    "WARN",
                    "web push disabled",
                    "private store or key generation unavailable",
                );
                None
            }
        };
        let notifications = crate::notify::Manager::new_for_runtime(
            config.notify.clone(),
            tasks.clone(),
            {
                let logger = logger.clone();
                Arc::new(move |operation, detail| logger.write("WARN", operation, detail))
            },
            &paths,
        )
        .map_err(io::Error::other)?;
        let security = crate::application::push::security::SecurityNotifications::new(
            notifications.clone(),
            push.clone(),
            {
                let logger = logger.clone();
                Arc::new(move |operation| logger.write("WARN", operation, ""))
            },
        );
        let one_tap = Arc::new(
            crate::approval::token::OneTapManager::new()
                .map_err(|_| io::Error::other("one-tap signing randomness unavailable"))?,
        );
        let audit_log =
            Dir::open_or_create_private_components(paths.resource(Resource::Logs).as_path())
                .and_then(|dir| {
                    crate::logging::RollingLog::new(Arc::new(dir), "auto-approval.jsonl")
                })
                .ok();
        let audit_config = context.config.clone();
        let approval_rules = crate::application::approval_rules::ApprovalRules::new(
            context.config.clone(),
            paths.clone(),
            tasks.clone(),
            warning.clone(),
            Arc::new(move |record| {
                let config = audit_config.snapshot().map_err(io::Error::other)?.config;
                let mut line = serde_json::to_vec(record).map_err(io::Error::other)?;
                line.push(b'\n');
                audit_log
                    .as_ref()
                    .ok_or_else(|| io::Error::other("approval audit log unavailable"))?
                    .write_and_close(&config.log, &line)
            }),
        );
        let bound_core = Arc::new(OnceLock::<Weak<SessionEngine>>::new());
        let bound_effects = Arc::new(OnceLock::<Weak<dyn CoreEffectSink>>::new());
        let bound_ordinary = Arc::new(OnceLock::<Weak<OrdinarySpawnService>>::new());
        let failed_core = bound_core.clone();
        let failed_effects = bound_effects.clone();
        let failure_tasks = tasks.clone();
        let failure_warning = warning.clone();
        let failure_ordinary = bound_ordinary.clone();
        let spawner = Arc::new(ProcessWrappedSpawner::new(
            WrapperSpawnOptions {
                paths: paths.clone(),
                executable: context.executable.clone(),
                application_home: context.application_home.clone(),
                environment: context.environment.clone(),
                hub_port: bound_port,
                parent_shell: context.shell.clone(),
                pending_capacity: 256,
                reap_timeout: Duration::from_secs(5),
            },
            spawn_policy.clone(),
            Arc::new(move |attempt, provider| {
                let core = failed_core
                    .get()
                    .and_then(Weak::upgrade)
                    .ok_or(SessionError::Shutdown)?;
                let effects = failed_effects
                    .get()
                    .and_then(Weak::upgrade)
                    .ok_or(SessionError::Shutdown)?;
                let metadata = core.peek_spawn_metadata(attempt, provider);
                let actions = core.spawn_failed(attempt, provider)?;
                let warning = failure_warning.clone();
                let ordinary = failure_ordinary.get().and_then(Weak::upgrade);
                drop(failure_tasks.effect_permit()?.start(async move {
                    if let Err(failure) = effects.apply(actions).await {
                        warning("spawn failure effects", &failure.error);
                    }
                    if let (Some(ordinary), Some(metadata)) = (ordinary, metadata) {
                        ordinary.startup_failed(&metadata).await;
                    }
                }));
                Ok(())
            }),
            {
                let logger = logger.clone();
                Arc::new(move |message| logger.write("WARN", "wrapped spawn", message))
            },
        )?);
        let events = CoreEventBus::new(4096).map_err(session_error)?;
        let sockets = Arc::new(SocketRegistry::default());
        let service_cancel = Cancellation::default();
        let shutdown_owner = Arc::new(crate::hub::lifecycle::ShutdownOwner::new(
            service_cancel.clone(),
            sockets.clone(),
            tasks.clone(),
        ));
        let workers = SessionWorkers::new(
            context.config.clone(),
            paths.clone(),
            files.clone(),
            tasks.clone(),
            warning.clone(),
        );
        workers
            .set_observation_registry(registry_snapshot.clone())
            .map_err(session_error)?;
        let native_temporary = context
            .environment
            .iter()
            .rev()
            .find_map(|entry| {
                entry
                    .split_once('=')
                    .filter(|(name, _)| {
                        if cfg!(windows) {
                            name.eq_ignore_ascii_case("TEMP")
                        } else {
                            *name == "TMPDIR"
                        }
                    })
                    .map(|(_, value)| std::path::PathBuf::from(value))
            })
            .unwrap_or_else(std::env::temp_dir);
        workers
            .set_observation_temporary(&native_temporary)
            .map_err(session_error)?;
        let routine_store = Arc::new(crate::routine::store::RoutineStore::open(Arc::new(
            Dir::open_or_create_private_components(paths.root())?,
        )));
        if !routine_store.ready() {
            logger.write(
                "WARN",
                "routine store unavailable",
                "persisted routine file could not be loaded",
            );
        }
        {
            let store = routine_store.clone();
            workers
                .set_active_routine_probe(Arc::new(move |label, at| {
                    store.active_session(label, at)
                }))
                .map_err(session_error)?;
        }
        let routine_callbacks =
            super::routines::RoutineCallbacks::new(workers.clone(), warning.clone());
        let observer = Arc::new(
            ApplicationEventObserver::new(
                context.config.clone(),
                paths.clone(),
                files.clone(),
                routine_callbacks.clone(),
                warning.clone(),
                tasks.clone(),
            )
            .with_push(push.clone())
            .with_notifications(notifications.clone())
            .with_one_tap(one_tap.clone())
            .with_approval_rules(approval_rules.clone()),
        );
        let login_observer = Arc::new(subscriptions::LoginObserver::new(
            observer.clone(),
            tasks.clone(),
            warning.clone(),
        ));
        let instruction_observer = super::instruction_observer::InstructionObserver::new(
            login_observer.clone(),
            tasks.clone(),
            service_cancel.clone(),
        );
        let effects: Arc<dyn CoreEffectSink> = Arc::new(
            EffectDriver::new(
                sockets.clone(),
                journal.clone(),
                Arc::new(events.clone()),
                instruction_observer.clone(),
            )
            .with_warning_handler(warning.clone()),
        );
        let child_executor = Arc::new(super::confirmations::BoundConfirmationExecutor::default());
        let hub_instance = crate::process::random_token()?;
        let model_policy = spawn_policy.clone();
        let model_warning = warning.clone();
        let core = Arc::new(SessionEngine::new(
            EngineOptions {
                subscription_name: {
                    let config = context.config.clone();
                    let warning = warning.clone();
                    Arc::new(move |provider, id| match config.snapshot() {
                        Ok(snapshot) => crate::config::find_subscription(
                            &snapshot.config.subscriptions,
                            provider,
                            id,
                        )
                        .map(|profile| profile.name.trim().to_owned())
                        .unwrap_or_default(),
                        Err(_) => {
                            warning(
                                "subscription display name",
                                &SessionError::InvalidRequest(
                                    "subscription configuration unavailable".into(),
                                ),
                            );
                            String::new()
                        }
                    })
                },
                branch_lookup: {
                    let source = files.branch_reader();
                    Arc::new(move |cwd| {
                        let source = source.clone();
                        Box::pin(async move { source.branch(&cwd).await })
                    })
                },
                model_route: Arc::new(move |provider, model| {
                    model_policy
                        .detected_model_route(provider, model)
                        .unwrap_or_else(|_| {
                            model_warning(
                                "detected model route",
                                &SessionError::InvalidRequest(
                                    "model route configuration unavailable".into(),
                                ),
                            );
                            String::new()
                        })
                }),
                hub_instance: hub_instance.clone(),
                token_statusbar: config.user_prefs.token_statusbar.enabled.unwrap_or(true),
                custom_providers: crate::config::effective_custom_providers(
                    &config.custom_providers,
                )
                .iter()
                .map(|provider| provider.id.clone())
                .collect(),
                warning: warning.clone(),
                approval_policy: Some(approval_rules.policy.live_callback()),
                confirmation_executor: Some(child_executor.clone()),
                ..Default::default()
            },
            journal.clone(),
            sockets.clone(),
            effects.clone(),
            spawner.clone(),
            events,
        ));
        bound_core
            .set(Arc::downgrade(&core))
            .map_err(|_| io::Error::other("core already bound"))?;
        bound_effects
            .set(Arc::downgrade(&effects))
            .map_err(|_| io::Error::other("effects already bound"))?;
        let trait_core: Arc<dyn SessionCore> = core.clone();
        let instruction_rules = crate::application::instruction_rules::InstructionRules::new(
            paths.clone(),
            context.vendor_home.clone(),
            context.environment.clone(),
            context.cwd.clone(),
            context.config.clone(),
            core.clone(),
            Arc::new(
                crate::application::instruction_rules::NativeInstructionRootResolver::new(
                    paths.clone(),
                    context.cwd.clone(),
                    context.environment.clone(),
                    service_cancel.clone(),
                )?,
            ),
            {
                let logger = logger.clone();
                Arc::new(move |operation| {
                    logger.write("WARN", operation, "instruction operation failed")
                })
            },
        )?;
        instruction_rules.recover().await;
        instruction_observer
            .bind(Arc::downgrade(&instruction_rules))
            .map_err(session_error)?;
        let usage_hooks = crate::application::usage_hooks::UsageHooks::new(
            context.config.clone(),
            core.clone(),
            paths.clone(),
            &context.vendor_home,
            &context.environment,
            &context.cwd,
            &context.executable,
            bound_port,
            {
                let logger = logger.clone();
                Arc::new(move |message| logger.write("WARN", "usage hooks", message))
            },
        )?;
        observer
            .bind_usage_hooks(&usage_hooks)
            .map_err(session_error)?;
        let approval_warning = warning.clone();
        let approval_actions = Arc::new(crate::hub::approval_actions::ApprovalActionHttp::new(
            trait_core.clone(),
            effects.clone(),
            Arc::new(owner.approval_effects()),
            one_tap.clone(),
            Arc::new(Timestamp::now),
            Arc::new(move |failure| approval_warning("approval action effects", &failure.error)),
        ));
        sockets
            .bind_core(Arc::downgrade(&trait_core))
            .map_err(session_error)?;
        observer
            .bind(Arc::downgrade(&core), Arc::downgrade(&effects))
            .map_err(session_error)?;
        workers
            .bind(Arc::downgrade(&core), Arc::downgrade(&effects))
            .map_err(session_error)?;
        {
            let core = Arc::downgrade(&core);
            let push = push.as_ref().map(Arc::downgrade);
            let config = context.config.clone();
            workers
                .set_workflow_completion(Arc::new(move |binding, progress| {
                    let (Some(core), Some(push)) =
                        (core.upgrade(), push.as_ref().and_then(Weak::upgrade))
                    else {
                        return Ok(());
                    };
                    let enabled = config
                        .snapshot()
                        .map_err(|_| {
                            SessionError::InvalidRequest(
                                "workflow notification configuration unavailable".into(),
                            )
                        })?
                        .config
                        .user_prefs
                        .workflow_completion_notify
                        .enabled;
                    push.notify_workflow_completion(&core, binding, progress, enabled)
                }))
                .map_err(session_error)?;
        }
        approval_rules
            .bind(Arc::downgrade(&core), Arc::downgrade(&effects))
            .map_err(session_error)?;
        let orchestration = crate::application::orchestration_program::OrchestrationProgram::new(
            crate::application::orchestration_program::OrchestrationDependencies {
                core: Arc::downgrade(&core),
                effects: Arc::downgrade(&effects),
                config: context.config.clone(),
                paths: paths.clone(),
                registry: registry_snapshot.clone(),
                environment: context.environment.clone(),
                hub_cwd: context.cwd.clone(),
                workers: workers.clone(),
                tasks: tasks.clone(),
                warning: warning.clone(),
            },
        )
        .map_err(session_error)?;
        observer
            .bind_orchestration(&orchestration)
            .map_err(session_error)?;
        let renderer = Arc::downgrade(&orchestration);
        workers
            .set_conductor_renderer(Arc::new(move |id| {
                Ok(renderer
                    .upgrade()
                    .ok_or(SessionError::Shutdown)?
                    .conductor_prompt(id))
            }))
            .map_err(session_error)?;
        let preparer = crate::orchestration::child_launch::ChildPreparer::new(
            &paths,
            context.cwd.clone(),
            Some(context.vendor_home.clone()),
            crate::orchestration::child_launch::worktree::WorktreeGit::new(
                files.git_executable.clone(),
                files.git_environment.clone(),
            ),
        )
        .map_err(|error| io::Error::other(error.detail))?;
        let actual_child_executor = Arc::new(
            crate::orchestration::child_launch::ChildLaunchExecutor::new(
                Arc::downgrade(&core),
                context.config.clone(),
                preparer.with_board_store(orchestration.board_store()),
                orchestration.clone(),
            ),
        );
        child_executor
            .bind(actual_child_executor.clone())
            .map_err(session_error)?;
        let relay = crate::application::relay_program::RelayProgram::new(
            crate::application::relay_program::RelayDependencies {
                core: Arc::downgrade(&core),
                effects: Arc::downgrade(&effects),
                config: context.config.clone(),
                paths: paths.clone(),
                orchestration: orchestration.clone(),
                executor: actual_child_executor,
                git: crate::orchestration::child_launch::worktree::WorktreeGit::new(
                    files.git_executable.clone(),
                    files.git_environment.clone(),
                ),
                hub_cwd: context.cwd.clone(),
                tasks: tasks.clone(),
                warning: warning.clone(),
            },
        )
        .map_err(session_error)?;
        if relay.restore(Timestamp::now()).await.is_err() {
            logger.write(
                "WARN",
                "relay restore unavailable",
                "existing relay records retained for recovery",
            );
        }
        let routine_cancel = TaskCancellation::default();
        let routine_warning = warning.clone();
        let routines = Arc::new(
            crate::routine::runner::RoutineRunner::new(
                routine_store.clone(),
                trait_core.clone(),
                context.vendor_home.clone(),
                hub_instance,
                routine_cancel.clone(),
                Arc::new(move |operation, _| {
                    routine_warning(
                        operation,
                        &SessionError::InvalidRequest("routine operation unavailable".into()),
                    )
                }),
            )
            .with_preparation(Arc::new(super::routines::RoutinePreparation {
                paths: paths.clone(),
                orchestration: orchestration.clone(),
                warning: warning.clone(),
            })),
        );
        routine_callbacks.bind(&routines).map_err(session_error)?;
        let memos = Arc::new(MemoManager::open(Arc::new(
            Dir::open_or_create_private_components(paths.root())?,
        )));
        let workspace = Arc::new(workspace::Workspace {
            memos: memos.clone(),
            handoff: HandoffStore::new(paths.clone()),
        });
        let hints = Arc::new(crate::hub::runtime_routes::NetHints::default());
        let jev = Arc::new(crate::hub::jev_routes::JevHttp::new(
            super::context::environment_value(&context.environment, "TYPESAFE_API_KEY")
                .unwrap_or("")
                .to_owned(),
            Arc::new(crate::hub::jev_routes::NativeJev::new(paths.clone())),
        ));
        let links = crate::application::link_defaults::LinkDefaults::new(Arc::new(
            crate::application::link_defaults::NativeLinkIo::new(paths.clone())?,
        ));
        let info = info::factory(&context, registry.clone(), hints.clone())?;
        let sessions = Arc::new(crate::hub::session_routes::SessionHttp::new(
            trait_core.clone(),
            journal.clone(),
        ));
        let ordinary = Arc::new(
            OrdinarySpawnService::new(OrdinarySpawnDependencies {
                core: core.clone(),
                config: context.config.clone(),
                policy: spawn_policy.clone(),
                registry: registry_snapshot.clone(),
                worktrees: crate::orchestration::normal_worktree::NormalWorktreeLifecycle::new(
                    context.cwd.clone(),
                    crate::orchestration::child_launch::worktree::WorktreeGit::new(
                        files.git_executable.clone(),
                        files.git_environment.clone(),
                    ),
                ),
                hooks: Arc::new(OrdinaryHooks(
                    workers.clone(),
                    logger.clone(),
                    usage_hooks.clone(),
                )),
                hub_cwd: context.cwd.clone(),
                home: context.vendor_home.clone(),
                environment: context.environment.clone(),
                tasks: tasks.clone(),
                orchestration: Some(orchestration.clone()),
            })
            .map_err(|error| io::Error::other(error.detail))?,
        );
        bound_ordinary
            .set(Arc::downgrade(&ordinary))
            .map_err(|_| io::Error::other("ordinary spawn already bound"))?;
        let probe_hooks = Arc::new(subscriptions::ProbeHooks {
            core: Arc::downgrade(&core),
            effects: Arc::downgrade(&effects),
            ordinary: Arc::downgrade(&ordinary),
            trial: paths.is_trial(),
        });
        login_observer.bind(Arc::downgrade(&probe_hooks))?;
        let subscription_cli = Arc::new(
            crate::application::subscriptions::cli::NativeSubscriptionCli::new(
                paths.clone(),
                context.cwd.clone(),
                context.environment.clone(),
            ),
        );
        let subscriptions = crate::application::subscriptions::SubscriptionService::new(
            crate::application::subscriptions::SubscriptionDependencies {
                paths: paths.clone(),
                home: context.vendor_home.clone(),
                config: context.config.clone(),
                launcher: Arc::new(crate::profile::subscriptions::SubscriptionLauncher::new(
                    paths.clone(),
                    context.vendor_home.clone(),
                    context.cwd.clone(),
                )),
                core: core.clone(),
                cli: subscription_cli.clone(),
                environment: context.environment.clone(),
                tasks: tasks.clone(),
                warning: {
                    let logger = logger.clone();
                    Arc::new(move |operation| logger.write("WARN", operation, "subscriptions"))
                },
                probe_hooks,
            },
        );
        {
            let service = Arc::downgrade(&subscriptions);
            spawner.set_probe_start_observer(Arc::new(move |label, pid| {
                if let Some(service) = service.upgrade() {
                    service.update_probe_pid(label, pid);
                }
            }))?;
        }
        subscriptions.recover_probes();
        let diagnostic_io = Arc::new(crate::application::diagnostics::NativeDiagnosticIo {
            paths: paths.clone(),
            cwd: context.cwd.clone(),
            environment: context.environment.clone(),
        });
        let nvidia = crate::application::nvidia_nim::NvidiaNim::new(
            crate::application::nvidia_nim::NvidiaDependencies {
                paths: paths.clone(),
                config: context.config.clone(),
                environment: context.environment.clone(),
                io: Arc::new(crate::application::nvidia_nim::NativeNvidiaIo {
                    paths: paths.clone(),
                }),
            },
        )?;
        let doctor = crate::application::diagnostics::Diagnostics::new(
            crate::application::diagnostics::DiagnosticsDependencies {
                config: context.config.clone(),
                paths: paths.clone(),
                cwd: context.cwd.clone(),
                home: context.vendor_home.clone(),
                environment: context.environment.clone(),
                platform: std::env::consts::OS.into(),
                io: diagnostic_io.clone(),
                subscription_cli,
                nvidia_key_configured: {
                    let catalog_io = catalog_io.clone();
                    Arc::new(move || {
                        use super::model_catalog::CatalogIo;
                        catalog_io.nvidia_key().map(|key| !key.is_empty())
                    })
                },
            },
        );
        let bug_report = crate::application::diagnostics::bug_report::BugReport::new(
            crate::application::diagnostics::bug_report::BugReportDependencies {
                config: context.config.clone(),
                paths: paths.clone(),
                core: core.clone(),
                version: env!("MANY_AI_BUILD_VERSION").into(),
                platform: std::env::consts::OS.into(),
                arch: std::env::consts::ARCH.into(),
                runtime_version: "Rust (compiler version unavailable)".into(),
                io: diagnostic_io,
            },
        );
        let mut mobile_identity = crate::launcher::ExportIdentity::local();
        let actor_env = |name: &str| {
            context
                .environment
                .iter()
                .rev()
                .find_map(|entry| {
                    entry
                        .split_once('=')
                        .filter(|(key, _)| {
                            if cfg!(windows) {
                                key.eq_ignore_ascii_case(name)
                            } else {
                                *key == name
                            }
                        })
                        .map(|(_, value)| value.to_owned())
                })
                .unwrap_or_default()
        };
        mobile_identity.username = actor_env("USERNAME");
        mobile_identity.user = actor_env("USER");
        mobile_identity.public_host = actor_env("MANY_AI_CLI_PUBLIC_HOST");
        mobile_identity.ssh_connection = actor_env("SSH_CONNECTION");
        mobile_identity.cwd = context.cwd.to_string_lossy().into_owned();
        mobile_identity.executable = context.executable.to_string_lossy().into_owned();
        let mobile = crate::application::mobile_connect::MobileConnect::new(
            context.config.clone(),
            Arc::new(
                crate::application::mobile_connect::native::NativeMobileIo::new(
                    paths.clone(),
                    context.environment.clone(),
                    context.cwd.clone(),
                    context.vendor_home.clone(),
                    service_cancel.clone(),
                )?,
            ),
            crate::application::mobile_connect::MobileIdentity::from_export(
                mobile_identity,
                &context.environment,
            ),
            bound_port,
            {
                let logger = logger.clone();
                Arc::new(move |operation| logger.write("WARN", operation, "mobile connection"))
            },
        );
        let session_usage = crate::application::session_usage::SessionUsage::new(
            core.clone(),
            effects.clone(),
            subscriptions.clone(),
            tasks.clone(),
            warning.clone(),
        );
        let provider_cwd = context.cwd.clone();
        let provider_environment = context.environment.clone();
        let cli_cwd = context.cwd.clone();
        let versions = Arc::new(crate::hub::cli_version::CliVersionHttp::new(
            crate::hub::cli_version::CliVersionDependencies {
                executor: Some(Arc::new(
                    crate::hub::cli_version::native::NativeCliVersionExecutor::new(
                        paths.clone(),
                        crate::hub::cli_version::native::TrialVersionPolicy::Disabled,
                    ),
                )),
                cwd: context.cwd.clone(),
                environment: context.environment.clone(),
                platform: crate::process::execpath::Platform::native(),
                path_environment,
                executable_fs: Arc::new(crate::process::execpath::NativeFs),
                modified_at: Arc::new(move |executable| {
                    let path = std::path::Path::new(executable);
                    let path = if path.is_absolute() {
                        path.to_owned()
                    } else {
                        cli_cwd.join(path)
                    };
                    Timestamp::from_system_time(std::fs::metadata(path).ok()?.modified().ok()?).ok()
                }),
                clock: Arc::new(Timestamp::now),
            },
        ));
        let cli_updates = crate::application::cli_updates::CliUpdates::new(
            crate::application::cli_updates::Dependencies {
                core: core.clone(),
                store: registry.clone(),
                versions: versions.clone(),
                paths: paths.clone(),
                cwd: context.cwd.clone(),
                environment: context.environment.clone(),
                tasks: tasks.clone(),
                resolve: {
                    let environment = context.environment.clone();
                    let cwd = context.cwd.clone();
                    Arc::new(move |name| {
                        crate::process::execpath::Resolver::new(
                            crate::process::execpath::Platform::native(),
                            &environment,
                            &cwd,
                            &crate::process::execpath::NativeFs,
                        )
                        .look_path(name)
                        .ok()
                        .map(std::path::PathBuf::from)
                    })
                },
                log_directory: {
                    let config = context.config.clone();
                    let selected = paths.clone();
                    Arc::new(move || {
                        let snapshot = config.snapshot().map_err(io::Error::other)?;
                        Ok(selected
                            .clone()
                            .with_log_dir(std::path::Path::new(&snapshot.config.hub.log_dir))?
                            .resource(Resource::Update))
                    })
                },
                executor: Arc::new(crate::application::cli_updates::NativeUpdateExecutor),
                warning: warning.clone(),
            },
        );
        let settings_core = core.clone();
        let settings_journal = journal.clone();
        let slash = crate::application::slash_commands::SlashCommands::new(
            context.config.clone(),
            paths.clone(),
            Arc::downgrade(&core),
            tasks.clone(),
            context.environment.clone(),
            Some(context.vendor_home.clone()),
            warning.clone(),
        )?;
        let approval_patterns = crate::application::approval_patterns::ApprovalPatterns::new(
            &paths,
            context.config.clone(),
            Arc::downgrade(&core),
            {
                let logger = logger.clone();
                Arc::new(move |operation| logger.write("WARN", operation, "approval patterns"))
            },
        )?;
        if approval_patterns.sync().is_err() {
            logger.write(
                "WARN",
                "approval pattern initialization failed",
                "private patterns unavailable",
            );
        }
        let approval_pattern_io: Arc<dyn crate::application::slash_commands::SlashIo> =
            Arc::new(crate::application::slash_commands::NativeSlashIo::new(
                paths.clone(),
                context.environment.clone(),
                Some(context.vendor_home.clone()),
            )?);
        let settings_notifications = notifications.clone();
        let mut connector =
            crate::launcher::ConnectorConfig::new(paths.clone(), context.cwd.clone());
        connector.environment = context
            .environment
            .iter()
            .filter_map(|entry| {
                entry
                    .split_once('=')
                    .map(|(key, value)| (key.into(), Some(value.into())))
            })
            .collect();
        let servers = crate::launcher::ConnectionManager::new(
            crate::launcher::LauncherStore::open(paths.clone())?,
            connector,
        );
        let whisper = crate::application::whisper::WhisperManager::new(
            crate::application::whisper::WhisperDependencies {
                paths: paths.clone(),
                config: context.config.clone(),
                tasks: tasks.clone(),
                environment: context.environment.clone(),
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
                io: Arc::new(crate::application::whisper::native::NativeWhisperIo {
                    paths: paths.clone(),
                }),
                warning: warning.clone(),
                // Candidate Windows builds prepare the same verified VS Redist
                // inputs as Go and retain their actual version/hash receipt.
                runtime_payload: crate::assets::windows_runtime_payload(),
            },
        )
        .map_err(io::Error::other)?;
        let host_dispatch = crate::application::host_actions::NativeHostDispatch::new(
            paths.clone(),
            context.cwd.clone(),
            context.environment.clone(),
            tasks.clone(),
        )?;
        let agent_history = crate::application::agent_history::AgentHistory::new(
            core.clone(),
            paths.clone(),
            context.vendor_home.clone(),
            host_dispatch.clone(),
            {
                let logger = logger.clone();
                Arc::new(move |operation| {
                    logger.write("WARN", operation, "agent history operation failed")
                })
            },
        );
        // Opt-in is a host process setting, never a browser/API-supplied path.
        // No directory or receipt database is touched in the default-off mode.
        let external_notice = match super::context::environment_value(
            &context.environment,
            "MANY_AI_CLI_EXTERNAL_NOTICE_STATE",
        ) {
            Some(directory) if !directory.is_empty() => Some(Arc::new(
                crate::hub::external_notice::ExternalNoticeHttp::open(
                    core.clone(),
                    std::path::Path::new(directory),
                )?,
            )),
            _ => None,
        };
        let mut services = ServiceRouter::new(
            context.config.clone(),
            paths.clone(),
            trait_core,
            bound_port,
        )?
        .with_task_owner(tasks.clone())
        .with_instruction_rules(Arc::new(
            crate::hub::approval_status_routes::ApprovalStatusHttp::new(instruction_rules.clone()),
        ))
        .with_grid(Arc::new(crate::hub::grid_spawn_routes::GridHttp::new(
            Arc::new(crate::application::grid_spawn::GridSpawn::new(
                crate::application::grid_spawn::Dependencies {
                    core: core.clone(),
                    registry: registry_snapshot.clone(),
                    hub_cwd: context.cwd.clone(),
                    home: context.vendor_home.clone(),
                    environment: context.environment.clone(),
                    tasks: tasks.clone(),
                },
            )),
        )))
        .with_agent_history(Arc::new(
            crate::hub::agent_history_routes::AgentHistoryHttp::new(agent_history),
        ))
        .with_approval_patterns(
            crate::hub::approval_pattern_routes::ApprovalPatternHttp::new(
                approval_patterns.clone(),
                context.config.clone(),
            ),
        )
        .with_diagnostics(crate::hub::diagnostic_routes::DiagnosticHttp::new(
            doctor, bug_report,
        ))
        .with_mobile(Arc::new(crate::hub::mobile_routes::MobileHttp::new(mobile)))
        .with_nvidia(crate::hub::nvidia_routes::NvidiaHttp::new(nvidia))
        .with_whisper(Arc::new(crate::hub::whisper_routes::WhisperHttp::new(
            whisper.clone(),
        )))
        .with_voice(Arc::new(crate::hub::voice_routes::VoiceHttp::new(
            context.config.clone(),
            Arc::new(whisper.clone()),
        )))
        .with_servers(Arc::new(crate::hub::server_routes::ServerHttp::new(
            servers.clone(),
        )))
        .with_models(models.clone())
        .with_provider_assets(
            crate::hub::icon_routes::IconHttp::new(
                crate::application::provider_assets::ProviderAssets::new(
                    &paths,
                    registry.clone(),
                    {
                        let logger = logger.clone();
                        Arc::new(move |_, _| {
                            logger.write("WARN", "provider icon", "provider icon operation failed")
                        })
                    },
                ),
            ),
            crate::hub::distribution_routes::DistributionHttp::new(&paths, registry.clone()),
        )
        .with_slash_commands(Arc::new(crate::hub::slash_routes::SlashHttp::new(slash)))
        .with_host(Arc::new(crate::hub::host_routes::HostHttp::new(
            paths.clone(),
            context.config.clone(),
            core.clone(),
            files.clone(),
            host_dispatch,
            Some(context.vendor_home.clone()),
        )))
        .with_shutdown(shutdown_owner.clone())
        .with_confirmations(Arc::new(crate::hub::confirmations::ConfirmationHttp::new(
            core.clone(),
        )))
        .with_children(Arc::new(crate::hub::child_routes::ChildHttp::new(
            core.clone(),
            orchestration.clone(),
            child_executor.clone(),
            effects.clone(),
            context.config.clone(),
            tasks.clone(),
        )))
        .with_relay(Arc::new(crate::hub::relay_routes::RelayHttp::new(
            relay.clone(),
        )))
        .with_child_controls(Arc::new(crate::hub::child_control::ChildControl::new(
            core.clone(),
            orchestration.clone(),
            tasks.clone(),
            warning.clone(),
        )))
        .with_routines(Arc::new(crate::hub::routines::RoutineHttp {
            store: routine_store.clone(),
            runner: Some(routines.clone()),
            home: context.vendor_home.clone(),
        }))
        .with_memos(memos)
        .with_settings_published(Arc::new(move |path, config| {
            if path == "/api/idle-timeout" {
                settings_core.restart_ui_idle_timer();
            }
            settings_notifications.update_config(config.notify.clone());
            if path == "/api/log-config" {
                settings_journal.set_enabled(config.log.session_enabled);
            }
            if path == "/api/user-prefs" || path == "/api/orchestration-config" {
                let _ = instruction_observer.settings_published();
            }
        }))
        .with_files(
            files.clone(),
            storage
                .clone()
                .map(|storage| storage as Arc<dyn SessionStorage>),
            workspace,
        )
        .with_session_routes(sessions, info, effects.clone())
        .with_provider_routes(
            registry.clone(),
            Arc::new(move |command| {
                crate::process::execpath::Resolver::new(
                    crate::process::execpath::Platform::native(),
                    &provider_environment,
                    &provider_cwd,
                    &crate::process::execpath::NativeFs,
                )
                .look_path(command)
                .is_ok()
            }),
            {
                let logger = logger.clone();
                Arc::new(move |operation, _, _| {
                    logger.write("WARN", operation, "provider operation failed")
                })
            },
        )
        .with_subscriptions(Arc::new(
            crate::hub::subscription_routes::SubscriptionHttp::new(subscriptions.clone()),
        ))
        .with_session_usage(Arc::new(crate::hub::usage_routes::UsageHttp::new(
            session_usage.clone(),
        )))
        .with_cli_updates(Arc::new(crate::hub::update_routes::UpdateHttp::new(
            cli_updates.clone(),
        )))
        .with_cli_versions(versions)
        .with_runtime_observations(hints, context.shell.clone())
        .with_link_defaults(links)
        .with_jev(jev)
        .with_security(security.clone())
        .with_push(Arc::new(crate::hub::push_routes::PushHttp::new(
            push.clone(),
        )))
        .with_push_presence({
            let manager = push.as_ref().map(Arc::downgrade);
            Arc::new(move || {
                manager
                    .as_ref()
                    .and_then(Weak::upgrade)
                    .is_some_and(|manager| manager.status().subscriptions > 0)
            })
        })
        .with_notifications(notifications.clone())
        .with_auto_approval(Arc::new(crate::hub::auto_approval::AutoApprovalHttp::new(
            approval_rules.clone(),
        )))
        .with_handoff(Arc::new(crate::hub::handoff_routes::HandoffHttp::new(
            paths.clone(),
            context.config.clone(),
            core.clone(),
            workers.clone(),
        )))
        .with_approval_actions(approval_actions, approval_rules.policy.clone())
        .with_spawn(
            Arc::new(crate::hub::spawn_routes::SpawnHttp::new(ordinary.clone())),
            Arc::new(|| chrono::Local::now().offset().local_minus_utc()),
        );
        if let Some(owner) = external_notice {
            services = services.with_external_notices(owner);
        }
        let ledger = RuntimeLedger::open(&paths)?;
        if options.dev {
            services = services.with_assets(Arc::new(super::dev_assets::DevAssets::new(
                &context.cwd,
                paths.is_trial(),
            )));
            logger.write("INFO", "dev mode: serving web assets from ./web/dist/", "");
        }
        Ok(Self {
            dependencies: HubCompositionDependencies {
                config: context.config,
                core,
                effects,
                files,
                journal,
                registry,
                spawn_policy,
                local_models,
                models,
                workers,
                usage_hooks,
                tasks,
                ordinary,
                subscriptions,
                session_usage,
                cli_updates,
                notifications,
                push,
                security,
                one_tap,
                approval_rules,
                approval_patterns,
                instruction_rules,
                approval_pattern_io,
                orchestration,
                relay,
                routines,
                routine_store,
                servers,
                whisper,
            },
            services,
            sockets,
            spawner,
            storage,
            owner,
            ledger,
            listener,
            service_cancel,
            open: options.open,
            paths,
            warning,
            logger,
            shutdown_owner,
            routine_cancel,
            _database_lease: database_lease,
            _runtime_lease: runtime_lease,
        })
    }
    /// Finish endpoint integration before creating the WebSocket/router owners
    /// or accepting requests. Failure drops all unpublished owners.
    pub fn with_routes(
        mut self,
        configure: impl FnOnce(ServiceRouter, &HubCompositionDependencies) -> io::Result<ServiceRouter>,
    ) -> io::Result<Self> {
        self.services = configure(self.services, &self.dependencies)?;
        Ok(self)
    }
    pub async fn run(self, shutdown: &Cancellation) -> io::Result<()> {
        let services = Arc::new(self.services);
        let core: Arc<dyn SessionCore> = self.dependencies.core.clone();
        let websockets = Arc::new(
            WebSocketService::new(
                services.clone(),
                core,
                self.sockets.clone(),
                self.dependencies.effects.clone(),
                self.service_cancel.clone(),
                self.warning.clone(),
            )
            .with_startup_owner(self.spawner.clone())
            .with_registration_preparation(Arc::new(HubRegistrationPreparation {
                rules: self.dependencies.instruction_rules.clone(),
                usage: self.dependencies.usage_hooks.clone(),
                cancel: self.service_cancel.clone(),
                relay: self.dependencies.relay.clone(),
            }))
            .with_registration_hook(Arc::new(HubRegistrationHooks(
                self.dependencies.workers.clone(),
            )))
            .with_ordinary_spawn_owner(self.dependencies.ordinary.clone()),
        );
        let worker_guard = self.dependencies.workers.start().map_err(session_error)?;
        {
            let patterns = self.dependencies.approval_patterns.clone();
            let io = self.dependencies.approval_pattern_io.clone();
            let effects = self.dependencies.effects.clone();
            let cancel = self.service_cancel.clone();
            if let Ok(permit) = self.dependencies.tasks.effect_permit() {
                drop(permit.start(async move {
                    patterns.remote_sync(io, &cancel, effects.as_ref()).await;
                }));
            }
        }
        self.dependencies.usage_hooks.registered();
        {
            let updates = self.dependencies.cli_updates.clone();
            let config = self.dependencies.config.clone();
            let warning = self.warning.clone();
            let permit = self
                .dependencies
                .tasks
                .effect_permit()
                .map_err(session_error)?;
            drop(permit.start(async move {
                if let Ok(snapshot) = config.snapshot()
                    && updates
                        .clean_logs(
                            std::time::SystemTime::now(),
                            snapshot.config.log.session_retention_days,
                        )
                        .is_err()
                {
                    warning(
                        "CLI update log cleanup",
                        &SessionError::InvalidRequest("private log cleanup failed".into()),
                    );
                }
            }));
        }
        let orchestration_guard = match self.dependencies.orchestration.start() {
            Ok(guard) => guard,
            Err(error) => {
                worker_guard.stop_and_join().await;
                return Err(session_error(error));
            }
        };
        let relay_guard = match self.dependencies.relay.start() {
            Ok(guard) => guard,
            Err(error) => {
                worker_guard.stop_and_join().await;
                orchestration_guard.stop_and_join().await;
                return Err(session_error(error));
            }
        };
        let routine_guard = super::routines::RoutineGuard::start(
            self.dependencies.routines.clone(),
            self.routine_cancel.clone(),
            self.service_cancel.clone(),
        );
        let idle_guard = idle::IdleGuard::start(
            self.dependencies.core.clone(),
            self.dependencies.config.clone(),
            self.dependencies.effects.clone(),
            self.dependencies.tasks.clone(),
            self.warning.clone(),
        );
        let pid = i64::from(std::process::id());
        if self
            .ledger
            .write(services.bound_port(), pid, Timestamp::now())
            .is_err()
        {
            self.logger
                .write("WARN", "runtime ledger could not be written", "");
        }
        if self.open
            && self.paths.automatic_external_actions_allowed()
            && let Ok(snapshot) = self.dependencies.config.snapshot()
            && let Ok(mut url) =
                url::Url::parse(&format!("http://127.0.0.1:{}/", services.bound_port()))
        {
            url.query_pairs_mut()
                .append_pair("token", &snapshot.config.token);
            if crate::launcher::open_browser(url.as_str()).is_err() {
                self.logger.write("WARN", "browser could not be opened", "");
            }
        }
        self.logger.write(
            "INFO",
            "MANY-AI-CLI started",
            &format!("pid={} port={}", std::process::id(), services.bound_port()),
        );
        let serve = crate::hub::transport::serve_with_websockets(
            self.listener,
            services,
            self.dependencies.effects.clone(),
            websockets.clone(),
            self.service_cancel.clone(),
        );
        tokio::pin!(serve);
        let result = tokio::select! {
            result = &mut serve => result,
            _ = shutdown.cancelled() => {
                self.owner.stop_requests();
                self.dependencies.core.shutdown_confirmations();
                if let Err(error) = self.shutdown_owner.announce("signal").await { (self.warning)("shutdown notice", &error); }
                self.service_cancel.cancel();
                serve.await
            }
        };
        self.owner.stop_requests();
        self.dependencies.core.shutdown_confirmations();
        self.service_cancel.cancel();
        // All source producers must stop before closing effect admission.
        worker_guard.stop_and_join().await;
        routine_guard.stop_and_join().await;
        relay_guard.stop_and_join().await;
        orchestration_guard.stop_and_join().await;
        idle_guard.stop_and_join().await;
        websockets.shutdown_and_drain().await;
        self.owner.cancel_requests().map_err(session_error)?;
        if tokio::time::timeout(Duration::from_secs(10), self.owner.drain_requests())
            .await
            .is_err()
        {
            self.owner.abort_requests().map_err(session_error)?;
            self.owner.drain_requests().await;
        }
        self.spawner.shutdown();
        self.dependencies.whisper.shutdown().await;
        self.dependencies.servers.close_all().await;
        self.dependencies.routines.join_launches().await;
        self.spawner.drain().await;
        self.dependencies.usage_hooks.shutdown();
        self.owner.stop_effects().map_err(session_error)?;
        self.owner.cancel_effects().map_err(session_error)?;
        self.owner.drain_effects().await;
        // Accepted instruction callbacks have settled. Cleanup uses a fresh
        // token so Hub shutdown cancellation cannot strand an injected block.
        self.dependencies
            .instruction_rules
            .shutdown(&Cancellation::default())
            .await;
        self.dependencies.journal.close_all();
        let report = if let Some(storage) = &self.storage {
            storage
                .shutdown(
                    ShutdownPolicy::CompatibilityStop,
                    &HubShutdownCancellation::default(),
                )
                .await
        } else {
            ShutdownReport::default()
        };
        let remove = self.ledger.remove_if_pid(pid);
        self.logger
            .write("INFO", "MANY-AI-CLI stopped", &format!("pid={pid}"));
        if self.logger.close().is_err() {
            crate::logging::write_diagnostic("Hub diagnostic log close unavailable\n");
        }
        result?;
        remove?;
        check_storage_shutdown(report)
    }
}
struct HubRegistrationPreparation {
    rules: Arc<crate::application::instruction_rules::InstructionRules>,
    usage: Arc<crate::application::usage_hooks::UsageHooks>,
    cancel: Cancellation,
    relay: Arc<crate::application::relay_program::RelayProgram>,
}
impl crate::hub::websocket::RegistrationPreparation for HubRegistrationPreparation {
    fn restored_metadata(
        &self,
        message: &crate::proto::Message,
    ) -> Option<SpawnRegistrationMetadata> {
        self.relay.resolve_reattach_metadata(message)
    }
    fn before_ack<'a>(
        &'a self,
        binding: SessionBinding,
        reattach: bool,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            if reattach {
                self.relay
                    .prepare_reattach(binding, Timestamp::now())
                    .await?;
            }
            Ok(())
        })
    }
    fn prepare<'a>(&'a self) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            self.rules.registered(&self.cancel).await;
            self.usage.registered();
            Ok(())
        })
    }
}
struct HubRegistrationHooks(Arc<SessionWorkers>);
impl crate::application::session_workers::RegistrationHook for HubRegistrationHooks {
    fn registered(
        &self,
        binding: SessionBinding,
        metadata: &SpawnRegistrationMetadata,
    ) -> Result<(), SessionError> {
        crate::application::session_workers::RegistrationHook::registered(
            self.0.as_ref(),
            binding,
            metadata,
        )?;
        Ok(())
    }
}
struct OrdinaryHooks(
    Arc<SessionWorkers>,
    Arc<super::logger::HubLogger>,
    Arc<crate::application::usage_hooks::UsageHooks>,
);
impl OrdinarySpawnHooks for OrdinaryHooks {
    fn registered<'a>(
        &'a self,
        binding: SessionBinding,
        metadata: SpawnRegistrationMetadata,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            crate::application::session_workers::RegistrationHook::registered(
                self.0.as_ref(),
                binding,
                &metadata,
            )?;
            self.2.registered();
            Ok(())
        })
    }
    fn warning(&self, operation: &'static str) {
        self.1.write("WARN", operation, "ordinary spawn");
    }
}
fn session_error(error: SessionError) -> io::Error {
    io::Error::other(format!("{error:?}"))
}
async fn bind_listener(port: u16, trial: bool) -> io::Result<tokio::net::TcpListener> {
    let count = if trial { 1 } else { 100 };
    for step in 0..count {
        let Some(candidate) = port.checked_add(step) else {
            break;
        };
        match tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, candidate)).await {
            Ok(listener) => return Ok(listener),
            Err(error) if error.kind() == io::ErrorKind::AddrInUse && !trial => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AddrInUse,
        "no available Hub port in configured range",
    ))
}

fn check_storage_shutdown(report: ShutdownReport) -> io::Result<()> {
    if let Some(error) = report.error {
        return Err(io::Error::other(error));
    }
    if report.timed_out || report.cancelled || report.remaining != 0 {
        return Err(io::Error::other(
            "session history writer did not finish shutdown",
        ));
    }
    Ok(())
}
