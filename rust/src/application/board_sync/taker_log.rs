//! What the `local-claude` taker did on this machine, kept so the person at
//! the dashboard can see it afterwards: one small record per run and the run's
//! `claude -p` stream (secrets masked), under `<root>/logs/local-taker/`.
//!
//! The board keeps what was handed over, when it was taken and how it ended;
//! this keeps only how the run went on this machine, named by the request id
//! the board shows. Only the newest `KEEP` runs are kept.
//!
//! Whether the taker is paused is kept in `<root>/local-taker.json`, so a
//! paused taker stays paused after the Hub restarts.
use crate::storage::mask_secrets;
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    path::{Path, PathBuf},
};

/// Runs kept (records and streams); older ones are removed.
pub(super) const KEEP: usize = 50;
/// Bytes of one run's stream kept on disk.
const STREAM_MAX: u64 = 4 * 1024 * 1024;
/// Bytes of one stream line kept (a longer line is cut).
const LINE_MAX: usize = 64 * 1024;
/// Entries of a stream shown, and characters of each.
const ENTRIES_MAX: usize = 400;
const ENTRY_TEXT_MAX: usize = 2000;
const PAUSE_FILE: &str = "local-taker.json";

pub(super) fn dir_of(root: &Path) -> PathBuf {
    root.join("logs").join("local-taker")
}

/// A file name for a request id: the id itself when it is plain, otherwise
/// its plain characters and a digest (two ids never share a name).
pub(super) fn stem_of(request: &str) -> String {
    use sha2::{Digest, Sha256};
    let plain: String = request
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .take(80)
        .collect();
    if plain == request && !plain.is_empty() && !plain.starts_with('.') {
        return plain;
    }
    let digest: String = Sha256::digest(request.as_bytes())
        .iter()
        .take(6)
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("{}-{digest}", plain.trim_start_matches('.'))
}

/// One run, as the dashboard lists it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RunRecord {
    pub request_id: String,
    pub item_id: String,
    pub project: String,
    pub title: String,
    /// Milliseconds since the epoch.
    pub started_at: i64,
    pub ended_at: Option<i64>,
    /// `running`, `done`, `failed`; a record left `running` by a Hub that
    /// stopped is shown as `interrupted`.
    pub outcome: String,
    /// The run's short result or why it did not finish (masked).
    pub summary: String,
}

pub(super) fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub(super) fn write_record(dir: &Path, record: &RunRecord) {
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let Ok(text) = serde_json::to_vec(record) else {
        return;
    };
    let file = dir.join(format!("{}.json", stem_of(&record.request_id)));
    let temporary = file.with_extension("json.tmp");
    if std::fs::write(&temporary, text).is_ok() && std::fs::rename(&temporary, &file).is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
}

/// The records, newest first.
pub(super) fn records(dir: &Path) -> Vec<RunRecord> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<RunRecord> = entries
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter(|e| {
            e.metadata()
                .is_ok_and(|m| m.is_file() && m.len() <= 64 * 1024)
        })
        .filter_map(|e| std::fs::read(e.path()).ok())
        .filter_map(|bytes| serde_json::from_slice(&bytes).ok())
        .collect();
    found.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    found
}

/// Removes all but the newest `keep` runs (record and stream).
pub(super) fn prune(dir: &Path, keep: usize) {
    for old in records(dir).into_iter().skip(keep) {
        let stem = stem_of(&old.request_id);
        let _ = std::fs::remove_file(dir.join(format!("{stem}.json")));
        let _ = std::fs::remove_file(dir.join(format!("{stem}.jsonl")));
    }
}

pub(super) fn stream_path(dir: &Path, request: &str) -> PathBuf {
    dir.join(format!("{}.jsonl", stem_of(request)))
}

/// Writes a run's stream line by line, masked and within `STREAM_MAX`.
pub struct StreamWriter {
    file: Option<std::fs::File>,
    written: u64,
    full: bool,
}

impl StreamWriter {
    pub fn create(path: &Path) -> Self {
        let file = path
            .parent()
            .and_then(|p| std::fs::create_dir_all(p).ok())
            .and_then(|_| std::fs::File::create(path).ok());
        Self {
            file,
            written: 0,
            full: false,
        }
    }

