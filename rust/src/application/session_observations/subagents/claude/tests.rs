use super::*;
use std::fs::{self, File};
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ChildInput {
    #[serde(rename = "ID")]
    id: String,
    meta: String,
    #[serde(rename = "JSONL")]
    jsonl: String,
    age: i64,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Stage {
    parent: Option<String>,
    now: i64,
    since: i64,
    max: usize,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Case {
    name: String,
    children: Vec<ChildInput>,
    stages: Vec<Stage>,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Golden {
    name: String,
    trees: Vec<Option<SubagentTree>>,
}
fn now() -> Timestamp {
    Timestamp::from_unix(1791158400, 0).unwrap()
}
fn fixture() -> (tempfile::TempDir, RuntimePaths, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let runtime = root.path().join("trial");
    fs::create_dir(&runtime).unwrap();
    let paths = RuntimePaths::trial(&runtime, 49689, &root.path().join("installed")).unwrap();
    let parent = runtime.join("parent.jsonl");
    fs::create_dir_all(parent.with_extension("").join("subagents")).unwrap();
    (root, paths, parent)
}
fn write(path: &Path, bytes: &[u8], at: Timestamp) {
    fs::write(path, bytes).unwrap();
    File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(at.to_system_time_exact().unwrap()))
        .unwrap();
}
fn child(parent: &Path, id: &str, meta: &str, body: &str, at: Timestamp) {
    let dir = parent.with_extension("").join("subagents");
    write(
        &dir.join(format!("agent-{id}.meta.json")),
        meta.as_bytes(),
        at,
    );
    write(&dir.join(format!("agent-{id}.jsonl")), body.as_bytes(), at);
}
const META: &str =
    r#"{"agentType":"general","description":"short label","toolUseId":"tool-a","spawnDepth":1}"#;
const BODY: &str = "{\"type\":\"assistant\",\"timestamp\":\"2026-10-05T00:00:00Z\",\"message\":{\"content\":[{\"type\":\"tool_use\",\"name\":\"Bash\",\"input\":{\"command\":\"git status\",\"prompt\":\"PRIVATE_BODY_SENTINEL\"}}]}}\n";
#[test]
fn pinned_go_native_reader_cases() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("cases.json")).unwrap();
    let golden: Vec<Golden> = serde_json::from_str(include_str!("golden.json")).unwrap();
    assert_eq!(cases.len(), 52);
    assert_eq!(cases.len(), golden.len());
    for (case, expected) in cases.into_iter().zip(golden) {
        assert_eq!(case.name, expected.name);
        let (_root, paths, parent) = fixture();
        for c in case.children {
            child(
                &parent,
                &c.id,
                &c.meta,
                &c.jsonl,
                now() - Duration::from_secs(c.age as u64),
            );
        }
        let mut state = State::default();
        let mut trees = vec![];
        for stage in case.stages {
            let at = now() + Duration::from_secs(stage.now as u64);
            if let Some(body) = stage.parent {
                write(&parent, body.as_bytes(), at);
            }
            let since = if stage.since >= 0 {
                now() + Duration::from_secs(stage.since as u64)
            } else {
                now() - Duration::from_secs(stage.since.unsigned_abs())
            };
            let (tree, next) = read(
                &parent,
                since,
                state,
                &ReadBudget {
                    max_nodes: stage.max,
                    ..Default::default()
                },
                &paths,
                at,
            )
            .unwrap();
            state = next;
            trees.push(tree);
        }
        assert_eq!(
            serde_json::to_value(trees).unwrap(),
            serde_json::to_value(expected.trees).unwrap(),
            "{}",
            case.name
        );
    }
}
#[test]
fn real_ten_mib_child_reads_never_exceed_head_tail_and_unchanged_poll_reads_zero_bytes() {
    let (_root, paths, parent) = fixture();
    write(&parent, b"", now());
    let mut body = BODY.as_bytes().to_vec();
    body.extend(std::iter::repeat_n(b'x', 10 * 1024 * 1024));
    body.push(b'\n');
    body.extend_from_slice(BODY.as_bytes());
    child(
        &parent,
        "a",
        META,
        std::str::from_utf8(&body).unwrap(),
        now(),
    );
    let mut stats = ReadStats::default();
    let (tree, state) = read_with_stats(
        &parent,
        now() - Duration::from_secs(1),
        State::default(),
        &ReadBudget::default(),
        &paths,
        now(),
        &mut stats,
    )
    .unwrap();
    let tree = tree.unwrap();
    assert_eq!(stats.head_bytes, 64 * 1024);
    assert_eq!(stats.tail_bytes, 128 * 1024);
    assert_eq!(tree.nodes[0].tool_calls, 0);
    assert_eq!(tree.nodes[0].last_tool_name, "Bash");
    assert_eq!(tree.nodes[0].last_tool_summary, "git status");
    assert!(
        !serde_json::to_string(&tree)
            .unwrap()
            .contains("PRIVATE_BODY_SENTINEL")
    );
    let mut cached = ReadStats::default();
    let (tree, _) = read_with_stats(
        &parent,
        now(),
        state,
        &ReadBudget::default(),
        &paths,
        now() + Duration::from_secs(1200),
        &mut cached,
    )
    .unwrap();
    assert_eq!(tree.unwrap().nodes[0].state, "running");
    assert_eq!(
        (
            cached.head_bytes,
            cached.tail_bytes,
            cached.parent_bytes,
            cached.metadata_bytes,
            cached.files_read
        ),
        (0, 0, 0, 0, 0)
    );
}
#[test]
fn metadata_cached_once_and_disappearing_entry_prunes_continuation() {
    let (_root, paths, parent) = fixture();
    write(&parent, b"", now());
    child(&parent, "a", META, BODY, now());
    let (_, state) = read(
        &parent,
        now(),
        State::default(),
        &ReadBudget::default(),
        &paths,
        now(),
    )
    .unwrap();
    let meta_path = parent
        .with_extension("")
        .join("subagents/agent-a.meta.json");
    write(
        &meta_path,
        b"malformed replacement",
        now() + Duration::from_secs(1),
    );
    let (tree, state) = read(
        &parent,
        now(),
        state,
        &ReadBudget::default(),
        &paths,
        now() + Duration::from_secs(1),
    )
    .unwrap();
    assert_eq!(tree.unwrap().nodes[0].label, "short label");
    fs::remove_file(&meta_path).unwrap();
    let (tree, state) = read(&parent, now(), state, &ReadBudget::default(), &paths, now()).unwrap();
    assert!(tree.is_none());
    assert!(state.children.is_empty());
}
#[test]
fn newest_complete_lines_only_and_500_parent_records_bound_signal_scan() {
    let (_root, paths, parent) = fixture();
    let old_body = BODY.replace("2026-10-05T00:00:00Z", "2026-10-04T23:40:00Z");
    child(
        &parent,
        "a",
        META,
        &old_body,
        now() - Duration::from_secs(1200),
    );
    let launch = "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"tool_use\",\"id\":\"tool-a\"}]}}\n";
    let parent_body = launch.to_owned()
        + &"{}\n".repeat(500)
        + "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"tool_use\",\"id\":\"tool-a\"}]}}";
    write(&parent, parent_body.as_bytes(), now());
    let (tree, _) = read(
        &parent,
        now() - Duration::from_secs(1),
        State::default(),
        &ReadBudget::default(),
        &paths,
        now(),
    )
    .unwrap();
    assert!(tree.is_none());
    write(
        &parent,
        format!("{}\n", parent_body).as_bytes(),
        now() + Duration::from_secs(1),
    );
    let (tree, _) = read(
        &parent,
        now(),
        State::default(),
        &ReadBudget::default(),
        &paths,
        now(),
    )
    .unwrap();
    assert_eq!(tree.unwrap().nodes[0].state, "running");
}
#[cfg(unix)]
#[test]
fn trial_child_symlink_cannot_read_foreign_transcript() {
    use std::os::unix::fs::symlink;
    let (root, paths, parent) = fixture();
    write(&parent, b"", now());
    let foreign = root.path().join("foreign.jsonl");
    write(&foreign, BODY.as_bytes(), now());
    let dir = parent.with_extension("").join("subagents");
    write(&dir.join("agent-a.meta.json"), META.as_bytes(), now());
    symlink(&foreign, dir.join("agent-a.jsonl")).unwrap();
    let (tree, _) = read(
        &parent,
        now(),
        State::default(),
        &ReadBudget::default(),
        &paths,
        now(),
    )
    .unwrap();
    assert!(tree.is_none());
}

