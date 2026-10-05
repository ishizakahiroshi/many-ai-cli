use super::{ReadBudget, check_path, read_head, read_tail};
use crate::{
    config::RuntimePaths,
    proto::{
        self, SubagentNode, SubagentTree,
        time::{Timestamp, parse_rfc3339},
        wire::{Field, GoWire, Schema},
    },
};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    io,
    path::Path,
    time::{Duration, Instant, SystemTime},
};
#[derive(Default, Deserialize)]
#[serde(default)]
struct Meta {
    parent_session_id: String,
    subagent_type: String,
    description: String,
    status: String,
    started_at: String,
    completed_at: String,
    tool_calls: i64,
    effective_model_id: String,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Envelope {
    timestamp: i64,
    params: Params,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Params {
    update: Option<String>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Update {
    #[serde(rename = "sessionUpdate")]
    kind: String,
    subagent_id: String,
    subagent_type: String,
    description: String,
    model: String,
    child_session_id: String,
    status: String,
    tool_calls: i64,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Event {
    r#type: String,
    tool_name: String,
}
#[derive(Deserialize)]
#[serde(transparent)]
struct Kind(Update);
#[derive(Deserialize)]
#[serde(transparent)]
struct Spawn(Update);
#[derive(Deserialize)]
#[serde(transparent)]
struct Finish(Update);
macro_rules! fields{($(($name:literal,$kind:literal)),* $(,)?)=>{&[$(Field{name:$name,kind:$kind}),*]};}
const SCHEMAS: &[Schema] = &[
    Schema {
        name: "GrokMeta",
        fields: fields![
            ("parent_session_id", "string"),
            ("subagent_type", "string"),
            ("description", "string"),
            ("status", "string"),
            ("started_at", "string"),
            ("completed_at", "string"),
            ("tool_calls", "int"),
            ("effective_model_id", "string")
        ],
    },
    Schema {
        name: "GrokEnvelope",
        fields: fields![
            ("timestamp", "int64"),
            ("method", "string"),
            ("params", "GrokParams")
        ],
    },
    Schema {
        name: "GrokParams",
        fields: fields![("sessionId", "string"), ("update", "json.RawMessage")],
    },
    Schema {
        name: "GrokKind",
        fields: fields![("sessionUpdate", "string")],
    },
    Schema {
        name: "GrokSpawn",
        fields: fields![
            ("subagent_id", "string"),
            ("subagent_type", "string"),
            ("description", "string"),
            ("model", "string"),
            ("child_session_id", "string")
        ],
    },
    Schema {
        name: "GrokFinish",
        fields: fields![
            ("subagent_id", "string"),
            ("status", "string"),
            ("tool_calls", "int")
        ],
    },
    Schema {
        name: "GrokEvent",
        fields: fields![("type", "string"), ("tool_name", "string")],
    },
];
macro_rules! wire {
    ($ty:ty,$kind:literal) => {
        impl GoWire for $ty {
            const GO_TYPE: &'static str = $kind;
            const SCHEMAS: &'static [Schema] = SCHEMAS;
        }
    };
}
wire!(Meta, "GrokMeta");
wire!(Envelope, "GrokEnvelope");
wire!(Kind, "GrokKind");
wire!(Spawn, "GrokSpawn");
wire!(Finish, "GrokFinish");
wire!(Event, "GrokEvent");
#[derive(Clone, Default)]
struct Child {
    label: String,
    agent_type: String,
    model: String,
    child_session: String,
    started_at: i64,
    finished_at: i64,
    failed: bool,
    done: bool,
    started: bool,
    tool_calls: i64,
    tool_calls_known: bool,
    last_tool: String,
    last_activity: i64,
    events_modified: Option<SystemTime>,
    events_size: u64,
}
#[derive(Clone, Default)]
pub struct State {
    children: BTreeMap<String, Child>,
}
fn millis(at: Timestamp) -> i64 {
    i64::try_from(at.unix_nanos() / 1_000_000).unwrap_or(0)
}
fn parse_time(value: &str) -> i64 {
    parse_rfc3339(value).map(millis).unwrap_or(0)
}
fn modified_time(value: SystemTime) -> i64 {
    Timestamp::from_system_time(value).map(millis).unwrap_or(0)
}
fn updates(
    paths: &RuntimePaths,
    parent: &Path,
) -> (BTreeMap<String, Child>, BTreeMap<String, Child>) {
    let mut spawned = BTreeMap::new();
    let mut finished = BTreeMap::new();
    let deadline = Instant::now() + Duration::from_millis(100);
    let Ok(bytes) = read_tail(paths, parent, 2 * 1024 * 1024) else {
        return (spawned, finished);
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
        let Ok(env) = proto::decode_wire::<Envelope>(line) else {
            continue;
        };
        let Some(raw) = env.params.update else {
            continue;
        };
        let Ok(Kind(kind)) = proto::decode_wire::<Kind>(raw.as_bytes()) else {
            continue;
        };
        match kind.kind.as_str() {
            "subagent_spawned" => {
                let Ok(Spawn(spawn)) = proto::decode_wire::<Spawn>(raw.as_bytes()) else {
                    continue;
                };
                if spawn.subagent_id.is_empty() || spawned.contains_key(&spawn.subagent_id) {
                    continue;
                }
                spawned.insert(
                    spawn.subagent_id,
                    Child {
                        label: spawn.description,
                        agent_type: spawn.subagent_type,
                        model: spawn.model,
                        child_session: spawn.child_session_id,
                        started_at: env.timestamp.wrapping_mul(1000),
                        started: true,
                        ..Default::default()
                    },
                );
            }
            "subagent_finished" => {
                let Ok(Finish(finish)) = proto::decode_wire::<Finish>(raw.as_bytes()) else {
                    continue;
                };
                if finish.subagent_id.is_empty() || finished.contains_key(&finish.subagent_id) {
                    continue;
                }
                finished.insert(
                    finish.subagent_id,
                    Child {
                        failed: finish.status != "completed",
                        tool_calls: finish.tool_calls,
                        finished_at: env.timestamp.wrapping_mul(1000),
                        done: true,
                        tool_calls_known: true,
                        ..Default::default()
                    },
                );
            }
            _ => {}
        }
    }
    (spawned, finished)
}
fn last_tool(paths: &RuntimePaths, path: &Path, cap: u64) -> String {
    let deadline = Instant::now() + Duration::from_millis(100);
    let Ok(bytes) = read_tail(paths, path, if cap == 0 { 128 * 1024 } else { cap }) else {
        return String::new();
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
        let Ok(event) = proto::decode_wire::<Event>(line) else {
            continue;
        };
        if !event.tool_name.is_empty()
            && matches!(event.r#type.as_str(), "tool_started" | "tool_completed")
        {
            return event.tool_name;
        }
    }
    String::new()
}
fn node(id: String, child: &Child) -> SubagentNode {
    SubagentNode {
        id,
        depth: 1,
        label: child.label.clone(),
        agent_type: child.agent_type.clone(),
        model: child.model.clone(),
        state: if child.done {
            if child.failed { "failed" } else { "done" }
        } else if child.started {
            "running"
        } else {
            "unknown"
        }
        .into(),
        started_at: child.started_at,
        finished_at: child.finished_at,
        last_activity_at: child.last_activity,
        last_tool_name: child.last_tool.clone(),
        tool_calls: if child.tool_calls_known {
            child.tool_calls
        } else {
            0
        },
        ..Default::default()
    }
}
pub fn read(
    parent: &Path,
    since: Timestamp,
    mut state: State,
    budget: &ReadBudget,
    paths: &RuntimePaths,
    now: Timestamp,
) -> io::Result<(Option<SubagentTree>, State)> {
    let Some(session_dir) = parent.parent() else {
        return Ok((None, state));
    };
    let Some(sessions_root) = session_dir.parent() else {
        return Ok((None, state));
    };
    let parent_id = session_dir
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let directory = session_dir.join("subagents");
    if check_path(paths, &directory).is_err() {
        return Ok((None, state));
    }
    let Ok(entries) = super::entries(paths, &directory) else {
        return Ok((None, state));
    };
    let mut entries = entries.into_iter().filter(|e| e.is_dir).collect::<Vec<_>>();
    entries.sort_by_key(|e| e.name.clone());
    let need = entries
        .iter()
        .any(|e| state.children.get(&e.name).is_none_or(|child| !child.done));
    let (spawned, finished) = if need {
        updates(paths, parent)
    } else {
        Default::default()
    };
    let mut next = BTreeMap::new();
    let mut nodes = BTreeMap::new();
    for entry in entries {
        let id = entry.name;
        if id.is_empty() {
            continue;
        }
        if let Some(child) = state.children.get(&id).filter(|child| child.done) {
            next.insert(id.clone(), child.clone());
            nodes.insert(id.clone(), node(id, child));
            continue;
        }
        let meta_path = directory.join(&id).join("meta.json");
        let meta = check_path(paths, &meta_path)
            .ok()
            .and_then(|_| super::artifact_metadata(paths, &meta_path).ok())
            .and_then(|info| {
                let modified = info.modified().ok()?;
                let bytes = read_head(paths, &meta_path, u64::MAX).ok()?;
                let meta = proto::decode_wire::<Meta>(&bytes).ok()?;
                Some((meta, modified))
            });
        if let Some((meta, modified)) = meta {
            if !meta.parent_session_id.is_empty() && meta.parent_session_id != parent_id {
                continue;
            }
            let child = Child {
                label: meta.description,
                agent_type: meta.subagent_type,
                model: meta.effective_model_id,
                started_at: parse_time(&meta.started_at),
                finished_at: parse_time(&meta.completed_at),
                failed: meta.status != "completed",
                done: true,
                started: true,
                tool_calls: meta.tool_calls,
                tool_calls_known: true,
                last_activity: modified_time(modified),
                ..Default::default()
            };
            nodes.insert(id.clone(), node(id.clone(), &child));
            next.insert(id, child);
            continue;
        }
        let prior = state.children.get(&id);
        let mut child = spawned
            .get(&id)
            .cloned()
            .or_else(|| prior.cloned())
            .unwrap_or_default();
        // Carry event mtime/tool cache regardless of whether a fresh spawn was seen.
        if let Some(prior) = prior {
            child.events_modified = prior.events_modified;
            child.events_size = prior.events_size;
            child.last_tool = prior.last_tool.clone();
            child.last_activity = prior.last_activity;
        }
        if !child.child_session.is_empty() {
            let events = sessions_root
                .join(&child.child_session)
                .join("events.jsonl");
            if check_path(paths, &events).is_ok()
                && let Ok(info) = super::artifact_metadata(paths, &events)
                && let Ok(modified) = info.modified()
            {
                if child.events_modified != Some(modified) || child.events_size != info.len() {
                    child.last_tool = last_tool(paths, &events, budget.tail_bytes);
                    child.events_modified = Some(modified);
                    child.events_size = info.len()
                }
                child.last_activity = modified_time(modified);
            }
        }
        if let Some(finish) = finished.get(&id) {
            child.done = true;
            child.failed = finish.failed;
            child.finished_at = finish.finished_at;
            child.tool_calls = finish.tool_calls;
            child.tool_calls_known = true
        }
        nodes.insert(id.clone(), node(id.clone(), &child));
        next.insert(id, child);
    }
    state.children = next;
    let since = millis(since);
    nodes.retain(|_, node| node.state == "running" || node.started_at >= since);
    let mut omitted = 0;
    while nodes.len() > budget.max_nodes {
        let Some(victim) = super::claude::pick_drop_victim(&nodes) else {
            break;
        };
        nodes.remove(&victim);
        omitted += 1
    }
    let mut nodes = nodes.into_values().collect::<Vec<_>>();
    nodes.sort_by(|a, b| {
        a.started_at
            .cmp(&b.started_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    let tree = (!nodes.is_empty()).then(|| SubagentTree {
        provider: "subagent:grok-v1".into(),
        nodes,
        omitted,
        updated_at: millis(now),
    });
    Ok((tree, state))
}
#[cfg(test)]
mod tests;
