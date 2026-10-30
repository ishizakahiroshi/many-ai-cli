//! Hub-owned list of ordinary worktrees that were still unmerged (or otherwise
//! kept) when their session closed under the `delete` policy, so the next Hub
//! start can judge them again. Only `<parent>/.git-worktrees/<name>` trees on
//! branch `many-ai/<name>` are listed. The list never decides removal itself:
//! every recheck repeats the full cleanup conditions against Git.
use super::spelling;
use crate::{files::safe_fs::Dir, proto::core::NormalWorktree};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsStr,
    io,
    path::Path,
    sync::{Mutex, MutexGuard, PoisonError},
};

const FILE: &str = "normal-worktrees-retained.json";
const MAX_BYTES: usize = 1024 * 1024;

#[derive(Default, Serialize, Deserialize)]
struct Document {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    worktrees: Vec<Record>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Record {
    path: String,
    parent_dir: String,
    branch: String,
}
impl Record {
    fn tree(self) -> NormalWorktree {
        NormalWorktree {
            path: self.path,
            parent_dir: self.parent_dir,
            branch: self.branch,
            created: true,
        }
    }
}

/// What one startup recheck did. `forgotten` counts records dropped without
/// removing anything (gone, malformed, or no longer on their branch) plus the
/// records of removed trees.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RecheckReport {
    pub removed: usize,
    pub retained: usize,
    pub in_use: usize,
    pub fetch_failed: usize,
    pub forgotten: usize,
    pub store_failed: bool,
}

pub struct RetainedWorktrees {
    root: Dir,
    lock: Mutex<()>,
}
impl RetainedWorktrees {
    /// `root` is the held runtime directory (`~/.many-ai-cli`).
    pub fn new(root: Dir) -> Self {
        Self {
            root,
            lock: Mutex::new(()),
        }
    }
    pub fn load(&self) -> io::Result<Vec<NormalWorktree>> {
        let _guard = self.guard();
        Ok(self
            .read()?
            .worktrees
            .into_iter()
            .map(Record::tree)
            .collect())
    }
    /// A tree outside the ordinary layout is not listed. An unreadable list is
    /// left untouched (the error is returned) rather than replaced.
    pub fn remember(&self, tree: &NormalWorktree) -> io::Result<()> {
        if !well_formed(tree) {
            return Ok(());
        }
        let _guard = self.guard();
        let mut document = self.read()?;
        document.worktrees.retain(|record| record.path != tree.path);
        document.worktrees.push(Record {
            path: tree.path.clone(),
            parent_dir: tree.parent_dir.clone(),
            branch: tree.branch.clone(),
        });
        self.write(&mut document)
    }
    pub fn forget(&self, path: &str) -> io::Result<()> {
        let _guard = self.guard();
        let mut document = self.read()?;
        let before = document.worktrees.len();
        document.worktrees.retain(|record| record.path != path);
        if document.worktrees.len() == before {
            return Ok(());
        }
        self.write(&mut document)
    }
    fn guard(&self) -> MutexGuard<'_, ()> {
        self.lock.lock().unwrap_or_else(PoisonError::into_inner)
    }
    fn read(&self) -> io::Result<Document> {
        let bytes = match self.root.read(FILE, MAX_BYTES + 1) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(Document::default());
            }
            Err(error) => return Err(error),
        };
        if bytes.len() > MAX_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "retained worktree list exceeds its size limit",
            ));
        }
        serde_json::from_slice(&bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }
    fn write(&self, document: &mut Document) -> io::Result<()> {
        document.version = 1;
        let bytes = serde_json::to_vec_pretty(document).map_err(io::Error::other)?;
        self.root.replace(FILE, &bytes, 0o600)
    }
}

/// `<parent>/.git-worktrees/<name>` on branch `many-ai/<name>`, as `prepare`
/// creates it.
pub(super) fn well_formed(tree: &NormalWorktree) -> bool {
    let path = Path::new(&tree.path);
    let (Some(name), Some(root)) = (path.file_name().and_then(OsStr::to_str), path.parent()) else {
        return false;
    };
    tree.created
        && path.is_absolute()
        && root.file_name() == Some(OsStr::new(".git-worktrees"))
        && root
            .parent()
            .is_some_and(|parent| spelling(parent) == spelling(Path::new(&tree.parent_dir)))
        && tree.branch == format!("many-ai/{name}")
}
