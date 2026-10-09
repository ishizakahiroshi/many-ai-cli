//! Authentication is enforced by the router before these source method guards.
use super::http::{Request, Response, require_method};
use crate::{application::instruction_rules::InstructionRules, process::Cancellation};
use std::sync::Arc;
pub fn methods(path: &str) -> Option<&'static [&'static str]> {
    match path {
        "/api/approval/status" => Some(&["GET"]),
        "/api/approval/enable" | "/api/approval/disable" | "/api/approval/dismiss" => {
            Some(&["POST"])
        }
        _ => None,
    }
}
pub struct ApprovalStatusHttp {
    owner: Arc<InstructionRules>,
}
impl ApprovalStatusHttp {
    pub fn new(owner: Arc<InstructionRules>) -> Self {
        Self { owner }
    }
    pub async fn handle_authenticated(&self, request: &Request, cancel: &Cancellation) -> Response {
        let Some(allowed) = methods(&request.path) else {
            return Response::error(404, "not_found", "not found");
        };
        if let Err(response) = require_method(request, allowed) {
            return response;
        }
        if request.path == "/api/approval/status" {
            return match self.owner.status() {
                Ok(v) => Response::json(200, &v),
                Err(_) => Response::error(500, "config_unavailable", "configuration unavailable"),
            };
        }
        let action = request.path.rsplit('/').next().unwrap_or("");
        match self.owner.change(action, cancel).await {
            Ok(()) if action == "dismiss" => {
                let mut response = Response::bytes(204, "", Vec::new());
                response.headers.remove("Content-Type");
                response
            }
            Ok(()) => Response::json(200, &serde_json::json!({"ok":true})),
            Err(_) => Response::error(500, "config_unavailable", "configuration unavailable"),
        }
    }
}
