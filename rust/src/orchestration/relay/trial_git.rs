//! Trial-only Git metadata boundary. Production Git/config semantics are left
//! untouched. Alias/includes/alternate-object layouts are deliberately refused
//! before running Git; this is not a sandbox against a concurrently hostile OS
//! user replacing metadata after inspection.
use crate::{
    config::{Resource, RuntimePaths},
    files::safe_fs::Dir,
    process::ProcessPlan,
};
use std::{
    collections::BTreeSet,
    io,
    path::{Path, PathBuf},
};
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "trial Git metadata is outside its selected root or uses an unsupported alias/configuration",
    )
}
fn checked(paths: &RuntimePaths, path: &Path) -> io::Result<PathBuf> {
    let path = super::worktree::clean(path.to_owned());
    crate::profile::subscriptions::check_path(paths, &path)?;
    Ok(path)
}
fn git_env_path(path: &Path) -> io::Result<std::ffi::OsString> {
    let spelling = crate::config::paths::equivalent_root_prefix(path);
    #[cfg(windows)]
    if spelling != path {
        // Git for Windows cannot read Rust's verbatim spelling for these
        // environment paths. Before removing the prefix, make sure Win32's
        // normalized spelling still resolves to the exact same filesystem
        // object (for example, a verbatim-only trailing-dot name must fail
        // closed instead of aliasing its normal-name sibling).
        if std::fs::canonicalize(path)? != std::fs::canonicalize(&spelling)? {
            return Err(invalid());
        }
    }
    Ok(spelling.into_os_string())
}
fn is_path_reparse_point(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}
fn target(paths: &RuntimePaths, base: &Path, raw: &str) -> io::Result<PathBuf> {
    let value = Path::new(raw.trim());
    if value.as_os_str().is_empty() {
        return Err(invalid());
    }
    let path = if value.is_absolute() {
        value.to_owned()
    } else {
        base.join(value)
    };
    let path = crate::config::paths::native_system_path(&path).into_owned();
    let cleaned = super::worktree::clean(path.clone());
    checked(paths, &cleaned)?;
    let suffix = paths.relative_to_selected_root_preserving_parent(&path)?;
    let root = paths.root().canonicalize()?;
    let components = suffix.components().collect::<Vec<_>>();
    let mut resolved = root.clone();
    let mut missing = false;
    for (index, component) in components.iter().enumerate() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if missing || resolved == root {
                    return Err(invalid());
                }
                resolved.pop();
                if !resolved.starts_with(&root) {
                    return Err(invalid());
                }
            }
            std::path::Component::Normal(name) => {
                #[cfg(windows)]
                {
                    use std::os::windows::ffi::OsStrExt;
                    // Git later resolves these pointer strings through ordinary
                    // Win32 paths, where a trailing dot or space aliases the
                    // corresponding trimmed name. The component walk starts at
                    // a canonical verbatim root, so accepting that spelling
                    // here could inspect a different entry from the one Git
                    // will follow before a subsequent `..`.
                    if name
                        .encode_wide()
                        .last()
                        .is_some_and(|unit| matches!(unit, 0x002e | 0x0020))
                    {
                        return Err(invalid());
                    }
                }
                resolved.push(name);
                if !missing {
                    match std::fs::symlink_metadata(&resolved) {
                        Ok(metadata) => {
                            if is_path_reparse_point(&metadata)
                                || (index + 1 < components.len() && !metadata.is_dir())
                            {
                                return Err(invalid());
                            }
                        }
                        Err(error) if error.kind() == io::ErrorKind::NotFound => missing = true,
                        Err(error) => return Err(error),
                    }
                }
            }
            _ => return Err(invalid()),
        }
    }
    paths.relative_to_selected_root(&resolved)?;
    Ok(resolved)
}
fn bounded(dir: &Dir, name: &str, cap: usize) -> io::Result<Vec<u8>> {
    let bytes = dir.read(name, cap + 1)?;
    if bytes.len() > cap {
        return Err(invalid());
    }
    Ok(bytes)
}
fn optional(dir: &Dir, name: &str, cap: usize) -> io::Result<Option<Vec<u8>>> {
    match bounded(dir, name, cap) {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}
fn validate_gitdir_pointer(
    paths: &RuntimePaths,
    dir: &Dir,
    raw: &str,
    allow_stale_worktree: bool,
) -> io::Result<()> {
    let path = target(paths, dir.path(), raw)?;
    let parent_path = path.parent().ok_or_else(invalid)?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(invalid)?;
    match Dir::open(parent_path) {
        Ok(parent) => match parent.open_file(name, false) {
            Ok(_) => Ok(()),
            Err(error) if allow_stale_worktree && error.kind() == io::ErrorKind::NotFound => {
                // Git maintenance may prune a bounded in-root pointer whose
                // worktree checkout has already disappeared.
                Ok(())
            }
            Err(error) => Err(error),
        },
        Err(error) if allow_stale_worktree && error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}
fn scan(
    paths: &RuntimePaths,
    dir: &Dir,
    depth: usize,
    count: &mut usize,
    allow_stale_worktree: bool,
) -> io::Result<()> {
    if depth > 64 {
        return Err(invalid());
    }
    for name in dir.entries()? {
        *count += 1;
        if *count > 100_000 {
            return Err(invalid());
        }
        if let Ok(child) = dir.child_dir(&name, false) {
            scan(paths, &child, depth + 1, count, allow_stale_worktree)?;
            continue;
        }
        // open_file is no-follow/nonblocking and requires a regular held inode.
        dir.open_file(&name, false)?;
        if name == "commondir" || name == "gitdir" {
            let bytes = bounded(dir, &name, 4096)?;
            let raw = std::str::from_utf8(&bytes).map_err(|_| invalid())?;
            let path = target(paths, dir.path(), raw)?;
            if name == "commondir" {
                Dir::open(&path)?;
            } else {
                validate_gitdir_pointer(paths, dir, raw, allow_stale_worktree)?;
            }
        }
        if name == "config" || name == "config.worktree" {
            let data = bounded(dir, &name, 1024 * 1024)?;
            let text = String::from_utf8_lossy(&data);
            let mut core = false;
            for line in text.lines() {
                let line = line.trim().to_ascii_lowercase();
                if line.starts_with('[') {
                    if line.starts_with("[include") || line.starts_with("[filter") {
                        return Err(invalid());
                    }
                    core = line.starts_with("[core");
                } else if core
                    && matches!(
                        line.split(['=', ' ', '\t']).next(),
                        Some("worktree" | "attributesfile" | "excludesfile")
                    )
                {
                    return Err(invalid());
                }
            }
        }
        if name == "alternates"
            && dir.path().file_name().is_some_and(|name| name == "info")
            && optional(dir, &name, 1024 * 1024)?
                .is_some_and(|bytes| !bytes.iter().all(u8::is_ascii_whitespace))
        {
            return Err(invalid());
        }
    }
    Ok(())
}
/// Return held metadata roots. An in-root worktree's regular gitdir/commondir
/// pointers are supported; symlinked metadata and include-driven config are not.
pub(super) fn inspect(
    paths: &RuntimePaths,
    cwd: &Path,
    allow_init: bool,
    allow_stale_worktree: bool,
) -> io::Result<Vec<Dir>> {
    if !paths.is_trial() {
        return Ok(Vec::new());
    }
    let cwd = checked(paths, cwd)?;
    let root = paths.root().canonicalize()?;
    let mut cursor = cwd;
    let mut found = None;
    loop {
        let directory = Dir::open(&cursor)?;
        if let Ok(git) = directory.child_dir(".git", false) {
            found = Some(git);
            break;
        }
        match bounded(&directory, ".git", 4096) {
            Ok(bytes) => {
                let text = std::str::from_utf8(&bytes).map_err(|_| invalid())?;
                let raw = text.trim().strip_prefix("gitdir:").ok_or_else(invalid)?;
                let path = target(paths, &cursor, raw)?;
                found = Some(Dir::open(&path)?);
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return Err(invalid()),
        }
        if cursor.canonicalize()? == root {
            break;
        }
        let Some(parent) = cursor.parent() else { break };
        let parent = checked(paths, parent)?;
        cursor = parent;
    }
    let Some(git) = found else {
        return if allow_init {
            Ok(Vec::new())
        } else {
            Err(io::Error::new(
                io::ErrorKind::NotFound,
                "not a Git repository inside selected trial root",
            ))
        };
    };
    let mut pending = vec![git];
    let mut held = Vec::new();
    let mut visited = BTreeSet::new();
    let mut count = 0;
    while let Some(git) = pending.pop() {
        if !visited.insert(git.path().to_owned()) {
            continue;
        }
        scan(paths, &git, 0, &mut count, allow_stale_worktree)?;
        if let Some(bytes) = optional(&git, "commondir", 4096)? {
            let raw = std::str::from_utf8(&bytes).map_err(|_| invalid())?;
            pending.push(Dir::open(&target(paths, git.path(), raw)?)?);
        }
        if let Some(bytes) = optional(&git, "gitdir", 4096)? {
            let raw = std::str::from_utf8(&bytes).map_err(|_| invalid())?;
            validate_gitdir_pointer(paths, &git, raw, allow_stale_worktree)?;
        }
        held.push(git);
    }
    Ok(held)
}
/// Remove ambient Git repository overrides and disable execution hooks/fsmonitor
/// in trial operations only. Config files used here are private, empty files.
pub(super) fn configure(paths: &RuntimePaths, plan: &mut ProcessPlan) -> io::Result<Vec<Dir>> {
    if !paths.is_trial() {
        return Ok(Vec::new());
    }
    let policy = paths.resource(Resource::Temporary).join("relay-git-policy");
    checked(paths, &policy)?;
    let directory = Dir::open_or_create_private(&policy)?;
    match directory.create_new("empty-config", b"", 0o600) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            if !directory.read("empty-config", 1)?.is_empty() {
                return Err(invalid());
            }
        }
        Err(error) => return Err(error),
    }
    let hooks = directory.child_dir("hooks", true)?;
    if !hooks.entries()?.is_empty() {
        return Err(invalid());
    }
    for key in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_COMMON_DIR",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_CONFIG",
        "GIT_CONFIG_PARAMETERS",
        "GIT_NAMESPACE",
    ] {
        plan.env.insert(key.into(), None);
    }
    for (key, value) in [
        ("GIT_CONFIG_NOSYSTEM", "1"),
        ("GIT_ALLOW_PROTOCOL", ""),
        ("GIT_NO_LAZY_FETCH", "1"),
        ("GIT_TERMINAL_PROMPT", "0"),
        ("GIT_CONFIG_COUNT", "2"),
        ("GIT_CONFIG_KEY_0", "core.hooksPath"),
        ("GIT_CONFIG_KEY_1", "core.fsmonitor"),
        ("GIT_CONFIG_VALUE_1", "false"),
    ] {
        plan.env.insert(key.into(), Some(value.into()));
    }
    plan.env.insert(
        "GIT_CONFIG_GLOBAL".into(),
        Some(git_env_path(&policy.join("empty-config"))?),
    );
    plan.env.insert(
        "GIT_CONFIG_VALUE_0".into(),
        Some(git_env_path(hooks.path())?),
    );
    plan.env.insert(
        "GIT_CEILING_DIRECTORIES".into(),
        Some(git_env_path(paths.root())?),
    );
    Ok(vec![directory, hooks])
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn git_env_path_rejects_verbatim_only_trailing_dot_aliases() {
        let temp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(temp.path()).unwrap();
        let ordinary = root.join("trial");
        let verbatim_only = root.join("trial.");
        std::fs::create_dir(&ordinary).unwrap();
        std::fs::create_dir(&verbatim_only).unwrap();

        let normalized = crate::config::paths::equivalent_root_prefix(&verbatim_only);
        assert_ne!(
            std::fs::canonicalize(&verbatim_only).unwrap(),
            std::fs::canonicalize(&normalized).unwrap()
        );
        assert!(git_env_path(&ordinary).is_ok());
        assert!(git_env_path(&verbatim_only).is_err());

        std::fs::remove_dir(&verbatim_only).unwrap();
    }

    #[test]
    fn target_rejects_win32_trailing_dot_and_space_before_parent_components() {
        let temp = tempfile::tempdir().unwrap();
        let trial = temp.path().join("trial");
        let installed = temp.path().join("installed");
        std::fs::create_dir(&trial).unwrap();
        std::fs::create_dir(trial.join("ordinary")).unwrap();
        let paths = RuntimePaths::trial(&trial, 49663, &installed).unwrap();

        for component in ["alias.", "alias "] {
            // RuntimePaths::root is canonical/verbatim on Windows, allowing
            // the filesystem to create the literal name that ordinary Win32
            // spelling would trim to `alias`.
            let literal_alias = paths.root().join(component);
            std::fs::create_dir(&literal_alias).unwrap();
            assert!(std::fs::symlink_metadata(&literal_alias).unwrap().is_dir());
            let raw = trial
                .join(component)
                .join("..")
                .join("metadata")
                .to_string_lossy()
                .into_owned();
            assert!(
                target(&paths, &trial, &raw).is_err(),
                "a Win32 trailing alias before .. must be rejected: {raw}"
            );
            std::fs::remove_dir(&literal_alias).unwrap();
        }

        let ordinary = trial
            .join("ordinary")
            .join("..")
            .join("metadata")
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            target(&paths, &trial, &ordinary).unwrap(),
            paths.root().join("metadata"),
            "ordinary in-root parent components remain supported"
        );
    }
}
