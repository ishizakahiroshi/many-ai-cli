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
    _root: tempfile::TempDir,
    _tasks: HubTaskOwner,
    core: Arc<SessionEngine>,
    route: Arc<ChildHttp>,
    io: Arc<Io>,
    parent: SessionBinding,
}
async fn fixture(confirm: &str, limit: i64) -> Fixture {
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
                    provider: "copilot".into(),
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
        registry: Arc::new(|| Err(std::io::Error::other("synthetic fixture forbids providers"))),
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
