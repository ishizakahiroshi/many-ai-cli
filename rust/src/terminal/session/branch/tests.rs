use super::*;
use crate::{
    config::RuntimePaths,
    terminal::{events::CoreEventBus, journal::JournalOptions},
};

#[derive(Default)]
struct Sink {
    messages: Mutex<Vec<(UiBinding, proto::Message)>>,
}
impl CoreEffectSink for Sink {
    fn apply<'a>(&'a self, effects: CoreEffects) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
        Box::pin(async move {
            for effect in effects.0 {
                match effect {
                    CoreEffect::SendUi { binding, message }
                    | CoreEffect::SendUiBestEffort { binding, message } => {
                        lock(&self.messages).push((binding, message));
                    }
                    CoreEffect::Broadcast(_) | CoreEffect::BroadcastGitTurn(_) => {
                        panic!("branch effects must pass through the core UI route")
                    }
                    _ => {}
                }
            }
            Ok(())
        })
    }
}
impl Sink {
    fn take(&self) -> Vec<(UiBinding, proto::Message)> {
        std::mem::take(&mut *lock(&self.messages))
    }
}
struct NoIo;
impl WrapperTransport for NoIo {
    fn send<'a>(
        &'a self,
        _: SessionBinding,
        _: proto::Message,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async { panic!("branch refresh must not write to a wrapper") })
    }
}
impl WrappedSessionSpawner for NoIo {
    fn spawn_and_wait<'a>(
        &'a self,
        _: WrappedSpawnSpec,
        _: Duration,
        _: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome> {
        Box::pin(async { panic!("branch fixture must not launch a provider") })
    }
}
struct Fixture {
    core: SessionEngine,
    sink: Arc<Sink>,
    lookup: Arc<Mutex<(Vec<String>, String)>>,
    _root: tempfile::TempDir,
}
fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let runtime = root.path().join("runtime");
    std::fs::create_dir(&runtime).unwrap();
    let paths = RuntimePaths::trial(&runtime, 49461, &root.path().join("installed")).unwrap();
    let sink = Arc::new(Sink::default());
    let lookup = Arc::new(Mutex::new((Vec::new(), String::new())));
    let callback = lookup.clone();
    let core = SessionEngine::new(
        EngineOptions {
            branch_lookup: Arc::new(move |cwd| {
                let mut lookup = lock(&callback);
                lookup.0.push(cwd);
                let branch = lookup.1.clone();
                Box::pin(async move { branch })
            }),
            ..Default::default()
        },
        Arc::new(SessionJournal::new(paths, None, JournalOptions::default())),
        Arc::new(NoIo),
        sink.clone(),
        Arc::new(NoIo),
        CoreEventBus::new(64).unwrap(),
    );
    Fixture {
        core,
        sink,
        lookup,
        _root: root,
    }
}
fn at(milliseconds: u64) -> Timestamp {
    Timestamp::from_unix(1_791_158_400, 0)
        .unwrap()
        .checked_add(Duration::from_millis(milliseconds))
        .unwrap()
}
async fn register(f: &Fixture, pid: i64, cwd: &str, probe: bool) -> SessionBinding {
    let registered = f
        .core
        .register(
            RegisterRequest {
                message: proto::Message {
                    provider: "copilot".into(),
                    display_name: "Initial display".into(),
                    cwd: cwd.into(),
                    pid,
                    label: "fixture label".into(),
                    model: "fixture model".into(),
                    route: "fixture route".into(),
                    provider_revision: "must-not-leak".into(),
                    subscription_id: "must-not-leak".into(),
                    usage_probe: probe,
                    cols: 120,
                    rows: 30,
                    ..Default::default()
                },
                spawn_proof: None,
            },
            WrapperConnectionId(pid as u64),
            at(0),
        )
        .await
        .unwrap();
    let binding = registered.binding;
    f.sink.apply(registered.after_registered).await.unwrap();
    f.sink.take();
    binding
}
async fn attach(f: &Fixture, connection: u64, live: bool) -> UiBinding {
    let ui = UiBinding {
        connection: UiConnectionId(connection),
        auth_epoch: f.core.auth_epoch(),
    };
    f.core.attach_ui(ui, None, None).unwrap();
    if live {
        f.sink
            .apply(f.core.finish_ui_priming(ui).unwrap())
            .await
            .unwrap();
    }
    ui
}
fn observation(
    branch: &str,
    changes: (i64, i64, i64),
    project: Option<&str>,
) -> SessionObservation {
    SessionObservation::Branch {
        branch: branch.into(),
        git_root: project.filter(|p| !p.is_empty()).map(PathBuf::from),
        changes,
        project_id: project.map(str::to_owned),
    }
}
async fn refresh(
    f: &Fixture,
    cwd: &str,
    ids: &[LiveSessionId],
    observation: SessionObservation,
) -> Vec<(UiBinding, proto::Message)> {
    let effects = f.core.apply_branch_refresh(cwd, ids, observation).unwrap();
    assert!(effects.0.iter().all(|effect| matches!(
        effect,
        CoreEffect::SendUi { .. } | CoreEffect::SendUiBestEffort { .. }
    )));
    f.sink.apply(effects).await.unwrap();
    f.sink.take()
}
async fn reattach(
    f: &Fixture,
    old: SessionBinding,
    pid: i64,
    cwd: &str,
    now: Timestamp,
) -> SessionBinding {
    let started = f.core.snapshot(old.session).unwrap().started_at;
    let receipt = f
        .core
        .reattach(
            ReattachRequest {
                restored_metadata: None,
                message: proto::Message {
                    session_id: old.session.0,
                    provider: "copilot".into(),
                    display_name: "Replacement display".into(),
                    cwd: cwd.into(),
                    pid,
                    model: "replacement model".into(),
                    route: "replacement route".into(),
                    started_at: started,
                    cols: 120,
                    rows: 30,
                    ..Default::default()
                },
            },
            WrapperConnectionId(10_000 + pid as u64),
            now,
        )
        .await
        .unwrap();
    let binding = receipt.binding;
    f.sink.apply(receipt.after_reattached).await.unwrap();
    f.sink.take();
    binding
}

