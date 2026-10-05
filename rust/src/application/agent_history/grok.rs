//! Grok-owned history stays in its provider directory, with bounded line reads.
use super::discovery::{self, existing_file, home, same_path};
use crate::{
    application::session_observations::subagents::{entries, open_artifact},
    config::RuntimePaths,
    proto::{
        core::TranscriptSessionIdentity,
        time::{Timestamp, parse_rfc3339},
        wire::{self, Field, GoWire, Schema},
    },
    storage::mask_secrets,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    io::{self, BufRead, BufReader, Read},
    path::{Path, PathBuf},
    time::Duration,
};
#[derive(Clone, Serialize)]
pub struct Message {
    pub role: String,
    pub text: String,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Record {
    #[serde(rename = "type")]
    kind: String,
    synthetic_reason: String,
    content: Option<Box<serde_json::value::RawValue>>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Part {
    #[serde(rename = "type")]
    kind: String,
    text: String,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Active {
    session_id: String,
    cwd: String,
    opened_at: String,
}
impl GoWire for Record {
    const GO_TYPE: &'static str = "GrokHistoryRecord";
    const SCHEMAS: &'static [Schema] = &[Schema {
        name: "GrokHistoryRecord",
        fields: &[
            Field {
                name: "type",
                kind: "string",
            },
            Field {
                name: "synthetic_reason",
                kind: "string",
            },
        ],
    }];
}
impl GoWire for Part {
    const GO_TYPE: &'static str = "GrokHistoryPart";
    const SCHEMAS: &'static [Schema] = &[Schema {
        name: "GrokHistoryPart",
        fields: &[
            Field {
                name: "type",
                kind: "string",
            },
            Field {
                name: "text",
                kind: "string",
            },
        ],
    }];
}
impl GoWire for Active {
    const GO_TYPE: &'static str = "GrokHistoryActive";
    const SCHEMAS: &'static [Schema] = &[Schema {
        name: "GrokHistoryActive",
        fields: &[
            Field {
                name: "session_id",
                kind: "string",
            },
            Field {
                name: "cwd",
                kind: "string",
            },
            Field {
                name: "opened_at",
                kind: "string",
            },
        ],
    }];
}
fn message(raw: &[u8]) -> Option<Message> {
    let mut record = wire::decode::<Record>(raw).ok()?;
    let fields = wire::decode_go_json_members(raw).ok().flatten()?;
    record.content =
        wire::last_go_raw_field(&fields, "content").and_then(|v| serde_json::from_slice(v).ok());
    let content = record.content?;
    let bytes = content.get().as_bytes();
    let text = if let Ok(text) = serde_json::from_slice::<Option<String>>(bytes) {
        text.unwrap_or_default()
    } else {
        let parts = wire::decode_go_json_array(bytes).ok().flatten()?;
        let parts = parts
            .iter()
            .map(|v| wire::decode::<Part>(v))
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        parts
            .into_iter()
            .filter(|p| p.kind == "text" && !p.text.is_empty())
            .map(|p| p.text)
            .collect::<Vec<_>>()
            .join("\n")
    };
    let text = match record.kind.as_str() {
        "assistant" => super::trim_space(&text),
        "user" if record.synthetic_reason.is_empty() => {
            let start = text.find("<user_query>")? + "<user_query>".len();
            let rest = &text[start..];
            super::trim_space(&rest[..rest.find("</user_query>").unwrap_or(rest.len())])
        }
        _ => return None,
    };
    (!text.is_empty()).then(|| Message {
        role: record.kind,
        text: mask_secrets(text),
    })
}
pub(super) fn scan(
    paths: &RuntimePaths,
    path: &Path,
    mut emit: impl FnMut(Message),
) -> io::Result<()> {
    let file = open_artifact(paths, path)?;
    let mut reader = BufReader::with_capacity(64 * 1024, file);
    let mut line = Vec::new();
    loop {
        line.clear();
        let n = reader
            .by_ref()
            .take((8 * 1024 * 1024 + 1) as u64)
            .read_until(b'\n', &mut line)?;
        if n == 0 {
            break;
        }
        if line.len() > 8 * 1024 * 1024
            || (line.len() == 8 * 1024 * 1024 && line.last() != Some(&b'\n'))
        {
            return Err(io::Error::other(
                "Grok history record exceeds scanner limit",
            ));
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        if let Some(record) = message(&line) {
            emit(record);
        }
    }
    Ok(())
}
/// Count all source records while retaining only the requested bounded page.
pub(super) fn page(
    paths: &RuntimePaths,
    path: &Path,
    offset: Option<usize>,
    limit: usize,
) -> io::Result<(usize, usize, Vec<Message>)> {
    let mut total = 0usize;
    let mut selected = VecDeque::new();
    scan(paths, path, |message| {
        if let Some(offset) = offset {
            if total >= offset && selected.len() < limit {
                selected.push_back(message);
            }
        } else {
            selected.push_back(message);
            if selected.len() > limit {
                selected.pop_front();
            }
        }
        total = total.saturating_add(1);
    })?;
    let start = offset.map_or_else(|| total.saturating_sub(limit), |v| v.min(total));
    Ok((total, start, selected.into_iter().collect()))
}
pub fn resolve_grok(paths: &RuntimePaths, identity: &TranscriptSessionIdentity) -> Option<PathBuf> {
    let root = home(&identity.grok_home, &identity.home_dir, ".grok")?;
    let started = parse_rfc3339(&identity.started_at).ok()?;
    resolve_root(paths, &root, &identity.cwd, started)
}
pub(super) fn resolve_root(
    paths: &RuntimePaths,
    root: &Path,
    cwd: &str,
    started: Timestamp,
) -> Option<PathBuf> {
    let sessions = root.join("sessions");
    let cwd_dir = entries(paths, &sessions)
        .ok()?
        .into_iter()
        .filter(|e| e.is_dir)
        .find_map(|e| {
            let decoded = path_unescape(&e.name)?;
            same_path(&decoded, cwd).then(|| sessions.join(e.name))
        })?;
    let mut candidates = Vec::<(PathBuf, Duration)>::new();
    let mut seen = BTreeMap::<PathBuf, usize>::new();
    let mut active_best = None;
    let mut uuid_best = None;
    let mut add = |path: PathBuf, delta: Duration| {
        if let Some(index) = seen.get(&path) {
            if delta < candidates[*index].1 {
                candidates[*index].1 = delta;
            }
        } else {
            seen.insert(path.clone(), candidates.len());
            candidates.push((path, delta));
        }
    };
    if let Ok(data) = discovery::read_metadata(paths, &root.join("active_sessions.json"))
        && let Ok(records) = wire::decode_go_json_array(&data)
    {
        let actives = records
            .unwrap_or_default()
            .iter()
            .map(|v| wire::decode::<Active>(v))
            .collect::<Result<Vec<_>, _>>();
        if let Ok(actives) = actives {
            for active in actives {
                if !same_path(&active.cwd, cwd) {
                    continue;
                }
                let Some(delta) = parse_rfc3339(&active.opened_at)
                    .ok()
                    .and_then(|v| discovery::distance(v, started))
                else {
                    continue;
                };
                if delta > Duration::from_secs(600) {
                    continue;
                }
                let path = cwd_dir.join(active.session_id).join("chat_history.jsonl");
                if existing_file(paths, &path) {
                    add(path.clone(), delta);
                    if active_best.as_ref().is_none_or(|(_, best)| delta <= *best) {
                        active_best = Some((path, delta));
                    }
                }
            }
        }
    }
    for entry in entries(paths, &cwd_dir)
        .unwrap_or_default()
        .into_iter()
        .filter(|e| e.is_dir)
    {
        let Some(delta) = uuid_time(&entry.name).and_then(|v| discovery::distance(v, started))
        else {
            continue;
        };
        if delta > Duration::from_secs(600) {
            continue;
        }
        let path = cwd_dir.join(entry.name).join("chat_history.jsonl");
        if existing_file(paths, &path) {
            add(path.clone(), delta);
            if uuid_best.as_ref().is_none_or(|(_, best)| delta <= *best) {
                uuid_best = Some((path, delta));
            }
        }
    }
    let mut nonempty = None;
    for (path, delta) in candidates {
        let mut found = false;
        let result = scan(paths, &path, |_| found = true);
        if result.is_ok() && found && nonempty.as_ref().is_none_or(|(_, best)| delta < *best) {
            nonempty = Some((path, delta));
        }
    }
    nonempty.or(active_best).or(uuid_best).map(|(path, _)| path)
}
pub(super) fn uuid_time(id: &str) -> Option<Timestamp> {
    let value = id.replace('-', "");
    if value.len() != 32 || value.as_bytes()[12] != b'7' {
        return None;
    }
    let ms = u64::from_str_radix(value.get(..12)?, 16).ok()?;
    Timestamp::from_unix((ms / 1000) as i64, ((ms % 1000) * 1_000_000) as u32).ok()
}
fn path_unescape(raw: &str) -> Option<String> {
    let mut bytes = Vec::new();
    let mut source = raw.bytes();
    while let Some(byte) = source.next() {
        if byte == b'%' {
            let h = char::from(source.next()?).to_digit(16)?;
            let l = char::from(source.next()?).to_digit(16)?;
            bytes.push((h * 16 + l) as u8);
        } else {
            bytes.push(byte);
        }
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}
