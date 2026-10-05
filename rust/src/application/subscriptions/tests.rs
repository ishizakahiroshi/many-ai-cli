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
        Arc::new(Boundary),
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
