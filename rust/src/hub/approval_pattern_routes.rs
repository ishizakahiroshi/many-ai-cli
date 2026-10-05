use super::http::{Request, Response, decode_json, require_method};
use crate::{
    application::approval_patterns::{self, ApprovalPatterns, PatternList},
    config::ConfigStore,
    proto::wire::{Field, GoWire, Schema},
};
use std::sync::Arc;
pub struct ApprovalPatternHttp {
    owner: Arc<ApprovalPatterns>,
    config: Arc<ConfigStore>,
}
#[derive(Default, serde::Deserialize)]
#[serde(default)]
struct ProfileRequest {
    provider: String,
    profile: String,
}
#[derive(Default, serde::Deserialize)]
#[serde(default)]
struct ProviderRequest {
    provider: String,
}
impl GoWire for ProviderRequest {
    const GO_TYPE: &'static str = "ApprovalPatternProviderRequest";
    const SCHEMAS: &'static [Schema] = &[Schema {
        name: "ApprovalPatternProviderRequest",
        fields: &[Field {
            name: "provider",
            kind: "string",
        }],
    }];
}
impl GoWire for ProfileRequest {
    const GO_TYPE: &'static str = "ApprovalProfileRequest";
    const SCHEMAS: &'static [Schema] = &[Schema {
        name: "ApprovalProfileRequest",
        fields: &[
            Field {
                name: "provider",
                kind: "string",
            },
            Field {
                name: "profile",
                kind: "string",
            },
        ],
    }];
}
pub fn is_route(path: &str) -> bool {
    path == "/api/approval-patterns"
        || path.starts_with("/api/approval-patterns/")
        || path.starts_with("/approval-patterns/")
}
pub fn methods(path: &str) -> &'static [&'static str] {
    match path {
        "/api/approval-patterns" => &["GET"],
        "/api/approval-patterns/profile" => &["GET", "POST"],
        "/api/approval-patterns/copy-official" => &["POST"],
        p if p.starts_with("/approval-patterns/") => &["GET"],
        _ => &[], // source item dispatch validates provider before PUT
    }
}
impl ApprovalPatternHttp {
    pub fn new(owner: Arc<ApprovalPatterns>, config: Arc<ConfigStore>) -> Arc<Self> {
        Arc::new(Self { owner, config })
    }
    pub fn handle_authenticated(&self, request: &Request) -> Response {
        if let Err(response) = require_method(request, methods(&request.path)) {
            return response;
        }
        match request.path.as_str() {
            "/api/approval-patterns" => match self.owner.active() {
                Ok(list) => Response::json(200, &list),
                Err(_) => Response::error(500, "read_failed", "approval patterns unavailable"),
            },
            "/api/approval-patterns/profile" if request.method == "GET" => {
                match self.config.snapshot() {
                    Ok(s) => Response::json(200, &s.config.approval_profiles.effective()),
                    Err(_) => Response::error(500, "read_failed", "configuration unavailable"),
                }
            }
            "/api/approval-patterns/profile" => {
                let body = match decode_json::<ProfileRequest>(request) {
                    Ok(v) => v,
                    Err(r) => return r,
                };
                if !approval_patterns::known(&body.provider) {
                    return Response::error(400, "bad_request", "unknown provider");
                }
                if !matches!(body.profile.as_str(), "official" | "custom") {
                    return Response::error(400, "bad_request", "invalid profile");
                }
                loop {
                    let snapshot = match self.config.snapshot() {
                        Ok(s) => s,
                        Err(_) => {
                            return Response::error(
                                500,
                                "read_failed",
                                "configuration unavailable",
                            );
                        }
                    };
                    let mut cfg = snapshot.config;
                    cfg.approval_profiles = cfg
                        .approval_profiles
                        .effective()
                        .with_provider(&body.provider, &body.profile);
                    match self
                        .config
                        .publish_then_persist_legacy(snapshot.revision, cfg)
                    {
                        Err(crate::config::ConfigError::Conflict { .. }) => continue,
                        _ => break, // source retains publication and returns204 on save failure
                    }
                }
                self.owner.refresh();
                no_content()
            }
            "/api/approval-patterns/copy-official" => {
                let body = match decode_json::<ProviderRequest>(request) {
                    Ok(v) => v,
                    Err(r) => return r,
                };
                if !approval_patterns::known(&body.provider) {
                    return Response::error(400, "bad_request", "unknown provider");
                }
                match self.owner.copy_official(&body.provider) {
                    Ok(()) => no_content(),
                    Err(_) => Response::error(500, "copy_failed", "approval patterns copy failed"),
                }
            }
            p if p.starts_with("/approval-patterns/") => match self.owner.asset(&p[19..]) {
                Ok(bytes) => {
                    let mut r = super::assets::file_table_response(request, &[(&p[1..], &bytes)]);
                    r.headers
                        .insert("X-Content-Type-Options".into(), "nosniff".into());
                    r
                }
                Err(_) => Response::error(404, "not_found", "not found"),
            },
            _ => {
                let provider = request
                    .path
                    .strip_prefix("/api/approval-patterns/")
                    .unwrap_or("")
                    .split('/')
                    .next()
                    .unwrap_or("");
                if !approval_patterns::known(provider) {
                    return Response::error(404, "not_found", "unknown provider");
                }
                if let Err(r) = require_method(request, &["PUT"]) {
                    return r;
                }
                let list = match decode_json::<PatternList>(request) {
                    Ok(v) => v.0.unwrap_or_default(),
                    Err(r) => return r,
                };
                let list = list
                    .into_iter()
                    .map(|v| v.trim().to_owned())
                    .filter(|v| !v.is_empty())
                    .collect::<Vec<_>>();
                match self.owner.write_custom(provider, &list) {
                    Ok(()) => no_content(),
                    Err(_) => {
                        Response::error(500, "write_failed", "approval patterns write failed")
                    }
                }
            }
        }
    }
}
fn no_content() -> Response {
    Response::bytes(204, "text/plain; charset=utf-8", vec![])
}
