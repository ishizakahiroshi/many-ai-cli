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
    let (root, engine, registration, at) = fixture_with_effort(provider, model, "medium").await;
    (root, engine, registration.binding, at)
}
async fn fixture_with_effort(
    provider: &str,
    model: &str,
    effort: &str,
) -> (tempfile::TempDir, SessionEngine, Registration, Timestamp) {
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
    let ui = UiBinding {
        connection: UiConnectionId(1),
        auth_epoch: AuthEpoch(0),
    };
    engine.attach_ui(ui, None, None).unwrap();
    assert!(engine.finish_ui_priming(ui).unwrap().0.is_empty());
    let registration = engine
        .register(
            RegisterRequest {
                message: proto::Message {
                    provider: provider.into(),
                    model: model.into(),
                    effort: effort.into(),
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
        .unwrap();
    (root, engine, registration, at)
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

fn ui_session_updates(effects: &CoreEffects) -> Vec<serde_json::Value> {
    effects
        .0
        .iter()
        .filter_map(|effect| match effect {
            CoreEffect::SendUi { message, .. } | CoreEffect::SendUiBestEffort { message, .. }
                if message.r#type == "session_update" =>
            {
                Some(serde_json::from_slice(&serde_json::to_vec(message).unwrap()).unwrap())
            }
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn detected_initial_model_emits_go_effort_update_to_live_ui() {
    let (_root, engine, binding, at) = fixture("claude", "").await;
    engine
        .apply_observation(
            binding,
            SessionObservation::Messages {
                first: "first synthetic request".into(),
                last: "latest synthetic request".into(),
            },
            at,
        )
        .unwrap();
    let updates = ui_session_updates(&output(
        &engine,
        binding,
        b"Claude Code v3.0\r\nOpus 5 with high effort\r\n".to_vec(),
        at,
    ));
    assert_eq!(updates.len(), 2, "activity then detected-model update");
    assert!(updates[0].get("effort").is_none());
    // Go d8fbf859 model_detect.go:188-202 has a deliberately narrower shape
    // than orchestration.go:sessionUpdateMessage and includes message summaries.
    assert_eq!(
        updates[1],
        serde_json::json!({
            "type": "session_update",
            "token_statusbar": false,
            "session_id": binding.session.0,
            "provider": "claude",
            "cwd": engine.snapshot(binding.session).unwrap().cwd,
            "model": "Opus 5",
            "effort": "high",
            "route": "claude:Opus 5",
            "state": "running",
            "last_output_at": timestamp(at).unwrap(),
            "first_message": "first synthetic request",
            "last_message": "latest synthetic request",
        })
    );
}

#[tokio::test]
async fn detected_effort_only_and_model_changes_emit_preserved_effort() {
    let (_root, engine, binding, at) = fixture("claude", "Opus 5").await;
    output(&engine, binding, b"ready\r\n".to_vec(), at);
    let updates = ui_session_updates(&output(
        &engine,
        binding,
        b"Set model to Opus 5 with high effort\r\n".to_vec(),
        at,
    ));
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0]["model"], "Opus 5");
    assert_eq!(updates[0]["effort"], "high");
    assert!(
        updates[0].get("route").is_none(),
        "unchanged model keeps route"
    );

    let updates = ui_session_updates(&output(
        &engine,
        binding,
        b"Set model to Sonnet 5\r\n".to_vec(),
        at,
    ));
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0]["model"], "Sonnet 5");
    assert_eq!(
        updates[0]["effort"], "high",
        "blank detection preserves effort"
    );
    assert_eq!(updates[0]["route"], "claude:Sonnet 5");

    for line in [
        b"Set model to Sonnet 5\r\n".as_slice(),
        b"Set model to Sonnet 5 with high effort\r\n".as_slice(),
    ] {
        assert!(ui_session_updates(&output(&engine, binding, line.to_vec(), at)).is_empty());
    }
}

#[tokio::test]
async fn detected_model_without_effort_omits_effort_on_the_wire() {
    let (_root, engine, registration, at) = fixture_with_effort("claude", "", "").await;
    let updates = ui_session_updates(&output(
        &engine,
        registration.binding,
        b"Claude Code v3.0\r\nOpus 5\r\n".to_vec(),
        at,
    ));
    let model_update = updates.last().unwrap();
    assert_eq!(model_update["model"], "Opus 5");
    assert!(model_update.get("effort").is_none());
}

#[tokio::test]
async fn launch_effort_stays_in_snapshot_and_out_of_generic_announcements() {
    let (_root, engine, registration, at) = fixture_with_effort("claude", "Opus 5", "medium").await;
    let binding = registration.binding;
    let registered_snapshot = registration.snapshot;
    let updates = ui_session_updates(&registration.after_registered);
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0]["model"], "Opus 5");
    assert!(updates[0].get("effort").is_none());
    // Retained persistence effects reserve the ordered lane used by reattach.
    drop(registration.after_registered);

    let reconnecting_ui = UiBinding {
        connection: UiConnectionId(2),
        auth_epoch: AuthEpoch(0),
    };
    let priming = engine.attach_ui(reconnecting_ui, None, None).unwrap();
    let snapshot: serde_json::Value =
        serde_json::from_slice(&serde_json::to_vec(&priming.sessions).unwrap()).unwrap();
    assert_eq!(snapshot[0]["effort"], "medium");
    engine.detach_ui(reconnecting_ui);

    let reattached = engine
        .reattach(
            ReattachRequest {
                restored_metadata: None,
                message: proto::Message {
                    session_id: binding.session.0,
                    provider: "claude".into(),
                    model: "Opus 5".into(),
                    effort: "medium".into(),
                    cwd: registered_snapshot.cwd,
                    started_at: registered_snapshot.started_at,
                    pid: 9,
                    cols: 120,
                    rows: 30,
                    ..Default::default()
                },
            },
            WrapperConnectionId(2),
            at,
        )
        .await
        .unwrap();
    let updates = ui_session_updates(&reattached.after_reattached);
    assert_eq!(updates.len(), 1);
    assert!(updates[0].get("effort").is_none());
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
