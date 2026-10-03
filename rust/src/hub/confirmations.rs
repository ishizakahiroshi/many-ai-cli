//! Ordinary authenticated spawn-confirm HTTP boundary. Source:
//! internal/hub/orchestration.go:803-868. No UI-origin cookie or WebSocket
//! membership is required; the existing token/Host/Origin/PIN guard precedes it.
use super::{
    http::{Request, Response, decode_json},
    task_owner::HubTaskHandle,
};
use crate::proto::{
    core::*,
    time::Timestamp,
    wire::{Field, GoWire, Schema},
};
use std::sync::Arc;
impl GoWire for SpawnConfirmationResponse {
    const GO_TYPE: &'static str = "SpawnConfirmationResponse";
    const SCHEMAS: &'static [Schema] = &[Schema {
        name: "SpawnConfirmationResponse",
        fields: &[
            Field {
                name: "confirmation_id",
                kind: "string",
            },
            Field {
                name: "approved",
                kind: "bool",
            },
            Field {
                name: "provider",
                kind: "string",
            },
            Field {
                name: "model",
                kind: "string",
            },
            Field {
                name: "effort",
                kind: "*string",
            },
            Field {
                name: "execution_mode",
                kind: "*string",
            },
            Field {
                name: "permission_preset",
                kind: "*string",
            },
            Field {
                name: "remember_permission",
                kind: "*bool",
            },
            Field {
                name: "grant_folder_trust",
                kind: "*bool",
            },
        ],
    }];
}
pub fn is_confirmation_route(path: &str) -> bool {
    path.strip_prefix("/api/sessions/")
        .is_some_and(|rest| rest.trim_matches('/').split('/').next_back() == Some("spawn-confirm"))
}
pub(crate) fn parent_path(path: &str) -> Result<i64, Response> {
    let rest = path
        .strip_prefix("/api/sessions/")
        .unwrap_or("")
        .trim_matches('/');
    let parts: Vec<_> = rest.split('/').collect();
    if parts.len() != 2 {
        return Err(Response::error(404, "not_found", "not found"));
    }
    parts[0]
        .parse::<i64>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or_else(|| Response::error(400, "bad_request", "invalid session id"))
}
pub struct ConfirmationHttp {
    core: Arc<dyn SpawnConfirmations>,
}
impl ConfirmationHttp {
    pub fn new(core: Arc<dyn SpawnConfirmations>) -> Self {
        Self { core }
    }
    pub fn handle_authenticated(
        &self,
        request: &Request,
        proof: VerifiedConfirmationRequest,
        now: Timestamp,
        tasks: &HubTaskHandle,
    ) -> Response {
        // Baseline validates the URL ID, then resolves the globally unique
        // pending confirmation ID; it does not require an active browser socket.
        if let Err(error) = parent_path(&request.path) {
            return error;
        }
        let body: SpawnConfirmationResponse = match decode_json(request) {
            Ok(body) => body,
            Err(error) => return error,
        };
        let accepted = match self.core.clone().accept_decision(body, proof, now) {
            Ok(accepted) => accepted,
            Err(error) => return response_error(error),
        };
        let permit = match tasks.effect_permit() {
            Ok(permit) => permit,
            Err(error) => return response_error(error),
        };
        let future = match accepted.run(permit.cancellation()) {
            Ok(future) => future,
            Err(error) => return response_error(error),
        };
        // No await/fallible enqueue after synchronous decision commit. Dropping
        // this completion waiter cannot cancel the accepted child/refusal task.
        let _completion = permit.start(future);
        Response::json(200, &serde_json::json!({"ok":true}))
    }
}
fn response_error(error: SessionError) -> Response {
    match error {
        SessionError::ConfirmationMissing => {
            Response::error(404, "not_found", "spawn confirmation is no longer pending")
        }
        SessionError::ConfirmationDecided => Response::error(
            409,
            "already_decided",
            "spawn confirmation has already been decided",
        ),
        SessionError::InvalidRequest(detail) => Response::error(400, "bad_request", &detail),
        SessionError::ChildLaunch {
            status,
            code,
            detail,
        } => Response::error(status, &code, &detail),
        SessionError::AuthenticationExpired => Response::error(401, "unauthorized", "unauthorized"),
        SessionError::Shutdown => Response::error(
            503,
            "confirmation_unavailable",
            "confirmation service is unavailable",
        ),
        _ => Response::error(500, "spawn_error", "confirmation operation failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decoder_retains_go_first_value_duplicate_and_pointer_semantics() {
        let request=Request { body:br#"{"CONFIRMATION_ID":"first","confirmation_id":"last","APPROVED":true,"effort":null,"permission_preset":"","grant_folder_trust":false} trailing"#.to_vec(),..Default::default() };
        let response: SpawnConfirmationResponse = decode_json(&request).unwrap();
        assert_eq!(response.confirmation_id, SpawnConfirmationId("last".into()));
        assert!(response.approved);
        assert_eq!(response.effort, None);
        assert_eq!(response.permission_preset, Some(String::new()));
        assert_eq!(response.grant_folder_trust, Some(false));
        let invalid = Request {
            body: br#"{"approved":7,"approved":true}"#.to_vec(),
            ..Default::default()
        };
        assert_eq!(
            decode_json::<SpawnConfirmationResponse>(&invalid)
                .unwrap_err()
                .status,
            400
        );
    }
    #[test]
    fn typed_missing_and_decided_responses_keep_source_statuses() {
        assert_eq!(
            response_error(SessionError::ConfirmationMissing).status,
            404
        );
        assert_eq!(
            response_error(SessionError::ConfirmationDecided).status,
            409
        );
        assert_eq!(
            parent_path("/api/sessions/0/spawn-confirm")
                .unwrap_err()
                .status,
            400
        );
        assert_eq!(
            parent_path("/api/sessions/one/spawn-confirm")
                .unwrap_err()
                .status,
            400
        );
        assert_eq!(
            parent_path("/api/sessions/1/x/spawn-confirm")
                .unwrap_err()
                .status,
            404
        );
        assert_eq!(parent_path("/api/sessions/+1/spawn-confirm").unwrap(), 1);
    }
}
