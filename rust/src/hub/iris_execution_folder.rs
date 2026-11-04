//! Bounded approved-cwd snapshot for the real bounded profile. It records only
//! relative names, entry types, sizes, link counts and SHA256 inside one folder.
//! Links and junctions are recorded, never followed. A comparison is a separate
//! local record: it is not effects verification, and writes outside the folder,
//! network use and provider-side remote jobs stay unverified.
use super::iris_execution_owner::{OwnerError, OwnerTicket, Result, fail, sha};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::File,
    io::Read,
    path::{Component, Path},
};

pub const MAX_ENTRIES: usize = 1000;
pub const MAX_TOTAL_BYTES: u64 = 16 * 1024 * 1024;
pub const PAYLOAD_ONLY: &str = "folder-payload-only";
pub const UNEXPECTED_CHANGE: &str = "folder-unexpected-change";
pub const SCOPE: &str = "approved-cwd-snapshot-only";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FolderEntry {
    pub path: String,
    pub kind: String,
    pub size: u64,
    pub links: u64,
    pub sha256: Option<String>,
}

fn is_link(meta: &std::fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}

#[cfg(windows)]
fn open_plain(path: &Path) -> Result<(File, u64)> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ,
        GetFileInformationByHandle,
    };
    let file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|_| fail("FOLDER_READ"))?;
    // Native structs are integer-only; the owned File retains the handle.
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(fail("FOLDER_READ"));
    }
    if info.dwFileAttributes & 0x400 != 0 {
        return Err(fail("FOLDER_ALIAS"));
    }
    Ok((file, u64::from(info.nNumberOfLinks)))
}

#[cfg(unix)]
fn open_plain(path: &Path) -> Result<(File, u64)> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| fail("FOLDER_READ"))?;
    let links = file.metadata().map_err(|_| fail("FOLDER_READ"))?.nlink();
    Ok((file, links))
}

#[cfg(not(any(windows, unix)))]
fn open_plain(_path: &Path) -> Result<(File, u64)> {
    Err(fail("UNSUPPORTED_PLATFORM"))
}

fn read_file(path: &Path, budget: u64) -> Result<(Vec<u8>, u64)> {
    let (mut file, links) = open_plain(path)?;
    let before = file.metadata().map_err(|_| fail("FOLDER_READ"))?;
    if !before.is_file() {
        return Err(fail("FOLDER_READ"));
    }
    if before.len() > budget {
        return Err(fail("FOLDER_LIMIT"));
    }
    let mut bytes = Vec::with_capacity(before.len() as usize);
    file.by_ref()
        .take(budget + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| fail("FOLDER_READ"))?;
    if bytes.len() as u64 > budget {
        return Err(fail("FOLDER_LIMIT"));
    }
    let after = file.metadata().map_err(|_| fail("FOLDER_READ"))?;
    if after.len() != bytes.len() as u64 || before.modified().ok() != after.modified().ok() {
        return Err(fail("FOLDER_CHANGED"));
    }
    Ok((bytes, links))
}

/// Full listing of `root`, sorted by relative path. Any entry beyond
/// MAX_ENTRIES, or file content beyond MAX_TOTAL_BYTES in total, is an error.
pub fn snapshot(root: &Path) -> Result<Vec<FolderEntry>> {
    let meta = std::fs::symlink_metadata(root).map_err(|_| fail("FOLDER_ROOT"))?;
    if !meta.is_dir() || is_link(&meta) {
        return Err(fail("FOLDER_ROOT"));
    }
    let mut entries = Vec::new();
    let mut total = 0u64;
    let mut pending = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, prefix)) = pending.pop() {
        let mut names = Vec::new();
        for item in std::fs::read_dir(&dir).map_err(|_| fail("FOLDER_READ"))? {
            let item = item.map_err(|_| fail("FOLDER_READ"))?;
            names.push(
                item.file_name()
                    .into_string()
                    .map_err(|_| fail("FOLDER_NAME"))?,
            );
            if names.len() + entries.len() > MAX_ENTRIES {
                return Err(fail("FOLDER_LIMIT"));
            }
        }
        names.sort();
        for name in names {
            if entries.len() >= MAX_ENTRIES {
                return Err(fail("FOLDER_LIMIT"));
            }
            let path = dir.join(&name);
            let relative = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            let meta = std::fs::symlink_metadata(&path).map_err(|_| fail("FOLDER_READ"))?;
            let (kind, size, links, digest) = if is_link(&meta) {
                ("link", 0, 0, None)
            } else if meta.is_dir() {
                pending.push((path, relative.clone()));
                ("dir", 0, 0, None)
            } else if meta.is_file() {
                let (bytes, links) = read_file(&path, MAX_TOTAL_BYTES - total)?;
                total += bytes.len() as u64;
                ("file", bytes.len() as u64, links, Some(sha(&bytes)))
            } else {
                ("other", 0, 0, None)
            };
            entries.push(FolderEntry {
                path: relative,
                kind: kind.into(),
                size,
                links,
                sha256: digest,
            });
        }
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}

