use super::*;
use crate::{
    hub::task_owner::HubTaskOwner,
    terminal::{
        events::CoreEventBus,
        journal::{JournalOptions, SessionJournal},
        session::EngineOptions,
    },
};
use std::sync::atomic::{AtomicUsize, Ordering};
struct Boundary;
impl WrapperTransport for Boundary {
    fn send<'a>(
        &'a self,
        _: SessionBinding,
        _: crate::proto::Message,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async { panic!("no provider terminal in CRUD fixture") })
    }
}
impl CoreEffectSink for Boundary {
    fn apply<'a>(&'a self, _: CoreEffects) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
        Box::pin(async { Ok(()) })
    }
}
impl WrappedSessionSpawner for Boundary {
    fn spawn_and_wait<'a>(
        &'a self,
        spec: WrappedSpawnSpec,
        _: Duration,
        _: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome> {
        Box::pin(async move {
            assert!(spec.subscription_login || spec.usage_probe);
            assert!(spec.cwd.ends_with(if spec.usage_probe {
                "usage-probe"
            } else {
                "subscription-login"
            }));
            SpawnWaitOutcome::Failed("synthetic startup failure".into())
        })
    }
}
#[derive(Default)]
struct Cli {
    calls: AtomicUsize,
    usage_calls: AtomicUsize,
}
impl cli::SubscriptionCli for Cli {
    fn status<'a>(
        &'a self,
        _: &'a str,
        _: &'a Path,
        _: Duration,
        _: &'a TaskCancellation,
    ) -> CoreFuture<'a, io::Result<cli::Status>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            Ok(cli::Status {
                logged_in: true,
                plan: "plus".into(),
                method: "chatgpt".into(),
            })
        })
    }
}
impl appserver::SubscriptionUsageCli for Cli {
    fn codex_usage<'a>(
        &'a self,
        _: &'a Path,
        _: &'a TaskCancellation,
    ) -> CoreFuture<'a, io::Result<appserver::CodexUsage>> {
        self.usage_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err(io::Error::other("synthetic appserver unavailable")) })
    }
}
struct Hooks;
impl probe::SubscriptionProbeHooks for Hooks {
    fn dismiss<'a>(&'a self, _: SessionBinding) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async { panic!("failed fixture never registered") })
    }
    fn wrapper_pid(&self, _: SessionBinding) -> Option<u32> {
        None
    }
    fn recover_process(&self, _: u32) -> io::Result<()> {
        panic!("no recovery PID in CRUD fixture")
    }
}
use std::io;
struct Fixture {
    service: Arc<SubscriptionService>,
    cli: Arc<Cli>,
    _tasks: HubTaskOwner,
    _root: tempfile::TempDir,
    _installed: tempfile::TempDir,
}
fn fixture() -> Fixture {
    fixture_with_spawner(Arc::new(Boundary))
}
fn fixture_with_spawner(spawner: Arc<dyn WrappedSessionSpawner>) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49273, installed.path()).unwrap();
    let home = root.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let config = Arc::new(ConfigStore::new(paths.clone(), Config::defaults(&paths)).unwrap());
    let core = Arc::new(SessionEngine::new(
        EngineOptions::default(),
        Arc::new(SessionJournal::new(
            paths.clone(),
            None,
            JournalOptions::default(),
        )),
        Arc::new(Boundary),
        Arc::new(Boundary),
        spawner,
        CoreEventBus::new(16).unwrap(),
    ));
    let tasks = HubTaskOwner::new(tokio::runtime::Handle::current());
    let cli = Arc::new(Cli::default());
    let service = SubscriptionService::new(SubscriptionDependencies {
        paths: paths.clone(),
        home: home.clone(),
        config,
        launcher: Arc::new(SubscriptionLauncher::new(paths, home, root.path().into())),
        core,
        cli: cli.clone(),
        environment: vec![],
        tasks: tasks.handle(),
        warning: Arc::new(|_| {}),
        probe_hooks: Arc::new(Hooks),
    });
    Fixture {
        service,
        cli,
        _tasks: tasks,
        _root: root,
        _installed: installed,
    }
}

struct StalledSpawn;
impl WrappedSessionSpawner for StalledSpawn {
    fn spawn_and_wait<'a>(
        &'a self,
        spec: WrappedSpawnSpec,
        wait: Duration,
        _: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome> {
        Box::pin(async move {
            assert!(spec.usage_probe);
            assert_eq!(wait, Duration::from_secs(20));
            // Deliberately violate the spawner's own 20-second wait to exercise
            // the actual probe service's independent work deadline.
            tokio::time::sleep(Duration::from_secs(70)).await;
            panic!("probe work deadline did not cancel the stalled synthetic spawn")
        })
    }
}

