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
                if path == "/api/open-terminal" {
                    if let Err(response) = self.allowed(r, &body.path, false) {
                        return response;
                    }
                    let app = match self.config.snapshot() {
                        Ok(cfg) => cfg.config.terminal_app,
                        Err(_) => {
                            return Response::error(500, "internal", "configuration unavailable");
                        }
                    };
                    return match self
                        .dispatch
                        .open(OpenKind::Terminal, Path::new(&body.path), &app)
                    {
                        Ok(()) => Response::json(200, &serde_json::json!({"ok":true})),
                        Err(error) => Response::error(500, "open_failed", &error.to_string()),
                    };
                }
                // Both routes take any local absolute path, so what the OS
                // receives is fixed once: a plain spelling is resolved to its
                // canonical path, the type is judged on that name, and that
                // same path is what is opened (U15).
                let raw = without_verbatim_disk(&body.path);
                if !plain_local_spelling(raw) {
                    return Response::error(403, "forbidden", "path spelling not allowed");
                }
                if let Err(response) = self.allowed(r, raw, true) {
                    return response;
                }
                let target = if path == "/api/open-default-file" {
                    default_app_target(Path::new(raw))
                } else {
                    folder_target(Path::new(raw))
                };
                let (kind, target) = match target {
                    Ok(target) => target,
                    Err(response) => return response,
                };
                match self.dispatch.open(kind, &target, "") {
                    Ok(()) if kind == OpenKind::Reveal => {
                        Response::json(200, &serde_json::json!({"ok":true,"revealed":true}))
                    }
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
/// `\\?\C:\…` is how Windows and Rust spell a canonical local path, so it is
/// read as `C:\…`. Any other path that starts with two separators still
/// names another machine or a device and is refused below.
fn without_verbatim_disk(raw: &str) -> &str {
    if cfg!(windows)
        && let Some(rest) = raw.strip_prefix(r"\\?\")
    {
        let bytes = rest.as_bytes();
        if bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && bytes[2] == b'\\'
        {
            return rest;
        }
    }
    raw
}
/// Win32 rewrites some spellings before it opens a file: it drops a trailing
/// dot or space from each name, reads `name:stream` as an alternate data
/// stream and resolves a final `.` or `..`. A path that starts with two
/// separators names another machine or a device. These are refused before
/// anything touches the file system, so the name judged is the name the OS
/// opens and no request leaves the machine (U15).
fn plain_local_spelling(raw: &str) -> bool {
    let windows = cfg!(windows);
    let separator = |c: char| c == '/' || (windows && c == '\\');
    let names: Vec<&str> = raw.split(separator).filter(|n| !n.is_empty()).collect();
    if matches!(names.last(), Some(&".") | Some(&"..")) {
        return false;
    }
    if !windows {
        return true;
    }
    let mut lead = raw.chars();
    if lead.next().is_some_and(separator) && lead.next().is_some_and(separator) {
        return false;
    }
    let bytes = raw.as_bytes();
    let drive = bytes.first().is_some_and(u8::is_ascii_alphabetic) && bytes.get(1) == Some(&b':');
    raw.char_indices()
        .all(|(i, c)| c != ':' || (i == 1 && drive))
        && names
            .iter()
            .all(|n| matches!(*n, "." | "..") || !(n.ends_with('.') || n.ends_with(' ')))
}
/// The canonical form of an existing path, spelled the way the OS takes it
/// (without the `\\?\` prefix `canonicalize` adds on Windows).
fn existing(path: &Path) -> std::io::Result<PathBuf> {
    std::fs::canonicalize(scope::clean(path))
        .map(|path| crate::config::paths::equivalent_root_prefix(&path))
}
/// "Open with default app": a folder opens as a folder, a file of a type in
/// `DEFAULT_APP_TYPES` opens in its app, and any other file is selected in
/// its folder instead.
fn default_app_target(raw: &Path) -> Result<(OpenKind, PathBuf), Response> {
    let missing = || Response::error(404, "not_found", "path not found");
    let target = existing(raw).map_err(|_| missing())?;
    let metadata = std::fs::metadata(&target).map_err(|_| missing())?;
    if metadata.is_dir() {
        return Ok((OpenKind::Directory, target));
    }
    if !metadata.is_file() {
        return Err(Response::error(403, "forbidden", "not a regular file"));
    }
    let kind = if opens_in_default_app(&target) {
        OpenKind::File
    } else {
        OpenKind::Reveal
    };
    Ok((kind, target))
}
/// "Open folder": a folder itself, or the folder holding a file or a missing
/// name. Only an existing folder is handed to the OS, which would open a
/// file it is given as a document.
fn folder_target(raw: &Path) -> Result<(OpenKind, PathBuf), Response> {
    let clean = scope::clean(raw);
    let target = match existing(&clean) {
        Ok(target) if target.is_dir() => target,
        Ok(target) => target.parent().map(Path::to_path_buf).unwrap_or_default(),
        Err(_) => clean
            .parent()
            .and_then(|parent| existing(parent).ok())
            .unwrap_or_default(),
    };
    if !target.as_os_str().is_empty() && target.is_dir() {
        Ok((OpenKind::Directory, target))
    } else {
        Err(Response::error(404, "not_found", "folder not found"))
    }
}
/// Types whose usual app only shows the content: Office documents that
/// cannot hold macros, PDF, images, audio, video, and the text and source
/// the Hub previews, without the scripts a machine may run when they are
/// "opened" (`.js`, `.py`, `.sh`, `.ps1`, `.bat` and the like). A list of
/// allowed types leaves every type it does not name closed, which a list of
/// executable types never could, since associations differ per machine.
const DEFAULT_APP_TYPES: &[&str] = &[
    "pdf",
    "docx",
    "xlsx",
    "pptx",
    "png",
    "jpg",
    "jpeg",
    "gif",
    "webp",
    "bmp",
    "svg",
    "ico",
    "tif",
    "tiff",
    "heic",
    "avif",
    "mp4",
    "webm",
    "ogv",
    "mov",
    "m4v",
    "mkv",
    "avi",
    "wmv",
    "mp3",
    "wav",
    "ogg",
    "m4a",
    "flac",
    "aac",
    "txt",
    "md",
    "markdown",
    "rst",
    "log",
    "json",
    "jsonl",
    "yaml",
    "yml",
    "toml",
    "ini",
    "cfg",
    "conf",
    "env",
    "csv",
    "tsv",
    "xml",
    "html",
    "htm",
    "css",
    "scss",
    "sass",
    "less",
    "ts",
    "tsx",
    "vue",
    "go",
    "rs",
    "java",
    "kt",
    "c",
    "cc",
    "cpp",
    "cxx",
    "h",
    "hh",
    "hpp",
    "cs",
    "sql",
    "graphql",
    "gql",
    "proto",
    "diff",
    "patch",
    "gitignore",
    "gitattributes",
    "editorconfig",
];
fn opens_in_default_app(path: &Path) -> bool {
    // A name without an extension has no type to judge, so it is shown in
    // its folder as well. Only an ASCII extension can match the list.
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.rsplit_once('.'))
        .is_some_and(|(_, extension)| {
            DEFAULT_APP_TYPES.contains(&extension.to_ascii_lowercase().as_str())
        })
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