#[tokio::test]
async fn first_tick_is_due_and_enqueue_time_advances_even_for_terminal_sessions() {
    let f = fixture();
    let first = register(&f, 1, "first-cwd", false).await;
    let terminal = register(&f, 2, "terminal-cwd", false).await;
    drop(
        f.core
            .observe_end(
                terminal,
                SessionEnd {
                    declared_state: "completed".into(),
                    exit_code: 0,
                    reason: String::new(),
                },
                at(0),
            )
            .unwrap(),
    );
    let expected = vec![
        (first.session, "first-cwd".into()),
        (terminal.session, "terminal-cwd".into()),
    ];
    assert_eq!(f.core.branch_refresh_requests(at(0)), expected);
    // No result has completed. Repeated ticks still use last admission time.
    assert!(f.core.branch_refresh_requests(at(0)).is_empty());
    assert!(f.core.branch_refresh_requests(at(1_999)).is_empty());
    assert_eq!(f.core.branch_refresh_requests(at(2_000)), expected);
    assert!(f.core.branch_refresh_requests(at(3_999)).is_empty());
    assert_eq!(f.core.branch_refresh_requests(at(4_000)), expected);
    assert_eq!(
        f.core.snapshot(terminal.session).unwrap().state,
        "completed"
    );
}

#[tokio::test]
async fn registration_and_cold_reattach_lookup_raw_cwd_but_only_reattach_delays_the_first_tick() {
    let f = fixture();
    lock(&f.lookup).1 = "initial".into();
    let first = register(&f, 1, "  raw-cwd  ", false).await;
    assert_eq!(f.core.snapshot(first.session).unwrap().branch, "initial");
    assert_eq!(
        f.core.branch_refresh_requests(at(0)),
        vec![(first.session, "  raw-cwd  ".into())]
    );
    let cold = f
        .core
        .reattach(
            ReattachRequest {
                restored_metadata: None,
                message: proto::Message {
                    session_id: 99,
                    provider: "copilot".into(),
                    cwd: "  cold-cwd  ".into(),
                    pid: 99,
                    cols: 120,
                    rows: 30,
                    ..Default::default()
                },
            },
            WrapperConnectionId(99),
            at(500),
        )
        .await
        .unwrap();
    let cold_id = cold.binding.session;
    f.sink.apply(cold.after_reattached).await.unwrap();
    assert_eq!(lock(&f.lookup).0, vec!["  raw-cwd  ", "  cold-cwd  "]);
    assert!(f.core.branch_project_needed("  cold-cwd  ", &[cold_id]));
    assert!(
        !lock(&f.core.state).sessions[&cold_id]
            .branch_refresh
            .checked
    );
    assert!(f.core.branch_refresh_requests(at(500)).is_empty());
    assert_eq!(
        f.core.branch_refresh_requests(at(2_499)),
        vec![(first.session, "  raw-cwd  ".into())]
    );
    assert_eq!(
        f.core.branch_refresh_requests(at(2_500)),
        vec![(cold_id, "  cold-cwd  ".into())]
    );
}

