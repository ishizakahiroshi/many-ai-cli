//! Handoff preview/list and manual note caller over the canonical path-only store.
use super::http::{Request, Response, decode_json, require_method};
use crate::{
    config::{ConfigStore, RuntimePaths},
    files::safe_fs::Dir,
    orchestration::handoff::{
        HandoffStore, KIND_NOTE, KIND_SESSION_END, KIND_SESSION_START, Record, RecordKey,
        end_follows_start, model_from_segment,
    },
    proto::{core::*, time::Timestamp},
    storage::mask_secrets,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, Write},
    path::PathBuf,
    sync::Arc,
};
pub const PATH: &str = "/api/handoff";
pub const PREFIX: &str = "/api/handoff/";
pub trait HandoffHttpHooks: Send + Sync {
    fn ensure_transcript(&self, id: LiveSessionId, now: Timestamp) -> Result<(), SessionError>;
    fn request_note<'a>(
        &'a self,
        id: LiveSessionId,
        now: Timestamp,
    ) -> CoreFuture<'a, Result<PathBuf, &'static str>>;
    fn warning(&self, operation: &'static str);
}
pub struct HandoffHttp {
    store: HandoffStore,
    config: Arc<ConfigStore>,
    core: Arc<dyn SessionCore>,
    hooks: Arc<dyn HandoffHttpHooks>,
}
#[derive(Default, Serialize)]
struct Preview {
    ok: bool,
    session_id: i64,
    /// File stem of the records shown (`s20261116-6`, or `s6` for records an
    /// earlier version wrote). Two launches shown as the same `#N` on
    /// different days differ here; the UI opens a listed one by this (#44).
    record: String,
    exists: bool,
    live: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    provider: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    cwd: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    branch: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    model: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    started_at: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    ended_at: String,
    #[serde(skip_serializing_if = "zero")]
    handoff_from: i64,
    #[serde(skip_serializing_if = "zero")]
    handoff_to: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    transcript_path: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    note_path: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    note_paths: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    markdown: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    candidate_providers: Vec<String>,
}
fn zero(value: &i64) -> bool {
    *value == 0
}
fn id(value: &str) -> Option<i64> {
    value
        .parse::<isize>()
        .ok()
        .filter(|id| *id > 0)
        .map(|id| id as i64)
}
/// What a request names: a session number as shown on screen, or one record
/// by its file stem (`s20261116-6` / `s6`).
enum Target {
    Number(i64),
    Record(RecordKey),
}
fn target(value: &str) -> Option<Target> {
    match value.strip_prefix('s') {
        Some(stem) => RecordKey::parse(stem).map(Target::Record),
        None => id(value).map(Target::Number),
    }
}
pub fn methods(path: &str) -> &'static [&'static str] {
    if path == PATH {
        return &["GET"];
    }
    let action = path
        .strip_prefix(PREFIX)
        .unwrap_or("")
        .trim_matches('/')
        .split_once('/')
        .map(|(_, action)| action)
        .unwrap_or("");
    if matches!(action, "note" | "manual-note") {
        &["POST"]
    } else {
        &["GET"]
    }
}
impl HandoffHttp {
    pub fn new(
        paths: RuntimePaths,
        config: Arc<ConfigStore>,
        core: Arc<dyn SessionCore>,
        hooks: Arc<dyn HandoffHttpHooks>,
    ) -> Self {
        Self {
            store: HandoffStore::new(paths),
            config,
            core,
            hooks,
        }
    }
    pub async fn handle_authenticated(
        &self,
        request: &Request,
        now: Timestamp,
    ) -> Option<Response> {
        if request.path != PATH && !request.path.starts_with(PREFIX) {
            return None;
        }
        if let Err(error) = require_method(request, methods(&request.path)) {
            return Some(error);
        }
        if request.path == PATH {
            return Some(self.list());
        }
        let rest = request.path.strip_prefix(PREFIX).unwrap().trim_matches('/');
        let (raw, action) = rest.split_once('/').unwrap_or((rest, ""));
        if !matches!(action, "" | "note" | "manual-note") {
            return Some(Response::error(404, "not_found", "unknown handoff action"));
        }
        let Some(target) = target(raw) else {
            return Some(Response::error(400, "bad_request", "invalid session id"));
        };
        let named_record = matches!(target, Target::Record(_));
        let key = self.resolve(target);
        let id = key.number();
        Some(match action {
            // Only the live launch can write its memo; an earlier launch
            // named by its record under the same number cannot (#44).
            "note" if named_record && !self.is_live(&key) => {
                Response::error(404, "session_not_writable", "handoff note request rejected")
            }
            "note" => match self.hooks.request_note(LiveSessionId(id), now).await {
                Ok(path) => Response::json(200, &serde_json::json!({"ok":true,"note_path":path})),
                Err(reason) => Response::error(
                    if reason == "session_not_writable" {
                        404
                    } else {
                        409
                    },
                    reason,
                    "handoff note request rejected",
                ),
            },
            "manual-note" => {
                #[derive(Default, Deserialize)]
                #[serde(default)]
                struct Body {
                    text: String,
                }
                impl crate::proto::wire::GoWire for Body {
                    const GO_TYPE: &'static str = "HandoffManualNote";
                    const SCHEMAS: &'static [crate::proto::wire::Schema] =
                        &[crate::proto::wire::Schema {
                            name: "HandoffManualNote",
                            fields: &[crate::proto::wire::Field {
                                name: "text",
                                kind: "string",
                            }],
                        }];
                }
                match decode_json::<Body>(request) {
                    Ok(body) => self.manual_note(&key, &body.text, now),
                    Err(error) => error,
                }
            }
            _ => {
                // Writes only to the live launch's own records, if any.
                let _ = self.hooks.ensure_transcript(LiveSessionId(id), now);
                let live = self.is_live(&key);
                match self.identity(&key).and_then(|mut preview| {
                    if preview.exists {
                        preview.markdown = self.store.write_rendered(&key)?.1;
                    }
                    preview.live = live;
                    preview.candidate_providers = crate::profile::declared::handoff_candidates();
                    Ok(preview)
                }) {
                    Ok(preview) => Response::json(200, &preview),
                    Err(_) => Response::error(
                        500,
                        "handoff_render_error",
                        "handoff render error: owned storage unavailable",
                    ),
                }
            }
        })
    }
    /// The records a request means. A bare number is the live session's own
    /// launch when one is on screen, else the newest records under that number.
    fn resolve(&self, target: Target) -> RecordKey {
        match target {
            Target::Record(key) => key,
            Target::Number(number) => self
                .live_key(number)
                .or_else(|| self.store.latest_key_for_number(number))
                .unwrap_or_else(|| RecordKey::legacy(number)),
        }
    }
    fn live_key(&self, number: i64) -> Option<RecordKey> {
        self.core
            .snapshot(LiveSessionId(number))
            .map(|snapshot| RecordKey::for_snapshot(&snapshot))
    }
    /// Whether these records belong to the launch on screen under their number.
    fn is_live(&self, key: &RecordKey) -> bool {
        self.live_key(key.number()).as_ref() == Some(key)
    }
    fn identity(&self, key: &RecordKey) -> io::Result<Preview> {
        // Same launch as the rendered handoff: a reused session number can
        // leave earlier launches in the same file (#46). They stay on disk.
        let records = self.store.read_current_session(key)?;
        let mut preview = Preview {
            ok: true,
            session_id: key.number(),
            record: key.stem(),
            exists: !records.is_empty(),
            // Like the rendered handoff, the model is shown only with a start.
            model: if records.iter().any(|r| r.kind == KIND_SESSION_START) {
                model_from_segment(&records).to_owned()
            } else {
                String::new()
            },
            ..Default::default()
        };
        let mut seen = BTreeSet::new();
        for record in records {
            match record.kind.as_str() {
                KIND_SESSION_START => {
                    preview.provider = record.provider;
                    preview.cwd = record.cwd;
                    preview.branch = record.branch;
                    preview.started_at = record.ts;
                    preview.handoff_from = record.handoff_from;
                }
                KIND_SESSION_END if end_follows_start(&preview.started_at, &record.ts) => {
                    preview.ended_at = record.ts
                }
                _ => {}
            }
            let transcript = record.transcript.trim();
            if !transcript.is_empty() && std::fs::metadata(transcript).is_ok_and(|m| !m.is_dir()) {
                preview.transcript_path = transcript.into();
            }
            let note = record.note.trim();
            if !note.is_empty() && std::fs::metadata(note).is_ok_and(|m| !m.is_dir()) {
                preview.note_path = note.into();
                if seen.insert(note.to_owned()) {
                    preview.note_paths.push(note.into());
                }
            }
        }
        Ok(preview)
    }
    fn list(&self) -> Response {
        let directory = match Dir::open(&self.store.directory()) {
            Ok(dir) => dir,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Response::json(200, &serde_json::json!({"ok":true,"entries":null}));
            }
            Err(_) => {
                return Response::error(
                    500,
                    "handoff_list_error",
                    "handoff list error: owned storage unavailable",
                );
            }
        };
        let names = match directory.entries() {
            Ok(names) => names,
            Err(_) => {
                return Response::error(
                    500,
                    "handoff_list_error",
                    "handoff list error: owned storage unavailable",
                );
            }
        };
        let mut keys = vec![];
        for name in names {
            if directory
                .metadata(&name)
                .is_ok_and(|metadata| metadata.is_dir())
            {
                continue;
            }
            // `s20261116-6.jsonl` and, from earlier versions, `s6.jsonl`.
            if let Some(stem) = name.strip_suffix(".jsonl")
                && let Some(key) = RecordKey::parse(stem.strip_prefix('s').unwrap_or(stem))
            {
                keys.push(key);
            }
        }
        // Newest day first, larger numbers first within a day; records from
        // before dated names come after every dated one.
        keys.sort_by(|a, b| b.cmp(a));
        let entries: Vec<_> = keys
            .iter()
            .filter_map(|key| {
                self.identity(key)
                    .ok()
                    .filter(|preview| preview.exists)
                    .map(|preview| (key, preview))
            })
            .collect();
        // A successor names its predecessor by number only. The same number
        // can stand for launches on several days; the predecessor is the
        // newest of them that started no later than the successor.
        let mut successors = BTreeMap::new();
        for (_, entry) in &entries {
            if entry.handoff_from == 0 {
                continue;
            }
            let started =
                |preview: &Preview| crate::proto::time::parse_rfc3339(&preview.started_at).ok();
            let predecessor = entries
                .iter()
                .filter(|(_, candidate)| candidate.session_id == entry.handoff_from)
                .find(
                    |(_, candidate)| match (started(candidate), started(entry)) {
                        (Some(before), Some(after)) => before <= after,
                        _ => true,
                    },
                )
                .map(|(key, _)| (*key).clone());
            if let Some(predecessor) = predecessor {
                successors.entry(predecessor).or_insert(entry.session_id);
            }
        }
        let entries: Vec<Preview> = entries
            .into_iter()
            .map(|(key, mut entry)| {
                entry.handoff_to = successors.get(key).copied().unwrap_or(0);
                entry.live = self.is_live(key);
                entry.note_paths.clear();
                entry
            })
            .collect();
        Response::json(200, &serde_json::json!({"ok":true,"entries":entries}))
    }
    fn manual_note(&self, key: &RecordKey, text: &str, now: Timestamp) -> Response {
        let result = (|| -> Result<PathBuf, &'static str> {
            if !self
                .config
                .snapshot()
                .map_err(|_| "memo_save_failed")?
                .config
                .handoff
                .enabled_or_default()
            {
                return Err("handoff_disabled");
            }
            if text.len() > 256 * 1024 {
                return Err("memo_too_large");
            }
            if text.trim().is_empty() {
                return Err("memo_empty");
            }
            if self
                .store
                .read_session(key)
                .map_err(|_| "memo_save_failed")?
                .is_empty()
            {
                return Err("handoff_record_not_found");
            }
            let path = self
                .store
                .manual_note_path_for(key)
                .map_err(|_| "memo_save_failed")?;
            let body = mask_secrets(text);
            if body.trim().is_empty() {
                return Err("memo_empty");
            }
            let directory = Dir::open_or_create_private_components(path.parent().unwrap())
                .map_err(|_| "memo_save_failed")?;
            let mut file = directory
                .open_write_or_create(path.file_name().unwrap().to_str().unwrap(), 0o600)
                .map_err(|_| "memo_save_failed")?;
            file.set_len(0).map_err(|_| "memo_save_failed")?;
            file.write_all(body.as_bytes())
                .map_err(|_| "memo_save_failed")?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(std::fs::Permissions::from_mode(0o600))
                    .map_err(|_| "memo_save_failed")?;
            }
            drop(file);
            self.store
                .append(
                    key,
                    Record {
                        kind: KIND_NOTE.into(),
                        note: path.to_string_lossy().into_owned(),
                        ..Default::default()
                    },
                    now,
                )
                .map_err(|_| "memo_save_failed")?;
            Ok(path)
        })();
        match result {
            Ok(path) => Response::json(200, &serde_json::json!({"ok":true,"note_path":path})),
            Err(reason) => {
                if reason == "memo_save_failed" {
                    self.hooks.warning("handoff manual memo save failed");
                }
                Response::error(
                    match reason {
                        "memo_empty" | "memo_too_large" => 400,
                        "handoff_record_not_found" => 404,
                        "handoff_disabled" => 409,
                        _ => 500,
                    },
                    reason,
                    "manual handoff memo could not be saved",
                )
            }
        }
    }
}
#[cfg(test)]
mod tests;
