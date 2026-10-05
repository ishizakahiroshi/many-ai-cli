use super::*;
#[test]
fn live_sibling_tool_and_started_latch_survive_parent_window_then_terminal_meta_sticks() {
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49318, installed.path()).unwrap();
    let session = root.path().join("profile/sessions/cwd/parent");
    std::fs::create_dir_all(session.join("subagents/worker")).unwrap();
    let sibling = session.parent().unwrap().join("child");
    std::fs::create_dir(&sibling).unwrap();
    std::fs::write(
        sibling.join("events.jsonl"),
        "{\"type\":\"tool_started\",\"tool_name\":\"Bash\",\"arguments\":\"synthetic omitted\"}\n",
    )
    .unwrap();
    let parent = session.join("updates.jsonl");
    let spawn = serde_json::json!({"timestamp":100,"params":{"update":{"sessionUpdate":"subagent_spawned","subagent_id":"worker","subagent_type":"review","description":"worker label","model":"model","child_session_id":"child"}}});
    std::fs::write(&parent, format!("{spawn}\n")).unwrap();
    let (tree, state) = read(
        &parent,
        Timestamp::UNIX_EPOCH,
        State::default(),
        &ReadBudget::default(),
        &paths,
        Timestamp::now(),
    )
    .unwrap();
    let node = &tree.unwrap().nodes[0];
    assert_eq!(node.state, "running");
    assert_eq!(node.last_tool_name, "Bash");
    assert!(node.last_tool_summary.is_empty());
    assert_eq!(node.started_at, 100000);
    std::fs::write(&parent, "").unwrap();
    let (tree, state) = read(
        &parent,
        Timestamp::UNIX_EPOCH,
        state,
        &ReadBudget::default(),
        &paths,
        Timestamp::now(),
    )
    .unwrap();
    assert_eq!(tree.unwrap().nodes[0].state, "running");
    let meta = session.join("subagents/worker/meta.json");
    std::fs::write(&meta,r#"{"parent_session_id":"parent","status":"completed","description":"final label","tool_calls":7,"prompt":"synthetic never retained","error":"synthetic never retained"}"#).unwrap();
    let (tree, state) = read(
        &parent,
        Timestamp::UNIX_EPOCH,
        state,
        &ReadBudget::default(),
        &paths,
        Timestamp::now(),
    )
    .unwrap();
    let tree = tree.unwrap();
    assert_eq!(tree.nodes[0].state, "done");
    assert_eq!(tree.nodes[0].tool_calls, 7);
    assert!(
        !serde_json::to_string(&tree)
            .unwrap()
            .contains("never retained")
    );
    std::fs::write(&meta, r#"{"parent_session_id":"wrong","status":"failed"}"#).unwrap();
    let (tree, _) = read(
        &parent,
        Timestamp::UNIX_EPOCH,
        state,
        &ReadBudget::default(),
        &paths,
        Timestamp::now(),
    )
    .unwrap();
    assert_eq!(tree.unwrap().nodes[0].state, "done");
}