    pub fn line(&mut self, line: &[u8]) {
        let Some(file) = self.file.as_mut() else {
            return;
        };
        if self.full {
            return;
        }
        let cut = &line[..line.len().min(LINE_MAX)];
        let text = mask_secrets(String::from_utf8_lossy(cut).trim_end());
        if text.is_empty() {
            return;
        }
        let size = text.len() as u64 + 1;
        if self.written + size > STREAM_MAX {
            self.full = true;
            let _ = writeln!(file, "{{\"type\":\"many-ai-cli\",\"note\":\"stream cut\"}}");
            return;
        }
        if writeln!(file, "{text}").is_ok() {
            self.written += size;
        }
    }
}

/// One step of a run, for the dashboard.
#[derive(Serialize, Debug, PartialEq)]
pub struct StreamEntry {
    /// `text` (what claude said), `tool` (a tool it used), `tool_result`,
    /// `result` (the run's answer), `note` (anything else worth a line).
    pub kind: &'static str,
    pub text: String,
}

fn cut(text: &str) -> String {
    if text.chars().count() <= ENTRY_TEXT_MAX {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(ENTRY_TEXT_MAX - 1).collect();
    out.push('…');
    out
}

/// The most telling input of a tool call: a command, a path or a pattern.
fn tool_line(block: &serde_json::Value) -> String {
    let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("tool");
    let input = block.get("input");
    let detail = [
        "command",
        "file_path",
        "path",
        "pattern",
        "url",
        "description",
    ]
    .iter()
    .find_map(|key| input?.get(*key)?.as_str())
    .unwrap_or_default();
    if detail.is_empty() {
        name.to_owned()
    } else {
        format!("{name}: {}", detail.replace(['\r', '\n'], " "))
    }
}

fn result_text(content: Option<&serde_json::Value>) -> String {
    match content {
        Some(serde_json::Value::String(text)) => text.clone(),
        Some(serde_json::Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Reads a run's stream into steps. Lines that are not the stream's JSON are
/// skipped; the stream was masked when it was written and is masked again.
pub(super) fn entries(path: &Path) -> Option<Vec<StreamEntry>> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut out = Vec::new();
    let mut push = |kind: &'static str, text: &str| {
        let text = text.trim();
        if !text.is_empty() && out.len() < ENTRIES_MAX {
            out.push(StreamEntry {
                kind,
                text: cut(&mask_secrets(text)),
            });
        }
    };
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let blocks = value
            .pointer("/message/content")
            .and_then(|c| c.as_array())
            .cloned()
            .unwrap_or_default();
        match value.get("type").and_then(|t| t.as_str()) {
            Some("assistant") => {
                for block in &blocks {
                    match block.get("type").and_then(|t| t.as_str()) {
                        Some("text") => push("text", block["text"].as_str().unwrap_or_default()),
                        Some("tool_use") => push("tool", &tool_line(block)),
                        _ => {}
                    }
                }
            }
            Some("user") => {
                for block in &blocks {
                    if block.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                        push("tool_result", &result_text(block.get("content")));
                    }
                }
            }
            Some("result") => push(
                "result",
                value
                    .get("result")
                    .and_then(|r| r.as_str())
                    .unwrap_or_default(),
            ),
            Some("many-ai-cli") => push(
                "note",
                "（記録が大きくなったので、ここから先は残していない）",
            ),
            _ => {}
        }
    }
    Some(out)
}

#[derive(Serialize, Deserialize, Default)]
struct PauseFile {
    paused: bool,
}

pub(super) fn paused(root: &Path) -> bool {
    std::fs::read(root.join(PAUSE_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<PauseFile>(&bytes).ok())
        .is_some_and(|file| file.paused)
}

pub(super) fn set_paused(root: &Path, paused: bool) -> std::io::Result<()> {
    let file = root.join(PAUSE_FILE);
    let temporary = root.join(format!("{PAUSE_FILE}.tmp"));
    std::fs::write(&temporary, serde_json::to_vec(&PauseFile { paused })?)?;
    std::fs::rename(&temporary, &file)
}
