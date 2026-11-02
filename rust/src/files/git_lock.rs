//! `index.lock` files a stopped Git left behind.
//!
//! Git cannot clean up after a forced stop. A `git commit -a` or `git add`
//! that goes down with its session, or a writing Git the Hub stops at its
//! deadline, leaves `.git/index.lock`, and the user's next `git add` /
//! `git commit` fails with "File exists" (measured 2026-10-18: a killed
//! `commit -a` left a 137-byte lock, so size proves nothing).
//!
//! When the Hub stops such work it looks at that repository [`SETTLE`] later. A
//! lock still there and written after the work started is listed for the UI.
//! Nothing is removed on its own: the user asks, and the lock goes only if it
//! is unchanged and no Git process works in that repository
//! (`process::live_git`; an unreadable process counts as one). While the
//! leftover exists no other Git can take the same lock, so a lock that passes
//! those checks cannot be in use.
use crate::process::live_git::Using;
use serde::Serialize;
use std::{
    io,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime},
};

/// How long after a stop the Hub looks for the lock.
pub const SETTLE: Duration = Duration::from_secs(3);
/// Modification times are compared with this much clock slack.
const SLACK: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Leftover {
    pub id: u64,
    /// The work tree the lock blocks.
    pub repository: String,
    pub lock: String,
    /// `session` (a stopped session) or `timeout` (a Hub Git stopped at its limit).
    pub cause: String,
    pub session: i64,
    pub stopped_at: String,
    pub modified_at: String,
    pub size: u64,
    #[serde(skip)]
    modified: SystemTime,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Already gone (removed by the user, or the owner finished after all).
    Gone,
    /// Rewritten since it was listed, so it belongs to a Git that is working.
    Changed,
    /// These Git processes work in the repository.
    InUse(Vec<u32>),
    /// Running Git processes could not be checked.
    Unverifiable(String),
    Failed(String),
}

impl Refusal {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Gone => "lock_gone",
            Self::Changed => "lock_changed",
            Self::InUse(_) => "git_running",
            Self::Unverifiable(_) => "git_unverifiable",
            Self::Failed(_) => "remove_failed",
        }
    }
    pub fn message(&self) -> String {
        match self {
            Self::Gone => "index.lock is already gone".into(),
            Self::Changed => {
                "index.lock changed since it was listed; a Git command is using it".into()
            }
            Self::InUse(pids) => format!(
                "a Git command is running in this repository (pid {}); try again when it finishes",
                pids.iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Unverifiable(detail) => {
                format!(
                    "running Git commands could not be checked, so index.lock was kept: {detail}"
                )
            }
            Self::Failed(detail) => format!("index.lock could not be removed: {detail}"),
        }
    }
}

/// The work tree root and Git directory that hold `cwd`: the nearest `.git`
/// directory, or a `.git` file whose `gitdir:` names it (a linked worktree).
pub fn locate(cwd: &Path) -> Option<(PathBuf, PathBuf)> {
    for root in cwd.ancestors() {
        let dot = root.join(".git");
        let Ok(meta) = std::fs::metadata(&dot) else {
            continue;
        };
        if meta.is_dir() {
            return Some((root.to_path_buf(), dot));
        }
        if meta.is_file() && meta.len() <= 4096 {
            let text = std::fs::read_to_string(&dot).ok()?;
            let target = text
                .lines()
                .find_map(|line| line.strip_prefix("gitdir:"))?
                .trim();
            let target = Path::new(target);
            let dir = if target.is_absolute() {
                target.to_path_buf()
            } else {
                root.join(target)
            };
            return Some((root.to_path_buf(), dir));
        }
        return None;
    }
    None
}

fn rfc3339(time: SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(time)
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
}

#[derive(Default)]
pub struct LockWatch {
    entries: Mutex<Vec<Leftover>>,
    next: AtomicU64,
}

