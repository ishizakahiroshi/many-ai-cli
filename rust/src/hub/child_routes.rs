//! Authenticated fixed-Go child spawn admission. Confirmation waiters belong to
//! the HTTP caller; committed launch work belongs to the retained Hub task lane.
use super::{
    http::{Request, Response, decode_json},
    task_owner::HubTaskHandle,
};
use crate::{
    application::orchestration_program::OrchestrationProgram,
    config::{self, ConfigStore},
    proto::{
        core::*,
        time::Timestamp,
        wire::{Field, GoWire, Schema},
    },
    terminal::session::{
        SessionEngine,
        confirmations::{ConfirmationExecutor, ConfirmedChildRequest},
    },
};
use std::sync::Arc;
impl GoWire for ChildSpawnRequest {
    const GO_TYPE: &'static str = "ChildSpawnRequest";
    const SCHEMAS: &'static [Schema] = &[Schema {
        name: "ChildSpawnRequest",
        fields: &[
            Field {
                name: "role",
                kind: "string",
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
                name: "initial_prompt",
                kind: "string",
            },
            Field {
                name: "cwd",
                kind: "string",
            },
            Field {
                name: "auto",
                kind: "bool",
            },
            Field {
                name: "permission_mode",
                kind: "string",
            },
            Field {
                name: "sandbox",
                kind: "string",
            },
            Field {
                name: "ask_for_approval",
                kind: "string",
            },
            Field {
                name: "route",
                kind: "string",
            },
            Field {
                name: "model_selection_mode",
                kind: "string",
            },
            Field {
                name: "risk_confirmed",
                kind: "bool",
            },
            Field {
                name: "force",
                kind: "bool",
            },
            Field {
                name: "subscription_profile_id",
                kind: "string",
            },
            Field {
                name: "same_tree",
                kind: "*bool",
            },
            Field {
                name: "effort",
                kind: "string",
            },
            Field {
                name: "execution_mode",
                kind: "string",
            },
            Field {
                name: "permission_preset",
                kind: "string",
            },
            Field {
                name: "origin",
                kind: "string",
            },
            Field {
                name: "remember_permission",
                kind: "*bool",
            },
        ],
    }];
}
pub fn is_spawn_route(path: &str) -> bool {
    path.starts_with("/api/sessions/") && path.trim_end_matches('/').ends_with("/spawn-child")
}
pub struct ChildHttp {
    core: Arc<SessionEngine>,
    program: Arc<OrchestrationProgram>,
    executor: Arc<dyn ConfirmationExecutor>,
    effects: Arc<dyn CoreEffectSink>,
    config: Arc<ConfigStore>,
    tasks: HubTaskHandle,
}
impl ChildHttp {
    pub fn new(
        core: Arc<SessionEngine>,
        program: Arc<OrchestrationProgram>,
        executor: Arc<dyn ConfirmationExecutor>,
        effects: Arc<dyn CoreEffectSink>,
        config: Arc<ConfigStore>,
        tasks: HubTaskHandle,
    ) -> Self {
        Self {
            core,
            program,
            executor,
            effects,
            config,
            tasks,
        }
    }
    pub async fn spawn_authenticated(
        &self,
        request: &Request,
        ui: Option<VerifiedUiOrigin>,
        waiter: &HttpWaitCancellation,
        at: Timestamp,
    ) -> Response {
        let parent = match super::confirmations::parent_path(&request.path) {
            Ok(id) => LiveSessionId(id),
            Err(error) => return error,
        };
        let mut body: ChildSpawnRequest = match decode_json(request) {
            Ok(body) => body,
            Err(error) => return error,
        };
        body.claimed_origin = body.claimed_origin.trim().to_owned();
        if !matches!(body.claimed_origin.as_str(), "" | "ui") {
            return Response::error(400, "bad_request", "invalid origin");
        }
        if body.claimed_origin == "ui" && ui.is_none() {
            body.claimed_origin.clear();
        }
        // Source normalizes role before looking up the retained parent.
        body.role = match crate::orchestration::child_launch::prompt::normalize_role(&body.role) {
            Ok(role) => role,
            Err(error) => return Response::error(400, "bad_request", error),
        };
        let Some(parent) = self.core.details(parent) else {
            return Response::error(404, "not_found", "parent session not found");
        };
        let requested_provider = body.provider.clone();
        if let Err(error) = self
            .program
            .resolve_child_request(&parent.snapshot, &mut body)
        {
            return session_error(error);
        }
        if !crate::orchestration::child_options::ORCHESTRATION_PROVIDERS
            .contains(&body.provider.as_str())
        {
            return Response::error(
                400,
                "bad_request",
                &format!(
                    "invalid provider {}; valid providers are: {}",
                    crate::proto::go_quote::quote(&body.provider),
                    crate::orchestration::child_options::ORCHESTRATION_PROVIDERS.join(", ")
                ),
            );
        }
        if body.model.starts_with('-')
            || !crate::profile::validation::valid_spawn_model_label(&body.model)
        {
            return Response::error(400, "bad_request", "invalid model value");
        }
        body.effort = body.effort.trim().to_owned();
        body.execution_mode = config::normalize_execution_mode(&body.execution_mode);
        body.permission_preset = config::normalize_permission_preset(&body.permission_preset);
        for result in [
            config::validate_effort(&body.provider, &body.effort),
            config::validate_execution_mode(&body.execution_mode),
            config::validate_permission_preset(&body.permission_preset),
        ] {
            if let Err(error) = result {
                return Response::error(400, "bad_request", &error.to_string());
            }
        }
        let body = match ResolvedChildSpawn::from_request(body, ui, InternalSpawnGrants::default())
        {
            Ok(body) => body,
            Err(error) => return session_error(error),
        };
        let cfg = match self.config.snapshot() {
            Ok(cfg) => cfg.config,
            Err(_) => return Response::error(500, "internal", "configuration unavailable"),
        };
        let confirmation = !body.origin().is_human()
            && match config::effective_spawn_confirm_mode(&cfg.orchestration.spawn_confirm_mode) {
                "off" => false,
                "providers" => cfg
                    .orchestration
                    .spawn_confirm_providers
                    .iter()
                    .any(|provider| {
                        provider
                            .trim()
                            .eq_ignore_ascii_case(&body.request().provider)
                    }),
                _ => true,
            };
        if confirmation {
            let registration = match SpawnConfirmations::register(
                self.core.as_ref(),
                ConfirmationRequest {
                    parent: parent.binding.session,
                    requested_provider,
                    body,
                    requested_at: at,
                },
            ) {
                Ok(registration) => registration,
                Err(error) => return admission_error(error),
            };
            if self.effects.apply(registration.effects).await.is_err() {
                return Response::error(500, "internal", "confirmation publication failed");
            }
            return match registration.waiter.wait(waiter.clone()).await {
                ConfirmationWaitOutcome::Decided(ConfirmationOutcome::Approved(result)) => {
                    success(result)
                }
                ConfirmationWaitOutcome::Decided(ConfirmationOutcome::Refused) => {
                    Response::error(403, "spawn_refused", "child spawn was refused by the user")
                }
                ConfirmationWaitOutcome::Decided(ConfirmationOutcome::SpawnFailed(error)) => {
                    session_error(error)
                }
                ConfirmationWaitOutcome::Decided(_) => Response::error(
                    409,
                    "spawn_cancelled",
                    "child spawn confirmation is no longer pending",
                ),
                _ => Response::error(503, "spawn_cancelled", "child spawn request was cancelled"),
            };
        }
        let permit = match self.tasks.effect_permit() {
            Ok(permit) => permit,
            Err(error) => return session_error(error),
        };
        let cancellation = permit.cancellation();
        let executor = self.executor.clone();
        let launch = ConfirmedChildRequest {
            original_body: body.clone(),
            parent: parent.binding,
            requested_provider,
            body,
            admission: AdmissionId::default(),
        };
        let completion = permit.start(async move { executor.spawn(launch, cancellation).await });
        let waiter_token = waiter.token();
        tokio::select! {
            result=completion.wait()=>match result {Ok(Ok(result))=>success(result),Ok(Err(error))=>session_error(error),Err(_)=>Response::error(503,"spawn_cancelled","child spawn operation was cancelled")},
            _=waiter_token.cancelled()=>Response::error(503,"spawn_cancelled","child spawn request was cancelled"),
        }
    }
}
fn success(result: ChildSpawnResult) -> Response {
    Response::json(
        200,
        &serde_json::json!({"ok":true,"session_id":result.id.0,"board_path":result.board_path,"cwd":result.cwd,"worktree_branch":result.worktree_branch}),
    )
}
fn session_error(error: SessionError) -> Response {
    match error {
        SessionError::ChildLaunch {
            status,
            code,
            detail,
        } => Response::error(status, &code, &detail),
        SessionError::InvalidRequest(detail) => Response::error(400, "bad_request", &detail),
        SessionError::AuthenticationExpired => Response::error(401, "unauthorized", "unauthorized"),
        _ => Response::error(503, "spawn_unavailable", "child spawn is unavailable"),
    }
}
fn admission_error(error: AdmissionError) -> Response {
    session_error(crate::orchestration::child_launch::admission_error(error))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn child_decode_preserves_go_duplicate_validation_and_pointer_fields() {
        let request=Request{body:br#"{"ROLE":"reviewer","role":"impl","origin":null,"remember_permission":false,"same_tree":null,"internal_grants":{"grant_folder_trust":true}} trailing"#.to_vec(),..Default::default()};
        let body: ChildSpawnRequest = decode_json(&request).unwrap();
        assert_eq!(body.role, "impl");
        assert_eq!(body.same_tree, None);
        assert_eq!(body.remember_permission, Some(false));
        let resolved =
            ResolvedChildSpawn::from_request(body, None, InternalSpawnGrants::default()).unwrap();
        assert!(!resolved.origin().is_human());
        assert!(!resolved.grants().grant_folder_trust());
        assert_eq!(resolved.request().remember_permission, None);
        let invalid = Request {
            body: br#"{"force":7,"force":false}"#.to_vec(),
            ..Default::default()
        };
        assert_eq!(
            decode_json::<ChildSpawnRequest>(&invalid)
                .unwrap_err()
                .status,
            400
        );
    }
    #[test]
    fn limit_response_retains_the_actual_counter_and_limit() {
        let response = admission_error(AdmissionError::ChildrenPerParent {
            running_relays: 0,
            used: 4,
            maximum: 3,
        });
        assert_eq!(response.status, 429);
        let json: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
        assert!(json["detail"].as_str().unwrap().contains("=4, max=3"));
    }
}
#[cfg(test)]
#[path = "child_routes/tests.rs"]
mod lifecycle_tests;