/// Owner-store document for one observation. A failed observation is kept as
/// an incomplete record with its fixed reason code, never as an empty folder.
pub fn snapshot_document(
    ticket: &OwnerTicket,
    phase: &str,
    observed: &std::result::Result<Vec<FolderEntry>, OwnerError>,
    observed_at_ms: u64,
) -> Value {
    let (complete, reason, entries) = match observed {
        Ok(entries) => (true, Value::Null, entries.as_slice()),
        Err(error) => (false, json!(error.code), &[][..]),
    };
    let total: u64 = entries.iter().map(|e| e.size).sum();
    json!({"kind":"folder_snapshot","phase":phase,"ticket":ticket,"complete":complete,
        "reason":reason,"entries":entries,"entry_count":entries.len(),"total_bytes":total,
        "limits":{"max_entries":MAX_ENTRIES,"max_total_bytes":MAX_TOTAL_BYTES},
        "observed_at_ms":observed_at_ms,"verification_scope":SCOPE,
        "effects_verified":false,"task_outcome_verified":false,"remote_jobs_stopped":false})
}

/// The launch baseline must contain only plain single-link files and
/// directories; a link or hard link could make later writes leave the folder.
pub fn require_plain(entries: &[FolderEntry]) -> Result<()> {
    if entries
        .iter()
        .any(|e| !(e.kind == "dir" || (e.kind == "file" && e.links == 1)))
    {
        return Err(fail("FOLDER_ALIAS"));
    }
    Ok(())
}

/// Relative key of an operator-approved payload path, using `/` separators.
pub fn relative_key(path: &Path) -> Option<String> {
    let mut parts = Vec::new();
    for component in path.components() {
        let Component::Normal(name) = component else {
            return None;
        };
        parts.push(name.to_str()?.to_owned());
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// `PAYLOAD_ONLY` only when the single difference is the new creation of the
/// payload path as one plain single-link file. Everything else is unexpected.
pub fn compare(
    before: &[FolderEntry],
    after: &[FolderEntry],
    payload: &str,
) -> (&'static str, Vec<Value>) {
    let old: BTreeMap<_, _> = before.iter().map(|e| (e.path.as_str(), e)).collect();
    let new: BTreeMap<_, _> = after.iter().map(|e| (e.path.as_str(), e)).collect();
    let mut changes = Vec::new();
    for (path, entry) in &new {
        match old.get(path) {
            None => changes.push(json!({"path":path,"change":"added","kind":entry.kind})),
            Some(previous) if previous != entry => {
                changes.push(json!({"path":path,"change":"modified","kind":entry.kind}))
            }
            Some(_) => {}
        }
    }
    for (path, entry) in &old {
        if !new.contains_key(path) {
            changes.push(json!({"path":path,"change":"removed","kind":entry.kind}));
        }
    }
    let payload_only = changes.len() == 1
        && changes[0]["change"] == "added"
        && changes[0]["path"] == payload
        && new
            .get(payload)
            .is_some_and(|e| e.kind == "file" && e.links == 1);
    (
        if payload_only {
            PAYLOAD_ONLY
        } else {
            UNEXPECTED_CHANGE
        },
        changes,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_compare_and_limits_are_exact() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        std::fs::create_dir(root.join("sub")).unwrap();
        std::fs::write(root.join("sub/seed.txt"), b"synthetic seed").unwrap();
        let before = snapshot(&root).unwrap();
        require_plain(&before).unwrap();
        assert_eq!(before.len(), 2);
        std::fs::write(root.join("answer.json"), b"{}").unwrap();
        let after = snapshot(&root).unwrap();
        assert_eq!(compare(&before, &after, "answer.json").0, PAYLOAD_ONLY);
        assert_eq!(compare(&before, &after, "other.json").0, UNEXPECTED_CHANGE);
        std::fs::write(root.join("sub/seed.txt"), b"synthetic seed changed").unwrap();
        let changed = snapshot(&root).unwrap();
        let (effects, changes) = compare(&before, &changed, "answer.json");
        assert_eq!(effects, UNEXPECTED_CHANGE);
        assert_eq!(changes.len(), 2);
        std::fs::hard_link(root.join("answer.json"), root.join("alias.json")).unwrap();
        let linked = snapshot(&root).unwrap();
        assert_eq!(require_plain(&linked).unwrap_err().code, "FOLDER_ALIAS");
        assert_eq!(
            relative_key(Path::new("answer.json")).as_deref(),
            Some("answer.json")
        );
    }

    #[test]
    fn snapshot_refuses_more_than_bounded_entries_or_bytes() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        for i in 0..MAX_ENTRIES {
            std::fs::write(root.join(format!("f{i:04}")), b"").unwrap();
        }
        assert_eq!(snapshot(&root).unwrap().len(), MAX_ENTRIES);
        std::fs::write(root.join("one-more"), b"").unwrap();
        assert_eq!(snapshot(&root).unwrap_err().code, "FOLDER_LIMIT");
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        std::fs::write(
            root.join("large.bin"),
            vec![0u8; MAX_TOTAL_BYTES as usize + 1],
        )
        .unwrap();
        assert_eq!(snapshot(&root).unwrap_err().code, "FOLDER_LIMIT");
    }
}