impl LockWatch {
    fn entries(&self) -> std::sync::MutexGuard<'_, Vec<Leftover>> {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Record the repository's `index.lock` if it outlived work that started
    /// at `since` and was stopped at `stopped`. A lock some running Git in the
    /// repository still owns (a session that only lost its Hub connection, or
    /// the user's own command) is not listed.
    pub fn inspect(
        &self,
        cwd: &Path,
        since: SystemTime,
        stopped: SystemTime,
        cause: &str,
        session: i64,
        using: impl FnOnce(&Path) -> io::Result<Using>,
    ) -> Option<Leftover> {
        let (root, git_dir) = locate(cwd)?;
        let lock = git_dir.join("index.lock");
        let meta = std::fs::metadata(&lock).ok()?;
        let modified = meta.modified().ok()?;
        let earliest = since.checked_sub(SLACK).unwrap_or(SystemTime::UNIX_EPOCH);
        if !meta.is_file() || modified < earliest {
            return None;
        }
        if matches!(using(&root), Ok(Using::Running(_))) {
            return None;
        }
        let lock_text = lock.to_string_lossy().into_owned();
        let mut entries = self.entries();
        let id = entries
            .iter()
            .find(|e| e.lock == lock_text)
            .map(|e| e.id)
            .unwrap_or_else(|| self.next.fetch_add(1, Ordering::Relaxed) + 1);
        entries.retain(|e| e.lock != lock_text);
        let leftover = Leftover {
            id,
            repository: root.to_string_lossy().into_owned(),
            lock: lock_text,
            cause: cause.into(),
            session,
            stopped_at: rfc3339(stopped),
            modified_at: rfc3339(modified),
            size: meta.len(),
            modified,
        };
        entries.push(leftover.clone());
        Some(leftover)
    }

