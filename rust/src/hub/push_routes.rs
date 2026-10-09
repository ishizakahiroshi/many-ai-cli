//! Source push status/key and private subscription HTTP adapters.
use super::http::{Request, Response, decode_json, require_method};
use crate::{
    application::push::{Keys, PushManager, Status, Subscription},
    proto::{
        time::Timestamp,
        wire::{Field, GoWire, Schema},
    },
};
use serde::Deserialize;
use std::sync::Arc;
#[derive(Default, Deserialize)]
#[serde(default)]
struct Body {
    endpoint: String,
    keys: Keys,
}
impl GoWire for Body {
    const GO_TYPE: &'static str = "PushSubscriptionRequest";
    const SCHEMAS: &'static [Schema] = &[
        Schema {
            name: "PushSubscriptionRequest",
            fields: &[
                Field {
                    name: "endpoint",
                    kind: "string",
                },
                Field {
                    name: "keys",
                    kind: "PushKeys",
                },
            ],
        },
        Schema {
            name: "PushKeys",
            fields: &[
                Field {
                    name: "auth",
                    kind: "string",
                },
                Field {
                    name: "p256dh",
                    kind: "string",
                },
            ],
        },
    ];
}
pub struct PushHttp {
    manager: Option<Arc<PushManager>>,
}
impl PushHttp {
    pub fn new(manager: Option<Arc<PushManager>>) -> Self {
        Self { manager }
    }
    pub fn preflight_authenticated(&self, r: &Request) -> Result<(), Response> {
        if let Some(methods) = methods(&r.path) {
            require_method(r, methods)?;
            if r.path != "/api/push/status" && self.manager.is_none() {
                return Err(Response::error(503, "push_unavailable", "push unavailable"));
            }
        }
        Ok(())
    }
    pub fn handle_authenticated(&self, r: &Request, now: Timestamp) -> Option<Response> {
        methods(&r.path)?;
        if let Err(response) = self.preflight_authenticated(r) {
            return Some(response);
        }
        if r.path == "/api/push/status" {
            return Some(Response::json(
                200,
                &self
                    .manager
                    .as_ref()
                    .map(|manager| manager.status())
                    .unwrap_or(Status {
                        supported: false,
                        public_key: String::new(),
                        subscriptions: 0,
                    }),
            ));
        }
        let Some(manager) = &self.manager else {
            return Some(Response::error(503, "push_unavailable", "push unavailable"));
        };
        if r.path == "/api/push/vapid-public-key" {
            return Some(Response::json(
                200,
                &serde_json::json!({"public_key":manager.public_key()}),
            ));
        }
        let body = match decode_json::<Body>(r) {
            Ok(body) => body,
            Err(response) => return Some(response),
        };
        let result = if r.method == "DELETE" {
            manager.delete(&body.endpoint)
        } else {
            manager.upsert(
                Subscription {
                    endpoint: body.endpoint,
                    keys: body.keys,
                    user_agent: r.header("user-agent").trim().into(),
                    ..Default::default()
                },
                now,
            )
        };
        Some(match result {
            Ok(()) => Response::json(200, &serde_json::json!({"ok":true})),
            Err(error) => Response::error(400, "bad_subscription", &error.to_string()),
        })
    }
}
pub fn methods(path: &str) -> Option<&'static [&'static str]> {
    match path {
        "/api/push/status" | "/api/push/vapid-public-key" => Some(&["GET"]),
        "/api/push/subscriptions" => Some(&["POST", "DELETE"]),
        _ => None,
    }
}
#[cfg(test)]
mod tests;
