use super::*;
use crate::{
    application::{
        orchestration_program::OrchestrationDependencies, session_workers::SessionWorkers,
    },
    config::{Config, ConfigStore, RuntimePaths},
    files::FilesService,
    hub::task_owner::HubTaskOwner,
    orchestration::child_launch::{self, ChildLaunchServices, RegisteredChild},
    terminal::{
        events::CoreEventBus,
        journal::{JournalOptions, SessionJournal},
        session::{
            EngineOptions,
            confirmations::{ConfirmationExecutor, ConfirmationPresentation, ConfirmedChildRequest},
        },
    },
};
use std::{
    path::PathBuf,
    sync::{Mutex, Weak},
    time::Duration,
};
use tokio::sync::Semaphore;

fn lock<T>(value: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    value.lock().unwrap()
}

struct Io {
    owner: Mutex<Weak<OrchestrationProgram>>,
    dismissed: Semaphore,
    sent: Mutex<Vec<Vec<u8>>>,
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
            lock(&self.sent).push(message.data);
            Ok(())
        })
    }
}
// Mirror the production observer's dismissal callback. Drop the weak-owner
// mutex before awaiting: session_ended applies further effects through this sink.
impl CoreEffectSink for Io {
    fn apply<'a>(&'a self, effects: CoreEffects) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
        Box::pin(async move {
            for (index, effect) in effects.0.into_iter().enumerate() {
                match effect {
                    CoreEffect::CancelSession { binding, reason } => {
                        assert!(matches!(reason, StopReason::Dismissed));
                        lock(&self.stopped).push(binding.session);
                    }
                    CoreEffect::Notify(CoreEvent::Dismissed(id)) => {
                        let owner = lock(&self.owner).upgrade().unwrap();
                        owner
                            .session_ended(id, "dismissed")
                            .await
                            .map_err(|error| CoreEffectFailure { index, error })?;
                        self.dismissed.add_permits(1);
                    }
                    _ => {}
                }
            }
            Ok(())
        })
    }
}
struct Confirmations;
impl ConfirmationExecutor for Confirmations {
    fn presentation(&self) -> Result<ConfirmationPresentation, SessionError> {
        Ok(ConfirmationPresentation {
            config: Config::default(),
            trust_grant_providers: Vec::new(),
        })
    }
    fn spawn<'a>(
        &'a self,
        _: ConfirmedChildRequest,
        _: TaskCancellation,
    ) -> CoreFuture<'a, Result<ChildSpawnResult, SessionError>> {
        Box::pin(async { panic!("this fixture only refuses confirmations") })
    }
    fn record_refusal<'a>(
        &'a self,
        _: &'a PendingSpawnConfirmation,
        _: TaskCancellation,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async { Ok(()) })
    }
    fn notify_waiter_gone<'a>(
        &'a self,
        _: &'a PendingSpawnConfirmation,
        _: &'a Result<ChildSpawnResult, SessionError>,
        _: TaskCancellation,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async { Ok(()) })
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
    _root: tempfile::TempDir,
    runtime: PathBuf,
    control: ChildControl,
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
    let mut cfg = Config::default();
    cfg.orchestration.child_timeout_seconds = 0;
    cfg.orchestration.idle_done_threshold_sec = 0;
    cfg.orchestration.child_startup_fail = Some(false);
    let config = Arc::new(ConfigStore::new(paths.clone(), cfg).unwrap());
    let tasks = HubTaskOwner::new(tokio::runtime::Handle::current());
    let io = Arc::new(Io {
        owner: Mutex::new(Weak::new()),
        dismissed: Semaphore::new(0),
        sent: Mutex::new(Vec::new()),
        engine: Mutex::new(Weak::new()),
        starts: std::sync::atomic::AtomicU64::new(0),
        stopped: Mutex::new(Vec::new()),
    });
    let effects: Arc<dyn CoreEffectSink> = io.clone();
    let core = Arc::new(SessionEngine::new(
        EngineOptions {
            confirmation_executor: Some(Arc::new(Confirmations)),
            ..Default::default()
        },
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
        Arc::new(|_, error| panic!("unexpected worker failure: {error:?}")),
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
        hub_cwd: runtime.clone(),
        workers,
        tasks: tasks.handle(),
        warning: Arc::new(|_, error| panic!("unexpected worker failure: {error:?}")),
    })
    .unwrap();
    let id = owner
        .reserve_conductor(&mut String::new(), None, at)
        .unwrap();
    let snapshot = core.snapshot(binding.session).unwrap();
    let path = owner
        .board_store()
        .ensure(&id.0, &snapshot, "synthetic", at)
        .unwrap();
    core.mark_conductor(binding, id.clone(), path.to_string_lossy().into_owned())
        .unwrap();
    owner.register_conductor(binding, &id, &path).unwrap();
    *lock(&io.owner) = Arc::downgrade(&owner);
    let control = ChildControl::new(
        core.clone(),
        owner.clone(),
        tasks.handle(),
        Arc::new(|_, error| panic!("unexpected effect failure: {error:?}")),
    );
    Fixture {
        _root: root,
        runtime,
        control,
        _tasks: tasks,
        core,
        owner,
        io,
        binding,
        id,
        path,
    }
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
        cwd: f.runtime.clone(),
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
                    absolute_cwd: f.runtime.clone(),
                    child_cwd: f.runtime.clone(),
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

