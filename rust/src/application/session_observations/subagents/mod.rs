//! Provider reader budgets and the explicit trial file boundary.
pub mod claude;
pub mod codex;
pub mod grok;
use crate::config::RuntimePaths;
use crate::files::safe_fs::Dir;
use std::{
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};
pub struct ReadBudget {
    pub head_bytes: u64,
    pub tail_bytes: u64,
    pub max_nodes: usize,
}
impl Default for ReadBudget {
    fn default() -> Self {
        Self {
            head_bytes: 64 * 1024,
            tail_bytes: 128 * 1024,
            max_nodes: 50,
        }
    }
}
pub fn check_path(paths: &RuntimePaths, path: &Path) -> io::Result<()> {
    if paths.is_trial() {
        let root = fs::canonicalize(paths.root())?;
        let resolved = fs::canonicalize(path)?;
        if !resolved.starts_with(root) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "provider artifact escapes trial root",
            ));
        }
    }
    Ok(())
}
pub fn read_head(paths: &RuntimePaths, path: &Path, cap: u64) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    open_artifact(paths, path)?
        .take(cap)
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}
pub fn read_tail(paths: &RuntimePaths, path: &Path, cap: u64) -> io::Result<Vec<u8>> {
    let mut file = open_artifact(paths, path)?;
    let size = file.metadata()?.len();
    file.seek(SeekFrom::Start(size.saturating_sub(cap)))?;
    let mut bytes = Vec::new();
    file.take(cap).read_to_end(&mut bytes)?;
    Ok(bytes)
}
fn relative_artifact(root: &Path, path: &Path) -> io::Result<PathBuf> {
    if let Ok(relative) = path.strip_prefix(root) {
        return Ok(relative.into());
    }
    // RuntimePaths pins a canonical Windows root. A native wrapper may report
    // the same drive/UNC path without the canonical verbatim prefix. Change
    // only that prefix spelling; retain every caller-supplied component so the
    // subsequent held-directory walk still rejects traversal and aliases.
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        let mut components = root.components();
        let mut ordinary = PathBuf::new();
        if let Some(Component::Prefix(prefix)) = components.next() {
            match prefix.kind() {
                Prefix::VerbatimDisk(drive) => ordinary.push(format!("{}:", char::from(drive))),
                Prefix::VerbatimUNC(server, share) => {
                    ordinary.push(Path::new(r"\\").join(server).join(share));
                }
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "provider artifact escapes trial root",
                    ));
                }
            }
            for component in components {
                ordinary.push(component.as_os_str());
            }
            if let Ok(relative) = path.strip_prefix(ordinary) {
                return Ok(relative.into());
            }
        }
    }
    Err(io::Error::new(
        io::ErrorKind::PermissionDenied,
        "provider artifact escapes trial root",
    ))
}
/// Trial opens walk from the held selected-root capability. Every component is
/// opened relative to its pinned parent, refusing replaced symlinks/reparse
/// points rather than re-resolving an authorized absolute name afterward.
pub fn open_artifact(paths: &RuntimePaths, path: &Path) -> io::Result<File> {
    if !paths.is_trial() {
        return File::open(path);
    }
    let mut directory = Dir::open(paths.root())?;
    let relative = relative_artifact(paths.root(), path)?;
    let mut components = relative.components().peekable();
    while let Some(component) = components.next() {
        let std::path::Component::Normal(name) = component else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unclean trial artifact path",
            ));
        };
        let name = name
            .to_str()
            .ok_or_else(|| io::Error::other("non-UTF8 artifact component"))?;
        if components.peek().is_none() {
            return directory.open_file(name, false);
        }
        directory = directory.child_dir(name, false)?;
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        "artifact path names a directory",
    ))
}
pub fn artifact_metadata(paths: &RuntimePaths, path: &Path) -> io::Result<fs::Metadata> {
    open_artifact(paths, path)?.metadata()
}
pub fn open_directory(paths: &RuntimePaths, path: &Path) -> io::Result<Dir> {
    if !paths.is_trial() {
        return Dir::open(path);
    }
    let mut directory = Dir::open(paths.root())?;
    let relative = relative_artifact(paths.root(), path)?;
    for component in relative.components() {
        let std::path::Component::Normal(name) = component else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unclean artifact directory path",
            ));
        };
        let name = name
            .to_str()
            .ok_or_else(|| io::Error::other("non-UTF8 artifact component"))?;
        directory = directory.child_dir(name, false)?;
    }
    Ok(directory)
}
pub fn directory_metadata(paths: &RuntimePaths, path: &Path) -> io::Result<fs::Metadata> {
    if paths.is_trial() {
        open_directory(paths, path)?.own_metadata()
    } else {
        fs::metadata(path)
    }
}
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
}
pub fn entries(paths: &RuntimePaths, path: &Path) -> io::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    if paths.is_trial() {
        let directory = open_directory(paths, path)?;
        for name in directory.entries()? {
            let is_dir = directory.child_dir(&name, false).is_ok();
            if is_dir || directory.open_file(&name, false).is_ok() {
                entries.push(Entry { name, is_dir })
            }
        }
    } else {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            entries.push(Entry {
                name: entry.file_name().to_string_lossy().into_owned(),
                is_dir: entry.file_type()?.is_dir(),
            })
        }
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_trial_path_spelling_reads_through_owned_capability() {
        let root = tempfile::tempdir().unwrap();
        let installed = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::trial(root.path(), 49325, installed.path()).unwrap();
        let directory = root.path().join("native");
        fs::create_dir(&directory).unwrap();
        let file = directory.join("record.jsonl");
        fs::write(&file, b"synthetic").unwrap();
        assert_eq!(read_head(&paths, &file, 9).unwrap(), b"synthetic");
        assert_eq!(entries(&paths, &directory).unwrap()[0].name, "record.jsonl");
        assert!(open_artifact(&paths, &directory.join("../native/record.jsonl")).is_err());
        let outside = installed.path().join("record.jsonl");
        fs::write(&outside, b"outside").unwrap();
        assert!(open_artifact(&paths, &outside).is_err());
    }
}
