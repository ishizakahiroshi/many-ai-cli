use super::*;
use crate::{
    config::RuntimePaths,
    proto,
    terminal::{
        events::CoreEventBus,
        journal::{JournalOptions, SessionJournal},
        session::EngineOptions,
    },
};

struct NoIo;
impl WrapperTransport for NoIo {
    fn send<'a>(
        &'a self,
        _: SessionBinding,
        _: proto::Message,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async { panic!("probe binding fixture must not write to a wrapper") })
    }
}
impl CoreEffectSink for NoIo {
    fn apply<'a>(&'a self, _: CoreEffects) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
        Box::pin(async { panic!("probe binding fixture must not dispatch effects") })
    }
}
impl WrappedSessionSpawner for NoIo {
    fn spawn_and_wait<'a>(
        &'a self,
        _: WrappedSpawnSpec,
        _: std::time::Duration,
        _: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome> {
        Box::pin(async { panic!("probe binding fixture must not launch a provider") })
    }
}
struct Fixture {
    core: Arc<SessionEngine>,
    _root: tempfile::TempDir,
}
fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let runtime = root.path().join("runtime");
    std::fs::create_dir(&runtime).unwrap();
    let paths = RuntimePaths::trial(&runtime, 49467, &root.path().join("installed")).unwrap();
    Fixture {
        core: Arc::new(SessionEngine::new(
            EngineOptions::default(),
            Arc::new(SessionJournal::new(paths, None, JournalOptions::default())),
            Arc::new(NoIo),
            Arc::new(NoIo),
            Arc::new(NoIo),
            CoreEventBus::new(64).unwrap(),
        )),
        _root: root,
    }
}
async fn register(f: &Fixture) -> SessionBinding {
    f.core
        .register(
            RegisterRequest {
                message: proto::Message {
                    provider: "claude".into(),
                    cwd: f._root.path().to_string_lossy().into_owned(),
                    pid: 3001,
                    usage_probe: true,
                    cols: 120,
                    rows: 30,
                    ..Default::default()
                },
                spawn_proof: None,
            },
            WrapperConnectionId(1),
            Timestamp::now(),
        )
        .await
        .unwrap()
        .binding
}
async fn replace(f: &Fixture, old: SessionBinding) -> SessionBinding {
    f.core
        .reattach(
            ReattachRequest {
                restored_metadata: None,
                message: proto::Message {
                    session_id: old.session.0,
                    provider: "claude".into(),
                    cwd: f._root.path().to_string_lossy().into_owned(),
                    pid: 3001,
                    usage_probe: true,
                    cols: 120,
                    rows: 30,
                    started_at: f.core.snapshot(old.session).unwrap().started_at,
                    ..Default::default()
                },
            },
            WrapperConnectionId(2),
            Timestamp::now(),
        )
        .await
        .unwrap()
        .binding
}

#[tokio::test]
async fn replacement_after_probe_observation_is_not_dismissed() {
    let f = fixture();
    let old = register(&f).await;
    // These are the production adapter's observation and identity check.
    let observed = f.core.details(old.session).unwrap();
    assert_eq!(observed.binding, old);
    // A warm wrapper replacement can run on another runtime worker between
    // the adapter's two synchronous core calls. Force that order without sleep.
    let current = replace(&f, old).await;
    assert_eq!(current.session, old.session);
    assert_eq!(current.incarnation, old.incarnation);
    assert_ne!(current.wrapper, old.wrapper);
    let result = ProbeHooks::dismiss_observed(&f.core, &observed);
    assert!(matches!(result, Err(SessionError::StaleBinding)));
    assert!(f.core.is_current(current));
    assert_eq!(f.core.details(old.session).unwrap().binding, current);
}

#[tokio::test]
async fn current_probe_observation_dismisses_only_its_binding() {
    let f = fixture();
    let binding = register(&f).await;
    let observed = f.core.details(binding.session).unwrap();
    let effects = ProbeHooks::dismiss_observed(&f.core, &observed).unwrap();
    assert!(f.core.details(binding.session).is_none());
    assert!(effects.0.iter().any(|effect| matches!(
        effect,
        CoreEffect::CancelSession { binding: actual, reason: StopReason::Dismissed }
            if *actual == binding
    )));
}

#[tokio::test]
async fn removal_after_probe_observation_does_not_emit_duplicate_dismissal() {
    let f = fixture();
    let binding = register(&f).await;
    let observed = f.core.details(binding.session).unwrap();
    f.core.dismiss(binding.session, Timestamp::now()).unwrap();
    assert!(matches!(
        ProbeHooks::dismiss_observed(&f.core, &observed),
        Err(SessionError::StaleBinding)
    ));
}

#[tokio::test]
async fn probe_hook_rejects_already_stale_binding_and_accepts_absence() {
    let f = fixture();
    let binding = register(&f).await;
    let current = replace(&f, binding).await;
    let hooks = ProbeHooks {
        core: Arc::downgrade(&f.core),
        effects: Weak::<NoIo>::new(),
        ordinary: Weak::new(),
        trial: true,
    };
    assert!(matches!(
        hooks.dismiss(binding).await,
        Err(SessionError::StaleBinding)
    ));
    assert!(f.core.is_current(current));
    f.core.dismiss(current.session, Timestamp::now()).unwrap();
    hooks.dismiss(current).await.unwrap();
}
