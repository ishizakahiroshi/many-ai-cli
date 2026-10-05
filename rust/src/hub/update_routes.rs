//! Authenticated explicit CLI updater endpoints.
use super::http::{Request, Response, decode_json, require_method};
use crate::{
    application::cli_updates::CliUpdates,
    proto::{
        time::Timestamp,
        wire::{Field, GoWire, Schema},
    },
};
use serde::Deserialize;
use std::sync::Arc;
pub fn methods(path: &str) -> Option<&'static [&'static str]> {
    match path {
        "/api/cli-updates" => Some(&["POST"]),
        "/api/cli-update-eligibility" => Some(&["GET"]),
        p if p.starts_with("/api/cli-updates/") => Some(&["GET"]),
        _ => None,
    }
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Create {
    providers: Vec<String>,
    serial: bool,
}
impl GoWire for Create {
    const GO_TYPE: &'static str = "CLIUpdateCreateRequest";
    const SCHEMAS: &'static [Schema] = &[Schema {
        name: "CLIUpdateCreateRequest",
        fields: &[
            Field {
                name: "providers",
                kind: "[]string",
            },
            Field {
                name: "serial",
                kind: "bool",
            },
        ],
    }];
}
pub struct UpdateHttp {
    owner: Arc<CliUpdates>,
}
impl UpdateHttp {
    pub fn new(owner: Arc<CliUpdates>) -> Self {
        Self { owner }
    }
    pub fn handle_authenticated(&self, request: &Request, at: Timestamp) -> Option<Response> {
        let methods = methods(&request.path)?;
        if let Err(error) = require_method(request, methods) {
            return Some(error);
        }
        let result = match request.path.as_str() {
            "/api/cli-updates" => match decode_json::<Create>(request) {
                Ok(body) => self
                    .owner
                    .create(body.providers, body.serial, at)
                    .map(|v| Response::json(200, &v)),
                Err(error) => Err(error),
            },
            "/api/cli-update-eligibility" => self.owner.eligibility().map(|results| {
                Response::json(200, &serde_json::json!({"results":results})).no_store()
            }),
            path => {
                let parts: Vec<_> = path
                    .trim_start_matches("/api/cli-updates/")
                    .split('/')
                    .collect();
                match parts.as_slice() {
                    [id] if !id.is_empty() => self
                        .owner
                        .job(id)
                        .map(|j| Response::json(200, &j.snapshot()).no_store())
                        .ok_or_else(|| Response::error(404, "not_found", "unknown job")),
                    [id, provider, "log"] if !id.is_empty() && !provider.is_empty() => self
                        .owner
                        .log(id, provider)
                        .map(|v| Response::json(200, &v).no_store()),
                    _ => Err(Response::error(404, "not_found", "not found")),
                }
            }
        };
        Some(result.unwrap_or_else(|e| e))
    }
}