#[tokio::test]
async fn unresolved_project_retries_and_preserves_identity_until_resolved_empty_latches() {
    let f = fixture();
    let binding = register(&f, 1, "cwd", false).await;
    attach(&f, 1, true).await;
    // A retained identity is not proof that the lookup has been answered.
    lock(&f.core.state)
        .sessions
        .get_mut(&binding.session)
        .unwrap()
        .snapshot
        .project_id = "retained-root".into();
    assert!(f.core.branch_project_needed("cwd", &[binding.session]));
    let first = refresh(
        &f,
        "cwd",
        &[binding.session],
        observation("", (0, 0, 0), None),
    )
    .await;
    assert_eq!(
        first.len(),
        1,
        "the first zero stat is still an observation"
    );
    assert!(first[0].1.git_checked);
    assert_eq!(first[0].1.project_id, "retained-root");
    assert!(f.core.branch_project_needed("cwd", &[binding.session]));
    assert!(
        refresh(
            &f,
            "cwd",
            &[binding.session],
            observation("", (0, 0, 0), None)
        )
        .await
        .is_empty()
    );
    let resolved = refresh(
        &f,
        "cwd",
        &[binding.session],
        observation("", (0, 0, 0), Some("")),
    )
    .await;
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].1.project_id, "");
    assert!(!f.core.branch_project_needed("cwd", &[binding.session]));
    assert!(
        refresh(
            &f,
            "cwd",
            &[binding.session],
            observation("", (0, 0, 0), Some(""))
        )
        .await
        .is_empty()
    );
    assert!(
        refresh(
            &f,
            "cwd",
            &[binding.session],
            observation("", (0, 0, 0), Some("must-not-replace-latch"))
        )
        .await
        .is_empty()
    );
    assert_eq!(f.core.snapshot(binding.session).unwrap().project_id, "");
}

#[tokio::test]
async fn resolving_empty_identity_emits_once_even_when_every_visible_value_is_unchanged() {
    let f = fixture();
    let binding = register(&f, 1, "nonrepo", false).await;
    attach(&f, 1, true).await;
    let first = refresh(
        &f,
        "nonrepo",
        &[binding.session],
        observation("", (0, 0, 0), None),
    )
    .await;
    assert_eq!(first.len(), 1);
    let first_wire = serde_json::to_value(&first[0].1).unwrap();
    let resolved = refresh(
        &f,
        "nonrepo",
        &[binding.session],
        observation("", (0, 0, 0), Some("")),
    )
    .await;
    assert_eq!(
        resolved.len(),
        1,
        "an answered empty lookup must advance the latch"
    );
    assert_eq!(serde_json::to_value(&resolved[0].1).unwrap(), first_wire);
    assert!(!f.core.branch_project_needed("nonrepo", &[binding.session]));
    assert!(
        refresh(
            &f,
            "nonrepo",
            &[binding.session],
            observation("", (0, 0, 0), Some("")),
        )
        .await
        .is_empty()
    );
}

#[tokio::test]
async fn project_only_resolution_emits_once_and_mixed_groups_keep_checked_identity() {
    let f = fixture();
    let checked = register(&f, 1, "cwd", false).await;
    let unchecked = register(&f, 2, "cwd", false).await;
    attach(&f, 1, true).await;
    assert_eq!(
        refresh(
            &f,
            "cwd",
            &[checked.session],
            observation("main", (1, 2, 3), Some("old-root"))
        )
        .await
        .len(),
        1
    );
    assert_eq!(
        refresh(
            &f,
            "cwd",
            &[unchecked.session],
            observation("main", (1, 2, 3), None)
        )
        .await
        .len(),
        1
    );
    let ids = [checked.session, unchecked.session];
    assert!(f.core.branch_project_needed("cwd", &ids));
    let project_only = refresh(
        &f,
        "cwd",
        &ids,
        observation("main", (1, 2, 3), Some("new-root")),
    )
    .await;
    assert_eq!(project_only.len(), 1);
    assert_eq!(project_only[0].1.session_id, unchecked.session.0);
    assert_eq!(project_only[0].1.project_id, "new-root");
    assert_eq!(
        f.core.snapshot(checked.session).unwrap().project_id,
        "old-root"
    );
    assert!(!f.core.branch_project_needed("cwd", &ids));
    assert!(
        refresh(&f, "cwd", &ids, observation("main", (1, 2, 3), None))
            .await
            .is_empty()
    );
    let failure = refresh(&f, "cwd", &ids, observation("", (0, 0, 0), None)).await;
    assert_eq!(
        failure.len(),
        2,
        "later failed helpers clear branch and counts"
    );
    for (_, message) in failure {
        assert!(message.branch.is_empty());
        assert_eq!(
            (message.git_files, message.git_added, message.git_deleted),
            (0, 0, 0)
        );
        assert!(message.git_checked);
    }
    assert_eq!(
        f.core.snapshot(checked.session).unwrap().project_id,
        "old-root"
    );
}

