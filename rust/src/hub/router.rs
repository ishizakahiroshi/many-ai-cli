use super::{
    assets, auth,
    http::{Request, Response, random_hex},
    pin::AuthService,
    settings,
};
use crate::{
    config::{ConfigStore, RuntimePaths},
    proto::core::{CoreEffects, LiveSessionId, SessionCore, SessionStorage},
};
use serde_json::json;
use std::sync::Arc;

pub struct ServiceRouter {
    pub config: Arc<ConfigStore>,
    pub paths: RuntimePaths,
    pub auth: AuthService,
    pub(crate) core: Option<Arc<dyn SessionCore>>,
    bound_port: u16,
    memos: Option<Arc<crate::routine::memo::MemoManager>>,
    files: Option<Arc<crate::files::FilesService>>,
    storage: Option<Arc<dyn SessionStorage>>,
    workspace: Option<Arc<dyn crate::files::WorkspaceReads>>,
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
    /// Header-only authentication before the transport reads any request body.
    /// Handler entry repeats this check against a fresh config snapshot so a
    /// concurrent revocation cannot authorize a request that was still reading.
    pub fn preflight(&self, request: &Request, now: i64) -> Result<(), Response> {
        if assets::is_static(&request.path)
            || request.path == "/"
            || (!request.path.starts_with("/api/")
                && request.path != "/ws"
                && !request.path.starts_with("/approval-patterns/"))
        {
            return Ok(());
        }
        let snapshot = self
            .config
            .snapshot()
            .map_err(|_| Response::error(500, "internal", "configuration unavailable"))?;
        let config = &snapshot.config;
        let port = i64::from(self.bound_port);
        if request.path.starts_with("/api/approval-action/") {
            super::http::require_method(request, &["POST"])?;
            auth::require_host(request, config, port)?;
            return auth::require_origin(request, config, port);
        }
        let base = matches!(
            request.path.as_str(),
            "/api/auth/status" | "/api/auth/login" | "/api/auth/logout"
        );
        let methods = guard_methods(&request.path);
        if base {
            auth::guard_base(request, config, port, methods)
        } else {
            self.auth.guard(request, config, port, methods, now)
        }
    }
    pub fn new(
        config: Arc<ConfigStore>,
        paths: RuntimePaths,
        core: Arc<dyn SessionCore>,
        bound_port: u16,
    ) -> std::io::Result<Self> {
        validate_bound_port(&paths, bound_port)?;
        Ok(Self {
            config,
            paths,
            auth: AuthService::default(),
            core: Some(core),
            bound_port,
            memos: None,
            files: None,
            storage: None,
            workspace: None,
        })
    }
    pub fn bound_port(&self) -> u16 {
        self.bound_port
    }
    #[cfg(test)]
    pub(crate) fn isolated(config: Arc<ConfigStore>, paths: RuntimePaths) -> Self {
        let bound_port = paths.port();
        Self::isolated_at_port(config, paths, bound_port).expect("valid isolated port")
    }
    #[cfg(test)]
    pub(crate) fn isolated_at_port(
        config: Arc<ConfigStore>,
        paths: RuntimePaths,
        bound_port: u16,
    ) -> std::io::Result<Self> {
        validate_bound_port(&paths, bound_port)?;
        Ok(Self {
            config,
            paths,
            auth: AuthService::default(),
            core: None,
            bound_port,
            memos: None,
            files: None,
            storage: None,
            workspace: None,
        })
    }
    pub fn with_memos(mut self, memos: Arc<crate::routine::memo::MemoManager>) -> Self {
        self.memos = Some(memos);
        self
    }
    pub fn with_files(
        mut self,
        files: Arc<crate::files::FilesService>,
        storage: Option<Arc<dyn SessionStorage>>,
        workspace: Arc<dyn crate::files::WorkspaceReads>,
    ) -> Self {
        self.files = Some(files);
        self.storage = storage;
        self.workspace = Some(workspace);
        self
    }
    /// Called only while the WebSocket owner holds core's accepted UI guard.
    pub(crate) fn record_ws_attachment(
        &self,
        message: &crate::proto::Message,
    ) -> Result<(), crate::proto::core::SessionError> {
        let (Some(files), Some(core)) = (&self.files, &self.core) else {
            return Err(crate::proto::core::SessionError::InvalidRequest(
                "attachment service is not initialized".into(),
            ));
        };
        files
            .record_ws_attachment(message, core.as_ref(), self.storage.as_deref())
            .map_err(|_| {
                crate::proto::core::SessionError::Storage(crate::proto::core::StorageError {
                    kind: crate::proto::core::StorageErrorKind::Write,
                    detail: "attachment operation failed".into(),
                })
            })
    }
    pub async fn handle_async(&self, request: &Request, now: i64) -> Dispatch {
        self.handle_async_at(
            request,
            std::time::UNIX_EPOCH + std::time::Duration::from_secs(now.max(0) as u64),
        )
        .await
    }
    pub async fn handle_async_at(&self, request: &Request, at: std::time::SystemTime) -> Dispatch {
        let now = at
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        if crate::files::ROUTES.contains(&request.path.as_str()) {
            if let Err(error) = self.preflight(request, now) {
                return Dispatch::response(error);
            }
            let (Some(files), Some(core), Some(workspace)) =
                (&self.files, &self.core, &self.workspace)
            else {
                return Dispatch::response(Response::error(
                    503,
                    "files_unavailable",
                    "file services are not initialized",
                ));
            };
            if let Some(response) = files
                .handle(
                    request,
                    core.as_ref(),
                    self.storage.as_deref(),
                    auth::logically_remote(request),
                    workspace.as_ref(),
                )
                .await
            {
                return Dispatch::response(response);
            }
        }
        self.handle_with_time(request, now, at)
    }
    pub fn handle(&self, request: &Request, now: i64) -> Dispatch {
        self.handle_with_time(
            request,
            now,
            std::time::UNIX_EPOCH + std::time::Duration::from_secs(now.max(0) as u64),
        )
    }
    fn handle_with_time(&self, request: &Request, now: i64, at: std::time::SystemTime) -> Dispatch {
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
        let port = i64::from(self.bound_port);
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
            let methods = guard_methods(&request.path);
            let guard = if base {
                auth::guard_base(request, config, port, methods)
            } else {
                self.auth.guard(request, config, port, methods, now)
            };
            if let Err(error) = guard {
                return Dispatch::response(error);
            }
            match request.path.as_str() {
                p if p == "/api/memos" || p.starts_with("/api/memos/") => match &self.memos {
                    Some(memos) => memos.handle_memos(
                        request,
                        |id| {
                            if id == 0 {
                                return String::new();
                            }
                            self.core
                                .as_ref()
                                .and_then(|core| core.snapshot(LiveSessionId(id)))
                                .map(|session| {
                                    let ceiling = if self.paths.is_trial() {
                                        Some(self.paths.root())
                                    } else {
                                        self.paths.root().parent()
                                    };
                                    crate::files::scope::find_git_root_with_home(
                                        std::path::Path::new(&session.cwd),
                                        ceiling,
                                    )
                                    .to_string_lossy()
                                    .into_owned()
                                })
                                .unwrap_or_default()
                        },
                        at,
                    ),
                    None => Response::error(
                        503,
                        "memo_store_unavailable",
                        "memo storage is unavailable",
                    ),
                },
                p if p == "/api/memo-images" || p.starts_with("/api/memo-images/") => {
                    match &self.memos {
                        Some(memos) => memos.handle_images(request),
                        None => Response::error(
                            503,
                            "memo_store_unavailable",
                            "memo storage is unavailable",
                        ),
                    }
                }
                "/api/auth/status" => self.auth.status_at(request, config, at),
                "/api/auth/login" => self.auth.login_at(request, config, at),
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
fn validate_bound_port(paths: &RuntimePaths, port: u16) -> std::io::Result<()> {
    if port == 0 || (paths.is_trial() && port != paths.port()) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "bound port must match explicit trial isolation; production uses the selected listener port",
        ));
    }
    Ok(())
}
fn guard_methods(path: &str) -> &'static [&'static str] {
    match path {
        p if p == "/api/memos" || p.starts_with("/api/memos/") => {
            &["GET", "POST", "PATCH", "DELETE"]
        }
        p if p == "/api/memo-images" || p.starts_with("/api/memo-images/") => &["GET", "POST"],
        p if crate::files::ROUTES.contains(&p) => {
            if matches!(
                p,
                "/api/files-list"
                    | "/api/files-content"
                    | "/api/files-asset"
                    | "/api/files-download"
                    | "/api/files-roots"
                    | "/api/git-log"
                    | "/api/git-show"
                    | "/api/git-refs"
                    | "/api/git-status"
                    | "/api/git-diff"
                    | "/api/git-turns"
                    | "/api/git-turn-diff"
            ) {
                &["GET"]
            } else {
                &["POST"]
            }
        }
        "/api/auth/status" => &["GET"],
        "/api/auth/login"
        | "/api/auth/logout"
        | "/api/auth/set-pin"
        | "/api/auth/revoke-all"
        | "/api/notify-generate-topic" => &["POST"],
        p if settings::PATHS.contains(&p) => &["GET", "POST"],
        _ => &[],
    }
}
