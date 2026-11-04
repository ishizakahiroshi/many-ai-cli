//! Bounded approved-cwd snapshot for the real bounded profile. It records only
//! relative names, entry types, sizes, link counts, native attributes and
//! SHA256 inside one folder, including the folder's own attributes (the entry
//! with the empty relative path) and each named (alternate) data stream.
//! Access control lists are not recorded, so an ACL-only change is not seen.
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
    /// Native file attributes (Windows: hidden, read-only, ...; Unix: mode).
    /// Older records without it compare as changed, never as equal.
    #[serde(default)]
    pub attributes: u32,
}

#[cfg(windows)]
fn attributes(meta: &std::fs::Metadata) -> u32 {
    use std::os::windows::fs::MetadataExt;
    meta.file_attributes()
}

#[cfg(unix)]
fn attributes(meta: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode()
}

#[cfg(not(any(windows, unix)))]
fn attributes(_meta: &std::fs::Metadata) -> u32 {
    0
}

/// Named data streams of a plain file or directory, excluding the unnamed
/// `::$DATA` stream. Each name keeps its `:name:$type` suffix. Any enumeration
/// failure is an error so the caller fails closed.
#[cfg(windows)]
fn streams(path: &Path) -> Result<Vec<(String, u64)>> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        Foundation::{ERROR_HANDLE_EOF, GetLastError, HANDLE, INVALID_HANDLE_VALUE},
        Storage::FileSystem::{
            FindClose, FindFirstStreamW, FindNextStreamW, FindStreamInfoStandard,
            WIN32_FIND_STREAM_DATA,
        },
    };
    struct Find(HANDLE);
    impl Drop for Find {
        fn drop(&mut self) {
            unsafe { FindClose(self.0) };
        }
    }
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // Native struct is integer-only; zero is a valid initial value.
    let mut data: WIN32_FIND_STREAM_DATA = unsafe { std::mem::zeroed() };
    let handle = unsafe {
        FindFirstStreamW(
            wide.as_ptr(),
            FindStreamInfoStandard,
            (&mut data as *mut WIN32_FIND_STREAM_DATA).cast(),
            0,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        // A directory without named streams reports end-of-list at once.
        return if unsafe { GetLastError() } == ERROR_HANDLE_EOF {
            Ok(Vec::new())
        } else {
            Err(fail("FOLDER_STREAMS"))
        };
    }
    let find = Find(handle);
    let mut out = Vec::new();
    loop {
        let length = data
            .cStreamName
            .iter()
            .position(|c| *c == 0)
            .ok_or_else(|| fail("FOLDER_STREAMS"))?;
        let name =
            String::from_utf16(&data.cStreamName[..length]).map_err(|_| fail("FOLDER_NAME"))?;
        let size = u64::try_from(data.StreamSize).map_err(|_| fail("FOLDER_STREAMS"))?;
        if !name.starts_with(':') || name.len() < 2 {
            return Err(fail("FOLDER_STREAMS"));
        }
        if name != "::$DATA" {
            out.push((name, size));
            if out.len() > MAX_ENTRIES {
                return Err(fail("FOLDER_LIMIT"));
            }
        }
        if unsafe { FindNextStreamW(find.0, (&mut data as *mut WIN32_FIND_STREAM_DATA).cast()) }
            == 0
        {
            if unsafe { GetLastError() } == ERROR_HANDLE_EOF {
                break;
            }
            return Err(fail("FOLDER_STREAMS"));
        }
    }
    Ok(out)
}

#[cfg(not(windows))]
fn streams(_path: &Path) -> Result<Vec<(String, u64)>> {
    Ok(Vec::new())
}

/// Reads one named stream completely; its length must equal the enumerated
/// size, otherwise the folder is treated as changed while observed.
#[cfg(windows)]
fn read_stream(path: &Path, name: &str, size: u64, budget: u64) -> Result<Vec<u8>> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ};
    if size > budget {
        return Err(fail("FOLDER_LIMIT"));
    }
    let mut target = path.as_os_str().to_owned();
    target.push(name);
    let file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(Path::new(&target))
        .map_err(|_| fail("FOLDER_STREAMS"))?;
    let mut bytes = Vec::new();
    file.take(budget + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| fail("FOLDER_STREAMS"))?;
    if bytes.len() as u64 > budget {
        return Err(fail("FOLDER_LIMIT"));
    }
    if bytes.len() as u64 != size {
        return Err(fail("FOLDER_CHANGED"));
    }
    Ok(bytes)
}

