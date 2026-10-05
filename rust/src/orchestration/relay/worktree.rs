//! Shared relay worktrees. Every Git operation uses the existing process owner
//! and explicit environment; cleanup never merges/deletes the result branch.
use crate::{
    config::{OrchestrationConfig, RuntimePaths},
    files::safe_fs::Dir,
    orchestration::child_launch::{safe_token, worktree::WorktreeGit},
    process::{self, ExitOutcome},
    proto::core::TaskCancellation,
};
use std::{
    ffi::OsString,
    fs,
    io::{self, Write},
    path::{Component, Path, PathBuf},
    time::Duration,
};

#[derive(Clone)]
pub struct RelayGit {
    paths: RuntimePaths,
    git: WorktreeGit,
    cwd: PathBuf,
}
#[derive(Debug)]
pub struct GitError {
    pub detail: String,
    pub not_git: bool,
}
impl RelayGit {
    pub fn new(paths: RuntimePaths, git: WorktreeGit, cwd: PathBuf) -> Self {
        Self { paths, git, cwd }
    }
    pub fn root(cwd: &Path, cfg: &OrchestrationConfig) -> PathBuf {
        let configured = cfg.worktree_dir_root.trim();
        let root = if configured.is_empty() {
            PathBuf::from(".many-ai-cli").join("worktrees")
        } else {
            PathBuf::from(configured)
        };
        clean(if root.is_absolute() {
            root
        } else {
            cwd.join(root)
        })
    }
    fn check(&self, path: &Path) -> Result<(), String> {
        crate::profile::subscriptions::check_path(&self.paths, path).map_err(|e| e.to_string())
    }
    pub async fn run(
        &self,
        cwd: &Path,
        args: &[&str],
        timeout: Duration,
        cancel: &TaskCancellation,
    ) -> Result<String, String> {
        self.check(cwd)?;
        let allow_stale_worktree = args.first() == Some(&"worktree")
            && args
                .get(1)
                .is_some_and(|operation| matches!(*operation, "list" | "prune" | "remove"));
        let _metadata = super::trial_git::inspect(
            &self.paths,
            cwd,
            args.first() == Some(&"init"),
            allow_stale_worktree,
        )
        .map_err(|error| error.to_string())?;
        let args = args.iter().map(OsString::from).collect::<Vec<_>>();
        let mut plan = self.git.command_plan(&self.cwd, cwd, &args, timeout);
        let _policy = super::trial_git::configure(&self.paths, &mut plan)
            .map_err(|error| error.to_string())?;
        plan.output_cap = 16 * 1024 * 1024;
        let output = process::run_capped(&plan, cancel.token())
            .await
            .map_err(|e| e.to_string())?;
        if output.stdout_truncated || output.stderr_truncated || output.pipes_forced_closed {
            return Err("git output was incomplete".into());
        }
        match output.outcome {
            ExitOutcome::Exited { code: Some(0), .. } => {
                Ok(String::from_utf8_lossy(&output.stdout).into_owned())
            }
            ExitOutcome::Cancelled => Err("context canceled".into()),
            ExitOutcome::TimedOut => Err("context deadline exceeded".into()),
            other => Err(format!(
                "{}: {other:?}",
                String::from_utf8_lossy(&output.stderr).trim()
            )),
        }
    }
    pub async fn head(&self, cwd: &Path, cancel: &TaskCancellation) -> Result<String, String> {
        self.run(cwd, &["rev-parse", "HEAD"], Duration::from_secs(3), cancel)
            .await
            .map(|s| s.trim().into())
    }
    pub async fn changed(
        &self,
        cwd: &Path,
        from: &str,
        cancel: &TaskCancellation,
    ) -> Result<i64, String> {
        let range = format!("{}..HEAD", from.trim());
        let args = if from.trim().is_empty() || !valid_revision(from.trim()) {
            vec!["status", "--short"]
        } else {
            vec!["diff", "--name-only", &range]
        };
        Ok(self
            .run(cwd, &args, Duration::from_secs(3), cancel)
            .await?
            .lines()
            .filter(|s| !s.trim().is_empty())
            .count() as i64)
    }
    pub async fn prepare(
        &self,
        parent: &Path,
        id: &str,
        cfg: &OrchestrationConfig,
        cancel: &TaskCancellation,
    ) -> Result<(PathBuf, String, String), GitError> {
        let error = |detail| GitError {
            detail,
            not_git: false,
        };
        self.check(parent).map_err(error)?;
        let top=self.run(parent,&["rev-parse","--show-toplevel"],Duration::from_secs(3),cancel).await.map_err(|_|GitError{detail:"parent cwd is not a git repository; pick same-tree mode or start the relay inside a repository".into(),not_git:true})?;
        let root = Self::root(parent, cfg);
        self.check(&root).map_err(error)?;
        let token = safe_token(id);
        let path = root.join(&token).join("relay");
        let branch = format!("many-ai-cli/relay/{token}");
        if fs::metadata(&path).is_ok() {
            self.validate(parent, &path, &branch, cancel)
                .await
                .map_err(error)?;
            return Ok((path, branch, String::new()));
        }
        let base = self.head(parent, cancel).await.map_err(error)?;
        Dir::open_or_create_private(path.parent().expect("relay parent"))
            .map_err(|e| error(e.to_string()))?;
        if let Ok(relative) = root.strip_prefix(top.trim())
            && !relative.as_os_str().is_empty()
        {
            let entry = format!("/{}/", relative.to_string_lossy().replace('\\', "/"));
            self.exclude(parent, &entry, cancel).await;
        }
        let target = path.to_string_lossy();
        if let Err(first) = self
            .run(
                parent,
                &["worktree", "add", "-b", &branch, &target, &base],
                Duration::from_secs(30),
                cancel,
            )
            .await
        {
            if first.contains("already exists") {
                self.run(
                    parent,
                    &["worktree", "add", &target, &branch],
                    Duration::from_secs(30),
                    cancel,
                )
                .await
                .map_err(error)?;
            } else {
                return Err(error(first));
            }
        }
        Ok((path, branch, base))
    }
    pub async fn validate(
        &self,
        parent: &Path,
        path: &Path,
        branch: &str,
        cancel: &TaskCancellation,
    ) -> Result<(), String> {
        self.check(parent)?;
        self.check(path)?;
        if self.paths.is_trial() {
            Dir::open(path).map_err(|e| e.to_string())?;
        }
        let _parent_metadata = super::trial_git::inspect(&self.paths, parent, false, false)
            .map_err(|error| error.to_string())?;
        let _child_metadata = super::trial_git::inspect(&self.paths, path, false, false)
            .map_err(|error| error.to_string())?;
        let mut plan = self
            .git
            .command_plan(&self.cwd, parent, &[], Duration::from_secs(3));
        let _policy = super::trial_git::configure(&self.paths, &mut plan)
            .map_err(|error| error.to_string())?;
        WorktreeGit::new(plan.executable, plan.env)
            .validate_identity(parent, path, branch, cancel)
            .await
    }
    async fn exclude(&self, parent: &Path, entry: &str, cancel: &TaskCancellation) {
        let Ok(common) = self
            .run(
                parent,
                &["rev-parse", "--git-common-dir"],
                Duration::from_secs(3),
                cancel,
            )
            .await
        else {
            return;
        };
        let common = PathBuf::from(common.trim());
        let common = clean(if common.is_absolute() {
            common
        } else {
            parent.join(common)
        });
        if self.check(&common).is_err() {
            return;
        }
        let Ok(dir) = Dir::open_or_create_private(&common.join("info")) else {
            return;
        };
        let prior = dir.read("exclude", 1024 * 1024).unwrap_or_default();
        if String::from_utf8_lossy(&prior)
            .lines()
            .any(|s| s.trim() == entry)
        {
            return;
        }
        let Ok(mut file) = dir.open_append("exclude") else {
            return;
        };
        let prefix = if !prior.is_empty() && prior.last() != Some(&b'\n') {
            "\n"
        } else {
            ""
        };
        let _ = writeln!(file, "{prefix}{entry}");
    }
    pub fn validate_cleanup(
        &self,
        parent: &Path,
        path: &Path,
        cfg: &OrchestrationConfig,
    ) -> Result<(), String> {
        self.check(path)?;
        let path = cleanup_target(path)?;
        let root = Self::root(parent, cfg);
        if path == root {
            return Err("relay worktree path cannot be the worktree root".into());
        }
        let actual_root = resolve_cleanup_directory(&root)?;
        match fs::canonicalize(&path) {
            Ok(actual_path) => {
                if actual_path == actual_root || !actual_path.starts_with(&actual_root) {
                    return Err(
                        "relay worktree path is outside the configured worktree root".into(),
                    );
                }
                Ok(())
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                confined_missing(&actual_root, &path)
            }
            Err(_) => Err("relay worktree path is outside the configured worktree root".into()),
        }
    }
    pub async fn cleanup(
        &self,
        parent: &Path,
        path: &Path,
        cancel: &TaskCancellation,
    ) -> Result<(), String> {
        self.check(parent)?;
        self.check(path)?;
        // Use the same target as validation, including for all fallback IO.
        let path = cleanup_target(path)?;
        let path = path.as_path();
        let target = path.to_string_lossy();
        // A directory that is already gone cannot be locking a child. Its Git
        // registration is stale and prune is safe. An existing directory that
        // is still registered keeps the in-use error. The result branch stays.
        if let Err(error) = self
            .run(
                parent,
                &["worktree", "remove", "--force", &target],
                Duration::from_secs(30),
                cancel,
            )
            .await
        {
            let list = self
                .run(
                    parent,
                    &["worktree", "list", "--porcelain", "-z"],
                    Duration::from_secs(30),
                    cancel,
                )
                .await
                .map_err(|verify| {
                    format!(
                        "remove relay worktree: {error} (could not verify worktree registration: {verify})"
                    )
                })?;
            let listed_worktrees = registered_worktree_paths(&list).map_err(|verify| {
                format!(
                    "remove relay worktree: {error} (could not verify worktree registration: {verify})"
                )
            })?;
            // Inspect after the awaited Git commands; a directory recreated
            // while removing must retain the registered/in-use protection.
            let missing = cleanup_directory_missing(path)?;
            let mut registered = false;
            if !missing {
                for listed in &listed_worktrees {
                    if same_cleanup_directory(listed, path)? {
                        registered = true;
                        break;
                    }
                }
            }
            if registered {
                return Err(format!(
                    "relay child may still be using the worktree; close the child sessions and retry cleanup: {error}"
                ));
            }
            // Recheck after listing/identity inspection, before fallback Git IO.
            cleanup_directory_missing(path)?;
            self.run(
                parent,
                &["worktree", "prune"],
                Duration::from_secs(30),
                cancel,
            )
            .await?;
            // Prune can recreate/register a directory through concurrent Git
            // activity. Refresh registration after that await, then inspect the
            // target immediately before fallback; shape checks alone are insufficient.
            let list = self
                .run(
                    parent,
                    &["worktree", "list", "--porcelain", "-z"],
                    Duration::from_secs(30),
                    cancel,
                )
                .await?;
            let listed_worktrees = registered_worktree_paths(&list)?;
            let missing = cleanup_directory_missing(path)?;
            for listed in &listed_worktrees {
                let registered = if missing {
                    same_missing_cleanup_path(listed, path)
                } else {
                    same_cleanup_directory(listed, path)?
                };
                if registered {
                    return Err("relay worktree remains registered after prune; close its users and retry cleanup".into());
                }
            }
            let parent = match Dir::open(path.parent().ok_or("relay worktree has no parent")?) {
                Ok(parent) => Some(parent),
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(error.to_string()),
            };
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or("relay worktree has no basename")?;
            match parent.map_or(Ok(()), |parent| parent.remove_tree(name)) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(format!(
                        "relay worktree was unregistered but its directory is still in use; close child sessions and retry cleanup: {e}"
                    ));
                }
            }
        }
        // No recursive parent cleanup and no branch deletion.
        if let Some(parent) = path.parent() {
            let _ = fs::remove_dir(parent);
        }
        Ok(())
    }
    pub async fn resolve_plan(
        &self,
        parent: &Path,
        raw: &str,
        cancel: &TaskCancellation,
    ) -> Result<PathBuf, String> {
        let fail = |text: &str| format!("plan path must be an absolute path: {text}");
        if raw.trim().is_empty() {
            return Err(fail("plan_path is required"));
        }
        if parent.as_os_str().is_empty() {
            return Err(fail("parent session has no cwd"));
        }
        let parent = if parent.is_absolute() {
            parent.to_owned()
        } else {
            self.cwd.join(parent)
        };
        self.check(&parent)?;
        let base = fs::canonicalize(&parent).map_err(|_| fail("parent cwd is unavailable"))?;
        if !base.is_dir() {
            return Err(fail("parent cwd is not a directory"));
        }
        let requested = Path::new(raw.trim());
        let lexical = clean(if requested.is_absolute() {
            requested.to_owned()
        } else {
            base.join(requested)
        });
        self.check(&lexical)?;
        let path = fs::canonicalize(&lexical).map_err(|_| fail("plan file does not exist"))?;
        self.check(&path)?;
        let meta = fs::metadata(&path).map_err(|_| fail("plan file does not exist"))?;
        if !meta.is_file() {
            return Err(fail("plan_path must be a regular file"));
        }
        if !path
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("md"))
        {
            return Err(fail("plan_path must point to a .md file"));
        }
        if meta.len() > 1024 * 1024 {
            return Err(fail("plan_path must be at most 1048576 bytes"));
        }
        let git_root = self
            .run(
                &base,
                &["rev-parse", "--show-toplevel"],
                Duration::from_secs(3),
                cancel,
            )
            .await
            .unwrap_or_default();
        let resolved_root = if git_root.trim().is_empty() {
            PathBuf::new()
        } else {
            fs::canonicalize(git_root.trim()).unwrap_or_else(|_| PathBuf::from(git_root.trim()))
        };
        let root = resolved_root.as_path();
        let inside = |p: &Path| {
            p.starts_with(&base) || (!root.as_os_str().is_empty() && p.starts_with(root))
        };
        if !inside(&path)
            && (self.paths.is_trial()
                || !inside(&lexical)
                || fs::symlink_metadata(&lexical).map_or(true, |m| m.file_type().is_symlink()))
        {
            return Err(fail("plan_path is outside the parent cwd and git root"));
        }
        if self.paths.is_trial() {
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or_else(|| fail("plan file is unavailable"))?;
            Dir::open(path.parent().expect("plan parent"))
                .and_then(|dir| dir.open_file(name, false))
                .map_err(|_| fail("plan file is unavailable"))?;
        }
        Ok(path)
    }
}
fn registered_worktree_paths(output: &str) -> Result<Vec<PathBuf>, String> {
    if !output.ends_with("\0\0") {
        return Err("Git worktree list was not NUL-delimited".into());
    }
    let fields = output.split('\0').collect::<Vec<_>>();
    let mut paths = Vec::new();
    let mut in_record = false;
    for (index, field) in fields.iter().enumerate() {
        if field.is_empty() {
            if index + 1 == fields.len() && !in_record {
                continue;
            }
            if !in_record {
                return Err("Git worktree list contained a malformed record boundary".into());
            }
            in_record = false;
            continue;
        }
        if let Some(path) = field.strip_prefix("worktree ") {
            if in_record || path.is_empty() || !Path::new(path).is_absolute() {
                return Err("Git worktree list contained an invalid worktree path".into());
            }
            paths.push(PathBuf::from(path));
            in_record = true;
        } else if !in_record {
            return Err("Git worktree list contained an attribute without a record".into());
        }
    }
    if in_record || paths.is_empty() {
        return Err("Git worktree list contained an incomplete record".into());
    }
    Ok(paths)
}
fn cleanup_directory_missing(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            let linked = metadata.file_type().is_symlink();
            #[cfg(windows)]
            let linked = {
                use std::os::windows::fs::MetadataExt;
                linked || metadata.file_attributes() & 0x400 != 0
            };
            if linked || !metadata.is_dir() {
                return Err(
                    "relay worktree cleanup target must be a directory, not a file or link".into(),
                );
            }
            Ok(false)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(format!("inspect relay worktree during cleanup: {error}")),
    }
}
#[cfg(windows)]
fn cleanup_directory_identity(path: &Path) -> io::Result<(u32, u32, u32)> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
        FILE_SHARE_READ, FILE_SHARE_WRITE, GetFileInformationByHandle,
    };
    let file = fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let mut information: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: file owns a live handle, and information is a writable output.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if information.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0
        || information.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
    {
        return Err(io::Error::other(
            "cleanup identity is not a plain directory",
        ));
    }
    Ok((
        information.dwVolumeSerialNumber,
        information.nFileIndexHigh,
        information.nFileIndexLow,
    ))
}
fn same_cleanup_directory(listed: &Path, target: &Path) -> Result<bool, String> {
    #[cfg(windows)]
    {
        let target = cleanup_directory_identity(target)
            .map_err(|error| format!("inspect relay cleanup target identity: {error}"))?;
        match cleanup_directory_identity(listed) {
            Ok(listed) => Ok(listed == target),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(format!(
                "inspect registered relay worktree identity: {error}"
            )),
        }
    }
    #[cfg(not(windows))]
    {
        Ok(clean(listed.to_owned()) == clean(target.to_owned()))
    }
}
fn same_missing_cleanup_path(listed: &Path, target: &Path) -> bool {
    #[cfg(windows)]
    {
        // No target inode exists to compare. Conservatively retain a registration
        // whose spelling differs only by Windows case/verbatim/UNC aliases.
        fn spelling(path: &Path) -> String {
            let value = clean(path.to_owned())
                .to_string_lossy()
                .replace('/', "\\")
                .to_lowercase();
            if let Some(rest) = value.strip_prefix("\\\\?\\unc\\") {
                format!("\\\\{rest}")
            } else {
                value.strip_prefix("\\\\?\\").unwrap_or(&value).to_owned()
            }
        }
        spelling(listed) == spelling(target)
    }
    #[cfg(not(windows))]
    {
        clean(listed.to_owned()) == clean(target.to_owned())
    }
}
fn cleanup_target(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("relay worktree path must be absolute".into());
    }
    // Lexically removing `..` before canonicalization can validate a different
    // directory from the one Git resolves through a preceding symlink.
    if path
        .components()
        .any(|part| matches!(part, Component::ParentDir))
    {
        return Err("relay worktree cleanup path contains parent traversal".into());
    }
    #[cfg(windows)]
    if path.components().any(|part| match part {
        Component::Normal(name) => {
            use std::os::windows::ffi::OsStrExt;
            name.encode_wide()
                .last()
                .is_some_and(|unit| matches!(unit, 0x002e | 0x0020))
        }
        _ => false,
    }) {
        return Err("relay worktree cleanup path contains a Windows trailing-dot alias".into());
    }
    Ok(clean(path.to_owned()))
}
fn confined_missing(actual_root: &Path, path: &Path) -> Result<(), String> {
    let reject = "relay worktree path is outside the configured worktree root";
    // A dangling link is an existing entry, not a missing directory tail.
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        _ => return Err(reject.into()),
    }
    let resolved = resolve_cleanup_directory(path)?;
    if resolved == actual_root || !resolved.starts_with(actual_root) {
        return Err(reject.into());
    }
    Ok(())
}
fn resolve_cleanup_directory(path: &Path) -> Result<PathBuf, String> {
    let reject = "relay worktree path is outside the configured worktree root";
    let mut cursor = path.to_owned();
    let mut suffix = Vec::new();
    loop {
        match fs::symlink_metadata(&cursor) {
            Ok(meta) => {
                if !meta.is_dir() && !meta.file_type().is_symlink() {
                    return Err(reject.into());
                }
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let Some(name) = cursor.file_name() else {
                    return Err(reject.into());
                };
                if name.is_empty()
                    || name == std::ffi::OsStr::new(".")
                    || name == std::ffi::OsStr::new("..")
                {
                    return Err(reject.into());
                }
                suffix.push(name.to_os_string());
                if !cursor.pop() {
                    return Err(reject.into());
                }
            }
            Err(_) => return Err(reject.into()),
        }
    }
    let actual_ancestor = fs::canonicalize(&cursor).map_err(|_| reject.to_owned())?;
    if !actual_ancestor.is_dir() {
        return Err(reject.into());
    }
    let mut resolved = actual_ancestor;
    for name in suffix.iter().rev() {
        resolved.push(name);
    }
    Ok(resolved)
}
pub(super) fn clean(path: PathBuf) -> PathBuf {
    let mut result = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(result.components().next_back(), Some(Component::Normal(_))) {
                    result.pop();
                } else if !result.has_root() {
                    result.push("..");
                }
            }
            other => result.push(other.as_os_str()),
        }
    }
    result
}
fn valid_revision(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('-')
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._/-".contains(c))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    #[test]
    fn worktree_list_parser_preserves_newlines_inside_paths() {
        let path = std::env::temp_dir().join("registered\nrelay/with-newline");
        let output = format!("worktree {}\0HEAD synthetic\0\0", path.display());
        assert_eq!(registered_worktree_paths(&output).unwrap(), vec![path]);
        assert!(registered_worktree_paths("worktree /tmp/not-nul-delimited\n").is_err());
        let malformed_tail = format!(
            "worktree {}\0HEAD synthetic\0\0worktree {}\nHEAD synthetic",
            std::env::temp_dir().join("other").display(),
            std::env::temp_dir().join("truncated").display()
        );
        assert!(registered_worktree_paths(&malformed_tail).is_err());
        assert!(registered_worktree_paths("worktree relative\0\0").is_err());
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn prune_recreated_or_newly_registered_directory_is_not_deleted() {
        use std::os::unix::fs::PermissionsExt;
        for initially_missing in [true, false] {
            let root = tempfile::tempdir().unwrap();
            let repo = root.path().join("synthetic-repository");
            fs::create_dir(&repo).unwrap();
            let target = repo.join("worktrees/token\nnewline/relay");
            let marker = root.path().join("registered");
            if initially_missing {
                // The first listing retains a stale registration without a directory.
                fs::write(&marker, b"synthetic").unwrap();
            } else {
                fs::create_dir_all(&target).unwrap();
            }
            let executable = root.path().join("fake-git");
            fs::write(
                &executable,
                br#"#!/bin/sh
case "$3:$4" in
worktree:remove) exit 1 ;;
worktree:list)
    if [ -f "$SYNTHETIC_REGISTRATION" ]; then
        printf 'worktree %s\000HEAD synthetic\000branch refs/heads/synthetic\000\000' "$SYNTHETIC_TARGET"
    fi
    ;;
worktree:prune)
    mkdir -p "$SYNTHETIC_TARGET" || exit 2
    printf 'synthetic preserved' > "$SYNTHETIC_TARGET/sentinel" || exit 3
    printf 'synthetic' > "$SYNTHETIC_REGISTRATION" || exit 4
    ;;
*) exit 5 ;;
esac
"#,
            )
            .unwrap();
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
            let git = RelayGit::new(
                RuntimePaths::production(root.path()).unwrap(),
                WorktreeGit::new(
                    executable,
                    BTreeMap::from([
                        (
                            OsString::from("SYNTHETIC_TARGET"),
                            Some(target.clone().into_os_string()),
                        ),
                        (
                            OsString::from("SYNTHETIC_REGISTRATION"),
                            Some(marker.into_os_string()),
                        ),
                    ]),
                ),
                repo.clone(),
            );
            let error = git
                .cleanup(&repo, &target, &TaskCancellation::default())
                .await
                .unwrap_err();
            assert!(error.contains("remains registered after prune"), "{error}");
            assert_eq!(
                fs::read(target.join("sentinel")).unwrap(),
                b"synthetic preserved"
            );
        }
    }
    #[cfg(windows)]
    #[test]
    fn missing_registration_comparison_preserves_windows_spelling_aliases() {
        assert!(same_missing_cleanup_path(
            Path::new(r"\\?\C:\Synthetic\Relay"),
            Path::new(r"c:\synthetic\relay"),
        ));
        assert!(same_missing_cleanup_path(
            Path::new(r"\\?\UNC\server\share\Synthetic\Relay"),
            Path::new(r"\\SERVER\SHARE\synthetic\relay"),
        ));
        assert!(!same_missing_cleanup_path(
            Path::new(r"C:\Synthetic\other"),
            Path::new(r"C:\Synthetic\relay")
        ));
    }
    #[cfg(windows)]
    #[test]
    fn registered_directory_identity_matches_case_and_verbatim_aliases() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("MiXeD-relay-worktree");
        fs::create_dir(&target).unwrap();
        let case_alias = root.path().join("MIXED-RELAY-WORKTREE");
        let verbatim = target.canonicalize().unwrap();
        assert!(same_cleanup_directory(&case_alias, &target).unwrap());
        assert!(same_cleanup_directory(&verbatim, &case_alias).unwrap());
        let other = root.path().join("other-worktree");
        fs::create_dir(&other).unwrap();
        assert!(!same_cleanup_directory(&other, &target).unwrap());
        assert!(!same_cleanup_directory(&root.path().join("missing"), &target).unwrap());
    }
    #[test]
    fn fallback_target_inspection_distinguishes_missing_directory_and_file() {
        let root = tempfile::tempdir().unwrap();
        assert!(!cleanup_directory_missing(root.path()).unwrap());
        assert!(cleanup_directory_missing(&root.path().join("missing")).unwrap());
        let file = root.path().join("file");
        fs::write(&file, b"synthetic").unwrap();
        assert!(cleanup_directory_missing(&file).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn fallback_target_inspection_rejects_directory_and_dangling_symlinks() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let link = root.path().join("link");
        symlink(root.path(), &link).unwrap();
        assert!(cleanup_directory_missing(&link).is_err());
        fs::remove_file(&link).unwrap();
        symlink(root.path().join("missing"), &link).unwrap();
        assert!(cleanup_directory_missing(&link).is_err());
    }
    struct Fixture {
        _root: tempfile::TempDir,
        paths: RuntimePaths,
        repo: PathBuf,
        git: RelayGit,
    }
    fn home_for_hooks(runtime: &Path) -> PathBuf {
        let hooks = runtime.join("empty-hooks");
        fs::create_dir_all(&hooks).unwrap();
        hooks
    }
    async fn fixture() -> Fixture {
        let root = tempfile::tempdir().unwrap();
        let runtime = root.path().join("trial");
        fs::create_dir(&runtime).unwrap();
        let paths = RuntimePaths::trial(&runtime, 49661, &root.path().join("installed")).unwrap();
        let home = runtime.join("home");
        fs::create_dir(&home).unwrap();
        let empty = runtime.join("empty-git-config");
        fs::write(&empty, "").unwrap();
        let env: BTreeMap<OsString, Option<OsString>> = BTreeMap::from([
            ("HOME".into(), Some(home.clone().into_os_string())),
            ("USERPROFILE".into(), Some(home.into_os_string())),
            ("GIT_CONFIG_NOSYSTEM".into(), Some("1".into())),
            ("GIT_CONFIG_COUNT".into(), Some("0".into())),
            ("GIT_CONFIG_GLOBAL".into(), Some(empty.into_os_string())),
            ("GIT_CONFIG_PARAMETERS".into(), None),
            ("GIT_CONFIG".into(), None),
            ("GIT_DIR".into(), None),
            ("GIT_WORK_TREE".into(), None),
            ("GIT_INDEX_FILE".into(), None),
            ("GIT_COMMON_DIR".into(), None),
            ("GIT_OBJECT_DIRECTORY".into(), None),
            ("GIT_ALTERNATE_OBJECT_DIRECTORIES".into(), None),
        ]);
        let git = RelayGit::new(
            paths.clone(),
            WorktreeGit::new("git".into(), env),
            runtime.clone(),
        );
        let repo = runtime.join("repository with spaces 日本");
        fs::create_dir(&repo).unwrap();
        let cancel = TaskCancellation::default();
        for args in [
            &["init", "-q"][..],
            &["config", "user.name", "Synthetic Relay Fixture"],
            &["config", "user.email", "relay@example.invalid"],
            &["config", "commit.gpgsign", "false"],
            &[
                "config",
                "core.hooksPath",
                home_for_hooks(&runtime).to_str().unwrap(),
            ],
        ] {
            git.run(&repo, args, Duration::from_secs(10), &cancel)
                .await
                .unwrap();
        }
        fs::write(repo.join("plan.md"), "# Synthetic plan\n").unwrap();
        git.run(&repo, &["add", "plan.md"], Duration::from_secs(10), &cancel)
            .await
            .unwrap();
        git.run(
            &repo,
            &["commit", "-qm", "synthetic base"],
            Duration::from_secs(10),
            &cancel,
        )
        .await
        .unwrap();
        Fixture {
            _root: root,
            paths,
            repo,
            git,
        }
    }
    #[tokio::test]
    async fn shared_worktree_keeps_parent_unchanged_and_cleanup_retains_result_branch() {
        let f = fixture().await;
        let cancel = TaskCancellation::default();
        let cfg = OrchestrationConfig::default();
        let initial = f.git.head(&f.repo, &cancel).await.unwrap();
        let (path, branch, base) = f
            .git
            .prepare(&f.repo, "r7-synthetic", &cfg, &cancel)
            .await
            .unwrap();
        assert_eq!(base, initial);
        assert_eq!(branch, "many-ai-cli/relay/r7-synthetic");
        assert!(path.is_dir());
        fs::write(path.join("result with spaces.txt"), "synthetic result\n").unwrap();
        f.git
            .run(
                &path,
                &["add", "result with spaces.txt"],
                Duration::from_secs(10),
                &cancel,
            )
            .await
            .unwrap();
        f.git
            .run(
                &path,
                &["commit", "-qm", "synthetic result"],
                Duration::from_secs(10),
                &cancel,
            )
            .await
            .unwrap();
        let result = f.git.head(&path, &cancel).await.unwrap();
        assert_ne!(result, base);
        assert_eq!(f.git.head(&f.repo, &cancel).await.unwrap(), initial);
        assert!(!f.repo.join("result with spaces.txt").exists());
        assert_eq!(f.git.changed(&path, &base, &cancel).await.unwrap(), 1);
        let reused = f
            .git
            .prepare(&f.repo, "r7-synthetic", &cfg, &cancel)
            .await
            .unwrap();
        assert_eq!(reused.0, path);
        assert!(reused.2.is_empty());
        f.git.validate_cleanup(&f.repo, &path, &cfg).unwrap();
        f.git.cleanup(&f.repo, &path, &cancel).await.unwrap();
        assert!(!path.exists());
        assert_eq!(
            f.git
                .run(
                    &f.repo,
                    &["rev-parse", "--verify", &branch],
                    Duration::from_secs(10),
                    &cancel
                )
                .await
                .unwrap()
                .trim(),
            result
        );
        let restored = f
            .git
            .prepare(&f.repo, "r7-synthetic", &cfg, &cancel)
            .await
            .unwrap();
        assert_eq!(restored.0, path);
        assert_eq!(f.git.head(&path, &cancel).await.unwrap(), result);
        assert_eq!(f.git.head(&f.repo, &cancel).await.unwrap(), initial);
        assert!(f.paths.relative_to_selected_root(&path).is_ok());
    }
    #[tokio::test]
    async fn worktree_reuse_rejects_wrong_branch_without_repair() {
        let f = fixture().await;
        let cancel = TaskCancellation::default();
        let cfg = OrchestrationConfig::default();
        let (path, _, _) = f
            .git
            .prepare(&f.repo, "r8-synthetic", &cfg, &cancel)
            .await
            .unwrap();
        f.git
            .run(
                &path,
                &["checkout", "-qb", "unrelated-synthetic"],
                Duration::from_secs(10),
                &cancel,
            )
            .await
            .unwrap();
        let error = f
            .git
            .prepare(&f.repo, "r8-synthetic", &cfg, &cancel)
            .await
            .unwrap_err();
        assert!(error.detail.contains("identity mismatch"));
        assert_eq!(
            f.git
                .run(
                    &path,
                    &["branch", "--show-current"],
                    Duration::from_secs(10),
                    &cancel
                )
                .await
                .unwrap()
                .trim(),
            "unrelated-synthetic"
        );
    }
    #[tokio::test]
    async fn canceled_prepare_and_non_repository_never_fall_back_to_same_tree() {
        let f = fixture().await;
        let missing = f.paths.root().join("non-repository");
        fs::create_dir(&missing).unwrap();
        let cfg = OrchestrationConfig::default();
        let error = f
            .git
            .prepare(&missing, "r9-synthetic", &cfg, &TaskCancellation::default())
            .await
            .unwrap_err();
        assert!(error.not_git);
        assert!(!RelayGit::root(&missing, &cfg).exists());
        let cancel = TaskCancellation::default();
        cancel.token().cancel();
        assert!(
            f.git
                .prepare(&f.repo, "r10-canceled", &cfg, &cancel)
                .await
                .is_err()
        );
        assert!(!RelayGit::root(&f.repo, &cfg).join("r10-canceled").exists());
    }
    #[tokio::test]
    async fn missing_worktree_stays_inside_root_and_prune_keeps_branch() {
        let f = fixture().await;
        let cancel = TaskCancellation::default();
        let cfg = OrchestrationConfig::default();
        let (path, branch, _) = f
            .git
            .prepare(&f.repo, "r11-missing", &cfg, &cancel)
            .await
            .unwrap();
        let kept = f.git.head(&path, &cancel).await.unwrap();
        fs::remove_dir_all(&path).unwrap();
        f.git.validate_cleanup(&f.repo, &path, &cfg).unwrap();
        f.git.cleanup(&f.repo, &path, &cancel).await.unwrap();
        assert!(!path.exists());
        assert_eq!(
            f.git
                .run(
                    &f.repo,
                    &["rev-parse", "--verify", &branch],
                    Duration::from_secs(10),
                    &cancel
                )
                .await
                .unwrap()
                .trim(),
            kept
        );
        let list = f
            .git
            .run(
                &f.repo,
                &["worktree", "list", "--porcelain", "-z"],
                Duration::from_secs(10),
                &cancel,
            )
            .await
            .unwrap();
        assert!(
            !registered_worktree_paths(&list)
                .unwrap()
                .iter()
                .any(|listed| clean(listed.clone()) == clean(path.clone()))
        );
    }
    #[tokio::test]
    async fn missing_worktree_parent_or_root_is_pruned_without_losing_result_branch() {
        for remove_root in [false, true] {
            let f = fixture().await;
            let cancel = TaskCancellation::default();
            let cfg = OrchestrationConfig::default();
            let (path, branch, _) = f
                .git
                .prepare(&f.repo, "r12-missing-parent", &cfg, &cancel)
                .await
                .unwrap();
            let kept = f.git.head(&path, &cancel).await.unwrap();
            let removed = if remove_root {
                RelayGit::root(&f.repo, &cfg)
            } else {
                path.parent().unwrap().to_owned()
            };
            fs::remove_dir_all(&removed).unwrap();
            f.git.validate_cleanup(&f.repo, &path, &cfg).unwrap();
            f.git.cleanup(&f.repo, &path, &cancel).await.unwrap();
            assert!(!removed.exists());
            assert_eq!(
                f.git
                    .run(
                        &f.repo,
                        &["rev-parse", "--verify", &branch],
                        Duration::from_secs(10),
                        &cancel,
                    )
                    .await
                    .unwrap()
                    .trim(),
                kept
            );
            let list = f
                .git
                .run(
                    &f.repo,
                    &["worktree", "list", "--porcelain", "-z"],
                    Duration::from_secs(10),
                    &cancel,
                )
                .await
                .unwrap();
            assert!(
                !registered_worktree_paths(&list)
                    .unwrap()
                    .iter()
                    .any(|listed| clean(listed.clone()) == clean(path.clone()))
            );
        }
    }
    #[tokio::test]
    async fn absent_root_still_rejects_root_itself_and_missing_sibling() {
        let f = fixture().await;
        let cfg = OrchestrationConfig::default();
        let root = RelayGit::root(&f.repo, &cfg);
        assert!(!root.exists());
        assert!(f.git.validate_cleanup(&f.repo, &root, &cfg).is_err());
        let sibling = root.with_file_name("worktrees-other").join("token/relay");
        assert!(f.git.validate_cleanup(&f.repo, &sibling, &cfg).is_err());
        f.git
            .validate_cleanup(&f.repo, &root.join("token/relay"), &cfg)
            .unwrap();
    }
    #[tokio::test]
    async fn missing_worktree_below_regular_file_is_rejected() {
        let f = fixture().await;
        let cfg = OrchestrationConfig::default();
        let root = RelayGit::root(&f.repo, &cfg);
        fs::create_dir_all(&root).unwrap();
        let file = root.join("not-a-directory");
        fs::write(&file, b"synthetic").unwrap();
        assert!(
            f.git
                .validate_cleanup(&f.repo, &file.join("relay"), &cfg)
                .is_err()
        );
    }
    #[tokio::test]
    async fn missing_worktree_outside_root_is_rejected_and_inside_root_is_allowed() {
        let f = fixture().await;
        let cfg = OrchestrationConfig::default();
        let root = RelayGit::root(&f.repo, &cfg);
        fs::create_dir_all(&root).unwrap();
        let outside = f.paths.root().join("outside-synthetic").join("relay");
        let error = f.git.validate_cleanup(&f.repo, &outside, &cfg).unwrap_err();
        assert!(error.contains("outside the configured worktree root"));
        let missing = root.join("synthetic-token").join("relay");
        f.git.validate_cleanup(&f.repo, &missing, &cfg).unwrap();
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn missing_worktree_through_outside_symlink_is_rejected() {
        use std::os::unix::fs::symlink;
        let f = fixture().await;
        let cfg = OrchestrationConfig::default();
        let root = RelayGit::root(&f.repo, &cfg);
        fs::create_dir_all(&root).unwrap();
        let outside = f.paths.root().join("outside-link-target");
        fs::create_dir(&outside).unwrap();
        symlink(&outside, root.join("linked")).unwrap();
        let missing = root.join("linked").join("relay");
        assert!(f.git.validate_cleanup(&f.repo, &missing, &cfg).is_err());
    }
    #[tokio::test]
    async fn production_cleanup_rejects_parent_traversal_before_git() {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("synthetic-repository");
        fs::create_dir(&repo).unwrap();
        let paths = RuntimePaths::production(root.path()).unwrap();
        // An accidental Git launch fails distinctly, so the exact traversal
        // error also proves rejection occurs before invoking the executable.
        let git = RelayGit::new(
            paths,
            WorktreeGit::new(
                root.path().join("synthetic-unavailable-git"),
                BTreeMap::new(),
            ),
            repo.clone(),
        );
        let cfg = OrchestrationConfig::default();
        let worktree_root = RelayGit::root(&repo, &cfg);
        fs::create_dir_all(worktree_root.join("victim")).unwrap();
        for name in ["victim", "missing"] {
            let path = worktree_root.join("link").join("..").join(name);
            let expected = "relay worktree cleanup path contains parent traversal";
            assert_eq!(
                git.validate_cleanup(&repo, &path, &cfg).unwrap_err(),
                expected
            );
            assert_eq!(
                git.cleanup(&repo, &path, &TaskCancellation::default())
                    .await
                    .unwrap_err(),
                expected
            );
        }
        assert!(worktree_root.join("victim").is_dir());
        git.validate_cleanup(&repo, &worktree_root.join("victim"), &cfg)
            .unwrap();
        git.validate_cleanup(&repo, &worktree_root.join("token/relay"), &cfg)
            .unwrap();
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn production_cleanup_rejects_windows_trailing_dot_alias_before_git() {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("synthetic-repository");
        fs::create_dir(&repo).unwrap();
        let paths = RuntimePaths::production(root.path()).unwrap();
        // An accidental Git launch fails distinctly, so the exact alias error
        // also proves rejection occurs before invoking the executable.
        let git = RelayGit::new(
            paths,
            WorktreeGit::new(
                root.path().join("synthetic-unavailable-git"),
                BTreeMap::new(),
            ),
            repo.clone(),
        );
        let cfg = OrchestrationConfig::default();
        let worktree_root = RelayGit::root(&repo, &cfg);
        let victim = worktree_root.join("victim").join("relay");
        fs::create_dir_all(&victim).unwrap();
        fs::write(victim.join("sentinel"), b"synthetic preserved").unwrap();
        let expected = "relay worktree cleanup path contains a Windows trailing-dot alias";
        for path in [
            worktree_root.join("victim."),
            worktree_root.join("victim.").join("relay"),
            worktree_root.join("victim").join("relay."),
        ] {
            assert!(
                path.to_string_lossy().contains('.'),
                "fixture lost the trailing-dot component: {}",
                path.display()
            );
            assert_eq!(
                git.validate_cleanup(&repo, &path, &cfg).unwrap_err(),
                expected
            );
            assert_eq!(
                git.cleanup(&repo, &path, &TaskCancellation::default())
                    .await
                    .unwrap_err(),
                expected
            );
        }
        assert_eq!(
            fs::read(victim.join("sentinel")).unwrap(),
            b"synthetic preserved"
        );
        git.validate_cleanup(&repo, &victim, &cfg).unwrap();
        git.validate_cleanup(&repo, &worktree_root.join("token").join("relay"), &cfg)
            .unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn production_cleanup_rejects_symlink_then_parent_traversal() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("synthetic-repository");
        let cfg = OrchestrationConfig::default();
        let worktree_root = RelayGit::root(&repo, &cfg);
        fs::create_dir_all(worktree_root.join("victim")).unwrap();
        let outside = root.path().join("outside");
        fs::create_dir_all(outside.join("directory")).unwrap();
        fs::create_dir(outside.join("victim")).unwrap();
        fs::write(outside.join("victim/sentinel"), b"synthetic preserved").unwrap();
        symlink(outside.join("directory"), worktree_root.join("link")).unwrap();
        let path = worktree_root.join("link/../victim");
        assert_eq!(
            fs::canonicalize(&path).unwrap(),
            outside.join("victim").canonicalize().unwrap()
        );
        let git = RelayGit::new(
            RuntimePaths::production(root.path()).unwrap(),
            WorktreeGit::new(
                root.path().join("synthetic-unavailable-git"),
                BTreeMap::new(),
            ),
            repo.clone(),
        );
        let expected = "relay worktree cleanup path contains parent traversal";
        assert_eq!(
            git.validate_cleanup(&repo, &path, &cfg).unwrap_err(),
            expected
        );
        assert_eq!(
            git.cleanup(&repo, &path, &TaskCancellation::default())
                .await
                .unwrap_err(),
            expected
        );
        assert_eq!(
            fs::read(outside.join("victim/sentinel")).unwrap(),
            b"synthetic preserved"
        );
        assert!(worktree_root.join("victim").is_dir());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn dangling_cleanup_links_are_not_missing_directories() {
        use std::os::unix::fs::symlink;
        let f = fixture().await;
        let cfg = OrchestrationConfig::default();
        let root = RelayGit::root(&f.repo, &cfg);
        fs::create_dir_all(&root).unwrap();
        let absent = f.paths.root().join("absent-link-target");
        let linked = root.join("linked");
        symlink(&absent, &linked).unwrap();
        assert!(f.git.validate_cleanup(&f.repo, &linked, &cfg).is_err());
        assert!(
            f.git
                .validate_cleanup(&f.repo, &linked.join("relay"), &cfg)
                .is_err()
        );
        fs::remove_file(&linked).unwrap();
        fs::remove_dir(&root).unwrap();
        symlink(&absent, &root).unwrap();
        assert!(
            f.git
                .validate_cleanup(&f.repo, &root.join("token/relay"), &cfg)
                .is_err()
        );
    }
}

#[cfg(test)]
mod trial_boundary_tests {
    use super::*;
    use std::collections::BTreeMap;
    fn fixture() -> (tempfile::TempDir, RuntimePaths, RelayGit, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let trial = root.path().join("trial");
        fs::create_dir(&trial).unwrap();
        let paths = RuntimePaths::trial(&trial, 49663, &root.path().join("installed")).unwrap();
        let repo = trial.join("repository");
        fs::create_dir(&repo).unwrap();
        let git = RelayGit::new(
            paths.clone(),
            WorktreeGit::new(trial.join("must-not-execute"), BTreeMap::new()),
            trial,
        );
        (root, paths, git, repo)
    }
    #[cfg(unix)]
    #[test]
    fn trial_git_pointers_reject_symlink_before_parent_components() {
        use std::os::unix::fs::symlink;

        for kind in ["strict gitdir", "stale gitdir", "commondir", ".git file"] {
            let (root, paths, _git, repo) = fixture();
            let trial = paths.root();
            let outside = root.path().join("outside");
            fs::create_dir_all(outside.join("deep")).unwrap();
            symlink(outside.join("deep"), trial.join("alias")).unwrap();
            let raw = trial.join("alias").join("..").join("metadata");
            let cleaned = trial.join("metadata");
            let outside_target = outside.join("metadata");
            let is_gitdir = matches!(kind, "strict gitdir" | "stale gitdir");
            if kind != "stale gitdir" {
                if is_gitdir {
                    fs::write(&cleaned, b"in-root synthetic gitdir target\n").unwrap();
                } else {
                    fs::create_dir(&cleaned).unwrap();
                }
            }
            if is_gitdir {
                fs::write(&outside_target, b"outside synthetic gitdir target\n").unwrap();
                let gitdir = repo.join(".git/worktrees/token/gitdir");
                fs::create_dir_all(gitdir.parent().unwrap()).unwrap();
                fs::write(&gitdir, format!("{}\n", raw.display())).unwrap();
            } else {
                fs::create_dir(&outside_target).unwrap();
                if kind == "commondir" {
                    fs::create_dir_all(repo.join(".git")).unwrap();
                    fs::write(repo.join(".git/commondir"), format!("{}\n", raw.display())).unwrap();
                } else {
                    fs::write(repo.join(".git"), format!("gitdir: {}\n", raw.display())).unwrap();
                }
            }
            let allow_stale = kind == "stale gitdir";
            assert!(
                super::super::trial_git::inspect(&paths, &repo, false, allow_stale).is_err(),
                "{kind} pointer must not be approved through an outside symlink before .."
            );
        }
    }
    #[test]
    fn trial_git_commondir_accepts_in_root_parent_components() {
        let (_root, paths, _git, repo) = fixture();
        let metadata = repo.join(".git");
        fs::create_dir_all(metadata.join("worktrees/token")).unwrap();
        fs::write(metadata.join("worktrees/token/commondir"), "../..\n").unwrap();
        assert!(
            super::super::trial_git::inspect(&paths, &repo, false, false).is_ok(),
            "a normal in-root worktree commondir using ../.. remains supported"
        );
    }
    #[tokio::test]
    async fn trial_gitdir_and_commondir_cannot_touch_outside_metadata() {
        for direct in [true, false] {
            let (root, _paths, git, repo) = fixture();
            let outside = root.path().join("outside");
            fs::create_dir(&outside).unwrap();
            fs::write(outside.join("HEAD"), "FORBIDDEN SYNTHETIC SENTINEL\n").unwrap();
            if direct {
                fs::write(
                    repo.join(".git"),
                    format!("gitdir: {}\n", outside.display()),
                )
                .unwrap();
            } else {
                fs::create_dir(repo.join(".git")).unwrap();
                fs::write(
                    repo.join(".git/commondir"),
                    outside.to_string_lossy().as_bytes(),
                )
                .unwrap();
            }
            let error = git
                .head(&repo, &TaskCancellation::default())
                .await
                .unwrap_err();
            assert!(!error.contains("FORBIDDEN SYNTHETIC SENTINEL"));
            assert_eq!(
                fs::read_to_string(outside.join("HEAD")).unwrap(),
                "FORBIDDEN SYNTHETIC SENTINEL\n"
            );
            assert!(!outside.join("HEAD.lock").exists());
        }
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn trial_git_metadata_symlinks_and_configuration_includes_fail_closed() {
        use std::os::unix::fs::symlink;
        let (root, _paths, git, repo) = fixture();
        let outside = root.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("sentinel"), "untouched").unwrap();
        symlink(&outside, repo.join(".git")).unwrap();
        assert!(git.head(&repo, &TaskCancellation::default()).await.is_err());
        fs::remove_file(repo.join(".git")).unwrap();
        fs::create_dir(repo.join(".git")).unwrap();
        fs::write(
            repo.join(".git/config"),
            format!("[include]\npath={}\n", outside.join("sentinel").display()),
        )
        .unwrap();
        let error = git
            .head(&repo, &TaskCancellation::default())
            .await
            .unwrap_err();
        assert!(!error.contains("untouched"));
        assert_eq!(
            fs::read_to_string(outside.join("sentinel")).unwrap(),
            "untouched"
        );
    }
    #[tokio::test]
    async fn oversized_trial_git_metadata_is_rejected_before_git_execution() {
        for name in [
            "config",
            "config.worktree",
            "commondir",
            "gitdir",
            "objects/info/alternates",
        ] {
            let (_root, paths, git, repo) = fixture();
            let metadata = repo.join(".git");
            fs::create_dir_all(metadata.join("objects/info")).unwrap();
            let cap = if matches!(name, "commondir" | "gitdir") {
                4096
            } else {
                1024 * 1024
            };
            let mut value = if name.starts_with("config") {
                b"#\n".repeat(cap / 2)
            } else {
                vec![b' '; cap]
            };
            value.extend_from_slice(b"[include]\npath = /synthetic-outside/not-read\n");
            fs::write(metadata.join(name), value).unwrap();
            let result = super::super::trial_git::inspect(&paths, &repo, false, false);
            assert!(
                result.is_err(),
                "oversized {name} must never be prefix-accepted"
            );
            let error = git
                .head(&repo, &TaskCancellation::default())
                .await
                .unwrap_err();
            assert!(
                error.contains("trial Git metadata"),
                "unexpected pre-exec error: {error}"
            );
        }
        let (_root, paths, _git, repo) = fixture();
        let metadata = paths.root().join("selected-metadata");
        fs::create_dir(&metadata).unwrap();
        let prefix = format!("gitdir: {}\n", metadata.display());
        let mut bytes = prefix.into_bytes();
        bytes.resize(4097, b' ');
        fs::write(repo.join(".git"), bytes).unwrap();
        assert!(super::super::trial_git::inspect(&paths, &repo, false, false).is_err());
    }

    #[test]
    fn trial_policy_removes_ambient_repository_and_hook_execution_overrides() {
        let (_root, paths, git, repo) = fixture();
        let mut plan = git
            .git
            .command_plan(&repo, &repo, &[], Duration::from_secs(1));
        plan.env.insert("GIT_DIR".into(), Some("outside".into()));
        plan.env
            .insert("GIT_CONFIG_PARAMETERS".into(), Some("unsafe".into()));
        let _pins = super::super::trial_git::configure(&paths, &mut plan).unwrap();
        assert_eq!(plan.env[&OsString::from("GIT_DIR")], None);
        assert_eq!(plan.env[&OsString::from("GIT_CONFIG_PARAMETERS")], None);
        assert_eq!(
            plan.env[&OsString::from("GIT_ALLOW_PROTOCOL")],
            Some("".into())
        );
        assert_eq!(
            plan.env[&OsString::from("GIT_TERMINAL_PROMPT")],
            Some("0".into())
        );
        assert_eq!(
            plan.env[&OsString::from("GIT_CONFIG_COUNT")],
            Some("2".into())
        );
        assert_eq!(
            plan.env[&OsString::from("GIT_CONFIG_VALUE_1")],
            Some("false".into())
        );
        let hooks = PathBuf::from(
            plan.env[&OsString::from("GIT_CONFIG_VALUE_0")]
                .clone()
                .unwrap(),
        );
        assert!(hooks.starts_with(crate::config::paths::equivalent_root_prefix(paths.root())));
        assert!(fs::read_dir(hooks).unwrap().next().is_none());
    }
}
