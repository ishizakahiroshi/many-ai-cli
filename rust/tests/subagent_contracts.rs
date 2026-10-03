//! Source → Rust: internal/hub/subagent_source_*_test.go and subagent_tree_test.go
//! → orchestration/subagent/*.rs. Every provider record below is synthetic.
use many_ai_cli::proto::time::{Timestamp, UNIX_EPOCH};
use many_ai_cli::{
    orchestration::subagent::*,
    proto::{SubagentNode, SubagentTree},
};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};
const TS: &str = "2026-10-03T01:00:00Z";
fn now() -> Timestamp {
    many_ai_cli::proto::time::parse_rfc3339("2026-10-03T02:00:00Z").unwrap()
}
fn write(path: &Path, lines: &[Value]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut file = fs::File::create(path).unwrap();
    for line in lines {
        writeln!(file, "{line}").unwrap();
    }
}
fn append(path: &Path, line: Value) {
    writeln!(
        fs::OpenOptions::new().append(true).open(path).unwrap(),
        "{line}"
    )
    .unwrap();
}
fn read(adapter: Adapter, parent: &Path, prior: Option<Continuation>) -> ReadBatch {
    read_tree(
        adapter,
        parent,
        UNIX_EPOCH,
        prior,
        ReadBudget::default(),
        now(),
    )
    .unwrap()
}
fn node<'a>(tree: &'a SubagentTree, id: &str) -> &'a SubagentNode {
    tree.nodes.iter().find(|n| n.id == id).unwrap()
}
fn claude_parent(root: &Path) -> PathBuf {
    let p = root.join("parent.jsonl");
    write(&p, &[]);
    p
}
fn claude_child(parent: &Path, id: &str, tool: &str, parent_id: &str, depth: i64) {
    let dir = parent.with_extension("").join("subagents");
    write(
        &dir.join(format!("agent-{id}.meta.json")),
        &[
            json!({"agentType":"general","description":format!("Child {id}"),"toolUseId":tool,"spawnDepth":depth,"parentAgentId":parent_id,"model":"synthetic-model","prompt":"NEVER_COPY_PROMPT"}),
        ],
    );
    write(
        &dir.join(format!("agent-{id}.jsonl")),
        &[
            json!({"type":"user","timestamp":TS,"message":{"content":"NEVER_COPY_BODY"}}),
            json!({"type":"assistant","timestamp":TS,"message":{"content":[{"type":"tool_use","id":"call","name":"Read","input":{"file_path":"/synthetic/file","content":"NEVER_COPY_CONTENT"}}]}}),
        ],
    );
}
fn claude_launch(tool: &str) -> Value {
    json!({"type":"assistant","timestamp":TS,"message":{"content":[{"type":"tool_use","id":tool,"name":"Agent"}]}})
}
fn claude_finish(tool: &str, status: &str) -> Value {
    json!({"type":"user","timestamp":"2026-10-03T01:01:00Z","toolUseResult":{"status":status},"message":{"content":[{"type":"tool_result","tool_use_id":tool,"content":"NEVER_COPY_RESULT"}]}})
}
#[test]
fn adapter_registry_and_typed_mismatch() {
    assert!(Adapter::from_key("subagent:claude-v1") == Some(Adapter::Claude));
    assert!(Adapter::from_key("claude").is_none());
    let t = tempfile::tempdir().unwrap();
    let p = claude_parent(t.path());
    let batch = read(Adapter::Claude, &p, None);
    assert!(
        read_tree(
            Adapter::Grok,
            &p,
            UNIX_EPOCH,
            Some(batch.continuation),
            ReadBudget::default(),
            now()
        )
        .is_err()
    );
}
#[test]
fn claude_builds_nested_tree_and_drops_orphans_workflows() {
    let t = tempfile::tempdir().unwrap();
    let p = claude_parent(t.path());
    claude_child(&p, "one", "tool-one", "", 1);
    claude_child(&p, "two", "tool-two", "one", 2);
    claude_child(&p, "orphan", "tool-orphan", "missing", 2);
    claude_child(&p, "workflow", "tool-wf", "", 1);
    let path = p
        .with_extension("")
        .join("subagents/agent-workflow.meta.json");
    write(
        &path,
        &[json!({"agentType":"workflow-subagent","description":"workflow","toolUseId":"tool-wf"})],
    );
    let batch = read(Adapter::Claude, &p, None);
    let tree = batch.tree.unwrap();
    assert_eq!(tree.nodes.len(), 2);
    assert_eq!(node(&tree, "two").parent_id, "one");
    assert_eq!(node(&tree, "one").tool_calls, 1);
    assert_eq!(node(&tree, "one").last_tool_summary, "/synthetic/file");
    assert!(!serde_json::to_string(&tree).unwrap().contains("NEVER_COPY"));
}
#[test]
fn claude_launch_latches_and_terminal_survives_tail_rotation() {
    let t = tempfile::tempdir().unwrap();
    let p = claude_parent(t.path());
    claude_child(&p, "one", "tool-one", "", 1);
    write(&p, &[claude_launch("tool-one")]);
    let first = read(Adapter::Claude, &p, None);
    assert_eq!(first.tree.as_ref().unwrap().nodes[0].state, "running");
    write(&p, &[json!({"type":"irrelevant"})]);
    let second = read_tree(
        Adapter::Claude,
        &p,
        UNIX_EPOCH,
        Some(first.continuation),
        ReadBudget::default(),
        now() + Duration::from_secs(4000),
    )
    .unwrap();
    assert_eq!(second.tree.as_ref().unwrap().nodes[0].state, "running");
    append(&p, claude_finish("tool-one", "completed"));
    let third = read(Adapter::Claude, &p, Some(second.continuation));
    assert_eq!(third.tree.as_ref().unwrap().nodes[0].state, "done");
    write(&p, &[json!({"type":"irrelevant","different":true})]);
    let fourth = read(Adapter::Claude, &p, Some(third.continuation));
    assert_eq!(fourth.tree.unwrap().nodes[0].state, "done");
}
#[test]
fn claude_async_ack_is_running_and_notification_is_terminal() {
    let t = tempfile::tempdir().unwrap();
    let p = claude_parent(t.path());
    claude_child(&p, "one", "tool-one", "", 1);
    write(
        &p,
        &[
            json!({"type":"user","timestamp":TS,"toolUseResult":{"isAsync":true,"status":"async_launched"},"message":{"content":[{"type":"tool_result","tool_use_id":"tool-one"}]}}),
        ],
    );
    let first = read(Adapter::Claude, &p, None);
    assert_eq!(first.tree.as_ref().unwrap().nodes[0].state, "running");
    append(
        &p,
        json!({"type":"queue-operation","operation":"enqueue","timestamp":TS,"content":"<task-notification><tool-use-id>tool-one</tool-use-id><status>killed</status><result>NEVER_COPY_RESULT</result></task-notification>"}),
    );
    let second = read(Adapter::Claude, &p, Some(first.continuation));
    assert_eq!(second.tree.unwrap().nodes[0].state, "failed");
}
#[test]
fn claude_terminal_statuses_and_newest_signal_win() {
    let t = tempfile::tempdir().unwrap();
    let p = claude_parent(t.path());
    claude_child(&p, "one", "tool-one", "", 1);
    for status in [
        "failed",
        "killed",
        "stopped",
        "cancelled",
        "canceled",
        "aborted",
        "interrupted",
        "error",
        "timeout",
        "timed_out",
    ] {
        write(
            &p,
            &[
                claude_finish("tool-one", "completed"),
                claude_finish("tool-one", status),
            ],
        );
        assert_eq!(
            read(Adapter::Claude, &p, None).tree.unwrap().nodes[0].state,
            "failed",
            "{status}"
        );
    }
}
#[test]
fn claude_unchanged_child_and_parent_reuse_cached_reads() {
    let t = tempfile::tempdir().unwrap();
    let p = claude_parent(t.path());
    claude_child(&p, "one", "tool-one", "", 1);
    write(&p, &[claude_launch("tool-one")]);
    let first = read(Adapter::Claude, &p, None);
    assert!(first.stats.head_bytes > 0 && first.stats.parent_bytes > 0);
    let second = read(Adapter::Claude, &p, Some(first.continuation));
    assert_eq!(second.stats.files_read, 0);
    assert_eq!(
        second.stats.head_bytes
            + second.stats.tail_bytes
            + second.stats.parent_bytes
            + second.stats.metadata_bytes,
        0
    );
}
#[test]
fn claude_direct_read_budget_counts_and_partial_tail() {
    let t = tempfile::tempdir().unwrap();
    let p = claude_parent(t.path());
    claude_child(&p, "one", "tool-one", "", 1);
    let child = p.with_extension("").join("subagents/agent-one.jsonl");
    let mut file = fs::OpenOptions::new().append(true).open(&child).unwrap();
    file.write_all(&vec![b'x'; 10 * 1024 * 1024]).unwrap();
    file.write_all(b"\n").unwrap();
    writeln!(file,"{}",json!({"type":"assistant","timestamp":TS,"message":{"content":[{"type":"tool_use","name":"Shell","input":{"command":"echo   hello\n  world","file_path":"ignored","prompt":"NEVER_COPY"}}]}})).unwrap();
    file.write_all(b"{\"type\":\"assistant\"").unwrap();
    let batch = read(Adapter::Claude, &p, None);
    assert_eq!(batch.stats.head_bytes, 64 * 1024);
    assert_eq!(batch.stats.tail_bytes, 128 * 1024);
    let n = &batch.tree.unwrap().nodes[0];
    assert_eq!(n.tool_calls, 0);
    assert_eq!(n.last_tool_name, "Shell");
    assert_eq!(n.last_tool_summary, "echo hello world");
}
#[test]
fn since_and_cap_drop_old_completions_before_running_with_orphan_cascade() {
    let t = tempfile::tempdir().unwrap();
    let p = claude_parent(t.path());
    let mut signals = vec![];
    for i in 0..55 {
        let id = format!("child{i:02}");
        let tool = format!("tool{i}");
        claude_child(&p, &id, &tool, "", 1);
        signals.push(if i < 53 {
            claude_finish(&tool, "completed")
        } else {
            claude_launch(&tool)
        });
    }
    write(&p, &signals);
    let batch = read(Adapter::Claude, &p, None);
    let tree = batch.tree.unwrap();
    assert_eq!(tree.nodes.len(), 50);
    assert_eq!(tree.omitted, 5);
    assert_eq!(node(&tree, "child54").state, "running");
    let cutoff = read_tree(
        Adapter::Claude,
        &p,
        now(),
        Some(batch.continuation),
        ReadBudget::default(),
        now(),
    )
    .unwrap();
    assert_eq!(cutoff.tree.unwrap().nodes.len(), 2);
}
fn codex_meta(id: &str, parent: Option<&str>, depth: i64) -> Value {
    let mut meta = json!({"type":"session_meta","payload":{"id":id,"timestamp":TS,"base_instructions":"NEVER_COPY_BODY"}});
    if let Some(parent) = parent {
        meta["payload"]["source"] = json!({"subagent":{"thread_spawn":{"agent_nickname":format!("Nick {id}"),"agent_role":"reviewer","parent_thread_id":parent,"depth":depth}}});
    }
    meta
}
fn codex_root(root: &Path) -> PathBuf {
    let p = root.join("sessions/2026/10/03/parent.jsonl");
    write(&p, &[codex_meta("root", None, 0)]);
    p
}
fn codex_child(parent: &Path, id: &str, parent_id: &str, depth: i64) -> PathBuf {
    let p = parent.parent().unwrap().join(format!("{id}.jsonl"));
    write(
        &p,
        &[
            codex_meta(id, Some(parent_id), depth),
            json!({"type":"response_item","payload":{"type":"custom_tool_call","name":"exec","input":"echo   hello\nthere"}}),
        ],
    );
    p
}
fn codex_signal(id: &str, kind: &str, time: i64) -> Value {
    json!({"type":"event_msg","payload":{"type":"item_completed","completed_at_ms":time,"item":{"type":"SubAgentActivity","kind":kind,"agent_thread_id":id,"result":"NEVER_COPY"}}})
}
#[test]
fn codex_builds_transitive_tree_and_ignores_unrelated_candidates() {
    let t = tempfile::tempdir().unwrap();
    let p = codex_root(t.path());
    codex_child(&p, "one", "root", 1);
    codex_child(&p, "two", "one", 2);
    codex_child(&p, "unrelated", "other-root", 1);
    let tree = read(Adapter::Codex, &p, None).tree.unwrap();
    assert_eq!(tree.nodes.len(), 2);
    assert_eq!(node(&tree, "two").parent_id, "one");
    assert_eq!(node(&tree, "one").last_tool_summary, "echo hello there");
    assert!(!serde_json::to_string(&tree).unwrap().contains("NEVER_COPY"));
}
#[test]
fn codex_running_and_completion_latch_across_parent_window_loss() {
    let t = tempfile::tempdir().unwrap();
    let p = codex_root(t.path());
    codex_child(&p, "one", "root", 1);
    append(&p, codex_signal("one", "started", 1));
    let first = read(Adapter::Codex, &p, None);
    write(&p, &[codex_meta("root", None, 0)]);
    let second = read_tree(
        Adapter::Codex,
        &p,
        UNIX_EPOCH,
        Some(first.continuation),
        ReadBudget::default(),
        now() + Duration::from_secs(4000),
    )
    .unwrap();
    assert_eq!(second.tree.as_ref().unwrap().nodes[0].state, "running");
    append(&p, codex_signal("one", "interrupted", 99));
    let third = read(Adapter::Codex, &p, Some(second.continuation));
    assert_eq!(third.tree.as_ref().unwrap().nodes[0].state, "failed");
    assert_eq!(third.tree.as_ref().unwrap().nodes[0].finished_at, 99);
    write(&p, &[codex_meta("root", None, 0)]);
    let fourth = read(Adapter::Codex, &p, Some(third.continuation));
    assert_eq!(fourth.tree.unwrap().nodes[0].state, "failed");
}
#[test]
fn codex_function_call_has_no_message_summary_and_bounds_tail() {
    let t = tempfile::tempdir().unwrap();
    let p = codex_root(t.path());
    let c = codex_child(&p, "one", "root", 1);
    let mut file = fs::OpenOptions::new().append(true).open(&c).unwrap();
    file.write_all(&vec![b'x'; 512 * 1024]).unwrap();
    file.write_all(b"\n").unwrap();
    writeln!(file,"{}",json!({"type":"response_item","payload":{"type":"function_call","name":"send_message","arguments":"{\"message\":\"NEVER_COPY\"}"}})).unwrap();
    let batch = read(Adapter::Codex, &p, None);
    assert_eq!(batch.stats.tail_bytes, 128 * 1024);
    let n = &batch.tree.unwrap().nodes[0];
    assert_eq!(n.last_tool_name, "send_message");
    assert_eq!(n.last_tool_summary, "");
}
#[test]
fn codex_first_observation_unknown_is_frozen_despite_later_file_growth() {
    let t = tempfile::tempdir().unwrap();
    let p = codex_root(t.path());
    let c = codex_child(&p, "one", "root", 1);
    let file = fs::File::options().write(true).open(&c).unwrap();
    file.set_times(fs::FileTimes::new().set_modified(UNIX_EPOCH.to_system_time_exact().unwrap()))
        .unwrap();
    let first = read(Adapter::Codex, &p, None);
    assert_eq!(first.tree.as_ref().unwrap().nodes[0].state, "unknown");
    append(
        &c,
        json!({"type":"response_item","payload":{"type":"custom_tool_call","name":"exec","input":"echo next"}}),
    );
    let second = read(Adapter::Codex, &p, Some(first.continuation));
    assert_eq!(second.tree.unwrap().nodes[0].state, "unknown");
}
#[test]
fn codex_past_day_scanned_once_and_reattach_uses_cached_children() {
    let t = tempfile::tempdir().unwrap();
    let p = codex_root(t.path());
    codex_child(&p, "one", "root", 1);
    let later = now() + Duration::from_secs(86400);
    let first = read_tree(
        Adapter::Codex,
        &p,
        UNIX_EPOCH,
        None,
        ReadBudget::default(),
        later,
    )
    .unwrap();
    codex_child(&p, "late", "root", 1);
    let second = read_tree(
        Adapter::Codex,
        &p,
        UNIX_EPOCH,
        Some(first.continuation),
        ReadBudget::default(),
        later,
    )
    .unwrap();
    assert_eq!(second.tree.unwrap().nodes.len(), 1);
}
fn grok_root(root: &Path) -> PathBuf {
    let p = root.join("sessions/cwd/parent/updates.jsonl");
    write(&p, &[]);
    fs::create_dir_all(p.parent().unwrap().join("subagents")).unwrap();
    p
}
fn grok_spawn(parent: &Path, id: &str, session: &str) {
    fs::create_dir_all(parent.parent().unwrap().join("subagents").join(id)).unwrap();
    append(
        parent,
        json!({"timestamp":1790989200,"method":"_x.ai/session/update","params":{"sessionId":"parent","update":{"sessionUpdate":"subagent_spawned","subagent_id":id,"subagent_type":"general","description":format!("Child {id}"),"model":"synthetic","child_session_id":session,"prompt":"NEVER_COPY"}}}),
    );
}
fn grok_finish(id: &str, status: &str) -> Value {
    json!({"timestamp":1790989300,"params":{"update":{"sessionUpdate":"subagent_finished","subagent_id":id,"status":status,"tool_calls":4,"output":"NEVER_COPY"}}})
}
#[test]
fn grok_uses_sibling_flat_tool_events_and_never_subagent_events() {
    let t = tempfile::tempdir().unwrap();
    let p = grok_root(t.path());
    grok_spawn(&p, "one", "child-session");
    write(
        &p.parent().unwrap().join("subagents/one/events.jsonl"),
        &[json!({"type":"tool_started","tool_name":"WRONG_SOURCE"})],
    );
    write(
        &p.parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("child-session/events.jsonl"),
        &[
            json!({"type":"tool_started","tool_name":"read_file","rawInput":"NEVER_COPY"}),
            json!({"type":"phase_changed","tool_name":"WRONG_PHASE"}),
        ],
    );
    let tree = read(Adapter::Grok, &p, None).tree.unwrap();
    let n = &tree.nodes[0];
    assert_eq!(n.state, "running");
    assert_eq!(n.last_tool_name, "read_file");
    assert_eq!(n.last_tool_summary, "");
    assert_eq!(n.depth, 1);
    assert_eq!(n.parent_id, "");
    assert!(!serde_json::to_string(&tree).unwrap().contains("NEVER_COPY"));
}
#[test]
fn grok_spawn_and_finish_latch_across_window_loss() {
    let t = tempfile::tempdir().unwrap();
    let p = grok_root(t.path());
    grok_spawn(&p, "one", "");
    let first = read(Adapter::Grok, &p, None);
    write(&p, &[]);
    let second = read(Adapter::Grok, &p, Some(first.continuation));
    assert_eq!(second.tree.as_ref().unwrap().nodes[0].state, "running");
    append(&p, grok_finish("one", "completed"));
    let third = read(Adapter::Grok, &p, Some(second.continuation));
    assert_eq!(third.tree.as_ref().unwrap().nodes[0].state, "done");
    assert_eq!(third.tree.as_ref().unwrap().nodes[0].tool_calls, 4);
    write(&p, &[]);
    let fourth = read(Adapter::Grok, &p, Some(third.continuation));
    assert_eq!(fourth.tree.unwrap().nodes[0].state, "done");
}
#[test]
fn grok_meta_canceled_fails_and_wrong_parent_is_dropped() {
    let t = tempfile::tempdir().unwrap();
    let p = grok_root(t.path());
    for (id, parent) in [("one", "parent"), ("wrong", "different")] {
        write(
            &p.parent()
                .unwrap()
                .join(format!("subagents/{id}/meta.json")),
            &[
                json!({"parent_session_id":parent,"description":"Child","status":"cancelled","started_at":TS,"completed_at":TS,"tool_calls":9,"prompt":"NEVER_COPY","error":"NEVER_COPY"}),
            ],
        );
    }
    let batch = read(Adapter::Grok, &p, None);
    let tree = batch.tree.unwrap();
    assert_eq!(tree.nodes.len(), 1);
    assert_eq!(tree.nodes[0].state, "failed");
    assert_eq!(tree.nodes[0].tool_calls, 9);
    // Rejected sibling metadata is intentionally retried by the Go reader.
    // Move that fixture aside to isolate completed-child caching, then corrupt
    // the accepted child's metadata to prove the cached node is retained.
    std::fs::rename(
        p.parent().unwrap().join("subagents/wrong"),
        t.path().join("rejected-subagent"),
    )
    .unwrap();
    std::fs::write(
        p.parent().unwrap().join("subagents/one/meta.json"),
        b"changed after completion",
    )
    .unwrap();
    let next = read(Adapter::Grok, &p, Some(batch.continuation));
    assert_eq!(next.stats.files_read, 0);
    let cached = next.tree.unwrap();
    assert_eq!(cached.nodes[0].state, "failed");
    assert_eq!(cached.nodes[0].tool_calls, 9);
}
#[test]
fn grok_path_traversal_child_session_cannot_read_outside() {
    let t = tempfile::tempdir().unwrap();
    let p = grok_root(t.path());
    grok_spawn(&p, "one", "../../../outside");
    write(
        &t.path().join("outside/events.jsonl"),
        &[json!({"type":"tool_started","tool_name":"PRIVATE"})],
    );
    let tree = read(Adapter::Grok, &p, None).tree.unwrap();
    assert_eq!(tree.nodes[0].last_tool_name, "");
    assert_eq!(tree.nodes[0].state, "running");
}
#[test]
fn poll_nil_keeps_tree_empty_clears_once_and_generation_rejects_stale() {
    let mut poll = PollState::default();
    let t = tempfile::tempdir().unwrap();
    let p = grok_root(t.path());
    grok_spawn(&p, "one", "");
    let first = read(Adapter::Grok, &p, None);
    assert!(poll.apply(0, first).is_some());
    let missing = read(
        Adapter::Grok,
        &t.path().join("missing/updates.jsonl"),
        poll.continuation.clone(),
    );
    assert!(missing.tree.is_none());
    assert!(poll.apply(0, missing).is_none());
    assert_eq!(poll.running_count, 1);
    poll.stop();
    let batch = read(Adapter::Grok, &p, None);
    assert!(poll.apply(0, batch).is_none());
    fs::remove_dir_all(p.parent().unwrap().join("subagents/one")).unwrap();
    let empty = read(Adapter::Grok, &p, None);
    assert!(poll.apply(1, empty).unwrap().nodes.is_empty());
    let empty = read(Adapter::Grok, &p, None);
    assert!(poll.apply(1, empty).is_none());
}
#[test]
fn reattach_preserves_reader_cutoff_but_resets_dedupe() {
    let t = tempfile::tempdir().unwrap();
    let p = grok_root(t.path());
    grok_spawn(&p, "one", "");
    let mut poll = PollState::default();
    poll.confirmed_user_turn(UNIX_EPOCH);
    poll.apply(0, read(Adapter::Grok, &p, None));
    poll.confirmed_user_turn(now());
    assert_eq!(poll.turn_started_at, Some(UNIX_EPOCH));
    poll.reattach("bad", now());
    assert_eq!(poll.turn_started_at, Some(UNIX_EPOCH));
    assert!(poll.continuation.is_some());
    assert_eq!(poll.generation, 1);
    let same = read(Adapter::Grok, &p, poll.continuation.clone());
    assert!(poll.apply(1, same).is_some());
    let mut empty = PollState::default();
    empty.reattach("bad", now());
    assert_eq!(empty.turn_started_at, Some(now()));
    let mut valid = PollState::default();
    valid.reattach(TS, now());
    assert_eq!(
        valid.turn_started_at,
        Some(many_ai_cli::proto::time::parse_rfc3339(TS).unwrap())
    );
}
#[test]
fn signature_ignores_clock_but_includes_activity_and_allowlist() {
    let mut tree = SubagentTree {
        provider: "subagent:claude-v1".into(),
        nodes: vec![SubagentNode {
            id: "one".into(),
            state: "running".into(),
            last_activity_at: 1,
            ..Default::default()
        }],
        ..Default::default()
    };
    let first = tree_signature(&tree);
    tree.updated_at = 999;
    assert_eq!(first, tree_signature(&tree));
    tree.nodes[0].last_activity_at = 2;
    assert_ne!(first, tree_signature(&tree));
    let value = serde_json::to_string(&tree).unwrap();
    for forbidden in [
        "prompt",
        "result_preview",
        "transcript",
        "content",
        "environment",
    ] {
        assert!(!value.contains(forbidden));
    }
}
#[test]
fn changed_parent_path_resets_provider_cache() {
    let t = tempfile::tempdir().unwrap();
    let a = claude_parent(&t.path().join("a"));
    claude_child(&a, "one", "tool", "", 1);
    write(&a, &[claude_finish("tool", "completed")]);
    let first = read(Adapter::Claude, &a, None);
    let b = claude_parent(&t.path().join("b"));
    claude_child(&b, "one", "tool", "", 1);
    write(&b, &[claude_launch("tool")]);
    let second = read(Adapter::Claude, &b, Some(first.continuation));
    assert_eq!(second.tree.unwrap().nodes[0].state, "running");
}

