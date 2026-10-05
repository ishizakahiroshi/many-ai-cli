//! Guarded provider history reads; native opening additionally requires loopback.
use super::{
    auth,
    http::{Request, Response, require_method},
};
use crate::{
    application::agent_history::{AgentHistory, session_id},
    process::Cancellation,
};
use std::sync::Arc;
pub fn methods(path: &str) -> Option<&'static [&'static str]> {
    match path {
        "/api/agent-log" | "/api/agent-chat" | "/api/grok-history" => Some(&["GET"]),
        "/api/agent-log/open" => Some(&["POST"]),
        _ => None,
    }
}
pub struct AgentHistoryHttp {
    owner: Arc<AgentHistory>,
}
impl AgentHistoryHttp {
    pub fn new(owner: Arc<AgentHistory>) -> Self {
        Self { owner }
    }
    pub fn handle_authenticated(&self, request: &Request, cancel: &Cancellation) -> Response {
        let Some(allowed) = methods(&request.path) else {
            return Response::error(404, "not_found", "not found");
        };
        if let Err(response) = require_method(request, allowed) {
            return response;
        }
        if request.path == "/api/agent-log/open" && !auth::is_loopback(&request.remote_addr) {
            return Response::error(403, "forbidden", "loopback only");
        }
        let id = match session_id(&request.query("session_id")) {
            Ok(id) => id,
            Err(error) => return Response::error(error.status, error.code, error.detail),
        };
        if request.path == "/api/agent-log" {
            return Response::json(200, &self.owner.location(id));
        }
        let result = match request.path.as_str() {
            "/api/agent-chat" => self.owner.chat(
                id,
                &request.query("limit"),
                &request.query("cursor"),
                &request.query("offset"),
            ),
            "/api/grok-history" => {
                self.owner
                    .grok_history(id, &request.query("limit"), &request.query("offset"))
            }
            _ => self.owner.open_log(id, cancel),
        };
        match result {
            Ok(value) => Response::json(200, &value),
            Err(error) => Response::error(error.status, error.code, error.detail),
        }
    }
}