fn close(f: &Fixture, role: &str, id: Option<i64>) -> Response {
    let mut payload = serde_json::json!({"role": role});
    if let Some(id) = id {
        payload["id"] = id.into();
    }
    f.control.handle_authenticated(
        &Request {
            method: "POST".into(),
            path: format!("/api/sessions/{}/close-child", f.binding.session.0),
            body: serde_json::to_vec(&payload).unwrap(),
            ..Default::default()
        },
        Timestamp::now(),
    )
}
fn body(response: &Response) -> serde_json::Value {
    serde_json::from_slice(&response.body).unwrap()
}
fn evidence(f: &Fixture) -> String {
    std::fs::read_to_string(&f.path).unwrap()
}
async fn observed_dismissals(f: &Fixture, count: u32) {
    tokio::time::timeout(Duration::from_secs(5), f.io.dismissed.acquire_many(count))
        .await
        .expect("dismissal effects must reach the observer")
        .unwrap()
        .forget();
}
async fn delivered_notices(f: &Fixture, count: usize) -> String {
    f.core.evaluate_idle(Timestamp::now() + Duration::from_secs(4));
    let guard = f.owner.start().unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if lock(&f.io.sent).len() >= count {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("queued conductor notices must be delivered");
    guard.stop_and_join().await;
    lock(&f.io.sent)
        .iter()
        .map(|bytes| String::from_utf8(bytes.clone()).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
}
fn assert_conductor_record(f: &Fixture, child: SessionBinding, role: &str, state: &str) {
    let board = evidence(f);
    assert!(board.contains(&format!(
        "child closed by the conductor: role={role} session={} (state when closed: {state}; its work may be unfinished)",
        child.session.0
    )));
    assert!(board.contains(&format!(
        "[orchestration] child closed by you role={role} id={} state_before={state} (treat its work as unfinished unless it reported DONE)",
        child.session.0
    )));
    assert!(!board.contains("closed by the user"));
}

#[tokio::test]
async fn working_child_close_removes_session_and_delivers_conductor_notice() {
    let f = fixture().await;
    let binding = child(&f, "worker", Timestamp::now()).await;
    f.core
        .observe_output(
            binding,
            OutputChunk {
                bytes: b"working on synthetic task".to_vec(),
                total_pty_bytes: 0,
            },
            Timestamp::now(),
        )
        .unwrap();
    let state = f.core.snapshot(binding.session).unwrap().state;
    assert_eq!(state, "running");
    let response = close(&f, " Worker ", None);
    assert_eq!(response.status, 200);
    assert_eq!(
        body(&response),
        serde_json::json!({
            "ok": true, "session_id": binding.session.0, "role": "worker",
            "state_before": state, "board_path": f.path.to_string_lossy()
        })
    );
    assert!(f.core.details(binding.session).is_none());
    observed_dismissals(&f, 1).await;
    assert_eq!(*lock(&f.io.stopped), vec![binding.session]);
    assert_conductor_record(&f, binding, "worker", &state);
    let notice = delivered_notices(&f, 1).await;
    assert!(notice.contains(&format!(
        "child closed by you role=worker id={} state_before=running",
        binding.session.0
    )));
    assert!(!notice.contains("closed from the dashboard"));
}

#[tokio::test]
async fn completed_and_disconnected_children_are_closed_with_original_state() {
    for state in ["completed", "disconnected"] {
        let f = fixture().await;
        let binding = child(&f, "review", Timestamp::now()).await;
        if state == "completed" {
            f.core
                .observe_end(
                    binding,
                    SessionEnd {
                        declared_state: state.into(),
                        exit_code: 0,
                        reason: String::new(),
                    },
                    Timestamp::now(),
                )
                .unwrap();
        } else {
            f.core.disconnected(binding, Timestamp::now()).unwrap();
        }
        assert_eq!(f.core.snapshot(binding.session).unwrap().state, state);
        let response = close(&f, "review", None);
        assert_eq!(response.status, 200);
        assert_eq!(body(&response)["state_before"], state);
        assert!(f.core.snapshot(binding.session).is_none());
        observed_dismissals(&f, 1).await;
        assert_conductor_record(&f, binding, "review", state);
        if state == "disconnected" {
            assert!(lock(&f.io.stopped).is_empty());
        }
        let notice = delivered_notices(&f, 1).await;
        assert!(notice.contains(&format!(
            "child closed by you role=review id={} state_before={state}",
            binding.session.0
        )));
    }
}

#[tokio::test]
async fn explicit_id_selects_exact_child_and_role_only_selects_newest() {
    let f = fixture().await;
    let first = child(&f, "worker", Timestamp::now()).await;
    let second = child(&f, "worker", Timestamp::now()).await;
    let third = child(&f, "worker", Timestamp::now()).await;
    let response = close(&f, "worker", Some(first.session.0));
    assert_eq!(response.status, 200);
    assert_eq!(body(&response)["session_id"], first.session.0);
    assert!(f.core.snapshot(first.session).is_none());
    assert!(f.core.snapshot(second.session).is_some());
    assert!(f.core.snapshot(third.session).is_some());
    let response = close(&f, "worker", None);
    assert_eq!(response.status, 200);
    assert_eq!(body(&response)["session_id"], third.session.0);
    assert!(f.core.snapshot(third.session).is_none());
    assert!(f.core.snapshot(second.session).is_some());
    observed_dismissals(&f, 2).await;
    assert_eq!(close(&f, "worker", Some(first.session.0)).status, 404);
}

#[tokio::test]
async fn another_parents_children_are_indistinguishable_from_unknown_children() {
    let mut f = fixture().await;
    let caller = f.binding;
    let own = child(&f, "own-worker", Timestamp::now()).await;
    let other = f
        .core
        .register(
            RegisterRequest {
                message: crate::proto::Message {
                    provider: "claude".into(),
                    cwd: f.runtime.to_string_lossy().into_owned(),
                    pid: 999,
                    cols: 120,
                    rows: 30,
                    ..Default::default()
                },
                spawn_proof: None,
            },
            WrapperConnectionId(999),
            Timestamp::now(),
        )
        .await
        .unwrap()
        .binding;
    // Give the second parent a distinct conductor board, as in production.
    let own_id = f.id.clone();
    let own_path = f.path.clone();
    f.binding = other;
    f.id = f
        .owner
        .reserve_conductor(&mut String::new(), None, Timestamp::now())
        .unwrap();
    f.path = f
        .owner
        .board_store()
        .ensure(
            &f.id.0,
            &f.core.snapshot(other.session).unwrap(),
            "synthetic",
            Timestamp::now(),
        )
        .unwrap();
    f.core
        .mark_conductor(other, f.id.clone(), f.path.to_string_lossy().into_owned())
        .unwrap();
    f.owner.register_conductor(other, &f.id, &f.path).unwrap();
    let foreign = child(&f, "private-worker", Timestamp::now()).await;
    f.binding = caller;
    f.id = own_id;
    f.path = own_path;
    let unknown = close(&f, "unknown-role", None);
    assert_eq!(unknown.status, 404);
    let text = String::from_utf8(unknown.body.clone()).unwrap();
    let own_state = f.core.snapshot(own.session).unwrap().state;
    assert!(text.contains(&format!("role=own-worker id={} state={own_state}", own.session.0)));
    for response in [
        close(&f, "private-worker", None),
        close(&f, "private-worker", Some(foreign.session.0)),
        close(&f, "own-worker", Some(foreign.session.0)),
        close(&f, "private-worker", Some(i64::MAX)),
    ] {
        assert_eq!(response.status, 404);
        assert_eq!(response.body, unknown.body);
    }
    assert!(!text.contains("private-worker"));
    assert!(!text.contains(&format!("id={}", foreign.session.0)));
    assert!(f.core.snapshot(foreign.session).is_some());
    assert!(f.core.snapshot(own.session).is_some());
    assert!(lock(&f.io.stopped).is_empty());
}

async fn dashboard_dismiss(f: &Fixture, child: SessionBinding) {
    let ui = UiBinding {
        connection: UiConnectionId(1),
        auth_epoch: f.core.auth_epoch(),
    };
    f.core.attach_ui(ui, None, None).unwrap();
    let effects = f
        .core
        .dismiss_from_ui(ui, child.session, Timestamp::now())
        .unwrap();
    f.io.apply(effects).await.unwrap();
}

#[tokio::test]
async fn pending_confirmation_refuses_close_and_does_not_poison_dashboard_attribution() {
    let f = fixture().await;
    let binding = child(&f, "worker", Timestamp::now()).await;
    let before = f.core.snapshot(binding.session).unwrap().state;
    let pending = SpawnConfirmations::register(
        &*f.core,
        ConfirmationRequest {
            parent: binding.session,
            requested_provider: "claude".into(),
            body: ResolvedChildSpawn::from_request(
                ChildSpawnRequest {
                    role: "grandchild".into(),
                    provider: "claude".into(),
                    ..Default::default()
                },
                None,
                InternalSpawnGrants::default(),
            )
            .unwrap(),
            requested_at: Timestamp::now(),
        },
    )
    .unwrap();
    let response = close(&f, "worker", None);
    assert_eq!(response.status, 409);
    assert!(
        String::from_utf8(response.body)
            .unwrap()
            .contains("pending spawn confirmations")
    );
    assert_eq!(f.core.snapshot(binding.session).unwrap().state, before);
    assert!(f.core.details(binding.session).unwrap().connected);
    assert_eq!(f.core.pending().len(), 1);
    assert!(lock(&f.io.stopped).is_empty());
    assert!(!evidence(&f).contains("closed by the conductor"));
    let decision = f
        .core
        .clone()
        .accept_decision(
            SpawnConfirmationResponse {
                confirmation_id: pending.pending.id,
                approved: false,
                ..Default::default()
            },
            VerifiedConfirmationRequest::after_server_authentication(AuthEpoch(0)),
            Timestamp::now(),
        )
        .unwrap();
    assert!(matches!(
        decision.run(TaskCancellation::default()).unwrap().await,
        ConfirmationOutcome::Refused
    ));
    assert!(f.core.pending().is_empty());
    dashboard_dismiss(&f, binding).await;
    observed_dismissals(&f, 1).await;
    let board = evidence(&f);
    assert!(board.contains("closed by the user; its work may be unfinished"));
    assert!(!board.contains("closed by the conductor"));
    let notice = delivered_notices(&f, 1).await;
    assert!(notice.contains("closed from the dashboard by the user; treat its work as unfinished"));
    assert!(!notice.contains("child closed by you"));
}

#[tokio::test]
async fn rapid_conductor_closes_keep_marks_separate_from_dashboard_child() {
    let f = fixture().await;
    let first = child(&f, "first", Timestamp::now()).await;
    let second = child(&f, "second", Timestamp::now()).await;
    let dashboard = child(&f, "dashboard", Timestamp::now()).await;
    f.core
        .observe_end(
            second,
            SessionEnd {
                declared_state: "completed".into(),
                exit_code: 0,
                reason: String::new(),
            },
            Timestamp::now(),
        )
        .unwrap();
    let first_state = f.core.snapshot(first.session).unwrap().state;
    // Both close requests are admitted before yielding to their effect tasks.
    assert_eq!(close(&f, "first", None).status, 200);
    assert_eq!(close(&f, "second", None).status, 200);
    dashboard_dismiss(&f, dashboard).await;
    observed_dismissals(&f, 3).await;
    let board = evidence(&f);
    assert!(board.contains(&format!(
        "child closed by the conductor: role=first session={} (state when closed: {first_state};",
        first.session.0
    )));
    assert!(board.contains(&format!(
        "child closed by the conductor: role=second session={} (state when closed: completed;",
        second.session.0
    )));
    assert!(board.contains(&format!(
        "child dismissed from the dashboard: role=dashboard session={} (closed by the user;",
        dashboard.session.0
    )));
    assert!(!board.contains("child closed by the conductor: role=dashboard"));
    assert!(!board.contains("child dismissed from the dashboard: role=first"));
    assert!(!board.contains("child dismissed from the dashboard: role=second"));
    let notices = delivered_notices(&f, 3).await;
    assert!(notices.contains("child closed by you role=first"));
    assert!(notices.contains("child closed by you role=second"));
    assert!(notices.contains("child dismissed role=dashboard"));
}

#[tokio::test]
async fn closed_child_releases_parent_capacity_for_same_role_replacement() {
    let f = fixture().await;
    let binding = child(&f, "worker", Timestamp::now()).await;
    let request = AdmissionRequest {
        parent: f.binding.session,
        slots: 1,
        origin: VerifiedSpawnOrigin::Autonomous,
        replace: None,
    };
    let limits = AdmissionLimits {
        max_children_per_parent: 1,
        max_total_sessions: 8,
    };
    assert!(matches!(
        f.core.reserve_children(request.clone(), limits),
        Err(AdmissionError::ChildrenPerParent { .. })
    ));
    assert_eq!(close(&f, "worker", None).status, 200);
    observed_dismissals(&f, 1).await;
    let admission = f.core.reserve_children(request, limits).unwrap();
    assert!(f.core.release_children(&admission.id));
    let replacement = child(&f, "worker", Timestamp::now()).await;
    assert!(replacement.session != binding.session);
    assert!(f.core.snapshot(binding.session).is_none());
    assert_eq!(
        f.core
            .snapshots()
            .iter()
            .filter(|session| session.parent_session_id == f.binding.session)
            .count(),
        1
    );
}

#[tokio::test]
async fn close_route_rejects_invalid_requests_without_removing_child() {
    let f = fixture().await;
    let binding = child(&f, "worker", Timestamp::now()).await;
    assert!(is_control_route("/api/sessions/1/close-child"));
    for response in [
        close(&f, "", None),
        close(&f, "  ", None),
        close(&f, "worker", Some(0)),
        close(&f, "worker", Some(-1)),
    ] {
        assert_eq!(response.status, 400);
    }
    let response = f.control.handle_authenticated(
        &Request {
            method: "GET".into(),
            path: format!("/api/sessions/{}/close-child", f.binding.session.0),
            ..Default::default()
        },
        Timestamp::now(),
    );
    assert_eq!(response.status, 405);
    assert!(f.core.snapshot(binding.session).is_some());
}

#[tokio::test]
async fn relay_child_explicit_close_still_records_and_delivers_conductor_attribution() {
    let f = fixture().await;
    let binding = child(&f, "relay-worker", Timestamp::now()).await;
    let state = f.core.snapshot(binding.session).unwrap().state;
    f.owner.claim_relay_board(&f.id);
    assert_eq!(close(&f, "relay-worker", None).status, 200);
    assert!(f.core.snapshot(binding.session).is_none());
    observed_dismissals(&f, 1).await;
    assert_conductor_record(&f, binding, "relay-worker", &state);
    let notice = delivered_notices(&f, 1).await;
    assert!(notice.contains("child closed by you role=relay-worker"));
}

#[tokio::test]
async fn null_id_is_optional_but_blank_string_id_is_invalid() {
    let f = fixture().await;
    let binding = child(&f, "worker", Timestamp::now()).await;
    let mut request = Request {
        method: "POST".into(),
        path: format!("/api/sessions/{}/close-child", f.binding.session.0),
        body: br#"{"role":"worker","id":""}"#.to_vec(),
        ..Default::default()
    };
    assert_eq!(
        f.control.handle_authenticated(&request, Timestamp::now()).status,
        400
    );
    assert!(f.core.snapshot(binding.session).is_some());
    request.body = br#"{"role":"worker","id":null}"#.to_vec();
    let response = f.control.handle_authenticated(&request, Timestamp::now());
    assert_eq!(response.status, 200);
    assert_eq!(body(&response)["session_id"], binding.session.0);
    observed_dismissals(&f, 1).await;
}

#[tokio::test]
async fn warm_reattach_preserves_close_attribution_without_accepting_stale_wrapper() {
    for from_dashboard in [false, true] {
        let f = fixture().await;
        let original = child(&f, "worker", Timestamp::now()).await;
        let snapshot = f.core.snapshot(original.session).unwrap();
        let current = f
            .core
            .reattach(
                ReattachRequest {
                    restored_metadata: None,
                    message: crate::proto::Message {
                        session_id: original.session.0,
                        provider: snapshot.provider,
                        cwd: snapshot.cwd,
                        // The synthetic spawner uses its wrapper number as PID.
                        pid: original.wrapper.0 as i64,
                        cols: 120,
                        rows: 30,
                        ..Default::default()
                    },
                },
                WrapperConnectionId(900),
                Timestamp::now(),
            )
            .await
            .unwrap()
            .binding;
        assert!(current != original);
        assert_eq!(current.session, original.session);
        assert_eq!(current.incarnation, original.incarnation);
        assert!(f.core.details(current.session).unwrap().binding == current);
        // The board still owns the launch-time registration. Attribution must
        // follow this incarnation, while dismissal still rejects stale wrappers.
        assert!(matches!(
            f.owner.close_child_by_conductor(
                f.binding.session,
                original,
                Timestamp::now(),
            ),
            Err(SessionError::StaleBinding)
        ));
        assert!(f.core.snapshot(current.session).is_some());
        let state = f.core.snapshot(current.session).unwrap().state;
        if from_dashboard {
            dashboard_dismiss(&f, current).await;
        } else {
            let response = close(&f, "worker", Some(current.session.0));
            assert_eq!(response.status, 200);
            assert_eq!(body(&response)["state_before"], state);
        }
        assert!(f.core.snapshot(current.session).is_none());
        observed_dismissals(&f, 1).await;
        assert_eq!(*lock(&f.io.stopped), vec![current.session]);
        if from_dashboard {
            let board = evidence(&f);
            assert!(board.contains("closed by the user; its work may be unfinished"));
            assert!(!board.contains("closed by the conductor"));
            let notice = delivered_notices(&f, 1).await;
            assert!(notice.contains("closed from the dashboard by the user"));
            assert!(!notice.contains("child closed by you"));
        } else {
            assert_conductor_record(&f, current, "worker", &state);
            let notice = delivered_notices(&f, 1).await;
            assert!(notice.contains(&format!(
                "child closed by you role=worker id={} state_before={state}",
                current.session.0
            )));
            assert!(!notice.contains("closed from the dashboard"));
        }
    }
}
