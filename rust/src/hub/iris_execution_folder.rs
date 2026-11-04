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

/// True only for a snapshot of an empty folder: the folder itself and nothing
/// else, so no file, subfolder or named stream (including the folder's own).
/// Emptiness is required, not a list of forbidden names: a folder about to be
/// trusted can carry no project settings, hooks or server definitions.
pub fn only_the_folder(entries: &[FolderEntry]) -> bool {
    matches!(entries, [only] if only.path.is_empty() && only.kind == "dir")
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

/// Real project effects (`folder-diff` collector). `UNEXPECTED_CHANGE` above is
/// shared; the real bounded vocabulary is unchanged.
pub const NO_CHANGE: &str = "folder-no-change";
pub const SCOPED_CHANGE: &str = "folder-scoped-change";
/// Upper bound of the full change list artifact (JSON bytes).
pub const MAX_CHANGES_BYTES: u64 = 1024 * 1024;

/// Names that must never exist in a real project work folder and whose
/// creation is always unexpected: VCS metadata and project-level provider
/// settings or server definitions. Compared without letter case.
pub fn protected_name(name: &str) -> bool {
    [".git", ".claude", ".mcp.json"]
        .iter()
        .any(|p| name.eq_ignore_ascii_case(p))
}

/// The name Win32 path normalization resolves: trailing dots and spaces are
/// dropped, so `.git.` or `.claude ` created through a verbatim path is opened
/// as `.git` or `.claude` by an ordinary tool (for example a restore copy).
fn win32_name(name: &str) -> &str {
    name.trim_end_matches(['.', ' '])
}

/// Windows device names, with or without an extension (`NUL.txt`), in any
/// letter case. COM0/LPT0 and the superscript digits are included as well.
fn device_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).trim_end_matches(' ');
    let upper = stem.to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            upper.strip_prefix(prefix).is_some_and(|digit| {
                matches!(
                    digit,
                    "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        })
}

/// Every path component; a stream entry is "<file>:<name>:$DATA", whose file
/// part is the component checked.
fn components(path: &str) -> impl Iterator<Item = &str> {
    path.split('/')
        .map(|part| part.split(':').next().unwrap_or(part))
}

fn has_protected_component(path: &str) -> bool {
    components(path).any(|part| protected_name(part) || protected_name(win32_name(part)))
}

/// An 8.3 short name form: a `~` directly followed by one or more digits that
/// end the base name (at the end or before a `.`), as in `GIT~1`, `CLAUDE~1`,
/// `PROGRA~2` or `MCP~1.JSO`. Windows may resolve such a name to a different
/// entry (`GIT~1` to `.git`), so it is never a work folder name. Ordinary
/// names with a `~` not followed by digits (`a~b.py`) are not affected.
fn short_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    bytes.iter().enumerate().any(|(index, byte)| {
        if *byte != b'~' {
            return false;
        }
        let digits = bytes[index + 1..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        let end = index + 1 + digits;
        index > 0 && digits > 0 && (end == bytes.len() || bytes[end] == b'.')
    })
}

/// A component Windows does not keep as written (a trailing dot or space), a
/// device name or an 8.3 short name form: never a plain file or folder name
/// of a work folder.
fn has_unsafe_component(path: &str) -> bool {
    components(path).any(|part| {
        !part.is_empty() && (part.ends_with(['.', ' ']) || device_name(part) || short_name(part))
    })
}

/// NTFS sets the archive attribute (0x20) on every write, and a copy may clear
/// it, so it alone is never a difference of a real project folder. Windows
/// reports `FILE_ATTRIBUTE_NORMAL` (0x80) exactly when no other attribute is
/// set, so a file without the archive bit reads 0x80 and the same file after
/// a write 0x20: both mean "no attribute" here. Unix modes are compared as
/// they are.
fn same_attributes(a: &FolderEntry, b: &FolderEntry) -> bool {
    #[cfg(windows)]
    const IGNORED: u32 = 0x20 | 0x80;
    #[cfg(not(windows))]
    const IGNORED: u32 = 0;
    a.attributes & !IGNORED == b.attributes & !IGNORED
}

fn same_entry(a: &FolderEntry, b: &FolderEntry) -> bool {
    a.path == b.path
        && a.kind == b.kind
        && a.size == b.size
        && a.links == b.links
        && a.sha256 == b.sha256
        && same_attributes(a, b)
}

/// SHA256 of the canonical baseline text of a snapshot: one line per entry in
/// snapshot order (sorted by relative path, the folder itself first with the
/// empty path), `kind TAB size TAB sha256-or-dash TAB path LF`, UTF-8. Native
/// attributes and link counts are not part of the digest (links are checked
/// separately). A name with a control character has no digest.
pub fn baseline_digest(entries: &[FolderEntry]) -> Option<String> {
    let mut text = String::new();
    for e in entries {
        if e.path.chars().any(char::is_control) {
            return None;
        }
        text.push_str(&format!(
            "{}\t{}\t{}\t{}\n",
            e.kind,
            e.size,
            e.sha256.as_deref().unwrap_or("-"),
            e.path
        ));
    }
    Some(sha(text.as_bytes()))
}

/// A real project launch baseline: plain single-link files and directories
/// only, no named stream (including on the folder itself), no protected name
/// anywhere (also after dropping trailing dots and spaces) and no name with a
/// trailing dot or space, a Windows device name or an 8.3 short name form
/// (`GIT~1`). It must equal the operator-pinned digest and entry count.
pub fn check_project_baseline(
    entries: &[FolderEntry],
    baseline_sha256: &str,
    baseline_entry_count: u64,
) -> Result<()> {
    if entries
        .iter()
        .any(|e| !(e.kind == "dir" || (e.kind == "file" && e.links == 1)))
    {
        return Err(fail("FOLDER_ALIAS"));
    }
    if entries.iter().any(|e| has_protected_component(&e.path)) {
        return Err(fail("FOLDER_PROTECTED_NAME"));
    }
    if entries.iter().any(|e| has_unsafe_component(&e.path)) {
        return Err(fail("FOLDER_UNSAFE_NAME"));
    }
    if entries.len() as u64 != baseline_entry_count
        || baseline_digest(entries).as_deref() != Some(baseline_sha256)
    {
        return Err(fail("FOLDER_BASELINE_MISMATCH"));
    }
    Ok(())
}

/// Instruction files Claude reads from the folders above its working folder.
pub const ANCESTOR_INSTRUCTION_NAMES: [&str; 4] =
    ["CLAUDE.md", "CLAUDE.local.md", ".claude", "AGENTS.md"];

/// A real project launch refuses when any folder above the work folder, up
/// to and including the drive root, holds one of
/// `ANCESTOR_INSTRUCTION_NAMES` (any entry kind; Windows compares names
/// without letter case). The work folder itself is not checked here: its
/// content is the pinned baseline.
pub fn check_no_ancestor_instructions(folder: &std::path::Path) -> Result<()> {
    let mut current = folder.parent();
    while let Some(dir) = current {
        for name in ANCESTOR_INSTRUCTION_NAMES {
            match std::fs::symlink_metadata(dir.join(name)) {
                Ok(_) => return Err(fail("FOLDER_ANCESTOR_INSTRUCTIONS")),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                // An unreadable ancestor is not proven free of instructions.
                Err(_) => return Err(fail("FOLDER_ANCESTOR_INSTRUCTIONS")),
            }
        }
        current = dir.parent();
    }
    Ok(())
}

fn side(entry: Option<&&FolderEntry>) -> (Value, Value) {
    match entry {
        Some(e) => (json!(e.size), json!(e.sha256)),
        None => (Value::Null, Value::Null),
    }
}

/// Every difference between two snapshots, sorted by path: path, change
/// (`added`, `modified` or `removed`), kind (the after kind, or the before
/// kind for a removal), and the before and after size and SHA256.
pub fn change_records(before: &[FolderEntry], after: &[FolderEntry]) -> Vec<Value> {
    let old: BTreeMap<_, _> = before.iter().map(|e| (e.path.as_str(), e)).collect();
    let new: BTreeMap<_, _> = after.iter().map(|e| (e.path.as_str(), e)).collect();
    let mut paths: Vec<&str> = old.keys().chain(new.keys()).copied().collect();
    paths.sort_unstable();
    paths.dedup();
    let mut out = Vec::new();
    for path in paths {
        let (o, n) = (old.get(path), new.get(path));
        let change = match (o, n) {
            (None, Some(_)) => "added",
            (Some(_), None) => "removed",
            (Some(a), Some(b)) if !same_entry(a, b) => "modified",
            _ => continue,
        };
        let kind = n.or(o).map(|e| e.kind.clone()).unwrap_or_default();
        let (before_size, before_sha256) = side(o);
        let (after_size, after_sha256) = side(n);
        out.push(json!({"path":path,"change":change,"kind":kind,
            "before_size":before_size,"before_sha256":before_sha256,
            "after_size":after_size,"after_sha256":after_sha256}));
    }
    out
}

/// `.` is the whole folder; `a` and `a/` both mean `a` itself and everything
/// below it, at a `/` boundary only (so `src` never covers `src2.py`).
fn in_scope(path: &str, allowed: &[String]) -> bool {
    allowed.iter().any(|a| {
        let a = a.strip_suffix('/').unwrap_or(a);
        a == "." || path == a || path.strip_prefix(a).is_some_and(|r| r.starts_with('/'))
    })
}

/// Effects of a real project run. `NO_CHANGE` when nothing differs;
/// `SCOPED_CHANGE` only when every difference is the addition, modification
/// (content only) or removal of a plain single-link file, or the addition or
/// removal of a plain folder, under an allowed path; otherwise
/// `UNEXPECTED_CHANGE` with the first reason code in path order. A plain
/// folder is a `dir` entry, which is never a link, junction or other reparse
/// point (those are `link`). Links, streams, other entry kinds, kind changes,
/// attribute changes (of files or folders), hard links, protected names (also
/// after Windows drops trailing dots and spaces: `PROTECTED_NAME`) and names
/// with a trailing dot or space, a Windows device name or an 8.3 short name
/// form (`UNSAFE_NAME`) are
/// never scoped. The archive attribute alone is not a difference (Windows).
pub fn classify(
    before: &[FolderEntry],
    after: &[FolderEntry],
    allowed: &[String],
) -> (&'static str, Option<&'static str>) {
    let old: BTreeMap<_, _> = before.iter().map(|e| (e.path.as_str(), e)).collect();
    let new: BTreeMap<_, _> = after.iter().map(|e| (e.path.as_str(), e)).collect();
    let mut paths: Vec<&str> = old.keys().chain(new.keys()).copied().collect();
    paths.sort_unstable();
    paths.dedup();
    let mut changed = false;
    for path in paths {
        let (o, n) = (old.get(path).copied(), new.get(path).copied());
        if matches!((o, n), (Some(a), Some(b)) if same_entry(a, b)) {
            continue;
        }
        changed = true;
        let entries = [o, n];
        // A plain folder added or removed (not modified, not retyped) is
        // judged by scope like a file.
        let folder_added_or_removed =
            matches!((o, n), (Some(e), None) | (None, Some(e)) if e.kind == "dir");
        let reason = if has_protected_component(path) {
            Some("PROTECTED_NAME")
        } else if has_unsafe_component(path) {
            Some("UNSAFE_NAME")
        } else if entries.iter().flatten().any(|e| e.kind == "stream") {
            Some("STREAM")
        } else if entries.iter().flatten().any(|e| e.kind == "link") {
            Some("LINK")
        } else if !folder_added_or_removed && entries.iter().flatten().any(|e| e.kind != "file") {
            Some(
                if matches!((o, n), (Some(a), Some(b)) if a.kind == b.kind) {
                    "ATTRIBUTES"
                } else {
                    "NOT_PLAIN_FILE"
                },
            )
        } else if n.is_some_and(|e| e.kind == "file" && e.links != 1) {
            Some("HARD_LINK")
        } else if matches!((o, n), (Some(a), Some(b)) if !same_attributes(a, b)) {
            Some("ATTRIBUTES")
        } else if !in_scope(path, allowed) {
            Some("OUT_OF_SCOPE")
        } else {
            None
        };
        if reason.is_some() {
            return (UNEXPECTED_CHANGE, reason);
        }
    }
    (if changed { SCOPED_CHANGE } else { NO_CHANGE }, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, kind: &str, size: u64, links: u64, digest: Option<&str>) -> FolderEntry {
        FolderEntry {
            path: path.into(),
            kind: kind.into(),
            size,
            links,
            sha256: digest.map(Into::into),
            attributes: 0x20,
        }
    }

    #[test]
    fn baseline_digest_is_the_documented_canonical_text() {
        let entries = vec![
            entry("", "dir", 0, 0, None),
            entry("src", "dir", 0, 0, None),
            entry("src/a.py", "file", 3, 1, Some(&"c".repeat(64))),
        ];
        let text = format!(
            "dir\t0\t-\t\ndir\t0\t-\tsrc\nfile\t3\t{}\tsrc/a.py\n",
            "c".repeat(64)
        );
        assert_eq!(baseline_digest(&entries).unwrap(), sha(text.as_bytes()));
        let digest = baseline_digest(&entries).unwrap();
        check_project_baseline(&entries, &digest, 3).unwrap();
        assert_eq!(
            check_project_baseline(&entries, &digest, 2)
                .unwrap_err()
                .code,
            "FOLDER_BASELINE_MISMATCH"
        );
        assert_eq!(
            check_project_baseline(&entries, &"0".repeat(64), 3)
                .unwrap_err()
                .code,
            "FOLDER_BASELINE_MISMATCH"
        );
        let mut bad = entries.clone();
        bad[2].links = 2;
        assert_eq!(
            check_project_baseline(&bad, &digest, 3).unwrap_err().code,
            "FOLDER_ALIAS"
        );
        for kind in ["link", "stream", "other"] {
            let mut bad = entries.clone();
            bad.push(entry("z", kind, 0, 0, None));
            assert_eq!(
                check_project_baseline(&bad, &digest, 4).unwrap_err().code,
                "FOLDER_ALIAS",
                "{kind}"
            );
        }
        for name in [".git", ".Claude", ".MCP.json", "src/.claude"] {
            let mut bad = entries.clone();
            bad.push(entry(name, "dir", 0, 0, None));
            assert_eq!(
                check_project_baseline(&bad, &digest, 4).unwrap_err().code,
                "FOLDER_PROTECTED_NAME",
                "{name}"
            );
        }
        let mut control = entries.clone();
        control[2].path = "src/a\n.py".into();
        assert!(baseline_digest(&control).is_none());
    }

    #[test]
    fn change_records_list_every_difference_with_both_sides() {
        let before = vec![
            entry("", "dir", 0, 0, None),
            entry("keep.txt", "file", 1, 1, Some(&"a".repeat(64))),
            entry("old.txt", "file", 2, 1, Some(&"b".repeat(64))),
            entry("src/a.py", "file", 3, 1, Some(&"c".repeat(64))),
        ];
        let after = vec![
            entry("", "dir", 0, 0, None),
            entry("keep.txt", "file", 1, 1, Some(&"a".repeat(64))),
            entry("new.txt", "file", 4, 1, Some(&"d".repeat(64))),
            entry("src/a.py", "file", 5, 1, Some(&"e".repeat(64))),
        ];
        let records = change_records(&before, &after);
        assert_eq!(
            records,
            vec![
                json!({"path":"new.txt","change":"added","kind":"file","before_size":null,
                    "before_sha256":null,"after_size":4,"after_sha256":"d".repeat(64)}),
                json!({"path":"old.txt","change":"removed","kind":"file","before_size":2,
                    "before_sha256":"b".repeat(64),"after_size":null,"after_sha256":null}),
                json!({"path":"src/a.py","change":"modified","kind":"file","before_size":3,
                    "before_sha256":"c".repeat(64),"after_size":5,"after_sha256":"e".repeat(64)}),
            ]
        );
        assert!(change_records(&before, &before).is_empty());
    }

    #[test]
    fn classify_scopes_only_plain_file_content_under_allowed_paths() {
        let base = vec![
            entry("", "dir", 0, 0, None),
            entry("src", "dir", 0, 0, None),
            entry("src/a.py", "file", 3, 1, Some(&"c".repeat(64))),
            entry("README.md", "file", 3, 1, Some(&"f".repeat(64))),
        ];
        let src = vec!["src".to_string()];
        assert_eq!(classify(&base, &base, &src), (NO_CHANGE, None));
        let mut modified = base.clone();
        modified[2].sha256 = Some("d".repeat(64));
        assert_eq!(classify(&base, &modified, &src), (SCOPED_CHANGE, None));
        let mut added = base.clone();
        added.push(entry("src/b.py", "file", 1, 1, Some(&"e".repeat(64))));
        assert_eq!(classify(&base, &added, &src), (SCOPED_CHANGE, None));
        let removed: Vec<_> = base
            .iter()
            .filter(|e| e.path != "src/a.py")
            .cloned()
            .collect();
        assert_eq!(classify(&base, &removed, &src), (SCOPED_CHANGE, None));
        // The whole folder only with ".".
        let mut readme = base.clone();
        readme[3].size = 9;
        assert_eq!(
            classify(&base, &readme, &src),
            (UNEXPECTED_CHANGE, Some("OUT_OF_SCOPE"))
        );
        assert_eq!(
            classify(&base, &readme, &[".".to_string()]),
            (SCOPED_CHANGE, None)
        );
        // A prefix that is not a folder boundary is not in scope.
        let mut sibling = base.clone();
        sibling.push(entry("src2.py", "file", 1, 1, Some(&"e".repeat(64))));
        assert_eq!(
            classify(&base, &sibling, &src),
            (UNEXPECTED_CHANGE, Some("OUT_OF_SCOPE"))
        );
        let unexpected = |extra: FolderEntry| {
            let mut after = base.clone();
            after.push(extra);
            classify(&base, &after, &[".".to_string()])
        };
        // A plain folder under an allowed path is scoped (2026-11-04).
        assert_eq!(
            unexpected(entry("src/new", "dir", 0, 0, None)),
            (SCOPED_CHANGE, None)
        );
        // Any other non-file entry, and a kind change, stay unexpected.
        assert_eq!(
            unexpected(entry("src/dev", "other", 0, 0, None)),
            (UNEXPECTED_CHANGE, Some("NOT_PLAIN_FILE"))
        );
        let mut retyped = base.clone();
        retyped[2] = entry("src/a.py", "dir", 0, 0, None);
        assert_eq!(
            classify(&base, &retyped, &[".".to_string()]),
            (UNEXPECTED_CHANGE, Some("NOT_PLAIN_FILE"))
        );
        let mut folder_hidden = base.clone();
        folder_hidden[1].attributes |= 0x2;
        assert_eq!(
            classify(&base, &folder_hidden, &[".".to_string()]),
            (UNEXPECTED_CHANGE, Some("ATTRIBUTES"))
        );
        assert_eq!(
            unexpected(entry("src/l", "link", 0, 0, None)),
            (UNEXPECTED_CHANGE, Some("LINK"))
        );
        assert_eq!(
            unexpected(entry(
                "src/a.py:x:$DATA",
                "stream",
                1,
                0,
                Some(&"e".repeat(64))
            )),
            (UNEXPECTED_CHANGE, Some("STREAM"))
        );
        assert_eq!(
            unexpected(entry("src/h.py", "file", 1, 2, Some(&"e".repeat(64)))),
            (UNEXPECTED_CHANGE, Some("HARD_LINK"))
        );
        for name in [".claude", ".mcp.json", "src/.Claude", ".git"] {
            assert_eq!(
                unexpected(entry(name, "file", 1, 1, Some(&"e".repeat(64)))),
                (UNEXPECTED_CHANGE, Some("PROTECTED_NAME")),
                "{name}"
            );
        }
        let mut hidden = base.clone();
        hidden[2].attributes |= 0x2;
        assert_eq!(
            classify(&base, &hidden, &[".".to_string()]),
            (UNEXPECTED_CHANGE, Some("ATTRIBUTES"))
        );
        let mut folder_attributes = base.clone();
        folder_attributes[0].attributes |= 0x1;
        assert_eq!(
            classify(&base, &folder_attributes, &[".".to_string()]),
            (UNEXPECTED_CHANGE, Some("ATTRIBUTES"))
        );
    }

    #[test]
    fn trailing_slash_allowed_path_scopes_files_below_it() {
        // Case A (C9): allowed_paths ["src", ".pytest-tmp/"].
        let base = vec![
            entry("", "dir", 0, 0, None),
            entry(".pytest-tmp", "dir", 0, 0, None),
            entry("src", "dir", 0, 0, None),
        ];
        let allowed = vec!["src".to_string(), ".pytest-tmp/".to_string()];
        let mut file = base.clone();
        file.push(entry(
            ".pytest-tmp/log.txt",
            "file",
            1,
            1,
            Some(&"e".repeat(64)),
        ));
        assert_eq!(classify(&base, &file, &allowed), (SCOPED_CHANGE, None));
        // Not a prefix match: `.pytest-tmp2` is outside.
        let mut sibling = base.clone();
        sibling.push(entry(".pytest-tmp2", "file", 1, 1, Some(&"e".repeat(64))));
        assert_eq!(
            classify(&base, &sibling, &allowed),
            (UNEXPECTED_CHANGE, Some("OUT_OF_SCOPE"))
        );
        // A new plain folder under an allowed path (pytest's per-test
        // basetemp folders) is scoped since 2026-11-04.
        let mut folder = base.clone();
        folder.push(entry(".pytest-tmp/test0", "dir", 0, 0, None));
        assert_eq!(classify(&base, &folder, &allowed), (SCOPED_CHANGE, None));
    }

    #[test]
    fn plain_folders_under_allowed_paths_are_scoped_and_listed() {
        let base = vec![
            entry("", "dir", 0, 0, None),
            entry(".pytest-tmp", "dir", 0, 0, None),
            entry(".pytest-tmp/old", "dir", 0, 0, None),
            entry(".pytest-tmp/old/f.txt", "file", 1, 1, Some(&"a".repeat(64))),
            entry("docs", "dir", 0, 0, None),
            entry("src", "dir", 0, 0, None),
        ];
        let allowed = vec!["src".to_string(), ".pytest-tmp/".to_string()];
        // Nested additions with a file, and a removal of a folder with its file.
        let mut after: Vec<_> = base
            .iter()
            .filter(|e| !e.path.starts_with(".pytest-tmp/old"))
            .cloned()
            .collect();
        after.push(entry(".pytest-tmp/basetemp", "dir", 0, 0, None));
        after.push(entry(".pytest-tmp/basetemp/t0", "dir", 0, 0, None));
        after.push(entry(".pytest-tmp/basetemp/t0/sub", "dir", 0, 0, None));
        after.push(entry(
            ".pytest-tmp/basetemp/t0/sub/o.txt",
            "file",
            1,
            1,
            Some(&"b".repeat(64)),
        ));
        after.push(entry("src/new", "dir", 0, 0, None));
        after.sort_by(|a, b| a.path.cmp(&b.path));
        assert_eq!(classify(&base, &after, &allowed), (SCOPED_CHANGE, None));
        let records = change_records(&base, &after);
        assert_eq!(records.len(), 7, "{records:?}");
        assert!(
            records.contains(&json!({"path":".pytest-tmp/basetemp/t0","change":"added",
            "kind":"dir","before_size":null,"before_sha256":null,"after_size":0,
            "after_sha256":null}))
        );
        assert!(
            records.contains(&json!({"path":".pytest-tmp/old","change":"removed",
            "kind":"dir","before_size":0,"before_sha256":null,"after_size":null,
            "after_sha256":null}))
        );
        assert_eq!(
            records.iter().filter(|r| r["kind"] == "dir").count(),
            5,
            "{records:?}"
        );
        // Outside every allowed path: still unexpected, for scope.
        let mut outside = base.clone();
        outside.push(entry("docs/new", "dir", 0, 0, None));
        assert_eq!(
            classify(&base, &outside, &allowed),
            (UNEXPECTED_CHANGE, Some("OUT_OF_SCOPE"))
        );
        let removed: Vec<_> = base.iter().filter(|e| e.path != "docs").cloned().collect();
        assert_eq!(
            classify(&base, &removed, &allowed),
            (UNEXPECTED_CHANGE, Some("OUT_OF_SCOPE"))
        );
        // A link (junction or other reparse point) is never a plain folder.
        let mut link = base.clone();
        link.push(entry(".pytest-tmp/j", "link", 0, 0, None));
        assert_eq!(
            classify(&base, &link, &allowed),
            (UNEXPECTED_CHANGE, Some("LINK"))
        );
        // A protected folder name under an allowed path stays unexpected.
        for name in ["src/.claude", ".pytest-tmp/.git"] {
            let mut protected = base.clone();
            protected.push(entry(name, "dir", 0, 0, None));
            assert_eq!(
                classify(&base, &protected, &allowed),
                (UNEXPECTED_CHANGE, Some("PROTECTED_NAME")),
                "{name}"
            );
        }
    }

    /// C4 R5 reversed: names Win32 path normalization turns into a protected
    /// name (trailing dot or space), other names with a trailing dot or space
    /// and Windows device names are never scoped and never a baseline.
    #[test]
    fn trailing_dot_space_and_device_names_are_never_scoped() {
        let base = vec![
            entry("", "dir", 0, 0, None),
            entry("src", "dir", 0, 0, None),
            entry("src/a.py", "file", 3, 1, Some(&"c".repeat(64))),
        ];
        let digest = baseline_digest(&base).unwrap();
        check_project_baseline(&base, &digest, 3).unwrap();
        let src = vec!["src".to_string()];
        let mut checked = 0;
        for (name, kind, code, baseline) in [
            (
                "src/.git.",
                "dir",
                "PROTECTED_NAME",
                "FOLDER_PROTECTED_NAME",
            ),
            (
                "src/.git./config",
                "file",
                "PROTECTED_NAME",
                "FOLDER_PROTECTED_NAME",
            ),
            (
                "src/.claude ",
                "dir",
                "PROTECTED_NAME",
                "FOLDER_PROTECTED_NAME",
            ),
            (
                "src/.Mcp.json. .",
                "file",
                "PROTECTED_NAME",
                "FOLDER_PROTECTED_NAME",
            ),
            ("src/a.py.", "file", "UNSAFE_NAME", "FOLDER_UNSAFE_NAME"),
            ("src/b ", "file", "UNSAFE_NAME", "FOLDER_UNSAFE_NAME"),
            ("src/dir./f.py", "file", "UNSAFE_NAME", "FOLDER_UNSAFE_NAME"),
            ("src/CON", "file", "UNSAFE_NAME", "FOLDER_UNSAFE_NAME"),
            ("src/nul.txt", "file", "UNSAFE_NAME", "FOLDER_UNSAFE_NAME"),
            (
                "src/Aux.tar.gz",
                "file",
                "UNSAFE_NAME",
                "FOLDER_UNSAFE_NAME",
            ),
            ("src/prn", "dir", "UNSAFE_NAME", "FOLDER_UNSAFE_NAME"),
            ("src/com1.py", "file", "UNSAFE_NAME", "FOLDER_UNSAFE_NAME"),
            ("src/LPT9", "file", "UNSAFE_NAME", "FOLDER_UNSAFE_NAME"),
            ("src/nul .txt", "file", "UNSAFE_NAME", "FOLDER_UNSAFE_NAME"),
            // C4 re-review L3: 8.3 short names (a `~` and digits), which
            // Windows may resolve to another entry (`GIT~1` to `.git`).
            ("src/GIT~1", "dir", "UNSAFE_NAME", "FOLDER_UNSAFE_NAME"),
            (
                "src/git~1/config",
                "file",
                "UNSAFE_NAME",
                "FOLDER_UNSAFE_NAME",
            ),
            ("src/CLAUDE~1", "dir", "UNSAFE_NAME", "FOLDER_UNSAFE_NAME"),
            ("src/PROGRA~2", "dir", "UNSAFE_NAME", "FOLDER_UNSAFE_NAME"),
            ("src/MCP~1.JSO", "file", "UNSAFE_NAME", "FOLDER_UNSAFE_NAME"),
            (
                "src/TEST_A~12.PY",
                "file",
                "UNSAFE_NAME",
                "FOLDER_UNSAFE_NAME",
            ),
        ] {
            let mut after = base.clone();
            after.push(entry(
                name,
                kind,
                1,
                1,
                (kind == "file").then_some("e".repeat(64)).as_deref(),
            ));
            after.sort_by(|a, b| a.path.cmp(&b.path));
            assert_eq!(
                classify(&base, &after, &src),
                (UNEXPECTED_CHANGE, Some(code)),
                "{name}"
            );
            let digest = baseline_digest(&after).unwrap();
            assert_eq!(
                check_project_baseline(&after, &digest, after.len() as u64)
                    .unwrap_err()
                    .code,
                baseline,
                "{name}"
            );
            checked += 1;
        }
        // Ordinary names that only look alike stay scoped.
        for name in [
            "src/console.py",
            "src/com10.py",
            "src/com.py",
            "src/.gitignore",
            "src/a.b.py",
            "src/.git-notes",
            "src/a~b.py",
            "src/a~.py",
            "src/~1x.py",
        ] {
            let mut after = base.clone();
            after.push(entry(name, "file", 1, 1, Some(&"e".repeat(64))));
            after.sort_by(|a, b| a.path.cmp(&b.path));
            assert_eq!(
                classify(&base, &after, &src),
                (SCOPED_CHANGE, None),
                "{name}"
            );
        }
        assert_eq!(checked, 20);
    }

    /// C4 re-review L6: instruction files Claude reads from the folders
    /// above its working folder refuse the launch (up to the drive root);
    /// the same names inside the work folder itself are its own content.
    #[test]
    fn ancestor_instruction_files_refuse_the_launch() {
        for name in ["CLAUDE.md", "CLAUDE.local.md", "AGENTS.md", ".claude"] {
            let d = tempfile::tempdir().unwrap();
            let outer = d.path().canonicalize().unwrap();
            let work = outer.join("case").join("work");
            std::fs::create_dir_all(&work).unwrap();
            check_no_ancestor_instructions(&work).unwrap();
            // Inside the work folder: not an ancestor.
            std::fs::write(work.join("CLAUDE.md"), b"synthetic").unwrap();
            check_no_ancestor_instructions(&work).unwrap();
            for parent in [outer.join("case"), outer.clone()] {
                let target = parent.join(name);
                if name == ".claude" {
                    std::fs::create_dir(&target).unwrap();
                } else {
                    std::fs::write(&target, b"synthetic").unwrap();
                }
                assert_eq!(
                    check_no_ancestor_instructions(&work).unwrap_err().code,
                    "FOLDER_ANCESTOR_INSTRUCTIONS",
                    "{name} in {}",
                    parent.display()
                );
                if name == ".claude" {
                    std::fs::remove_dir(&target).unwrap();
                } else {
                    std::fs::remove_file(&target).unwrap();
                }
            }
            // A different letter case is the same name on Windows.
            #[cfg(windows)]
            {
                std::fs::write(outer.join(name.to_ascii_lowercase()), b"synthetic").unwrap();
                assert!(check_no_ancestor_instructions(&work).is_err(), "{name}");
            }
        }
    }

    /// C4 R5 reversed on disk: the same names created through a verbatim path.
    #[cfg(windows)]
    #[test]
    fn verbatim_trailing_dot_and_space_names_are_unexpected_on_disk() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        let before = snapshot(&root).unwrap();
        let verbatim = |rel: &str| {
            let s = root.to_string_lossy().to_string();
            let s = if s.starts_with(r"\\?\") {
                s
            } else {
                format!(r"\\?\{s}")
            };
            std::path::PathBuf::from(format!(r"{s}\{rel}"))
        };
        std::fs::create_dir(verbatim(r"src\.git.")).unwrap();
        std::fs::write(verbatim(r"src\.git.\config"), b"[core]\n").unwrap();
        std::fs::create_dir(verbatim(r"src\.claude ")).unwrap();
        std::fs::write(verbatim(r"src\.claude \settings.json"), b"{}").unwrap();
        let after = snapshot(&root).unwrap();
        let result = classify(&before, &after, &["src".to_string()]);
        let baseline = check_project_baseline(
            &after,
            &baseline_digest(&after).unwrap(),
            after.len() as u64,
        );
        let _ = std::fs::remove_dir_all(verbatim(r"src\.git."));
        let _ = std::fs::remove_dir_all(verbatim(r"src\.claude "));
        assert_eq!(result, (UNEXPECTED_CHANGE, Some("PROTECTED_NAME")));
        assert_eq!(baseline.unwrap_err().code, "FOLDER_PROTECTED_NAME");
    }

    /// C4 R3 reversed: NTFS sets the archive attribute (0x20) on every write,
    /// so it alone is never a difference; any other attribute still is.
    #[cfg(windows)]
    #[test]
    fn the_archive_attribute_alone_is_not_a_difference() {
        let base = vec![
            entry("", "dir", 0, 0, None),
            entry("src", "dir", 0, 0, None),
            entry("src/a.py", "file", 3, 1, Some(&"c".repeat(64))),
        ];
        let src = vec!["src".to_string()];
        // Without the archive bit and nothing else, Windows reports NORMAL.
        let mut cleared = base.clone();
        for e in &mut cleared {
            e.attributes = 0x80;
        }
        assert_eq!(classify(&cleared, &base, &src), (NO_CHANGE, None));
        assert!(change_records(&cleared, &base).is_empty());
        let mut edited = base.clone();
        edited[2].sha256 = Some("d".repeat(64));
        assert_eq!(classify(&cleared, &edited, &src), (SCOPED_CHANGE, None));
        assert_eq!(change_records(&cleared, &edited).len(), 1);
        let mut hidden = edited.clone();
        hidden[2].attributes |= 0x2;
        assert_eq!(
            classify(&cleared, &hidden, &src),
            (UNEXPECTED_CHANGE, Some("ATTRIBUTES"))
        );
        // On disk: a file copied without the archive bit, then edited.
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        std::fs::create_dir(root.join("src")).unwrap();
        let file = root.join("src").join("a.py");
        std::fs::write(&file, b"print(1)\n").unwrap();
        let status = std::process::Command::new("attrib")
            .arg("-A")
            .arg(&file)
            .status()
            .unwrap();
        assert!(status.success());
        let before = snapshot(&root).unwrap();
        std::fs::write(&file, b"print(2)\n").unwrap();
        let after = snapshot(&root).unwrap();
        assert_eq!(classify(&before, &after, &src), (SCOPED_CHANGE, None));
    }

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
