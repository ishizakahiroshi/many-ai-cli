use super::*;
use crate::{
    proto::{self, core::*, time::Timestamp},
    terminal::{
        events::CoreEventBus,
        journal::{JournalOptions, SessionJournal},
        session::EngineOptions,
    },
};
use std::time::Duration;
struct Io;
impl WrapperTransport for Io {
    fn send<'a>(
        &'a self,
        _: SessionBinding,
        _: proto::Message,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async { Ok(()) })
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
        _: WrappedSpawnSpec,
        _: Duration,
        _: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome> {
        Box::pin(async { SpawnWaitOutcome::Failed("no synthetic provider".into()) })
    }
}
struct Fixture {
    _root: tempfile::TempDir,
    owner: Arc<UsageHooks>,
    core: Arc<SessionEngine>,
    path: std::path::PathBuf,
}
fn fixture(enabled: bool) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let runtime = root.path().join("trial");
    std::fs::create_dir(&runtime).unwrap();
    let paths = RuntimePaths::trial(&runtime, 49687, &root.path().join("installed")).unwrap();
    let mut config = Config {
        token: "synthetic_test_token".into(),
        ..Default::default()
    };
    config.user_prefs.token_statusbar.enabled = Some(enabled);
    let config = Arc::new(ConfigStore::new(paths.clone(), config).unwrap());
    let core = Arc::new(SessionEngine::new(
        EngineOptions::default(),
        Arc::new(SessionJournal::new(
            paths.clone(),
            None,
            JournalOptions::default(),
        )),
        Arc::new(Io),
        Arc::new(Io),
        Arc::new(Io),
        CoreEventBus::new(64).unwrap(),
    ));
    let home = runtime.join("home");
    let codex_home = runtime.join("codex");
    let owner = UsageHooks::new(
        config,
        core.clone(),
        paths,
        &home,
        &[format!("CODEX_HOME={}", codex_home.display())],
        &runtime,
        &runtime.join("many-ai-cli.exe"),
        49687,
        Arc::new(|_| panic!("unexpected hook failure")),
    )
    .unwrap();
    Fixture {
        _root: root,
        owner,
        core,
        path: codex_home.join("config.toml"),
    }
}
fn now() -> Timestamp {
    Timestamp::from_unix(1791158400, 0).unwrap()
}
async fn register(
    f: &Fixture,
    provider: &str,
    pid: i64,
    probe: bool,
    login: bool,
) -> SessionBinding {
    f.core
        .register(
            RegisterRequest {
                message: proto::Message {
                    provider: provider.into(),
                    cwd: f._root.path().to_string_lossy().into_owned(),
                    pid,
                    cols: 120,
                    rows: 30,
                    usage_probe: probe,
                    subscription_login: login,
                    ..Default::default()
                },
                spawn_proof: None,
            },
            WrapperConnectionId(pid as u64),
            now(),
        )
        .await
        .unwrap()
        .binding
}
#[tokio::test]
async fn canonical_hidden_probe_and_subscription_flags_are_bound_and_all_registered_ids_retained() {
    let f = fixture(true);
    let ordinary = register(&f, "codex", 701, false, false).await;
    let probe = register(&f, "claude", 702, true, true).await;
    assert!(!f.core.is_subscription_login(ordinary).unwrap());
    assert!(f.core.is_subscription_login(probe).unwrap());
    assert_eq!(f.core.wrapper_pid(probe).unwrap(), Some(702));
    let stale = SessionBinding {
        wrapper: WrapperConnectionId(999),
        ..probe
    };
    assert!(matches!(
        f.core.wrapper_pid(stale),
        Err(SessionError::StaleBinding)
    ));
    assert!(matches!(
        f.core.is_subscription_login(stale),
        Err(SessionError::StaleBinding)
    ));
    assert_eq!(f.core.snapshots().len(), 1);
    let canonical = &*f.core as &dyn SessionCore;
    assert_eq!(canonical.registered_session_ids().len(), 2);
    assert_eq!(f.core.usage_hook_sessions().len(), 2);
}
#[tokio::test]
async fn real_registry_sibling_keeps_global_hook_until_last_codex_disconnects() {
    let f = fixture(true);
    let first = register(&f, "codex", 711, false, false).await;
    f.owner.registered();
    assert!(
        std::fs::read_to_string(&f.path)
            .unwrap()
            .contains(&format!("--session {}", first.session.0))
    );
    let second = register(&f, "codex", 712, true, false).await;
    f.owner.registered();
    assert_eq!(
        std::fs::read_to_string(&f.path)
            .unwrap()
            .matches("[[hooks.Stop]]")
            .count(),
        1
    );
    f.core.disconnected(first, now()).unwrap();
    f.owner.ended("codex");
    assert!(
        std::fs::read_to_string(&f.path)
            .unwrap()
            .contains("[[hooks.Stop]]")
    );
    f.core.disconnected(second, now()).unwrap();
    f.owner.ended("codex");
    assert!(
        !std::fs::read_to_string(&f.path)
            .unwrap()
            .contains("[[hooks.Stop]]")
    );
}
#[tokio::test]
async fn disabled_registration_does_not_write_and_shutdown_removes_old_global_block() {
    let f = fixture(false);
    register(&f, "codex", 721, false, false).await;
    f.owner.registered();
    assert!(!f.path.exists());
    std::fs::create_dir_all(f.path.parent().unwrap()).unwrap();
    std::fs::write(
        &f.path,
        "model=\"keep\"\n# any-ai-cli:usage-hook-start\nstale\n# any-ai-cli:usage-hook-end\n",
    )
    .unwrap();
    f.owner.shutdown();
    assert_eq!(std::fs::read_to_string(&f.path).unwrap(), "model=\"keep\"");
}
#[tokio::test]
async fn claude_keeps_wrapper_owned_settings_and_does_not_create_codex_config() {
    let f = fixture(true);
    register(&f, "claude", 731, false, false).await;
    f.owner.registered();
    f.owner.ended("claude");
    assert!(!f.path.exists());
}
