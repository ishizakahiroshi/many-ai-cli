use super::*;
use chrono::{DateTime, Datelike, Local};
use serde_json::value::RawValue;
use std::{collections::BTreeSet, io::BufRead};

#[derive(Default, Deserialize)]
#[serde(default)]
struct Meta {
    #[serde(rename = "type", deserialize_with = "null_default")]
    kind: String,
    #[serde(deserialize_with = "null_default")]
    payload: Identity,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Identity {
    #[serde(deserialize_with = "null_default")]
    id: String,
    #[serde(deserialize_with = "null_default")]
    timestamp: String,
    #[serde(deserialize_with = "null_default")]
    source: Source,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Source {
    #[serde(deserialize_with = "null_default")]
    subagent: Subagent,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Subagent {
    thread_spawn: Option<Spawn>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Spawn {
    #[serde(deserialize_with = "null_default")]
    agent_nickname: String,
    #[serde(deserialize_with = "null_default")]
    agent_role: String,
    #[serde(deserialize_with = "null_default")]
    parent_thread_id: String,
    #[serde(deserialize_with = "null_default")]
    depth: i64,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Record {
    #[serde(rename = "type", deserialize_with = "null_default")]
    kind: String,
    #[serde(deserialize_with = "null_default")]
    payload: Payload,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Payload {
    #[serde(rename = "type", deserialize_with = "null_default")]
    kind: String,
    #[serde(deserialize_with = "null_default")]
    completed_at_ms: i64,
    #[serde(deserialize_with = "null_default")]
    item: Item,
    #[serde(deserialize_with = "null_default")]
    name: String,
    input: Option<Box<RawValue>>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Item {
    #[serde(rename = "type", deserialize_with = "null_default")]
    kind: String,
    #[serde(rename = "kind", deserialize_with = "null_default")]
    activity: String,
    #[serde(deserialize_with = "null_default")]
    agent_thread_id: String,
}
#[derive(Clone, Default)]
struct Child {
    path: PathBuf,
    nickname: String,
    role: String,
    parent: String,
    depth: i64,
    started_at: i64,
    stamp: Stamp,
    last_tool_name: String,
    last_tool_summary: String,
    seen_started: bool,
    terminal: bool,
    failed: bool,
    finished_at: i64,
    first_running: Option<bool>,
}
#[derive(Clone, Default)]
pub struct CodexState {
    parent: PathBuf,
    children: BTreeMap<String, Child>,
    day_dirs: BTreeMap<PathBuf, Option<Timestamp>>,
}
#[derive(Default)]
struct Signal {
    started: bool,
    terminal: bool,
    failed: bool,
    finished_at: i64,
}
fn read_identity(path: &Path, stats: &mut ReadStats) -> Option<Meta> {
    let mut reader = io::BufReader::with_capacity(
        64 * 1024,
        File::open(path).ok()?.take(METADATA_BYTES as u64),
    );
    let mut line = vec![];
    reader.read_until(b'\n', &mut line).ok()?;
    stats.metadata_bytes += line.len();
    stats.files_read += 1;
    let meta: Meta = serde_json::from_slice(&line).ok()?;
    (meta.kind == "session_meta").then_some(meta)
}
fn sessions_root(parent: &Path) -> Option<PathBuf> {
    parent
        .parent()?
        .ancestors()
        .take(8)
        .find(|p| p.file_name().is_some_and(|n| n == "sessions"))
        .map(Path::to_path_buf)
}
pub(super) fn read(
    parent: &Path,
    since: i64,
    mut state: CodexState,
    budget: ReadBudget,
    now: Timestamp,
    stats: &mut ReadStats,
) -> io::Result<(Option<SubagentTree>, CodexState)> {
    if state.parent != parent {
        state = CodexState {
            parent: parent.into(),
            ..Default::default()
        };
    }
    let Some(meta) = read_identity(parent, stats).filter(|m| !m.payload.id.is_empty()) else {
        return Ok((None, state));
    };
    let Some(root) = sessions_root(parent) else {
        return Ok((None, state));
    };
    let root_id = meta.payload.id;
    let start = crate::proto::time::parse_rfc3339(&meta.payload.timestamp)
        .ok()
        .filter(|t| millis(*t) > 0)
        .unwrap_or(now);
    scan_days(&root, start, now, &mut state, stats);
    let mut connected = BTreeSet::new();
    loop {
        let before = connected.len();
        for (id, child) in &state.children {
            if child.parent == root_id || connected.contains(&child.parent) {
                connected.insert(id.clone());
            }
        }
        if before == connected.len() {
            break;
        }
    }
    let mut nodes = BTreeMap::new();
    for id in connected {
        let child = state.children.get_mut(&id).expect("connected cache");
        let Ok(stamp) = Stamp::read(&child.path) else {
            continue;
        };
        if child.stamp != stamp {
            let (name, summary) = last_tool(&child.path, budget.tail_bytes, stats);
            child.last_tool_name = name;
            child.last_tool_summary = summary;
            child.stamp = stamp;
        }
        let node = SubagentNode {
            id: id.clone(),
            parent_id: if child.parent != root_id {
                child.parent.clone()
            } else {
                String::new()
            },
            depth: child.depth,
            label: if child.nickname.is_empty() {
                child.role.clone()
            } else {
                child.nickname.clone()
            },
            started_at: child.started_at,
            last_activity_at: stamp.millis(),
            last_tool_name: child.last_tool_name.clone(),
            last_tool_summary: child.last_tool_summary.clone(),
            ..Default::default()
        };
        nodes.insert(id, node);
    }
    let signals = scan_signals(parent, &nodes, stats);
    for (id, node) in &mut nodes {
        let child = state.children.get_mut(id).expect("node cache");
        if let Some(sig) = signals.get(id) {
            child.seen_started |= sig.started;
            if sig.terminal && !child.terminal {
                child.terminal = true;
                child.failed = sig.failed;
                child.finished_at = sig.finished_at;
            }
        }
        node.state = if child.terminal && child.failed {
            "failed"
        } else if child.terminal {
            "done"
        } else if child.seen_started
            || *child.first_running.get_or_insert_with(|| {
                millis(now).saturating_sub(node.last_activity_at) <= FRESH_MS
            })
        {
            "running"
        } else {
            "unknown"
        }
        .into();
        if child.terminal {
            node.finished_at = child.finished_at;
        }
    }
    Ok((
        Some(finish_tree(
            Adapter::Codex,
            nodes,
            since,
            budget.max_nodes,
            millis(now),
        )),
        state,
    ))
}
fn scan_days(
    root: &Path,
    from: Timestamp,
    to: Timestamp,
    state: &mut CodexState,
    stats: &mut ReadStats,
) {
    let (Ok(from), Ok(to)) = (crate::proto::time::utc(from), crate::proto::time::utc(to)) else {
        return;
    };
    let from: DateTime<Local> = from.with_timezone(&Local);
    let to: DateTime<Local> = to.with_timezone(&Local);
    let today = to.date_naive();
    let (from, to) = if from > to { (to, from) } else { (from, to) };
    let mut day = from.date_naive();
    for _ in 0..32 {
        if day > to.date_naive() {
            break;
        }
        let path = root
            .join(format!("{:04}", day.year()))
            .join(format!("{:02}", day.month()))
            .join(format!("{:02}", day.day()));
        if let Ok(info) = fs::metadata(&path) {
            let stamp = info
                .modified()
                .ok()
                .and_then(|at| Timestamp::from_system_time(at).ok());
            let skip = state
                .day_dirs
                .get(&path)
                .is_some_and(|old| day != today || old == &stamp);
            if !skip && let Ok(entries) = list(&path) {
                for entry in entries {
                    let path = entry.path();
                    if entry.file_type().is_ok_and(|t| t.is_dir())
                        || path.extension().is_none_or(|e| e != "jsonl")
                        || state.children.values().any(|c| c.path == path)
                    {
                        continue;
                    }
                    let Some(meta) = read_identity(&path, stats) else {
                        continue;
                    };
                    let Some(spawn) = meta.payload.source.subagent.thread_spawn else {
                        continue;
                    };
                    if meta.payload.id.is_empty() || spawn.parent_thread_id.is_empty() {
                        continue;
                    }
                    state.children.insert(
                        meta.payload.id,
                        Child {
                            path,
                            nickname: spawn.agent_nickname,
                            role: spawn.agent_role,
                            parent: spawn.parent_thread_id,
                            depth: spawn.depth.max(1),
                            started_at: parse_timestamp(&meta.payload.timestamp),
                            ..Default::default()
                        },
                    );
                }
                state.day_dirs.insert(path, stamp);
            }
        }
        let Some(next) = day.succ_opt() else {
            break;
        };
        day = next;
    }
}
fn scan_signals(
    path: &Path,
    nodes: &BTreeMap<String, SubagentNode>,
    stats: &mut ReadStats,
) -> BTreeMap<String, Signal> {
    let mut signals: BTreeMap<String, Signal> = BTreeMap::new();
    if nodes.is_empty() {
        return signals;
    }
    for line in read_tail(path, PARENT_BYTES, true, stats) {
        let Ok(record) = serde_json::from_slice::<Record>(&line) else {
            continue;
        };
        let p = record.payload;
        if record.kind != "event_msg"
            || p.kind != "item_completed"
            || p.item.kind != "SubAgentActivity"
            || !nodes.contains_key(&p.item.agent_thread_id)
        {
            continue;
        }
        let s = signals.entry(p.item.agent_thread_id).or_default();
        match p.item.activity.as_str() {
            "started" => s.started = true,
            "completed" | "interrupted" if !s.terminal => {
                s.terminal = true;
                s.failed = p.item.activity == "interrupted";
                s.finished_at = p.completed_at_ms;
            }
            _ => {}
        }
    }
    signals
}
fn last_tool(path: &Path, cap: usize, stats: &mut ReadStats) -> (String, String) {
    for line in read_tail(path, cap, false, stats) {
        let Ok(record) = serde_json::from_slice::<Record>(&line) else {
            continue;
        };
        let p = record.payload;
        if record.kind != "response_item" {
            continue;
        }
        match p.kind.as_str() {
            "custom_tool_call" => {
                let text = p
                    .input
                    .as_deref()
                    .and_then(|v| serde_json::from_str::<String>(v.get()).ok())
                    .map(|s| clamp_summary(&s))
                    .unwrap_or_default();
                return (p.name, text);
            }
            "function_call" => return (p.name, String::new()),
            _ => {}
        }
    }
    (String::new(), String::new())
}
