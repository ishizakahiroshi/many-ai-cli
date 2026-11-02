//! Compose relay publication with the actual main-program completion owners.
use super::*;
use crate::{
    orchestration::handoff::{HandoffStore, KIND_DONE, KIND_INTENT},
    proto::{self, core::*, time::Timestamp},
    routine::{model::Definition, schedule::Schedule},
};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Fixture {
    _root: tempfile::TempDir,
    composition: HubComposition,
    paths: RuntimePaths,
    binding: SessionBinding,
    run_id: String,
    summary: proto::DoneSummary,
    priming: UiBinding,
}
impl Fixture {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let mut context = fixture_context(&root).await;
        let mut config = context.config.snapshot().unwrap().config;
        config.log.enabled = true;
        context.config = Arc::new(ConfigStore::new(context.paths.clone(), config).unwrap());
        let paths = context.paths.clone();
        let project = paths.root().join("relay-project");
        std::fs::create_dir(&project).unwrap();
        let composition = HubComposition::new(context, ServeOptions::default())
            .await
            .unwrap();
        let deps = &composition.dependencies;
        let now = Timestamp::now();
        let definition = deps
            .routine_store
            .save(
                None,
                Definition {
                    name: "synthetic relay completion".into(),
                    cwd: project.to_string_lossy().into_owned(),
                    provider: "codex".into(),
                    prompt: "synthetic prompt; never launch".into(),
                    schedule: Schedule {
                        kind: "manual".into(),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                paths.root(),
                now,
            )
            .unwrap();
        let run = deps
            .routine_store
            .admit(&definition.id, "synthetic-relay-once", "manual", now)
            .unwrap()
            .run;
        // Register directly with the core; no wrapper/provider process is run.
        let binding = deps
            .core
            .register(
                RegisterRequest {
                    message: proto::Message {
                        provider: "codex".into(),
                        cwd: project.to_string_lossy().into_owned(),
                        label: run.session_label.clone(),
                        pid: 7,
                        ..Default::default()
                    },
                    spawn_proof: None,
                },
                WrapperConnectionId(991),
                now,
            )
            .await
            .unwrap()
            .binding;
        assert_eq!(
            deps.core.snapshot(binding.session).unwrap().launch_label,
            run.session_label
        );
        let priming = UiBinding {
            connection: UiConnectionId(991),
            auth_epoch: deps.core.auth_epoch(),
        };
        deps.core.attach_ui(priming, None, None).unwrap();
        let summary = proto::DoneSummary {
            session_id: binding.session.0,
            provider: "codex".into(),
            title: "synthetic relay stopped".into(),
            text: "synthetic relay stopped 次: synthetic continuation 未検証: synthetic check"
                .into(),
            kind: "relay".into(),
            at: "2026-10-25T00:00:00Z".into(),
            ..Default::default()
        };
        Self {
            _root: root,
            composition,
            paths,
            binding,
            run_id: run.id,
            summary,
            priming,
        }
    }
    fn queued_summary_count(&self) -> usize {
        self.composition
            .dependencies
            .core
            .finish_ui_priming(self.priming)
            .unwrap()
            .0
            .iter()
            .filter(|effect| {
                matches!(effect, CoreEffect::SendUi { message, .. }
                    if message.r#type == "done_summary"
                    && message.done_summary.as_ref().is_some_and(|summary| summary.kind == "relay"))
            })
            .count()
    }
    fn assert_routine_finished(&self) {
        let run = self
            .composition
            .dependencies
            .routine_store
            .run(&self.run_id)
            .unwrap()
            .unwrap();
        assert_eq!(run.status, "finished");
        assert_eq!(run.result, self.summary.text);
        assert_eq!(run.session_id, self.binding.session.0);
        let reloaded = crate::routine::store::RoutineStore::open(Arc::new(
            crate::files::safe_fs::Dir::open(self.paths.root()).unwrap(),
        ));
        assert_eq!(reloaded.run(&self.run_id).unwrap(), Some(run));
    }
}

async fn published_kinds(events: &mut dyn CoreEventSubscription) -> Vec<&'static str> {
    let cancellation = HubShutdownCancellation::default();
    let mut kinds = Vec::new();
    while let Ok(CoreEventPoll::Event(event)) =
        tokio::time::timeout(Duration::from_millis(30), events.next(&cancellation)).await
    {
        kinds.push(match event.event {
            CoreEvent::RoutineCompleted { .. } => "routine",
            CoreEvent::CompletionRecord { .. } => "record",
            CoreEvent::Completed { .. } => "completed",
            _ => "other",
        });
    }
    kinds
}

