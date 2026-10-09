use super::*;
use crate::{config::RuntimePaths, terminal::journal::JournalOptions};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

type SnapshotHook = Arc<dyn Fn() + Send + Sync>;
#[derive(Default)]
struct Executor {
    snapshot_fails: AtomicBool,
    refusal_fails: AtomicBool,
    forget_permission: AtomicBool,
    pause_spawn: AtomicBool,
    snapshots: AtomicUsize,
    spawns: AtomicUsize,
    refusals: AtomicUsize,
    notifications: AtomicUsize,
    entered: Notify,
    parent_seen: Mutex<Option<SessionBinding>>,
    snapshot_hook: Mutex<Option<SnapshotHook>>,
}
impl ConfirmationExecutor for Executor {
    fn presentation(&self) -> Result<ConfirmationPresentation, SessionError> {
        self.snapshots.fetch_add(1, Ordering::SeqCst);
        let hook = lock(&self.snapshot_hook).take();
        if let Some(hook) = hook {
            hook();
        }
        if self.snapshot_fails.load(Ordering::SeqCst) {
            return Err(SessionError::Transport(
                "synthetic snapshot unavailable".into(),
            ));
        }
        let mut config = config::Config::default();
        config
            .user_prefs
            .spawn
            .role_permission
            .insert("worker".into(), "bounded".into());
        if self.forget_permission.load(Ordering::SeqCst) {
            config.user_prefs.spawn.role_permission.clear();
        }
        Ok(ConfirmationPresentation {
            config,
            trust_grant_providers: vec!["synthetic-trust-provider".into()],
        })
    }
    fn spawn<'a>(
        &'a self,
        request: ConfirmedChildRequest,
        cancel: TaskCancellation,
    ) -> CoreFuture<'a, Result<ChildSpawnResult, SessionError>> {
        Box::pin(async move {
            self.spawns.fetch_add(1, Ordering::SeqCst);
            *lock(&self.parent_seen) = Some(request.parent);
            self.entered.notify_one();
            if self.pause_spawn.load(Ordering::SeqCst) {
                cancel.token().cancelled().await;
                return Err(SessionError::Cancelled);
            }
            Err(SessionError::Transport(
                "synthetic preparation failed".into(),
            ))
        })
    }
    fn record_refusal<'a>(
        &'a self,
        _: &'a PendingSpawnConfirmation,
        _: TaskCancellation,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            self.refusals.fetch_add(1, Ordering::SeqCst);
            if self.refusal_fails.load(Ordering::SeqCst) {
                Err(SessionError::Transport(
                    "synthetic board write failed".into(),
                ))
            } else {
                Ok(())
            }
        })
    }
    fn notify_waiter_gone<'a>(
        &'a self,
        _: &'a PendingSpawnConfirmation,
        _: &'a Result<ChildSpawnResult, SessionError>,
        _: TaskCancellation,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            self.notifications.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }
}
#[derive(Default)]
struct Sink {
    messages: Mutex<Vec<proto::Message>>,
}
impl CoreEffectSink for Sink {
    fn apply<'a>(&'a self, effects: CoreEffects) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
        Box::pin(async move {
            for effect in effects.0 {
                match effect {
                    CoreEffect::SendUi { message, .. }
                    | CoreEffect::SendUiBestEffort { message, .. } => {
                        lock(&self.messages).push(message);
                    }
                    CoreEffect::Broadcast(_) => panic!("unrouted confirmation broadcast"),
                    _ => {}
                }
            }
            Ok(())
        })
    }
}
struct NoTransport;
impl WrapperTransport for NoTransport {
    fn send<'a>(
        &'a self,
        _: SessionBinding,
        _: proto::Message,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async { Err(SessionError::Transport("no synthetic transport".into())) })
    }
}
struct NoSpawner;
impl WrappedSessionSpawner for NoSpawner {
    fn spawn_and_wait<'a>(
        &'a self,
        _: WrappedSpawnSpec,
        _: Duration,
        _: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome> {
        Box::pin(async { SpawnWaitOutcome::Failed("no synthetic wrapper launcher".into()) })
    }
}
struct Fixture {
    _root: tempfile::TempDir,
    _installed: tempfile::TempDir,
    engine: Arc<SessionEngine>,
    executor: Arc<Executor>,
    sink: Arc<Sink>,
    parent: SessionBinding,
    warnings: Arc<AtomicUsize>,
}
fn now() -> Timestamp {
    Timestamp::UNIX_EPOCH + Duration::from_secs(100)
}
async fn fixture(available: bool) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49129, installed.path()).unwrap();
    let journal = Arc::new(SessionJournal::new(paths, None, JournalOptions::default()));
    let executor = Arc::new(Executor::default());
    let sink = Arc::new(Sink::default());
    let warnings = Arc::new(AtomicUsize::new(0));
    let observed_warnings = warnings.clone();
    let engine = Arc::new(SessionEngine::new(
        EngineOptions {
            confirmation_executor: available
                .then(|| executor.clone() as Arc<dyn ConfirmationExecutor>),
            warning: Arc::new(move |_, _| {
                observed_warnings.fetch_add(1, Ordering::SeqCst);
            }),
            ..Default::default()
        },
        journal,
        Arc::new(NoTransport),
        sink.clone(),
        Arc::new(NoSpawner),
        CoreEventBus::new(32).unwrap(),
    ));
    let registration = engine
        .register(
            RegisterRequest {
                message: proto::Message {
                    provider: "codex".into(),
                    cwd: root.path().to_string_lossy().into_owned(),
                    pid: 17,
                    ..Default::default()
                },
                spawn_proof: None,
            },
            WrapperConnectionId(1),
            now(),
        )
        .await
        .unwrap();
    let parent = registration.binding;
    drop(registration.after_registered);
    let ui = UiBinding {
        connection: UiConnectionId(1),
        auth_epoch: engine.auth_epoch(),
    };
    engine.attach_ui(ui, None, None).unwrap();
    assert!(engine.finish_ui_priming(ui).unwrap().0.is_empty());
    Fixture {
        _root: root,
        _installed: installed,
        engine,
        executor,
        sink,
        parent,
        warnings,
    }
}
fn request(parent: LiveSessionId, role: &str) -> ConfirmationRequest {
    ConfirmationRequest {
        parent,
        requested_provider: "codex".into(),
        requested_at: now(),
        body: ResolvedChildSpawn::from_request(
            ChildSpawnRequest {
                role: role.into(),
                provider: "codex".into(),
                model: "synthetic".into(),
                cwd: "synthetic-project".into(),
                initial_prompt: "owned fixture".into(),
                effort: "high".into(),
                ..Default::default()
            },
            None,
            InternalSpawnGrants::default(),
        )
        .unwrap(),
    }
}
fn register(f: &Fixture, role: &str) -> ConfirmationRegistration {
    SpawnConfirmations::register(f.engine.as_ref(), request(f.parent.session, role)).unwrap()
}
fn response(id: &SpawnConfirmationId, approved: bool) -> SpawnConfirmationResponse {
    SpawnConfirmationResponse {
        confirmation_id: id.clone(),
        approved,
        ..Default::default()
    }
}
fn accept(f: &Fixture, id: &SpawnConfirmationId, approved: bool) -> Box<dyn AcceptedSpawnDecision> {
    f.engine
        .clone()
        .accept_decision(
            response(id, approved),
            VerifiedConfirmationRequest::after_server_authentication(f.engine.auth_epoch()),
            now(),
        )
        .unwrap()
}
fn messages(effects: &CoreEffects) -> Vec<&proto::Message> {
    effects
        .0
        .iter()
        .map(|effect| match effect {
            CoreEffect::SendUiBestEffort { message, .. } => message,
            _ => panic!("unexpected confirmation effect"),
        })
        .collect()
}
async fn wait(waiter: Box<dyn ConfirmationWaiter>) -> ConfirmationWaitOutcome {
    tokio::time::timeout(
        Duration::from_secs(2),
        waiter.wait(HttpWaitCancellation::default()),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn absent_executor_or_snapshot_fails_before_confirmation_capacity_or_publication() {
    for available in [false, true] {
        let f = fixture(available).await;
        f.executor.snapshot_fails.store(true, Ordering::SeqCst);
        assert!(matches!(
            SpawnConfirmations::register(f.engine.as_ref(), request(f.parent.session, "worker")),
            Err(AdmissionError::Unavailable)
        ));
        assert!(f.engine.pending().is_empty());
        assert_eq!(lock(&f.engine.state).next_confirmation, 0);
        assert!(
            lock(&f.engine.state)
                .admission
                .reserve_confirmation(f.parent.session, 256, None)
                .is_ok()
        );
        assert_eq!(f.warnings.load(Ordering::SeqCst), 1);
        assert!(lock(&f.sink.messages).is_empty());
    }
}
#[tokio::test]
async fn registration_uses_live_disclosure_and_negative_logical_millisecond_floor() {
    let f = fixture(true).await;
    let weak = Arc::downgrade(&f.engine);
    *lock(&f.executor.snapshot_hook) = Some(Arc::new(move || {
        assert!(weak.upgrade().unwrap().state.try_lock().is_ok());
    }));
    let mut req = request(f.parent.session, "worker");
    req.requested_at = Timestamp::from_unix(-1, 999_999_999).unwrap();
    let r = SpawnConfirmations::register(f.engine.as_ref(), req).unwrap();
    assert_eq!(r.pending.id.0, "sc--1-1");
    let wire = messages(&r.effects)[0];
    assert_eq!(wire.r#type, "spawn_confirmation_requested");
    assert_eq!(wire.spawn_requested_at_ms, -1);
    assert!(wire.remember_permission);
    assert_eq!(wire.trust_grant_providers, ["synthetic-trust-provider"]);
    assert!(!wire.spawn_child_approval.is_empty());
    assert!(
        f.engine
            .admission_matches(&r.pending.admission, f.parent.session, 1)
    );
}
#[tokio::test]
async fn replacement_orders_close_before_request_and_completes_unstarted_waiter() {
    let f = fixture(true).await;
    let old = register(&f, "worker");
    let new = register(&f, "worker");
    let wire = messages(&new.effects);
    assert_eq!(wire.len(), 2);
    assert_eq!(wire[0].reason, "superseded");
    assert_eq!(wire[0].spawn_confirmation_id, old.pending.id.0);
    assert_eq!(wire[1].r#type, "spawn_confirmation_requested");
    assert_eq!(
        wait(old.waiter).await,
        ConfirmationWaitOutcome::Decided(ConfirmationOutcome::Superseded)
    );
    assert!(
        !f.engine
            .admission_matches(&old.pending.admission, f.parent.session, 1)
    );
    assert!(
        f.engine
            .admission_matches(&new.pending.admission, f.parent.session, 1)
    );
    assert_eq!(f.executor.refusals.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn failed_admission_or_identity_allocation_preserves_old_pending_reservation() {
    let f = fixture(true).await;
    let old = register(&f, "worker");
    let occupied = lock(&f.engine.state)
        .admission
        .reserve_confirmation(f.parent.session, 255, None)
        .unwrap();
    assert!(matches!(
        SpawnConfirmations::register(f.engine.as_ref(), request(f.parent.session, "other")),
        Err(AdmissionError::ChildrenPerParent { maximum: 256, .. })
    ));
    let replacement = register(&f, "worker");
    assert_eq!(
        wait(old.waiter).await,
        ConfirmationWaitOutcome::Decided(ConfirmationOutcome::Superseded)
    );
    lock(&f.engine.state).next_confirmation = u64::MAX;
    assert!(matches!(
        SpawnConfirmations::register(f.engine.as_ref(), request(f.parent.session, "worker")),
        Err(AdmissionError::IdentityExhausted)
    ));
    assert_eq!(f.engine.pending()[0].id, replacement.pending.id);
    assert!(
        f.engine
            .admission_matches(&replacement.pending.admission, f.parent.session, 1)
    );
    f.engine.release_children(&occupied.id);
}
#[tokio::test]
async fn cancelled_and_dropped_waiters_leave_pending_admission_answerable() {
    let f = fixture(true).await;
    let first = register(&f, "worker");
    let cancel = HttpWaitCancellation::default();
    cancel.cancel();
    assert_eq!(
        first.waiter.wait(cancel).await,
        ConfirmationWaitOutcome::WaiterCancelled
    );
    assert!(f.engine.pending()[0].waiter_gone);
    assert!(
        f.engine
            .admission_matches(&first.pending.admission, f.parent.session, 1)
    );
    let second = register(&f, "other");
    drop(second.waiter.wait(HttpWaitCancellation::default()));
    assert!(f.engine.pending().iter().all(|p| p.waiter_gone));
}
#[tokio::test]
async fn accepted_guard_drop_releases_only_its_decision_lease() {
    let f = fixture(true).await;
    let r = register(&f, "worker");
    let accepted = accept(&f, &r.pending.id, true);
    assert!(
        f.engine
            .clone()
            .accept_decision(
                response(&r.pending.id, true),
                VerifiedConfirmationRequest::after_server_authentication(f.engine.auth_epoch()),
                now()
            )
            .is_err()
    );
    assert_eq!(f.executor.spawns.load(Ordering::SeqCst), 0);
    drop(accepted);
    assert!(
        f.engine
            .admission_matches(&r.pending.admission, f.parent.session, 1)
    );
    drop(accept(&f, &r.pending.id, true));
    assert_eq!(f.engine.pending().len(), 1);
}
#[tokio::test]
async fn dismiss_cannot_remove_a_parent_with_pending_or_leased_confirmation() {
    let f = fixture(true).await;
    let r = register(&f, "worker");
    let accepted = accept(&f, &r.pending.id, true);
    let effects = f.engine.dismiss(f.parent.session, now()).unwrap();
    assert_eq!(messages(&effects)[0].r#type, "session_dismiss_refused");
    assert!(f.engine.details(f.parent.session).is_some());
    assert_eq!(f.engine.pending()[0].id, r.pending.id);
    assert!(
        f.engine
            .admission_matches(&r.pending.admission, f.parent.session, 1)
    );
    drop(accepted);
    assert_eq!(f.executor.refusals.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn superseded_or_expired_leases_never_restore_pending_on_drop_or_run() {
    let f = fixture(true).await;
    let old = register(&f, "worker");
    let accepted = accept(&f, &old.pending.id, true);
    let new = register(&f, "worker");
    assert!(accepted.run(TaskCancellation::default()).is_err());
    assert_eq!(f.engine.pending()[0].id, new.pending.id);
    let accepted = accept(&f, &new.pending.id, true);
    let effects = f.engine.parent_ended(f.parent.session);
    assert_eq!(messages(&effects)[0].reason, "parent_gone");
    drop(accepted);
    assert!(f.engine.pending().is_empty());
    assert_eq!(
        wait(new.waiter).await,
        ConfirmationWaitOutcome::Decided(ConfirmationOutcome::ParentEnded)
    );
    assert_eq!(f.executor.refusals.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn validation_failure_keeps_pending_and_refusal_ignores_invalid_overrides() {
    let f = fixture(true).await;
    let r = register(&f, "worker");
    let mut choice = response(&r.pending.id, true);
    choice.execution_mode = Some("bad-mode".into());
    assert!(
        f.engine
            .clone()
            .accept_decision(
                choice.clone(),
                VerifiedConfirmationRequest::after_server_authentication(f.engine.auth_epoch()),
                now()
            )
            .is_err()
    );
    assert_eq!(f.engine.pending().len(), 1);
    choice.approved = false;
    f.executor.refusal_fails.store(true, Ordering::SeqCst);
    let accepted = f
        .engine
        .clone()
        .accept_decision(
            choice,
            VerifiedConfirmationRequest::after_server_authentication(f.engine.auth_epoch()),
            now(),
        )
        .unwrap();
    let future = accepted.run(TaskCancellation::default()).unwrap();
    assert!(f.engine.pending().is_empty());
    assert!(
        !f.engine
            .admission_matches(&r.pending.admission, f.parent.session, 1)
    );
    assert_eq!(future.await, ConfirmationOutcome::Refused);
    assert_eq!(
        wait(r.waiter).await,
        ConfirmationWaitOutcome::Decided(ConfirmationOutcome::Refused)
    );
    assert_eq!(f.executor.refusals.load(Ordering::SeqCst), 1);
    assert_eq!(f.executor.spawns.load(Ordering::SeqCst), 0);
    assert_eq!(f.warnings.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn unpolled_committed_task_drop_fails_waiter_and_releases_capacity() {
    let f = fixture(true).await;
    let r = register(&f, "worker");
    let future = accept(&f, &r.pending.id, true)
        .run(TaskCancellation::default())
        .unwrap();
    assert!(f.engine.pending().is_empty());
    assert!(
        f.engine
            .admission_matches(&r.pending.admission, f.parent.session, 1)
    );
    drop(future);
    assert!(matches!(
        wait(r.waiter).await,
        ConfirmationWaitOutcome::Decided(ConfirmationOutcome::SpawnFailed(_))
    ));
    assert!(
        !f.engine
            .admission_matches(&r.pending.admission, f.parent.session, 1)
    );
    assert!(f.engine.pending().is_empty());
    assert_eq!(f.executor.spawns.load(Ordering::SeqCst), 0);
    assert_eq!(f.warnings.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn committed_drop_cannot_release_a_new_same_role_reservation() {
    let f = fixture(true).await;
    let first = register(&f, "worker");
    let task = accept(&f, &first.pending.id, true)
        .run(TaskCancellation::default())
        .unwrap();
    let second = register(&f, "worker");
    drop(task);
    assert!(matches!(
        wait(first.waiter).await,
        ConfirmationWaitOutcome::Decided(ConfirmationOutcome::SpawnFailed(_))
    ));
    assert_eq!(f.engine.pending()[0].id, second.pending.id);
    assert!(
        f.engine
            .admission_matches(&second.pending.admission, f.parent.session, 1)
    );
    assert!(
        !f.engine
            .admission_matches(&first.pending.admission, f.parent.session, 1)
    );
}
#[tokio::test]
async fn snapshot_callback_replacement_is_revalidated_without_holding_state_lock() {
    let f = fixture(true).await;
    let r = register(&f, "worker");
    let weak = Arc::downgrade(&f.engine);
    let parent = f.parent.session;
    *lock(&f.executor.snapshot_hook) = Some(Arc::new(move || {
        let engine = weak.upgrade().unwrap();
        assert!(engine.state.try_lock().is_ok());
        let replacement =
            SpawnConfirmations::register(engine.as_ref(), request(parent, "worker")).unwrap();
        drop(replacement);
    }));
    assert!(
        f.engine
            .clone()
            .accept_decision(
                response(&r.pending.id, true),
                VerifiedConfirmationRequest::after_server_authentication(f.engine.auth_epoch()),
                now()
            )
            .is_err()
    );
    assert_eq!(
        wait(r.waiter).await,
        ConfirmationWaitOutcome::Decided(ConfirmationOutcome::Superseded)
    );
    assert_eq!(f.engine.pending().len(), 1);
}
#[tokio::test]
async fn auth_rotation_rejects_before_handoff_but_does_not_cancel_committed_task() {
    let f = fixture(true).await;
    let r = register(&f, "worker");
    let accepted = accept(&f, &r.pending.id, true);
    drop(f.engine.invalidate_all_ui());
    assert!(matches!(
        accepted.run(TaskCancellation::default()),
        Err(SessionError::AuthenticationExpired)
    ));
    let task = accept(&f, &r.pending.id, true)
        .run(TaskCancellation::default())
        .unwrap();
    drop(f.engine.invalidate_all_ui());
    assert!(matches!(task.await, ConfirmationOutcome::SpawnFailed(_)));
    assert_eq!(f.executor.spawns.load(Ordering::SeqCst), 1);
    assert!(matches!(
        wait(r.waiter).await,
        ConfirmationWaitOutcome::Decided(ConfirmationOutcome::SpawnFailed(_))
    ));
}
#[tokio::test]
async fn pending_survives_disconnect_and_warm_reattach_uses_current_parent_binding() {
    let f = fixture(true).await;
    let r = register(&f, "worker");
    drop(f.engine.disconnected(f.parent, now()).unwrap());
    assert_eq!(f.engine.pending().len(), 1);
    let reattached = f
        .engine
        .reattach(
            ReattachRequest {
                restored_metadata: None,
                message: proto::Message {
                    session_id: f.parent.session.0,
                    provider: "codex".into(),
                    cwd: f._root.path().to_string_lossy().into_owned(),
                    pid: 17,
                    ..Default::default()
                },
            },
            WrapperConnectionId(2),
            now(),
        )
        .await
        .unwrap();
    let current = reattached.binding;
    drop(reattached.after_reattached);
    assert_eq!(current.session, f.parent.session);
    assert_ne!(current.wrapper, f.parent.wrapper);
    let task = accept(&f, &r.pending.id, true)
        .run(TaskCancellation::default())
        .unwrap();
    assert!(matches!(task.await, ConfirmationOutcome::SpawnFailed(_)));
    assert_eq!(*lock(&f.executor.parent_seen), Some(current));
}
#[tokio::test]
async fn waiter_gone_only_before_handoff_triggers_one_post_spawn_notification() {
    for gone_before in [true, false] {
        let f = fixture(true).await;
        let r = register(&f, "worker");
        let mut waiter = Some(r.waiter);
        if gone_before {
            drop(waiter.take());
        }
        let task = accept(&f, &r.pending.id, true)
            .run(TaskCancellation::default())
            .unwrap();
        if !gone_before {
            drop(waiter.take());
        }
        assert!(matches!(task.await, ConfirmationOutcome::SpawnFailed(_)));
        assert_eq!(
            f.executor.notifications.load(Ordering::SeqCst),
            usize::from(gone_before)
        );
        assert_eq!(f.executor.spawns.load(Ordering::SeqCst), 1);
    }
}
#[tokio::test]
async fn aborted_running_task_fails_waiter_without_resurrecting_accepted_decision() {
    let f = fixture(true).await;
    f.executor.pause_spawn.store(true, Ordering::SeqCst);
    let r = register(&f, "worker");
    let task = tokio::spawn(
        accept(&f, &r.pending.id, true)
            .run(TaskCancellation::default())
            .unwrap(),
    );
    tokio::time::timeout(Duration::from_secs(2), f.executor.entered.notified())
        .await
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(matches!(
        wait(r.waiter).await,
        ConfirmationWaitOutcome::Decided(ConfirmationOutcome::SpawnFailed(_))
    ));
    assert!(f.engine.pending().is_empty());
    assert!(
        !f.engine
            .admission_matches(&r.pending.admission, f.parent.session, 1)
    );
    assert_eq!(f.executor.spawns.load(Ordering::SeqCst), 1);
    assert_eq!(f.warnings.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn active_task_cancellation_drains_executor_and_releases_once() {
    let f = fixture(true).await;
    f.executor.pause_spawn.store(true, Ordering::SeqCst);
    let r = register(&f, "worker");
    let cancel = TaskCancellation::default();
    let task = tokio::spawn(accept(&f, &r.pending.id, true).run(cancel.clone()).unwrap());
    tokio::time::timeout(Duration::from_secs(2), f.executor.entered.notified())
        .await
        .unwrap();
    cancel.cancel();
    assert!(matches!(
        task.await.unwrap(),
        ConfirmationOutcome::SpawnFailed(_)
    ));
    assert!(
        !f.engine
            .admission_matches(&r.pending.admission, f.parent.session, 1)
    );
    assert_eq!(f.executor.spawns.load(Ordering::SeqCst), 1);
    assert_eq!(f.warnings.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn reconnect_dialogs_use_one_live_snapshot_before_approval_and_keep_priming() {
    let f = fixture(true).await;
    let first = register(&f, "worker");
    assert!(messages(&first.effects)[0].remember_permission);
    let mut earlier = request(f.parent.session, "earlier");
    earlier.requested_at = now() - Duration::from_secs(1);
    let earlier = SpawnConfirmations::register(f.engine.as_ref(), earlier).unwrap();
    let before = f.executor.snapshots.load(Ordering::SeqCst);
    f.executor.forget_permission.store(true, Ordering::SeqCst);
    let weak = Arc::downgrade(&f.engine);
    *lock(&f.executor.snapshot_hook) = Some(Arc::new(move || {
        assert!(weak.upgrade().unwrap().state.try_lock().is_ok());
    }));
    let ui = UiBinding {
        connection: UiConnectionId(2),
        auth_epoch: f.engine.auth_epoch(),
    };
    let priming = f.engine.attach_ui(ui, None, None).unwrap();
    assert_eq!(f.executor.snapshots.load(Ordering::SeqCst), before + 1);
    assert_eq!(
        priming
            .ordered_frames
            .iter()
            .map(|m| m.r#type.as_str())
            .collect::<Vec<_>>(),
        [
            "spawn_confirmation_requested",
            "spawn_confirmation_requested",
            "approval_snapshot"
        ]
    );
    assert_eq!(
        priming.ordered_frames[0].spawn_confirmation_id,
        earlier.pending.id.0
    );
    assert_eq!(
        priming.ordered_frames[1].spawn_confirmation_id,
        first.pending.id.0
    );
    assert!(!priming.ordered_frames[1].remember_permission);
    let later = register(&f, "later");
    let drained = f.engine.finish_ui_priming(ui).unwrap();
    assert!(
        matches!(&drained.0[..], [CoreEffect::SendUi { message, .. }]
        if message.spawn_confirmation_id == later.pending.id.0)
    );
    assert!(lock(&f.engine.state).uis[&ui.connection].priming);
    assert!(f.engine.finish_ui_priming(ui).unwrap().0.is_empty());
    assert!(!lock(&f.engine.state).uis[&ui.connection].priming);
}

#[tokio::test]
async fn ordinary_ui_attach_needs_no_confirmation_executor_or_unused_snapshot() {
    for available in [false, true] {
        let f = fixture(available).await;
        f.executor.snapshot_fails.store(true, Ordering::SeqCst);
        let before = f.executor.snapshots.load(Ordering::SeqCst);
        let ui = UiBinding {
            connection: UiConnectionId(2),
            auth_epoch: f.engine.auth_epoch(),
        };
        assert!(f.engine.attach_ui(ui, None, None).is_ok());
        assert_eq!(f.executor.snapshots.load(Ordering::SeqCst), before);
    }
}

#[tokio::test]
async fn pending_disclosure_failure_leaves_ui_unattached_and_request_intact() {
    let f = fixture(true).await;
    let r = register(&f, "worker");
    f.executor.snapshot_fails.store(true, Ordering::SeqCst);
    let ui = UiBinding {
        connection: UiConnectionId(2),
        auth_epoch: f.engine.auth_epoch(),
    };
    assert!(f.engine.attach_ui(ui, None, None).is_err());
    assert!(!lock(&f.engine.state).uis.contains_key(&ui.connection));
    assert_eq!(f.engine.pending()[0].id, r.pending.id);
}

#[tokio::test]
async fn attach_rechecks_auth_and_pending_after_unlocked_snapshot_callback() {
    let f = fixture(true).await;
    let r = register(&f, "worker");
    let weak = Arc::downgrade(&f.engine);
    *lock(&f.executor.snapshot_hook) = Some(Arc::new(move || {
        drop(weak.upgrade().unwrap().invalidate_all_ui());
    }));
    let ui = UiBinding {
        connection: UiConnectionId(2),
        auth_epoch: f.engine.auth_epoch(),
    };
    assert!(matches!(
        f.engine.attach_ui(ui, None, None),
        Err(SessionError::AuthenticationExpired)
    ));
    assert!(!lock(&f.engine.state).uis.contains_key(&ui.connection));
    let weak = Arc::downgrade(&f.engine);
    *lock(&f.executor.snapshot_hook) = Some(Arc::new(move || {
        weak.upgrade().unwrap().shutdown_confirmations();
    }));
    f.executor.snapshot_fails.store(true, Ordering::SeqCst);
    let ui = UiBinding {
        auth_epoch: f.engine.auth_epoch(),
        ..ui
    };
    let priming = f.engine.attach_ui(ui, None, None).unwrap();
    assert_eq!(priming.ordered_frames.len(), 1);
    assert_eq!(priming.ordered_frames[0].r#type, "approval_snapshot");
    assert_eq!(wait(r.waiter).await, ConfirmationWaitOutcome::HubStopped);
}

#[tokio::test]
async fn shutdown_finishes_only_pending_waiters_and_leaves_committed_tasks_owned() {
    let f = fixture(true).await;
    let pending = register(&f, "pending");
    let lease = accept(&f, &pending.pending.id, true);
    let committed = register(&f, "committed");
    let task = accept(&f, &committed.pending.id, true)
        .run(TaskCancellation::default())
        .unwrap();
    f.engine.shutdown_confirmations();
    f.engine.shutdown_confirmations();
    assert_eq!(
        wait(pending.waiter).await,
        ConfirmationWaitOutcome::HubStopped
    );
    assert!(
        !f.engine
            .admission_matches(&pending.pending.admission, f.parent.session, 1)
    );
    assert!(
        f.engine
            .admission_matches(&committed.pending.admission, f.parent.session, 1)
    );
    assert!(matches!(
        lease.run(TaskCancellation::default()),
        Err(SessionError::Shutdown)
    ));
    assert!(f.engine.pending().is_empty());
    assert!(matches!(
        SpawnConfirmations::register(f.engine.as_ref(), request(f.parent.session, "late")),
        Err(AdmissionError::Unavailable)
    ));
    assert!(lock(&f.sink.messages).is_empty());
    drop(task);
    assert!(matches!(
        wait(committed.waiter).await,
        ConfirmationWaitOutcome::Decided(ConfirmationOutcome::SpawnFailed(_))
    ));
    assert!(
        !f.engine
            .admission_matches(&committed.pending.admission, f.parent.session, 1)
    );
}

#[tokio::test]
async fn shutdown_during_disclosure_cannot_publish_or_reserve_a_new_dialog() {
    let f = fixture(true).await;
    let weak = Arc::downgrade(&f.engine);
    *lock(&f.executor.snapshot_hook) = Some(Arc::new(move || {
        weak.upgrade().unwrap().shutdown_confirmations();
    }));
    assert!(matches!(
        SpawnConfirmations::register(f.engine.as_ref(), request(f.parent.session, "worker")),
        Err(AdmissionError::Unavailable)
    ));
    assert!(f.engine.pending().is_empty());
    assert_eq!(lock(&f.engine.state).next_confirmation, 0);
    assert!(
        lock(&f.engine.state)
            .admission
            .reserve_confirmation(f.parent.session, 256, None)
            .is_ok()
    );
}

#[tokio::test]
async fn served_confirmation_uses_ordinary_guard_and_transfers_to_the_real_task_owner() {
    use crate::hub::{
        ServiceRouter, confirmations::ConfirmationHttp, task_owner::HubTaskOwner, transport,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let f = fixture(true).await;
    f.engine.detach_ui(UiBinding {
        connection: UiConnectionId(1),
        auth_epoch: f.engine.auth_epoch(),
    });
    let pending = register(&f, "worker");
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let paths = RuntimePaths::trial(f._root.path(), port, f._installed.path()).unwrap();
    let config = Arc::new(
        config::ConfigStore::load_or_create(paths.clone(), || Ok("synthetic-confirm-token".into()))
            .unwrap(),
    );
    let owner = HubTaskOwner::new(tokio::runtime::Handle::current());
    let services = Arc::new(
        ServiceRouter::new(config, paths, f.engine.clone(), port)
            .unwrap()
            .with_task_owner(owner.handle())
            .with_confirmations(Arc::new(ConfirmationHttp::new(f.engine.clone()))),
    );
    let cancel = crate::process::Cancellation::default();
    let server = tokio::spawn(transport::serve(
        listener,
        services,
        f.sink.clone(),
        cancel.clone(),
    ));
    // URL parent is validated but is not compared with the pending parent in
    // fixed Go handleSpawnConfirmation. Keep that inherited single-user behavior.
    let body = serde_json::to_vec(&response(&pending.pending.id, false)).unwrap();
    let mut client = tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
    let header = format!(
        "POST /api/sessions/999999/spawn-confirm?token=synthetic-confirm-token HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    client.write_all(header.as_bytes()).await.unwrap();
    client.write_all(&body).await.unwrap();
    let mut bytes = vec![];
    tokio::time::timeout(Duration::from_secs(3), client.read_to_end(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert!(String::from_utf8_lossy(&bytes).starts_with("HTTP/1.1 200"));
    assert_eq!(
        wait(pending.waiter).await,
        ConfirmationWaitOutcome::Decided(ConfirmationOutcome::Refused)
    );
    assert_eq!(f.executor.refusals.load(Ordering::SeqCst), 1);
    owner.stop_requests();
    assert_eq!(owner.drain_requests().await.completed, 1);
    f.engine.shutdown_confirmations();
    owner.stop_effects().unwrap();
    assert_eq!(owner.drain_effects().await.completed, 1);
    cancel.cancel();
    server.await.unwrap().unwrap();
}
