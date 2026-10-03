//! Frozen Go Files/Git/attachment service call chains. Authentication and Origin
//! guards run in Hub before this service; this module applies file scope policy.
mod attachments;
mod content;
mod git;
mod mutation;
pub mod safe_fs;
pub mod scope;
#[cfg(test)]
mod tests;
mod time;
use crate::{
    config::paths::RuntimePaths,
    hub::http::{Request, Response},
    proto::core::{LiveSessionId, SessionCore, SessionStorage},
};
pub use git::{GitTurnCompleted, GitTurnSnapshot, GitTurnState};
use std::path::PathBuf;

/// Values come from the actual memo/handoff stores, never request body claims.
pub trait WorkspaceReads: Send + Sync {
    fn memo_mentions(&self) -> Vec<(String, String)>;
    fn recorded_handoff_note(&self, path: &std::path::Path, canonical: &std::path::Path) -> bool;
}
pub struct FilesService {
    pub hub_cwd: PathBuf,
    pub paths: RuntimePaths,
    pub git_executable: PathBuf,
    /// Explicit per-command environment overlays, useful for isolated Git runners.
    pub git_environment: std::collections::BTreeMap<std::ffi::OsString, Option<std::ffi::OsString>>,
    pub turns: std::sync::Mutex<GitTurnState>,
}
impl FilesService {
    pub fn new(hub_cwd: PathBuf, paths: RuntimePaths) -> Self {
        Self {
            hub_cwd,
            paths,
            git_executable: resolve_git(),
            git_environment: Default::default(),
            turns: std::sync::Mutex::new(GitTurnState::default()),
        }
    }
    pub fn cwd(&self, request: &Request, core: &dyn SessionCore) -> PathBuf {
        request
            .query("session")
            .parse::<i64>()
            .ok()
            .and_then(|id| core.snapshot(LiveSessionId(id)))
            .map(|s| PathBuf::from(s.cwd))
            .unwrap_or_else(|| self.hub_cwd.clone())
    }
    pub async fn handle(
        &self,
        request: &Request,
        core: &dyn SessionCore,
        storage: Option<&dyn SessionStorage>,
        logical_remote: bool,
        workspace: &dyn WorkspaceReads,
    ) -> Option<Response> {
        let path = request.path.as_str();
        if !ROUTES.contains(&path) {
            return None;
        }
        let method = if path.starts_with("/api/git-") {
            if matches!(
                path,
                "/api/git-commit-all"
                    | "/api/git-commit-message"
                    | "/api/git-fetch"
                    | "/api/git-pull"
                    | "/api/git-push"
            ) {
                "POST"
            } else {
                "GET"
            }
        } else if matches!(
            path,
            "/api/files-list"
                | "/api/files-content"
                | "/api/files-asset"
                | "/api/files-download"
                | "/api/files-roots"
        ) {
            "GET"
        } else {
            "POST"
        };
        if let Err(e) = crate::hub::http::require_method(request, &[method]) {
            return Some(e);
        }
        let result = if path.starts_with("/api/git-") {
            self.git_handle(request, core).await
        } else if path == "/api/attach" {
            self.attach(request, core, storage)
        } else if path == "/api/attachments/purge" {
            self.purge_attachments(core)
        } else {
            let cwd = self.cwd(request, core);
            let scope = scope::Scope::new(cwd, self.paths.clone());
            if method == "GET" {
                self.read_handle(request, &scope, storage, logical_remote, workspace)
            } else {
                mutation::handle(request, &scope)
            }
        };
        Some(result.unwrap_or_else(|e| e))
    }
}
fn resolve_git() -> PathBuf {
    let name = if cfg!(windows) { "git.exe" } else { "git" };
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .map(|p| p.join(name))
        .find(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from(name))
}
pub const ROUTES: &[&str] = &[
    "/api/files-list",
    "/api/files-content",
    "/api/files-asset",
    "/api/files-download",
    "/api/files-roots",
    "/api/files-move",
    "/api/files-rename",
    "/api/files-mkdir",
    "/api/files-create",
    "/api/files-save",
    "/api/files-delete-dir",
    "/api/git-log",
    "/api/git-show",
    "/api/git-refs",
    "/api/git-status",
    "/api/git-diff",
    "/api/git-turns",
    "/api/git-turn-diff",
    "/api/git-commit-all",
    "/api/git-commit-message",
    "/api/git-fetch",
    "/api/git-pull",
    "/api/git-push",
    "/api/attach",
    "/api/attachments/purge",
];
pub(crate) type Result<T> = std::result::Result<T, Response>;
pub(crate) fn err(status: u16, code: &str, detail: &str) -> Response {
    Response::error(status, code, detail)
}
pub(crate) fn io_error(e: std::io::Error, code: &str) -> Response {
    if e.kind() == std::io::ErrorKind::NotFound {
        err(404, "not_found", "not found")
    } else if e.kind() == std::io::ErrorKind::AlreadyExists {
        err(409, "conflict", "target already exists")
    } else {
        err(500, code, &format!("{code}: {e}"))
    }
}
