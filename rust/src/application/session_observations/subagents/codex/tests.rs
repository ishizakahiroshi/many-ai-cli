use super::*;
fn fixture() -> (
    tempfile::TempDir,
    tempfile::TempDir,
    RuntimePaths,
    PathBuf,
    PathBuf,
    Timestamp,
) {
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49317, installed.path()).unwrap();
    let now = Timestamp::now();
    let date = utc(now).unwrap().with_timezone(&Local);
    let dir = root
        .path()
        .join("profile/sessions")
        .join(format!("{:04}", date.year()))
        .join(format!("{:02}", date.month()))
        .join(format!("{:02}", date.day()));
    std::fs::create_dir_all(&dir).unwrap();
    let parent = dir.join("parent.jsonl");
    let child = dir.join("child.jsonl");
    let timestamp = proto::time::format_rfc3339_nano(now).unwrap();
    std::fs::write(&parent,serde_json::to_string(&serde_json::json!({"type":"session_meta","payload":{"id":"root","timestamp":timestamp,"source":{}}})).unwrap()+"\n").unwrap();
    std::fs::write(&child,serde_json::to_string(&serde_json::json!({"type":"session_meta","payload":{"id":"child","timestamp":timestamp,"source":{"subagent":{"thread_spawn":{"parent_thread_id":"root","depth":1,"agent_nickname":"worker","agent_role":"reviewer"}}}}})).unwrap()+"\n").unwrap();
    (root, installed, paths, parent, child, now)
}
fn append(path: &Path, value: serde_json::Value) {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    writeln!(file, "{value}").unwrap();
}
#[test]
fn sticky_parent_signal_and_allowlisted_custom_tool_summary() {
    let (_root, _installed, paths, parent, child, now) = fixture();
    append(
        &child,
        serde_json::json!({"type":"response_item","payload":{"type":"custom_tool_call","name":"exec","input":"echo synthetic"}}),
    );
    append(
        &parent,
        serde_json::json!({"type":"event_msg","payload":{"type":"item_completed","completed_at_ms":83,"item":{"type":"SubAgentActivity","agent_thread_id":"child","kind":"completed"}}}),
    );
    let (tree, state) = read(
        &parent,
        Timestamp::UNIX_EPOCH,
        State::default(),
        &ReadBudget::default(),
        &paths,
        now,
    )
    .unwrap();
    let node = &tree.unwrap().nodes[0];
    assert_eq!(node.state, "done");
    assert_eq!(node.finished_at, 83);
    assert_eq!(node.last_tool_name, "exec");
    assert_eq!(node.last_tool_summary, "echo synthetic");
    assert_eq!(node.label, "worker");
    let parent_meta = std::fs::read_to_string(&parent)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_owned();
    std::fs::write(&parent, parent_meta + "\n").unwrap();
    append(
        &child,
        serde_json::json!({"type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"never retain synthetic arguments"}}),
    );
    let (tree, _) = read(
        &parent,
        Timestamp::UNIX_EPOCH,
        state,
        &ReadBudget::default(),
        &paths,
        now,
    )
    .unwrap();
    let node = &tree.unwrap().nodes[0];
    assert_eq!(node.state, "done");
    assert_eq!(node.finished_at, 83);
    assert_eq!(node.last_tool_name, "shell");
    assert!(node.last_tool_summary.is_empty());
}
#[test]
fn unseen_first_observation_remains_unknown_after_new_growth() {
    let (_root, _installed, paths, parent, child, now) = fixture();
    std::fs::File::options()
        .write(true)
        .open(&child)
        .unwrap()
        .set_modified(SystemTime::UNIX_EPOCH)
        .unwrap();
    let (tree, state) = read(
        &parent,
        Timestamp::UNIX_EPOCH,
        State::default(),
        &ReadBudget::default(),
        &paths,
        now,
    )
    .unwrap();
    assert_eq!(tree.unwrap().nodes[0].state, "unknown");
    append(
        &child,
        serde_json::json!({"type":"response_item","payload":{"type":"function_call","name":"shell"}}),
    );
    let (tree, _) = read(
        &parent,
        Timestamp::UNIX_EPOCH,
        state,
        &ReadBudget::default(),
        &paths,
        now,
    )
    .unwrap();
    assert_eq!(tree.unwrap().nodes[0].state, "unknown");
}
