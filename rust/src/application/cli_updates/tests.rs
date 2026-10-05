use super::*;
use crate::{
    application::spawn_policy::PathEnvironment,
    config::{Config, ConfigStore, Resource},
    hub::{
        cli_version::{
            CliVersionCommand, CliVersionDependencies, CliVersionExecutor, CliVersionFailure,
            CliVersionProcessOutput,
        },
        http::Request,
        task_owner::HubTaskOwner,
    },
    process::execpath::{ExecutableFs, Platform},
    profile::store::FileStore,
    proto::provider::{Definition, LaunchDefinition, UpdateDefinition},
    terminal::{
        events::CoreEventBus,
        journal::{JournalOptions, SessionJournal},
        session::EngineOptions,
    },
};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{Notify, Semaphore};
struct Fs;
impl ExecutableFs for Fs {
    fn exists(&self, path: &str) -> bool {
        std::path::Path::new(path).is_file()
    }
    fn is_executable(&self, path: &str, _: Platform) -> bool {
        self.exists(path)
    }
    fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        std::fs::read(path)
    }
}
struct Environment;
impl PathEnvironment for Environment {
    fn sanitize(&self, environment: &[String]) -> io::Result<Vec<String>> {
        Ok(environment.to_vec())
    }
}
struct Versions {
    calls: Mutex<BTreeMap<String, usize>>,
    stage: AtomicUsize,
    error_stage: AtomicUsize,
    active: AtomicUsize,
    entered: Notify,
    gate: Semaphore,
}
struct Active<'a>(&'a AtomicUsize);
impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
impl Default for Versions {
    fn default() -> Self {
        Self {
            calls: Mutex::new(BTreeMap::new()),
            stage: AtomicUsize::new(0),
            error_stage: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
            entered: Notify::new(),
            gate: Semaphore::new(0),
        }
    }
}
impl CliVersionExecutor for Versions {
    fn execute(
        &self,
        command: CliVersionCommand,
    ) -> CoreFuture<'_, Result<CliVersionProcessOutput, CliVersionFailure>> {
        Box::pin(async move {
            let count = {
                let mut calls = self.calls.lock().unwrap();
                let count = calls.entry(command.executable.clone()).or_default();
                *count += 1;
                *count
            };
            self.active.fetch_add(1, Ordering::SeqCst);
            let _active = Active(&self.active);
            self.entered.notify_one();
            if self.stage.load(Ordering::SeqCst) == count {
                self.gate.acquire().await.unwrap().forget();
            }
            Ok(CliVersionProcessOutput {
                output: format!("synthetic v{count}\n").into_bytes(),
                exit_code: if self.error_stage.load(Ordering::SeqCst) == count {
                    1
                } else {
                    0
                },
                ..Default::default()
            })
        })
    }
}
struct Executor {
    calls: Mutex<Vec<String>>,
    active: AtomicUsize,
    maximum: AtomicUsize,
    entered: Notify,
    gate: Semaphore,
}
impl Default for Executor {
    fn default() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            active: AtomicUsize::new(0),
            maximum: AtomicUsize::new(0),
            entered: Notify::new(),
            gate: Semaphore::new(0),
        }
    }
}
impl UpdateExecutor for Executor {
    fn execute<'a>(
        &'a self,
        plan: &'a UpdatePlan,
        cancel: &'a Cancellation,
    ) -> CoreFuture<'a, io::Result<ProcessOutput>> {
        Box::pin(async move {
            self.calls
                .lock()
                .unwrap()
                .push(plan.command.provider.clone());
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.maximum.fetch_max(active, Ordering::SeqCst);
            let _active = Active(&self.active);
            self.entered.notify_one();
            let outcome = tokio::select! {_=cancel.cancelled()=>ExitOutcome::Cancelled,permit=self.gate.acquire()=>{permit.unwrap().forget();ExitOutcome::Exited{code:Some(0),signal:None}}};
            Ok(ProcessOutput {
                stdout: b"synthetic combined output".to_vec(),
                stderr: Vec::new(),
                stdout_truncated: false,
                stderr_truncated: false,
                pipes_forced_closed: false,
                outcome,
            })
        })
    }
}
struct Transport;
impl WrapperTransport for Transport {
    fn send<'a>(
        &'a self,
        _: SessionBinding,
        _: crate::proto::Message,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async { Ok(()) })
    }
}
struct Effects;
impl CoreEffectSink for Effects {
    fn apply<'a>(&'a self, _: CoreEffects) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
        Box::pin(async { Ok(()) })
    }
}
struct Spawn;
impl WrappedSessionSpawner for Spawn {
    fn spawn_and_wait<'a>(
        &'a self,
        _: WrappedSpawnSpec,
        _: std::time::Duration,
        _: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome> {
        Box::pin(async { panic!("updater fixture must not spawn provider") })
    }
}
struct Fixture {
    _root: tempfile::TempDir,
    paths: RuntimePaths,
    owner: HubTaskOwner,
    updates: Arc<CliUpdates>,
    executor: Arc<Executor>,
    versions: Arc<Versions>,
    directory: Arc<Mutex<PathBuf>>,
    warnings: Arc<AtomicUsize>,
}
impl Fixture {
    fn new(count: usize) -> Self {
        let root = tempfile::tempdir().unwrap();
        let runtime = root.path().join("runtime");
        let installed = root.path().join("installed");
        std::fs::create_dir(&runtime).unwrap();
        std::fs::create_dir(&installed).unwrap();
        let paths = RuntimePaths::trial(&runtime, 49971, &installed).unwrap();
        let config = Arc::new(ConfigStore::new(paths.clone(), Config::default()).unwrap());
        for index in 0..count {
            let id = format!("fixture-{index:02}");
            let file = if cfg!(windows) {
                format!("{id}.exe")
            } else {
                id.clone()
            };
            std::fs::write(runtime.join(file), b"synthetic fixture; never executed").unwrap();
            FileStore::new(&paths)
                .create_new(&Definition {
                    schema_version: 1,
                    id: id.clone(),
                    display_name: id.clone(),
                    launch: Some(LaunchDefinition {
                        executable: id,
                        ..Default::default()
                    }),
                    update: Some(UpdateDefinition {
                        enabled: Some(true),
                        args: vec!["update".into()],
                        ..Default::default()
                    }),
                    ..Default::default()
                })
                .unwrap();
        }
        let store = Arc::new(ProviderRegistryStore::new(&paths, config).unwrap());
        let versions = Arc::new(Versions::default());
        let http = Arc::new(CliVersionHttp::new(CliVersionDependencies {
            executor: Some(versions.clone()),
            cwd: runtime.clone(),
            environment: vec![format!("PATH={}", runtime.display()), "PATHEXT=.EXE".into()],
            platform: Platform::native(),
            path_environment: Arc::new(Environment),
            executable_fs: Arc::new(Fs),
            modified_at: Arc::new(|_| None),
            clock: Arc::new(Timestamp::now),
        }));
        let core = Arc::new(SessionEngine::new(
            EngineOptions::default(),
            Arc::new(SessionJournal::new(
                paths.clone(),
                None,
                JournalOptions::default(),
            )),
            Arc::new(Transport),
            Arc::new(Effects),
            Arc::new(Spawn),
            CoreEventBus::new(32).unwrap(),
        ));
        let owner = HubTaskOwner::new(tokio::runtime::Handle::current());
        let executor = Arc::new(Executor::default());
        let directory = Arc::new(Mutex::new(paths.resource(Resource::Update)));
        let selected = directory.clone();
        let warnings = Arc::new(AtomicUsize::new(0));
        let warn = warnings.clone();
        let lookup = runtime.clone();
        let updates = CliUpdates::new(Dependencies {
            core,
            store,
            versions: http,
            paths: paths.clone(),
            cwd: runtime,
            environment: vec!["PATH=".into()],
            tasks: owner.handle(),
            resolve: Arc::new(move |name| {
                let file = if cfg!(windows) {
                    format!("{name}.exe")
                } else {
                    name.into()
                };
                let path = lookup.join(file);
                path.is_file().then_some(path)
            }),
            executor: executor.clone(),
            warning: Arc::new(move |_, _| {
                warn.fetch_add(1, Ordering::SeqCst);
            }),
            log_directory: Arc::new(move || Ok(selected.lock().unwrap().clone())),
        });
        Self {
            _root: root,
            paths,
            owner,
            updates,
            executor,
            versions,
            directory,
            warnings,
        }
    }
    fn create(&self, ids: Vec<String>, serial: bool) -> String {
        let response = self
            .updates
            .create(
                ids,
                serial,
                crate::proto::time::parse_rfc3339("2026-10-05T01:02:03.123456789Z").unwrap(),
            )
            .unwrap();
        response["job_id"].as_str().unwrap().into()
    }
    async fn active(&self, count: usize) {
        while self.executor.active.load(Ordering::SeqCst) < count {
            self.executor.entered.notified().await;
        }
    }
}
#[tokio::test]
async fn admitted_job_outlives_http_response_and_holds_real_provider_lease_through_version_after() {
    let fixture = Fixture::new(1);
    fixture.versions.stage.store(2, Ordering::SeqCst);
    let id = fixture.create(vec!["fixture-00".into()], false);
    fixture.active(1).await;
    assert!(
        fixture
            .updates
            .dependencies
            .core
            .provider_updating("fixture-00")
    );
    assert!(matches!(
        fixture
            .updates
            .dependencies
            .core
            .begin_provider_spawn("fixture-00"),
        Err(ProviderAdmissionError::AlreadyUpdating)
    ));
    let duplicate = fixture
        .updates
        .create(vec!["fixture-00".into()], false, Timestamp::now())
        .unwrap();
    assert!(duplicate["accepted"].is_null());
    assert_eq!(duplicate["excluded"][0]["reason"], "already_updating");
    fixture.executor.gate.add_permits(1);
    while fixture.versions.active.load(Ordering::SeqCst) == 0 {
        fixture.versions.entered.notified().await;
    }
    assert!(
        fixture
            .updates
            .dependencies
            .core
            .provider_updating("fixture-00")
    );
    fixture.versions.gate.add_permits(1);
    fixture.owner.drain_effects().await;
    assert!(
        !fixture
            .updates
            .dependencies
            .core
            .provider_updating("fixture-00")
    );
    let job = fixture.updates.job(&id).unwrap().snapshot();
    assert_eq!(job["providers"][0]["state"], "updated");
    assert_eq!(job["providers"][0]["version_before"], "synthetic v1");
    assert_eq!(job["providers"][0]["version_after"], "synthetic v2");
    assert_eq!(job["started_at"], "2026-10-05T01:02:03Z");
    assert!(job["providers"][0].get("exit_code").is_none());
    let cached = fixture
        .updates
        .dependencies
        .versions
        .handle_authenticated(
            &Request {
                method: "GET".into(),
                path: "/api/cli-versions".into(),
                ..Default::default()
            },
            None,
        )
        .await;
    let cached: serde_json::Value = serde_json::from_slice(&cached.body).unwrap();
    assert_eq!(cached["results"][0]["version_line"], "synthetic v2");
}
#[tokio::test]
async fn parallel_jobs_are_bounded_to_eight_and_serial_preserves_request_order() {
    let fixture = Fixture::new(13);
    let ids = (0..13).map(|index| format!("fixture-{index:02}")).collect();
    fixture.create(ids, false);
    fixture.active(8).await;
    assert_eq!(fixture.executor.maximum.load(Ordering::SeqCst), 8);
    fixture.executor.gate.add_permits(13);
    fixture.owner.drain_effects().await;
    assert_eq!(fixture.executor.calls.lock().unwrap().len(), 13);
    let serial = Fixture::new(3);
    serial.create(
        vec![
            "fixture-02".into(),
            "fixture-00".into(),
            "fixture-01".into(),
        ],
        true,
    );
    serial.active(1).await;
    assert_eq!(*serial.executor.calls.lock().unwrap(), vec!["fixture-02"]);
    serial.executor.gate.add_permits(3);
    serial.owner.drain_effects().await;
    assert_eq!(serial.executor.maximum.load(Ordering::SeqCst), 1);
    assert_eq!(
        *serial.executor.calls.lock().unwrap(),
        vec!["fixture-02", "fixture-00", "fixture-01"]
    );
}
#[tokio::test]
async fn shutdown_cancellation_interrupts_before_and_after_version_without_releasing_lease_early() {
    for stage in [1, 2] {
        let fixture = Fixture::new(1);
        fixture.versions.stage.store(stage, Ordering::SeqCst);
        let id = fixture.create(vec!["fixture-00".into()], true);
        if stage == 2 {
            fixture.active(1).await;
            fixture.executor.gate.add_permits(1);
        }
        while fixture.versions.active.load(Ordering::SeqCst) == 0 {
            fixture.versions.entered.notified().await;
        }
        fixture.owner.stop_requests();
        fixture.owner.drain_requests().await;
        fixture.owner.stop_effects().unwrap();
        fixture.owner.cancel_effects().unwrap();
        fixture.owner.drain_effects().await;
        assert_eq!(fixture.versions.active.load(Ordering::SeqCst), 0);
        assert!(
            !fixture
                .updates
                .dependencies
                .core
                .provider_updating("fixture-00")
        );
        let state = fixture.updates.job(&id).unwrap().snapshot();
        assert_eq!(
            state["providers"][0]["state"],
            if stage == 1 { "failed" } else { "unknown" }
        );
        if stage == 1 {
            assert!(fixture.executor.calls.lock().unwrap().is_empty());
        }
    }
}
#[tokio::test]
async fn history_eviction_does_not_drop_active_job_lease_and_log_remains_on_original_directory() {
    let fixture = Fixture::new(1);
    let id = fixture.create(vec!["fixture-00".into()], false);
    fixture.active(1).await;
    for _ in 0..10 {
        fixture.create(vec!["missing".into()], false);
    }
    assert!(fixture.updates.job(&id).is_none());
    assert!(
        fixture
            .updates
            .dependencies
            .core
            .provider_updating("fixture-00")
    );
    fixture.executor.gate.add_permits(1);
    fixture.owner.drain_effects().await;
    assert!(
        !fixture
            .updates
            .dependencies
            .core
            .provider_updating("fixture-00")
    );
    let id = fixture.create(vec!["fixture-00".into()], true);
    fixture.executor.gate.add_permits(1);
    fixture.owner.drain_effects().await;
    *fixture.directory.lock().unwrap() = fixture.paths.root().join("new-private-log-dir");
    let log = fixture.updates.log(&id, "fixture-00").unwrap();
    assert!(
        log["content"]
            .as_str()
            .unwrap()
            .contains("resolved_executable:")
    );
    assert_eq!(log["truncated"], false);
}
#[tokio::test]
async fn canonical_trial_checks_both_update_and_version_executable_and_log_failure_does_not_hold_lease()
 {
    let fixture = Fixture::new(1);
    let lease = fixture
        .updates
        .dependencies
        .core
        .begin_provider_spawn("fixture-00")
        .unwrap();
    let id = fixture.create(vec!["fixture-00".into()], true);
    assert_eq!(
        fixture.updates.job(&id).unwrap().snapshot()["excluded"][0]["reason"],
        "running_sessions"
    );
    fixture.updates.dependencies.core.end_provider_spawn(lease);
    let blocked = fixture.paths.root().join("blocked");
    std::fs::write(&blocked, b"not a directory").unwrap();
    *fixture.directory.lock().unwrap() = blocked;
    fixture.create(vec!["fixture-00".into()], true);
    fixture.executor.gate.add_permits(1);
    fixture.owner.drain_effects().await;
    assert_eq!(fixture.warnings.load(Ordering::SeqCst), 1);
    assert!(
        !fixture
            .updates
            .dependencies
            .core
            .provider_updating("fixture-00")
    );
}
#[test]
fn log_read_failure_is_500_and_prefix_is_bounded() {
    struct Failing;
    impl Read for Failing {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("synthetic read failure"))
        }
    }
    let error = read_log(&mut Failing).unwrap_err();
    assert_eq!(error.status, 500);
    let data: serde_json::Value = serde_json::from_slice(&error.body).unwrap();
    assert_eq!(data["error"], "read_failed");
    let bytes = vec![b'x'; 256 * 1024 + 100];
    assert_eq!(read_log(&mut &bytes[..]).unwrap().len(), 256 * 1024 + 1);
}
#[tokio::test]
async fn trial_refuses_external_updater_and_external_version_launch_before_any_probe() {
    for external_launch in [false, true] {
        let mut fixture = Fixture::new(1);
        let outside = fixture._root.path().join("installed/outside.exe");
        std::fs::write(&outside, b"never execute").unwrap();
        let inside = fixture.paths.root().join("inside-updater.exe");
        std::fs::write(&inside, b"never execute").unwrap();
        FileStore::new(&fixture.paths)
            .save(&Definition {
                schema_version: 1,
                id: "fixture-00".into(),
                display_name: "Synthetic boundary".into(),
                launch: Some(LaunchDefinition {
                    executable: "launch".into(),
                    ..Default::default()
                }),
                update: Some(UpdateDefinition {
                    executable: "updater".into(),
                    enabled: Some(true),
                    args: vec!["update".into()],
                    ..Default::default()
                }),
                ..Default::default()
            })
            .unwrap();
        fixture.updates.dependencies.store.reload().unwrap();
        Arc::get_mut(&mut fixture.updates)
            .unwrap()
            .dependencies
            .resolve = Arc::new(move |name| {
            Some(if (name == "launch") == external_launch {
                outside.clone()
            } else {
                inside.clone()
            })
        });
        let response = fixture
            .updates
            .create(vec!["fixture-00".into()], false, Timestamp::now())
            .unwrap();
        assert!(response["accepted"].is_null());
        assert_eq!(response["excluded"][0]["reason"], "not_installed");
        fixture.owner.drain_effects().await;
        assert!(fixture.executor.calls.lock().unwrap().is_empty());
        assert!(fixture.versions.calls.lock().unwrap().is_empty());
    }
}
#[tokio::test]
async fn abort_releases_all_pending_leases_and_terminalizes_job_including_queued_serial_work() {
    let fixture = Fixture::new(3);
    let id = fixture.create(
        vec![
            "fixture-00".into(),
            "fixture-01".into(),
            "fixture-02".into(),
        ],
        true,
    );
    fixture.active(1).await;
    fixture.owner.stop_requests();
    fixture.owner.drain_requests().await;
    fixture.owner.stop_effects().unwrap();
    fixture.owner.abort_effects().unwrap();
    fixture.owner.drain_effects().await;
    let state = fixture.updates.job(&id).unwrap().snapshot();
    for provider in state["providers"].as_array().unwrap() {
        assert_eq!(provider["state"], "failed");
        assert!(
            !fixture
                .updates
                .dependencies
                .core
                .provider_updating(provider["provider"].as_str().unwrap())
        );
    }
}
#[tokio::test]
async fn retention_zero_is_disabled_and_exact_cutoff_keeps_current_logs() {
    let fixture = Fixture::new(1);
    let dir = Dir::open_or_create_private(&fixture.directory.lock().unwrap()).unwrap();
    dir.create_new("old.log", b"fixture", 0o600).unwrap();
    let timestamp = dir
        .open_file("old.log", false)
        .unwrap()
        .metadata()
        .unwrap()
        .modified()
        .unwrap();
    fixture
        .updates
        .clean_logs(timestamp + std::time::Duration::from_secs(3 * 86400), 0)
        .unwrap();
    assert!(dir.open_file("old.log", false).is_ok());
    fixture
        .updates
        .clean_logs(timestamp + std::time::Duration::from_secs(86400), 1)
        .unwrap();
    assert!(dir.open_file("old.log", false).is_ok());
    fixture
        .updates
        .clean_logs(timestamp + std::time::Duration::from_secs(86401), 1)
        .unwrap();
    assert!(dir.open_file("old.log", false).is_err());
}
#[tokio::test]
async fn failed_version_probe_keeps_partial_version_line_without_classifying_it_as_success() {
    let fixture = Fixture::new(1);
    fixture.versions.error_stage.store(1, Ordering::SeqCst);
    let id = fixture.create(vec!["fixture-00".into()], true);
    fixture.executor.gate.add_permits(1);
    fixture.owner.drain_effects().await;
    let job = fixture.updates.job(&id).unwrap().snapshot();
    assert_eq!(job["providers"][0]["state"], "unknown");
    assert_eq!(job["providers"][0]["version_before"], "synthetic v1");
    assert_eq!(job["providers"][0]["version_after"], "synthetic v2");
}
#[test]
fn native_updater_resolves_windows_shims_with_the_explicit_environment_and_keeps_deadline_and_cap()
{
    let root = tempfile::tempdir().unwrap();
    let script = root.path().join("fixture.cmd");
    std::fs::write(&script, b"@echo synthetic shim").unwrap();
    let plan = crate::process::ProcessPlan {
        executable: script.clone(),
        args: vec!["update".into()],
        cwd: root.path().into(),
        env: std::collections::BTreeMap::from([(
            "ComSpec".into(),
            Some("D:\\synthetic\\cmd.exe".into()),
        )]),
        stdin: Vec::new(),
        timeout: std::time::Duration::from_secs(300),
        output_cap: 1024 * 1024,
        pipe_drain_timeout: std::time::Duration::from_millis(500),
    };
    let resolved = normalize_process(&plan, Platform::Windows, &Fs);
    assert_ne!(resolved.executable, script);
    assert_eq!(resolved.executable, PathBuf::from("D:\\synthetic\\cmd.exe"));
    assert!(resolved.args.iter().any(|arg| arg == "/c" || arg == "/C"));
    assert_eq!(resolved.timeout, plan.timeout);
    assert_eq!(resolved.output_cap, plan.output_cap);
    assert_eq!(resolved.env, plan.env);
    let direct = normalize_process(&plan, Platform::Unix, &Fs);
    assert_eq!(direct.executable, script);
    assert_eq!(direct.args, plan.args);
}
