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
