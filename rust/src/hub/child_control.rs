//! Fixed-Go retained-session controls over the sole SessionEngine input queue.
use super::{
    http::{Request, Response, decode_json, require_method},
    task_owner::HubTaskHandle,
};
use crate::{
    application::{event_observer::EventWarning, orchestration_program::OrchestrationProgram},
    orchestration::child_launch::prompt::sanitize_inject_text,
    proto::{
        core::*,
        time::Timestamp,
        wire::{Field, GoWire, Schema},
    },
    terminal::session::SessionEngine,
};
use serde::Deserialize;
use std::{path::Path, sync::Arc};
#[derive(Default, Deserialize)]
#[serde(default)]
struct SendRequest {
    role: String,
    text: String,
}
impl GoWire for SendRequest {
    const GO_TYPE: &'static str = "sendChildRequest";
    const SCHEMAS: &'static [Schema] = &[Schema {
        name: "sendChildRequest",
        fields: &[
            Field {
                name: "role",
                kind: "string",
            },
            Field {
                name: "text",
                kind: "string",
            },
        ],
    }];
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct InjectRequest {
    text: String,
    from_session_id: i64,
    press_enter: bool,
    interrupt: bool,
}
impl GoWire for InjectRequest {
    const GO_TYPE: &'static str = "injectRequest";
    const SCHEMAS: &'static [Schema] = &[Schema {
        name: "injectRequest",
        fields: &[
            Field {
                name: "text",
                kind: "string",
            },
            Field {
                name: "from_session_id",
                kind: "int",
            },
            Field {
                name: "press_enter",
                kind: "bool",
            },
            Field {
                name: "interrupt",
                kind: "bool",
            },
        ],
    }];
}
pub fn is_control_route(path: &str) -> bool {
    path.starts_with("/api/sessions/")
        && matches!(
            path.trim_end_matches('/').rsplit('/').next(),
            Some("send-child" | "inject" | "children")
        )
}
pub struct ChildControl {
    core: Arc<SessionEngine>,
    program: Arc<OrchestrationProgram>,
    tasks: HubTaskHandle,
    warning: EventWarning,
}
impl ChildControl {
    pub fn new(
        core: Arc<SessionEngine>,
        program: Arc<OrchestrationProgram>,
        tasks: HubTaskHandle,
        warning: EventWarning,
    ) -> Self {
        Self {
            core,
            program,
            tasks,
            warning,
        }
    }
    pub fn handle_authenticated(&self, request: &Request, now: Timestamp) -> Response {
        let path = request
            .path
            .trim_start_matches("/api/sessions/")
            .trim_matches('/');
        let parts: Vec<_> = path.split('/').collect();
        if parts.len() != 2 {
            return Response::error(404, "not_found", "not found");
        }
        let id = match parts[0].parse::<i64>() {
            Ok(id) if id > 0 => LiveSessionId(id),
            _ => return Response::error(400, "bad_request", "invalid session id"),
        };
        let method = if parts[1] == "children" {
            "GET"
        } else {
            "POST"
        };
        if let Err(error) = require_method(request, &[method]) {
            return error;
        }
        match parts[1] {
            "children" => Response::json(
                200,
                &serde_json::json!({"ok":true,"children":self.core.snapshots().into_iter().filter(|child|child.parent_session_id==id).collect::<Vec<_>>()}),
            ),
            "inject" => {
                let body: InjectRequest = match decode_json(request) {
                    Ok(body) => body,
                    Err(error) => return error,
                };
                if body.from_session_id == id.0 {
                    return Response::error(400, "bad_request", "self inject is not allowed");
                }
                if body.text.trim().is_empty() {
                    return Response::error(400, "bad_request", "text is required");
                }
                let Some(session) = self.core.details(id) else {
                    return Response::error(404, "not_found", "session not found");
                };
                if let Err(error) = self.enqueue(
                    session.binding,
                    &session.snapshot.provider,
                    body.text,
                    body.press_enter,
                    body.interrupt,
                    now,
                ) {
                    return error;
                }
                Response::json(200, &serde_json::json!({"ok":true}))
            }
            "send-child" => {
                let body: SendRequest = match decode_json(request) {
                    Ok(body) => body,
                    Err(error) => return error,
                };
                // Go sanitizeRole's safeToken deliberately maps blank to item.
                let role = crate::orchestration::child_launch::safe_token(
                    &crate::proto::unicode::simple_lower(body.role.trim()),
                );
                if role.is_empty() {
                    return Response::error(400, "bad_request", "role is required");
                }
                if body.text.trim().is_empty() {
                    return Response::error(400, "bad_request", "text is required");
                }
                let Some(parent) = self.core.details(id) else {
                    return Response::error(404, "not_found", "parent session not found");
                };
                let child = self
                    .core
                    .snapshots()
                    .into_iter()
                    .filter(|child| {
                        child.parent_session_id == id
                            && child.role == role
                            && !matches!(
                                child.state.as_str(),
                                "completed"
                                    | "error"
                                    | "disconnected"
                                    | "done"
                                    | "timeout"
                                    | "dismissed"
                            )
                    })
                    .filter_map(|child| {
                        self.core
                            .details(child.id)
                            .filter(|details| details.connected)
                    })
                    .max_by_key(|details| details.binding.session);
                let Some(child) = child else {
                    return Response::error(
                        404,
                        "no_live_child",
                        &format!(
                            "no live child for role {}; use `many-ai-cli orchestrate spawn --role {role} \"<prompt>\"`",
                            crate::proto::go_quote::quote(&role)
                        ),
                    );
                };
                match self
                    .program
                    .launch_instruction_not_taken(child.binding.session)
                {
                    Ok(true) => {
                        return Response::error(
                            409,
                            "child_not_ready",
                            &format!(
                                "child role={} id={} has not taken its launch instructions yet (it is still starting or waiting on a startup screen such as folder trust); nothing was typed. The instructions it was spawned with start by themselves once the user answers that screen in the child session; send this again after the child has started working",
                                child.snapshot.role, child.binding.session.0
                            ),
                        );
                    }
                    Ok(false) => {}
                    Err(_) => {
                        return Response::error(
                            503,
                            "inject_unavailable",
                            "child input admission unavailable",
                        );
                    }
                }
                let safe = sanitize_inject_text(&body.text);
                let text = format!(
                    "\n[orchestration] instruction from conductor (session={}):\n{safe}\n",
                    id.0
                );
                // Reserve owned work before recording a command we cannot enqueue.
                let permit = match self.tasks.effect_permit() {
                    Ok(permit) => permit,
                    Err(_) => {
                        return Response::error(
                            503,
                            "inject_unavailable",
                            "session input unavailable",
                        );
                    }
                };
                if !parent.snapshot.board_path.is_empty() {
                    self.program.append_conductor_instruction(
                        Path::new(&parent.snapshot.board_path),
                        &child.snapshot,
                        &safe,
                        now,
                    );
                }
                self.start_input(
                    permit,
                    child.binding,
                    &child.snapshot.provider,
                    text,
                    true,
                    false,
                    now,
                );
                Response::json(
                    200,
                    &serde_json::json!({"ok":true,"session_id":child.binding.session.0,"role":child.snapshot.role,"board_path":parent.snapshot.board_path}),
                )
            }
            _ => Response::error(404, "not_found", "not found"),
        }
    }
    fn enqueue(
        &self,
        binding: SessionBinding,
        provider: &str,
        text: String,
        enter: bool,
        interrupt: bool,
        now: Timestamp,
    ) -> Result<(), Response> {
        let permit = self
            .tasks
            .effect_permit()
            .map_err(|_| Response::error(503, "inject_unavailable", "session input unavailable"))?;
        self.start_input(permit, binding, provider, text, enter, interrupt, now);
        Ok(())
    }
    // Retain the distinct admitted binding, payload and source input flags.
    #[allow(clippy::too_many_arguments)]
    fn start_input(
        &self,
        permit: super::task_owner::OwnedTaskPermit,
        binding: SessionBinding,
        provider: &str,
        text: String,
        enter: bool,
        interrupt: bool,
        now: Timestamp,
    ) {
        let core = self.core.clone();
        let warning = self.warning.clone();
        let shell = provider == "shell";
        let cancellation = permit.cancellation();
        drop(permit.start(async move {
            let mut frames = Vec::new();
            if interrupt {
                frames.push(vec![0x1b]);
            }
            frames.push(if !enter {
                text.into_bytes()
            } else if shell {
                let mut text = text;
                if !text.ends_with('\r') {
                    text.push('\r');
                }
                text.into_bytes()
            } else {
                format!(
                    "\x1b[200~{}\x1b[201~\r",
                    text.trim_end_matches(['\r', '\n'])
                )
                .into_bytes()
            });
            for bytes in frames {
                let receipt = core
                    .submit(
                        binding,
                        InputRequest {
                            bytes,
                            authority: InputAuthority::Internal,
                        },
                        now,
                        &cancellation,
                    )
                    .await;
                if !matches!(
                    receipt.disposition,
                    InputDisposition::TransportWritten { .. } | InputDisposition::Deferred { .. }
                ) {
                    (warning)(
                        "session inject delivery",
                        &SessionError::Transport("internal input delivery failed".into()),
                    );
                }
            }
        }));
    }
}