#[tokio::test]
async fn warm_reattach_keeps_cached_stats_and_accepts_an_old_same_cwd_result_on_the_new_wrapper() {
    let f = fixture();
    let old = register(&f, 1, "cwd", false).await;
    attach(&f, 1, true).await;
    refresh(
        &f,
        "cwd",
        &[old.session],
        observation("main", (2, 3, 4), Some("root")),
    )
    .await;
    f.core.branch_refresh_requests(at(0));
    lock(&f.lookup).1 = "reattached".into();
    let replacement = reattach(&f, old, 1, "cwd", at(1_000)).await;
    assert_eq!(replacement.session, old.session);
    assert_ne!(replacement.wrapper, old.wrapper);
    assert!(!f.core.is_current(old));
    assert_eq!(f.core.snapshot(old.session).unwrap().branch, "reattached");
    assert_eq!(f.core.snapshot(old.session).unwrap().project_id, "root");
    assert!(!f.core.branch_project_needed("cwd", &[old.session]));
    {
        let state = lock(&f.core.state);
        let cached = &state.sessions[&old.session].branch_refresh;
        assert!(cached.checked);
        assert_eq!(cached.changes, (2, 3, 4));
        assert_eq!(cached.checked_at, Some(at(1_000)));
    }
    assert!(f.core.branch_refresh_requests(at(2_999)).is_empty());
    assert_eq!(
        f.core.branch_refresh_requests(at(3_000)),
        vec![(old.session, "cwd".into())]
    );
    let late = refresh(
        &f,
        "cwd",
        &[old.session],
        observation("older-worker", (2, 3, 4), None),
    )
    .await;
    assert_eq!(late.len(), 1);
    assert_eq!(late[0].1.branch, "older-worker");
    assert_eq!(late[0].1.display_name, "Replacement display");
    assert_eq!(late[0].1.model, "replacement model");
    assert_eq!(late[0].1.route, "replacement route");
}

#[tokio::test]
async fn exact_cwd_guard_skips_changed_deleted_and_whitespace_ids_but_preserves_warm_project_cache()
{
    let f = fixture();
    let keep = register(&f, 1, "cwd", false).await;
    let moved = register(&f, 2, "cwd", false).await;
    let deleted = register(&f, 3, "cwd", false).await;
    let whitespace = register(&f, 4, " cwd ", false).await;
    attach(&f, 1, true).await;
    refresh(
        &f,
        "cwd",
        &[moved.session],
        observation("main", (2, 3, 4), Some("old-root")),
    )
    .await;
    drop(f.core.disconnected(moved, at(500)).unwrap());
    let replacement = reattach(&f, moved, 2, "new-cwd", at(1_000)).await;
    assert_eq!(replacement.session, moved.session);
    assert_eq!(
        f.core.snapshot(moved.session).unwrap().project_id,
        "old-root"
    );
    assert!(!f.core.branch_project_needed("new-cwd", &[moved.session]));
    assert_eq!(
        lock(&f.core.state).sessions[&moved.session]
            .branch_refresh
            .changes,
        (2, 3, 4)
    );
    drop(f.core.dismiss(deleted.session, at(1_000)).unwrap());
    let stale = [
        moved.session,
        deleted.session,
        whitespace.session,
        LiveSessionId(999),
    ];
    assert!(!f.core.branch_project_needed("cwd", &stale));
    let ids = [
        keep.session,
        moved.session,
        deleted.session,
        whitespace.session,
        LiveSessionId(999),
    ];
    let messages = refresh(
        &f,
        "cwd",
        &ids,
        observation("late", (1, 1, 1), Some("fresh-root")),
    )
    .await;
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].1.session_id, keep.session.0);
    assert_eq!(f.core.snapshot(keep.session).unwrap().branch, "late");
    assert!(f.core.snapshot(moved.session).unwrap().branch.is_empty());
    assert!(f.core.snapshot(deleted.session).is_none());
    assert!(
        f.core
            .snapshot(whitespace.session)
            .unwrap()
            .branch
            .is_empty()
    );
    assert!(f.core.branch_project_needed(" cwd ", &[whitespace.session]));
}