#[tokio::test(start_paused = true)]
async fn probe_work_deadline_releases_claim_when_spawn_never_registers() {
    let f = fixture_with_spawner(Arc::new(StalledSpawn));
    f.service
        .add("claude", "p1", "synthetic")
        .unwrap_or_else(|_| panic!("add profile"));
    let started = tokio::time::Instant::now();
    let error = f
        .service
        .probe(
            "claude",
            "p1",
            false,
            Timestamp::now(),
            &TaskCancellation::default(),
        )
        .await
        .err()
        .unwrap();
    assert_eq!(error.status, 504);
    assert_eq!(error.code, "probe_timeout");
    assert_eq!(started.elapsed(), Duration::from_secs(60));
    assert!(!f.service.probes.running("claude", "p1"));
    assert!(
        !f.service
            .paths
            .root()
            .join("usage-probe/active/claude-p1.json")
            .exists()
    );
    assert_eq!(f._tasks.snapshot().effects.running, 0);
    eprintln!(
        "synthetic stalled probe: work_elapsed_ms={}",
        started.elapsed().as_millis()
    );
}
#[tokio::test]
async fn crud_uses_single_config_registry_and_never_reads_credentials() {
    let f = fixture();
    let base = f
        .service
        .paths
        .resource(Resource::Subscriptions)
        .join("codex");
    std::fs::create_dir_all(base.join("p1")).unwrap();
    let list = f
        .service
        .add("codex", "", " Main profile ")
        .unwrap_or_else(|e| panic!("{}", e.detail));
    let row = &list["providers"][1]["profiles"][0];
    assert_eq!(row["id"], "main-profile");
    assert!(row["profile_dir"].as_str().unwrap().ends_with("p2"));
    let profile = PathBuf::from(row["profile_dir"].as_str().unwrap());
    std::fs::write(
        profile.join("auth.json"),
        b"synthetic credential never returned",
    )
    .unwrap();
    let list = f
        .service
        .update("codex", "MAIN-PROFILE", Some(" New name "), Some(false))
        .unwrap_or_else(|e| panic!("{}", e.detail));
    assert_eq!(list["providers"][1]["profiles"][0]["enabled"], false);
    assert!(!list.to_string().contains("synthetic credential"));
    let removed = f
        .service
        .remove("codex", "main-profile", true)
        .unwrap_or_else(|e| panic!("{}", e.detail));
    assert_eq!(removed["credentials_deleted"], true);
    assert!(!profile.exists());
    assert_eq!(f.cli.calls.load(Ordering::SeqCst), 0);
    assert!(f.service.core.snapshots().is_empty());
}
#[tokio::test]
async fn signed_in_status_updates_plan_and_probe_failure_releases_marker() {
    let f = fixture();
    f.service
        .add("codex", "p1", "codex")
        .unwrap_or_else(|e| panic!("{}", e.detail));
    let status = f
        .service
        .test("codex", "p1", &TaskCancellation::default())
        .await
        .unwrap_or_else(|e| panic!("{}", e.detail));
    assert_eq!(status["logged_in"], true);
    assert_eq!(
        f.service.config.snapshot().unwrap().config.subscriptions["codex"][0].plan,
        "plus"
    );
    f.service
        .add("claude", "p1", "claude")
        .unwrap_or_else(|e| panic!("{}", e.detail));
    let failed = f
        .service
        .probe(
            "claude",
            "p1",
            false,
            Timestamp::now(),
            &TaskCancellation::default(),
        )
        .await
        .err()
        .unwrap();
    assert_eq!(failed.status, 502);
    assert!(!f.service.probes.running("claude", "p1"));
    assert!(
        !f.service
            .paths
            .root()
            .join("usage-probe/active/claude-p1.json")
            .exists()
    );
    let cancelled = f
        .service
        .probe(
            "claude",
            "p1",
            true,
            Timestamp::now(),
            &TaskCancellation::default(),
        )
        .await
        .err()
        .unwrap();
    assert_eq!(cancelled.status, 404);
}
#[tokio::test]
async fn add_save_failure_rolls_back_reserved_profile_but_update_keeps_memory() {
    let f = fixture();
    let config_path = f.service.paths.resource(Resource::Config);
    std::fs::create_dir(&config_path).unwrap();
    let failed = f.service.add("codex", "p1", "test").err().unwrap();
    assert_eq!(failed.status, 500);
    assert!(
        !f.service
            .config
            .snapshot()
            .unwrap()
            .config
            .subscriptions
            .contains_key("codex")
    );
    std::fs::remove_dir(&config_path).unwrap();
    f.service
        .add("codex", "p1", "first")
        .unwrap_or_else(|e| panic!("{}", e.detail));
    std::fs::remove_file(&config_path).unwrap();
    std::fs::create_dir(&config_path).unwrap();
    assert!(
        f.service
            .update("codex", "p1", Some("retained"), None)
            .is_err()
    );
    assert_eq!(
        f.service.config.snapshot().unwrap().config.subscriptions["codex"][0].name,
        "retained"
    );
}
#[test]
fn subscription_ids_use_pinned_go_simple_lower_without_expansion() {
    assert_eq!(config::normalize_subscription_id(" İ "), "i");
    assert!(config::validate_subscription_id(&config::normalize_subscription_id("İ")).is_ok());
    assert_eq!(slug(" Main profile "), "main-profile");
    assert_eq!(display_name(" x\u{0007}y "), "xy");
}

