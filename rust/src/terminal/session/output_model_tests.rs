use super::*;
use crate::{
    config::RuntimePaths,
    terminal::{events::CoreEventBus, journal::JournalOptions},
};
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
        Box::pin(async { SpawnWaitOutcome::Failed("synthetic no provider".into()) })
    }
}
async fn fixture(
    provider: &str,
    model: &str,
) -> (tempfile::TempDir, SessionEngine, SessionBinding, Timestamp) {
    let root = tempfile::tempdir().unwrap();
    let runtime = root.path().join("trial");
    std::fs::create_dir(&runtime).unwrap();
    let paths = RuntimePaths::trial(&runtime, 49682, &root.path().join("installed")).unwrap();
    let engine = SessionEngine::new(
        EngineOptions {
            model_route: Arc::new(|provider, model| format!("{provider}:{model}")),
            ..Default::default()
        },
        Arc::new(SessionJournal::new(paths, None, JournalOptions::default())),
        Arc::new(Io),
        Arc::new(Io),
        Arc::new(Io),
        CoreEventBus::new(16).unwrap(),
    );
    let at = Timestamp::from_unix(1791158400, 0).unwrap();
    let binding = engine
        .register(
            RegisterRequest {
                message: proto::Message {
                    provider: provider.into(),
                    model: model.into(),
                    effort: "medium".into(),
                    cwd: runtime.to_string_lossy().into_owned(),
                    pid: 9,
                    cols: 120,
                    rows: 30,
                    ..Default::default()
                },
                spawn_proof: None,
            },
            WrapperConnectionId(1),
            at,
        )
        .await
        .unwrap()
        .binding;
    (root, engine, binding, at)
}
fn output(
    engine: &SessionEngine,
    binding: SessionBinding,
    data: Vec<u8>,
    at: Timestamp,
) -> CoreEffects {
    engine
        .observe_output(
            binding,
            OutputChunk {
                bytes: data,
                total_pty_bytes: 0,
            },
            at,
        )
        .unwrap()
}
#[tokio::test]
async fn explicit_model_banner_cannot_overwrite_and_changes_preserve_blank_effort() {
    let (_root, engine, binding, at) = fixture("claude", "configured").await;
    output(
        &engine,
        binding,
        b"Claude Code v2.1.246\r\nOpus 5 with high effort\r\n".to_vec(),
        at,
    );
    assert_eq!(
        engine.details(binding.session).unwrap().snapshot.model,
        "configured"
    );
    output(
        &engine,
        binding,
        b"Set model to Sonnet 5 with low effort\r\n".to_vec(),
        at,
    );
    let got = engine.details(binding.session).unwrap().snapshot;
    assert_eq!(
        (got.model.as_str(), got.effort.as_str(), got.route.as_str()),
        ("Sonnet 5", "low", "claude:Sonnet 5")
    );
    output(&engine, binding, b"Set model to Opus 6\r\n".to_vec(), at);
    let got = engine.details(binding.session).unwrap().snapshot;
    assert_eq!((got.model.as_str(), got.effort.as_str()), ("Opus 6", "low"));
    let oldroute = got.route;
    output(
        &engine,
        binding,
        b"Set model to Opus 6 with high effort\r\n".to_vec(),
        at,
    );
    let got = engine.details(binding.session).unwrap().snapshot;
    assert_eq!(
        (got.model.as_str(), got.effort.as_str()),
        ("Opus 6", "high")
    );
    assert_eq!(got.route, oldroute);
}
#[tokio::test]
async fn banner_budget_is_raw_bytes_inclusive_and_exhaustion_persists() {
    let max = crate::application::output_model::INITIAL_SCAN_MAX_BYTES;
    let banner = b"model: gpt-5.3 /model to change\r\n";
    for (over, want) in [(false, "gpt-5.3"), (true, "")] {
        let (_root, engine, binding, at) = fixture("codex", "").await;
        output(
            &engine,
            binding,
            vec![0; max - banner.len() + usize::from(over)],
            at,
        );
        output(&engine, binding, banner.to_vec(), at);
        assert_eq!(
            engine.details(binding.session).unwrap().snapshot.model,
            want
        );
        if over {
            output(&engine, binding, banner.to_vec(), at);
            assert_eq!(engine.details(binding.session).unwrap().snapshot.model, "");
            output(&engine, binding, b"Model changed to gpt-6\r\n".to_vec(), at);
            assert_eq!(
                engine.details(binding.session).unwrap().snapshot.model,
                "gpt-6"
            );
        }
    }
}
#[tokio::test]
async fn change_wins_over_initial_banner_in_same_chunk_and_stale_binding_has_no_update() {
    let (_root, engine, binding, at) = fixture("claude", "").await;
    output(
        &engine,
        binding,
        b"Claude Code v3.0\r\nInitial with low effort\r\nSet model to Final with high effort\r\n"
            .to_vec(),
        at,
    );
    let got = engine.details(binding.session).unwrap().snapshot;
    assert_eq!((got.model.as_str(), got.effort.as_str()), ("Final", "high"));
    let mut stale = binding;
    stale.wrapper = WrapperConnectionId(2);
    assert!(matches!(
        engine.observe_output(
            stale,
            OutputChunk {
                bytes: b"Set model to Stale".to_vec(),
                total_pty_bytes: 0
            },
            at
        ),
        Err(SessionError::StaleBinding)
    ));
    assert_eq!(
        engine.details(binding.session).unwrap().snapshot.model,
        "Final"
    );
}