    fn unchanged(entry: &Leftover) -> Result<(), Refusal> {
        match std::fs::metadata(&entry.lock) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Err(Refusal::Gone),
            Err(e) => Err(Refusal::Failed(e.to_string())),
            Ok(meta)
                if meta.len() != entry.size || meta.modified().ok() != Some(entry.modified) =>
            {
                Err(Refusal::Changed)
            }
            Ok(_) => Ok(()),
        }
    }

    /// Leftovers still on disk and unchanged; the rest are forgotten.
    pub fn list(&self) -> Vec<Leftover> {
        let mut entries = self.entries();
        entries.retain(|entry| Self::unchanged(entry).is_ok());
        entries.clone()
    }

    /// Remove one listed lock after checking it again.
    pub fn remove(
        &self,
        id: u64,
        using: impl FnOnce(&Path) -> io::Result<Using>,
    ) -> Result<Leftover, Refusal> {
        let entry = self
            .entries()
            .iter()
            .find(|e| e.id == id)
            .cloned()
            .ok_or(Refusal::Gone)?;
        let forget = |watch: &Self| watch.entries().retain(|e| e.id != id);
        if let Err(refusal) = Self::unchanged(&entry) {
            if matches!(refusal, Refusal::Gone | Refusal::Changed) {
                forget(self);
            }
            return Err(refusal);
        }
        match using(Path::new(&entry.repository)) {
            Ok(Using::None) => {}
            Ok(Using::Running(pids)) => return Err(Refusal::InUse(pids)),
            Ok(Using::Unknown(pids)) => {
                return Err(Refusal::Unverifiable(format!(
                    "Git processes {} could not be read",
                    pids.iter()
                        .map(u32::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
            Err(e) => return Err(Refusal::Unverifiable(e.to_string())),
        }
        match std::fs::remove_file(&entry.lock) {
            Ok(()) => {
                forget(self);
                Ok(entry)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                forget(self);
                Err(Refusal::Gone)
            }
            Err(e) => Err(Refusal::Failed(e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn idle(_: &Path) -> io::Result<Using> {
        Ok(Using::None)
    }

    fn repo() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("synthetic repo");
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::create_dir_all(root.join("src").join("deep")).unwrap();
        (dir, root)
    }

    fn lock(root: &Path, bytes: &[u8]) -> PathBuf {
        let lock = root.join(".git").join("index.lock");
        fs::write(&lock, bytes).unwrap();
        lock
    }

    #[test]
    fn locate_finds_the_git_directory_or_a_worktree_link() {
        let (dir, root) = repo();
        let deep = root.join("src").join("deep");
        assert_eq!(locate(&deep), Some((root.clone(), root.join(".git"))));
        let linked = dir.path().join("linked");
        fs::create_dir(&linked).unwrap();
        fs::write(
            linked.join(".git"),
            "gitdir: ../synthetic repo/.git/worktrees/linked\n",
        )
        .unwrap();
        assert_eq!(
            locate(&linked),
            Some((
                linked.clone(),
                linked.join("../synthetic repo/.git/worktrees/linked")
            ))
        );
        let plain = dir.path().join("plain");
        fs::create_dir(&plain).unwrap();
        // The temporary directory itself is not inside a repository.
        assert!(locate(&plain).is_none_or(|(found, _)| !found.starts_with(dir.path())));
    }

    #[test]
    fn inspect_lists_only_a_lock_written_after_the_work_started() {
        let (_dir, root) = repo();
        let watch = LockWatch::default();
        let now = SystemTime::now();
        assert_eq!(watch.inspect(&root, now, now, "session", 1, idle), None);
        lock(&root, b"137 synthetic bytes");
        // Work that started an hour after the lock was written did not leave it.
        let later = now + Duration::from_secs(3600);
        assert_eq!(watch.inspect(&root, later, later, "session", 1, idle), None);
        let found = watch
            .inspect(
                &root.join("src"),
                now - Duration::from_secs(60),
                now,
                "session",
                7,
                idle,
            )
            .unwrap();
        assert_eq!(found.repository, root.to_string_lossy());
        assert_eq!(found.cause, "session");
        assert_eq!(found.session, 7);
        assert_eq!(found.size, 19);
        // Seeing the same lock again keeps one entry with the same id.
        let again = watch
            .inspect(
                &root,
                now - Duration::from_secs(60),
                now,
                "timeout",
                0,
                idle,
            )
            .unwrap();
        assert_eq!(again.id, found.id);
        assert_eq!(watch.list().len(), 1);
        assert_eq!(watch.list()[0].cause, "timeout");
    }

    #[test]
    fn inspect_skips_a_lock_a_running_git_still_owns() {
        let (_dir, root) = repo();
        let watch = LockWatch::default();
        lock(&root, b"held");
        let since = SystemTime::now() - Duration::from_secs(60);
        let running = |_: &Path| Ok(Using::Running(vec![7]));
        assert_eq!(
            watch.inspect(&root, since, SystemTime::now(), "session", 1, running),
            None
        );
        // Unreadable processes do not hide it; removal stays refused instead.
        let unknown = |_: &Path| Ok(Using::Unknown(vec![8]));
        assert!(
            watch
                .inspect(&root, since, SystemTime::now(), "session", 1, unknown)
                .is_some()
        );
    }

    #[test]
    fn list_forgets_locks_that_are_gone_or_rewritten() {
        let (_dir, root) = repo();
        let watch = LockWatch::default();
        let path = lock(&root, b"one");
        let since = SystemTime::now() - Duration::from_secs(60);
        watch
            .inspect(&root, since, SystemTime::now(), "session", 1, idle)
            .unwrap();
        fs::write(&path, b"rewritten by a working git").unwrap();
        assert!(watch.list().is_empty());
        watch
            .inspect(&root, since, SystemTime::now(), "session", 1, idle)
            .unwrap();
        fs::remove_file(&path).unwrap();
        assert!(watch.list().is_empty());
    }

    #[test]
    fn remove_deletes_only_an_unchanged_lock_no_git_is_using() {
        let (_dir, root) = repo();
        let watch = LockWatch::default();
        let path = lock(&root, b"left behind");
        let since = SystemTime::now() - Duration::from_secs(60);
        let id = watch
            .inspect(&root, since, SystemTime::now(), "session", 1, idle)
            .unwrap()
            .id;
        // A running Git in the repository keeps it, and so does one that cannot be read.
        assert_eq!(
            watch.remove(id, |_| Ok(Using::Running(vec![41]))),
            Err(Refusal::InUse(vec![41]))
        );
        assert!(matches!(
            watch.remove(id, |_| Ok(Using::Unknown(vec![42]))),
            Err(Refusal::Unverifiable(_))
        ));
        assert!(matches!(
            watch.remove(id, |_| Err(io::Error::other("synthetic"))),
            Err(Refusal::Unverifiable(_))
        ));
        assert!(path.exists());
        // The check is asked about the work tree root.
        let mut asked = None;
        let removed = watch
            .remove(id, |root| {
                asked = Some(root.to_path_buf());
                Ok(Using::None)
            })
            .unwrap();
        assert_eq!(asked, Some(root.clone()));
        assert_eq!(removed.id, id);
        assert!(!path.exists());
        assert!(watch.list().is_empty());
        assert_eq!(watch.remove(id, |_| Ok(Using::None)), Err(Refusal::Gone));
    }

    #[test]
    fn unsupported_process_inspection_preserves_lock_bytes_and_tracked_identity() {
        let (_dir, root) = repo();
        let watch = LockWatch::default();
        let path = lock(&root, b"synthetic lock to retain");
        let since = SystemTime::now() - Duration::from_secs(60);
        let entry = watch
            .inspect(&root, since, SystemTime::now(), "session", 1, idle)
            .unwrap();
        let before = fs::read(&path).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let result = watch.remove(entry.id, |asked| {
            assert_eq!(asked, root);
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "synthetic platform cannot inspect Git processes",
            ))
        });
        assert!(matches!(result, Err(Refusal::Unverifiable(ref detail))
            if detail == "synthetic platform cannot inspect Git processes"));
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        assert_eq!(watch.list().len(), 1);
        assert_eq!(watch.list()[0].id, entry.id);
        assert!(watch.remove(entry.id, idle).is_ok());
        assert!(!path.exists());
    }

    #[test]
    fn modified_time_only_change_rejects_before_process_inspection_and_keeps_bytes() {
        let (_dir, root) = repo();
        let watch = LockWatch::default();
        let path = lock(&root, b"same bytes and size");
        let since = SystemTime::now() - Duration::from_secs(60);
        let entry = watch
            .inspect(&root, since, SystemTime::now(), "session", 1, idle)
            .unwrap();
        let before = fs::read(&path).unwrap();
        let old_time = fs::metadata(&path).unwrap().modified().unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(old_time + Duration::from_secs(2)))
            .unwrap();
        assert_ne!(fs::metadata(&path).unwrap().modified().unwrap(), old_time);
        assert_eq!(
            watch.remove(entry.id, |_| panic!(
                "changed lock must reject before inspection"
            )),
            Err(Refusal::Changed)
        );
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(path.exists());
    }

    #[test]
    fn remove_refuses_a_lock_rewritten_after_it_was_listed() {
        let (_dir, root) = repo();
        let watch = LockWatch::default();
        let path = lock(&root, b"left behind");
        let since = SystemTime::now() - Duration::from_secs(60);
        let id = watch
            .inspect(&root, since, SystemTime::now(), "session", 1, idle)
            .unwrap()
            .id;
        fs::write(&path, b"now held by a working git").unwrap();
        assert_eq!(watch.remove(id, |_| Ok(Using::None)), Err(Refusal::Changed));
        assert!(path.exists());
    }
}
