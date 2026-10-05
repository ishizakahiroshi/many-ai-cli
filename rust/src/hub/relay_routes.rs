//! Five fixed-Go relay branches behind the router's token/Host/Origin guard.
//! Mutations transfer to HubTaskOwner before waiting; disconnecting an HTTP
//! caller never abandons a reserved or partially launched relay.
use super::http::{Request, Response, decode_json, require_method};
use crate::{
    application::relay_program::{
        RelayControlRequest, RelayError, RelayProgram, RelayStartRequest,
    },
    proto::{core::*, time::Timestamp},
};
use std::sync::Arc;
pub fn is_relay_route(path: &str) -> bool {
    path.strip_prefix("/api/sessions/").is_some_and(|tail| {
        matches!(
            tail.trim_matches('/').split('/').next_back(),
            Some("relay" | "relay-stop" | "relay-resume" | "relay-cleanup")
        )
    })
}
pub fn methods(path: &str) -> &'static [&'static str] {
    if path.trim_end_matches('/').ends_with("/relay") {
        &["GET", "POST"]
    } else {
        &["POST"]
    }
}
pub struct RelayHttp {
    program: Arc<RelayProgram>,
}
impl RelayHttp {
    pub fn new(program: Arc<RelayProgram>) -> Self {
        Self { program }
    }
    pub async fn handle_authenticated(
        &self,
        request: &Request,
        waiter: &HttpWaitCancellation,
        now: Timestamp,
    ) -> Option<Response> {
        if !is_relay_route(&request.path) {
            return None;
        }
        let parent = match super::confirmations::parent_path(&request.path) {
            Ok(id) => LiveSessionId(id),
            Err(error) => return Some(error),
        };
        if let Err(error) = require_method(request, methods(&request.path)) {
            return Some(error);
        }
        let action = request
            .path
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or("");
        if action == "relay" && request.method == "GET" {
            return Some(match self.program.list(parent) {
                Ok(items) => Response::json(200, &serde_json::json!({"ok":true,"relays":items})),
                Err(error) => response(error),
            });
        }
        enum Operation {
            Start(RelayStartRequest),
            Stop(String),
            Resume(String),
            Cleanup(String),
        }
        let operation = if action == "relay" {
            match decode_json(request) {
                Ok(body) => Operation::Start(body),
                Err(error) => return Some(error),
            }
        } else {
            let body: RelayControlRequest = if request.body.is_empty() {
                RelayControlRequest::default()
            } else {
                match decode_json(request) {
                    Ok(body) => body,
                    Err(error) => return Some(error),
                }
            };
            match action {
                "relay-stop" => Operation::Stop(body.orchestration_id),
                "relay-resume" => Operation::Resume(body.orchestration_id),
                _ => Operation::Cleanup(body.orchestration_id),
            }
        };
        let permit = match self.program.tasks().effect_permit() {
            Ok(permit) => permit,
            Err(error) => return Some(response(error.into())),
        };
        let cancel = permit.cancellation();
        let program = self.program.clone();
        let complete=permit.start(async move{match operation{
            Operation::Start(body)=>match program.start_relay(parent,body,now,cancel).await{Ok((status,board))=>Response::json(200,&serde_json::json!({"ok":true,"orchestration_id":status.orchestration_id,"board_path":board,"implementation_session_id":status.implementation_session_id,"worktree_path":status.worktree_path,"branch":status.branch,"relay":status})),Err(error)=>response(error)},
            Operation::Stop(id)=>control(program.stop_relay(parent,&id,now).await),
            Operation::Resume(id)=>control(program.resume_relay(parent,&id,now,cancel).await),
            Operation::Cleanup(id)=>control(program.cleanup_relay(parent,&id,now,cancel).await),
        }});
        let token = waiter.token();
        Some(
            tokio::select! {result=complete.wait()=>match result{Ok(response)=>response,Err(_)=>Response::error(503,"relay_cancelled","relay operation was cancelled")},_=token.cancelled()=>Response::error(503,"relay_cancelled","relay request was cancelled")},
        )
    }
}
fn control(result: Result<crate::proto::RelayStatus, RelayError>) -> Response {
    match result {
        Ok(status) => Response::json(200, &serde_json::json!({"ok":true,"relay":status})),
        Err(error) => response(error),
    }
}
fn response(error: RelayError) -> Response {
    Response::error(error.status, &error.code, &error.detail)
}
