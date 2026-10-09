use super::*;
use serde_json::value::RawValue;
use std::sync::OnceLock;

#[derive(Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Meta {
    #[serde(deserialize_with = "null_default")]
    agent_type: String,
    #[serde(deserialize_with = "null_default")]
    description: String,
    #[serde(rename = "toolUseId", deserialize_with = "null_default")]
    tool_use_id: String,
    #[serde(deserialize_with = "null_default")]
    spawn_depth: i64,
    #[serde(rename = "parentAgentId", deserialize_with = "null_default")]
    parent_agent_id: String,
    #[serde(deserialize_with = "null_default")]
    model: String,
}
#[derive(Clone, Default)]
struct Child {
    meta: Meta,
    stamp: Stamp,
    started_at: i64,
    last_tool_name: String,
    last_tool_summary: String,
    tool_calls: i64,
    ever_running: bool,
    resolved: bool,
    failed: bool,
    finished_at: i64,
}
#[derive(Clone, Default)]
struct Signal {
    launched: bool,
    resolved: bool,
    failed: bool,
    finished_at: i64,
}
/// Reader-owned metadata and state latches; no transcript body is retained.
#[derive(Clone, Default)]
pub struct ClaudeState {
    parent: PathBuf,
    children: BTreeMap<String, Child>,
    parent_stamp: Option<Stamp>,
    signals: BTreeMap<String, Signal>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Record {
    #[serde(rename = "type", deserialize_with = "null_default")]
    kind: String,
    #[serde(deserialize_with = "null_default")]
    timestamp: String,
    #[serde(deserialize_with = "null_default")]
    message: Message,
    #[serde(deserialize_with = "null_default")]
    operation: String,
    // Only queue-operation's notification tags are inspected; never retained.
    content: Option<Box<RawValue>>,
    #[serde(rename = "toolUseResult", deserialize_with = "some_raw")]
    tool_use_result: Option<Box<RawValue>>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Message {
    content: Option<Box<RawValue>>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Block {
    #[serde(rename = "type", deserialize_with = "null_default")]
    kind: String,
    #[serde(deserialize_with = "null_default")]
    id: String,
    #[serde(deserialize_with = "null_default")]
    tool_use_id: String,
    #[serde(deserialize_with = "null_default")]
    name: String,
    input: Option<Box<RawValue>>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct ToolResult {
    #[serde(rename = "isAsync", deserialize_with = "null_default")]
    is_async: bool,
    #[serde(deserialize_with = "null_default")]
    status: String,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Input {
    #[serde(deserialize_with = "null_default")]
    command: String,
    #[serde(deserialize_with = "null_default")]
    pattern: String,
    #[serde(deserialize_with = "null_default")]
    path: String,
    #[serde(deserialize_with = "null_default")]
    file_path: String,
}
fn some_raw<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Box<RawValue>>, D::Error> {
    Box::<RawValue>::deserialize(d).map(Some)
}
fn blocks(record: &Record) -> Vec<Block> {
    record
        .message
        .content
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Option<Vec<Option<Block>>>>(raw.get()).ok())
        .flatten()
        .unwrap_or_default()
        .into_iter()
        .map(Option::unwrap_or_default)
        .collect()
}
fn summary(raw: Option<&RawValue>) -> String {
    let Some(raw) = raw else {
        return String::new();
    };
    let Ok(input) = serde_json::from_str::<Input>(raw.get()) else {
        return String::new();
    };
    [input.command, input.pattern, input.path, input.file_path]
        .iter()
        .find(|s| !s.is_empty())
        .map(|s| clamp_summary(s))
        .unwrap_or_default()
}
pub(super) fn read(
    parent: &Path,
    since: i64,
    mut state: ClaudeState,
    budget: ReadBudget,
    now: i64,
    stats: &mut ReadStats,
) -> io::Result<(Option<SubagentTree>, ClaudeState)> {
    if state.parent != parent {
        state = ClaudeState {
            parent: parent.into(),
            ..Default::default()
        };
    }
    if parent.extension().is_none_or(|e| e != "jsonl") {
        return Ok((None, state));
    }
    let subdir = parent.with_extension("").join("subagents");
    let entries = match list(&subdir) {
        Ok(entries) => entries,
        Err(_) => return Ok((None, state)),
    };
    let mut next = BTreeMap::new();
    let mut nodes = BTreeMap::new();
    for entry in entries {
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let filename = entry.file_name().to_string_lossy().into_owned();
        let Some(id) = filename
            .strip_prefix("agent-")
            .and_then(|s| s.strip_suffix(".meta.json"))
        else {
            continue;
        };
        if !component(id) {
            continue;
        }
        let cached = state.children.get(id);
        let Some(mut meta) = cached
            .map(|c| c.meta.clone())
            .or_else(|| read_meta(&entry.path(), stats))
        else {
            continue;
        };
        if meta.agent_type == "workflow-subagent"
            || meta.description.is_empty()
            || meta.tool_use_id.is_empty()
        {
            continue;
        }
        if meta.spawn_depth <= 0 {
            meta.spawn_depth = 1;
        }
        let path = subdir.join(format!("agent-{id}.jsonl"));
        let Ok(stamp) = Stamp::read(&path) else {
            continue;
        };
        let mut child = cached.cloned().unwrap_or_default();
        child.meta = meta.clone();
        if cached.is_none() || child.stamp != stamp {
            child.stamp = stamp;
            let (started, calls) = read_head(&path, budget.head_bytes, stamp.size, stats);
            child.started_at = started;
            child.tool_calls = calls;
            let (name, summary) = read_last_tool(&path, budget.tail_bytes, stats);
            child.last_tool_name = name;
            child.last_tool_summary = summary;
        }
        let node = SubagentNode {
            id: id.into(),
            parent_id: if meta.spawn_depth >= 2 {
                meta.parent_agent_id
            } else {
                String::new()
            },
            depth: meta.spawn_depth,
            label: meta.description,
            agent_type: meta.agent_type,
            model: meta.model,
            started_at: child.started_at,
            last_activity_at: stamp.millis(),
            tool_calls: child.tool_calls,
            last_tool_name: child.last_tool_name.clone(),
            last_tool_summary: child.last_tool_summary.clone(),
            ..Default::default()
        };
        next.insert(id.to_string(), child);
        nodes.insert(id.to_string(), node);
    }
    state.children = next;
    if !nodes.is_empty() {
        let stamp = Stamp::read(parent).ok();
        if state.parent_stamp.is_none() || stamp != state.parent_stamp {
            state.signals = scan_parent(parent, stats);
            state.parent_stamp = stamp;
        }
        for (id, node) in &mut nodes {
            let child = state.children.get_mut(id).expect("node owns child cache");
            if !child.resolved {
                if let Some(signal) = state.signals.get(&child.meta.tool_use_id) {
                    if signal.resolved {
                        child.resolved = true;
                        child.failed = signal.failed;
                        child.finished_at = signal.finished_at;
                        child.ever_running = true;
                    } else if signal.launched {
                        child.ever_running = true;
                    }
                }
                if !child.resolved
                    && !child.ever_running
                    && now.saturating_sub(child.stamp.millis()) <= FRESH_MS
                {
                    child.ever_running = true;
                }
            }
            node.state = if child.resolved && child.failed {
                "failed"
            } else if child.resolved {
                "done"
            } else if child.ever_running {
                "running"
            } else {
                "unknown"
            }
            .into();
            if child.resolved {
                node.finished_at = child.finished_at;
            }
        }
    }
    Ok((
        Some(finish_tree(
            Adapter::Claude,
            nodes,
            since,
            budget.max_nodes,
            now,
        )),
        state,
    ))
}
fn read_head(path: &Path, cap: usize, size: u64, stats: &mut ReadStats) -> (i64, i64) {
    let Ok(bytes) = limited(path, cap) else {
        return (0, 0);
    };
    stats.head_bytes += bytes.len();
    stats.files_read += 1;
    let mut started = None;
    let mut calls = 0;
    if size > cap as u64 {
        if let Some(line) = bytes.split(|b| *b == b'\n').next()
            && let Ok(record) = decode_default::<Record>(line)
        {
            return (parse_timestamp(&record.timestamp), 0);
        }
        return (0, 0);
    }
    for line in bytes.split(|b| *b == b'\n') {
        let Ok(record) = decode_default::<Record>(line) else {
            continue;
        };
        started.get_or_insert_with(|| parse_timestamp(&record.timestamp));
        if record.kind == "assistant" {
            calls += blocks(&record)
                .iter()
                .filter(|b| b.kind == "tool_use")
                .count() as i64;
        }
    }
    (started.unwrap_or(0), calls)
}
fn read_last_tool(path: &Path, cap: usize, stats: &mut ReadStats) -> (String, String) {
    for line in read_tail(path, cap, false, stats) {
        let Ok(record) = decode_default::<Record>(&line) else {
            continue;
        };
        if record.kind != "assistant" {
            continue;
        }
        for block in blocks(&record) {
            if block.kind == "tool_use" {
                return (block.name, summary(block.input.as_deref()));
            }
        }
    }
    (String::new(), String::new())
}
fn classify(status: &str) -> Option<bool> {
    let s = status.trim().to_lowercase();
    if s == "completed" {
        Some(false)
    } else if s.contains("fail")
        || matches!(
            s.as_str(),
            "killed"
                | "stopped"
                | "cancelled"
                | "canceled"
                | "aborted"
                | "interrupted"
                | "error"
                | "timeout"
                | "timed_out"
        )
    {
        Some(true)
    } else {
        None
    }
}
fn mark(signals: &mut BTreeMap<String, Signal>, id: &str, completion: Option<bool>, ts: i64) {
    if id.is_empty() {
        return;
    }
    let signal = signals.entry(id.into()).or_default();
    if signal.resolved {
        return;
    }
    signal.launched = true;
    if let Some(failed) = completion {
        signal.resolved = true;
        signal.failed = failed;
        signal.finished_at = ts;
    }
}
fn scan_parent(path: &Path, stats: &mut ReadStats) -> BTreeMap<String, Signal> {
    static NOTICE: OnceLock<regex::Regex> = OnceLock::new();
    static ID: OnceLock<regex::Regex> = OnceLock::new();
    static STATUS: OnceLock<regex::Regex> = OnceLock::new();
    let notice = NOTICE.get_or_init(|| {
        regex::Regex::new(r"(?s)<task-notification>(.*?)</task-notification>").unwrap()
    });
    let id_re = ID
        .get_or_init(|| regex::Regex::new(r"<tool-use-id>\s*([^<\s]+)\s*</tool-use-id>").unwrap());
    let status_re =
        STATUS.get_or_init(|| regex::Regex::new(r"<status>\s*([^<\s]+)\s*</status>").unwrap());
    let mut signals = BTreeMap::new();
    for line in read_tail(path, PARENT_BYTES, true, stats) {
        let Ok(record) = decode_default::<Record>(&line) else {
            continue;
        };
        let ts = parse_timestamp(&record.timestamp);
        match record.kind.as_str() {
            "assistant" => {
                for b in blocks(&record) {
                    if b.kind == "tool_use" {
                        mark(&mut signals, &b.id, None, ts);
                    }
                }
            }
            "user" => {
                if let Some(raw) = &record.tool_use_result
                    && let Ok(result) = decode_default::<ToolResult>(raw.get().as_bytes())
                    && let Some(block) = blocks(&record)
                        .into_iter()
                        .find(|b| b.kind == "tool_result" && !b.tool_use_id.is_empty())
                {
                    mark(
                        &mut signals,
                        &block.tool_use_id,
                        if result.is_async {
                            None
                        } else {
                            classify(&result.status)
                        },
                        ts,
                    );
                }
            }
            "queue-operation" if record.operation == "enqueue" => {
                if let Some(raw) = record.content.as_deref()
                    && let Ok(text) = serde_json::from_str::<String>(raw.get())
                    && let Some(inner) = notice.captures(&text)
                    && let Some(id) = id_re.captures(&inner[1])
                    && let Some(status) = status_re.captures(&inner[1])
                {
                    mark(&mut signals, &id[1], classify(&status[1]), ts);
                }
            }
            _ => {}
        }
    }
    signals
}
