//! Source host filesystem completion and OS dispatch routes. Hub authentication
//! precedes this adapter; native dispatch authority is supplied explicitly.
use super::{
    auth,
    http::{Request, Response, require_method},
};
use crate::{
    application::host_actions::{HostDispatch, OpenKind, PickerKind, terminal_description},
    config::{ConfigError, ConfigStore, RuntimePaths, paths::Resource},
    files::{
        FilesService,
        scope::{self, Scope},
    },
    proto::{
        core::SessionCore,
        wire::{Field, Schema},
    },
};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};
#[derive(Default, Deserialize)]
#[serde(default)]
struct Body {
    paths: Option<Vec<String>>,
    path: String,
    kind: String,
    terminal_app: String,
}
pub struct HostHttp {
    paths: RuntimePaths,
    config: Arc<ConfigStore>,
    core: Arc<dyn SessionCore>,
    files: Arc<FilesService>,
    dispatch: Arc<dyn HostDispatch>,
    home: Option<PathBuf>,
}
impl HostHttp {
    pub fn new(
        paths: RuntimePaths,
        config: Arc<ConfigStore>,
        core: Arc<dyn SessionCore>,
        files: Arc<FilesService>,
        dispatch: Arc<dyn HostDispatch>,
        home: Option<PathBuf>,
    ) -> Self {
        Self {
            paths,
            config,
            core,
            files,
            dispatch,
            home,
        }
    }
    pub async fn handle_authenticated(&self, r: &Request) -> Option<Response> {
        methods(&r.path)?;
        if let Err(response) = self.preflight_authenticated(r) {
            return Some(response);
        }
        Some(self.handle(r).await)
    }
    /// Shared header-only checks let the transport reject a peer before it
    /// consumes any request body, as the fixed Go handler does.
    pub fn preflight_authenticated(&self, r: &Request) -> Result<(), Response> {
        if let Some(methods) = methods(&r.path) {
            require_method(r, methods)?;
        } else {
            return Ok(());
        }
        let host_operation = !(r.path == "/api/terminal-app" && r.method == "GET")
            && matches!(
                r.path.as_str(),
                "/api/open-default-file"
                    | "/api/open-folder"
                    | "/api/open-terminal"
                    | "/api/open-dir"
                    | "/api/terminal-app"
            );
        if host_operation {
            let Some(host) = remote_host(&r.remote_addr) else {
                return Err(Response::error(
                    403,
                    "forbidden",
                    if r.path == "/api/open-dir" {
                        "loopback remote address required"
                    } else {
                        "loopback only"
                    },
                ));
            };
            if !native_loopback(host) {
                return Err(Response::error(403, "forbidden", "loopback only"));
            }
        }
        Ok(())
    }
    async fn handle(&self, r: &Request) -> Response {
        if matches!(r.path.as_str(), "/api/pick-directory" | "/api/pick-file") {
            let kind = if r.path == "/api/pick-directory" {
                PickerKind::Directory
            } else {
                PickerKind::File {
                    executable: r.query("filter") == "exe",
                }
            };
            return match self.dispatch.pick(kind).await {
                Ok(path) if path.is_empty() => {
                    Response::json(200, &serde_json::json!({"ok":false}))
                }
                Ok(path) => Response::json(200, &serde_json::json!({"ok":true,"path":path})),
                Err(error) => Response::error(500, "pick_failed", &format!("pick error: {error}")),
            };
        }
        if r.path == "/api/terminal-app" && r.method == "GET" {
            return match self.config.snapshot() {
                Ok(snapshot) => terminal_response(&snapshot.config.terminal_app, false),
                Err(_) => Response::error(500, "internal", "configuration unavailable"),
            };
        }
        let body = match decode_body(r) {
            Ok(body) => body,
            Err(_) => return Response::error(400, "bad_request", "invalid json"),
        };
        match r.path.as_str() {
            "/api/path-exists" => {
                let mut results = BTreeMap::new();
                for p in body.paths.unwrap_or_default() {
                    if p.is_empty() {
                        continue;
                    }
                    let target = if Path::new(&p).is_absolute() {
                        PathBuf::from(&p)
                    } else {
                        self.files.hub_cwd.join(&p)
                    };
                    results.insert(p, self.trial_allowed(&target) && target.is_dir());
                }
                Response::json(200, &serde_json::json!({"results":results}))
            }
            "/api/list-subdirs" => self.subdirs(r, &body.path),
            "/api/terminal-app" => self.save_terminal(&body.terminal_app),
            "/api/open-dir" => self.open_dir(r, body),
            path => {
                if body.path.is_empty() {
                    return Response::error(400, "bad_request", "path is required");
                }
                let kind = match path {
                    "/api/open-default-file" => OpenKind::File,
                    "/api/open-folder" => OpenKind::Directory,
                    _ => OpenKind::Terminal,
                };
                if let Err(response) = self.allowed(r, &body.path, kind != OpenKind::Terminal) {
                    return response;
                }
                if kind == OpenKind::File && denied_extension(&body.path) {
                    return Response::error(403, "forbidden", "executable file type not allowed");
                }
                let mut target = PathBuf::from(&body.path);
                if kind == OpenKind::Directory && !target.is_dir() {
                    target = scope::clean(target.parent().unwrap_or(Path::new(".")));
                }
                let app = if kind == OpenKind::Terminal {
                    match self.config.snapshot() {
                        Ok(cfg) => cfg.config.terminal_app,
                        Err(_) => {
                            return Response::error(500, "internal", "configuration unavailable");
                        }
                    }
                } else {
                    String::new()
                };
                match self.dispatch.open(kind, &target, &app) {
                    Ok(()) => Response::json(200, &serde_json::json!({"ok":true})),
                    Err(error) => Response::error(500, "open_failed", &error.to_string()),
                }
            }
        }
    }
    fn trial_allowed(&self, path: &Path) -> bool {
        !self.paths.is_trial() || scope::under_roots(path, &[self.paths.root().to_owned()])
    }
    fn allowed(&self, r: &Request, path: &str, relax: bool) -> Result<(), Response> {
        let path = Path::new(path);
        if !path.is_absolute() {
            return Err(Response::error(400, "bad_request", "path must be absolute"));
        }
        if !self.trial_allowed(path) {
            return Err(Response::error(
                403,
                "forbidden",
                "path is outside allowed roots",
            ));
        }
        if relax && !auth::logically_remote(r) {
            return Ok(());
        }
        let mut roots =
            Scope::new(self.files.cwd(r, self.core.as_ref()), self.paths.clone()).write_roots();
        roots.push(self.paths.resource(Resource::Attachments));
        if scope::under_roots(path, &roots) {
            Ok(())
        } else {
            Err(Response::error(
                403,
                "forbidden",
                "path is outside allowed roots",
            ))
        }
    }
    fn subdirs(&self, r: &Request, path: &str) -> Response {
        let raw = path.trim();
        if raw.is_empty() {
            return Response::json(200, &serde_json::json!({"ok":false,"subdirs":[]}));
        }
        if !Path::new(raw).is_absolute() {
            return Response::error(400, "bad_request", "path must be absolute");
        }
        let clean = super::preference_media::clean_native_path(raw, cfg!(windows));
        let target = Path::new(&clean);
        if !self.trial_allowed(target)
            || (auth::logically_remote(r) && !self.remote_list_allowed(target))
        {
            return Response::error(
                403,
                "forbidden",
                "path is outside allowed roots for remote callers",
            );
        }
        if !target.is_dir() {
            return Response::json(
                200,
                &serde_json::json!({"ok":false,"path":clean,"subdirs":[]}),
            );
        }
        let entries = match std::fs::read_dir(target) {
            Ok(entries) => entries,
            Err(error) => {
                return Response::error(500, "read_failed", &format!("readdir failed: {error}"));
            }
        };
        let mut subdirs = vec![];
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    return Response::error(
                        500,
                        "read_failed",
                        &format!("readdir failed: {error}"),
                    );
                }
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.is_empty() || name.starts_with('.') {
                continue;
            }
            if entry.file_type().is_ok_and(|ty| ty.is_dir()) {
                subdirs.push(name);
            }
        }
        subdirs.sort();
        subdirs.truncate(500);
        Response::json(
            200,
            &serde_json::json!({"ok":true,"path":clean,"subdirs":subdirs}),
        )
    }
    fn remote_list_allowed(&self, target: &Path) -> bool {
        let mut roots = vec![self.files.hub_cwd.to_string_lossy().into_owned()];
        if let Some(home) = &self.home {
            roots.push(home.to_string_lossy().into_owned());
        }
        roots.extend(self.core.snapshots().into_iter().map(|s| s.cwd));
        if let Ok(snapshot) = self.config.snapshot() {
            roots.extend(snapshot.config.user_prefs.project_favorites);
            roots.extend(snapshot.config.user_prefs.cwd_history);
            roots.extend(snapshot.config.user_prefs.cwd_favorites);
        }
        roots
            .into_iter()
            .filter(|v| !v.trim().is_empty() && Path::new(v.trim()).is_absolute())
            .any(|root| {
                ancestor(Path::new(root.trim()), target) || ancestor(target, Path::new(root.trim()))
            })
    }
    fn save_terminal(&self, app: &str) -> Response {
        let app = app.trim();
        loop {
            let mut snapshot = match self.config.snapshot() {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    return Response::error(500, "save_failed", &format!("save failed: {error}"));
                }
            };
            snapshot.config.terminal_app = app.to_owned();
            match self
                .config
                .publish_then_persist_legacy(snapshot.revision, snapshot.config)
            {
                Ok(snapshot) => return terminal_response(&snapshot.config.terminal_app, true),
                Err(ConfigError::Conflict { .. }) => continue,
                Err(error) => {
                    return Response::error(500, "save_failed", &format!("save failed: {error}"));
                }
            }
        }
    }
    fn open_dir(&self, r: &Request, body: Body) -> Response {
        if body.kind == "path" {
            if let Err(response) = self.allowed(r, &body.path, true) {
                return response;
            }
            return match self
                .dispatch
                .open(OpenKind::Reveal, Path::new(&body.path), "")
            {
                Ok(()) => Response::json(200, &serde_json::json!({"ok":true,"path":body.path})),
                Err(error) => Response::error(500, "open_failed", &format!("open failed: {error}")),
            };
        }
        let target = match body.kind.as_str() {
            "attach" => self.paths.resource(Resource::Attachments),
            "log" => match self.config.snapshot() {
                Ok(snapshot) => PathBuf::from(snapshot.config.hub.log_dir),
                Err(_) => return Response::error(500, "internal", "configuration unavailable"),
            },
            _ => return Response::error(400, "bad_request", "unknown kind"),
        };
        if target.as_os_str().is_empty() {
            return Response::error(500, "not_configured", "target dir not configured");
        }
        if !self.trial_allowed(&target) {
            return Response::error(403, "forbidden", "path is outside allowed roots");
        }
        if let Err(error) = std::fs::create_dir_all(&target) {
            return Response::error(500, "mkdir_failed", &format!("mkdir failed: {error}"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Err(error) =
                std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700))
            {
                return Response::error(500, "chmod_failed", &format!("chmod failed: {error}"));
            }
        }
        #[cfg(windows)]
        // Windows clears a DOS readonly attribute; this is not Unix mode 0777.
        #[allow(clippy::permissions_set_readonly_false)]
        {
            let metadata = match std::fs::metadata(&target) {
                Ok(metadata) => metadata,
                Err(error) => {
                    return Response::error(500, "chmod_failed", &format!("chmod failed: {error}"));
                }
            };
            let mut permissions = metadata.permissions();
            permissions.set_readonly(false);
            if let Err(error) = std::fs::set_permissions(&target, permissions) {
                return Response::error(500, "chmod_failed", &format!("chmod failed: {error}"));
            }
        }
        match self.dispatch.open(OpenKind::Directory, &target, "") {
            Ok(()) => Response::json(200, &serde_json::json!({"ok":true,"path":target})),
            Err(error) => Response::error(500, "open_failed", &format!("open failed: {error}")),
        }
    }
}
pub fn methods(path: &str) -> Option<&'static [&'static str]> {
    match path {
        "/api/terminal-app" => Some(&["GET", "POST"]),
        "/api/path-exists"
        | "/api/list-subdirs"
        | "/api/pick-directory"
        | "/api/pick-file"
        | "/api/open-default-file"
        | "/api/open-folder"
        | "/api/open-terminal"
        | "/api/open-dir" => Some(&["POST"]),
        _ => None,
    }
}
fn terminal_response(app: &str, ok: bool) -> Response {
    let mut value =
        serde_json::json!({"terminal_app":app,"effective_terminal_app":terminal_description(app)});
    if ok {
        value["ok"] = true.into();
    }
    Response::json(200, &value)
}
fn remote_host(remote: &str) -> Option<&str> {
    if remote.starts_with('[') {
        let close = remote.find("]:")?;
        Some(&remote[1..close])
    } else {
        let (host, port) = remote.rsplit_once(':')?;
        if host.contains(':') || port.contains(':') {
            None
        } else {
            Some(host)
        }
    }
}
fn denied_extension(path: &str) -> bool {
    let name = Path::new(path)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let extension = name
        .rfind('.')
        .map(|i| crate::proto::unicode::simple_lower(&name[i..]))
        .unwrap_or_default();
    [
        ".bat", ".cmd", ".com", ".exe", ".scr", ".pif", ".hta", ".cpl", ".msc", ".msi", ".reg",
        ".ps1", ".psm1", ".vbs", ".vbe", ".js", ".jse", ".wsf", ".wsh", ".lnk", ".scf", ".url",
    ]
    .contains(&extension.as_str())
}
fn ancestor(parent: &Path, child: &Path) -> bool {
    let clean =
        |p: &Path| super::preference_media::clean_native_path(&p.to_string_lossy(), cfg!(windows));
    let mut a = clean(parent);
    let mut b = clean(child);
    if cfg!(windows) {
        a = crate::proto::unicode::simple_fold_key(&a);
        b = crate::proto::unicode::simple_fold_key(&b);
    }
    if a == b {
        return true;
    }
    let separator = std::path::MAIN_SEPARATOR;
    if !a.ends_with(separator) {
        a.push(separator);
    }
    b.starts_with(&a)
}
#[cfg(test)]
mod tests;

