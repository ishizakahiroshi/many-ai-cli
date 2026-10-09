//! Provider reader budgets and the explicit trial file boundary.
pub mod claude;
pub mod codex;
pub mod grok;
use crate::config::RuntimePaths;
use crate::files::safe_fs::Dir;
use std::{
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom},
    path::Path,
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
        // Validate through the same held capabilities as the eventual read.
        // Canonicalizing an arbitrary target here would admit unselected aliases.
        paths.relative_to_selected_root(path)?;
        if open_directory(paths, path).is_err() {
            open_artifact(paths, path)?;
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
/// Trial opens walk from the held selected-root capability. Every component is
/// opened relative to its pinned parent, refusing replaced symlinks/reparse
/// points rather than re-resolving an authorized absolute name afterward.
pub fn open_artifact(paths: &RuntimePaths, path: &Path) -> io::Result<File> {
    if !paths.is_trial() {
        return File::open(path);
    }
    let mut directory = Dir::open(paths.root())?;
    let relative = paths.relative_to_selected_root(path)?;
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
    let relative = paths.relative_to_selected_root(path)?;
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

    #[cfg(unix)]
    #[test]
    fn selected_trial_alias_keeps_artifacts_in_the_canonical_tree() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("physical");
        let installed = temp.path().join("installed");
        fs::create_dir_all(root.join("records")).unwrap();
        fs::create_dir_all(installed.join("records")).unwrap();
        fs::write(root.join("records/events.jsonl"), b"original").unwrap();
        fs::write(installed.join("records/events.jsonl"), b"outside").unwrap();
        let selected = temp.path().join("selected");
        let unselected = temp.path().join("unselected");
        symlink(&root, &selected).unwrap();
        symlink(&root, &unselected).unwrap();
        let paths = RuntimePaths::trial(&selected, 49325, &installed).unwrap();
        let file = selected.join("records/events.jsonl");
        assert_eq!(read_head(&paths, &file, 8).unwrap(), b"original");
        assert_eq!(
            read_tail(&paths, &paths.root().join("records/events.jsonl"), 4).unwrap(),
            b"inal"
        );
        assert!(open_directory(&paths, &selected).is_ok());
        assert!(check_path(&paths, &file).is_ok());
        assert!(check_path(&paths, &unselected.join("records/events.jsonl")).is_err());
        assert!(check_path(&paths, &selected.join("records/../records/events.jsonl")).is_err());
        symlink(root.join("records"), root.join("inside-alias")).unwrap();
        symlink(&installed, root.join("outside-alias")).unwrap();
        symlink(root.join("records/events.jsonl"), root.join("leaf-alias")).unwrap();
        for alias in [
            "inside-alias/events.jsonl",
            "outside-alias/records/events.jsonl",
            "leaf-alias",
        ] {
            assert!(open_artifact(&paths, &selected.join(alias)).is_err());
            assert!(check_path(&paths, &selected.join(alias)).is_err());
        }
        fs::remove_file(&selected).unwrap();
        symlink(&installed, &selected).unwrap();
        assert_eq!(read_head(&paths, &file, 8).unwrap(), b"original");
        assert!(check_path(&paths, &file).is_ok());
        assert_eq!(
            entries(&paths, &selected.join("records")).unwrap()[0].name,
            "events.jsonl"
        );
    }
}
