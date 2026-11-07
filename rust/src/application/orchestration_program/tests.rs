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
    // Same boundary as the Hub socket: the guard runs right before dispatch.
    fn send_guarded<'a>(
        &'a self,
        _: SessionBinding,
        message: crate::proto::Message,
        guard: InputSendGuard<'a>,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            if self.fail.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(SessionError::Transport("synthetic frame rejected".into()));
            }
            let mut dispatch = || {
                lock(&self.sent).push(message.data.clone());
                Ok(())
            };
            guard(&mut dispatch)
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
async fn sequential_startups_restore_live_predecessor_and_ignore_stale_rollback() {
    let f = fixture().await;
    let first = OrchestrationId("synthetic-first-startup".into());
    let second = OrchestrationId("synthetic-second-startup".into());
    let (_, first_previous) = f
        .core
        .mark_conductor_with_previous(f.binding, first.clone(), "first-board".into())
        .unwrap();
    assert!(first_previous.0 == f.id);
    assert_eq!(first_previous.1, f.path.to_string_lossy());
    f.core
        .restore_conductor_if_matches(
            f.binding,
            &first,
            "first-board",
            first_previous.0.clone(),
            first_previous.1.clone(),
        )
        .unwrap();
    assert!(f.core.snapshot(f.binding.session).unwrap().orchestration_id == f.id);

    let (_, second_previous) = f
        .core
        .mark_conductor_with_previous(f.binding, second.clone(), "second-board".into())
        .unwrap();
    assert!(second_previous.0 == f.id);
    assert_eq!(second_previous.1, f.path.to_string_lossy());
    let (effects, unchanged_previous) = f
        .core
        .mark_conductor_with_previous(f.binding, second.clone(), "second-board".into())
        .unwrap();
    assert!(effects.0.is_empty());
    assert!(unchanged_previous.0 == second);
    assert_eq!(unchanged_previous.1, "second-board");
    // A delayed compensation from the completed first start must not replace
    // the newer startup, even though both belong to the same parent session.
    assert!(
        f.core
            .restore_conductor_if_matches(
                f.binding,
                &first,
                "first-board",
                first_previous.0.clone(),
                first_previous.1.clone(),
            )
            .unwrap()
            .0
            .is_empty()
    );
    assert!(f.core.snapshot(f.binding.session).unwrap().orchestration_id == second);
    f.core
        .restore_conductor_if_matches(
            f.binding,
            &second,
            "second-board",
            second_previous.0.clone(),
            second_previous.1.clone(),
        )
        .unwrap();
    let restored = f.core.snapshot(f.binding.session).unwrap();
    assert!(restored.orchestration_id == f.id);
    assert_eq!(restored.board_path, f.path.to_string_lossy());
}
#[tokio::test]
async fn startup_rollback_restores_card_and_board_after_warm_wrapper_reattach() {
    let f = fixture().await;
    let at = Timestamp::now();
    let relay = OrchestrationId("synthetic-warm-rollback".into());
    let snapshot = f.core.snapshot(f.binding.session).unwrap();
    let relay_path = f
        .owner
        .boards
        .ensure(&relay.0, &snapshot, "synthetic", at)
        .unwrap();
    let relay_path_text = relay_path.to_string_lossy().into_owned();
    f.owner.claim_relay_board(&relay);
    let (_, previous) = f
        .core
        .mark_conductor_with_previous(f.binding, relay.clone(), relay_path_text.clone())
        .unwrap();
    assert!(previous.0 == f.id);
    assert_eq!(previous.1, f.path.to_string_lossy());
    let reattached = f
        .core
        .reattach(
            ReattachRequest {
                restored_metadata: None,
                message: crate::proto::Message {
                    session_id: f.binding.session.0,
                    provider: snapshot.provider,
                    cwd: snapshot.cwd,
                    pid: 7,
                    cols: 120,
                    rows: 30,
                    ..Default::default()
                },
            },
            WrapperConnectionId(2),
            at,
        )
        .await
        .unwrap()
        .binding;
    assert!(reattached != f.binding);
    assert!(reattached.session == f.binding.session);
    assert!(reattached.incarnation == f.binding.incarnation);
    // The startup still holds its original wrapper binding, but the board
    // records the currently attached wrapper of that same parent incarnation.
    f.owner
        .register_conductor(f.binding, &relay, &relay_path)
        .unwrap();
    assert!(
        lock(&f.owner.state).boards[&relay].conductor_bindings[&f.binding.session] == reattached
    );
    let other_incarnation = SessionBinding {
        incarnation: SessionIncarnation(f.binding.incarnation.0 + 1),
        ..f.binding
    };
    for (binding, id, path) in [
        (other_incarnation, relay.clone(), relay_path_text.clone()),
        (
            f.binding,
            OrchestrationId("different-relay".into()),
            relay_path_text.clone(),
        ),
        (f.binding, relay.clone(), "different-board".into()),
    ] {
        assert!(
            f.core
                .restore_conductor_if_matches(
                    binding,
                    &id,
                    &path,
                    previous.0.clone(),
                    previous.1.clone(),
                )
                .unwrap()
                .0
                .is_empty()
        );
    }
    assert!(
        !f.owner
            .release_relay_board_if_matches(other_incarnation, &relay, &relay_path)
    );
    assert!(
        !f.owner
            .release_relay_board_if_matches(f.binding, &relay, &f.path)
    );
    f.core
        .restore_conductor_if_matches(
            f.binding,
            &relay,
            &relay_path_text,
            previous.0.clone(),
            previous.1.clone(),
        )
        .unwrap();
    let restored = f.core.details(reattached.session).unwrap();
    assert!(restored.binding == reattached);
    assert!(restored.snapshot.orchestration_id == previous.0);
    assert_eq!(restored.snapshot.board_path, previous.1);
    assert!(
        f.owner
            .release_relay_board_if_matches(f.binding, &relay, &relay_path)
    );
    let state = lock(&f.owner.state);
    assert!(!state.relay_boards.contains(&relay));
    assert!(!state.boards.contains_key(&relay));
    assert!(
        state.boards[&f.id]
            .sessions
            .contains_key(&f.binding.session)
    );
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
    child_under(f, f.binding, 1, role, at, "").await
}
async fn child_under(
    f: &Fixture,
    parent: SessionBinding,
    depth: i64,
    role: &str,
    at: Timestamp,
    execution_mode: &str,
) -> SessionBinding {
    let spec = WrappedSpawnSpec {
        registration_metadata: SpawnRegistrationMetadata {
            parent: parent.session,
            role: role.into(),
            auto: true,
            depth,
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
        execution_mode: execution_mode.into(),
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
                parent,
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
async fn interrupt_notice_is_held_while_the_conductor_waits_on_an_approval() {
    // Audit U27: a notice's Enter must not answer the conductor's own prompt.
    let f = fixture().await;
    configure(&f, |config| {
        config.orchestration.board_notify_mode = "interrupt".into()
    });
    let record = |f: &Fixture| {
        f.core
            .details(f.binding.session)
            .unwrap()
            .approval
            .record
            .is_some()
    };
    output(
        &f,
        f.binding,
        "\x1b[2J\x1b[HRun: git push\r\nDo you want to proceed?\r\n❯ 1. Yes\r\n  2. No",
        Timestamp::now(),
    );
    assert!(record(&f));
    f.owner
        .notify_relay_parent(f.binding.session, &f.id, "relay stopped\n".into())
        .await
        .unwrap();
    f.owner
        .notify_relay_parent(f.binding.session, &f.id, "second notice\n".into())
        .await
        .unwrap();
    f.owner.flush_held().await;
    assert!(lock(&f.io.sent).is_empty(), "nothing may reach the prompt");
    assert!(record(&f), "the approval stays for the user");
    // A native record vanishes after three changed screens without the prompt.
    for step in 0..8 {
        if !record(&f) {
            break;
        }
        output(
            &f,
            f.binding,
            &format!("\x1b[2J\x1b[Hidle {step}"),
            Timestamp::now(),
        );
    }
    assert!(!record(&f), "the synthetic approval must close");
    f.owner.flush_held().await;
    // Each notice is pasted, then entered (the ordinary submit path).
    let sent = lock(&f.io.sent).clone();
    let body = |text: &str| {
        let mut bytes = framed(&sanitize_notice(text));
        assert_eq!(bytes.pop(), Some(b'\r'));
        bytes
    };
    assert_eq!(
        sent.iter()
            .filter(|frame| frame.starts_with(b"\x1b[200~"))
            .cloned()
            .collect::<Vec<_>>(),
        vec![body("relay stopped\n"), body("second notice\n")],
        "held notices are typed in order once the parent can take them"
    );
    assert_eq!(sent.last().map(Vec::as_slice), Some(b"\r".as_slice()));
    assert!(lock(&f.owner.state).held.is_empty());
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
#[test]
fn startup_failed_tail_masks_secrets_and_keeps_only_the_last_five_lines() {
    // 合成の値だけ。接頭辞と本体を分けて組み立て、秘密の検査に当たらないようにする。
    let key = format!("{}{}", "ghp_", "A".repeat(30));
    let bearer = format!("{}{}", "Bearer ", "b".repeat(20));
    let screen = vec![
        "dropped one".to_owned(),
        "dropped two".to_owned(),
        format!("ANTHROPIC_API_KEY={key}"),
        "│ boxed line is skipped".to_owned(),
        "   ".to_owned(),
        format!("Authorization: {bearer}"),
        format!("pasted {key} inline"),
        "last line".to_owned(),
        "after".to_owned(),
    ];
    let tail = lifecycle::startup_failed_tail(&screen);
    assert!(!tail.contains(&key), "raw key leaked: {tail}");
    assert!(!tail.contains(&"b".repeat(20)), "raw bearer leaked: {tail}");
    assert!(tail.contains("ANTHROPIC_API_KEY=***"), "{tail}");
    assert!(
        tail.contains("last line") && tail.contains("after"),
        "{tail}"
    );
    assert!(!tail.contains("dropped"), "{tail}");
    assert!(!tail.contains("boxed line"), "{tail}");
    assert_eq!(tail.lines().count(), 5, "{tail}");
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
async fn audit_u26_timeout_respawn_keeps_the_depth_of_the_child_it_replaces() {
    let f = fixture().await;
    configure(&f, |cfg| {
        cfg.orchestration.child_startup_fail = Some(false);
        cfg.orchestration.child_timeout_seconds = 10;
        cfg.orchestration.idle_done_threshold_sec = 0;
        cfg.orchestration.timeout_respawn = true;
        cfg.orchestration.max_timeout_respawns = 1;
        cfg.orchestration.max_depth = 2;
    });
    let at = Timestamp::now();
    // A child at depth 1 started one at depth 2. Headless, so starting it
    // again needs no provider on this machine.
    let middle = child_under(&f, f.binding, 1, "impl", at, "headless").await;
    let deep = child_under(&f, middle, 2, "review", at, "headless").await;
    lifecycle::poll(&f.owner, at + Duration::from_secs(30))
        .await
        .unwrap();
    // The respawn runs as a task; wait for the review child's replacement.
    let mut replacement = None;
    for _ in 0..300 {
        replacement = lock(&f.owner.state).boards[&f.id]
            .children
            .iter()
            .find(|(id, child)| **id != deep.session && child.registration.role == "review")
            .map(|(id, _)| *id);
        if replacement.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let replacement = replacement.expect("the timed-out review child starts again");
    let snapshot = f.core.snapshot(replacement).unwrap();
    assert_eq!(snapshot.depth, 2);
    assert_eq!(snapshot.parent_session_id, middle.session);
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
