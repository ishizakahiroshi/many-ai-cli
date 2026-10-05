//! /api/session-log is a bounded, masked JSON range, not a raw file download.
//! Handing the original file to StreamedFile would bypass its privacy contract.
//!
//! Open compatibility decision: the inherited shared reader rejects non-regular
//! entries as 404 "session log not readable". Go os.Open permits such entries;
//! directories may fail at ReadAt with 500 and FIFOs may block at open. This
//! slice proves ordinary regular-log behavior only, not acceptance of that
//! stricter shared-reader boundary. No new non-regular/path probes are included.
use super::*;
use crate::{files::scope, storage::mask_secret_bytes};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::{
    io::{Read, Seek, SeekFrom},
    path::Path,
};

const DEFAULT_CHUNK: u64 = 128 * 1024;
const MAX_CHUNK: u64 = 512 * 1024;

impl SessionHttp {
    pub(super) fn log(&self, request: &Request, config: &Config) -> Response {
        let db = request.query("session_db_id");
        let (db_id, live_id) = if !db.is_empty() {
            let Some(id) = positive_id(&db) else {
                return bad_request("invalid session_db_id");
            };
            (id, 0)
        } else {
            let Some(id) = positive_id(&request.query("session_id")) else {
                return bad_request("invalid session_id");
            };
            (0, id)
        };
        let path = if db_id > 0 && self.storage().is_some() {
            self.storage()
                .and_then(|s| s.session_overview_by_session_id(DbSessionId(db_id)).ok())
                .map(|s| s.log_path)
                .unwrap_or_default()
        } else {
            let live = self
                .core
                .snapshot(LiveSessionId(live_id))
                .map(|s| s.log_path)
                .unwrap_or_default();
            if live.is_empty() {
                self.storage()
                    .and_then(|s| {
                        s.session_overview_by_live_session(LiveSessionId(live_id))
                            .ok()
                    })
                    .map(|s| s.log_path)
                    .unwrap_or_default()
            } else {
                live
            }
        };
        if path.is_empty() {
            return Response::error(404, "not_found", "session log not found");
        }
        if config.hub.log_dir.is_empty() {
            return Response::error(404, "not_found", "session log not available");
        }
        let clean = scope::clean(Path::new(&path));
        let allowed = Path::new(&config.hub.log_dir).join("sessions");
        let canonical = match scope::canonical(&clean) {
            Ok(path) => path,
            Err(_) => return Response::error(403, "forbidden", "log path outside log dir"),
        };
        if !scope::under_roots(&canonical, &[allowed]) {
            return Response::error(403, "forbidden", "log path outside log dir");
        }
        // Resolve the same canonical scope, then use the shared held-directory
        // reader so the raw path is not reopened after permission validation.
        let file = scope::parent_capability(&canonical)
            .and_then(|(dir, name)| dir.open_file(&name, false));
        let Ok(mut file) = file else {
            return Response::error(404, "not_found", "session log not readable");
        };
        let size = match file.metadata() {
            Ok(meta) => meta.len(),
            Err(error) => {
                return Response::error(500, "internal_error", &format!("stat failed: {error}"));
            }
        };
        let raw_limit = request.query("limit");
        let limit = if raw_limit.is_empty() {
            DEFAULT_CHUNK
        } else {
            let Some(limit) = positive_id(&raw_limit) else {
                return bad_request("invalid limit");
            };
            (limit as u64).min(MAX_CHUNK)
        };
        let raw_offset = request.query("offset");
        let offset = if raw_offset.is_empty() {
            -1
        } else {
            let Some(offset) = integer(&raw_offset) else {
                return bad_request("invalid offset");
            };
            offset
        };
        let offset = if offset < 0 {
            size.saturating_sub(limit)
        } else {
            (offset as u64).min(size)
        };
        let wanted = limit.min(size - offset);
        let mut bytes = Vec::with_capacity(wanted as usize);
        if let Err(error) = file
            .seek(SeekFrom::Start(offset))
            .and_then(|_| file.take(wanted).read_to_end(&mut bytes))
        {
            return Response::error(500, "internal_error", &format!("read failed: {error}"));
        }
        let masked = mask_secret_bytes(&bytes);
        Response::json(
            200,
            &json!({"ok":true,"size":size,"offset":offset,"length":masked.len(),"data_b64":STANDARD.encode(masked)}),
        )
    }
}
