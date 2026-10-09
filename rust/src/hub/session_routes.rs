//! Fixed-Go session HTTP adapters. The router owns authentication and must run
//! the source-specific method/Host/Origin/PIN guards before these operations.
//! This service reads the very same journal repository and core used by sockets;
//! it neither creates another session registry nor opens a second database.
use super::http::{Request, Response};
use crate::{
    config::Config,
    proto::core::{DbSessionId, LiveSessionId, SessionCore, SessionStorage},
    terminal::journal::SessionJournal,
};
use serde_json::json;
use std::sync::{Arc, Mutex};

mod info;
mod log;
mod maintenance;
mod metadata;
pub use info::InfoContext;

pub const READ_ROUTES: &[&str] = &[
    "/api/session-chat",
    "/api/session-log",
    "/api/session-search",
    "/api/session-history",
    "/api/approval-history",
];
pub const MAINTENANCE_ROUTES: &[&str] = &[
    "/api/session-store/reset",
    "/api/session-store/prune-transcript-noise",
];
pub const LOG_MAINTENANCE_ROUTES: &[&str] = &["/api/logs/purge", "/api/logs/legacy-notice"];

pub struct SessionHttp {
    core: Arc<dyn SessionCore>,
    journal: Arc<SessionJournal>,
    stale_notified: Mutex<bool>,
    log_maintenance: Mutex<()>,
}
impl SessionHttp {
    pub fn handle_maintenance_authenticated(&self, request: &Request) -> Response {
        match request.path.as_str() {
            "/api/session-store/reset" => {
                let Some(storage) = self.storage() else {
                    return Response::json(200, &json!({"ok":true,"result":{}}));
                };
                let ids = self.core.registered_session_ids();
                let result = storage.reset_history(&ids);
                let pending = storage.file_reset_pending();
                match result {
                    Ok(result) => Response::json(
                        200,
                        &json!({"ok":true,"result":result,"file_reset_scheduled":pending}),
                    ),
                    Err(_) => Response::error(
                        500,
                        "session_store_reset_failed",
                        "failed to reset saved session history",
                    ),
                }
            }
            "/api/session-store/prune-transcript-noise" => match self
                .storage()
                .map(|s| s.prune_transcript_noise())
                .unwrap_or(Ok(0))
            {
                Ok(count) => Response::json(200, &json!({"ok":true,"deleted_messages":count})),
                Err(_) => Response::error(
                    500,
                    "transcript_noise_prune_failed",
                    "failed to prune transcript noise",
                ),
            },
            _ => Response::error(404, "not_found", "not found"),
        }
    }
    pub fn new(core: Arc<dyn SessionCore>, journal: Arc<SessionJournal>) -> Self {
        Self {
            core,
            journal,
            stale_notified: Mutex::new(false),
            log_maintenance: Mutex::new(()),
        }
    }
    fn storage(&self) -> Option<&dyn SessionStorage> {
        self.journal.storage().map(AsRef::as_ref)
    }
    /// GET guards must precede this call, including on absent-store results.
    pub fn handle_read_authenticated(&self, request: &Request, config: &Config) -> Response {
        match request.path.as_str() {
            "/api/session-chat" => self.chat(request),
            "/api/session-log" => self.log(request, config),
            "/api/session-search" => self.search(request),
            "/api/session-history" => self.history(request),
            "/api/approval-history" => self.approvals(request),
            _ => Response::error(404, "not_found", "not found"),
        }
    }
    fn chat(&self, request: &Request) -> Response {
        let Some(storage) = self.storage() else {
            return Response::json(200, &json!({"ok": true, "messages": []}));
        };
        let limit = integer(&request.query("limit")).unwrap_or_default();
        let db = request.query("session_db_id");
        let result = if !db.trim().is_empty() {
            let Some(id) = positive_id(db.trim()) else {
                return bad_request("invalid session_db_id");
            };
            storage.chat_messages_by_session_id(DbSessionId(id), limit)
        } else {
            // Unlike the database ID and approval query, Go does not trim this.
            let Some(id) = positive_id(&request.query("session_id")) else {
                return bad_request("session_id required");
            };
            storage.chat_messages_by_live_session(LiveSessionId(id), limit)
        };
        match result {
            Ok(messages) => Response::json(200, &json!({"ok": true, "messages": messages})),
            Err(_) => Response::error(500, "session_chat_failed", "failed to restore chat"),
        }
    }
    fn search(&self, request: &Request) -> Response {
        let query = request.query("q");
        let query = query.trim();
        if query.is_empty() {
            return Response::json(200, &json!({"ok": true, "results": []}));
        }
        let Some(storage) = self.storage() else {
            return Response::json(200, &json!({"ok": true, "results": []}));
        };
        match storage.search_messages(query, integer(&request.query("limit")).unwrap_or_default()) {
            Ok(results) => Response::json(200, &json!({"ok": true, "results": results})),
            Err(_) => Response::error(500, "session_search_failed", "failed to search sessions"),
        }
    }
    fn history(&self, request: &Request) -> Response {
        let Some(storage) = self.storage() else {
            return Response::json(200, &json!({"ok": true, "sessions": []}));
        };
        match storage.list_sessions(integer(&request.query("limit")).unwrap_or_default(), true) {
            Ok(sessions) => Response::json(200, &json!({"ok": true, "sessions": sessions})),
            Err(_) => Response::error(
                500,
                "session_history_failed",
                "failed to list saved sessions",
            ),
        }
    }
    fn approvals(&self, request: &Request) -> Response {
        let Some(storage) = self.storage() else {
            return Response::json(200, &json!({"ok": true, "approvals": []}));
        };
        let limit = integer(&request.query("limit")).unwrap_or_default();
        let pending = request
            .query("state")
            .trim()
            .eq_ignore_ascii_case("pending");
        let db = request.query("session_db_id");
        let live = request.query("session_id");
        let result = if !db.trim().is_empty() {
            let Some(id) = positive_id(db.trim()) else {
                return bad_request("invalid session_db_id");
            };
            storage.approvals_by_session_id(DbSessionId(id), limit, pending)
        } else if !live.trim().is_empty() {
            let Some(id) = positive_id(live.trim()) else {
                return bad_request("invalid session_id");
            };
            storage.approvals_by_live_session(LiveSessionId(id), limit, pending)
        } else {
            storage.recent_approvals(limit, pending)
        };
        match result {
            Ok(approvals) => Response::json(
                200,
                &json!({"ok": true, "approvals": approvals.unwrap_or_default()}),
            ),
            Err(_) => Response::error(
                500,
                "approval_history_failed",
                "failed to read approval history",
            ),
        }
    }
}
fn bad_request(detail: &str) -> Response {
    Response::error(400, "bad_request", detail)
}
/// strconv.Atoi/ParseInt base 10: signs are accepted, whitespace/underscores are
/// not. All supported Go targets have 64-bit int.
fn integer(value: &str) -> Option<i64> {
    value.parse().ok()
}
fn positive_id(value: &str) -> Option<i64> {
    integer(value).filter(|id| *id > 0)
}

