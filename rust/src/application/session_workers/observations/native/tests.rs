use super::*;
use crate::{
    config::{Config, ConfigStore},
    hub::task_owner::HubTaskOwner,
    profile::registry::{self, Registry},
    proto::provider::Layers,
};
use chrono::{Datelike, Local};
fn view(
    binding: SessionBinding,
    identity: TranscriptSessionIdentity,
) -> SessionObservationSnapshot {
    SessionObservationSnapshot {
        details: SessionDetails {
            binding,
            last_output_at: None,
            snapshot: SessionSnapshot {
                provider: identity.provider.clone(),
                cwd: identity.cwd.clone(),
                state: "active".into(),
                ..Default::default()
            },
            db_id: None,
            git_root: None,
            transcript: identity,
            approval: ApprovalSessionSnapshot {
                session: binding.session,
                version: ApprovalStateVersion(0),
                record: None,
            },
            workflow: None,
            subagents: None,
            done: None,
            connected: true,
        },
        tail: Vec::new(),
        output_generation: 0,
        confirmed_turn: 1,
        turn_started_at: Timestamp::UNIX_EPOCH,
        resize_debounce: None,
    }
}
#[tokio::test]
async fn custom_definition_adapter_runs_real_codex_reader_clears_once_and_replays_after_reattach() {
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49328, installed.path()).unwrap();
    let tasks = HubTaskOwner::new(tokio::runtime::Handle::current());
    let config = Arc::new(ConfigStore::new(paths.clone(), Config::defaults(&paths)).unwrap());
    let worker = SessionWorkers::new(
        config,
        paths.clone(),
        Arc::new(FilesService::new(root.path().into(), paths.clone())),
        tasks.handle(),
        Arc::new(|_, _| {}),
    );
    let mut definition = registry::embedded_definitions()
        .unwrap()
        .into_iter()
        .find(|d| d.id == "codex")
        .unwrap();
    definition.id = "custom-codex".into();
    let registry = Registry::build(
        Layers {
            embedded: Some(vec![definition]),
            ..Default::default()
        },
        &registry::default_adapters(),
    );
    assert_eq!(
        registry
            .lookup("custom-codex")
            .unwrap()
            .definition
            .adapters
            .subagents,
        "subagent:codex-v1"
    );
    worker
        .set_observation_registry(Arc::new(move || Ok(registry.clone())))
        .unwrap();
    let now = Timestamp::now();
    let date = proto::time::utc(now).unwrap().with_timezone(&Local);
    let directory = root
        .path()
        .join("profile/sessions")
        .join(format!("{:04}", date.year()))
        .join(format!("{:02}", date.month()))
        .join(format!("{:02}", date.day()));
    std::fs::create_dir_all(&directory).unwrap();
    let parent = directory.join("parent.jsonl");
    let child = directory.join("child.jsonl");
    let timestamp = proto::time::format_rfc3339_nano(now).unwrap();
    let parent_meta = serde_json::json!({"type":"session_meta","payload":{"id":"root","timestamp":timestamp,"source":{}}});
    let finished = serde_json::json!({"type":"event_msg","payload":{"type":"item_completed","completed_at_ms":83,"item":{"type":"SubAgentActivity","agent_thread_id":"child","kind":"completed"}}});
    std::fs::write(&parent, format!("{parent_meta}\n{finished}\n")).unwrap();
    let metadata = serde_json::json!({"type":"session_meta","payload":{"id":"child","timestamp":timestamp,"source":{"subagent":{"thread_spawn":{"parent_thread_id":"root","depth":1}}}}});
    std::fs::write(&child, format!("{metadata}\n")).unwrap();
    let binding = SessionBinding {
        session: LiveSessionId(7),
        incarnation: SessionIncarnation(1),
        wrapper: WrapperConnectionId(1),
    };
    let mut snapshot = view(
        binding,
        TranscriptSessionIdentity {
            provider: "custom-codex".into(),
            cwd: "synthetic".into(),
            started_at: timestamp,
            codex_home: root.path().join("profile").to_string_lossy().into_owned(),
            native_log_path: parent.to_string_lossy().into_owned(),
            ..Default::default()
        },
    );
    let mut native = Native::default();
    assert!(native.due.is_none());
    native.start(now);
    let tree = native.poll(&worker, &snapshot, true, now).unwrap().unwrap();
    assert_eq!(tree.nodes[0].state, "done");
    assert_eq!(tree.provider, "subagent:codex-v1");
    snapshot.details.subagents = Some(tree.clone());
    let later = now.checked_add(Duration::from_secs(3)).unwrap();
    assert!(
        native
            .poll(&worker, &snapshot, true, later)
            .unwrap()
            .is_none()
    );
    native.reattach(later);
    assert!(
        native
            .poll(&worker, &snapshot, true, later)
            .unwrap()
            .unwrap()
            .nodes
            == tree.nodes
    );
    snapshot.turn_started_at = now.checked_add(Duration::from_secs(1)).unwrap();
    let later = later.checked_add(Duration::from_secs(3)).unwrap();
    let empty = native
        .poll(&worker, &snapshot, true, later)
        .unwrap()
        .unwrap();
    assert!(empty.nodes.is_empty());
    snapshot.details.subagents = None;
    let later = later.checked_add(Duration::from_secs(3)).unwrap();
    assert!(
        native
            .poll(&worker, &snapshot, true, later)
            .unwrap()
            .is_none()
    );
    assert!(
        native
            .poll(
                &worker,
                &snapshot,
                false,
                later.checked_add(Duration::from_secs(3)).unwrap()
            )
            .unwrap()
            .is_none()
    );
    assert!(native.due.is_none());
}
#[test]
fn native_signature_ignores_clock_only_updates_but_tracks_actual_activity() {
    let tree = proto::SubagentTree {
        provider: "subagent:claude-v1".into(),
        updated_at: 1,
        nodes: vec![proto::SubagentNode {
            id: "a".into(),
            last_activity_at: 2,
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut changed = tree.clone();
    changed.updated_at = 3;
    assert_eq!(signature(&tree), signature(&changed));
    changed.nodes[0].last_activity_at = 3;
    assert_ne!(signature(&tree), signature(&changed));
}