#[tokio::test]
async fn relay_real_callbacks_commit_routine_and_both_handoff_records_before_completed() {
    let fixture = Fixture::new().await;
    let deps = &fixture.composition.dependencies;
    let before =
        serde_json::to_value(deps.core.snapshot(fixture.binding.session).unwrap()).unwrap();
    let hook_calls = Arc::new(AtomicUsize::new(0));
    let calls = hook_calls.clone();
    let store = deps.routine_store.clone();
    let run_id = fixture.run_id.clone();
    deps.workers.set_completion_append_hook(Arc::new(move || {
        assert_eq!(store.run(&run_id).unwrap().unwrap().status, "finished");
        calls.fetch_add(1, Ordering::SeqCst);
    }));
    let mut events = deps.core.subscribe();
    deps.effects
        .apply(deps.core.publish_relay_done(fixture.summary.clone()))
        .await
        .unwrap();
    fixture.assert_routine_finished();
    assert_eq!(hook_calls.load(Ordering::SeqCst), 1);
    let records = HandoffStore::new(fixture.paths.clone())
        .read_session(fixture.binding.session.0)
        .unwrap();
    assert_eq!(
        records.iter().map(|r| r.kind.as_str()).collect::<Vec<_>>(),
        [KIND_DONE, KIND_INTENT]
    );
    assert_eq!(records[0].text, format!("[relay] {}", fixture.summary.text));
    assert_eq!(
        records[1].text,
        "次: synthetic continuation / 未検証: synthetic check"
    );
    assert_eq!(
        published_kinds(events.as_mut()).await,
        ["routine", "record", "completed"]
    );
    assert_eq!(fixture.queued_summary_count(), 1);
    assert_eq!(
        serde_json::to_value(deps.core.snapshot(fixture.binding.session).unwrap()).unwrap(),
        before
    );
}

#[tokio::test]
async fn relay_real_routine_write_failure_warns_but_handoff_and_completed_continue_then_retry() {
    let fixture = Fixture::new().await;
    let deps = &fixture.composition.dependencies;
    let routine_path = fixture.paths.root().join("routines.json");
    let held = fixture.paths.root().join("routines-held.json");
    let before = std::fs::read(&routine_path).unwrap();
    std::fs::rename(&routine_path, &held).unwrap();
    std::fs::create_dir(&routine_path).unwrap();
    let mut events = deps.core.subscribe();
    let outcome = deps
        .effects
        .apply(deps.core.publish_relay_done(fixture.summary.clone()))
        .await;
    assert!(
        outcome.is_ok(),
        "routine persistence warning does not veto DONE"
    );
    assert_eq!(std::fs::read(&held).unwrap(), before);
    assert!(routine_path.is_dir());
    assert_eq!(
        deps.routine_store
            .run(&fixture.run_id)
            .unwrap()
            .unwrap()
            .status,
        "starting"
    );
    assert_eq!(
        HandoffStore::new(fixture.paths.clone())
            .read_session(fixture.binding.session.0)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        published_kinds(events.as_mut()).await,
        ["routine", "record", "completed"]
    );
    assert_eq!(fixture.queued_summary_count(), 1);
    let log = std::fs::read_to_string(
        fixture
            .paths
            .resource(crate::config::Resource::Logs)
            .join("hub.log"),
    )
    .unwrap();
    assert!(log.contains("routine DONE persistence"));
    // Restore this synthetic obstruction and exercise the real pending-result retry.
    std::fs::remove_dir(&routine_path).unwrap();
    std::fs::rename(&held, &routine_path).unwrap();
    deps.routines.refresh(Timestamp::now()).unwrap();
    fixture.assert_routine_finished();
}

#[tokio::test]
async fn relay_real_second_handoff_append_failure_keeps_prior_commits_and_priming_without_completed()
 {
    let fixture = Fixture::new().await;
    let deps = &fixture.composition.dependencies;
    let store = HandoffStore::new(fixture.paths.clone());
    let path = store.path_for(fixture.binding.session.0).unwrap();
    let held = path.with_extension("held-jsonl");
    let first_path = path.clone();
    let held_path = held.clone();
    let routine = deps.routine_store.clone();
    let run_id = fixture.run_id.clone();
    deps.workers.set_completion_append_hook(Arc::new(move || {
        assert_eq!(routine.run(&run_id).unwrap().unwrap().status, "finished");
        std::fs::rename(&first_path, &held_path).unwrap();
        std::fs::create_dir(&first_path).unwrap();
    }));
    let mut events = deps.core.subscribe();
    let failure = deps
        .effects
        .apply(deps.core.publish_relay_done(fixture.summary.clone()))
        .await
        .unwrap_err();
    assert_eq!(
        failure.index, 1,
        "real CompletionRecord fails before the broadcast and Completed effects"
    );
    assert!(matches!(failure.error, SessionError::Storage(_)));
    fixture.assert_routine_finished();
    let first = std::fs::read(&held).unwrap();
    let records = first
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["kind"], KIND_DONE);
    assert_eq!(
        records[0]["text"],
        format!("[relay] {}", fixture.summary.text)
    );
    assert_eq!(published_kinds(events.as_mut()).await, ["routine"]);
    assert_eq!(
        fixture.queued_summary_count(),
        1,
        "precomputed priming delivery survives a later callback failure"
    );
    assert!(deps.core.is_current(fixture.binding));
    std::fs::remove_dir(&path).unwrap();
    std::fs::rename(&held, &path).unwrap();
    assert_eq!(std::fs::read(path).unwrap(), first);
}
