//! Independent synthetic coordinator acceptance. No native process, Hub, or
//! provider is started; registration and root ownership use separate barriers.
use many_ai_cli::{
    application::{
        iris_execution_dispatcher::{DispatcherConfig, OwnedDispatcher},
        iris_execution_launch::OwnedLaunchPort,
    },
    hub::{
        iris_execution_journal::{Binding, Journal},
        iris_execution_owner::{OwnerStore, OwnerTicket},
        iris_execution_runtime::BridgeError,
    },
    proto::core::{
        CoreFuture, HttpWaitCancellation, LiveSessionId, SessionBinding, SessionIncarnation,
        SpawnWaitOutcome, WrappedSpawnSpec, WrapperConnectionId,
    },
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::Semaphore;
fn ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
#[derive(Clone, Copy)]
enum Mode {
    Barrier,
    Timeout,
    Crash,
}
struct Pending {
    ticket: OwnerTicket,
    session: SessionBinding,
}
struct FakePort {
    owners: Arc<Mutex<OwnerStore>>,
    mode: Mode,
    registration: Semaphore,
    launches: AtomicUsize,
    binds: AtomicUsize,
    stop_calls: AtomicUsize,
    stop_effects: AtomicUsize,
    pending: Mutex<Vec<Pending>>,
    bound: Mutex<BTreeSet<String>>,
    stopped: Mutex<BTreeSet<(String, String)>>,
}
impl FakePort {
    fn new(owners: Arc<Mutex<OwnerStore>>, mode: Mode) -> Self {
        Self {
            owners,
            mode,
            registration: Semaphore::new(0),
            launches: AtomicUsize::new(0),
            binds: AtomicUsize::new(0),
            stop_calls: AtomicUsize::new(0),
            stop_effects: AtomicUsize::new(0),
            pending: Mutex::new(vec![]),
            bound: Mutex::new(BTreeSet::new()),
            stopped: Mutex::new(BTreeSet::new()),
        }
    }
    fn root(&self, attempt: &str) {
        let pending = self.pending.lock().unwrap();
        let p = pending
            .iter()
            .find(|p| p.ticket.binding.attempt_id == attempt)
            .unwrap();
        self.owners
            .lock()
            .unwrap()
            .bind_root(&p.ticket, 7000 + p.session.session.0 as u32, ms())
            .unwrap();
    }
}
impl OwnedLaunchPort for FakePort {
    fn launch_registered<'a>(
        &'a self,
        spec: WrappedSpawnSpec,
        _: Duration,
        _: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome> {
        Box::pin(async move {
            let ticket: OwnerTicket =
                serde_json::from_str(&spec.registration_metadata.iris_owner_context).unwrap();
            assert_eq!(spec.provider, "synthetic");
            assert_eq!(spec.subscription_profile_id, "synthetic");
            assert_eq!(
                spec.initial_prompt,
                "fixed synthetic dispatcher instruction"
            );
            assert!(spec.registration_proof.is_none() && spec.spawn_attempt.is_none());
            assert!(!spec.risk_confirmed);
            let index = self.launches.fetch_add(1, Ordering::SeqCst) + 1;
            let session = SessionBinding {
                session: LiveSessionId(10 + index as i64),
                incarnation: SessionIncarnation(1),
                wrapper: WrapperConnectionId(20 + index as u64),
            };
            self.owners
                .lock()
                .unwrap()
                .claim_spawn(&ticket, session.session.0, ms())
                .unwrap();
            self.pending
                .lock()
                .unwrap()
                .push(Pending { ticket, session });
            match self.mode {
                Mode::Timeout => SpawnWaitOutcome::TimedOut,
                Mode::Crash => {
                    self.registration.acquire().await.unwrap().forget();
                    panic!("synthetic launch worker crash")
                }
                Mode::Barrier => {
                    self.registration.acquire().await.unwrap().forget();
                    SpawnWaitOutcome::Registered(session)
                }
            }
        })
    }
    fn is_current(&self, binding: SessionBinding) -> bool {
        self.pending
            .lock()
            .unwrap()
            .iter()
            .any(|p| p.session == binding)
    }
    fn bind_owner(
        &self,
        ticket: &OwnerTicket,
        binding: SessionBinding,
    ) -> Result<Value, BridgeError> {
        let record = self
            .owners
            .lock()
            .unwrap()
            .query(&ticket.binding)
            .map_err(|_| BridgeError::StateUnavailable)?;
        if record["ticket"] != json!(ticket)
            || record["claimed_session"] != binding.session.0
            || record["root_pid"].as_u64().is_none_or(|pid| pid == 0)
            || !self.is_current(binding)
        {
            return Err(BridgeError::OwnershipConflict);
        }
        self.binds.fetch_add(1, Ordering::SeqCst);
        self.bound
            .lock()
            .unwrap()
            .insert(ticket.binding.attempt_id.clone());
        Ok(json!({"bound":true,"owned_execution_ceased":false}))
    }
    fn stop_owner<'a>(
        &'a self,
        ticket: &'a OwnerTicket,
        id: &'a str,
    ) -> CoreFuture<'a, Result<Value, BridgeError>> {
        Box::pin(async move {
            self.stop_calls.fetch_add(1, Ordering::SeqCst);
            if !self
                .bound
                .lock()
                .unwrap()
                .contains(&ticket.binding.attempt_id)
            {
                return Err(BridgeError::UnknownAttempt);
            }
            let record = self
                .owners
                .lock()
                .unwrap()
                .query(&ticket.binding)
                .map_err(|_| BridgeError::StateUnavailable)?;
            if record["ticket"] != json!(ticket) {
                return Err(BridgeError::OwnershipConflict);
            }
            if self
                .stopped
                .lock()
                .unwrap()
                .insert((ticket.binding.attempt_id.clone(), id.into()))
            {
                self.stop_effects.fetch_add(1, Ordering::SeqCst);
            }
            Ok(
                json!({"stop_request_id":id,"execution_state":"stop_unconfirmed","owned_execution_ceased":false}),
            )
        })
    }
}
struct Fixture {
    root: tempfile::TempDir,
    bindings: Vec<Binding>,
    manifest: Value,
    journal: Arc<Mutex<Journal>>,
    owners: Arc<Mutex<OwnerStore>>,
    port: Arc<FakePort>,
}
impl Fixture {
    fn new(mode: Mode) -> Self {
        let root = tempfile::tempdir().unwrap();
        let path = root.path();
        std::fs::create_dir(path.join("workspace")).unwrap();
        std::fs::create_dir(path.join("iris-execution-artifacts")).unwrap();
        std::fs::write(
            path.join("iris-execution-artifacts/instruction.txt"),
            b"fixed synthetic dispatcher instruction",
        )
        .unwrap();
        let bindings = (1..=2)
            .map(|i| Binding {
                principal_id: "iris".into(),
                workspace_id: "workspace".into(),
                plan_id: "plan".into(),
                plan_revision: 1,
                task_id: format!("task-{i}"),
                instruction_revision: 1,
                instruction_hash: hash(b"fixed synthetic dispatcher instruction"),
                approval_id: "approval".into(),
                approval_revision: 1,
                scope_hash: hash(b"scope"),
                execution_host_id: "host".into(),
                executor_instance_id: "boot".into(),
                attempt_id: format!("attempt-{i}"),
                fence: 1,
                provider: "synthetic".into(),
                profile_id: "synthetic".into(),
            })
            .collect::<Vec<_>>();
        let expires = ms() / 1000 + 3600;
        let entries=bindings.iter().map(|b|json!({"binding":b,"expires_at":expires,"revoked":false,"approved_cwd":"workspace","instruction_path":"instruction.txt","result_path":format!("{}-result.json",b.task_id),"max_artifact_bytes":65536,"effect_scope":["none"],"allow_remote_jobs":false})).collect::<Vec<_>>();
        let manifest = json!({"schema_version":1,"manifest_revision":1,"approval_state":"approved","grants":entries});
        std::fs::write(
            path.join("iris-execution-approved-grants.json"),
            manifest.to_string(),
        )
        .unwrap();
        let journal = Arc::new(Mutex::new(
            Journal::open(&path.join("iris-execution/journal.sqlite3"), "host", "boot").unwrap(),
        ));
        let owners = Arc::new(Mutex::new(
            OwnerStore::open(&path.join("iris-execution/owner.sqlite3")).unwrap(),
        ));
        let port = Arc::new(FakePort::new(owners.clone(), mode));
        Self {
            root,
            bindings,
            manifest,
            journal,
            owners,
            port,
        }
    }
    fn dispatcher(&self, root_timeout: Duration) -> OwnedDispatcher {
        OwnedDispatcher::new(
            self.journal.clone(),
            self.owners.clone(),
            PathBuf::from(self.root.path()),
            self.port.clone(),
            DispatcherConfig {
                queue_capacity: 4,
                registration_timeout: Duration::from_millis(300),
                root_timeout,
                poll_interval: Duration::from_millis(5),
                stop_timeout: Duration::from_millis(100),
            },
        )
        .unwrap()
    }
    fn write(&self, value: &Value) {
        std::fs::write(
            self.root.path().join("iris-execution-approved-grants.json"),
            value.to_string(),
        )
        .unwrap();
    }
}
async fn until(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("independent dispatcher barrier deadline");
}
#[tokio::test]
async fn duplicate_receipt_and_registration_ack_cannot_duplicate_or_prematurely_run() {
    let f = Fixture::new(Mode::Barrier);
    let b = &f.bindings[0];
    let mut d = f.dispatcher(Duration::from_secs(1));
    let first = d.submit("start-1", b).unwrap();
    until(|| f.port.launches.load(Ordering::SeqCst) == 1).await;
    assert_eq!(d.submit("start-1", b).unwrap(), first);
    assert!(d.submit("different-start", b).is_err());
    f.port.registration.add_permits(1);
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(d.query(b).unwrap()["execution_state"], "accepted");
    assert_eq!(f.port.binds.load(Ordering::SeqCst), 0);
    f.port.root(&b.attempt_id);
    until(|| d.query(b).unwrap()["execution_state"] == "running").await;
    assert_eq!(f.port.launches.load(Ordering::SeqCst), 1);
    assert_eq!(f.port.binds.load(Ordering::SeqCst), 1);
    d.shutdown().await.unwrap();
}

