use super::http::{Request, Response, require_method};
use crate::application::provider_assets::{ICON_LIMIT, ProviderAssets, image_type, valid_icon_id};
use std::sync::Arc;
pub struct IconHttp {
    owner: Arc<ProviderAssets>,
}
#[cfg(test)]
#[path = "icon_routes/tests.rs"]
mod tests;
impl IconHttp {
    pub fn new(owner: Arc<ProviderAssets>) -> Arc<Self> {
        Arc::new(Self { owner })
    }
    pub fn methods(path: &str) -> Option<&'static [&'static str]> {
        path.starts_with("/api/provider-icons/")
            .then_some(&["GET", "PUT", "DELETE"])
    }
    pub fn handle_authenticated(&self, request: &Request) -> Option<Response> {
        let methods = Self::methods(&request.path)?;
        if let Err(e) = require_method(request, methods) {
            return Some(e);
        }
        let id = request
            .path
            .strip_prefix("/api/provider-icons/")
            .expect("matched route");
        if !valid_icon_id(id) {
            return Some(Response::error(
                404,
                "provider_icon_not_found",
                "provider icon was not found",
            ));
        }
        let snapshot = match self.owner.registry.snapshot() {
            Ok(s) => s,
            Err(_) => {
                return Some(Response::error(
                    503,
                    "provider_registry_unavailable",
                    "provider registry is unavailable",
                ));
            }
        };
        if snapshot.registry.lookup(id).is_none() {
            return Some(Response::error(
                404,
                "provider_not_found",
                "provider was not found",
            ));
        }
        Some(match request.method.as_str() {
            "GET" => match self.owner.read(id) {
                Ok(bytes) => {
                    let mut response = Response::bytes(
                        200,
                        image_type(&bytes).unwrap_or("application/octet-stream"),
                        bytes,
                    );
                    response
                        .headers
                        .insert("Cache-Control".into(), "max-age=3600".into());
                    response
                        .headers
                        .insert("X-Content-Type-Options".into(), "nosniff".into());
                    response
                }
                Err(_) => Response::bytes(
                    404,
                    "text/plain; charset=utf-8",
                    b"404 page not found\n".to_vec(),
                ),
            },
            "PUT" => {
                if request.body.len() > ICON_LIMIT {
                    Response::error(
                        413,
                        "provider_icon_too_large",
                        "image must be 512 KB or smaller",
                    )
                } else if image_type(&request.body).is_none() {
                    Response::error(
                        415,
                        "provider_icon_bad_type",
                        "image/png, image/jpeg, image/gif, or image/webp required",
                    )
                } else {
                    match self.owner.write(id, &request.body) {
                        Ok(()) => Response::json(200, &serde_json::json!({"ok":true})),
                        Err(_) => Response::error(500, "write_error", "could not save the image"),
                    }
                }
            }
            "DELETE" => {
                self.owner.remove(id);
                Response::json(200, &serde_json::json!({"ok":true}))
            }
            _ => unreachable!(),
        })
    }
    pub fn preflight_authenticated(&self, request: &Request) -> Result<(), Response> {
        if Self::methods(&request.path).is_none() {
            return Ok(());
        }
        let id = request
            .path
            .strip_prefix("/api/provider-icons/")
            .expect("matched route");
        if !valid_icon_id(id) {
            return Err(Response::error(
                404,
                "provider_icon_not_found",
                "provider icon was not found",
            ));
        }
        let snapshot = self.owner.registry.snapshot().map_err(|_| {
            Response::error(
                503,
                "provider_registry_unavailable",
                "provider registry is unavailable",
            )
        })?;
        if snapshot.registry.lookup(id).is_none() {
            return Err(Response::error(
                404,
                "provider_not_found",
                "provider was not found",
            ));
        }
        Ok(())
    }
}