#[cfg(not(windows))]
fn read_stream(_path: &Path, _name: &str, _size: u64, _budget: u64) -> Result<Vec<u8>> {
    Err(fail("FOLDER_STREAMS"))
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

fn push_streams(
    path: &Path,
    relative: &str,
    entries: &mut Vec<FolderEntry>,
    total: &mut u64,
) -> Result<()> {
    for (name, size) in streams(path)? {
        if entries.len() >= MAX_ENTRIES {
            return Err(fail("FOLDER_LIMIT"));
        }
        let bytes = read_stream(path, &name, size, MAX_TOTAL_BYTES - *total)?;
        *total += size;
        entries.push(FolderEntry {
            path: format!("{relative}{name}"),
            kind: "stream".into(),
            size,
            links: 0,
            sha256: Some(sha(&bytes)),
            attributes: 0,
        });
    }
    Ok(())
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
    // The approved folder itself is the entry with the empty relative path, so
    // a change of its own attributes (hidden, read-only, ...) is a difference.
    // It counts toward MAX_ENTRIES like every other entry. Access control lists
    // (of the folder or anything in it) are not recorded and so not compared.
    entries.push(FolderEntry {
        path: String::new(),
        kind: "dir".into(),
        size: 0,
        links: 0,
        sha256: None,
        attributes: attributes(&meta),
    });
    // Named streams of the folder itself are keyed ":name:$DATA".
    push_streams(root, "", &mut entries, &mut total)?;
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
            let plain = !is_link(&meta) && (meta.is_dir() || meta.is_file());
            let (kind, size, links, digest) = if is_link(&meta) {
                ("link", 0, 0, None)
            } else if meta.is_dir() {
                pending.push((path.clone(), relative.clone()));
                ("dir", 0, 0, None)
            } else if meta.is_file() {
                let (bytes, links) = read_file(&path, MAX_TOTAL_BYTES - total)?;
                total += bytes.len() as u64;
                ("file", bytes.len() as u64, links, Some(sha(&bytes)))
            } else {
                ("other", 0, 0, None)
            };
            entries.push(FolderEntry {
                path: relative.clone(),
                kind: kind.into(),
                size,
                links,
                sha256: digest,
                attributes: attributes(&meta),
            });
            if plain {
                // Alternate data streams leave names, main-stream size and
                // hash unchanged; each one is its own compared entry.
                push_streams(&path, &relative, &mut entries, &mut total)?;
            }
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
        .any(|e| !(e.kind == "dir" || e.kind == "stream" || (e.kind == "file" && e.links == 1)))
    {
        return Err(fail("FOLDER_ALIAS"));
    }
    Ok(())
}

/// True only when the after snapshot holds the payload as a plain file whose
/// SHA256 equals the bytes the collector later read for the envelope. A
/// missing or failed snapshot, or any other digest, means the payload changed.
pub fn payload_matches(
    after: &std::result::Result<Vec<FolderEntry>, OwnerError>,
    payload: &str,
    sha256: &str,
) -> bool {
    after.as_ref().is_ok_and(|entries| {
        entries
            .iter()
            .any(|e| e.path == payload && e.kind == "file" && e.sha256.as_deref() == Some(sha256))
    })
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
        // The folder itself (empty path), the subfolder and its file.
        assert_eq!(before.len(), 3);
        assert_eq!(
            (before[0].path.as_str(), before[0].kind.as_str()),
            ("", "dir")
        );
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

    #[cfg(windows)]
    #[test]
    fn alternate_streams_and_attributes_are_compared() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        std::fs::create_dir(root.join("sub")).unwrap();
        std::fs::write(root.join("seed.txt"), b"synthetic seed").unwrap();
        let before = snapshot(&root).unwrap();
        require_plain(&before).unwrap();
        assert!(before.iter().all(|e| e.kind != "stream"));
        for (name, stream) in [
            ("file", "seed.txt:hidden"),
            ("dir", "sub:dirstream"),
            ("root", ":rootstream"),
        ] {
            let d = tempfile::tempdir().unwrap();
            let root = d.path().canonicalize().unwrap();
            std::fs::create_dir(root.join("sub")).unwrap();
            std::fs::write(root.join("seed.txt"), b"synthetic seed").unwrap();
            let before = snapshot(&root).unwrap();
            std::fs::write(root.join("answer.json"), b"{}").unwrap();
            let mut target = root.as_os_str().to_owned();
            target.push(if name == "root" { "" } else { "\\" });
            target.push(stream);
            std::fs::write(Path::new(&target), b"synthetic stream").unwrap();
            let after = snapshot(&root).unwrap();
            let added: Vec<_> = after.iter().filter(|e| e.kind == "stream").collect();
            assert_eq!(added.len(), 1, "{name}");
            assert_eq!(added[0].size, 16, "{name}");
            // A pre-existing stream is a plain, comparable baseline entry.
            require_plain(&after).unwrap();
            let (effects, changes) = compare(&before, &after, "answer.json");
            assert_eq!(effects, UNEXPECTED_CHANGE, "{name}");
            assert!(
                changes.contains(
                    &json!({"path":format!("{stream}:$DATA"),"change":"added","kind":"stream"})
                ),
                "{name}: {changes:?}"
            );
            // NTFS also flips the directory's own attributes when a stream is
            // added to it; that is recorded as a modification of "sub", or of
            // the approved folder's own entry (empty path) for the root.
            let expected = if name == "file" { 2 } else { 3 };
            assert_eq!(changes.len(), expected, "{name}: {changes:?}");
        }
        // Rewriting a stream with the same size is still a modification.
        let mut seed = root.as_os_str().to_owned();
        seed.push("\\seed.txt:hidden");
        std::fs::write(Path::new(&seed), b"one").unwrap();
        let one = snapshot(&root).unwrap();
        std::fs::write(Path::new(&seed), b"two").unwrap();
        let two = snapshot(&root).unwrap();
        assert_eq!(compare(&one, &two, "answer.json").1.len(), 1);
        // Attributes alone (hidden / read-only) are a modification.
        let status = std::process::Command::new("attrib")
            .arg("+H")
            .arg(root.join("seed.txt"))
            .status()
            .unwrap();
        assert!(status.success());
        let hidden = snapshot(&root).unwrap();
        let (effects, changes) = compare(&two, &hidden, "answer.json");
        assert_eq!(effects, UNEXPECTED_CHANGE);
        assert_eq!(
            changes,
            vec![json!({"path":"seed.txt","change":"modified","kind":"file"})]
        );
    }

    #[test]
    fn the_approved_folders_own_attributes_are_part_of_the_snapshot() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("work");
        std::fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let before = snapshot(&root).unwrap();
        assert_eq!(
            (before[0].path.as_str(), before[0].kind.as_str()),
            ("", "dir")
        );
        // Creating the payload alone leaves the folder's own entry unchanged.
        std::fs::write(root.join("answer.json"), b"{}").unwrap();
        let created = snapshot(&root).unwrap();
        assert_eq!(compare(&before, &created, "answer.json").0, PAYLOAD_ONLY);
        // Any change of the folder's own attributes is a difference, with the
        // payload created in the same window.
        let original = std::fs::metadata(&root).unwrap().permissions();
        let mut changed = original.clone();
        changed.set_readonly(true);
        std::fs::set_permissions(&root, changed).unwrap();
        let after = snapshot(&root);
        std::fs::set_permissions(&root, original).unwrap();
        let (effects, changes) = compare(&before, &after.unwrap(), "answer.json");
        assert_eq!(effects, UNEXPECTED_CHANGE);
        assert!(
            changes.contains(&json!({"path":"","change":"modified","kind":"dir"})),
            "{changes:?}"
        );
    }

    #[test]
    fn payload_digest_must_match_the_after_snapshot() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        std::fs::write(root.join("answer.json"), b"{\"a\":1}").unwrap();
        let after = snapshot(&root);
        let digest = sha(b"{\"a\":1}");
        assert!(payload_matches(&after, "answer.json", &digest));
        assert!(!payload_matches(&after, "answer.json", &sha(b"{\"a\":2}")));
        assert!(!payload_matches(&after, "other.json", &digest));
        assert!(!payload_matches(
            &Err(fail("FOLDER_READ")),
            "answer.json",
            &digest
        ));
    }

    #[test]
    fn snapshot_refuses_more_than_bounded_entries_or_bytes() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        // The folder's own entry counts, so MAX_ENTRIES - 1 files still fit.
        for i in 0..MAX_ENTRIES - 1 {
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
