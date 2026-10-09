//! Native tasks-output metadata. Result/log bodies are absent from the DTO.
use super::subagents::read_tail;
use crate::{
    config::RuntimePaths,
    proto::{
        self, WfAgent, WfAgentDetail, WfPhase, WorkflowProgress,
        wire::{Field, GoWire, Schema},
    },
};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
    time::SystemTime,
};
#[derive(Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Entry {
    pub r#type: String,
    pub index: i64,
    pub title: String,
    pub label: String,
    pub phase_index: i64,
    pub phase_title: String,
    pub agent_id: String,
    pub model: String,
    pub state: String,
    pub started_at: i64,
    pub queued_at: i64,
    pub attempt: i64,
    pub last_tool_name: String,
    pub last_tool_summary: String,
    pub prompt_preview: String,
    pub last_progress_at: i64,
    pub tokens: i64,
    pub tool_calls: i64,
    pub duration_ms: i64,
    pub result_preview: String,
}
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Output {
    agent_count: i64,
    workflow_progress: Option<Vec<Entry>>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Transcript {
    r#type: String,
    message: TranscriptMessage,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct TranscriptMessage {
    content: Option<String>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Block {
    r#type: String,
    text: String,
    id: String,
    name: String,
    tool_use_id: String,
    content: Option<String>,
}
#[derive(Deserialize)]
#[serde(transparent)]
struct Blocks(Vec<Block>);
macro_rules! fields{($(($name:literal,$kind:literal)),* $(,)?)=>{&[$(Field{name:$name,kind:$kind}),*]};}
const SCHEMAS: &[Schema] = &[
    Schema {
        name: "WorkflowTranscript",
        fields: fields![
            ("type", "string"),
            ("sessionId", "string"),
            ("timestamp", "string"),
            ("isSidechain", "bool"),
            ("message", "WorkflowTranscriptMessage")
        ],
    },
    Schema {
        name: "WorkflowTranscriptMessage",
        fields: fields![("role", "string"), ("content", "json.RawMessage")],
    },
    Schema {
        name: "WorkflowToolBlock",
        fields: fields![
            ("type", "string"),
            ("text", "string"),
            ("thinking", "string"),
            ("id", "string"),
            ("name", "string"),
            ("input", "json.RawMessage"),
            ("tool_use_id", "string"),
            ("content", "json.RawMessage")
        ],
    },
    Schema {
        name: "WorkflowTaskOutput",
        fields: &[
            Field {
                name: "agentCount",
                kind: "int",
            },
            Field {
                name: "workflowProgress",
                kind: "[]WorkflowTaskEntry",
            },
        ],
    },
    Schema {
        name: "WorkflowTaskEntry",
        fields: &[
            Field {
                name: "type",
                kind: "string",
            },
            Field {
                name: "index",
                kind: "int",
            },
            Field {
                name: "title",
                kind: "string",
            },
            Field {
                name: "label",
                kind: "string",
            },
            Field {
                name: "phaseIndex",
                kind: "int",
            },
            Field {
                name: "phaseTitle",
                kind: "string",
            },
            Field {
                name: "agentId",
                kind: "string",
            },
            Field {
                name: "model",
                kind: "string",
            },
            Field {
                name: "state",
                kind: "string",
            },
            Field {
                name: "startedAt",
                kind: "int64",
            },
            Field {
                name: "queuedAt",
                kind: "int64",
            },
            Field {
                name: "attempt",
                kind: "int",
            },
            Field {
                name: "lastToolName",
                kind: "string",
            },
            Field {
                name: "lastToolSummary",
                kind: "string",
            },
            Field {
                name: "promptPreview",
                kind: "string",
            },
            Field {
                name: "lastProgressAt",
                kind: "int64",
            },
            Field {
                name: "tokens",
                kind: "int",
            },
            Field {
                name: "toolCalls",
                kind: "int",
            },
            Field {
                name: "durationMs",
                kind: "int64",
            },
            Field {
                name: "resultPreview",
                kind: "string",
            },
        ],
    },
];
impl GoWire for Output {
    const GO_TYPE: &'static str = "WorkflowTaskOutput";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
impl GoWire for Transcript {
    const GO_TYPE: &'static str = "WorkflowTranscript";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
impl GoWire for Blocks {
    const GO_TYPE: &'static str = "[]WorkflowToolBlock";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
#[derive(Clone, Default)]
pub struct FileState {
    pub path: PathBuf,
    pub modified: Option<SystemTime>,
    pub loaded: bool,
    pub entries: Vec<Entry>,
    pub agent_count: i64,
}
pub fn poll(
    paths: &RuntimePaths,
    path: &Path,
    mut prior: FileState,
) -> io::Result<(FileState, bool)> {
    super::subagents::check_path(paths, path)?;
    let mut file = super::subagents::open_artifact(paths, path)?;
    let info = file.metadata()?;
    if info.is_dir() {
        return Err(io::Error::other("workflow task output path is a directory"));
    }
    if prior.loaded && prior.path == path && prior.modified == Some(info.modified()?) {
        return Ok((prior, false));
    }
    if info.len() > 32 * 1024 * 1024 {
        prior.path = path.into();
        return Ok((prior, false));
    }
    let mut bytes = Vec::new();
    use std::io::Read;
    file.by_ref()
        .take(32 * 1024 * 1024)
        .read_to_end(&mut bytes)?;
    let output = proto::decode_wire::<Output>(&bytes)
        .map_err(|_| io::Error::other("workflow task output invalid JSON"))?;
    Ok((
        FileState {
            path: path.into(),
            modified: Some(info.modified()?),
            loaded: true,
            entries: output.workflow_progress.unwrap_or_default(),
            agent_count: output.agent_count,
        },
        true,
    ))
}
pub fn build(entries: &[Entry]) -> Option<WorkflowProgress> {
    if entries.is_empty() {
        return None;
    }
    let mut phases = BTreeMap::<i64, WfPhase>::new();
    for entry in entries {
        if entry.r#type == "workflow_phase" {
            phases.insert(
                entry.index,
                WfPhase {
                    title: entry.title.clone(),
                    agents: None,
                },
            );
        }
    }
    let truncate = |value: &str| value.chars().take(500).collect();
    for entry in entries {
        if entry.r#type != "workflow_agent" {
            continue;
        }
        let phase = phases.entry(entry.phase_index).or_insert_with(|| WfPhase {
            title: entry.phase_title.clone(),
            agents: None,
        });
        phase.agents.get_or_insert_with(Vec::new).push(WfAgent {
            label: entry.label.clone(),
            state: entry.state.clone(),
            detail: Some(WfAgentDetail {
                model: entry.model.clone(),
                started_at: entry.started_at,
                last_progress_at: entry.last_progress_at,
                duration_ms: entry.duration_ms,
                tokens: entry.tokens,
                tool_calls: entry.tool_calls,
                last_tool_name: truncate(&entry.last_tool_name),
                last_tool_summary: truncate(&entry.last_tool_summary),
                prompt_preview: truncate(&entry.prompt_preview),
                result_preview: truncate(&entry.result_preview),
            }),
            ..Default::default()
        });
    }
    (!phases.is_empty()).then(|| WorkflowProgress {
        source: "task-output".into(),
        task_detail_source: "task-output".into(),
        phases: phases.into_values().collect(),
        ..Default::default()
    })
}
pub fn overlay(output: &mut WorkflowProgress, detail: &WorkflowProgress) {
    output.phases = detail.phases.clone();
    output.task_detail_source = detail.task_detail_source.clone()
}
pub fn resolve(paths: &RuntimePaths, transcript: &Path, workflow_dir: &str) -> Option<String> {
    let workflow_dir = workflow_dir.trim();
    if transcript.as_os_str().is_empty() || workflow_dir.is_empty() {
        return None;
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(100);
    let bytes = read_tail(paths, transcript, 2 * 1024 * 1024).ok()?;
    let text = proto::wire::go_utf8_lossy(&bytes);
    let mut results = BTreeMap::<String, (String, String)>::new();
    let task = regex::Regex::new(r"Task ID:[\t\n\x0c\r ]*([^\t\n\x0c\r ]+)").unwrap();
    let directory = regex::Regex::new(r"Transcript dir:[\t\n\x0c\r ]*(.+)").unwrap();
    for line in text.lines().rev().take(500) {
        if std::time::Instant::now() >= deadline {
            return None;
        }
        let Ok(value) = proto::decode_wire::<Transcript>(line.as_bytes()) else {
            continue;
        };
        let Some(content) = value.message.content else {
            continue;
        };
        let Ok(Blocks(blocks)) = proto::decode_wire::<Blocks>(content.as_bytes()) else {
            continue;
        };
        match value.r#type.as_str() {
            "user" => {
                for block in blocks {
                    if block.r#type != "tool_result" {
                        continue;
                    }
                    if block.tool_use_id.is_empty() {
                        continue;
                    }
                    let id = block.tool_use_id;
                    let text = block
                        .content
                        .as_ref()
                        .map(|raw| extract_text(raw.as_bytes()))
                        .unwrap_or_default();
                    let text = if text.is_empty() { block.text } else { text };
                    if let (Some(task), Some(dir)) =
                        (task.captures(&text), directory.captures(&text))
                    {
                        let task = task[1].trim();
                        let dir = dir[1].trim();
                        if !task.is_empty() && !dir.is_empty() {
                            results.insert(id, (task.into(), dir.into()));
                        }
                    }
                }
            }
            "assistant" => {
                for block in blocks {
                    if block.r#type == "tool_use"
                        && block.name == "Workflow"
                        && !block.id.is_empty()
                        && let Some(result) = results.get(&block.id)
                        && result.1 == workflow_dir
                    {
                        return Some(result.0.clone());
                    }
                }
            }
            _ => {}
        }
    }
    None
}
fn extract_text(raw: &[u8]) -> String {
    if let Ok(text) = serde_json::from_slice::<String>(raw) {
        return text.trim().into();
    }
    let Ok(Some(items)) = proto::wire::decode_go_json_array(raw) else {
        return String::new();
    };
    // Go decodes the whole []map[string]RawMessage first. One scalar item
    // invalidates the array, including otherwise usable neighboring text.
    let Ok(items) = items
        .iter()
        .map(|item| proto::wire::decode_go_json_object(item))
        .collect::<Result<Vec<_>, _>>()
    else {
        return String::new();
    };
    items
        .into_iter()
        .flatten()
        .map(|item| {
            let text = item
                .get("text")
                .map_or_else(String::new, |raw| extract_text(raw));
            if text.is_empty() {
                item.get("content")
                    .map_or_else(String::new, |raw| extract_text(raw))
            } else {
                text
            }
        })
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .into()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raw_nested_result_text_preserves_ignored_large_numbers_and_invalid_array_rules() {
        assert_eq!(extract_text(br#"[{"text":"Task ID: task-a","ignored":1e1000},{"content":"Transcript dir: wf_a"}]"#),"Task ID: task-a\nTranscript dir: wf_a");
        assert_eq!(extract_text(br#"[{"text":"neighbor"},123]"#), "");
        assert_eq!(extract_text(br#"[null,{"text":" usable "}]"#), "usable");
    }
    #[test]
    fn actual_trial_transcript_resolution_matches_only_workflow_tool_and_exact_run_directory() {
        let root = tempfile::tempdir().unwrap();
        let installed = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::trial(root.path(), 49327, installed.path()).unwrap();
        let transcript = root.path().join("session.jsonl");
        let launch = serde_json::json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":"wf-tool","name":"Workflow","input":{"ignored":"synthetic"}}]}});
        let result = serde_json::json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"wf-tool","content":[{"text":"Task ID: task-a"},{"content":"Transcript dir: wf_a"}]}]}});
        std::fs::write(&transcript, format!("{launch}\n{result}\n")).unwrap();
        assert_eq!(
            resolve(&paths, &transcript, " wf_a ").as_deref(),
            Some("task-a")
        );
        assert!(resolve(&paths, &transcript, "wf_b").is_none());
        let invalid = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":123,"tool_use_id":"wf-tool","content":"Task ID: task-b\nTranscript dir: wf_a"}]}}"#;
        std::fs::write(&transcript, format!("{launch}\n{invalid}\n")).unwrap();
        assert!(resolve(&paths, &transcript, "wf_a").is_none());
    }
}
