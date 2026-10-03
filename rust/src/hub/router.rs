use super::{
    assets, auth,
    http::{Request, Response, random_hex},
    pin::AuthService,
    settings,
};
use crate::{
    config::{ConfigStore, RuntimePaths},
    proto::core::{CoreEffects, SessionCore},
};
use serde_json::json;
use std::sync::Arc;

pub struct ServiceRouter {
    pub config: Arc<ConfigStore>,
    pub paths: RuntimePaths,
    pub auth: AuthService,
    pub(crate) core: Option<Arc<dyn SessionCore>>,
}
/// Side effects must be driven by the actual WebSocket owner, in order, before
/// acknowledging an auth revocation. They are never dropped inside the router.
pub struct Dispatch {
    pub response: Response,
    pub effects: CoreEffects,
}
impl Dispatch {
    fn response(response: Response) -> Self {
        Self {
            response,
            effects: CoreEffects::default(),
        }
    }
}
impl ServiceRouter {
    pub fn new(config: Arc<ConfigStore>, paths: RuntimePaths, core: Arc<dyn SessionCore>) -> Self {
        Self {
            config,
            paths,
            auth: AuthService::default(),
            core: Some(core),
        }
    }
    #[cfg(test)]
    pub(crate) fn isolated(config: Arc<ConfigStore>, paths: RuntimePaths) -> Self {
        Self {
            config,
            paths,
            auth: AuthService::default(),
            core: None,
        }
    }
    pub fn handle(&self, request: &Request, now: i64) -> Dispatch {
        if assets::is_static(&request.path) {
            return Dispatch::response(assets::static_asset(request));
        }
        let snapshot = match self.config.snapshot() {
            Ok(s) => s,
            Err(_) => {
                return Dispatch::response(Response::error(
                    500,
                    "internal",
                    "configuration unavailable",
                ));
            }
        };
        let config = &snapshot.config;
        let port = i64::from(self.paths.port());
        let response = if request.path == "/"
            || (!request.path.starts_with("/api/")
                && request.path != "/ws"
                && !request.path.starts_with("/approval-patterns/"))
        {
            assets::index(request, config, port, &self.auth, now)
        } else if request.path.starts_with("/api/approval-action/") {
            // One-tap action tokens do not authenticate with the Hub token. Keep
            // this dispatcher isolated until its consuming core action is wired.
            Response::error(
                501,
                "migration_unimplemented",
                "one-tap approval service is not implemented",
            )
        } else {
            let base = matches!(
                request.path.as_str(),
                "/api/auth/status" | "/api/auth/login" | "/api/auth/logout"
            );
            let methods: &[&str] = match request.path.as_str() {
                "/api/auth/status" => &["GET"],
                "/api/auth/login"
                | "/api/auth/logout"
                | "/api/auth/set-pin"
                | "/api/auth/revoke-all"
                | "/api/notify-generate-topic" => &["POST"],
                p if settings::PATHS.contains(&p) => &["GET", "POST"],
                _ => &[],
            };
            let guard = if base {
                auth::guard_base(request, config, port, methods)
            } else {
                self.auth.guard(request, config, port, methods, now)
            };
            if let Err(error) = guard {
                return Dispatch::response(error);
            }
            match request.path.as_str() {
                "/api/auth/status" => self.auth.status(request, config, now),
                "/api/auth/login" => self.auth.login(request, config, now),
                "/api/auth/logout" => self.auth.logout(request, config),
                "/api/auth/set-pin" => self.auth.set_pin(request, &self.config, now),
                "/api/auth/revoke-all" => {
                    let Some(core) = &self.core else {
                        return Dispatch::response(Response::error(
                            503,
                            "core_unavailable",
                            "session core not initialized",
                        ));
                    };
                    match self.auth.rotate(request, &self.config, port, now) {
                        Ok(response) => {
                            return Dispatch {
                                response,
                                effects: core.invalidate_all_ui(),
                            };
                        }
                        Err(e) => e,
                    }
                }
                "/api/notify-generate-topic" => match random_hex(12) {
                    Ok(topic) => Response::json(200, &json!({"topic":format!("anyaicli-{topic}")})),
                    Err(_) => {
                        Response::error(500, "generate_failed", "generate random topic failed")
                    }
                },
                p if settings::PATHS.contains(&p) => {
                    settings::handle(request, &self.config, &self.paths)
                }
                _ => Response::error(
                    501,
                    "migration_unimplemented",
                    "service operation is not implemented",
                ),
            }
        };
        Dispatch::response(response)
    }
}
