//! Deterministic inherited Go behavior, not a fix for asynchronous history
//! ordering. Explicit StoreEvent calls model the observed late commit order.
use many_ai_cli::{
    config::{Resource, RuntimePaths},
    proto::{
        core::*,
        time::{self, Timestamp},
    },
    storage::SqliteSessionStorage,
};
use serde_json::{Value, json};

fn card(meta: SessionCardMeta) -> Value {
    json!({"label":meta.label,"pinned":meta.pinned,"color":meta.color,
        "note":meta.note,"auto_title":meta.auto_title})
}
fn event(kind: &str) -> HistoryEvent {
    HistoryEvent(
        json!({"type":kind,"session_id":7,"ts":"2026-01-02T03:04:05Z","label":"launch-label"})
            .as_object()
            .unwrap()
            .clone(),
    )
}

#[test]
fn late_lifecycle_event_overwrite_matches_fixed_go_for_registration_and_reattach() {
    let observations: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/core/session/late-card-meta-go.json")).unwrap();
    assert_eq!(observations.len(), 2);
    for expected in observations {
        let kind = expected["kind"].as_str().unwrap();
        let root = tempfile::tempdir().unwrap();
        let installed = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::trial(root.path(), 49131, installed.path()).unwrap();
        let store = SqliteSessionStorage::open(
            &paths,
            StorageOptions::baseline(paths.resource(Resource::Logs)),
        )
        .unwrap();
        let id = LiveSessionId(7);
        let mut start = SessionStart {
            live_session_id: id,
            provider: "copilot".into(),
            cwd: root.path().to_string_lossy().into_owned(),
            label: "launch-label".into(),
            state: "standby".into(),
            started_at: "2026-01-02T03:04:05Z".into(),
            jsonl_path: root
                .path()
                .join("sessions/synthetic.jsonl")
                .to_string_lossy()
                .into_owned(),
            ..Default::default()
        };
        let ended_at: Timestamp = time::parse_rfc3339(&start.started_at).unwrap();
        store.start_session(start.clone()).unwrap();
        if kind == "session_reattach" {
            store.store_event(id, event("session_start")).unwrap();
        }
        store
            .update_session_card_meta(
                id,
                SessionCardMeta {
                    label: "renamed card".into(),
                    pinned: true,
                    color: "blue".into(),
                    note: "synthetic note".into(),
                    auto_title: "synthetic title".into(),
                },
            )
            .unwrap();
        if kind == "session_reattach" {
            store.end_session(id, "disconnected", "", ended_at);
            start.state = "running".into();
            store.start_session(start.clone()).unwrap();
        }
        let before = card(store.session_card_meta_by_live_session(id).unwrap());
        // Commit the lifecycle event after the card edit, exactly as a delayed
        // history worker can. No scheduling sleeps or changed production SQL.
        store.store_event(id, event(kind)).unwrap();
        let after = card(store.session_card_meta_by_live_session(id).unwrap());
        let history = store.timeline_by_live_session(id, 10).unwrap().unwrap();
        let label = history
            .iter()
            .rev()
            .find(|entry| entry.r#type == kind)
            .unwrap()
            .payload["label"]
            .clone();
        store.end_session(id, "disconnected", "", ended_at);
        start.state = "running".into();
        store.start_session(start).unwrap();
        let next = card(store.session_card_meta_by_live_session(id).unwrap());
        assert_eq!(
            json!({"kind":kind,"before_event":before,"after_event":after,"after_next_start":next,"history_label":label}),
            expected
        );
        store.close().unwrap();
    }
}