#[derive(serde::Deserialize)]
struct GoldenCase {
    name: String,
    adapter: String,
    parent: String,
    files: std::collections::BTreeMap<String, String>,
    since: String,
    tree: Option<SubagentTree>,
    signature: String,
}
#[test]
fn actual_go_three_provider_tree_and_signature_corpus() {
    let cases: Vec<GoldenCase> =
        serde_json::from_str(include_str!("fixtures/core/subagent/go-golden.json")).unwrap();
    assert_eq!(cases.len(), 10);
    let modified = many_ai_cli::proto::time::parse_rfc3339(TS).unwrap();
    for c in cases {
        let root = tempfile::tempdir().unwrap();
        for (relative, data) in &c.files {
            let path = root.path().join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, data).unwrap();
            fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_times(
                    fs::FileTimes::new().set_modified(modified.to_system_time_exact().unwrap()),
                )
                .unwrap();
        }
        let batch = read_tree(
            Adapter::from_key(&c.adapter).unwrap(),
            &root.path().join(&c.parent),
            many_ai_cli::proto::time::parse_rfc3339(&c.since).unwrap(),
            None,
            ReadBudget::default(),
            now(),
        )
        .unwrap();
        let mut actual = batch.tree.unwrap_or_default();
        actual.updated_at = 0;
        if let Some(expected) = c.tree {
            assert_eq!(
                serde_json::to_value(&actual).unwrap(),
                serde_json::to_value(&expected).unwrap(),
                "{}",
                c.name
            );
            assert_eq!(tree_signature(&actual), c.signature, "{}", c.name);
        } else {
            assert!(actual.nodes.is_empty(), "{}", c.name);
        }
    }
}
