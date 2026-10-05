//! Hub guard precedes this adapter; source endpoint bodies are ignored.
use super::http::{Request, Response, require_method};
use crate::{application::mobile_connect::MobileConnect, process::Cancellation};
use std::sync::Arc;
#[cfg(test)]
mod tests;
pub fn methods(path: &str) -> Option<&'static [&'static str]> {
    match path {
        "/api/mobile-connect" | "/api/mobile-connect/tailscale" => Some(&["GET"]),
        "/api/mobile-connect/tailscale/serve" => Some(&["POST", "DELETE"]),
        _ => None,
    }
}
pub struct MobileHttp {
    owner: Arc<MobileConnect>,
}
impl MobileHttp {
    pub fn new(owner: Arc<MobileConnect>) -> Self {
        Self { owner }
    }
    pub async fn handle_authenticated(&self, request: &Request, cancel: &Cancellation) -> Response {
        let Some(allowed) = methods(&request.path) else {
            return Response::error(404, "not_found", "not found");
        };
        if let Err(response) = require_method(request, allowed) {
            return response;
        }
        let result = match request.path.as_str() {
            "/api/mobile-connect" => self.owner.metadata(cancel).await,
            "/api/mobile-connect/tailscale" => self.owner.status(cancel).await,
            _ if request.method == "POST" => self.owner.enable(cancel).await,
            _ => Ok(self.owner.disable(cancel).await),
        };
        match result {
            Ok(result) => {
                let mut response = Response::json(200, &result.value);
                if result.no_store {
                    response
                        .headers
                        .insert("Cache-Control".into(), "no-store".into());
                }
                response
            }
            Err(_) => Response::error(500, "config_unavailable", "configuration unavailable"),
        }
    }
}
