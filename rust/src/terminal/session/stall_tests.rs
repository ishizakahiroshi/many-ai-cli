//! "No response" clock (`transcript_grew_at`): growth is pushed to the UI at
//! most every ten seconds, sub-agent log activity counts as progress, and a new
//! turn restarts the clock. Synthetic values only.
use super::observer_tests::{Io, now, register};
use super::*;
use crate::{
    config::RuntimePaths,
    terminal::{events::CoreEventBus, journal::JournalOptions},
};

fn fixture() -> (tempfile::TempDir, SessionEngine) {
    let root = tempfile::tempdir().unwrap();
    let runtime = root.path().join("trial");
    std::fs::create_dir(&runtime).unwrap();
    let paths = RuntimePaths::trial(&runtime, 49689, &root.path().join("installed")).unwrap();
    let core = SessionEngine::new(
        EngineOptions {
            // The clock under test must not be disturbed by running -> standby.
            idle_after: Duration::from_secs(3600),
            ..Default::default()
        },
        Arc::new(SessionJournal::new(paths, None, JournalOptions::default())),
        Arc::new(Io),
        Arc::new(Io),
        Arc::new(Io),
        CoreEventBus::new(64).unwrap(),
    );
    // One live browser: Broadcast frames are routed to it as SendUiBestEffort.
    let ui = UiBinding {
        connection: UiConnectionId(3100),
        auth_epoch: core.auth_epoch(),
    };
    core.attach_ui(ui, None, None).unwrap();
    core.finish_ui_priming(ui).unwrap();
    (root, core)
}
fn t(seconds: u64) -> Timestamp {
    now() + Duration::from_secs(seconds)
}
fn stamp(seconds: u64) -> String {
    proto::time::format_rfc3339(t(seconds)).unwrap()
}
fn output(core: &SessionEngine, binding: SessionBinding, seconds: u64) -> CoreEffects {
    core.observe_output(
        binding,
        OutputChunk {
            bytes: b"synthetic output".to_vec(),
            total_pty_bytes: 0,
        },
        t(seconds),
    )
    .unwrap()
}
fn transcript(
    core: &SessionEngine,
    binding: SessionBinding,
    seconds: u64,
    offset: i64,
) -> CoreEffects {
    core.apply_observation(
        binding,
        SessionObservation::Transcript {
            path: "synthetic-session.jsonl".into(),
            agent_session_id: String::new(),
            safe_offset: offset,
            grew_at: stamp(seconds),
        },
        t(seconds),
    )
    .unwrap()
}
fn subagent(
    core: &SessionEngine,
    binding: SessionBinding,
    latest: u64,
    seconds: u64,
) -> CoreEffects {
    core.apply_observation(
        binding,
        SessionObservation::SubagentActivity { latest: t(latest) },
        t(seconds),
    )
    .unwrap()
}
fn grew(core: &SessionEngine, binding: SessionBinding) -> String {
    core.details(binding.session)
        .unwrap()
        .snapshot
        .transcript_grew_at
}
/// The `transcript_grew_at` values carried by session_update frames.
fn sent(effects: &CoreEffects) -> Vec<String> {
    effects
        .0
        .iter()
        .filter_map(|effect| match effect {
            CoreEffect::SendUiBestEffort { message: m, .. }
                if m.r#type == "session_update" && !m.transcript_grew_at.is_empty() =>
            {
                Some(m.transcript_grew_at.clone())
            }
            _ => None,
        })
        .collect()
}
fn handoff_events(effects: &CoreEffects) -> usize {
    effects
        .0
        .iter()
        .filter(|effect| {
            matches!(
                effect,
                CoreEffect::Notify(CoreEvent::TranscriptChanged { .. })
            )
        })
        .count()
}

#[tokio::test]
async fn growth_after_a_quiet_period_is_pushed_without_a_state_change() {
    let (_root, core) = fixture();
    let binding = register(&core, 3101).await;
    output(&core, binding, 0);
    let effects = transcript(&core, binding, 20, 100);
    assert_eq!(sent(&effects), vec![stamp(20)]);
    assert_eq!(grew(&core, binding), stamp(20));
}

#[tokio::test]
async fn growth_inside_the_interval_is_delivered_by_the_tick_at_ten_seconds() {
    let (_root, core) = fixture();
    let binding = register(&core, 3102).await;
    output(&core, binding, 0);
    // Held back (sent at turn start, 1 s ago) but kept in the snapshot.
    assert!(sent(&transcript(&core, binding, 1, 100)).is_empty());
    assert_eq!(grew(&core, binding), stamp(1));
    assert!(sent(&core.evaluate_idle(t(9))).is_empty());
    assert_eq!(sent(&core.evaluate_idle(t(10))), vec![stamp(1)]);
    // Delivered once; a silent transcript sends nothing afterwards.
    assert!(sent(&core.evaluate_idle(t(11))).is_empty());
    assert!(sent(&core.evaluate_idle(t(600))).is_empty());
}

