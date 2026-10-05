use super::*;
use crate::{
    application::instruction_rules::InstructionRootResolver,
    config::{Config, ConfigStore, RuntimePaths},
    hub::task_owner::HubTaskOwner,
    proto::{self, time::Timestamp},
    terminal::{
        events::CoreEventBus,
        journal::{JournalOptions, SessionJournal},
        session::{EngineOptions, SessionEngine},
    },
};
use std::path::{Path, PathBuf};

struct Unused;
impl WrapperTransport for Unused {
    fn send<'a>(
        &'a self,
        _: SessionBinding,
        _: proto::Message,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async { panic!("instruction observer must not write PTY") })
    }
}
impl CoreEffectSink for Unused {
    fn apply<'a>(&'a self, _: CoreEffects) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
        Box::pin(async { panic!("instruction observer must not apply unrelated effects") })
    }
}
impl WrappedSessionSpawner for Unused {
    fn spawn_and_wait<'a>(
        &'a self,
        _: WrappedSpawnSpec,
        _: std::time::Duration,
        _: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome> {
        Box::pin(async { panic!("instruction observer must not spawn provider") })
    }
}
struct Root(PathBuf);
impl InstructionRootResolver for Root {
    fn root<'a>(&'a self, _: &'a Path, _: &'a Cancellation) -> CoreFuture<'a, PathBuf> {
        Box::pin(async { self.0.clone() })
    }
}
struct FailedPublication(SessionError);
impl OrderedEventObserver for FailedPublication {
    fn observe<'a>(&'a self, _: &'a CoreEvent) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async { Err(self.0.clone()) })
    }
}

#[tokio::test]
async fn downstream_failure_preserves_error_after_registration_prepares_then_cleans_actual_target()
{
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49337, installed.path()).unwrap();
    let home = root.path().join("actor");
    let cwd = root.path().join("project");
    std::fs::create_dir(&home).unwrap();
    std::fs::create_dir(&cwd).unwrap();
    let target = cwd.join("AGENTS.md");
    std::fs::write(&target, b"original").unwrap();
    let core = Arc::new(SessionEngine::new(
        EngineOptions::default(),
        Arc::new(SessionJournal::new(
            paths.clone(),
            None,
            JournalOptions::default(),
        )),
        Arc::new(Unused),
        Arc::new(Unused),
        Arc::new(Unused),
        CoreEventBus::new(64).unwrap(),
    ));
    let config = Arc::new(ConfigStore::new(paths.clone(), Config::default()).unwrap());
    let warnings = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = warnings.clone();
    let rules = InstructionRules::new(
        paths.clone(),
        home.clone(),
        vec![],
        root.path().to_path_buf(),
        config,
        core.clone(),
        Arc::new(Root(cwd.clone())),
        Arc::new(move |message| captured.lock().unwrap().push(message)),
    )
    .unwrap();
    let cancel = Cancellation::default();
    // Enable before registration: the actual Registered callback must create
    // the target even if the downstream publication returns an error.
    rules.change("enable", &cancel).await.unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"original");
    let at = Timestamp::from_unix(1791158400, 0).unwrap();
    let binding = core
        .register(
            RegisterRequest {
                message: proto::Message {
                    provider: "copilot".into(),
                    pid: 3011,
                    cwd: cwd.to_string_lossy().into_owned(),
                    home_dir: home.to_string_lossy().into_owned(),
                    ..Default::default()
                },
                spawn_proof: None,
            },
            WrapperConnectionId(3011),
            at,
        )
        .await
        .unwrap()
        .binding;
    // The WebSocket registration barrier invokes this before ACK. Event
    // publication is a later phase and must not own a second injection sweep.
    rules.registered(&cancel).await;
    let tasks = HubTaskOwner::new(tokio::runtime::Handle::current());
    let original_error = SessionError::Transport("synthetic board publication failed".into());
    let observer = InstructionObserver::new(
        Arc::new(FailedPublication(original_error.clone())),
        tasks.handle(),
        cancel,
    );
    observer.bind(Arc::downgrade(&rules)).unwrap();
    assert_eq!(
        observer.observe(&CoreEvent::Registered(binding)).await,
        Err(original_error.clone())
    );
    let body = std::fs::read(&target).unwrap();
    assert!(
        body.windows(b"<!-- many-ai-cli:approval-rules -->".len())
            .any(|v| v == b"<!-- many-ai-cli:approval-rules -->")
    );
    let journal = paths.root().join("approval-rule-targets.json");
    assert!(journal.exists());

    core.disconnected(binding, at).unwrap();
    assert_eq!(
        observer
            .observe(&CoreEvent::Ended {
                binding,
                end: SessionEnd {
                    declared_state: "exited".into(),
                    exit_code: 0,
                    reason: "complete".into()
                },
            })
            .await,
        Err(original_error)
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"original");
    assert!(!journal.exists());
    assert!(warnings.lock().unwrap().is_empty());
    tasks.stop_requests();
    tasks.drain_requests().await;
    tasks.stop_effects().unwrap();
    tasks.drain_effects().await;
}