#[tokio::test]
async fn refresh_wrong_session_profile_or_home_never_launches_usage_cli() {
    let f = fixture();
    for id in ["p1", "p2"] {
        f.service
            .add("codex", id, id)
            .unwrap_or_else(|e| panic!("{}", e.detail));
    }
    let home = f
        .service
        .resolve("codex", "p1")
        .unwrap_or_else(|e| panic!("{}", e.detail))
        .unwrap()
        .path;
    let binding = f
        .service
        .core
        .register(
            RegisterRequest {
                message: crate::proto::Message {
                    provider: "codex".into(),
                    cwd: f._root.path().to_string_lossy().into_owned(),
                    pid: 3002,
                    subscription_id: "p1".into(),
                    codex_home: home.to_string_lossy().into_owned(),
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
    let before = f.service.config.snapshot().unwrap();
    let bytes = std::fs::read(f.service.paths.resource(Resource::Config)).unwrap();
    for session in [binding.session.0, i64::MAX] {
        let error = f
            .service
            .refresh_codex("codex", "p2", session, &TaskCancellation::default())
            .await
            .err()
            .unwrap();
        assert_eq!((error.status, error.code), (409, "session_profile_changed"));
    }
    assert_eq!(f.cli.usage_calls.load(Ordering::SeqCst), 0);
    assert_eq!(f.cli.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        f.service.config.snapshot().unwrap().revision,
        before.revision
    );
    assert_eq!(
        std::fs::read(f.service.paths.resource(Resource::Config)).unwrap(),
        bytes
    );

    // A valid binding reaches the fake CLI. A later rejected binding must not
    // inherit that successful admission, or leave a refresh claim behind.
    let error = f
        .service
        .refresh_codex(
            "codex",
            "p1",
            binding.session.0,
            &TaskCancellation::default(),
        )
        .await
        .err()
        .unwrap();
    assert_ne!(error.code, "session_profile_changed");
    assert_eq!(f.cli.usage_calls.load(Ordering::SeqCst), 1);
    let error = f
        .service
        .refresh_codex(
            "codex",
            "p1",
            binding.session.0,
            &TaskCancellation::default(),
        )
        .await
        .err()
        .unwrap();
    assert_eq!(error.code, "usage_refresh_failed");
    assert_eq!(f.cli.usage_calls.load(Ordering::SeqCst), 2);
    let mut changed = f.service.config.snapshot().unwrap().config;
    let changed_home = f
        .service
        .paths
        .resource(Resource::Subscriptions)
        .join("codex/other-owned-home");
    std::fs::create_dir_all(&changed_home).unwrap();
    changed.subscriptions.get_mut("codex").unwrap()[0].profile_dir =
        changed_home.to_string_lossy().into_owned();
    let revision = f.service.config.snapshot().unwrap().revision;
    f.service
        .config
        .publish_then_persist_legacy(revision, changed)
        .unwrap();
    let error = f
        .service
        .refresh_codex(
            "codex",
            "p1",
            binding.session.0,
            &TaskCancellation::default(),
        )
        .await
        .err()
        .unwrap();
    assert_eq!((error.status, error.code), (409, "session_profile_changed"));
    assert_eq!(f.cli.usage_calls.load(Ordering::SeqCst), 2);
}

struct NoProbeSpawn;
impl WrappedSessionSpawner for NoProbeSpawn {
    fn spawn_and_wait<'a>(
        &'a self,
        _: WrappedSpawnSpec,
        _: Duration,
        _: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome> {
        panic!("deleting an absent probe must not invoke any spawn")
    }
}
#[tokio::test]
async fn delete_absent_probe_does_not_create_marker_or_start_any_work() {
    let f = fixture_with_spawner(Arc::new(NoProbeSpawn));
    f.service
        .add("claude", "p1", "synthetic")
        .unwrap_or_else(|e| panic!("{}", e.detail));
    let config_path = f.service.paths.resource(Resource::Config);
    let before = std::fs::read(&config_path).unwrap();
    let revision = f.service.config.snapshot().unwrap().revision;
    for id in ["p1", "unregistered"] {
        let error = f
            .service
            .probe(
                "claude",
                id,
                true,
                Timestamp::now(),
                &TaskCancellation::default(),
            )
            .await
            .err()
            .unwrap();
        assert_eq!((error.status, error.code), (404, "not_found"));
    }
    assert_eq!(f.cli.calls.load(Ordering::SeqCst), 0);
    assert_eq!(f.cli.usage_calls.load(Ordering::SeqCst), 0);
    assert!(f.service.core.snapshots().is_empty());
    assert!(!f.service.paths.root().join("usage-probe").exists());
    assert_eq!(f._tasks.snapshot().effects.running, 0);
    assert_eq!(f.service.config.snapshot().unwrap().revision, revision);
    assert_eq!(std::fs::read(config_path).unwrap(), before);
}

#[tokio::test]
async fn usage_get_runs_only_the_recorded_auth_status_effect_and_reuses_its_cache() {
    let f = fixture_with_spawner(Arc::new(NoProbeSpawn));
    f.service
        .add("codex", "p1", "synthetic")
        .unwrap_or_else(|e| panic!("{}", e.detail));
    let path = f.service.paths.resource(Resource::Config);
    let before = std::fs::read(&path).unwrap();
    let revision = f.service.config.snapshot().unwrap().revision;
    let http = crate::hub::subscription_routes::SubscriptionHttp::new(f.service.clone());
    let mut request = crate::hub::http::Request {
        method: "GET".into(),
        path: "/api/subscription-usage".into(),
        ..Default::default()
    };
    for (force, expected) in [(false, 1), (false, 1), (true, 2)] {
        request.query = if force {
            "refresh_auth=1".into()
        } else {
            String::new()
        };
        assert_eq!(
            http.handle_authenticated(&request, Timestamp::now())
                .await
                .unwrap()
                .status,
            200
        );
        f._tasks.drain_effects().await;
        assert_eq!(f.cli.calls.load(Ordering::SeqCst), expected);
        assert_eq!(f.cli.usage_calls.load(Ordering::SeqCst), 0);
    }
    assert_eq!(f.service.config.snapshot().unwrap().revision, revision);
    assert_eq!(std::fs::read(path).unwrap(), before);
    assert!(f.service.core.snapshots().is_empty());
}

#[tokio::test]
async fn usage_get_reads_synthetic_profile_logs_into_memory_without_provider_calls() {
    use std::io::Write;
    let f = fixture_with_spawner(Arc::new(NoProbeSpawn));
    for provider in ["codex", "grok"] {
        f.service
            .add(provider, "p1", "synthetic")
            .unwrap_or_else(|e| panic!("{}", e.detail));
    }
    let at = Timestamp::now();
    for provider in ["codex", "grok"] {
        f.service
            .usage
            .set_auth(provider, "p1", "authenticated", at);
    }
    let config = f.service.config.snapshot().unwrap();
    let config_path = f.service.paths.resource(Resource::Config);
    let config_bytes = std::fs::read(&config_path).unwrap();
    let profile_path = |provider: &str| {
        config::resolve_subscription_profile_dir(
            &f.service.paths,
            provider,
            &config.config.subscriptions[provider][0],
            Some(&f.service.home),
        )
        .unwrap()
    };
    let codex_log = profile_path("codex")
        .join("sessions")
        .join("rollout-synthetic.jsonl");
    let grok_log = profile_path("grok").join("logs").join("unified.jsonl");
    let http = crate::hub::subscription_routes::SubscriptionHttp::new(f.service.clone());
    let request = crate::hub::http::Request {
        method: "GET".into(),
        path: "/api/subscription-usage".into(),
        ..Default::default()
    };
    let row = |value: &Value, provider: &str| {
        value["providers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|section| section["provider"] == provider)
            .unwrap()["profiles"][0]
            .clone()
    };
    let first = http.handle_authenticated(&request, at).await.unwrap();
    assert_eq!(first.status, 200);
    let first: Value = serde_json::from_slice(&first.body).unwrap();
    assert!(row(&first, "codex").get("codex").is_none());
    assert!(row(&first, "grok").get("grok").is_none());
    std::fs::create_dir_all(codex_log.parent().unwrap()).unwrap();
    std::fs::create_dir_all(grok_log.parent().unwrap()).unwrap();
    let codex_event = |timestamp: &str, used: u8| {
        serde_json::json!({
            "timestamp":timestamp,"payload":{"type":"token_count","rate_limits":{
                "primary":{"used_percent":used,"window_minutes":300},"plan_type":"synthetic"
            }}
        })
        .to_string()
            + "\n"
    };
    let grok_event = |timestamp: &str, used: u8| {
        serde_json::json!({
            "ts":timestamp,"msg":"billing: fetched credits config","ctx":{"config":{
                "creditUsagePercent":used,"currentPeriod":{"type":"weekly"}
            }}
        })
        .to_string()
            + "\n"
    };
    std::fs::write(&codex_log, codex_event("2026-10-01T00:00:00Z", 80)).unwrap();
    std::fs::write(&grok_log, grok_event("2026-10-01T00:00:00Z", 35)).unwrap();
    for (codex_used, grok_used) in [(80, 35), (20, 65)] {
        let expected_codex = std::fs::read(&codex_log).unwrap();
        let expected_grok = std::fs::read(&grok_log).unwrap();
        let response = http.handle_authenticated(&request, at).await.unwrap();
        assert_eq!(response.status, 200);
        let value: Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(
            row(&value, "codex")["codex"]["primary"]["used_percent"].as_f64(),
            Some(f64::from(codex_used))
        );
        assert_eq!(
            row(&value, "codex")["codex"]["primary"]["remaining_percent"].as_f64(),
            Some(f64::from(100 - codex_used))
        );
        assert_eq!(
            row(&value, "grok")["grok"]["used_percent"].as_f64(),
            Some(f64::from(grok_used))
        );
        assert_eq!(
            row(&value, "grok")["grok"]["remaining_percent"].as_f64(),
            Some(f64::from(100 - grok_used))
        );
        assert_eq!(row(&value, "grok")["grok"]["period_type"], "weekly");
        assert_eq!(std::fs::read(&codex_log).unwrap(), expected_codex);
        assert_eq!(std::fs::read(&grok_log).unwrap(), expected_grok);
        if codex_used == 80 {
            std::fs::OpenOptions::new()
                .append(true)
                .open(&codex_log)
                .unwrap()
                .write_all(codex_event("2026-10-01T00:00:01Z", 20).as_bytes())
                .unwrap();
            std::fs::OpenOptions::new()
                .append(true)
                .open(&grok_log)
                .unwrap()
                .write_all(grok_event("2026-10-01T00:00:01Z", 65).as_bytes())
                .unwrap();
        }
    }
    // Remove only these synthetic log files, then observe the owner's retained
    // memory values through GET; no provider refresh can refill them.
    std::fs::remove_file(&codex_log).unwrap();
    std::fs::remove_file(&grok_log).unwrap();
    let cached = http.handle_authenticated(&request, at).await.unwrap();
    let cached: Value = serde_json::from_slice(&cached.body).unwrap();
    assert_eq!(
        row(&cached, "codex")["codex"]["primary"]["used_percent"].as_f64(),
        Some(20.0)
    );
    assert_eq!(
        row(&cached, "grok")["grok"]["used_percent"].as_f64(),
        Some(65.0)
    );
    assert_eq!(f.cli.calls.load(Ordering::SeqCst), 0);
    assert_eq!(f.cli.usage_calls.load(Ordering::SeqCst), 0);
    assert_eq!(f._tasks.snapshot().effects.running, 0);
    assert_eq!(f._tasks.snapshot().effects.completed, 0);
    assert_eq!(
        f.service.config.snapshot().unwrap().revision,
        config.revision
    );
    assert_eq!(std::fs::read(config_path).unwrap(), config_bytes);
    assert!(f.service.core.snapshots().is_empty());
}
