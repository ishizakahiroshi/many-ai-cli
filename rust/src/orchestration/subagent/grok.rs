use super::*;
#[derive(Default, Deserialize)]
#[serde(default)]
struct Meta {
    #[serde(deserialize_with = "null_default")]
    parent_session_id: String,
    #[serde(deserialize_with = "null_default")]
    subagent_type: String,
    #[serde(deserialize_with = "null_default")]
    description: String,
    #[serde(deserialize_with = "null_default")]
    status: String,
    #[serde(deserialize_with = "null_default")]
    started_at: String,
    #[serde(deserialize_with = "null_default")]
    completed_at: String,
    #[serde(deserialize_with = "null_default")]
    tool_calls: i64,
    #[serde(deserialize_with = "null_default")]
    effective_model_id: String,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Envelope {
    #[serde(deserialize_with = "null_default")]
    timestamp: i64,
    #[serde(deserialize_with = "null_default")]
    params: Params,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Params {
    #[serde(deserialize_with = "null_default")]
    update: Update,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Update {
    #[serde(rename = "sessionUpdate", deserialize_with = "null_default")]
    kind: String,
    #[serde(deserialize_with = "null_default")]
    subagent_id: String,
    #[serde(deserialize_with = "null_default")]
    subagent_type: String,
    #[serde(deserialize_with = "null_default")]
    description: String,
    #[serde(deserialize_with = "null_default")]
    model: String,
    #[serde(deserialize_with = "null_default")]
    child_session_id: String,
    #[serde(deserialize_with = "null_default")]
    status: String,
    #[serde(deserialize_with = "null_default")]
    tool_calls: i64,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Event {
    #[serde(rename = "type", deserialize_with = "null_default")]
    kind: String,
    #[serde(deserialize_with = "null_default")]
    tool_name: String,
}
#[derive(Clone, Default)]
struct Child {
    node: SubagentNode,
    child_session_id: String,
    done: bool,
    started: bool,
    failed: bool,
    stamp: Stamp,
}
impl Child {
    fn node(&self, id: &str) -> SubagentNode {
        let mut n = self.node.clone();
        n.id = id.into();
        n.depth = 1;
        n.state = if self.done && self.failed {
            "failed"
        } else if self.done {
            "done"
        } else if self.started {
            "running"
        } else {
            "unknown"
        }
        .into();
        n
    }
}
#[derive(Clone, Default)]
pub struct GrokState {
    parent: PathBuf,
    children: BTreeMap<String, Child>,
}
struct Finish {
    failed: bool,
    tool_calls: i64,
    at: i64,
}
pub(super) fn read(
    parent: &Path,
    since: i64,
    mut state: GrokState,
    budget: ReadBudget,
    now: i64,
    stats: &mut ReadStats,
) -> io::Result<(Option<SubagentTree>, GrokState)> {
    if state.parent != parent {
        state = GrokState {
            parent: parent.into(),
            ..Default::default()
        };
    }
    let Some(session) = parent.parent() else {
        return Ok((None, state));
    };
    let Some(root) = session.parent() else {
        return Ok((None, state));
    };
    let parent_id = session.file_name().unwrap_or_default().to_string_lossy();
    let entries = match list(&session.join("subagents")) {
        Ok(e) => e,
        Err(_) => return Ok((None, state)),
    };
    let need_signals = entries
        .iter()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .any(|e| {
            state
                .children
                .get(e.file_name().to_string_lossy().as_ref())
                .is_none_or(|c| !c.done)
        });
    let (spawned, finished) = if need_signals {
        scan_updates(parent, stats)
    } else {
        (BTreeMap::new(), BTreeMap::new())
    };
    let mut next = BTreeMap::new();
    let mut nodes = BTreeMap::new();
    for e in entries {
        if !e.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let id = e.file_name().to_string_lossy().into_owned();
        if !component(&id) {
            continue;
        }
        let cached = state.children.get(&id);
        if let Some(c) = cached.filter(|c| c.done) {
            next.insert(id.clone(), c.clone());
            nodes.insert(id.clone(), c.node(&id));
            continue;
        }
        let meta_path = e.path().join("meta.json");
        if let Some(meta) = read_meta::<Meta>(&meta_path, stats) {
            if !meta.parent_session_id.is_empty() && meta.parent_session_id != parent_id {
                continue;
            }
            let c = Child {
                node: SubagentNode {
                    label: meta.description,
                    agent_type: meta.subagent_type,
                    model: meta.effective_model_id,
                    started_at: parse_timestamp(&meta.started_at),
                    finished_at: parse_timestamp(&meta.completed_at),
                    tool_calls: meta.tool_calls,
                    last_activity_at: Stamp::read(&meta_path).map(Stamp::millis).unwrap_or(0),
                    ..Default::default()
                },
                done: true,
                started: true,
                failed: meta.status != "completed",
                ..Default::default()
            };
            nodes.insert(id.clone(), c.node(&id));
            next.insert(id, c);
            continue;
        }
        let mut child = if let Some(spawn) = spawned.get(&id) {
            let mut c = spawn.clone();
            if let Some(old) = cached {
                c.node.last_activity_at = old.node.last_activity_at;
                c.node.last_tool_name = old.node.last_tool_name.clone();
                c.stamp = old.stamp;
            }
            c
        } else if let Some(c) = cached {
            c.clone()
        } else {
            continue;
        };
        if component(&child.child_session_id) {
            let events = root.join(&child.child_session_id).join("events.jsonl");
            if let Ok(stamp) = Stamp::read(&events) {
                if stamp != child.stamp {
                    child.node.last_tool_name = last_tool(&events, budget.tail_bytes, stats);
                    child.stamp = stamp;
                }
                child.node.last_activity_at = stamp.millis();
            }
        }
        if let Some(fin) = finished.get(&id) {
            child.done = true;
            child.failed = fin.failed;
            child.node.finished_at = fin.at;
            child.node.tool_calls = fin.tool_calls;
        }
        nodes.insert(id.clone(), child.node(&id));
        next.insert(id, child);
    }
    state.children = next;
    Ok((
        Some(finish_tree(
            Adapter::Grok,
            nodes,
            since,
            budget.max_nodes,
            now,
        )),
        state,
    ))
}
fn scan_updates(
    path: &Path,
    stats: &mut ReadStats,
) -> (BTreeMap<String, Child>, BTreeMap<String, Finish>) {
    let mut spawned = BTreeMap::new();
    let mut finished = BTreeMap::new();
    for line in read_tail(path, PARENT_BYTES, true, stats) {
        let Ok(env) = serde_json::from_slice::<Envelope>(&line) else {
            continue;
        };
        let u = env.params.update;
        if u.subagent_id.is_empty() {
            continue;
        }
        let at = env.timestamp.saturating_mul(1000);
        match u.kind.as_str() {
            "subagent_spawned" => {
                spawned.entry(u.subagent_id).or_insert_with(|| Child {
                    node: SubagentNode {
                        label: u.description,
                        agent_type: u.subagent_type,
                        model: u.model,
                        started_at: at,
                        last_activity_at: at,
                        ..Default::default()
                    },
                    child_session_id: u.child_session_id,
                    started: true,
                    ..Default::default()
                });
            }
            "subagent_finished" => {
                finished.entry(u.subagent_id).or_insert(Finish {
                    failed: u.status != "completed",
                    tool_calls: u.tool_calls,
                    at,
                });
            }
            _ => {}
        }
    }
    (spawned, finished)
}
fn last_tool(path: &Path, cap: usize, stats: &mut ReadStats) -> String {
    for line in read_tail(path, cap, false, stats) {
        let Ok(event) = serde_json::from_slice::<Event>(&line) else {
            continue;
        };
        if !event.tool_name.is_empty()
            && matches!(event.kind.as_str(), "tool_started" | "tool_completed")
        {
            return event.tool_name;
        }
    }
    String::new()
}