#[tokio::test]
async fn sparse_wire_payload_contains_current_metadata_and_omits_zero_counts_and_unrelated_fields()
{
    let f = fixture();
    let binding = register(&f, 1, "cwd", false).await;
    attach(&f, 1, true).await;
    {
        let mut state = lock(&f.core.state);
        let snapshot = &mut state.sessions.get_mut(&binding.session).unwrap().snapshot;
        snapshot.first_message = "first".into();
        snapshot.last_message = "last".into();
        snapshot.last_output_at = "2026-10-04T00:00:01Z".into();
        snapshot.transcript_grew_at = "must-not-leak".into();
        snapshot.activity.workflow_active = true;
    }
    let messages = refresh(
        &f,
        "cwd",
        &[binding.session],
        observation("", (0, 0, 0), Some("")),
    )
    .await;
    assert_eq!(messages.len(), 1);
    let wire = serde_json::to_value(&messages[0].1).unwrap();
    assert_eq!(
        wire,
        serde_json::json!({
            "type": "session_update",
            "session_id": binding.session.0,
            "provider": "copilot",
            "display_name": "Initial display",
            "cwd": "cwd",
            "label": "fixture label",
            "model": "fixture model",
            "route": "fixture route",
            "state": "standby",
            "started_at": f.core.snapshot(binding.session).unwrap().started_at,
            "last_output_at": "2026-10-04T00:00:01Z",
            "first_message": "first",
            "last_message": "last",
            "git_checked": true,
            // This shared Go wire field intentionally has no omitempty tag.
            "token_statusbar": false
        })
    );
    let changed = refresh(
        &f,
        "cwd",
        &[binding.session],
        observation("feature", (5, 7, -2), None),
    )
    .await;
    assert_eq!(changed.len(), 1);
    let wire = serde_json::to_value(&changed[0].1).unwrap();
    assert_eq!(wire["branch"], "feature");
    assert_eq!(wire["git_files"], 5);
    assert_eq!(wire["git_added"], 7);
    assert_eq!(wire["git_deleted"], -2);
    assert!(
        refresh(
            &f,
            "cwd",
            &[binding.session],
            observation("feature", (5, 7, -2), None)
        )
        .await
        .is_empty()
    );
    let snapshot = serde_json::to_value(f.core.snapshot(binding.session).unwrap()).unwrap();
    for field in ["git_checked", "git_files", "git_added", "git_deleted"] {
        assert!(
            snapshot.get(field).is_none(),
            "{field} must remain private snapshot state"
        );
    }
}

#[tokio::test]
async fn priming_queues_branch_frames_in_order_and_hides_probes_without_skipping_observation() {
    let f = fixture();
    let visible = register(&f, 1, "cwd", false).await;
    let probe = register(&f, 2, "cwd", true).await;
    let ui = attach(&f, 1, false).await;
    let ids = [visible.session, probe.session];
    assert!(
        refresh(
            &f,
            "cwd",
            &ids,
            observation("main", (0, 0, 0), Some("root"))
        )
        .await
        .is_empty()
    );
    assert!(
        refresh(&f, "cwd", &ids, observation("feature", (1, 0, 0), None))
            .await
            .is_empty()
    );
    assert_eq!(f.core.snapshot(probe.session).unwrap().branch, "feature");
    assert!(!f.core.branch_project_needed("cwd", &[probe.session]));
    let batch = f.core.finish_ui_priming(ui).unwrap();
    assert_eq!(batch.0.len(), 2);
    assert!(
        batch
            .0
            .iter()
            .all(|effect| matches!(effect, CoreEffect::SendUi { .. }))
    );
    f.sink.apply(batch).await.unwrap();
    let queued = f.sink.take();
    assert_eq!(
        queued
            .iter()
            .map(|(_, message)| message.branch.as_str())
            .collect::<Vec<_>>(),
        vec!["main", "feature"]
    );
    assert!(
        queued
            .iter()
            .all(|(binding, message)| *binding == ui && message.session_id == visible.session.0)
    );
    // A frame prepared during draining must follow the earlier batch.
    assert!(
        refresh(
            &f,
            "cwd",
            &ids,
            observation("during-drain", (1, 0, 0), None)
        )
        .await
        .is_empty()
    );
    f.sink
        .apply(f.core.finish_ui_priming(ui).unwrap())
        .await
        .unwrap();
    let draining = f.sink.take();
    assert_eq!(draining.len(), 1);
    assert_eq!(draining[0].1.branch, "during-drain");
    assert!(f.core.finish_ui_priming(ui).unwrap().0.is_empty());
    let live = refresh(&f, "cwd", &ids, observation("live", (1, 0, 0), None)).await;
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].1.session_id, visible.session.0);
    assert_eq!(live[0].1.branch, "live");
}