// ---- 呼び直し (SendMessage + resumedAgentId) の合成ログ。値はすべて合成。 ----
fn ts(sec: u64) -> String {
    let t = now() + Duration::from_secs(sec);
    crate::proto::time::format_rfc3339(t).unwrap()
}
fn agent_launch(at: u64, id: &str) -> String {
    format!(
        "{{\"type\":\"assistant\",\"timestamp\":\"{}\",\"message\":{{\"content\":[{{\"type\":\"tool_use\",\"id\":\"{id}\",\"name\":\"Agent\"}}]}}}}\n{{\"type\":\"user\",\"timestamp\":\"{}\",\"toolUseResult\":{{\"isAsync\":true}},\"message\":{{\"content\":[{{\"type\":\"tool_result\",\"tool_use_id\":\"{id}\"}}]}}}}\n",
        ts(at),
        ts(at)
    )
}
fn notice(at: u64, id: &str, status: &str) -> String {
    let text = format!(
        "<task-notification><task-id>a</task-id><tool-use-id>{id}</tool-use-id><status>{status}</status><summary>synthetic</summary></task-notification>"
    );
    format!(
        "{{\"type\":\"queue-operation\",\"operation\":\"enqueue\",\"timestamp\":\"{}\",\"content\":\"{text}\"}}\n",
        ts(at)
    )
}
fn resume(at: u64, id: &str, agent: &str) -> String {
    format!(
        "{{\"type\":\"assistant\",\"timestamp\":\"{}\",\"message\":{{\"content\":[{{\"type\":\"tool_use\",\"id\":\"{id}\",\"name\":\"SendMessage\",\"input\":{{\"to\":\"{agent}\"}}}}]}}}}\n{{\"type\":\"user\",\"timestamp\":\"{}\",\"toolUseResult\":{{\"success\":true,\"message\":\"Resuming agent a\",\"resumedAgentId\":\"{agent}\",\"pin\":{{\"id\":\"{agent}\",\"name\":\"{agent}\",\"ref\":\"r\"}}}},\"message\":{{\"content\":[{{\"type\":\"tool_result\",\"tool_use_id\":\"{id}\"}}]}}}}\n",
        ts(at),
        ts(at)
    )
}
fn observe(
    paths: &RuntimePaths,
    parent: &Path,
    log: &str,
    state: State,
    at: u64,
) -> (String, i64, State) {
    let at = now() + Duration::from_secs(at);
    write(parent, log.as_bytes(), at);
    let (tree, state) = read(
        parent,
        now() - Duration::from_secs(3600),
        state,
        &ReadBudget::default(),
        paths,
        at,
    )
    .unwrap();
    let node = tree.unwrap().nodes.remove(0);
    (node.state, node.finished_at, state)
}
#[test]
fn resumed_agent_goes_running_done_running_done_by_each_notice() {
    let (_root, paths, parent) = fixture();
    child(&parent, "a", META, BODY, now());
    let mut log = agent_launch(1, "tool-a");
    let (s, _, st) = observe(&paths, &parent, &log, State::default(), 2);
    assert_eq!(s, "running");
    log += &notice(10, "tool-a", "completed");
    let (s, fin, st) = observe(&paths, &parent, &log, st, 11);
    assert_eq!(s, "done");
    assert_eq!(fin, parse_timestamp(&ts(10)));
    log += &resume(20, "tool-b", "a");
    let (s, _, st) = observe(&paths, &parent, &log, st, 21);
    assert_eq!(s, "running");
    log += &notice(30, "tool-b", "completed");
    let (s, fin, st) = observe(&paths, &parent, &log, st, 31);
    assert_eq!((s.as_str(), fin), ("done", parse_timestamp(&ts(30))));
    // 3 回目の呼び直しでも同じ（最新の実行で決める）。
    log += &resume(40, "tool-c", "a");
    let (s, _, st) = observe(&paths, &parent, &log, st, 41);
    assert_eq!(s, "running");
    log += &notice(50, "tool-c", "completed");
    let (s, fin, _) = observe(&paths, &parent, &log, st, 51);
    assert_eq!((s.as_str(), fin), ("done", parse_timestamp(&ts(50))));
    // 状態を引き継がず最後のログだけ読んでも同じ結果。
    let (s, fin, _) = observe(&paths, &parent, &log, State::default(), 52);
    assert_eq!((s.as_str(), fin), ("done", parse_timestamp(&ts(50))));
}
#[test]
fn resumed_run_with_failure_status_is_failed() {
    let (_root, paths, parent) = fixture();
    child(&parent, "a", META, BODY, now());
    let mut log = agent_launch(1, "tool-a") + &notice(10, "tool-a", "completed");
    let (s, _, st) = observe(&paths, &parent, &log, State::default(), 11);
    assert_eq!(s, "done");
    log += &resume(20, "tool-b", "a");
    let (s, _, st) = observe(&paths, &parent, &log, st, 21);
    assert_eq!(s, "running");
    log += &notice(30, "tool-b", "failed");
    let (s, fin, _) = observe(&paths, &parent, &log, st, 31);
    assert_eq!((s.as_str(), fin), ("failed", parse_timestamp(&ts(30))));
}
#[test]
fn newest_resume_wins_when_resumed_twice() {
    let (_root, paths, parent) = fixture();
    child(&parent, "a", META, BODY, now());
    let log = agent_launch(1, "tool-a")
        + &notice(10, "tool-a", "completed")
        + &resume(20, "tool-b", "a")
        + &notice(30, "tool-b", "completed")
        + &resume(40, "tool-c", "a");
    let (s, _, _) = observe(&paths, &parent, &log, State::default(), 41);
    assert_eq!(s, "running");
}
#[test]
fn resume_record_without_resumed_agent_id_changes_nothing() {
    let (_root, paths, parent) = fixture();
    child(&parent, "a", META, BODY, now());
    let plain = resume(20, "tool-b", "a").replace("resumedAgentId", "other");
    let log = agent_launch(1, "tool-a") + &notice(10, "tool-a", "completed") + &plain;
    let (s, fin, _) = observe(&paths, &parent, &log, State::default(), 21);
    assert_eq!((s.as_str(), fin), ("done", parse_timestamp(&ts(10))));
    // 型が違う値でも既存の信号（isAsync/status）の読み取りは壊れない。
    let odd = resume(20, "tool-b", "a").replace("\"resumedAgentId\":\"a\"", "\"resumedAgentId\":7");
    let log = agent_launch(1, "tool-a") + &notice(10, "tool-a", "completed") + &odd;
    let (s, _, _) = observe(&paths, &parent, &log, State::default(), 21);
    assert_eq!(s, "done");
}
#[test]
fn resume_leaving_the_parent_window_does_not_stay_running_forever() {
    // 呼び直しの記録が読み取り範囲（500 件）の外へ出ても、完了通知が範囲内なら done になる。
    let (_root, paths, parent) = fixture();
    child(&parent, "a", META, BODY, now());
    let mut log = agent_launch(1, "tool-a") + &notice(10, "tool-a", "completed");
    log += &resume(20, "tool-b", "a");
    let (s, _, st) = observe(&paths, &parent, &log, State::default(), 21);
    assert_eq!(s, "running");
    log += &"{}\n".repeat(300);
    log += &notice(30, "tool-b", "completed");
    log += &"{}\n".repeat(300);
    let (s, fin, _) = observe(&paths, &parent, &log, st, 31);
    assert_eq!((s.as_str(), fin), ("done", parse_timestamp(&ts(30))));
    // 呼び直しも完了通知も範囲外に出て、子のログも古いなら、running のまま残らない。
    let mut log = agent_launch(1, "tool-a") + &notice(10, "tool-a", "completed");
    log += &resume(20, "tool-b", "a");
    let (_root2, paths2, parent2) = fixture();
    child(&parent2, "a", META, BODY, now() - Duration::from_secs(1200));
    let (s, _, st2) = observe(&paths2, &parent2, &log, State::default(), 21);
    assert_eq!(s, "running"); // 呼び直し直後。子のログは古いが launched の信号がある。
    log += &"{}\n".repeat(600);
    let (s, _, _) = observe(&paths2, &parent2, &log, st2, 1300);
    assert_eq!(s, "done");
}
