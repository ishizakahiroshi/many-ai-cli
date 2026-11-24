use super::*;
use crate::{
    application::{
        orchestration_program::OrchestrationDependencies, session_workers::SessionWorkers,
    },
    config::{Config, RuntimePaths},
    files::FilesService,
    hub::task_owner::HubTaskOwner,
    orchestration::child_launch::{ChildLaunchExecutor, ChildPreparer, worktree::WorktreeGit},
    terminal::{
        events::CoreEventBus,
        journal::{JournalOptions, SessionJournal},
        session::{EngineOptions, confirmations::ConfirmationPresentation},
    },
};
use std::{
    sync::{
        Mutex, OnceLock, Weak,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::Semaphore;
struct Io {
    core: Mutex<Weak<SessionEngine>>,
    entered: Semaphore,
    release: Semaphore,
    starts: AtomicUsize,
    sent: Mutex<Vec<Vec<u8>>>,
    specs: Mutex<Vec<WrappedSpawnSpec>>,
}
impl WrapperTransport for Io {
    fn send<'a>(
        &'a self,
        _: SessionBinding,
        message: crate::proto::Message,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            self.sent.lock().unwrap().push(message.data);
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
            let mut dispatch = || {
                self.sent.lock().unwrap().push(message.data.clone());
                Ok(())
            };
            guard(&mut dispatch)
        })
    }
}
impl CoreEffectSink for Io {
    fn apply<'a>(&'a self, _: CoreEffects) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
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
            let number = self.starts.fetch_add(1, Ordering::SeqCst) + 100;
            self.specs.lock().unwrap().push(spec.clone());
            self.entered.add_permits(1);
            self.release.acquire().await.unwrap().forget();
            let core = self.core.lock().unwrap().upgrade().unwrap();
            let registered = core
                .register(
                    RegisterRequest {
                        message: crate::proto::Message {
                            provider: spec.provider,
                            cwd: spec.cwd.to_string_lossy().into_owned(),
                            label: spec.label,
                            pid: number as i64,
                            cols: 80,
                            rows: 24,
                            ..Default::default()
                        },
                        spawn_proof: spec
                            .registration_proof
                            .map(|proof| proof.as_header_value().to_owned()),
                    },
                    WrapperConnectionId(number as u64),
                    Timestamp::now(),
                )
                .await
                .unwrap();
            SpawnWaitOutcome::Registered(registered.binding)
        })
    }
}
#[derive(Default)]
struct BoundExecutor(OnceLock<Arc<dyn ConfirmationExecutor>>);
impl ConfirmationExecutor for BoundExecutor {
    fn presentation(&self) -> Result<ConfirmationPresentation, SessionError> {
        self.0.get().unwrap().presentation()
    }
    fn prepare_billing(
        &self,
        body: ResolvedChildSpawn,
        shown_revision: Option<&str>,
    ) -> Result<ResolvedChildSpawn, SessionError> {
        self.0.get().unwrap().prepare_billing(body, shown_revision)
    }
    fn spawn<'a>(
        &'a self,
        r: ConfirmedChildRequest,
        c: TaskCancellation,
    ) -> CoreFuture<'a, Result<ChildSpawnResult, SessionError>> {
        self.0.get().unwrap().spawn(r, c)
    }
    fn record_refusal<'a>(
        &'a self,
        r: &'a PendingSpawnConfirmation,
        c: TaskCancellation,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        self.0.get().unwrap().record_refusal(r, c)
    }
    fn notify_waiter_gone<'a>(
        &'a self,
        r: &'a PendingSpawnConfirmation,
        outcome: &'a Result<ChildSpawnResult, SessionError>,
        c: TaskCancellation,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        self.0.get().unwrap().notify_waiter_gone(r, outcome, c)
    }
}
struct Fixture {
    _tasks: HubTaskOwner,
    core: Arc<SessionEngine>,
    route: Arc<ChildHttp>,
    io: Arc<Io>,
    parent: SessionBinding,
    paths: RuntimePaths,
    // Last: fields drop in this order, and on Windows the folder cannot be
    // removed while a field above still holds a file inside it.
    _root: tempfile::TempDir,
}
async fn fixture(confirm: &str, limit: i64) -> Fixture {
    fixture_with_registry(
        confirm,
        limit,
        Arc::new(|| Err(std::io::Error::other("synthetic fixture forbids providers"))),
    )
    .await
}
async fn fixture_with_registry(
    confirm: &str,
    limit: i64,
    registry: crate::application::spawn_policy::RegistrySnapshot,
) -> Fixture {
    fixture_with_parent(confirm, limit, registry, "copilot").await
}
async fn fixture_with_parent(
    confirm: &str,
    limit: i64,
    registry: crate::application::spawn_policy::RegistrySnapshot,
    parent_provider: &str,
) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let runtime = root.path().join("trial");
    std::fs::create_dir(&runtime).unwrap();
    let project = runtime.join("project");
    std::fs::create_dir(&project).unwrap();
    let paths = RuntimePaths::trial(&runtime, 49677, &root.path().join("installed")).unwrap();
    let mut cfg = Config::default();
    cfg.orchestration.spawn_confirm_mode = confirm.into();
    cfg.orchestration.max_children_per_parent = limit;
    let config = Arc::new(ConfigStore::new(paths.clone(), cfg).unwrap());
    let tasks = HubTaskOwner::new(tokio::runtime::Handle::current());
    let io = Arc::new(Io {
        core: Mutex::new(Weak::new()),
        entered: Semaphore::new(0),
        release: Semaphore::new(0),
        starts: AtomicUsize::new(0),
        sent: Mutex::new(Vec::new()),
        specs: Mutex::new(Vec::new()),
    });
    let effects: Arc<dyn CoreEffectSink> = io.clone();
    let bound = Arc::new(BoundExecutor::default());
    let core = Arc::new(SessionEngine::new(
        EngineOptions {
            confirmation_executor: Some(bound.clone()),
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
    *io.core.lock().unwrap() = Arc::downgrade(&core);
    let parent = core
        .register(
            RegisterRequest {
                message: crate::proto::Message {
                    provider: parent_provider.into(),
                    cwd: project.to_string_lossy().into_owned(),
                    pid: 7,
                    cols: 80,
                    rows: 24,
                    ..Default::default()
                },
                spawn_proof: None,
            },
            WrapperConnectionId(1),
            Timestamp::now(),
        )
        .await
        .unwrap()
        .binding;
    let workers = SessionWorkers::new(
        config.clone(),
        paths.clone(),
        Arc::new(FilesService::new(project.clone(), paths.clone())),
        tasks.handle(),
        Arc::new(|_, _| {}),
    );
    workers
        .bind(Arc::downgrade(&core), Arc::downgrade(&effects))
        .unwrap();
    let program = OrchestrationProgram::new(OrchestrationDependencies {
        core: Arc::downgrade(&core),
        effects: Arc::downgrade(&effects),
        config: config.clone(),
        paths: paths.clone(),
        registry,
        environment: vec![],
        hub_cwd: project.clone(),
        workers,
        tasks: tasks.handle(),
        warning: Arc::new(|_, _| {}),
    })
    .unwrap();
    let preparation = ChildPreparer::new(
        &paths,
        project,
        None,
        WorktreeGit::new("git".into(), Default::default()),
    )
    .unwrap();
    let executor: Arc<dyn ConfirmationExecutor> = Arc::new(ChildLaunchExecutor::new(
        Arc::downgrade(&core),
        config.clone(),
        preparation,
        program.clone(),
    ));
    assert!(bound.0.set(executor.clone()).is_ok());
    let route = Arc::new(ChildHttp::new(
        core.clone(),
        program,
        executor,
        effects,
        config,
        tasks.handle(),
    ));
    Fixture {
        _root: root,
        _tasks: tasks,
        core,
        route,
        io,
        parent,
        paths,
    }
}
fn request(parent: SessionBinding, origin: &str) -> Request {
    Request {method:"POST".into(),path:format!("/api/sessions/{}/spawn-child",parent.session.0),body:serde_json::to_vec(&serde_json::json!({"role":"worker","provider":"copilot","same_tree":true,"origin":origin})).unwrap(),..Default::default()}
}
async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), future)
        .await
        .expect("synthetic lifecycle stalled")
}
#[tokio::test]
async fn cancelled_http_retains_direct_real_executor_launch_and_admission() {
    let f = fixture("off", 4).await;
    let route = f.route.clone();
    let req = request(f.parent, "");
    let wait = HttpWaitCancellation::default();
    let owned_wait = wait.clone();
    let http = tokio::spawn(async move {
        route
            .spawn_authenticated(&req, None, &owned_wait, Timestamp::now())
            .await
    });
    bounded(f.io.entered.acquire()).await.unwrap().forget();
    wait.cancel();
    assert_eq!(bounded(http).await.unwrap().status, 503);
    assert_eq!(f.io.starts.load(Ordering::SeqCst), 1);
    assert_eq!(f.core.snapshots().len(), 1);
    f.io.release.add_permits(1);
    bounded(async {
        while f.core.snapshots().len() != 2 {
            tokio::task::yield_now().await;
        }
    })
    .await;
    let child = f
        .core
        .snapshots()
        .into_iter()
        .find(|s| s.parent_session_id == f.parent.session)
        .unwrap();
    assert_eq!(child.role, "worker");
    assert!(!child.board_path.is_empty());
    assert!(f.core.pending().is_empty());
}
#[tokio::test]
async fn cancelled_confirmation_waiter_leaves_pending_then_real_approval_starts_child() {
    let f = fixture("on", 4).await;
    let route = f.route.clone();
    let req = request(f.parent, "ui");
    let wait = HttpWaitCancellation::default();
    let owned_wait = wait.clone();
    let http = tokio::spawn(async move {
        route
            .spawn_authenticated(&req, None, &owned_wait, Timestamp::now())
            .await
    });
    let pending = bounded(async {
        loop {
            if let Some(p) = f.core.pending().into_iter().next() {
                break p;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    wait.cancel();
    assert_eq!(bounded(http).await.unwrap().status, 503);
    assert_eq!(f.core.pending().len(), 1);
    assert_eq!(f.io.starts.load(Ordering::SeqCst), 0);
    let decision = f
        .core
        .clone()
        .accept_decision(
            SpawnConfirmationResponse {
                confirmation_id: pending.id,
                approved: true,
                ..Default::default()
            },
            VerifiedConfirmationRequest::after_server_authentication(AuthEpoch(0)),
            Timestamp::now(),
        )
        .unwrap();
    let future = decision.run(TaskCancellation::default()).unwrap();
    let task = tokio::spawn(future);
    bounded(f.io.entered.acquire()).await.unwrap().forget();
    f.io.release.add_permits(1);
    assert!(matches!(
        bounded(task).await.unwrap(),
        ConfirmationOutcome::Approved(_)
    ));
    assert!(f.core.pending().is_empty());
    assert_eq!(f.core.snapshots().len(), 2);
}
#[tokio::test]
async fn bound_ui_and_approved_confirmation_reuse_source_human_capacity() {
    let f = fixture("on", 1).await;
    for role in ["first", "second"] {
        let route = f.route.clone();
        let mut req = request(f.parent, "ui");
        req.body = serde_json::to_vec(
            &serde_json::json!({"role":role,"provider":"copilot","same_tree":true,"origin":"ui"}),
        )
        .unwrap();
        let launch = tokio::spawn(async move {
            route
                .spawn_authenticated(
                    &req,
                    Some(VerifiedUiOrigin::after_server_verification(AuthEpoch(0))),
                    &HttpWaitCancellation::default(),
                    Timestamp::now(),
                )
                .await
        });
        bounded(f.io.entered.acquire()).await.unwrap().forget();
        assert!(f.core.pending().is_empty());
        f.io.release.add_permits(1);
        assert_eq!(bounded(launch).await.unwrap().status, 200);
    }
    assert_eq!(f.io.starts.load(Ordering::SeqCst), 2);
    let route = f.route.clone();
    let req = request(f.parent, "");
    let http = tokio::spawn(async move {
        route
            .spawn_authenticated(
                &req,
                None,
                &HttpWaitCancellation::default(),
                Timestamp::now(),
            )
            .await
    });
    let pending = bounded(async {
        loop {
            if let Some(p) = f.core.pending().into_iter().next() {
                break p;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert_eq!(f.io.starts.load(Ordering::SeqCst), 2);
    let decision = f
        .core
        .clone()
        .accept_decision(
            SpawnConfirmationResponse {
                confirmation_id: pending.id,
                approved: true,
                ..Default::default()
            },
            VerifiedConfirmationRequest::after_server_authentication(AuthEpoch(0)),
            Timestamp::now(),
        )
        .unwrap();
    let committed = decision.run(TaskCancellation::default()).unwrap();
    let accepted = tokio::spawn(committed);
    bounded(f.io.entered.acquire()).await.unwrap().forget();
    f.io.release.add_permits(1);
    let outcome = bounded(accepted).await.unwrap();
    assert!(matches!(outcome, ConfirmationOutcome::Approved(_)));
    let response = bounded(http).await.unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(f.io.starts.load(Ordering::SeqCst), 3);
    assert!(f.core.pending().is_empty());
}
#[tokio::test]
async fn duplicate_role_is_refused_before_publishing_a_confirmation() {
    let f = fixture("on", 4).await;
    let route = f.route.clone();
    let req = request(f.parent, "ui");
    let first = tokio::spawn(async move {
        route
            .spawn_authenticated(
                &req,
                Some(VerifiedUiOrigin::after_server_verification(AuthEpoch(0))),
                &HttpWaitCancellation::default(),
                Timestamp::now(),
            )
            .await
    });
    bounded(f.io.entered.acquire()).await.unwrap().forget();
    f.io.release.add_permits(1);
    assert_eq!(bounded(first).await.unwrap().status, 200);
    let response = bounded(f.route.spawn_authenticated(
        &request(f.parent, ""),
        None,
        &HttpWaitCancellation::default(),
        Timestamp::now(),
    ))
    .await;
    assert_eq!(response.status, 409);
    let json: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(json["error"], "duplicate_role_child");
    assert!(f.core.pending().is_empty());
    assert_eq!(f.io.starts.load(Ordering::SeqCst), 1);
    let mut forced = request(f.parent, "");
    forced.body = serde_json::to_vec(
        &serde_json::json!({"role":"worker","provider":"copilot","same_tree":true,"force":true}),
    )
    .unwrap();
    let route = f.route.clone();
    let http = tokio::spawn(async move {
        route
            .spawn_authenticated(
                &forced,
                None,
                &HttpWaitCancellation::default(),
                Timestamp::now(),
            )
            .await
    });
    bounded(async {
        while f.core.pending().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert_eq!(f.io.starts.load(Ordering::SeqCst), 1);
    http.abort();
}
#[tokio::test]
async fn actual_inject_queue_preserves_raw_controls_and_interrupt_order() {
    let f = fixture("off", 4).await;
    let control = crate::hub::child_control::ChildControl::new(
        f.core.clone(),
        f.route.program.clone(),
        f._tasks.handle(),
        Arc::new(|_, _| {}),
    );
    let req = Request {
        method: "POST".into(),
        path: format!("/api/sessions/{}/inject", f.parent.session.0),
        body: serde_json::to_vec(
            &serde_json::json!({"text":"raw\u{1b}[31m\u{7}","press_enter":false,"interrupt":true}),
        )
        .unwrap(),
        ..Default::default()
    };
    assert_eq!(
        control.handle_authenticated(&req, Timestamp::now()).status,
        200
    );
    bounded(async {
        while f.io.sent.lock().unwrap().len() < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert_eq!(
        &*f.io.sent.lock().unwrap(),
        &[vec![0x1b], b"raw\x1b[31m\x07".to_vec()]
    );
    let invalid = Request {
        body: serde_json::to_vec(
            &serde_json::json!({"text":"no","from_session_id":f.parent.session.0}),
        )
        .unwrap(),
        ..req
    };
    assert_eq!(
        control
            .handle_authenticated(&invalid, Timestamp::now())
            .status,
        400
    );
    assert_eq!(f.io.sent.lock().unwrap().len(), 2);
}
#[tokio::test]
async fn audit_n06_inject_types_nothing_into_a_session_that_shows_an_approval_prompt() {
    let f = fixture("off", 4).await;
    let control = crate::hub::child_control::ChildControl::new(
        f.core.clone(),
        f.route.program.clone(),
        f._tasks.handle(),
        Arc::new(|_, _| {}),
    );
    let inject = |body: serde_json::Value| Request {
        method: "POST".into(),
        path: format!("/api/sessions/{}/inject", f.parent.session.0),
        body: serde_json::to_vec(&body).unwrap(),
        ..Default::default()
    };
    let screen =
        "\x1b[2J\x1b[HAllow this command?\r\nDo you want to proceed?\r\n❯ 1. Yes\r\n  2. No"
            .as_bytes()
            .to_vec();
    drop(
        f.core
            .observe_output(
                f.parent,
                OutputChunk {
                    total_pty_bytes: screen.len() as i64,
                    bytes: screen,
                },
                Timestamp::now(),
            )
            .unwrap(),
    );
    assert!(
        f.core
            .details(f.parent.session)
            .unwrap()
            .approval
            .record
            .is_some(),
        "fixture must show a prompt"
    );
    // An instruction with its Enter would answer the prompt; an interrupt's ESC
    // would dismiss it. Neither is what the caller meant.
    for body in [
        serde_json::json!({"text":"continue with the next step","press_enter":true}),
        serde_json::json!({"text":"stop","press_enter":false,"interrupt":true}),
    ] {
        let response = control.handle_authenticated(&inject(body), Timestamp::now());
        assert_eq!(response.status, 409);
        assert!(
            String::from_utf8_lossy(&response.body).contains("session_waiting_approval"),
            "{}",
            String::from_utf8_lossy(&response.body)
        );
    }
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
    assert!(f.io.sent.lock().unwrap().is_empty());
}
#[tokio::test]
async fn autonomous_spawn_without_confirmation_keeps_configured_capacity() {
    let f = fixture("off", 1).await;
    let route = f.route.clone();
    let req = request(f.parent, "ui");
    let first = tokio::spawn(async move {
        route
            .spawn_authenticated(
                &req,
                Some(VerifiedUiOrigin::after_server_verification(AuthEpoch(0))),
                &HttpWaitCancellation::default(),
                Timestamp::now(),
            )
            .await
    });
    bounded(f.io.entered.acquire()).await.unwrap().forget();
    f.io.release.add_permits(1);
    assert_eq!(bounded(first).await.unwrap().status, 200);
    let mut req = request(f.parent, "");
    req.body = serde_json::to_vec(
        &serde_json::json!({"role":"another","provider":"copilot","same_tree":true}),
    )
    .unwrap();
    let response = bounded(f.route.spawn_authenticated(
        &req,
        None,
        &HttpWaitCancellation::default(),
        Timestamp::now(),
    ))
    .await;
    assert_eq!(response.status, 429);
    assert_eq!(f.io.starts.load(Ordering::SeqCst), 1);
    assert!(f.core.pending().is_empty());
}
#[tokio::test]
async fn children_query_includes_terminal_children_and_unknown_parent_is_empty() {
    let f = fixture("off", 4).await;
    let route = f.route.clone();
    let req = request(f.parent, "");
    let http = tokio::spawn(async move {
        route
            .spawn_authenticated(
                &req,
                None,
                &HttpWaitCancellation::default(),
                Timestamp::now(),
            )
            .await
    });
    bounded(f.io.entered.acquire()).await.unwrap().forget();
    f.io.release.add_permits(1);
    assert_eq!(bounded(http).await.unwrap().status, 200);
    let child = f
        .core
        .snapshots()
        .into_iter()
        .find(|s| s.parent_session_id == f.parent.session)
        .unwrap();
    f.core
        .mark_orchestration_child_state(
            f.core.details(child.id).unwrap().binding,
            "timeout",
            Timestamp::now(),
        )
        .unwrap();
    let control = crate::hub::child_control::ChildControl::new(
        f.core.clone(),
        f.route.program.clone(),
        f._tasks.handle(),
        Arc::new(|_, _| {}),
    );
    let req = Request {
        method: "GET".into(),
        path: format!("/api/sessions/{}/children", f.parent.session.0),
        ..Default::default()
    };
    let response = control.handle_authenticated(&req, Timestamp::now());
    assert_eq!(response.status, 200);
    let json: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(json["children"].as_array().unwrap().len(), 1);
    assert_eq!(json["children"][0]["state"], "timeout");
    let unknown = Request {
        path: "/api/sessions/999999/children".into(),
        ..req
    };
    let json: serde_json::Value = serde_json::from_slice(
        &control
            .handle_authenticated(&unknown, Timestamp::now())
            .body,
    )
    .unwrap();
    assert!(json["children"].as_array().unwrap().is_empty());
}

fn registered_registry() -> crate::application::spawn_policy::RegistrySnapshot {
    use crate::proto::provider::{Definition, HeadlessDefinition, LaunchDefinition, Layers};
    let definition = |id: &str, headless: bool| Definition {
        schema_version: 1,
        id: id.into(),
        display_name: "Fixture Agent".into(),
        launch: Some(LaunchDefinition {
            executable: "fixture-agent-bin".into(),
            headless: headless.then(|| HeadlessDefinition {
                args: vec!["-p".into()],
                format: "text".into(),
                prompt_via: "arg".into(),
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    let registry = crate::profile::registry::Registry::build(
        Layers {
            embedded: Some(crate::profile::registry::embedded_definitions().unwrap()),
            user: Some(vec![
                definition("fixture-headless", true),
                definition("fixture-plain", false),
            ]),
            ..Default::default()
        },
        &crate::profile::registry::default_adapters(),
    );
    Arc::new(move || Ok(registry.clone()))
}
fn child_request(parent: SessionBinding, body: serde_json::Value) -> Request {
    let mut request = request(parent, "");
    request.body = serde_json::to_vec(&body).unwrap();
    request
}
#[tokio::test]
async fn unknown_provider_is_refused_with_built_ins_and_registered_ids() {
    let f = fixture_with_registry("off", 4, registered_registry()).await;
    let response = f
        .route
        .spawn_authenticated(
            &child_request(
                f.parent,
                serde_json::json!({"role":"worker","provider":"nope","same_tree":true}),
            ),
            None,
            &HttpWaitCancellation::default(),
            Timestamp::now(),
        )
        .await;
    assert_eq!(response.status, 400);
    let body = String::from_utf8(response.body).unwrap();
    assert!(
        body.contains(
            "valid providers are: claude, codex, copilot, cursor-agent, opencode, grok, command-code, fixture-headless, fixture-plain"
        ),
        "{body}"
    );
    assert_eq!(f.io.starts.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn registered_headless_provider_starts_a_child_without_built_in_flags() {
    let f = fixture_with_registry("off", 4, registered_registry()).await;
    let route = f.route.clone();
    let req = child_request(
        f.parent,
        serde_json::json!({"role":"worker","provider":"fixture-headless","execution_mode":"headless","permission_preset":"full","same_tree":true}),
    );
    let launch = tokio::spawn(async move {
        route
            .spawn_authenticated(
                &req,
                None,
                &HttpWaitCancellation::default(),
                Timestamp::now(),
            )
            .await
    });
    bounded(f.io.entered.acquire()).await.unwrap().forget();
    f.io.release.add_permits(1);
    assert_eq!(bounded(launch).await.unwrap().status, 200);
    let specs = f.io.specs.lock().unwrap();
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].provider, "fixture-headless");
    assert_eq!(specs[0].execution_mode, "headless");
    assert!(specs[0].permission_mode.is_empty());
    assert!(specs[0].sandbox.is_empty());
    assert!(!specs[0].risk_confirmed);
}
#[tokio::test]
async fn registered_provider_without_headless_definition_is_refused_for_headless() {
    let f = fixture_with_registry("off", 4, registered_registry()).await;
    let response = f
        .route
        .spawn_authenticated(
            &child_request(
                f.parent,
                serde_json::json!({"role":"worker","provider":"fixture-plain","execution_mode":"headless","same_tree":true}),
            ),
            None,
            &HttpWaitCancellation::default(),
            Timestamp::now(),
        )
        .await;
    assert_eq!(response.status, 400);
    let body = String::from_utf8(response.body).unwrap();
    assert!(
        body.contains("headless is not supported for this provider"),
        "{body}"
    );
    assert_eq!(f.io.starts.load(Ordering::SeqCst), 0);
}

/// A registry with one registered ACP provider, `fixture-acp`. A built-in id
/// cannot be made an ACP one (an override keeps its shipped connection), so a
/// test that needs an ACP parent registers the parent as `fixture-acp`.
fn acp_registry_build() -> crate::profile::registry::Registry {
    use crate::proto::provider::{Definition, LaunchDefinition, Layers};
    let acp = |id: &str| Definition {
        schema_version: 1,
        id: id.into(),
        display_name: "Fixture ACP".into(),
        connection: "acp".into(),
        launch: Some(LaunchDefinition {
            executable: "fixture-acp-bin".into(),
            ..Default::default()
        }),
        ..Default::default()
    };
    crate::profile::registry::Registry::build(
        Layers {
            embedded: Some(crate::profile::registry::embedded_definitions().unwrap()),
            user: Some(vec![acp("fixture-acp")]),
            ..Default::default()
        },
        &crate::profile::registry::default_adapters(),
    )
}
fn acp_registry() -> crate::application::spawn_policy::RegistrySnapshot {
    let registry = acp_registry_build();
    Arc::new(move || Ok(registry.clone()))
}
#[tokio::test]
async fn an_acp_provider_is_refused_as_a_child() {
    let f = fixture_with_registry("off", 4, acp_registry()).await;
    let response = f
        .route
        .spawn_authenticated(
            &child_request(
                f.parent,
                serde_json::json!({"role":"worker","provider":"fixture-acp","same_tree":true}),
            ),
            None,
            &HttpWaitCancellation::default(),
            Timestamp::now(),
        )
        .await;
    assert_eq!(response.status, 403);
    assert!(
        String::from_utf8(response.body)
            .unwrap()
            .contains("automation_refused")
    );
    assert_eq!(f.io.starts.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn no_child_is_started_under_an_acp_parent() {
    let f = fixture_with_parent("off", 4, acp_registry(), "fixture-acp").await;
    let response = f
        .route
        .spawn_authenticated(
            &child_request(
                f.parent,
                serde_json::json!({"role":"worker","provider":"claude","same_tree":true}),
            ),
            None,
            &HttpWaitCancellation::default(),
            Timestamp::now(),
        )
        .await;
    assert_eq!(response.status, 403);
    assert_eq!(f.io.starts.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn an_acp_session_takes_no_inject_from_another_session() {
    let _guard = crate::profile::declared::install_test_registry(acp_registry_build());
    let f = fixture_with_parent("off", 4, registered_registry(), "fixture-acp").await;
    let control = crate::hub::child_control::ChildControl::new(
        f.core.clone(),
        f.route.program.clone(),
        f._tasks.handle(),
        Arc::new(|_, _| {}),
    );
    let req = Request {
        method: "POST".into(),
        path: format!("/api/sessions/{}/inject", f.parent.session.0),
        body: br#"{"text":"hello","press_enter":true}"#.to_vec(),
        ..Default::default()
    };
    let response = control.handle_authenticated(&req, Timestamp::now());
    assert_eq!(response.status, 403);
}

// ---- Issue #14: API key billing needs a person's spawn confirmation. ----
// Synthetic values only: no key, no key command is run, no provider starts.
const SYNTHETIC_RECIPE: &str = "synthetic-resolver-never-run";
fn save_billing_profile(paths: &RuntimePaths) {
    crate::config::private_io::write_atomic(
        &paths.resource(crate::config::Resource::HeadlessBilling),
        format!(r#"{{"version":1,"profiles":[{{"id":"synthetic","display_name":"Synthetic","provider":"claude","command":"{SYNTHETIC_RECIPE}"}}]}}"#).as_bytes(),
    )
    .unwrap();
}
fn api_request(parent: SessionBinding, origin: &str) -> Request {
    child_request(
        parent,
        serde_json::json!({"role":"worker","provider":"claude","same_tree":true,"execution_mode":"headless","billing_mode":"api-key","billing_profile":"synthetic","max_budget_usd":0.5,"origin":origin}),
    )
}
async fn first_pending(f: &Fixture) -> PendingSpawnConfirmation {
    bounded(async {
        loop {
            if let Some(p) = f.core.pending().into_iter().next() {
                break p;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
}
/// A decision sent with only the Hub token (what a conductor's shell has).
fn token_only() -> VerifiedConfirmationRequest {
    VerifiedConfirmationRequest::after_server_authentication(AuthEpoch(0))
}
/// A decision the router verified as coming from the dashboard.
fn from_dashboard() -> VerifiedConfirmationRequest {
    token_only().with_ui_origin(Some(VerifiedUiOrigin::after_server_verification(
        AuthEpoch(0),
    )))
}
fn assert_human_required(error: &SessionError) {
    let SessionError::ChildLaunch { status, code, .. } = error else {
        panic!("expected a refusal that needs a person: {error:?}");
    };
    assert_eq!(
        (*status, code.as_str()),
        (403, "human_confirmation_required")
    );
}
fn approve(revision: Option<&str>, id: SpawnConfirmationId) -> SpawnConfirmationResponse {
    SpawnConfirmationResponse {
        confirmation_id: id,
        approved: true,
        billing_mode: Some("api-key".into()),
        billing_profile: Some("synthetic".into()),
        billing_revision: revision.map(str::to_owned),
        max_budget_usd: Some(0.5),
        ..Default::default()
    }
}

#[tokio::test]
async fn an_autonomous_api_key_spawn_waits_for_a_person_even_with_confirmation_off() {
    let f = fixture("off", 4).await;
    save_billing_profile(&f.paths);
    let route = f.route.clone();
    let req = api_request(f.parent, "");
    let http = tokio::spawn(async move {
        route
            .spawn_authenticated(
                &req,
                None,
                &HttpWaitCancellation::default(),
                Timestamp::now(),
            )
            .await
    });
    let pending = first_pending(&f).await;
    // The conductor's own request never starts a paid run.
    assert_eq!(f.io.starts.load(Ordering::SeqCst), 0);
    let disclosure = f.route.executor.presentation().unwrap().billing;
    assert_eq!(disclosure.profiles.len(), 1);
    assert_eq!(disclosure.revision.len(), 64);
    assert!(!format!("{disclosure:?}").contains(SYNTHETIC_RECIPE));
    // The conductor's shell holds the Hub token and can read the confirmation
    // ID and revision from the socket. Approving with the token alone, even
    // with the current revision, is refused and the confirmation stays.
    let error = f
        .core
        .clone()
        .accept_decision(
            approve(Some(&disclosure.revision), pending.id.clone()),
            token_only(),
            Timestamp::now(),
        )
        .err()
        .unwrap();
    assert_human_required(&error);
    assert_eq!(f.core.pending().len(), 1);
    assert_eq!(f.io.starts.load(Ordering::SeqCst), 0);
    let proof = from_dashboard;
    // An approval that never saw the billing (no revision) is shown again.
    for revision in [None, Some("0".repeat(64))] {
        let error = f
            .core
            .clone()
            .accept_decision(
                approve(revision.as_deref(), pending.id.clone()),
                proof(),
                Timestamp::now(),
            )
            .err()
            .unwrap();
        let SessionError::BillingProfileChanged { disclosure: fresh } = error.clone() else {
            panic!("expected a refreshed disclosure: {error:?}");
        };
        assert!(fresh.contains("spawn_confirmation_requested"));
        assert!(!fresh.contains(SYNTHETIC_RECIPE));
        assert_eq!(f.core.pending().len(), 1);
        assert_eq!(f.io.starts.load(Ordering::SeqCst), 0);
    }
    // The person approves what the dialog showed: the launch carries it.
    let decision = f
        .core
        .clone()
        .accept_decision(
            approve(Some(&disclosure.revision), pending.id),
            proof(),
            Timestamp::now(),
        )
        .unwrap();
    let task = tokio::spawn(decision.run(TaskCancellation::default()).unwrap());
    bounded(f.io.entered.acquire()).await.unwrap().forget();
    let spec = f.io.specs.lock().unwrap().last().cloned().unwrap();
    let billing = spec.registration_metadata.headless_billing.clone().unwrap();
    assert!(billing.is_api());
    assert_eq!(billing.profile_id, "synthetic");
    assert_eq!(billing.revision, disclosure.revision);
    assert_eq!(billing.max_budget_usd, Some(0.5));
    assert!(spec.subscription_profile_id.is_empty());
    assert_eq!(spec.execution_mode, "headless");
    f.io.release.add_permits(1);
    assert!(matches!(
        bounded(task).await.unwrap(),
        ConfirmationOutcome::Approved(_)
    ));
    assert_eq!(bounded(http).await.unwrap().status, 200);
}

#[tokio::test]
async fn a_direct_ui_api_key_spawn_also_goes_through_the_dialog() {
    let f = fixture("off", 4).await;
    save_billing_profile(&f.paths);
    let route = f.route.clone();
    let req = api_request(f.parent, "ui");
    let http = tokio::spawn(async move {
        route
            .spawn_authenticated(
                &req,
                Some(VerifiedUiOrigin::after_server_verification(AuthEpoch(0))),
                &HttpWaitCancellation::default(),
                Timestamp::now(),
            )
            .await
    });
    let pending = first_pending(&f).await;
    assert_eq!(f.io.starts.load(Ordering::SeqCst), 0);
    // Refusing ends it; nothing starts.
    let decision = f
        .core
        .clone()
        .accept_decision(
            SpawnConfirmationResponse {
                confirmation_id: pending.id,
                approved: false,
                ..Default::default()
            },
            VerifiedConfirmationRequest::after_server_authentication(AuthEpoch(0)),
            Timestamp::now(),
        )
        .unwrap();
    bounded(decision.run(TaskCancellation::default()).unwrap()).await;
    assert_eq!(bounded(http).await.unwrap().status, 403);
    assert_eq!(f.io.starts.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn the_executor_refuses_api_key_billing_without_a_confirmed_human_decision() {
    let f = fixture("off", 4).await;
    save_billing_profile(&f.paths);
    let body: ChildSpawnRequest = serde_json::from_value(serde_json::json!({"role":"worker","provider":"claude","same_tree":true,"execution_mode":"headless","billing_mode":"api-key","billing_profile":"synthetic","max_budget_usd":0.5})).unwrap();
    let autonomous =
        ResolvedChildSpawn::from_request(body.clone(), None, InternalSpawnGrants::default())
            .unwrap();
    let mut ui = body;
    ui.claimed_origin = "ui".into();
    let ui = ResolvedChildSpawn::from_request(
        ui,
        Some(VerifiedUiOrigin::after_server_verification(AuthEpoch(0))),
        InternalSpawnGrants::default(),
    )
    .unwrap();
    // Even a resolved, current billing selection does not stand in for the
    // person's approval of this confirmation.
    let mut resolved = autonomous.clone();
    let selection = crate::config::headless_billing::validate_selection(
        "claude",
        "headless",
        "api-key",
        "synthetic",
        Some(0.5),
        "",
    )
    .unwrap();
    let resolved_billing = crate::config::headless_billing::resolve(&f.paths, selection).unwrap();
    resolved.set_billing(resolved_billing.clone());
    // A confirmation approved with the Hub token alone (as a plan login, the
    // only thing it may approve) that later carries API key billing is still
    // not a dashboard decision: the launch itself refuses it as well.
    let plan: ChildSpawnRequest = serde_json::from_value(serde_json::json!({"role":"worker","provider":"claude","same_tree":true,"execution_mode":"headless"})).unwrap();
    let mut token_approved =
        ResolvedChildSpawn::from_request(plan, None, InternalSpawnGrants::default())
            .unwrap()
            .approve(
                &SpawnConfirmationResponse {
                    confirmation_id: SpawnConfirmationId("sc-synthetic".into()),
                    approved: true,
                    ..Default::default()
                },
                token_only(),
            )
            .unwrap();
    token_approved.request_mut().billing_mode = "api-key".into();
    token_approved.request_mut().billing_profile = "synthetic".into();
    token_approved.request_mut().max_budget_usd = Some(0.5);
    token_approved.set_billing(resolved_billing);
    for body in [autonomous, ui, resolved, token_approved] {
        let result = bounded(f.route.executor.spawn(
            ConfirmedChildRequest {
                original_body: body.clone(),
                parent: f.parent,
                requested_provider: "claude".into(),
                body,
                admission: AdmissionId::default(),
            },
            TaskCancellation::default(),
        ))
        .await;
        let Err(SessionError::ChildLaunch { status, code, .. }) = result else {
            panic!("API key billing must be refused");
        };
        assert_eq!(
            (status, code.as_str()),
            (403, "human_confirmation_required")
        );
    }
    assert_eq!(f.io.starts.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn api_key_billing_for_a_provider_without_support_is_refused_before_any_dialog() {
    let f = fixture("on", 4).await;
    save_billing_profile(&f.paths);
    for provider in ["codex", "copilot", "grok", "command-code"] {
        let req = child_request(
            f.parent,
            serde_json::json!({"role":"worker","provider":provider,"same_tree":true,"execution_mode":"headless","billing_mode":"api-key","billing_profile":"synthetic","max_budget_usd":0.5}),
        );
        let route = &f.route;
        // The caller gets the answer at once: no dialog waits for a person.
        let response = bounded(route.spawn_authenticated(
            &req,
            None,
            &HttpWaitCancellation::default(),
            Timestamp::now(),
        ))
        .await;
        assert_eq!(response.status, 400, "{provider}");
        assert!(f.core.pending().is_empty(), "{provider}");
    }
    assert_eq!(f.io.starts.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_plan_login_confirmation_cannot_be_raised_to_api_key_billing_with_the_token_alone() {
    let f = fixture("on", 4).await;
    save_billing_profile(&f.paths);
    let route = f.route.clone();
    let req = child_request(
        f.parent,
        serde_json::json!({"role":"worker","provider":"claude","same_tree":true,"execution_mode":"headless"}),
    );
    let http = tokio::spawn(async move {
        route
            .spawn_authenticated(
                &req,
                None,
                &HttpWaitCancellation::default(),
                Timestamp::now(),
            )
            .await
    });
    let pending = first_pending(&f).await;
    let revision = f.route.executor.presentation().unwrap().billing.revision;
    // The conductor answers its own plan login confirmation and raises it to
    // API key billing: refused, still pending, nothing started.
    let error = f
        .core
        .clone()
        .accept_decision(
            approve(Some(&revision), pending.id.clone()),
            token_only(),
            Timestamp::now(),
        )
        .err()
        .unwrap();
    assert_human_required(&error);
    assert_eq!(f.core.pending().len(), 1);
    assert_eq!(f.io.starts.load(Ordering::SeqCst), 0);
    // A person picking API key billing in the dashboard launches with it.
    let decision = f
        .core
        .clone()
        .accept_decision(
            approve(Some(&revision), pending.id),
            from_dashboard(),
            Timestamp::now(),
        )
        .unwrap();
    let task = tokio::spawn(decision.run(TaskCancellation::default()).unwrap());
    bounded(f.io.entered.acquire()).await.unwrap().forget();
    let spec = f.io.specs.lock().unwrap().last().cloned().unwrap();
    let billing = spec.registration_metadata.headless_billing.clone().unwrap();
    assert!(billing.is_api());
    assert_eq!(billing.revision, revision);
    f.io.release.add_permits(1);
    assert!(matches!(
        bounded(task).await.unwrap(),
        ConfirmationOutcome::Approved(_)
    ));
    assert_eq!(bounded(http).await.unwrap().status, 200);
}
