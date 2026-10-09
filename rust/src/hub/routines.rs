//! Routine HTTP operations. The caller applies the ordinary Hub token/PIN,
//! method, Host and Origin guard before invoking this authenticated boundary.
use super::http::{Request, Response, decode_json, require_method};
use crate::proto::time::Timestamp;
use crate::{
    proto::wire::{Field, GoWire, Schema},
    routine::{
        model::Definition,
        runner::RoutineRunner,
        store::{Error, RoutineStore},
    },
};
use serde::Deserialize;
use serde_json::json;
use std::{path::PathBuf, sync::Arc};
pub struct RoutineHttp {
    pub store: Arc<RoutineStore>,
    pub runner: Option<Arc<RoutineRunner>>,
    pub home: PathBuf,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct StartRequest {
    request_id: String,
}
impl GoWire for StartRequest {
    const GO_TYPE: &'static str = "RoutineStartRequest";
    const SCHEMAS: &'static [Schema] = &[Schema {
        name: "RoutineStartRequest",
        fields: &[Field {
            name: "request_id",
            kind: "string",
        }],
    }];
}
impl RoutineHttp {
    pub fn handle_authenticated(&self, request: &Request, now: Timestamp) -> Option<Response> {
        if request.path == "/api/routine-runs" || request.path.starts_with("/api/routine-runs/") {
            return Some(self.runs(request));
        }
        if request.path != "/api/routines" && !request.path.starts_with("/api/routines/") {
            return None;
        }
        Some(self.routines(request, now))
    }
    fn routines(&self, request: &Request, now: Timestamp) -> Response {
        if let Err(error) = require_method(request, &["GET", "POST", "PUT", "DELETE"]) {
            return error;
        }
        if !self.store.ready() {
            return error_response(Error::Unavailable);
        }
        let path = request
            .path
            .strip_prefix("/api/routines")
            .expect("checked routine route")
            .trim_matches('/');
        let parts: Vec<_> = path.split('/').collect();
        if parts.len() == 2 && parts[1] == "runs" && request.method == "POST" {
            let body: StartRequest = match decode_json(request) {
                Ok(body) => body,
                Err(error) => return error,
            };
            if body.request_id.trim().is_empty() || body.request_id.len() > 128 {
                return Response::error(
                    400,
                    "bad_request",
                    "request_id is required (max 128 bytes)",
                );
            }
            let Some(runner) = &self.runner else {
                return error_response(Error::LaunchUnavailable);
            };
            return match runner.start(parts[0], &body.request_id, "manual", now) {
                Ok(admission) => Response::json(
                    200,
                    &json!({"run":admission.run,"existing":admission.existing}),
                ),
                Err(error) => error_response(error),
            };
        }
        if parts.len() > 1 {
            return Response::error(404, "not_found", "routine route not found");
        }
        if request.method == "GET" {
            let list = match self.store.definitions() {
                Ok(list) => list,
                Err(error) => return error_response(error),
            };
            if path.is_empty() {
                return Response::json(200, &json!({"routines":list}));
            }
            return match list.into_iter().find(|item| item.id == path) {
                Some(item) => Response::json(200, &json!({"routine":item})),
                None => error_response(Error::Missing),
            };
        }
        if (request.method == "POST" && !path.is_empty())
            || (request.method != "POST" && path.is_empty())
        {
            return Response::error(405, "method_not_allowed", "unsupported routine operation");
        }
        if request.method == "DELETE" {
            return match self.store.delete(path) {
                Ok(()) => Response::json(200, &json!({"ok":true})),
                Err(error) => error_response(error),
            };
        }
        let item: Definition = match decode_json(request) {
            Ok(item) => item,
            Err(error) => return error,
        };
        match self.store.save(
            if request.method == "POST" {
                None
            } else {
                Some(path)
            },
            item,
            &self.home,
            now,
        ) {
            Ok(item) => Response::json(200, &json!({"routine":item})),
            Err(error) => error_response(error),
        }
    }
    fn runs(&self, request: &Request) -> Response {
        if let Err(error) = require_method(request, &["GET"]) {
            return error;
        }
        if !self.store.ready() {
            return error_response(Error::Unavailable);
        }
        let id = request
            .path
            .strip_prefix("/api/routine-runs")
            .expect("checked run route")
            .trim_matches('/');
        if !id.is_empty() {
            return match self.store.run(id) {
                Ok(Some(run)) => Response::json(200, &json!({"run":run})),
                Ok(None) => Response::error(404, "not_found", "routine run not found"),
                Err(error) => error_response(error),
            };
        }
        match self.store.runs(&request.query("routine_id")) {
            Ok(runs) => Response::json(200, &json!({"runs":runs})),
            Err(error) => error_response(error),
        }
    }
}
fn error_response(error: Error) -> Response {
    match error {
        Error::Unavailable => Response::error(
            503,
            "routine_store_unavailable",
            "routine storage is unavailable",
        ),
        Error::LaunchUnavailable => Response::error(
            503,
            "routine_launcher_unavailable",
            "routine session launcher is unavailable",
        ),
        Error::Missing => Response::error(404, "not_found", "routine not found"),
        Error::Changed => Response::error(
            409,
            "routine_changed",
            "routine changed; reload before saving",
        ),
        Error::Running => Response::error(
            409,
            "routine_running",
            "wait for the current run before deleting this routine",
        ),
        Error::Limit => Response::error(
            409,
            "routine_limit",
            "maximum of 200 saved routines reached",
        ),
        Error::Invalid(detail) => Response::error(400, "bad_request", &detail),
        _ => Response::error(
            500,
            "routine_operation_failed",
            "routine operation could not be saved or started",
        ),
    }
}

#[cfg(test)]
#[path = "../routine/http_tests.rs"]
mod tests;