fn decode_body(r: &Request) -> Result<Body, serde_json::Error> {
    const PATHS: &[Schema] = &[Schema {
        name: "HostRequest",
        fields: &[Field {
            name: "paths",
            kind: "[]string",
        }],
    }];
    const PATH: &[Schema] = &[Schema {
        name: "HostRequest",
        fields: &[Field {
            name: "path",
            kind: "string",
        }],
    }];
    const OPEN: &[Schema] = &[Schema {
        name: "HostRequest",
        fields: &[
            Field {
                name: "path",
                kind: "string",
            },
            Field {
                name: "kind",
                kind: "string",
            },
        ],
    }];
    const TERMINAL: &[Schema] = &[Schema {
        name: "HostRequest",
        fields: &[Field {
            name: "terminal_app",
            kind: "string",
        }],
    }];
    let schemas = match r.path.as_str() {
        "/api/path-exists" => PATHS,
        "/api/open-dir" => OPEN,
        "/api/terminal-app" => TERMINAL,
        _ => PATH,
    };
    crate::proto::wire::decode_http_schema(
        &r.body[..r.body.len().min(super::http::JSON_BODY_LIMIT)],
        "HostRequest",
        schemas,
    )
}

fn native_loopback(host: &str) -> bool {
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => ip.is_loopback(),
        Ok(std::net::IpAddr::V6(ip)) => ip
            .to_ipv4_mapped()
            .map_or_else(|| ip.is_loopback(), |ip| ip.is_loopback()),
        Err(_) => false,
    }
}