#[tokio::test]
async fn route_callback_can_read_core_and_older_output_cannot_overwrite_newer_model() {
    let (_root, mut engine, binding, at) = fixture("claude", "").await;
    let held = Arc::new(Mutex::new(std::sync::Weak::<SessionEngine>::new()));
    let callback_held = held.clone();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let callback_calls = calls.clone();
    engine.options.model_route = Arc::new(move |_, model| {
        let core = callback_held.lock().unwrap().upgrade().unwrap();
        assert!(
            core.state.try_lock().is_ok(),
            "route callback must not hold core lock"
        );
        assert!(core.details(binding.session).is_some());
        if callback_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            output(&core, binding, b"Set model to Newer\r\n".to_vec(), at);
        }
        format!("route:{model}")
    });
    let core = Arc::new(engine);
    *held.lock().unwrap() = Arc::downgrade(&core);
    output(&core, binding, b"Set model to Older\r\n".to_vec(), at);
    let snapshot = core.details(binding.session).unwrap().snapshot;
    assert_eq!(
        (snapshot.model.as_str(), snapshot.route.as_str()),
        ("Newer", "route:Newer")
    );
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
}

#[tokio::test]
async fn later_non_output_model_observation_invalidates_inflight_detection() {
    let (_root, mut engine, binding, at) = fixture("claude", "").await;
    let owner = Arc::new(Mutex::new(std::sync::Weak::<SessionEngine>::new()));
    let callback_owner = owner.clone();
    engine.options.model_route = Arc::new(move |_, _| {
        let core = callback_owner.lock().unwrap().upgrade().unwrap();
        core.apply_observation(
            binding,
            SessionObservation::Model {
                model: "TranscriptNew".into(),
                effort: "high".into(),
            },
            at,
        )
        .unwrap();
        "older-route".into()
    });
    let core = Arc::new(engine);
    *owner.lock().unwrap() = Arc::downgrade(&core);
    output(
        &core,
        binding,
        b"Set model to Older with low effort\r\n".to_vec(),
        at,
    );
    let snapshot = core.details(binding.session).unwrap().snapshot;
    assert_eq!(
        (
            snapshot.model.as_str(),
            snapshot.effort.as_str(),
            snapshot.route.as_str()
        ),
        ("TranscriptNew", "high", "")
    );
}
