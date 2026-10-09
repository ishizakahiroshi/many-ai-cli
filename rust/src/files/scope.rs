use super::{Result, WorkspaceReads, err, safe_fs::Dir};
use crate::{
    config::paths::{Resource, RuntimePaths},
    hub::http::Request,
    proto::core::{LiveSessionId, SessionStorage},
};
use std::{
    io,
    path::{Component, Path, PathBuf},
};
#[derive(Clone)]
pub struct Scope {
    pub cwd: PathBuf,
    pub git_root: PathBuf,
    pub paths: RuntimePaths,
}
pub struct ReadGrant {
    pub path: PathBuf,
    pub read_only: bool,
    pub via_mention: bool,
}
impl Scope {
    pub fn new(cwd: PathBuf, paths: RuntimePaths) -> Self {
        let home = if paths.is_trial() {
            Some(paths.root())
        } else {
            paths.root().parent()
        };
        let git_root = find_git_root_with_home(&cwd, home);
        Self {
            cwd,
            git_root,
            paths,
        }
    }
    pub fn write_roots(&self) -> Vec<PathBuf> {
        vec![self.cwd.clone(), self.git_root.clone()]
    }
    pub fn read_roots(&self) -> Vec<PathBuf> {
        let mut roots = self.write_roots();
        roots.push(self.paths.resource(Resource::Attachments));
        roots.push(self.paths.resource(Resource::Orchestration));
        roots
    }
    pub fn allowed_write(&self, path: &Path) -> bool {
        under_roots(path, &self.write_roots())
    }
    pub fn root_itself(&self, path: &Path) -> bool {
        self.write_roots()
            .iter()
            .filter(|r| !r.as_os_str().is_empty())
            .any(|r| canonical(path).ok() == canonical(r).ok())
    }
    pub fn write_parent(&self, path: &Path) -> Result<(Dir, String)> {
        if !path.is_absolute() {
            return Err(err(400, "bad_request", "path must be an absolute path"));
        }
        let canonical = canonical(path).map_err(|e| super::io_error(e, "open_failed"))?;
        if !under_roots(&canonical, &self.write_roots()) {
            return Err(err(403, "forbidden", "path is outside allowed roots"));
        }
        if vcs_path(path) || vcs_path(&canonical) {
            return Err(err(
                403,
                "forbidden",
                "cannot modify files in version control directories",
            ));
        }
        let parent = canonical
            .parent()
            .ok_or_else(|| err(400, "bad_request", "path has no parent"))?;
        let name = canonical
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| err(400, "bad_request", "invalid name"))?
            .to_owned();
        Ok((
            Dir::open(parent).map_err(|e| super::io_error(e, "open_failed"))?,
            name,
        ))
    }
    pub fn write_entry_parent(&self, path: &Path) -> Result<(Dir, String)> {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| err(400, "bad_request", "invalid name"))?
            .to_owned();
        let parent = canonical(
            path.parent()
                .ok_or_else(|| err(400, "bad_request", "path has no parent"))?,
        )
        .map_err(|e| super::io_error(e, "open_failed"))?;
        if !under_roots(&parent, &self.write_roots()) {
            return Err(err(403, "forbidden", "path is outside allowed roots"));
        }
        if vcs_path(&parent) || vcs_path(Path::new(&name)) {
            return Err(err(403, "forbidden", "cannot modify version control paths"));
        }
        Ok((
            Dir::open(&parent).map_err(|e| super::io_error(e, "open_failed"))?,
            name,
        ))
    }
    pub fn read_grant(
        &self,
        r: &Request,
        storage: Option<&dyn SessionStorage>,
        remote: bool,
        workspace: &dyn WorkspaceReads,
    ) -> Result<ReadGrant> {
        let raw = r.query("path");
        if raw.is_empty() {
            return Err(err(400, "bad_request", "path parameter is required"));
        }
        let input = Path::new(&raw);
        if !input.is_absolute() {
            return Err(err(400, "bad_request", "path must be an absolute path"));
        }
        let path = clean(input);
        let read = canonical(&path).unwrap_or_else(|_| path.clone());
        if r.path == "/api/files-content" && workspace.recorded_handoff_note(&path, &read) {
            return Ok(ReadGrant {
                path: read,
                read_only: true,
                via_mention: false,
            });
        }
        if under_roots(&read, &self.read_roots()) {
            return Ok(ReadGrant {
                path: read,
                read_only: false,
                via_mention: false,
            });
        }
        if secret_denied(&path, &self.paths) || secret_denied(&read, &self.paths) {
            return Err(err(
                403,
                "forbidden",
                "forbidden: path is denied as a secret-like file",
            ));
        }
        if !remote {
            return Ok(ReadGrant {
                path: read,
                read_only: true,
                via_mention: false,
            });
        }
        let mentioned = [&path, &read].iter().any(|p| {
            let session = r.query("session").parse::<i64>().ok().filter(|id| *id > 0);
            let chat = match (storage, session) {
                (Some(store), Some(id)) => store
                    .messages_mention_text(LiveSessionId(id), &mention_variants(p, &self.cwd))
                    .unwrap_or(false),
                _ => false,
            };
            chat || workspace.memo_mentions().iter().any(|(project, text)| {
                mention_variants(p, Path::new(project))
                    .iter()
                    .any(|v| text.contains(v))
            })
        });
        if mentioned {
            Ok(ReadGrant {
                path: read,
                read_only: true,
                via_mention: true,
            })
        } else {
            Err(err(
                403,
                "forbidden",
                "forbidden: path is outside allowed roots",
            ))
        }
    }
}
pub fn clean(path: &Path) -> PathBuf {
    let mut p = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                p.pop();
            }
            _ => p.push(c),
        }
    }
    p
}
pub fn canonical(path: &Path) -> io::Result<PathBuf> {
    let path = clean(path);
    match std::fs::canonicalize(&path) {
        Ok(p) => Ok(p),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            // An existing dangling symlink is never treated as a missing plain leaf.
            if std::fs::symlink_metadata(&path).is_ok() {
                return Err(e);
            }
            let parent = path.parent().ok_or_else(|| io::Error::other("no parent"))?;
            let name = path
                .file_name()
                .ok_or_else(|| io::Error::other("no basename"))?;
            Ok(canonical(parent)?.join(name))
        }
        Err(e) => Err(e),
    }
}
pub fn under_roots(path: &Path, roots: &[PathBuf]) -> bool {
    let Ok(p) = canonical(path) else {
        return false;
    };
    roots
        .iter()
        .filter(|r| !r.as_os_str().is_empty())
        .filter_map(|r| canonical(r).ok())
        .any(|r| p.starts_with(r))
}
pub fn parent_capability(path: &Path) -> io::Result<(Dir, String)> {
    let p = clean(path);
    let parent = p.parent().ok_or_else(|| io::Error::other("no parent"))?;
    let name = p
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| io::Error::other("invalid filename"))?
        .to_owned();
    Ok((Dir::open(parent)?, name))
}
pub fn find_git_root(cwd: &Path) -> PathBuf {
    find_git_root_with_home(cwd, None)
}
pub fn find_git_root_with_home(cwd: &Path, home: Option<&Path>) -> PathBuf {
    let mut p = clean(cwd);
    let ceiling = home.filter(|h| p != *h && p.starts_with(h));
    loop {
        if ceiling == Some(p.as_path()) {
            break;
        }
        if p.join(".git").is_dir() {
            return p;
        }
        if !p.pop() {
            break;
        }
    }
    cwd.to_path_buf()
}
pub fn vcs_path(path: &Path) -> bool {
    fn has(p: &Path) -> bool {
        p.components().any(|c|matches!(c,Component::Normal(n)if matches!(n.to_string_lossy().to_ascii_lowercase().as_str(),".git"|".hg"|".svn")))
    }
    has(path) || canonical(path).is_ok_and(|p| has(&p))
}
pub fn secret_denied(path: &Path, paths: &RuntimePaths) -> bool {
    let base = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    // Go filepath.Ext includes a leading dot: .pem and .key are extensions
    // too. Path::extension deliberately treats those names as extensionless.
    let ext = base.rsplit_once('.').map_or("", |(_, extension)| extension);
    let parent = path
        .parent()
        .and_then(|p| p.file_name())
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    matches!(ext, "pem" | "key")
        || matches!(
            base.as_str(),
            "any-ai-cli.db"
                | "any-ai-cli.db-wal"
                | "any-ai-cli.db-shm"
                | "authorized_keys"
                | "known_hosts"
                | ".netrc"
                | "_netrc"
                | ".npmrc"
                | ".pypirc"
                | ".git-credentials"
                | ".htpasswd"
                | "kubeconfig"
                | ".pgpass"
                | ".my.cnf"
                | ".s3cfg"
                | ".boto"
                | ".dockercfg"
                | "secrets.yaml"
                | "secrets.yml"
                | "secrets.json"
        )
        || matches!(
            (parent.as_str(), base.as_str()),
            (".kube", "config") | (".docker", "config.json") | (".aws", "config")
        )
        || path.components().any(|p| {
            p.as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case(".gnupg")
        })
        || ["id_rsa", "id_dsa", "id_ecdsa", "id_ed25519"]
            .iter()
            .any(|p| base.starts_with(p))
        || base.contains("credentials")
        || base == ".env"
        || base.starts_with(".env.")
        || (base.starts_with("config.yaml") && under_roots(path, &[paths.root().into()]))
}
pub fn relative(path: &Path, base: &Path) -> PathBuf {
    let p = clean(path);
    let b = clean(base);
    let pc: Vec<_> = p.components().collect();
    let bc: Vec<_> = b.components().collect();
    let n = pc.iter().zip(&bc).take_while(|(a, b)| a == b).count();
    if n == 0 {
        return p;
    }
    let mut out = PathBuf::new();
    for _ in n..bc.len() {
        out.push("..");
    }
    for c in &pc[n..] {
        out.push(c);
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}
pub fn mention_variants(path: &Path, cwd: &Path) -> Vec<String> {
    let mut values = vec![path.to_string_lossy().into_owned()];
    if !cwd.as_os_str().is_empty() {
        let r = relative(path, cwd);
        if r != Path::new(".") {
            values.push(r.to_string_lossy().into_owned());
        }
    }
    let originals = values.clone();
    for s in originals {
        let v = if s.contains('\\') {
            s.replace('\\', "/")
        } else {
            s.replace('/', "\\")
        };
        if v != s {
            values.push(v);
        }
    }
    values
}
