use super::*;
use crate::proto::time::UNIX_EPOCH;
use crate::{
    config::RuntimePaths,
    files::safe_fs::Dir,
    proto::Message,
    terminal::{
        events::CoreEventBus,
        journal::{JournalOptions, SessionJournal},
        session::{EngineOptions, SessionEngine},
    },
};
use std::sync::{
    Weak,
    atomic::{AtomicUsize, Ordering},
};
struct SyntheticTransport;
impl WrapperTransport for SyntheticTransport {
    fn send<'a>(
        &'a self,
        _: SessionBinding,
        _: Message,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async {
            Err(SessionError::InvalidRequest(
                "synthetic routine fixture has no PTY input".into(),
            ))
        })
    }
}
struct SyntheticEffects;
impl CoreEffectSink for SyntheticEffects {
    fn apply<'a>(&'a self, _: CoreEffects) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
        Box::pin(async { Ok(()) })
    }
}
struct SyntheticSpawner {
    core: Mutex<Option<Weak<dyn SessionCore>>>,
    store: Arc<RoutineStore>,
    calls: AtomicUsize,
    registered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
impl WrappedSessionSpawner for SyntheticSpawner {
    fn spawn_and_wait<'a>(
        &'a self,
        spec: WrappedSpawnSpec,
        _: Duration,
        _: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome> {
        Box::pin(async move {
            assert!(
                self.store
                    .runs("")
                    .unwrap()
                    .iter()
                    .any(|r| r.session_label == spec.label && r.status == "starting")
            );
            let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            let core = self
                .core
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .upgrade()
                .unwrap();
            let proof = spec
                .registration_proof
                .as_ref()
                .unwrap()
                .as_header_value()
                .to_owned();
            let registration = core
                .register(
                    RegisterRequest {
                        message: Message {
                            r#type: "register".into(),
                            provider: spec.provider,

                            // Synthetic positive PID for the internal startup proof.
                            pid: 71,
                            cwd: spec.cwd.to_string_lossy().into(),
                            label: spec.label,
                            cols: 120,
                            rows: 30,
                            ..Default::default()
                        },
                        spawn_proof: Some(proof),
                    },
                    WrapperConnectionId(call as u64),
                    clock(),
                )
                .await
                .unwrap();
            self.registered.notify_one();
            self.release.notified().await;
            SpawnWaitOutcome::Registered(registration.binding)
        })
    }
}
struct SyntheticPreparation {
    prompts: Mutex<Vec<String>>,
}
impl RoutineLaunchPreparation for SyntheticPreparation {
    fn prepare<'a>(
        &'a self,
        run: &'a Run,
        prompt: String,
        cancellation: TaskCancellation,
    ) -> CoreFuture<'a, Result<WrappedSpawnSpec, Error>> {
        self.prompts.lock().unwrap().push(prompt.clone());
        Box::pin(async move {
            Ok(WrappedSpawnSpec {
                registration_metadata: SpawnRegistrationMetadata::default(),
                spawn_attempt: None,
                registration_proof: None,
                provider: run.provider.clone(),
                cwd: run.cwd.clone().into(),
                model: run.model.clone(),
                model_selection: String::new(),
                risk_confirmed: false,
                label: run.session_label.clone(),
                permission_mode: String::new(),
                sandbox: String::new(),
                ask_for_approval: String::new(),
                route: String::new(),
                utf8_session: false,
                effort: String::new(),
                execution_mode: String::new(),
                permission_preset: String::new(),
                initial_prompt: prompt,
                subscription_profile_id: String::new(),
                subscription_login: false,
                usage_probe: false,
                grants: InternalSpawnGrants::default(),
                cancellation,
            })
        })
    }
    fn finish(&self, _: &Run, _: &SpawnWaitOutcome) {}
}
fn clock() -> Timestamp {
    UNIX_EPOCH + Duration::new(1_700_000_000, 123_456_789)
}
struct Fixture {
    _root: tempfile::TempDir,
    runner: Arc<RoutineRunner>,
    spawner: Arc<SyntheticSpawner>,
    preparation: Arc<SyntheticPreparation>,
    id: String,
}
fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let trial = root.path().join("trial");
    std::fs::create_dir(&trial).unwrap();
    let cwd = root.path().join("project");
    std::fs::create_dir(&cwd).unwrap();
    let store = Arc::new(RoutineStore::open(Arc::new(Dir::open(&trial).unwrap())));
    let paths = RuntimePaths::trial(&trial, 48891, &root.path().join("installed")).unwrap();
    let journal = Arc::new(SessionJournal::new(paths, None, JournalOptions::default()));
    let bus = CoreEventBus::new(128).unwrap();
    let spawner = Arc::new(SyntheticSpawner {
        core: Mutex::new(None),
        store: store.clone(),
        calls: AtomicUsize::new(0),
        registered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    let core: Arc<dyn SessionCore> = Arc::new(SessionEngine::new(
        EngineOptions {
            hub_instance: "synthetic-routine-hub".into(),
            ..Default::default()
        },
        journal,
        Arc::new(SyntheticTransport),
        Arc::new(SyntheticEffects),
        spawner.clone(),
        bus,
    ));
    *spawner.core.lock().unwrap() = Some(Arc::downgrade(&core));
    let def = store
        .save(
            None,
            super::super::model::Definition {
                name: "Review".into(),
                cwd: cwd.to_string_lossy().into(),
                provider: "codex".into(),
                prompt: "Inspect synthetic changes.".into(),
                ..Default::default()
            },
            root.path(),
            clock(),
        )
        .unwrap();
    let preparation = Arc::new(SyntheticPreparation {
        prompts: Mutex::new(vec![]),
    });
    let mut runner = RoutineRunner::new(
        store,
        core,
        root.path().into(),
        "synthetic-routine-hub".into(),
        TaskCancellation::default(),
        Arc::new(|_, error| panic!("unexpected warning: {error:?}")),
    )
    .with_preparation(preparation.clone());
    runner.now = Arc::new(clock);
    Fixture {
        _root: root,
        runner: Arc::new(runner),
        spawner,
        preparation,
        id: def.id,
    }
}
async fn registered(f: &Fixture) {
    tokio::time::timeout(Duration::from_secs(3), f.spawner.registered.notified())
        .await
        .unwrap();
}
#[tokio::test]
async fn real_core_spawn_admission_is_single_and_completion_wins_launch_wait() {
    let f = fixture();
    let run = f
        .runner
        .start(&f.id, "click", "manual", clock())
        .unwrap()
        .run;
    for _ in 0..20 {
        assert!(
            f.runner
                .start(&f.id, "click", "manual", clock())
                .unwrap()
                .existing
        );
    }
    registered(&f).await;
    assert_eq!(f.spawner.calls.load(Ordering::SeqCst), 1);
    let session = f.runner.core.snapshots().pop().unwrap();
    assert_eq!(session.launch_label, run.session_label);
    assert!(f.runner.core.begin_provider_update("codex").is_err());
    let rename = f
        .runner
        .core
        .update_card_meta(
            session.id,
            SessionCardMeta {
                label: "Renamed display card".into(),
                ..Default::default()
            },
        )
        .unwrap();
    SyntheticEffects.apply(rename).await.unwrap();
    f.runner.refresh(clock()).unwrap();
    assert_eq!(
        f.runner.store.run(&run.id).unwrap().unwrap().session_id,
        session.id.0
    );
    assert_eq!(
        f.runner.core.snapshot(session.id).unwrap().label,
        "Renamed display card"
    );
    assert!(f.runner.active_session(session.id, clock()));
    assert_eq!(
        f.runner.run_url(session.id),
        Some(format!("/?routine_run={}", run.id))
    );
    f.runner
        .record_done(&DoneSummary {
            session_id: session.id.0,
            text: "Observed outcome. Tests unperformed.".into(),
            at: "2023-11-14T22:14:00Z".into(),
            ..Default::default()
        })
        .unwrap();
    f.spawner.release.notify_one();
    f.runner.join_launches().await;
    assert_eq!(
        f.runner.store.run(&run.id).unwrap().unwrap().status,
        "finished"
    );
    assert_eq!(
        f.preparation.prompts.lock().unwrap()[0],
        prompt("Inspect synthetic changes.")
    );
    let retried = f.runner.start(&f.id, "click", "manual", clock()).unwrap();
    assert!(retried.existing);
    assert_eq!(f.spawner.calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn missing_preparation_is_a_gate_without_persisted_phantom_launch() {
    let f = fixture();
    let runner = Arc::new(RoutineRunner::new(
        f.runner.store.clone(),
        f.runner.core.clone(),
        f.runner.home.clone(),
        "hub".into(),
        TaskCancellation::default(),
        Arc::new(|_, _| {}),
    ));
    assert_eq!(
        runner.start(&f.id, "click", "manual", clock()).unwrap_err(),
        Error::LaunchUnavailable
    );
    assert!(runner.store.runs("").unwrap().is_empty());
    assert_eq!(f.spawner.calls.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn startup_and_sleep_skip_without_replaying_prompts() {
    let f = fixture();
    let mut def = f.runner.store.definitions().unwrap().pop().unwrap();
    def.enabled = true;
    def.schedule = super::super::schedule::Schedule {
        kind: "daily".into(),
        time: "10:00".into(),
        timezone: "UTC".into(),
    };
    let def = f
        .runner
        .store
        .save(Some(&f.id), def, &f.runner.home, clock())
        .unwrap();
    let due = crate::proto::time::parse_rfc3339(&def.next_run_at).unwrap();
    f.runner.tick(due, true).unwrap();
    let runs = f.runner.store.runs("").unwrap();
    assert_eq!(runs[0].status, "skipped");
    assert!(!runs[0].result_available);
    let def = f.runner.store.definitions().unwrap().pop().unwrap();
    let next = crate::proto::time::parse_rfc3339(&def.next_run_at).unwrap();
    f.runner.tick(next + Duration::new(60, 1), false).unwrap();
    assert_eq!(f.runner.store.runs("").unwrap().len(), 2);
    assert_eq!(f.spawner.calls.load(Ordering::SeqCst), 0);
    let def = f.runner.store.definitions().unwrap().pop().unwrap();
    let next = crate::proto::time::parse_rfc3339(&def.next_run_at).unwrap();
    f.runner
        .tick(next + Duration::from_secs(60), false)
        .unwrap();
    registered(&f).await;
    assert_eq!(f.spawner.calls.load(Ordering::SeqCst), 1);
    f.spawner.release.notify_one();
    f.runner.join_launches().await;
}
