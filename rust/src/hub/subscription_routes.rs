//! HTTP adapters for subscription registration and explicit usage operations.
//! Guard ordering and raw-output privacy are owned by the service and Hub.
use super::http::{JSON_BODY_LIMIT, Request, Response, require_method};
use crate::{
    application::subscriptions::{ServiceError, SubscriptionService},
    proto::{
        time::Timestamp,
        wire::{Field, Schema},
    },
};
use serde::Deserialize;
use std::sync::Arc;
#[derive(Default, Deserialize)]
#[serde(default)]
struct Body {
    provider: String,
    id: String,
    name: Option<String>,
    enabled: Option<bool>,
    delete_credentials: bool,
    session_id: i64,
    cwd: String,
}
pub struct SubscriptionHttp {
    service: Arc<SubscriptionService>,
}
impl SubscriptionHttp {
    pub fn new(service: Arc<SubscriptionService>) -> Self {
        Self { service }
    }
    pub fn preflight_authenticated(&self, r: &Request) -> Result<(), Response> {
        if let Some(methods) = methods(&r.path) {
            require_method(r, methods)?;
        }
        Ok(())
    }
    pub async fn handle_authenticated(&self, r: &Request, now: Timestamp) -> Option<Response> {
        methods(&r.path)?;
        if let Err(response) = self.preflight_authenticated(r) {
            return Some(response);
        }
        if r.path == "/api/subscriptions" && r.method == "GET" {
            return Some(result(self.service.list()));
        }
        if r.path == "/api/subscription-usage" {
            return Some(result(
                self.service
                    .usage_snapshot(r.query("refresh_auth") == "1", now),
            ));
        }
        let current_action = action(&r.path);
        if r.path.starts_with("/api/subscriptions/")
            && !matches!(current_action, "update" | "remove" | "test" | "login")
        {
            return Some(Response::error(404, "not_found", "not found"));
        }
        let body = match decode(r, current_action) {
            Ok(body) => body,
            Err(_) => return Some(Response::error(400, "bad_request", "invalid json")),
        };
        if r.path == "/api/subscriptions" {
            return Some(result(self.service.add(
                &body.provider,
                &body.id,
                body.name.as_deref().unwrap_or(""),
            )));
        }
        if current_action == "update" {
            return Some(result(self.service.update(
                &body.provider,
                &body.id,
                body.name.as_deref(),
                body.enabled,
            )));
        }
        if current_action == "remove" {
            return Some(result(self.service.remove(
                &body.provider,
                &body.id,
                body.delete_credentials,
            )));
        }
        let permit = match self.service.effect_permit() {
            Ok(permit) => permit,
            Err(_) => {
                return Some(Response::error(
                    503,
                    "shutdown",
                    "Hub task admission stopped",
                ));
            }
        };
        let cancel = permit.cancellation();
        let service = self.service.clone();
        let path = r.path.clone();
        let method = r.method.clone();
        let waiter = permit.start(async move {
            let response = if path == "/api/subscription-usage/refresh" {
                service
                    .refresh_codex(&body.provider, &body.id, body.session_id, &cancel)
                    .await
            } else if path == "/api/subscription-usage/probe" {
                service
                    .probe(&body.provider, &body.id, method == "DELETE", now, &cancel)
                    .await
            } else if action(&path) == "test" {
                service.test(&body.provider, &body.id, &cancel).await
            } else {
                let _ignored_cwd = body.cwd;
                service.login(&body.provider, &body.id, &cancel).await
            };
            result(response)
        });
        Some(match waiter.wait().await {
            Ok(response) => response,
            Err(_) => Response::error(503, "shutdown", "subscription request owner ended"),
        })
    }
}
fn result(result: Result<serde_json::Value, ServiceError>) -> Response {
    match result {
        Ok(value) => Response::json(200, &value),
        Err(error) => Response::error(error.status, error.code, &error.detail),
    }
}
fn action(path: &str) -> &str {
    path.strip_prefix("/api/subscriptions/")
        .unwrap_or("")
        .trim_matches('/')
}
pub fn methods(path: &str) -> Option<&'static [&'static str]> {
    match path {
        "/api/subscriptions" => Some(&["GET", "POST"]),
        "/api/subscription-usage" => Some(&["GET"]),
        "/api/subscription-usage/refresh" => Some(&["POST"]),
        "/api/subscription-usage/probe" => Some(&["POST", "DELETE"]),
        _ if path.starts_with("/api/subscriptions/") => Some(
            if matches!(action(path), "update" | "remove" | "test" | "login") {
                &["POST"]
            } else {
                &["GET", "POST"]
            },
        ),
        _ => None,
    }
}
fn decode(r: &Request, action: &str) -> Result<Body, serde_json::Error> {
    const ADD: &[Schema] = &[Schema {
        name: "SubscriptionRequest",
        fields: &[
            Field {
                name: "provider",
                kind: "string",
            },
            Field {
                name: "id",
                kind: "string",
            },
            Field {
                name: "name",
                kind: "string",
            },
        ],
    }];
    const UPDATE: &[Schema] = &[Schema {
        name: "SubscriptionRequest",
        fields: &[
            Field {
                name: "provider",
                kind: "string",
            },
            Field {
                name: "id",
                kind: "string",
            },
            Field {
                name: "name",
                kind: "*string",
            },
            Field {
                name: "enabled",
                kind: "*bool",
            },
        ],
    }];
    const REMOVE: &[Schema] = &[Schema {
        name: "SubscriptionRequest",
        fields: &[
            Field {
                name: "provider",
                kind: "string",
            },
            Field {
                name: "id",
                kind: "string",
            },
            Field {
                name: "delete_credentials",
                kind: "bool",
            },
        ],
    }];
    const LOGIN: &[Schema] = &[Schema {
        name: "SubscriptionRequest",
        fields: &[
            Field {
                name: "provider",
                kind: "string",
            },
            Field {
                name: "id",
                kind: "string",
            },
            Field {
                name: "cwd",
                kind: "string",
            },
        ],
    }];
    const ID: &[Schema] = &[Schema {
        name: "SubscriptionRequest",
        fields: &[
            Field {
                name: "provider",
                kind: "string",
            },
            Field {
                name: "id",
                kind: "string",
            },
        ],
    }];
    const REFRESH: &[Schema] = &[Schema {
        name: "SubscriptionRequest",
        fields: &[
            Field {
                name: "provider",
                kind: "string",
            },
            Field {
                name: "id",
                kind: "string",
            },
            Field {
                name: "session_id",
                kind: "int",
            },
        ],
    }];
    let schema = if r.path == "/api/subscriptions" {
        ADD
    } else if r.path == "/api/subscription-usage/refresh" {
        REFRESH
    } else {
        match action {
            "update" => UPDATE,
            "remove" => REMOVE,
            "login" => LOGIN,
            _ => ID,
        }
    };
    crate::proto::wire::decode_http_schema(
        &r.body[..r.body.len().min(JSON_BODY_LIMIT)],
        "SubscriptionRequest",
        schema,
    )
}
#[cfg(test)]
mod tests;
