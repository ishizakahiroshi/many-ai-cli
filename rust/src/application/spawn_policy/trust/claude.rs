use super::{FolderTrustResult, project::Plan};
use crate::{config::RuntimePaths, files::safe_fs::Dir, profile::subscriptions::check_path};
use serde_json::value::RawValue;
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};
type Object = BTreeMap<String, Box<RawValue>>;
const DEFAULT_ENTRY: &str = r#"{"allowedTools":[],"mcpContextUris":[],"mcpServers":{},"enabledMcpjsonServers":[],"disabledMcpjsonServers":[],"hasTrustDialogAccepted":true,"hasClaudeMdExternalIncludesApproved":false,"hasClaudeMdExternalIncludesWarningShown":false}"#;
const LIMIT: usize = 16 * 1024 * 1024;
fn error() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Claude folder trust configuration is invalid",
    )
}
fn read(path: &Path) -> io::Result<Object> {
    use std::io::Read;
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Object::new()),
        Err(e) => return Err(e),
    };
    if !file.metadata()?.is_file() {
        return Err(error());
    }
    let mut bytes = vec![];
    file.take((LIMIT + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.len() > LIMIT {
        return Err(error());
    }
    serde_json::from_slice::<Option<Object>>(&bytes)
        .map(Option::unwrap_or_default)
        .map_err(|_| error())
}
fn projects(root: &Object) -> io::Result<Object> {
    root.get("projects")
        .map(|raw| {
            serde_json::from_str::<Option<Object>>(raw.get())
                .map(Option::unwrap_or_default)
                .map_err(|_| error())
        })
        .unwrap_or_else(|| Ok(Object::new()))
}
fn state(raw: &RawValue) -> &'static str {
    #[derive(serde::Deserialize)]
    struct Flags {
        #[serde(rename = "hasTrustDialogAccepted")]
        accepted: Option<bool>,
    }
    match serde_json::from_str::<Flags>(raw.get())
        .ok()
        .and_then(|f| f.accepted)
    {
        Some(true) => "trusted",
        Some(false) => "untrusted",
        None => "other",
    }
}
struct Lock {
    path: PathBuf,
}
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.path);
    }
}
fn lock(path: &Path) -> io::Result<Lock> {
    let path = PathBuf::from(format!("{}.lock", path.to_string_lossy()));
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut reclaimed = false;
    loop {
        let builder = fs::DirBuilder::new();
        #[cfg(unix)]
        let builder = {
            use std::os::unix::fs::DirBuilderExt;
            let mut builder = builder;
            builder.mode(0o700);
            builder
        };
        match builder.create(&path) {
            Ok(()) => return Ok(Lock { path }),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
        if !reclaimed
            && fs::metadata(&path)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| SystemTime::now().duration_since(t).ok())
                .is_some_and(|age| age > Duration::from_secs(10))
        {
            let _ = fs::remove_dir(&path);
            reclaimed = true;
            continue;
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Claude folder trust lock timed out",
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
pub(super) fn grant(
    path: &Path,
    plan: &Plan,
    paths: &RuntimePaths,
) -> io::Result<FolderTrustResult> {
    let result = {
        let _lock = lock(path)?;
        let mut root = read(path)?;
        let mut projects = projects(&root)?;
        for key in &plan.lookup {
            if projects
                .get(key)
                .is_some_and(|value| state(value) == "trusted")
            {
                return Ok(FolderTrustResult {
                    key: key.clone(),
                    existing: "trusted".into(),
                    ..Default::default()
                });
            }
        }
        let key = &plan.write_key;
        if let Some(entry) = projects.get(key) {
            if state(entry) != "untrusted" {
                return Ok(FolderTrustResult {
                    key: key.clone(),
                    existing: state(entry).into(),
                    ..Default::default()
                });
            }
            let mut entry = serde_json::from_str::<Object>(entry.get()).map_err(|_| error())?;
            entry.insert(
                "hasTrustDialogAccepted".into(),
                RawValue::from_string("true".into()).map_err(|_| error())?,
            );
            projects.insert(
                key.clone(),
                serde_json::value::to_raw_value(&entry).map_err(|_| error())?,
            );
        } else {
            projects.insert(
                key.clone(),
                RawValue::from_string(DEFAULT_ENTRY.into()).map_err(|_| error())?,
            );
        }
        root.insert(
            "projects".into(),
            serde_json::value::to_raw_value(&projects).map_err(|_| error())?,
        );
        write(path, &root, paths)?;
        FolderTrustResult {
            written: true,
            key: key.clone(),
            ..Default::default()
        }
    };
    // Verify after dropping the CLI-compatible lock, as the Go source does.
    if projects(&read(path)?)?
        .get(&plan.write_key)
        .is_none_or(|value| state(value) != "trusted")
    {
        return Err(io::Error::other("Claude folder trust verification failed"));
    }
    Ok(result)
}
fn write(path: &Path, root: &Object, paths: &RuntimePaths) -> io::Result<()> {
    let target = path.canonicalize().unwrap_or_else(|_| path.into());
    check_path(paths, &target)?;
    reclaim(&target);
    let body = serde_json::to_vec_pretty(root).map_err(|_| error())?;
    let parent = target.parent().ok_or_else(error)?;
    let directory = Dir::open(parent)?;
    let name = target
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(error)?;
    let temporary = format!(
        "{name}.many-ai-cli-{}-{}.tmp",
        std::process::id(),
        &crate::process::random_token()?[..16]
    );
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(&target)
            .map(|m| m.permissions().mode() & 0o777)
            .unwrap_or(0o600)
    };
    #[cfg(not(unix))]
    let mode = 0o600;
    directory.create_new(&temporary, &body, mode)?;
    let mut result = Err(io::Error::other(
        "Claude folder trust atomic replacement failed",
    ));
    for _ in 0..5 {
        result = rename(&directory, &temporary, name);
        if result.is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = directory.remove_file(&temporary);
    result
}
fn rename(directory: &Dir, source: &str, target: &str) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::{ffi::CString, os::fd::AsRawFd};
        let handle = directory.try_clone_file()?;
        let source = CString::new(source).map_err(|_| error())?;
        let target = CString::new(target).map_err(|_| error())?;
        // SAFETY: same pinned directory and validated single basenames.
        if unsafe {
            libc::renameat(
                handle.as_raw_fd(),
                source.as_ptr(),
                handle.as_raw_fd(),
                target.as_ptr(),
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        handle.sync_all()
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        };
        let source = directory
            .path()
            .join(source)
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let target = directory
            .path()
            .join(target)
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        // SAFETY: terminated paths beneath the held directory capability.
        if unsafe {
            MoveFileExW(
                source.as_ptr(),
                target.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (directory, source, target);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "native Claude atomic trust write is unavailable",
        ))
    }
}
pub(super) fn reclaim(target: &Path) -> (usize, usize) {
    let Some(parent) = target.parent() else {
        return (0, 1);
    };
    let Some(name) = target.file_name().and_then(|s| s.to_str()) else {
        return (0, 1);
    };
    let directory = match Dir::open(parent) {
        Ok(dir) => dir,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return (0, 0),
        Err(_) => return (0, 1),
    };
    let Ok(entries) = directory.entries() else {
        return (0, 1);
    };
    let mut result = (0, 0);
    let prefix = format!("{name}.many-ai-cli-");
    for name in entries {
        if !name.starts_with(&prefix) || !name.ends_with(".tmp") {
            continue;
        }
        let Ok(metadata) = directory.metadata(&name) else {
            continue;
        };
        if !metadata.is_file()
            || metadata
                .modified()
                .ok()
                .and_then(|t| SystemTime::now().duration_since(t).ok())
                .is_none_or(|age| age <= Duration::from_secs(60))
        {
            continue;
        }
        match directory.remove_file(&name) {
            Ok(()) => result.0 += 1,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(_) => result.1 += 1,
        }
    }
    result
}
