//! `/api/acp/...`: the routes of ACP sessions (`application::acp_sessions`).
//!
//! Every route needs the dashboard's proof (`VerifiedUiOrigin`: the signed UI
//! cookie, Fetch Metadata and Origin). A caller that holds only the Hub token
//! (a child session, `orchestrate`, a routine, a board item) gets 403 here, so
//! the "person at the screen" boundary is held by the server, not by hiding a
//! button. The router has already applied the method/Host/Origin/PIN guards.
use super::http::{Request, Response};
use crate::{
    acp::PermissionDecision,
    application::acp_sessions::{AcpSessionError, AcpSessions, MAX_WAIT},
    proto::core::VerifiedUiOrigin,
};
use serde::Deserialize;
use serde_json::json;
use std::{sync::Arc, time::Duration};

pub const PREFIX: &str = "/api/acp/";

/// The shape of a path under `/api/acp/`.
#[derive(Debug, PartialEq, Eq)]
enum Route {
    Sessions,
    Session(u64, &'static str),
}

fn parse(path: &str) -> Option<Route> {
    let rest = path.strip_prefix(PREFIX)?.trim_end_matches('/');
    if rest == "sessions" {
        return Some(Route::Sessions);
    }
    let rest = rest.strip_prefix("sessions/")?;
    let (id, action) = rest.split_once('/')?;
    let id: u64 = id.parse().ok()?;
    let action = match action {
        "events" => "events",
        "prompt" => "prompt",
        "permission" => "permission",
        "cancel" => "cancel",
        "end" => "end",
        "agent-sessions" => "agent-sessions",
        _ => return None,
    };
    Some(Route::Session(id, action))
}

pub fn is_route(path: &str) -> bool {
    parse(path).is_some()
}

pub fn methods(path: &str) -> Option<&'static [&'static str]> {
    Some(match parse(path)? {
        Route::Sessions => &["GET", "POST"],
        Route::Session(_, "events" | "agent-sessions") => &["GET"],
        Route::Session(..) => &["POST"],
    })
}

#[derive(Deserialize)]
#[serde(default)]
struct OpenBody {
    provider: String,
    cwd: String,
    resume_agent_session_id: Option<String>,
}
impl Default for OpenBody {
    fn default() -> Self {
        Self {
            provider: String::new(),
            cwd: String::new(),
            resume_agent_session_id: None,
        }
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PromptBody {
    text: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PermissionBody {
    request_key: u64,
    option_id: Option<String>,
    cancelled: bool,
}

pub struct AcpHttp {
    sessions: AcpSessions,
}

fn refused(error: AcpSessionError) -> Response {
    let text = error.to_string();
    match error {
        AcpSessionError::NotAcp(_) => Response::error(400, "not_acp_provider", &text),
        AcpSessionError::BadRequest(_) => Response::error(400, "bad_request", &text),
        AcpSessionError::NotFound => Response::error(404, "not_found", &text),
        AcpSessionError::Busy => Response::error(409, "turn_running", &text),
        AcpSessionError::Ended(_) => Response::error(409, "session_ended", &text),
        AcpSessionError::AuthRequired { methods, .. } => Response::json(
            409,
            &json!({
                "ok": false,
                "error": "auth_required",
                "detail": text,
                "auth_methods": methods.iter().map(|m| json!({"id": m.id, "name": m.name, "kind": m.kind})).collect::<Vec<_>>(),
            }),
        ),
        AcpSessionError::Unsupported(_) => Response::error(409, "unsupported", &text),
        AcpSessionError::Launch(_) => Response::error(500, "agent_launch_failed", &text),
        AcpSessionError::Agent(_) => Response::error(502, "agent_error", &text),
    }
}

fn body<T: for<'de> Deserialize<'de> + Default>(request: &Request) -> Result<T, Response> {
    if request.body.is_empty() {
        return Ok(T::default());
    }
    serde_json::from_slice(&request.body)
        .map_err(|_| Response::error(400, "bad_request", "the request body is not valid JSON"))
}

impl AcpHttp {
    pub fn new(sessions: AcpSessions) -> Arc<Self> {
        Arc::new(Self { sessions })
    }

    /// `human` is `Some` only when the router verified the dashboard's cookie.
    pub async fn handle_authenticated(
        &self,
        request: &Request,
        human: Option<VerifiedUiOrigin>,
    ) -> Response {
        let Some(route) = parse(&request.path) else {
            return Response::error(404, "not_found", "not found");
        };
        if !methods(&request.path).is_some_and(|m| m.contains(&request.method.as_str())) {
            return Response::error(405, "method_not_allowed", "method not allowed");
        }
        let Some(human) = human else {
            return Response::error(
                403,
                "ui_required",
                "ACP sessions are driven from the signed-in dashboard only",
            );
        };
        match route {
            Route::Sessions if request.method == "GET" => Response::json(
                200,
                &json!({"sessions": self.sessions.list(), "saved": self.sessions.saved()}),
            ),
            Route::Sessions => {
                let open: OpenBody = match body(request) {
                    Ok(open) => open,
                    Err(response) => return response,
                };
                match self
                    .sessions
                    .open(
                        &human,
                        &open.provider,
                        &open.cwd,
                        open.resume_agent_session_id.as_deref(),
                    )
                    .await
                {
                    Ok(view) => Response::json(200, &view),
                    Err(error) => refused(error),
                }
            }
            Route::Session(id, "events") => {
                let after = request.query("after").parse().unwrap_or(0);
                let wait = request
                    .query("wait_ms")
                    .parse::<u64>()
                    .map(Duration::from_millis)
                    .unwrap_or_default()
                    .min(MAX_WAIT);
                match self.sessions.events(&human, id, after, wait).await {
                    Ok(view) => Response::json(200, &view).no_store(),
                    Err(error) => refused(error),
                }
            }
            Route::Session(id, "prompt") => {
                let prompt: PromptBody = match body(request) {
                    Ok(prompt) => prompt,
                    Err(response) => return response,
                };
                match self.sessions.prompt(&human, id, &prompt.text).await {
                    Ok(view) => Response::json(202, &view),
                    Err(error) => refused(error),
                }
            }
            Route::Session(id, "permission") => {
                let answer: PermissionBody = match body(request) {
                    Ok(answer) => answer,
                    Err(response) => return response,
                };
                let decision = match (answer.option_id, answer.cancelled) {
                    (Some(option), false) => PermissionDecision::Select(option),
                    (None, true) => PermissionDecision::Cancelled,
                    _ => {
                        return Response::error(
                            400,
                            "bad_request",
                            "give either option_id or cancelled",
                        );
                    }
                };
                match self
                    .sessions
                    .answer_permission(&human, id, answer.request_key, decision)
                    .await
                {
                    Ok(()) => Response::json(200, &json!({"ok": true})),
                    Err(error) => refused(error),
                }
            }
            Route::Session(id, "cancel") => match self.sessions.cancel(&human, id).await {
                Ok(()) => Response::json(200, &json!({"ok": true})),
                Err(error) => refused(error),
            },
            Route::Session(id, "end") => match self.sessions.end(&human, id).await {
                Ok(view) => Response::json(200, &view),
                Err(error) => refused(error),
            },
            Route::Session(id, _) => {
                let cursor = request.query("cursor");
                let cursor = (!cursor.is_empty()).then_some(cursor);
                match self
                    .sessions
                    .agent_sessions(&human, id, cursor.as_deref())
                    .await
                {
                    Ok(list) => Response::json(200, &list),
                    Err(error) => refused(error),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
