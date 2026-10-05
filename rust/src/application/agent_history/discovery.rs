//! Metadata-only file selection from the canonical registered identity.
use crate::{
    application::session_observations::subagents::{
        artifact_metadata, directory_metadata, entries, open_artifact,
    },
    config::RuntimePaths,
    proto::{
        core::TranscriptSessionIdentity,
        time::{Timestamp, parse_rfc3339, utc},
        unicode::{simple_fold_key, simple_lower},
        wire::{self, Field, Schema},
    },
};
use chrono::{Datelike, Local};
use serde::{Deserialize, Serialize};
use std::{
    io::{self, BufRead, BufReader, Read},
    path::{Path, PathBuf},
    time::Duration,
};
#[derive(Clone, Default, Serialize)]
pub struct Location {
    pub available: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub path: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub reason: String,
}
impl Location {
    fn found(path: PathBuf, label: &str) -> Self {
        Self {
            available: true,
            path: path.to_string_lossy().into_owned(),
            label: label.into(),
            ..Default::default()
        }
    }
    fn missing(reason: &str) -> Self {
        Self {
            reason: reason.into(),
            ..Default::default()
        }
    }
}
pub(super) fn existing_file(paths: &RuntimePaths, path: &Path) -> bool {
    artifact_metadata(paths, path).is_ok_and(|v| !v.is_dir())
}
pub(super) fn existing_dir(paths: &RuntimePaths, path: &Path) -> bool {
    directory_metadata(paths, path).is_ok_and(|v| v.is_dir())
}
pub(super) fn home(value: &str, home: &str, suffix: &str) -> Option<PathBuf> {
    if !super::trim_space(value).is_empty() {
        Some(super::trim_space(value).into())
    } else if !super::trim_space(home).is_empty() {
        Some(Path::new(home).join(suffix))
    } else {
        None
    }
}
pub(super) fn same_path(a: &str, b: &str) -> bool {
    let clean = |v: &str| {
        crate::files::scope::clean(Path::new(
            &v.replace('/', if cfg!(windows) { "\\" } else { "/" }),
        ))
        .to_string_lossy()
        .into_owned()
    };
    simple_fold_key(&clean(a)) == simple_fold_key(&clean(b))
}
pub(super) fn distance(a: Timestamp, b: Timestamp) -> Option<Duration> {
    a.duration_since(b).or_else(|_| b.duration_since(a)).ok()
}
pub(super) fn read_metadata(paths: &RuntimePaths, path: &Path) -> io::Result<Vec<u8>> {
    // Whole-file metadata has no body persistence. Refuse oversized opaque
    // metadata instead of allocating unbounded data from a provider artifact.
    let mut bytes = Vec::new();
    open_artifact(paths, path)?
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err(io::Error::other("provider metadata too large"));
    }
    Ok(bytes)
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Meta {
    #[serde(rename = "type")]
    kind: String,
    cwd: String,
    timestamp: String,
    #[serde(rename = "sessionId")]
    _session_id: String,
    #[serde(rename = "id")]
    _id: String,
    #[serde(rename = "createdAtMs")]
    created_ms: i64,
    payload: Payload,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Payload {
    cwd: String,
    timestamp: String,
}
macro_rules! fields {($($name:literal=>$kind:literal),*)=>{&[$(Field{name:$name,kind:$kind}),*]};}
const SCHEMAS: &[Schema] = &[
    Schema {
        name: "HistoryClaudeMeta",
        fields: fields!("sessionId"=>"string","cwd"=>"string","timestamp"=>"string"),
    },
    Schema {
        name: "HistoryCodexMeta",
        fields: fields!("type"=>"string","payload"=>"HistoryCodexPayload"),
    },
    Schema {
        name: "HistoryCodexPayload",
        fields: fields!("cwd"=>"string","timestamp"=>"string","thread_source"=>"string","parent_thread_id"=>"string"),
    },
    Schema {
        name: "HistoryCommandMeta",
        fields: fields!("type"=>"string","id"=>"string","cwd"=>"string","timestamp"=>"string"),
    },
    Schema {
        name: "HistoryCursorMeta",
        fields: fields!("cwd"=>"string","createdAtMs"=>"int"),
    },
];
fn decode_meta(bytes: &[u8], kind: &str) -> Option<Meta> {
    wire::decode_go_json_members(bytes).ok()?;
    wire::decode_http_schema(bytes, kind, SCHEMAS).ok()
}
fn metadata_lines(paths: &RuntimePaths, path: &Path, count: usize, kind: &str) -> Option<Meta> {
    let mut reader = BufReader::with_capacity(64 * 1024, open_artifact(paths, path).ok()?);
    let mut line = vec![];
    for _ in 0..count {
        line.clear();
        let n = reader
            .by_ref()
            .take(4 * 1024 * 1024 + 1)
            .read_until(b'\n', &mut line)
            .ok()?;
        if n == 0
            || line.len() > 4 * 1024 * 1024
            || (line.len() == 4 * 1024 * 1024 && line.last() != Some(&b'\n'))
        {
            return None;
        }
        let Some(meta) = decode_meta(&line, kind) else {
            continue;
        };
        if kind == "HistoryCodexMeta" {
            return (meta.kind == "session_meta").then_some(meta);
        }
        if kind == "HistoryCommandMeta" {
            return Some(meta);
        }
        if !meta.cwd.is_empty() && !meta.timestamp.is_empty() {
            return Some(meta);
        }
    }
    None
}
pub(super) fn uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}
pub(super) fn claude_dir(root: &Path, cwd: &str) -> PathBuf {
    root.join("projects").join(
        crate::files::scope::clean(Path::new(cwd))
            .to_string_lossy()
            .replace(['\\', '/', ':'], "-"),
    )
}
pub(super) fn command_dir(home: &str, cwd: &str) -> PathBuf {
    let mut slug = String::new();
    for c in simple_lower(super::trim_space(cwd)).chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            slug.push(c);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-');
    Path::new(home)
        .join(".commandcode/projects")
        .join(if slug.is_empty() { "root" } else { slug })
}
fn exact(paths: &RuntimePaths, dir: &Path, id: &str) -> Option<PathBuf> {
    if !uuid(id) {
        return None;
    }
    let path = dir.join(format!("{id}.jsonl"));
    existing_file(paths, &path).then_some(path)
}
fn nearest(
    paths: &RuntimePaths,
    dirs: &[PathBuf],
    identity: &TranscriptSessionIdentity,
    kind: &str,
) -> Option<PathBuf> {
    let started = parse_rfc3339(&identity.started_at).ok()?;
    let mut best: Option<(PathBuf, Duration)> = None;
    let mut second = None;
    for dir in dirs {
        for entry in entries(paths, dir).unwrap_or_default() {
            if entry.is_dir || !entry.name.ends_with(".jsonl") {
                continue;
            }
            let path = dir.join(entry.name);
            let Some(meta) = metadata_lines(
                paths,
                &path,
                if kind == "HistoryClaudeMeta" { 8 } else { 1 },
                kind,
            ) else {
                continue;
            };
            let (cwd, stamp) = if kind == "HistoryCodexMeta" {
                (meta.payload.cwd, meta.payload.timestamp)
            } else {
                if kind == "HistoryCommandMeta" && meta.kind != "session" {
                    continue;
                }
                (meta.cwd, meta.timestamp)
            };
            if !same_path(&cwd, &identity.cwd) {
                continue;
            }
            let Some(delta) = parse_rfc3339(&stamp)
                .ok()
                .and_then(|v| distance(v, started))
            else {
                continue;
            };
            if delta > Duration::from_secs(600) {
                continue;
            }
            if best.as_ref().is_none_or(|(_, d)| delta < *d) {
                if let Some((_, delta)) = &best {
                    second = Some(*delta);
                }
                best = Some((path, delta));
            } else if second.is_none_or(|d| delta < d) {
                second = Some(delta);
            }
        }
    }
    let (path, best) = best?;
    if second.is_some_and(|second| {
        if kind == "HistoryClaudeMeta" {
            second == best
        } else {
            second.saturating_sub(best) < Duration::from_secs(1)
        }
    }) {
        None
    } else {
        Some(path)
    }
}
pub fn structured_path(
    paths: &RuntimePaths,
    identity: &TranscriptSessionIdentity,
) -> Option<PathBuf> {
    if identity.cwd.is_empty() {
        return None;
    }
    match identity.provider.as_str() {
        "claude" => {
            let root = home(&identity.claude_dir, &identity.home_dir, ".claude")?;
            let dir = claude_dir(&root, &identity.cwd);
            if !identity.agent_session_id.is_empty() {
                exact(paths, &dir, &identity.agent_session_id)
            } else {
                nearest(paths, &[dir], identity, "HistoryClaudeMeta")
            }
        }
        "codex" => {
            let root = home(&identity.codex_home, &identity.home_dir, ".codex")?;
            if !identity.native_log_path.is_empty()
                && existing_file(paths, Path::new(&identity.native_log_path))
            {
                return Some(identity.native_log_path.clone().into());
            }
            let date = utc(parse_rfc3339(&identity.started_at).ok()?)
                .ok()?
                .with_timezone(&Local)
                .date_naive();
            let dirs = [-1, 0, 1]
                .into_iter()
                .map(|offset| {
                    let day = date + chrono::Duration::days(offset);
                    root.join("sessions").join(format!(
                        "{:04}/{:02}/{:02}",
                        day.year(),
                        day.month(),
                        day.day()
                    ))
                })
                .collect::<Vec<_>>();
            nearest(paths, &dirs, identity, "HistoryCodexMeta")
        }
        "command-code" => {
            if super::trim_space(&identity.home_dir).is_empty() {
                return None;
            }
            let dir = command_dir(&identity.home_dir, &identity.cwd);
            exact(paths, &dir, &identity.agent_session_id)
                .or_else(|| nearest(paths, &[dir], identity, "HistoryCommandMeta"))
        }
        _ => None,
    }
}
pub fn location(paths: &RuntimePaths, identity: &TranscriptSessionIdentity) -> Location {
    let missing = Location::missing;
    match identity.provider.as_str() {
        "claude" => {
            let Some(root) = home(&identity.claude_dir, &identity.home_dir, ".claude") else {
                return missing("Claude project directory is unavailable");
            };
            if identity.cwd.is_empty() {
                return missing("Claude project directory is unavailable");
            }
            let dir = claude_dir(&root, &identity.cwd);
            if !identity.agent_session_id.is_empty() {
                return exact(paths, &dir, &identity.agent_session_id)
                    .map(|v| Location::found(v, "Claude Code transcript"))
                    .unwrap_or_else(|| missing("Claude Code transcript file not found yet"));
            }
            if existing_dir(paths, &dir) {
                Location::found(dir, "Claude Code project transcripts")
            } else {
                missing("Claude Code transcript directory not found yet")
            }
        }
        "codex" => {
            if identity.cwd.is_empty()
                || home(&identity.codex_home, &identity.home_dir, ".codex").is_none()
            {
                return missing("Codex home directory is unavailable");
            }
            structured_path(paths, identity)
                .map(|v| Location::found(v, "Codex rollout transcript"))
                .unwrap_or_else(|| {
                    missing("Codex rollout log is available after the first completed turn")
                })
        }
        "grok" => {
            if home(&identity.grok_home, &identity.home_dir, ".grok").is_none()
                || identity.cwd.is_empty()
            {
                return missing("Grok session directory is unavailable");
            }
            if parse_rfc3339(&identity.started_at).is_err() {
                return missing("Grok session start time is unavailable");
            }
            super::grok::resolve_grok(paths, identity)
                .map(|v| Location::found(v, "Grok Build chat history"))
                .unwrap_or_else(|| missing("Grok Build chat history not found yet"))
        }
        "copilot" | "cursor-agent" => {
            let provider = if identity.provider == "copilot" {
                "Copilot"
            } else {
                "Cursor Agent"
            };
            if identity.home_dir.is_empty() || identity.cwd.is_empty() {
                return missing(&format!("{provider} session directory is unavailable"));
            }
            let Ok(started) = parse_rfc3339(&identity.started_at) else {
                return missing(&format!("{provider} session start time is unavailable"));
            };
            let root = Path::new(&identity.home_dir).join(if identity.provider == "copilot" {
                ".copilot/session-state"
            } else {
                ".cursor/chats"
            });
            let mut dirs = Vec::new();
            for entry in entries(paths, &root)
                .unwrap_or_default()
                .into_iter()
                .filter(|v| v.is_dir)
            {
                let dir = root.join(entry.name);
                if identity.provider == "copilot" {
                    dirs.push(dir);
                } else {
                    for entry in entries(paths, &dir)
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|v| v.is_dir)
                    {
                        dirs.push(dir.join(entry.name));
                    }
                }
            }
            let mut best: Option<(PathBuf, Duration)> = None;
            for dir in dirs {
                let meta = if identity.provider == "copilot" {
                    let Ok(bytes) = read_metadata(paths, &dir.join("workspace.yaml")) else {
                        continue;
                    };
                    #[derive(Default, Deserialize)]
                    #[serde(default)]
                    struct Workspace {
                        cwd: String,
                        created_at: String,
                    }
                    const SCHEMA: &[crate::config::YamlSchema] = &[crate::config::YamlSchema {
                        name: "HistoryCopilotWorkspace",
                        fields: &[
                            crate::config::YamlField {
                                name: "cwd",
                                kind: "string",
                            },
                            crate::config::YamlField {
                                name: "created_at",
                                kind: "string",
                            },
                        ],
                    }];
                    let Ok(raw) = std::str::from_utf8(&bytes) else {
                        continue;
                    };
                    let Ok(decoded) =
                        crate::config::decode_yaml_schema(raw, "HistoryCopilotWorkspace", SCHEMA)
                    else {
                        continue;
                    };
                    let value: Workspace = if decoded.is_null() {
                        Workspace::default()
                    } else {
                        let Ok(value) = serde_json::from_value(decoded) else {
                            continue;
                        };
                        value
                    };
                    let Ok(stamp) = parse_rfc3339(&value.created_at) else {
                        continue;
                    };
                    (value.cwd, stamp)
                } else {
                    let Ok(bytes) = read_metadata(paths, &dir.join("meta.json")) else {
                        continue;
                    };
                    let Some(value) = decode_meta(&bytes, "HistoryCursorMeta") else {
                        continue;
                    };
                    let sec = value.created_ms.div_euclid(1000);
                    let Ok(stamp) = Timestamp::from_unix(
                        sec,
                        (value.created_ms.rem_euclid(1000) * 1_000_000) as u32,
                    ) else {
                        continue;
                    };
                    (value.cwd, stamp)
                };
                if !same_path(&meta.0, &identity.cwd) {
                    continue;
                }
                let Some(delta) = distance(meta.1, started) else {
                    continue;
                };
                if delta <= Duration::from_secs(600)
                    && best.as_ref().is_none_or(|(_, d)| delta <= *d)
                {
                    best = Some((dir, delta));
                }
            }
            if let Some((path, _)) = best {
                Location::found(
                    path,
                    if identity.provider == "copilot" {
                        "GitHub Copilot CLI session state"
                    } else {
                        "Cursor Agent CLI chat"
                    },
                )
            } else {
                missing(if identity.provider == "copilot" {
                    "Copilot session state not found yet"
                } else {
                    "Cursor Agent chat history not found yet"
                })
            }
        }
        "command-code" => {
            if identity.home_dir.is_empty() || identity.cwd.is_empty() {
                return missing("Command Code project directory is unavailable");
            }
            if let Some(path) = structured_path(paths, identity) {
                return Location::found(path, "Command Code session transcript");
            }
            let dir = command_dir(&identity.home_dir, &identity.cwd);
            if existing_dir(paths, &dir) {
                Location::found(dir, "Command Code project transcripts")
            } else {
                missing("Command Code transcript not found yet")
            }
        }
        "opencode" => {
            if identity.home_dir.is_empty() {
                return missing("opencode home directory is unavailable");
            }
            let path = Path::new(&identity.home_dir).join(".local/share/opencode/opencode.db");
            if existing_file(paths, &path) {
                Location::found(
                    path,
                    "opencode session store (shared across all sessions, not scoped to this one)",
                )
            } else {
                missing("opencode session store not found")
            }
        }
        _ => missing("This provider's native transcript location is not supported yet"),
    }
}