#[tokio::test]
async fn full_queue_preserves_replay_and_shutdown_never_accepts_an_unqueued_request() {
    let f = Fixture::new(Mode::Barrier);
    let mut d = OwnedDispatcher::new(
        f.journal.clone(),
        f.owners.clone(),
        f.root.path().to_path_buf(),
        f.port.clone(),
        DispatcherConfig {
            queue_capacity: 1,
            registration_timeout: Duration::from_secs(3),
            root_timeout: Duration::from_secs(1),
            poll_interval: Duration::from_millis(5),
            stop_timeout: Duration::from_millis(100),
        },
    )
    .unwrap();
    let first = d.submit("capacity-first", &f.bindings[0]).unwrap();
    until(|| f.port.launches.load(Ordering::SeqCst) == 1).await;
    let second = d.submit("capacity-second", &f.bindings[1]).unwrap();
    assert_eq!(d.submit("capacity-first", &f.bindings[0]).unwrap(), first);
    assert_eq!(d.submit("capacity-second", &f.bindings[1]).unwrap(), second);
    assert_eq!(
        d.submit("capacity-unaccepted", &f.bindings[0])
            .unwrap_err()
            .code(),
        "DISPATCH_QUEUE_FULL"
    );
    assert_eq!(
        f.journal
            .lock()
            .unwrap()
            .query_request("iris", "workspace", "capacity-unaccepted")
            .unwrap_err()
            .code,
        "NOT_FOUND"
    );
    tokio::time::timeout(Duration::from_secs(5), d.shutdown())
        .await
        .expect("shutdown must not wait for a second watch notification")
        .unwrap();
    for binding in &f.bindings {
        assert_eq!(d.query(binding).unwrap()["execution_state"], "unknown");
    }
    assert_eq!(d.submit("capacity-first", &f.bindings[0]).unwrap(), first);
    assert_eq!(
        d.submit("capacity-after-shutdown", &f.bindings[0])
            .unwrap_err()
            .code(),
        "DISPATCH_WORKER_STOPPED"
    );
    assert_eq!(f.port.launches.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn stop_between_registration_and_root_is_not_overwritten_by_running() {
    let f = Fixture::new(Mode::Barrier);
    let b = &f.bindings[0];
    let mut d = f.dispatcher(Duration::from_secs(1));
    d.submit("start-1", b).unwrap();
    until(|| f.port.launches.load(Ordering::SeqCst) == 1).await;
    f.port.registration.add_permits(1);
    tokio::time::sleep(Duration::from_millis(20)).await;
    d.request_stop("stop-before-root", b).await.unwrap();
    assert_eq!(d.query(b).unwrap()["execution_state"], "stop_requested");
    assert_eq!(f.port.stop_effects.load(Ordering::SeqCst), 0);
    f.port.root(&b.attempt_id);
    until(|| f.port.stop_effects.load(Ordering::SeqCst) == 1).await;
    let state = d.query(b).unwrap();
    assert_ne!(state["execution_state"], "running");
    assert_eq!(state["stop_request_id"], "stop-before-root");
    assert_eq!(
        f.port
            .stopped
            .lock()
            .unwrap()
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        vec![(b.attempt_id.clone(), "stop-before-root".into())]
    );
    let calls_before_replay = f.port.stop_calls.load(Ordering::SeqCst);
    d.request_stop("stop-before-root", b).await.unwrap();
    assert_eq!(f.port.stop_effects.load(Ordering::SeqCst), 1);
    assert_eq!(
        f.port.stop_calls.load(Ordering::SeqCst),
        calls_before_replay,
        "replay must not even enter the effect port"
    );
    d.shutdown().await.unwrap();
}
#[tokio::test]
async fn timeout_and_worker_crash_preserve_unknown_without_relaunch() {
    for mode in [Mode::Timeout, Mode::Crash, Mode::Barrier] {
        let f = Fixture::new(mode);
        let b = &f.bindings[0];
        let mut d = f.dispatcher(Duration::from_millis(80));
        d.submit("start-1", b).unwrap();
        until(|| f.port.launches.load(Ordering::SeqCst) == 1).await;
        if matches!(mode, Mode::Barrier | Mode::Crash) {
            f.port.registration.add_permits(1);
        }
        until(|| d.query(b).unwrap()["execution_state"] == "unknown").await;
        let _ = d.submit("start-1", b);
        assert!(d.submit("new-request", b).is_err());
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(f.port.launches.load(Ordering::SeqCst), 1);
        assert_eq!(f.port.stop_effects.load(Ordering::SeqCst), 0);
        let _ = d.shutdown().await;
    }
}

#[tokio::test]
async fn worker_crash_marks_already_accepted_queued_work_unknown_without_launch() {
    let f = Fixture::new(Mode::Crash);
    let first = &f.bindings[0];
    let second = &f.bindings[1];
    let mut d = f.dispatcher(Duration::from_secs(1));
    d.submit("first", first).unwrap();
    until(|| f.port.launches.load(Ordering::SeqCst) == 1).await;
    let second_receipt = d.submit("second", second).unwrap();
    f.port.registration.add_permits(1);
    until(|| d.query(first).unwrap()["execution_state"] == "unknown").await;
    until(|| d.query(second).unwrap()["execution_state"] == "unknown").await;
    assert_eq!(d.submit("second", second).unwrap(), second_receipt);
    assert_eq!(f.port.launches.load(Ordering::SeqCst), 1);
    assert!(d.submit("second-new-request", second).is_err());
    let _ = d.shutdown().await;
}
#[tokio::test]
async fn queued_revocation_or_newer_approval_prevents_the_second_launch() {
    for revoked in [true, false] {
        let f = Fixture::new(Mode::Barrier);
        let first = &f.bindings[0];
        let second = &f.bindings[1];
        let mut d = f.dispatcher(Duration::from_secs(1));
        d.submit("first", first).unwrap();
        until(|| f.port.launches.load(Ordering::SeqCst) == 1).await;
        d.submit("second", second).unwrap();
        let mut changed = f.manifest.clone();
        changed["manifest_revision"] = json!(2);
        changed["grants"][1]["binding"]["approval_revision"] = json!(2);
        changed["grants"][1]["revoked"] = json!(revoked);
        f.write(&changed);
        f.port.root(&first.attempt_id);
        f.port.registration.add_permits(1);
        until(|| d.query(second).unwrap()["execution_state"] == "unknown").await;
        assert_eq!(
            f.port.launches.load(Ordering::SeqCst),
            1,
            "queued stale approval launched"
        );
        assert_eq!(f.port.pending.lock().unwrap().len(), 1);
        d.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn actual_core_launch_port_requires_core_to_issue_registration_proof_and_lease() {
    use many_ai_cli::{
        application::iris_execution_launch::{CoreLaunchPort, prepare_spec},
        config::RuntimePaths,
        hub::iris_execution_grants::GrantLoader,
        proto::{
            Message,
            core::{
                CoreEffectFailure, CoreEffectSink, CoreEffects, SessionError,
                WrappedSessionSpawner, WrapperTransport,
            },
        },
        terminal::{
            events::CoreEventBus,
            journal::{JournalOptions, SessionJournal},
            session::{EngineOptions, SessionEngine},
        },
    };
    struct Spy {
        launches: AtomicUsize,
        expected_owner: String,
    }
    impl WrappedSessionSpawner for Spy {
        fn spawn_and_wait<'a>(
            &'a self,
            spec: WrappedSpawnSpec,
            _: Duration,
            _: &'a HttpWaitCancellation,
        ) -> CoreFuture<'a, SpawnWaitOutcome> {
            Box::pin(async move {
                assert!(
                    spec.registration_proof
                        .as_ref()
                        .is_some_and(|proof| !proof.as_header_value().is_empty()),
                    "core must issue its own one-use registration proof before the spawner"
                );
                assert!(
                    spec.spawn_attempt.is_some(),
                    "core must acquire the provider admission lease"
                );
                assert_eq!(
                    spec.registration_metadata.iris_owner_context,
                    self.expected_owner
                );
                self.launches.fetch_add(1, Ordering::SeqCst);
                SpawnWaitOutcome::Failed("synthetic-spawner-sentinel".into())
            })
        }
    }
    impl WrapperTransport for Spy {
        fn send<'a>(
            &'a self,
            _: SessionBinding,
            _: Message,
        ) -> CoreFuture<'a, Result<(), SessionError>> {
            Box::pin(async { panic!("forwarding test must never contact a wrapper") })
        }
    }
    impl CoreEffectSink for Spy {
        fn apply<'a>(&'a self, _: CoreEffects) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
            Box::pin(async { panic!("forwarding test must never perform an external effect") })
        }
    }
    let f = Fixture::new(Mode::Barrier);
    let mut loader = GrantLoader::open(f.root.path()).unwrap();
    let grant = loader
        .load_approved()
        .unwrap()
        .into_iter()
        .find(|grant| grant.grant.binding == f.bindings[0])
        .unwrap();
    let ticket = OwnerTicket {
        binding: f.bindings[0].clone(),
        owner_id: "forward-owner".into(),
        owner_epoch: 1,
    };
    let spec = prepare_spec(&grant, &ticket).unwrap();
    assert!(spec.registration_proof.is_none() && spec.spawn_attempt.is_none());
    let spy = Arc::new(Spy {
        launches: AtomicUsize::new(0),
        expected_owner: serde_json::to_string(&ticket).unwrap(),
    });
    let runtime = f.root.path().join("core-runtime");
    std::fs::create_dir(&runtime).unwrap();
    let paths = RuntimePaths::trial(&runtime, 49467, &f.root.path().join("installed")).unwrap();
    let core = Arc::new(SessionEngine::new(
        EngineOptions::default(),
        Arc::new(SessionJournal::new(paths, None, JournalOptions::default())),
        spy.clone(),
        spy.clone(),
        spy.clone(),
        CoreEventBus::new(16).unwrap(),
    ));
    let port = CoreLaunchPort::new(core, spy.clone(), f.owners.clone());
    let result = port
        .launch_registered(
            spec,
            Duration::from_millis(100),
            &HttpWaitCancellation::default(),
        )
        .await;
    assert_eq!(
        result,
        SpawnWaitOutcome::Failed("synthetic-spawner-sentinel".into())
    );
    assert_eq!(spy.launches.load(Ordering::SeqCst), 1);
    assert_eq!(f.port.launches.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn shutdown_deadline_survives_journal_mutex_and_external_sqlite_writer_contention() {
    use std::sync::mpsc;
    // Always release and join even if an assertion fails. The holder also has
    // its own finite fallback, so the old blocking implementation cannot hang.
    struct Holder {
        release: Option<mpsc::Sender<()>>,
        thread: Option<std::thread::JoinHandle<()>>,
    }
    impl Holder {
        fn finish(&mut self) {
            if let Some(tx) = self.release.take() {
                let _ = tx.send(());
            }
            if let Some(thread) = self.thread.take() {
                thread.join().expect("synthetic storage holder panicked");
            }
        }
    }
    impl Drop for Holder {
        fn drop(&mut self) {
            if let Some(tx) = self.release.take() {
                let _ = tx.send(());
            }
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }
    for sqlite_writer in [false, true] {
        let f = Fixture::new(Mode::Barrier);
        let first = &f.bindings[0];
        let second = &f.bindings[1];
        let mut d = f.dispatcher(Duration::from_secs(1));
        let first_receipt = d.submit("contention-first", first).unwrap();
        until(|| f.port.launches.load(Ordering::SeqCst) == 1).await;
        let second_receipt = d.submit("contention-second", second).unwrap();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let journal = f.journal.clone();
        let db = f.root.path().join("iris-execution/journal.sqlite3");
        let thread = std::thread::spawn(move || {
            if sqlite_writer {
                let db = rusqlite::Connection::open(db).unwrap();
                db.busy_timeout(Duration::from_secs(1)).unwrap();
                db.execute_batch("BEGIN IMMEDIATE").unwrap();
                ready_tx.send(()).unwrap();
                let _ = release_rx.recv_timeout(Duration::from_secs(2));
                db.execute_batch("ROLLBACK").unwrap();
            } else {
                let _held = journal.lock().unwrap();
                ready_tx.send(()).unwrap();
                let _ = release_rx.recv_timeout(Duration::from_secs(2));
            }
        });
        let mut holder = Holder {
            release: Some(release_tx),
            thread: Some(thread),
        };
        ready_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("synthetic contention barrier");
        let started = std::time::Instant::now();
        let result = d.shutdown().await;
        let elapsed = started.elapsed();
        // Release before any assertion or query: old source reaches here only
        // after the holder's 2-second fallback, which the elapsed check catches.
        holder.finish();
        println!(
            "{}",
            json!({"contention":if sqlite_writer {"sqlite-writer"} else {"journal-mutex"},"configured_shutdown_ms":100,"elapsed_ms":elapsed.as_millis(),"pending_cleanup_reported":result.is_err()})
        );
        assert!(
            result.is_err(),
            "shutdown must report pending cleanup while storage is unavailable"
        );
        assert!(
            elapsed < Duration::from_millis(500),
            "current-thread shutdown blocked for {elapsed:?}, sqlite_writer={sqlite_writer}"
        );
        until(|| {
            d.query(first).unwrap()["execution_state"] == "unknown"
                && d.query(second).unwrap()["execution_state"] == "unknown"
        })
        .await;
        assert_eq!(d.submit("contention-first", first).unwrap(), first_receipt);
        assert_eq!(
            d.submit("contention-second", second).unwrap(),
            second_receipt
        );
        assert_eq!(f.port.launches.load(Ordering::SeqCst), 1);
        assert_eq!(f.port.stop_effects.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn legacy_v1_dispatcher_constructor_rejects_without_creating_start_intent() {
    let f = Fixture::new(Mode::Barrier);
    let path = f.root.path().join("iris-execution/journal.sqlite3");
    let db = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM audit", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM requests", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    db.execute_batch("DROP TABLE dispatch_claims; PRAGMA user_version=1;")
        .unwrap();
    assert!(!f.journal.lock().unwrap().supports_owned_dispatch().unwrap());
    let rejected = OwnedDispatcher::new(
        f.journal.clone(),
        f.owners.clone(),
        f.root.path().to_path_buf(),
        f.port.clone(),
        DispatcherConfig::default(),
    );
    assert!(rejected.is_err());
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM requests", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        f.journal
            .lock()
            .unwrap()
            .query_request("iris", "workspace", "must-not-exist")
            .unwrap_err()
            .code,
        "NOT_FOUND"
    );
    assert_eq!(f.port.launches.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn restored_journal_keeps_old_receipts_without_restoring_dispatch_authority() {
    use many_ai_cli::hub::iris_execution_journal::Grant;
    let f = Fixture::new(Mode::Barrier);
    let binding = &f.bindings[0];
    let path = f.root.path().join("restored-journal.sqlite3");
    let before = {
        let mut journal = Journal::open(&path, "host", "boot").unwrap();
        journal
            .register_grant(Grant {
                binding: binding.clone(),
                expires_at: (ms() / 1000 + 3600) as i64,
                revoked: false,
            })
            .unwrap();
        journal
            .start("restored-start", binding, (ms() / 1000) as i64)
            .unwrap();
        journal
            .stop("restored-stop", binding, (ms() / 1000) as i64)
            .unwrap();
        journal
            .query_request("iris", "workspace", "restored-stop")
            .unwrap()["request_receipt"]
            .clone()
    };
    let journal = Arc::new(Mutex::new(
        Journal::open_restored(&path, "host", "restored-boot").unwrap(),
    ));
    assert!(!journal.lock().unwrap().supports_owned_dispatch().unwrap());
    assert!(
        OwnedDispatcher::new(
            journal.clone(),
            f.owners.clone(),
            f.root.path().to_path_buf(),
            f.port.clone(),
            DispatcherConfig::default()
        )
        .is_err()
    );
    let after = journal
        .lock()
        .unwrap()
        .query_request("iris", "workspace", "restored-stop")
        .unwrap();
    assert_eq!(after["request_receipt"], before);
    assert_eq!(after["current"]["restore_hold"], true);
    assert_eq!(after["current"]["execution_state"], "unknown");
    assert_eq!(after["current"]["stop_request_id"], "restored-stop");
    assert_eq!(f.port.launches.load(Ordering::SeqCst), 0);
    assert_eq!(f.port.stop_calls.load(Ordering::SeqCst), 0);
}
