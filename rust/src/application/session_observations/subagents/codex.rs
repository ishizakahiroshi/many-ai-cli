use super::{ReadBudget, check_path, read_tail};
use crate::{
    config::RuntimePaths,
    proto::{
        self, SubagentNode, SubagentTree,
        time::{Timestamp, parse_rfc3339, utc},
        wire::{Field, GoWire, Schema},
    },
};
use chrono::{Datelike, Local};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, BufRead, Read},
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};
#[derive(Clone, Default, Deserialize)]
#[serde(default)]
struct Meta {
    r#type: String,
    payload: Payload,
}
#[derive(Clone, Default, Deserialize)]
#[serde(default)]
struct Payload {
    id: String,
    timestamp: String,
    source: Source,
    r#type: String,
    item: Item,
    completed_at_ms: i64,
    name: String,
    input: Option<String>,
}
#[derive(Clone, Default, Deserialize)]
#[serde(default)]
struct Source {
    subagent: Subagent,
}
#[derive(Clone, Default, Deserialize)]
#[serde(default)]
struct Subagent {
    thread_spawn: Option<Spawn>,
}
#[derive(Clone, Default, Deserialize)]
#[serde(default)]
struct Spawn {
    agent_nickname: String,
    agent_role: String,
    parent_thread_id: String,
    depth: i64,
}
#[derive(Clone, Default, Deserialize)]
#[serde(default)]
struct Item {
    r#type: String,
    agent_thread_id: String,
    kind: String,
}
#[derive(Deserialize)]
#[serde(transparent)]
struct SignalWire(Meta);
#[derive(Deserialize)]
#[serde(transparent)]
struct ToolWire(Meta);
const SCHEMAS: &[Schema] = &[
    Schema {
        name: "CodexSignal",
        fields: &[
            Field {
                name: "type",
                kind: "string",
            },
            Field {
                name: "payload",
                kind: "CodexSignalPayload",
            },
        ],
    },
    Schema {
        name: "CodexSignalPayload",
        fields: &[
            Field {
                name: "type",
                kind: "string",
            },
            Field {
                name: "item",
                kind: "CodexObservationItem",
            },
            Field {
                name: "completed_at_ms",
                kind: "int64",
            },
        ],
    },
    Schema {
        name: "CodexTool",
        fields: &[
            Field {
                name: "type",
                kind: "string",
            },
            Field {
                name: "payload",
                kind: "CodexToolPayload",
            },
        ],
    },
    Schema {
        name: "CodexToolPayload",
        fields: &[
            Field {
                name: "type",
                kind: "string",
            },
            Field {
                name: "name",
                kind: "string",
            },
            Field {
                name: "input",
                kind: "json.RawMessage",
            },
        ],
    },
    Schema {
        name: "CodexObservation",
        fields: &[
            Field {
                name: "type",
                kind: "string",
            },
            Field {
                name: "payload",
                kind: "CodexObservationPayload",
            },
        ],
    },
    Schema {
        name: "CodexObservationPayload",
        fields: &[
            Field {
                name: "id",
                kind: "string",
            },
            Field {
                name: "timestamp",
                kind: "string",
            },
            Field {
                name: "source",
                kind: "CodexObservationSource",
            },
        ],
    },
    Schema {
        name: "CodexObservationSource",
        fields: &[Field {
            name: "subagent",
            kind: "CodexObservationSubagent",
        }],
    },
    Schema {
        name: "CodexObservationSubagent",
        fields: &[Field {
            name: "thread_spawn",
            kind: "*CodexObservationSpawn",
        }],
    },
    Schema {
        name: "CodexObservationSpawn",
        fields: &[
            Field {
                name: "agent_nickname",
                kind: "string",
            },
            Field {
                name: "agent_role",
                kind: "string",
            },
            Field {
                name: "parent_thread_id",
                kind: "string",
            },
            Field {
                name: "depth",
                kind: "int",
            },
        ],
    },
    Schema {
        name: "CodexObservationItem",
        fields: &[
            Field {
                name: "type",
                kind: "string",
            },
            Field {
                name: "agent_thread_id",
                kind: "string",
            },
            Field {
                name: "kind",
                kind: "string",
            },
        ],
    },
];
impl GoWire for Meta {
    const GO_TYPE: &'static str = "CodexObservation";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
impl GoWire for SignalWire {
    const GO_TYPE: &'static str = "CodexSignal";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
impl GoWire for ToolWire {
    const GO_TYPE: &'static str = "CodexTool";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
#[derive(Clone, Default)]
struct Child {
    path: PathBuf,
    nickname: String,
    role: String,
    parent: String,
    depth: i64,
    started: i64,
    modified: Option<SystemTime>,
    size: u64,
    last_tool: String,
    last_summary: String,
    seen_started: bool,
    terminal: bool,
    failed: bool,
    finished: i64,
    first_decided: bool,
    first_running: bool,
}
#[derive(Clone, Default)]
pub struct State {
    children: BTreeMap<String, Child>,
    days: BTreeMap<PathBuf, SystemTime>,
}
#[derive(Default)]
struct Signal {
    started: bool,
    terminal: bool,
    failed: bool,
    finished: i64,
}
fn millis(value: Timestamp) -> i64 {
    i64::try_from(value.unix_nanos() / 1_000_000).unwrap_or(0)
}
fn parse_time(value: &str) -> i64 {
    parse_rfc3339(value).map(millis).unwrap_or(0)
}
fn modified_millis(value: SystemTime) -> i64 {
    Timestamp::from_system_time(value).map(millis).unwrap_or(0)
}
fn metadata(paths: &RuntimePaths, path: &Path) -> Option<Meta> {
    let file = super::open_artifact(paths, path).ok()?;
    let mut reader = io::BufReader::new(file.take(4 * 1024 * 1024));
    let mut first = Vec::new();
    reader.read_until(b'\n', &mut first).ok()?;
    if first.len() >= 4 * 1024 * 1024 {
        return None;
    }
    let meta = proto::decode_wire::<Meta>(&first).ok()?;
    (meta.r#type == "session_meta").then_some(meta)
}
fn root(path: &Path) -> Option<PathBuf> {
    let mut dir = path.parent()?;
    for _ in 0..8 {
        if dir.file_name().is_some_and(|v| v == "sessions") {
            return Some(dir.into());
        }
        dir = dir.parent()?
    }
    None
}
fn connected(root: &str, children: &BTreeMap<String, Child>) -> BTreeSet<String> {
    let mut result = BTreeSet::new();
    loop {
        let mut changed = false;
        for (id, child) in children {
            if child.parent == root || result.contains(&child.parent) {
                changed |= result.insert(id.clone());
            }
        }
        if !changed {
            return result;
        }
    }
}
fn scan_days(
    paths: &RuntimePaths,
    root: &Path,
    from: Timestamp,
    now: Timestamp,
    state: &mut State,
) {
    let (Ok(from), Ok(now)) = (utc(from), utc(now)) else {
        return;
    };
    let mut start = from.with_timezone(&Local).date_naive();
    let mut end = now.with_timezone(&Local).date_naive();
    if end < start {
        std::mem::swap(&mut start, &mut end)
    }
    let today = now.with_timezone(&Local).date_naive();
    for i in 0..32 {
        let Some(day) = start.checked_add_days(chrono::Days::new(i)) else {
            break;
        };
        if day > end {
            break;
        }
        let dir = root
            .join(format!("{:04}", day.year()))
            .join(format!("{:02}", day.month()))
            .join(format!("{:02}", day.day()));
        if check_path(paths, &dir).is_err() {
            continue;
        }
        let Ok(info) = super::directory_metadata(paths, &dir) else {
            continue;
        };
        let Ok(modified) = info.modified() else {
            continue;
        };
        if state
            .days
            .get(&dir)
            .is_some_and(|prior| day != today || *prior == modified)
        {
            continue;
        }
        let Ok(entries) = super::entries(paths, &dir) else {
            continue;
        };
        for entry in entries {
            let path = dir.join(&entry.name);
            if entry.is_dir
                || !entry.name.ends_with(".jsonl")
                || state.children.values().any(|child| child.path == path)
            {
                continue;
            }
            let Some(meta) = metadata(paths, &path) else {
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
                    started: parse_time(&meta.payload.timestamp),
                    ..Default::default()
                },
            );
        }
        state.days.insert(dir, modified);
    }
}
fn signals(
    paths: &RuntimePaths,
    path: &Path,
    nodes: &BTreeMap<String, SubagentNode>,
) -> BTreeMap<String, Signal> {
    let mut signals = BTreeMap::<String, Signal>::new();
    let deadline = Instant::now() + Duration::from_millis(100);
    let Ok(bytes) = read_tail(paths, path, 2 * 1024 * 1024) else {
        return signals;
    };
    for line in bytes
        .split(|b| *b == b'\n')
        .rev()
        .filter(|line| !line.is_empty())
        .take(500)
    {
        if Instant::now() >= deadline {
            break;
        }
        let Ok(SignalWire(meta)) = proto::decode_wire::<SignalWire>(line) else {
            continue;
        };
        if meta.r#type != "event_msg"
            || meta.payload.r#type != "item_completed"
            || meta.payload.item.r#type != "SubAgentActivity"
        {
            continue;
        }
        let id = meta.payload.item.agent_thread_id;
        if id.is_empty() || !nodes.contains_key(&id) {
            continue;
        }
        let signal = signals.entry(id).or_default();
        match meta.payload.item.kind.as_str() {
            "started" => signal.started = true,
            "completed" | "interrupted" => {
                if !signal.terminal {
                    signal.terminal = true;
                    signal.failed = meta.payload.item.kind == "interrupted";
                    signal.finished = meta.payload.completed_at_ms
                }
            }
            _ => {}
        }
    }
    signals
}
fn last_tool(paths: &RuntimePaths, path: &Path, cap: u64) -> (String, String) {
    let deadline = Instant::now() + Duration::from_millis(100);
    let Ok(bytes) = read_tail(paths, path, if cap == 0 { 128 * 1024 } else { cap }) else {
        return Default::default();
    };
    for line in bytes
        .split(|b| *b == b'\n')
        .rev()
        .filter(|line| !line.is_empty())
        .take(512)
    {
        if Instant::now() >= deadline {
            break;
        }
        let Ok(ToolWire(meta)) = proto::decode_wire::<ToolWire>(line) else {
            continue;
        };
        if meta.r#type != "response_item" {
            continue;
        }
        match meta.payload.r#type.as_str() {
            "custom_tool_call" => {
                let summary = meta
                    .payload
                    .input
                    .as_ref()
                    .and_then(|v| serde_json::from_str::<String>(v).ok())
                    .map(|v| super::claude::clamp_summary(&v))
                    .unwrap_or_default();
                return (meta.payload.name, summary);
            }
            "function_call" => return (meta.payload.name, String::new()),
            _ => {}
        }
    }
    Default::default()
}
pub fn read(
    parent: &Path,
    since: Timestamp,
    mut state: State,
    budget: &ReadBudget,
    paths: &RuntimePaths,
    now: Timestamp,
) -> io::Result<(Option<SubagentTree>, State)> {
    let Some(parent_meta) = metadata(paths, parent) else {
        return Ok((None, state));
    };
    let root_id = parent_meta.payload.id;
    if root_id.is_empty() {
        return Ok((None, state));
    }
    let Some(sessions_root) = root(parent) else {
        return Ok((None, state));
    };
    let from = parse_rfc3339(&parent_meta.payload.timestamp)
        .ok()
        .filter(|at| millis(*at) > 0)
        .unwrap_or(now);
    scan_days(paths, &sessions_root, from, now, &mut state);
    let connected = connected(&root_id, &state.children);
    let mut nodes = BTreeMap::<String, SubagentNode>::new();
    for id in connected {
        let child = state.children.get_mut(&id).unwrap();
        if check_path(paths, &child.path).is_err() {
            continue;
        }
        let Ok(info) = super::artifact_metadata(paths, &child.path) else {
            continue;
        };
        let Ok(modified) = info.modified() else {
            continue;
        };
        if child.modified != Some(modified) || child.size != info.len() {
            (child.last_tool, child.last_summary) =
                last_tool(paths, &child.path, budget.tail_bytes);
            child.modified = Some(modified);
            child.size = info.len();
        }
        let label = if !child.nickname.is_empty() {
            child.nickname.clone()
        } else {
            child.role.clone()
        };
        nodes.insert(
            id.clone(),
            SubagentNode {
                id,
                parent_id: if child.parent == root_id {
                    String::new()
                } else {
                    child.parent.clone()
                },
                depth: child.depth,
                label,
                started_at: child.started,
                last_activity_at: modified_millis(modified),
                last_tool_name: child.last_tool.clone(),
                last_tool_summary: child.last_summary.clone(),
                ..Default::default()
            },
        );
    }
    if nodes.is_empty() {
        return Ok((None, state));
    }
    let signals = signals(paths, parent, &nodes);
    for (id, node) in &mut nodes {
        let child = state.children.get_mut(id).unwrap();
        if let Some(signal) = signals.get(id) {
            if signal.started {
                child.seen_started = true
            }
            if signal.terminal && !child.terminal {
                child.terminal = true;
                child.failed = signal.failed;
                child.finished = signal.finished
            }
        }
        node.state = if child.terminal {
            node.finished_at = child.finished;
            if child.failed { "failed" } else { "done" }
        } else if child.seen_started {
            "running"
        } else {
            if !child.first_decided {
                child.first_decided = true;
                child.first_running = millis(now) - node.last_activity_at <= 600_000
            }
            if child.first_running {
                "running"
            } else {
                "unknown"
            }
        }
        .into();
    }
    super::claude::drop_orphans(&mut nodes);
    let since_ms = millis(since);
    nodes.retain(|_, node| node.state == "running" || node.started_at >= since_ms);
    super::claude::drop_orphans(&mut nodes);
    let mut omitted = 0;
    while nodes.len() > budget.max_nodes {
        let Some(victim) = super::claude::pick_drop_victim(&nodes) else {
            break;
        };
        nodes.remove(&victim);
        omitted += 1;
        let before = nodes.len();
        super::claude::drop_orphans(&mut nodes);
        omitted += (before - nodes.len()) as i64;
    }
    let mut nodes = nodes.into_values().collect::<Vec<_>>();
    nodes.sort_by(|a, b| {
        a.started_at
            .cmp(&b.started_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    let tree = (!nodes.is_empty()).then(|| SubagentTree {
        provider: "subagent:codex-v1".into(),
        nodes,
        omitted,
        updated_at: millis(now),
    });
    Ok((tree, state))
}
#[cfg(test)]
mod tests;
