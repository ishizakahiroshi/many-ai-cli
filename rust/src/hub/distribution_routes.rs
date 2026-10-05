//! Official signing keys are deliberately empty in the fixed Go source.
use super::http::{Request, Response, require_method};
use crate::{
    config::RuntimePaths,
    profile::store::{DistributionStore, ProviderRegistryStore, diff_distribution},
};
use std::sync::Arc;
pub struct DistributionHttp {
    store: DistributionStore,
    registry: Arc<ProviderRegistryStore>,
}
#[cfg(test)]
#[path = "distribution_routes/tests.rs"]
mod tests;
impl DistributionHttp {
    pub fn new(paths: &RuntimePaths, registry: Arc<ProviderRegistryStore>) -> Arc<Self> {
        Arc::new(Self {
            store: DistributionStore::new(paths),
            registry,
        })
    }
    pub fn methods(path: &str) -> Option<&'static [&'static str]> {
        match path.strip_prefix("/api/provider-distributions/")? {
            "status" | "diff" => Some(&["GET"]),
            "check" | "accept" | "rollback" => Some(&["POST"]),
            _ => Some(&[]),
        }
    }
    pub fn handle_authenticated(&self, request: &Request) -> Option<Response> {
        let methods = Self::methods(&request.path)?;
        if let Err(e) = require_method(request, methods) {
            return Some(e);
        }
        Some(
            match request.path.strip_prefix("/api/provider-distributions/")? {
                "status" => match self.store.status() {
                    Ok(status) => Response::json(
                        200,
                        &serde_json::json!({"state":status.state,"enabled":false,"status":status}),
                    ),
                    Err(e) => Response::error(500, "distribution_status_failed", &e.to_string()),
                },
                "check" | "accept" => Response::error(
                    503,
                    "distribution_keys_unconfigured",
                    "official catalog signing keys are not configured",
                ),
                "diff" => self.diff(request),
                "rollback" => self.rollback(),
                _ => Response::error(404, "not_found", "provider distribution endpoint not found"),
            },
        )
    }
    fn diff(&self, request: &Request) -> Response {
        let digest = request.query("digest");
        let digest = digest.trim();
        if digest.is_empty() {
            return Response::error(400, "digest_required", "digest is required");
        }
        let candidate = match self.store.load_downloaded(digest) {
            Ok(b) => b,
            Err(e) => {
                return Response::error(404, "distribution_candidate_not_found", &e.to_string());
            }
        };
        let current = match self.store.load_accepted() {
            Ok(b) => b,
            Err(e) => {
                return Response::error(500, "distribution_accepted_load_failed", &e.to_string());
            }
        };
        let overrides = match self.registry.history.load_overrides() {
            Ok(o) => o,
            Err(e) => return Response::error(500, "override_load_failed", &e.to_string()),
        };
        Response::json(
            200,
            &serde_json::json!({"ok":true,"diff":diff_distribution(current.as_ref().and_then(|b|b.payload.definitions.as_deref()).unwrap_or_default(),candidate.payload.definitions.as_deref().unwrap_or_default(),&overrides.definitions)}),
        )
    }
    fn rollback(&self) -> Response {
        let Ok(_gate) = self.registry.mutation_gate.lock() else {
            return Response::error(
                503,
                "provider_registry_unavailable",
                "provider registry is unavailable",
            );
        };
        let snapshot = match self.store.snapshot_pointers() {
            Ok(v) => v,
            Err(e) => return Response::error(500, "distribution_snapshot_failed", &e.to_string()),
        };
        let status = match self.store.rollback() {
            Ok(v) => v,
            Err(e) => return Response::error(422, "distribution_rollback_failed", &e.to_string()),
        };
        match self.registry.reload() {
            Ok(loaded) => Response::json(
                200,
                &serde_json::json!({"ok":true,"status":status,"diagnostics":if loaded.diagnostics.is_empty(){None}else{Some(loaded.diagnostics)}}),
            ),
            Err(e) => {
                if let Err(restore) = self.store.restore_pointers(snapshot) {
                    return Response::error(
                        500,
                        "provider_reload_restore_failed",
                        &format!("{e}; restore distribution pointers: {restore}"),
                    );
                }
                let _ = self.registry.reload();
                Response::error(
                    500,
                    "provider_reload_failed",
                    &format!("{e}; distribution rollback was reverted"),
                )
            }
        }
    }
}
