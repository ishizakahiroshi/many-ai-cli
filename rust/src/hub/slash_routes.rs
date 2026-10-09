//! Authenticated method guard lives in Hub; this route owns source/cache calls.
use super::http::{Request, Response, decode_json, require_method};
use crate::{
    application::slash_commands::{SlashCommands, SlashFailure, SourceFailure, SourcePatch},
    proto::time::Timestamp,
};
use std::sync::Arc;
pub const SOURCES_PATH: &str = "/api/slash-cmd-sources";
pub const COMMANDS_PATH: &str = "/api/slash-commands";
pub fn is_route(path: &str) -> bool {
    matches!(path, SOURCES_PATH | COMMANDS_PATH)
}
pub struct SlashHttp {
    owner: Arc<SlashCommands>,
}
impl SlashHttp {
    pub fn new(owner: Arc<SlashCommands>) -> Self {
        Self { owner }
    }
    pub async fn handle_authenticated(
        &self,
        request: &Request,
        now: Timestamp,
    ) -> Option<Response> {
        if !is_route(&request.path) {
            return None;
        }
        if let Err(error) = require_method(request, &["GET", "POST"]) {
            return Some(error);
        }
        Some(if request.path == SOURCES_PATH {
            if request.method == "GET" {
                match self.owner.sources() {
                    Ok(sources) => Response::json(200, &sources),
                    Err(_) => Response::error(500, "save_failed", "save failed"),
                }
            } else {
                let patch: SourcePatch = match decode_json(request) {
                    Ok(patch) => patch,
                    Err(error) => return Some(error),
                };
                match self.owner.patch_sources(&patch) {
                    Ok(()) => Response::json(200, &serde_json::json!({"ok":true})),
                    Err(SourceFailure::Invalid(detail)) => {
                        Response::error(400, "bad_request", &detail)
                    }
                    Err(SourceFailure::Save) => Response::error(500, "save_failed", "save failed"),
                }
            }
        } else {
            let provider = request.query("provider");
            let mut session = request.query("session_id");
            if session.trim().is_empty() {
                session = request.query("session")
            }
            let context = self.owner.search_context(&provider, &session);
            match self
                .owner
                .commands(&provider, context, request.method == "POST", now)
                .await
            {
                Ok(response) => Response::json(200, &response),
                Err(SlashFailure::InvalidProvider) => {
                    Response::error(400, "bad_request", "invalid provider")
                }
                Err(SlashFailure::NotConfigured) => {
                    Response::error(404, "not_found", "source URL not configured")
                }
                Err(SlashFailure::Fetch(detail)) => {
                    Response::error(502, "fetch_failed", &format!("fetch failed: {detail}"))
                }
                Err(SlashFailure::Unavailable) => Response::error(
                    503,
                    "slash_unavailable",
                    "slash command service unavailable",
                ),
            }
        })
    }
}
