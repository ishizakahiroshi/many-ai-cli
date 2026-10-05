//! Handoff preview/list and manual note caller over the canonical path-only store.
use super::http::{Request, Response, decode_json, require_method};
use crate::{
    config::{ConfigStore, RuntimePaths},
    files::safe_fs::Dir,
    orchestration::handoff::{
        HandoffStore, KIND_NOTE, KIND_SESSION_END, KIND_SESSION_START, Record,
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
        let Some(id) = id(raw) else {
            return Some(Response::error(400, "bad_request", "invalid session id"));
        };
        Some(match action {
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
                    Ok(body) => self.manual_note(id, &body.text, now),
                    Err(error) => error,
                }
            }
            _ => {
                let _ = self.hooks.ensure_transcript(LiveSessionId(id), now);
                match self.identity(id).and_then(|mut preview| {
                    if preview.exists {
                        preview.markdown = self.store.write_rendered(id)?.1;
                    }
                    preview.live = self.core.snapshot(LiveSessionId(id)).is_some();
                    preview.candidate_providers = [
                        "claude",
                        "codex",
                        "grok",
                        "copilot",
                        "cursor-agent",
                        "opencode",
                        "command-code",
                    ]
                    .into_iter()
                    .map(str::to_owned)
                    .collect();
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
    fn identity(&self, id: i64) -> io::Result<Preview> {
        let records = self.store.read_session(id)?;
        let mut preview = Preview {
            ok: true,
            session_id: id,
            exists: !records.is_empty(),
            ..Default::default()
        };
        let mut seen = BTreeSet::new();
        for record in records {
            match record.kind.as_str() {
                KIND_SESSION_START => {
                    preview.provider = record.provider;
                    preview.cwd = record.cwd;
                    preview.branch = record.branch;
                    preview.model = record.model;
                    preview.started_at = record.ts;
                    preview.handoff_from = record.handoff_from;
                }
                KIND_SESSION_END => preview.ended_at = record.ts,
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
        let mut ids = vec![];
        for name in names {
            if directory
                .metadata(&name)
                .is_ok_and(|metadata| metadata.is_dir())
            {
                continue;
            }
            if let Some(name) = name.strip_suffix(".jsonl")
                && let Ok(id) = name.strip_prefix('s').unwrap_or(name).parse::<isize>()
            {
                ids.push(id as i64);
            }
        }
        ids.sort_by(|a, b| b.cmp(a));
        let mut entries: Vec<_> = ids
            .into_iter()
            .filter_map(|id| self.identity(id).ok().filter(|preview| preview.exists))
            .collect();
        let mut successors = BTreeMap::new();
        for entry in &entries {
            if entry.handoff_from != 0 {
                successors
                    .entry(entry.handoff_from)
                    .or_insert(entry.session_id);
            }
        }
        for entry in &mut entries {
            entry.handoff_to = successors.get(&entry.session_id).copied().unwrap_or(0);
            entry.live = self
                .core
                .snapshot(LiveSessionId(entry.session_id))
                .is_some();
            entry.note_paths.clear();
        }
        Response::json(200, &serde_json::json!({"ok":true,"entries":entries}))
    }
    fn manual_note(&self, id: i64, text: &str, now: Timestamp) -> Response {
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
                .read_session(id)
                .map_err(|_| "memo_save_failed")?
                .is_empty()
            {
                return Err("handoff_record_not_found");
            }
            let path = self
                .store
                .manual_note_path_for(id)
                .map_err(|_| "memo_save_failed")?;
            let body = mask_secrets(text);
            if body.trim().is_empty() {
                return Err("memo_empty");
            }
            let directory = Dir::open_or_create_private(path.parent().unwrap())
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
                    id,
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
