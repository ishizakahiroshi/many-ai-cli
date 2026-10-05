use super::*;
use crate::{
    config::{Config, RuntimePaths},
    files::FilesService,
    hub::task_owner::HubTaskOwner,
    terminal::{
        events::CoreEventBus,
        journal::{JournalOptions, SessionJournal},
        session::EngineOptions,
    },
};
struct Io {
    sent: Mutex<Vec<Vec<u8>>>,
    fail: std::sync::atomic::AtomicBool,
    engine: Mutex<Weak<SessionEngine>>,
    starts: std::sync::atomic::AtomicU64,
    stopped: Mutex<Vec<LiveSessionId>>,
}
impl WrapperTransport for Io {
    fn send<'a>(
        &'a self,
        _: SessionBinding,
        message: crate::proto::Message,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            if self.fail.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(SessionError::Transport("synthetic frame rejected".into()));
            }
            lock(&self.sent).push(message.data);
            Ok(())
        })
    }
}
impl CoreEffectSink for Io {
    fn apply<'a>(&'a self, effects: CoreEffects) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
        Box::pin(async move {
            for effect in effects.0 {
                if let CoreEffect::CancelSession { binding, .. } = effect {
                    lock(&self.stopped).push(binding.session);
                }
            }
            Ok(())
        })
    }
}
impl WrappedSessionSpawner for Io {
    fn spawn_and_wait<'a>(
        &'a self,
        spec: WrappedSpawnSpec,
        _: Duration,
        _: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome> {
        Box::pin(async move {
            // Only a synthetic wrapper receipt, never an executable/provider.
            let n = self
                .starts
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                + 100;
            let core = lock(&self.engine).upgrade().unwrap();
            let registration = core
                .register(
                    RegisterRequest {
                        message: crate::proto::Message {
                            provider: spec.provider,
                            cwd: spec.cwd.to_string_lossy().into_owned(),
                            label: spec.label,
                            pid: n as i64,
                            cols: 120,
                            rows: 30,
                            ..Default::default()
                        },
                        spawn_proof: spec
                            .registration_proof
                            .map(|proof| proof.as_header_value().to_owned()),
                    },
                    WrapperConnectionId(n),
                    Timestamp::now(),
                )
                .await
                .unwrap();
            SpawnWaitOutcome::Registered(registration.binding)
        })
    }
}
struct Fixture {
    root: tempfile::TempDir,
    _tasks: HubTaskOwner,
    core: Arc<SessionEngine>,
    owner: Arc<OrchestrationProgram>,
    io: Arc<Io>,
    binding: SessionBinding,
    id: OrchestrationId,
    path: PathBuf,
}
async fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let runtime = root.path().join("trial");
    std::fs::create_dir(&runtime).unwrap();
    let paths = RuntimePaths::trial(&runtime, 49674, &root.path().join("installed")).unwrap();
    let config = Arc::new(ConfigStore::new(paths.clone(), Config::default()).unwrap());
    let tasks = HubTaskOwner::new(tokio::runtime::Handle::current());
    let io = Arc::new(Io {
        sent: Mutex::new(Vec::new()),
        fail: false.into(),
        engine: Mutex::new(Weak::new()),
        starts: std::sync::atomic::AtomicU64::new(0),
        stopped: Mutex::new(Vec::new()),
    });
    let effects: Arc<dyn CoreEffectSink> = io.clone();
    let core = Arc::new(SessionEngine::new(
        EngineOptions::default(),
        Arc::new(SessionJournal::new(
            paths.clone(),
            None,
            JournalOptions::default(),
        )),
        io.clone(),
        effects.clone(),
        io.clone(),
        CoreEventBus::new(64).unwrap(),
    ));
    *lock(&io.engine) = Arc::downgrade(&core);
    let at = Timestamp::from_unix(1791158400, 0).unwrap();
    let binding = core
        .register(
            RegisterRequest {
                message: crate::proto::Message {
                    provider: "claude".into(),
                    cwd: runtime.to_string_lossy().into_owned(),
                    pid: 7,
                    cols: 120,
                    rows: 30,
                    ..Default::default()
                },
                spawn_proof: None,
            },
            WrapperConnectionId(1),
            at,
        )
        .await
        .unwrap()
        .binding;
    let workers = SessionWorkers::new(
        config.clone(),
        paths.clone(),
        Arc::new(FilesService::new(runtime.clone(), paths.clone())),
        tasks.handle(),
        Arc::new(|_, _| {}),
    );
    workers
        .bind(Arc::downgrade(&core), Arc::downgrade(&effects))
        .unwrap();
    let owner = OrchestrationProgram::new(OrchestrationDependencies {
        core: Arc::downgrade(&core),
        effects: Arc::downgrade(&effects),
        config,
        paths: paths.clone(),
        registry: Arc::new(|| {
            Err(std::io::Error::other(
                "test rejects native provider discovery",
            ))
        }),
        environment: Vec::new(),
        hub_cwd: runtime,
        workers,
        tasks: tasks.handle(),
        warning: Arc::new(|_, _| {}),
    })
    .unwrap();
    let id = owner
        .reserve_conductor(&mut String::new(), None, at)
        .unwrap();
    let snapshot = core.snapshot(binding.session).unwrap();
    let path = owner
        .boards
        .ensure(&id.0, &snapshot, "synthetic", at)
        .unwrap();
    core.mark_conductor(binding, id.clone(), path.to_string_lossy().into_owned())
        .unwrap();
    owner.register_conductor(binding, &id, &path).unwrap();
    Fixture {
        root,
        _tasks: tasks,
        core,
        owner,
        io,
        binding,
        id,
        path,
    }
}
#[tokio::test]
async fn actionable_fifo_is_held_through_approval_then_sends_one_framed_notice_per_poll() {
    let f = fixture().await;
    let at = Timestamp::now();
    let marker =
        crate::approval::marker::extract("[MANY-AI-CLI]\ncontinue? (Y:1/N:0)\n[/MANY-AI-CLI]")
            .unwrap();
    f.core
        .observe_transcript_marker(f.binding, Some(marker), at)
        .unwrap();
    f.owner
        .queue_event(f.binding.session, &f.id, "first\n".into())
        .await
        .unwrap();
    f.owner
        .queue_event(f.binding.session, &f.id, "second\n".into())
        .await
        .unwrap();
    f.owner.flush_events().await.unwrap();
    assert!(lock(&f.io.sent).is_empty());
    assert_eq!(lock(&f.owner.state).boards[&f.id].events.len(), 2);
    f.core
        .observe_transcript_batch(
            f.binding,
            f.root.path().join("synthetic.jsonl"),
            &[crate::approval::transcript::parser::AgentChatMessage {
                role: "user".into(),
                text: "1".into(),
                ..Default::default()
            }],
            false,
            at,
        )
        .unwrap();
    f.core.evaluate_idle(at + Duration::from_secs(4));
    f.owner.flush_events().await.unwrap();
    assert_eq!(&*lock(&f.io.sent), &[b"\x1b[200~first\x1b[201~\r".to_vec()]);
    f.owner.flush_events().await.unwrap();
    assert_eq!(lock(&f.io.sent).len(), 2);
    assert!(lock(&f.owner.state).boards[&f.id].events.is_empty());
    assert!(
        std::fs::read_to_string(&f.path)
            .unwrap()
            .contains("event pending: second")
    );
    let _ = &f.root;
}
#[tokio::test]
async fn full_queue_retains_every_overflow_on_board_and_later_points_to_evidence() {
    let f = fixture().await;
    for index in 0..258 {
        f.owner
            .queue_event(f.binding.session, &f.id, format!("event {index}"))
            .await
            .unwrap();
    }
    assert_eq!(lock(&f.owner.state).boards[&f.id].events.len(), 256);
    assert_eq!(
        lock(&f.owner.state).boards[&f.id].overflow[&f.binding.session],
        2
    );
    let evidence = std::fs::read_to_string(&f.path).unwrap();
    assert!(evidence.contains("event pending: event 257"));
    f.core
        .evaluate_idle(Timestamp::now() + Duration::from_secs(4));
    f.owner.flush_events().await.unwrap();
    f.owner.flush_events().await.unwrap();
    let state = lock(&f.owner.state);
    assert!(state.boards[&f.id].overflow.is_empty());
    assert!(
        state.boards[&f.id]
            .events
            .back()
            .unwrap()
            .text
            .contains("2 additional events were retained on the board")
    );
}
#[tokio::test]
async fn role_prompt_and_subscription_come_from_validated_same_orchestration_mapping() {
    let f = fixture().await;
    let mut settings = BTreeMap::new();
    settings.insert(
        "review".into(),
        Some(RoleSettings {
            provider: "codex".into(),
            model: "gpt-test".into(),
            subscription: "p2".into(),
            ..Default::default()
        }),
    );
    settings.insert("ignored".into(), None);
    let id = f
        .owner
        .reserve_conductor(&mut String::new(), Some(&settings), Timestamp::now())
        .unwrap();
    let prompt = f.owner.conductor_prompt(&id);
    assert!(prompt.contains("- review: provider=codex model=gpt-test\n"));
    assert!(!prompt.contains("No child role mapping"));
    assert_eq!(
        f.owner.role_subscription(&id, "review").unwrap(),
        Some("p2".into())
    );
    assert!(
        f.owner
            .role_subscription(&f.id, "review")
            .unwrap()
            .is_none()
    );
    settings.get_mut("review").unwrap().as_mut().unwrap().model = "--hostile".into();
    assert!(
        f.owner
            .reserve_conductor(&mut String::new(), Some(&settings), Timestamp::now())
            .is_err()
    );
}