#[tokio::test]
async fn nothing_is_sent_while_nothing_grows() {
    let (_root, core) = fixture();
    let binding = register(&core, 3103).await;
    output(&core, binding, 0);
    for seconds in [1, 5, 10, 30, 120, 599, 601, 1200] {
        assert!(
            sent(&core.evaluate_idle(t(seconds))).is_empty(),
            "{seconds}"
        );
    }
    assert_eq!(grew(&core, binding), stamp(0));
}

#[tokio::test]
async fn subagent_log_activity_counts_as_progress() {
    let (_root, core) = fixture();
    let binding = register(&core, 3104).await;
    output(&core, binding, 0);
    // The parent transcript never grows; only the sub-agent folder does.
    let effects = subagent(&core, binding, 100, 101);
    assert_eq!(grew(&core, binding), stamp(101));
    assert_eq!(sent(&effects), vec![stamp(101)]);
    // It is not a transcript change: handoff must not get an extra record.
    assert_eq!(handoff_events(&effects), 0);
    let effects = subagent(&core, binding, 250, 251);
    assert_eq!(grew(&core, binding), stamp(251));
    assert_eq!(handoff_events(&effects), 0);
}

#[tokio::test]
async fn unchanged_subagent_folder_does_not_advance_the_clock() {
    let (_root, core) = fixture();
    let binding = register(&core, 3105).await;
    output(&core, binding, 0);
    subagent(&core, binding, 100, 101);
    assert_eq!(grew(&core, binding), stamp(101));
    // 600 s later the folder is untouched (same or older mtime): still a stall.
    let effects = subagent(&core, binding, 100, 701);
    assert!(sent(&effects).is_empty());
    let effects = subagent(&core, binding, 90, 702);
    assert!(sent(&effects).is_empty());
    assert_eq!(grew(&core, binding), stamp(101));
}

#[tokio::test]
async fn subagent_activity_is_ignored_unless_the_session_is_running() {
    let (_root, core) = fixture();
    let binding = register(&core, 3106).await;
    assert_eq!(
        core.details(binding.session).unwrap().snapshot.state,
        "standby"
    );
    let effects = subagent(&core, binding, 5, 6);
    assert!(sent(&effects).is_empty());
    assert_eq!(grew(&core, binding), "");
}

#[tokio::test]
async fn entering_running_from_standby_restarts_the_clock_and_sends_it() {
    let (_root, core) = fixture();
    let binding = register(&core, 3107).await;
    {
        // A value left over from a long idle spell.
        let mut state = lock(&core.state);
        state.session(binding).unwrap().snapshot.transcript_grew_at = stamp(0);
    }
    let effects = output(&core, binding, 5000);
    assert_eq!(grew(&core, binding), stamp(5000));
    assert_eq!(sent(&effects), vec![stamp(5000)]);
}

#[tokio::test]
async fn output_inside_a_running_turn_does_not_touch_the_clock() {
    let (_root, core) = fixture();
    let binding = register(&core, 3108).await;
    output(&core, binding, 0);
    let effects = output(&core, binding, 300);
    assert_eq!(grew(&core, binding), stamp(0));
    assert!(sent(&effects).is_empty());
}

#[tokio::test]
async fn a_new_turn_forgets_the_previous_turns_subagent_mark() {
    let (_root, core) = fixture();
    let binding = register(&core, 3109).await;
    output(&core, binding, 0);
    subagent(&core, binding, 500, 501);
    assert_eq!(grew(&core, binding), stamp(501));
    // running -> standby (idle_after is 1 h), then a new turn.
    core.evaluate_idle(t(5000));
    assert_eq!(
        core.details(binding.session).unwrap().snapshot.state,
        "standby"
    );
    output(&core, binding, 6000);
    assert_eq!(grew(&core, binding), stamp(6000));
    // The mark was reset: a lower mtime is "new" again, not swallowed by the old mark.
    subagent(&core, binding, 400, 6020);
    assert_eq!(grew(&core, binding), stamp(6020));
}

#[tokio::test]
async fn leaving_waiting_for_running_restarts_the_clock_in_the_same_frame() {
    let (_root, core) = fixture();
    let binding = register(&core, 3110).await;
    output(&core, binding, 0);
    let update = {
        // Waiting on a question for a long time; the record is gone, so the
        // refresh moves the session back to running.
        let mut state = lock(&core.state);
        let s = state.session(binding).unwrap();
        s.snapshot.state = "waiting".into();
        s.snapshot.activity.awaiting_user = true;
        s.snapshot.activity.awaiting_approval = true;
        s.refresh_approval_activity(t(4000)).unwrap()
    };
    assert_eq!(update.state, "running");
    assert_eq!(update.transcript_grew_at, stamp(4000));
    assert_eq!(grew(&core, binding), stamp(4000));
}