/// The singular prefix owns all suffixes. For the plural namespace only the
/// meta suffix is ours; orchestration keeps the other operations.
pub fn is_metadata_route(path: &str) -> bool {
    path.starts_with("/api/session/")
        || path
            .strip_prefix("/api/sessions/")
            .is_some_and(|rest| rest.trim_matches('/').split('/').next_back() == Some("meta"))
}
/// Call after token authentication but BEFORE method/Host/Origin/PIN guards.
/// Go trims leading/trailing slashes in the remainder, not interior ones.
pub fn metadata_path(path: &str) -> Result<LiveSessionId, Response> {
    let singular = path.strip_prefix("/api/session/");
    let rest = singular.or_else(|| path.strip_prefix("/api/sessions/"));
    let Some(rest) = rest else {
        return Err(Response::error(404, "not_found", "not found"));
    };
    let parts: Vec<_> = rest.trim_matches('/').split('/').collect();
    if parts.len() != 2 || (singular.is_some() && parts[1] != "meta") {
        return Err(Response::error(404, "not_found", "not found"));
    }
    let Some(id) = positive_id(parts[0]) else {
        return Err(bad_request("invalid session id"));
    };
    if parts[1] != "meta" {
        return Err(Response::error(404, "not_found", "not found"));
    }
    Ok(LiveSessionId(id))
}

#[cfg(test)]
mod tests;