async fn child(f: &Fixture, role: &str, at: Timestamp) -> SessionBinding {
    let spec = WrappedSpawnSpec {
        registration_metadata: SpawnRegistrationMetadata {
            parent: f.binding.session,
            role: role.into(),
            auto: true,
            depth: 1,
            orchestration: f.id.clone(),
            board_path: f.path.to_string_lossy().into_owned(),
            spawned_at: Some(at),
            prompt_at_launch: true,
            ..Default::default()
        },
        spawn_attempt: None,
        registration_proof: None,
        provider: "claude".into(),
        cwd: f.owner.deps.hub_cwd.clone(),
        model: String::new(),
        model_selection: String::new(),
        risk_confirmed: false,
        label: format!("test-{role}"),
        permission_mode: String::new(),
        sandbox: String::new(),
        ask_for_approval: String::new(),
        route: String::new(),
        utf8_session: false,
        effort: String::new(),
        execution_mode: String::new(),
        permission_preset: String::new(),
        initial_prompt: String::new(),
        subscription_profile_id: String::new(),
        subscription_login: false,
        usage_probe: false,
        grants: InternalSpawnGrants::default(),
        cancellation: TaskCancellation::default(),
    };
    let binding = match f
        .core
        .spawn_and_wait(
            spec.clone(),
            Duration::from_secs(1),
            &HttpWaitCancellation::default(),
        )
        .await
    {
        SpawnWaitOutcome::Registered(binding) => binding,
        _ => panic!("synthetic wrapper must register"),
    };
    f.owner
        .child_registered(
            RegisteredChild {
                binding,
                parent: f.binding,
                role: role.into(),
                preparation: child_launch::ChildPreparation {
                    orchestration: f.id.clone(),
                    board_path: f.path.clone(),
                    absolute_cwd: f.owner.deps.hub_cwd.clone(),
                    child_cwd: f.owner.deps.hub_cwd.clone(),
                    branch: String::new(),
                },
                restart_spec: spec,
                initial_prompt: "synthetic task".into(),
                prompt_via_launch_arg: true,
                inject_after_registration: None,
                spawned_at: at,
            },
            TaskCancellation::default(),
        )
        .await
        .unwrap();
    binding
}
fn output(f: &Fixture, binding: SessionBinding, text: &str, at: Timestamp) {
    f.core
        .observe_output(
            binding,
            OutputChunk {
                bytes: text.as_bytes().to_vec(),
                total_pty_bytes: 0,
            },
            at,
        )
        .unwrap();
}
fn configure(f: &Fixture, apply: impl FnOnce(&mut Config)) {
    let mut snapshot = f.owner.deps.config.snapshot().unwrap();
    apply(&mut snapshot.config);
    f.owner
        .deps
        .config
        .publish_then_persist_legacy(snapshot.revision, snapshot.config)
        .unwrap();
}
#[tokio::test]
async fn initial_gate_and_failed_transport_keep_fifo_out_of_reconnect_input() {
    let f = fixture().await;
    f.core
        .evaluate_idle(Timestamp::now() + Duration::from_secs(4));
    f.core
        .set_initial_gate(f.binding, Timestamp::now())
        .unwrap();
    f.owner
        .queue_event(f.binding.session, &f.id, "held".into())
        .await
        .unwrap();
    f.owner.flush_events().await.unwrap();
    assert!(lock(&f.io.sent).is_empty());
    f.core.clear_initial_gate_scoped(f.binding);
    f.io.fail.store(true, std::sync::atomic::Ordering::SeqCst);
    f.owner.flush_events().await.unwrap();
    assert_eq!(
        lock(&f.owner.state).boards[&f.id].events[0].remainder,
        framed("held")
    );
    f.io.fail.store(false, std::sync::atomic::Ordering::SeqCst);
    f.core.flush(f.binding, &TaskCancellation::default()).await;
    assert!(
        lock(&f.io.sent).is_empty(),
        "failed board notice must never be reconnect input"
    );
    f.owner.flush_events().await.unwrap();
    assert_eq!(&*lock(&f.io.sent), &[framed("held")]);
}
#[tokio::test]
async fn progress_questions_cannot_claim_sibling_and_rewrite_cannot_replay_old_question() {
    let f = fixture().await;
    let at = Timestamp::now();
    let first = child(&f, "impl", at).await;
    let second = child(&f, "review", at).await;
    let path = child_launch::prompt::child_progress_path(&f.path, &first.session.0.to_string());
    std::fs::write(
        &path,
        format!("## QUESTION review session={}\n", second.session.0),
    )
    .unwrap();
    progress::poll(&f.owner).await.unwrap();
    assert!(lock(&f.owner.state).boards[&f.id].events.is_empty());
    assert!(std::fs::read_to_string(&f.path).unwrap().contains(&format!(
        "QUESTION rejected: source_session={} claimed_role=review claimed_session={}",
        first.session.0, second.session.0
    )));
    let text = format!(
        "{}## QUESTION impl session={}\nlong appended progress\n",
        std::fs::read_to_string(&path).unwrap(),
        first.session.0
    );
    std::fs::write(&path, &text).unwrap();
    progress::poll(&f.owner).await.unwrap();
    assert_eq!(lock(&f.owner.state).boards[&f.id].events.len(), 1);
    std::fs::write(
        &path,
        format!("## QUESTION impl session={}\n", first.session.0),
    )
    .unwrap();
    progress::poll(&f.owner).await.unwrap();
    assert_eq!(
        lock(&f.owner.state).boards[&f.id].events.len(),
        1,
        "truncation must not replay previous QUESTION"
    );
}
#[tokio::test]
async fn startup_failure_requires_confirmed_delivery_and_never_writes_progress() {
    let f = fixture().await;
    configure(&f, |cfg| {
        cfg.orchestration.child_timeout_seconds = 0;
        cfg.orchestration.idle_done_threshold_sec = 0;
        cfg.orchestration.child_startup_grace_seconds = 10;
    });
    let at = Timestamp::now();
    let binding = child(&f, "impl", at).await;
    output(&f, binding, "ready", at);
    f.core.evaluate_idle(at + Duration::from_secs(4));
    lifecycle::poll(&f.owner, at + Duration::from_secs(4))
        .await
        .unwrap();
    lifecycle::poll(&f.owner, at + Duration::from_secs(30))
        .await
        .unwrap();
    assert!(!lock(&f.owner.state).boards[&f.id].children[&binding.session].startup_failed);
    assert!(lock(&f.io.stopped).is_empty());
    f.core
        .record_initial_prompt_outcome(
            binding,
            &InitialPromptOutcome::Delivered {
                evidence: crate::orchestration::initial_prompt::DeliveryEvidence::EchoObserved,
                attempts: 1,
                composer_enter_retried: false,
            },
            at + Duration::from_secs(31),
        )
        .unwrap();
    lifecycle::poll(&f.owner, at + Duration::from_secs(31))
        .await
        .unwrap();
    assert!(lock(&f.owner.state).boards[&f.id].children[&binding.session].startup_failed);
    assert_eq!(&*lock(&f.io.stopped), &[binding.session]);
    let evidence = std::fs::read_to_string(&f.path).unwrap();
    assert!(evidence.contains("Hub has no other evidence of why it stopped"));
}
#[tokio::test]
async fn progress_and_new_pty_activity_prevent_timeout_but_elapsed_inactivity_fires_once() {
    let f = fixture().await;
    configure(&f, |cfg| {
        cfg.orchestration.child_startup_fail = Some(false);
        cfg.orchestration.child_timeout_seconds = 10;
        cfg.orchestration.idle_done_threshold_sec = 0;
        cfg.orchestration.timeout_respawn = false;
    });
    let at = Timestamp::now();
    let binding = child(&f, "impl", at).await;
    let path = child_launch::prompt::child_progress_path(&f.path, &binding.session.0.to_string());
    std::fs::write(&path, "working\n").unwrap();
    progress::poll(&f.owner).await.unwrap();
    output(&f, binding, "active", at + Duration::from_secs(19));
    lifecycle::poll(&f.owner, at + Duration::from_secs(20))
        .await
        .unwrap();
    assert!(!lock(&f.owner.state).boards[&f.id].children[&binding.session].timed_out);
    lifecycle::poll(&f.owner, at + Duration::from_secs(30))
        .await
        .unwrap();
    assert!(lock(&f.owner.state).boards[&f.id].children[&binding.session].timed_out);
    let queued = lock(&f.owner.state).boards[&f.id].events.len();
    lifecycle::poll(&f.owner, at + Duration::from_secs(60))
        .await
        .unwrap();
    assert_eq!(lock(&f.owner.state).boards[&f.id].events.len(), queued);
}
#[tokio::test]
async fn dismissal_records_unfinished_work_and_latches_child_done() {
    let f = fixture().await;
    let binding = child(&f, "review", Timestamp::now()).await;
    f.owner
        .session_ended(binding.session, "dismissed")
        .await
        .unwrap();
    assert!(lock(&f.owner.state).boards[&f.id].children[&binding.session].done);
    let evidence = std::fs::read_to_string(&f.path).unwrap();
    assert!(evidence.contains("closed by the user; its work may be unfinished"));
    assert!(evidence.contains("treat its work as unfinished"));
    assert!(!evidence.contains("child complete via session_end"));
}
#[tokio::test]
async fn request_resolution_uses_runtime_mapping_even_with_explicit_provider_and_preserves_ui_unset()
 {
    let f = fixture().await;
    configure(&f, |cfg| {
        cfg.user_prefs
            .spawn
            .role_provider
            .insert("review".into(), "claude".into());
        cfg.user_prefs
            .spawn
            .role_effort
            .insert("review".into(), "high".into());
        cfg.user_prefs
            .spawn
            .role_permission
            .insert("review".into(), "full".into());
    });
    let mut roles = BTreeMap::new();
    roles.insert(
        "review".into(),
        Some(RoleSettings {
            provider: "codex".into(),
            model: "mapped-model".into(),
            effort: "low".into(),
            execution_mode: "headless".into(),
            permission_preset: "attended".into(),
            ..Default::default()
        }),
    );
    let id = f
        .owner
        .reserve_conductor(&mut String::new(), Some(&roles), Timestamp::now())
        .unwrap();
    let mut parent = f.core.snapshot(f.binding.session).unwrap();
    parent.orchestration_id = id;
    let mut body = ChildSpawnRequest {
        role: " Review ".into(),
        ..Default::default()
    };
    assert_eq!(
        f.owner.resolve_child_request(&parent, &mut body).unwrap(),
        "role_map"
    );
    assert_eq!(
        (
            &*body.provider,
            &*body.model,
            &*body.effort,
            &*body.permission_preset
        ),
        ("codex", "mapped-model", "high", "full")
    );
    assert!(
        body.execution_mode.is_empty(),
        "retained role execution mode is not an undocumented default"
    );
    let mut body = ChildSpawnRequest {
        role: "review".into(),
        provider: "claude".into(),
        claimed_origin: "ui".into(),
        ..Default::default()
    };
    assert_eq!(
        f.owner.resolve_child_request(&parent, &mut body).unwrap(),
        "explicit"
    );
    assert_eq!(body.model, "mapped-model");
    assert!(body.permission_preset.is_empty());
}
