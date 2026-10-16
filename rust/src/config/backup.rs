//! Numbered backups of `config.yaml` under `<root>/backups/config/`.
//!
//! Each backup is a byte-for-byte copy named `config.yaml.bak_YYYYMMDD_HHmmss`
//! (local time). A name that already exists is never overwritten, nothing is
//! ever deleted automatically, and files are created readable by the owner
//! only (the settings hold the Hub token).
use super::paths::{Resource, RuntimePaths};
use crate::files::safe_fs::Dir;
use chrono::NaiveDateTime;
use std::{io, path::PathBuf, time::SystemTime};

pub(super) const PREFIX: &str = "config.yaml.bak_";
const LEGACY_NAME: &str = "config.yaml.bak";
const CONFIG_NAME: &str = "config.yaml";
/// The most a backup will copy. A file over this is refused with an error and
/// nothing is written; a backup is never a cut-off copy.
pub(super) const COPY_LIMIT: usize = 256 * 1024 * 1024;
/// The most a small record (the notice about what a save took out) may be.
const RECORD_LIMIT: usize = 1024 * 1024;

/// Read `name` completely. A file over `COPY_LIMIT` is an error, never a
/// silently shortened read (the callers copy these bytes as a backup, or
/// restore from one).
pub(super) fn read_whole(dir: &Dir, name: &str) -> io::Result<Vec<u8>> {
    read_whole_limited(dir, name, COPY_LIMIT)
}
/// The error `read_whole` returns for a file over the limit (so a caller can
/// tell it from any other read failure).
#[derive(Debug)]
struct FileTooLarge(String);
impl std::fmt::Display for FileTooLarge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for FileTooLarge {}
/// An error that says part of the work was done: the caller must not tell the
/// person "nothing was changed" (see `partial_failure`).
#[derive(Debug)]
struct PartialFailure(String);
impl std::fmt::Display for PartialFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for PartialFailure {}
/// `message` must say exactly what was and was not changed.
pub(super) fn partial_failure(message: String) -> io::Error {
    io::Error::other(PartialFailure(message))
}
/// True for an error made by `partial_failure`: something was already changed,
/// so a caller must not add "the settings were not changed".
pub fn is_partial_failure(error: &io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|inner| inner.is::<PartialFailure>())
}
pub(super) fn is_too_large(error: &io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|inner| inner.is::<FileTooLarge>())
}
fn read_whole_limited(dir: &Dir, name: &str, limit: usize) -> io::Result<Vec<u8>> {
    let bytes = dir.read(name, limit + 1)?;
    if bytes.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            FileTooLarge(format!(
                "{name} is larger than {} MiB, which is more than a backup copies; nothing was copied or changed",
                limit / (1024 * 1024)
            )),
        ));
    }
    Ok(bytes)
}
/// Backups made within one second: the first, then `_2` ... `_999`.
const MAX_SAME_SECOND: usize = 999;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackupRecord {
    /// `YYYYMMDD_HHmmss`, the part users pass to `config backup restore`.
    pub id: String,
    pub file_name: String,
    pub path: PathBuf,
    pub bytes: u64,
}
impl BackupRecord {
    /// `YYYY-MM-DD HH:MM:SS` for display.
    pub fn created(&self) -> String {
        let id = &self.id;
        format!(
            "{}-{}-{} {}:{}:{}",
            &id[0..4],
            &id[4..6],
            &id[6..8],
            &id[9..11],
            &id[11..13],
            &id[13..15]
        )
    }
}

#[derive(Debug)]
pub struct RestoreOutcome {
    pub restored: BackupRecord,
    /// The settings that were in place before the restore (None if there were none).
    pub previous: Option<BackupRecord>,
    /// The settings in place were already the same bytes as the copy, so there
    /// was nothing to save.
    pub already_identical: bool,
}

pub struct ConfigBackups {
    paths: RuntimePaths,
    clock: fn() -> NaiveDateTime,
}
impl ConfigBackups {
    pub fn new(paths: &RuntimePaths) -> Self {
        Self::with_clock(paths, || chrono::Local::now().naive_local())
    }
    pub fn with_clock(paths: &RuntimePaths, clock: fn() -> NaiveDateTime) -> Self {
        Self {
            paths: paths.clone(),
            clock,
        }
    }
    pub fn directory(&self) -> PathBuf {
        self.paths.resource(Resource::ConfigBackups)
    }
    fn open_root(&self) -> io::Result<Dir> {
        Dir::open_or_create_private(self.paths.root())
    }
    fn open_dir(&self, root: &Dir, create: bool) -> io::Result<Dir> {
        root.child_dir("backups", create)?
            .child_dir("config", create)
    }
    /// `YYYY-MM-DD HH:MM:SS` now, by this backup writer's clock.
    pub(super) fn now_text(&self) -> String {
        (self.clock)().format("%Y-%m-%d %H:%M:%S").to_string()
    }
    /// The names in the backup folder (`[]` when there is none yet).
    pub(super) fn record_names(&self) -> io::Result<Vec<String>> {
        let root = match Dir::open(self.paths.root()) {
            Ok(root) => root,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let dir = match self.open_dir(&root, false) {
            Ok(dir) => dir,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        dir.entries()
    }
    /// The bytes of a small record kept in the backup folder (`None`: no such
    /// record). Records are not backups: they never match the backup name. A
    /// record over 1 MiB is an error, never a cut-off read.
    pub(super) fn record_read(&self, name: &str) -> io::Result<Option<Vec<u8>>> {
        let root = match Dir::open(self.paths.root()) {
            Ok(root) => root,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let dir = match self.open_dir(&root, false) {
            Ok(dir) => dir,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        match dir.read(name, RECORD_LIMIT + 1) {
            Ok(bytes) if bytes.len() > RECORD_LIMIT => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "backup record exceeds safe size",
            )),
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }
    /// Create a record that does not exist yet (owner-only, like the backups).
    /// An existing record is never replaced. `root` is the already opened
    /// settings folder.
    pub(super) fn record_create_new(&self, root: &Dir, name: &str, bytes: &[u8]) -> io::Result<()> {
        self.open_dir(root, true)?.create_new(name, bytes, 0o600)
    }
    /// Rename a record inside the backup folder. A taken name is never replaced.
    pub(super) fn record_rename(&self, root: &Dir, from: &str, to: &str) -> io::Result<()> {
        let dir = self.open_dir(root, false)?;
        if dir.metadata(to).is_ok() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "the new name is taken",
            ));
        }
        dir.rename_to(from, &dir, to)
    }
    /// Whether the backup folder has an entry of this name.
    pub(super) fn record_exists(&self, root: &Dir, name: &str) -> bool {
        self.open_dir(root, false)
            .is_ok_and(|dir| dir.metadata(name).is_ok())
    }
    /// Move `config.yaml` itself into the backups (a rename inside one volume,
    /// so its size does not matter) instead of copying it. Nothing is
    /// overwritten: a taken name gets `_2`, `_3`, ...
    pub(super) fn move_config_in(&self, root: &Dir) -> io::Result<BackupRecord> {
        let bytes = root.metadata(CONFIG_NAME)?.len();
        let dir = self.open_dir(root, true)?;
        let stamp = (self.clock)().format("%Y%m%d_%H%M%S").to_string();
        for attempt in 1..=MAX_SAME_SECOND {
            let name = if attempt == 1 {
                format!("{PREFIX}{stamp}")
            } else {
                format!("{PREFIX}{stamp}_{attempt}")
            };
            if dir.metadata(&name).is_ok() {
                continue;
            }
            match root.rename_to(CONFIG_NAME, &dir, &name) {
                Ok(()) => {
                    return self
                        .record(&name, bytes)
                        .ok_or_else(|| io::Error::other("backup name was not recognised"));
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{MAX_SAME_SECOND} backups already exist for {stamp}; nothing was moved"),
        ))
    }
    /// Undo `move_config_in` when what was meant to replace the file could not be written.
    pub(super) fn move_back_in(&self, root: &Dir, record: &BackupRecord) -> io::Result<()> {
        let dir = self.open_dir(root, false)?;
        dir.rename_to(&record.file_name, root, CONFIG_NAME)
    }
    fn record(&self, name: &str, bytes: u64) -> Option<BackupRecord> {
        let id = name.strip_prefix(PREFIX)?;
        valid_id(id).then(|| BackupRecord {
            id: id.to_owned(),
            file_name: name.to_owned(),
            path: self.directory().join(name),
            bytes,
        })
    }
    /// Copy `bytes` to a new backup. A backup is never overwritten: when the
    /// name for this second is taken, `_2`, `_3`, ... is added so both stay.
    pub fn create(&self, bytes: &[u8]) -> io::Result<BackupRecord> {
        let root = self.open_root()?;
        self.create_in(&root, bytes)
    }
    pub(super) fn create_in(&self, root: &Dir, bytes: &[u8]) -> io::Result<BackupRecord> {
        let dir = self.open_dir(root, true)?;
        let stamp = (self.clock)().format("%Y%m%d_%H%M%S").to_string();
        for attempt in 1..=MAX_SAME_SECOND {
            let name = if attempt == 1 {
                format!("{PREFIX}{stamp}")
            } else {
                format!("{PREFIX}{stamp}_{attempt}")
            };
            match dir.create_new(&name, bytes, 0o600) {
                Ok(()) => {
                    return self
                        .record(&name, bytes.len() as u64)
                        .ok_or_else(|| io::Error::other("backup name was not recognised"));
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "{MAX_SAME_SECOND} backups already exist for {stamp}; nothing was written (backups are never overwritten)"
            ),
        ))
    }
    /// Back up the current `config.yaml` as it is. `None` when there is none.
    pub fn backup_current(&self) -> io::Result<Option<BackupRecord>> {
        let root = self.open_root()?;
        self.backup_current_in(&root)
    }
    pub(super) fn backup_current_in(&self, root: &Dir) -> io::Result<Option<BackupRecord>> {
        match read_whole(root, CONFIG_NAME) {
            Ok(bytes) => self.create_in(root, &bytes).map(Some),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }
    /// Oldest first. A missing backups directory is an empty list.
    pub fn list(&self) -> io::Result<Vec<BackupRecord>> {
        let root = match Dir::open(self.paths.root()) {
            Ok(root) => root,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let dir = match self.open_dir(&root, false) {
            Ok(dir) => dir,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let mut records = Vec::new();
        for name in dir.entries()? {
            if !name.starts_with(PREFIX) {
                continue;
            }
            let Ok(metadata) = dir.metadata(&name) else {
                continue;
            };
            if let Some(record) = self.record(&name, metadata.len()) {
                records.push(record);
            }
        }
        records.sort_by_key(|record| sort_key(&record.id));
        Ok(records)
    }
    /// Accepts `20261016_123456` (or `20261016_123456_2` when several were made in
    /// one second) or the full `config.yaml.bak_...` name.
    pub fn find(&self, id: &str) -> io::Result<BackupRecord> {
        let wanted = id.trim();
        let wanted = wanted.strip_prefix(PREFIX).unwrap_or(wanted);
        if !valid_id(wanted) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a backup id looks like 20261016_123456 or 20261016_123456_2 (see `many-ai-cli config backup list`)",
            ));
        }
        self.list()?
            .into_iter()
            .find(|record| record.id == wanted)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "no backup with that id (see `many-ai-cli config backup list`)",
                )
            })
    }
    pub fn read(&self, record: &BackupRecord) -> io::Result<Vec<u8>> {
        let root = Dir::open(self.paths.root())?;
        read_whole(&self.open_dir(&root, false)?, &record.file_name)
    }
    /// Put a backup back as `config.yaml`. The settings in place now are backed
    /// up first; if that fails nothing is replaced. When the settings in place
    /// are already identical to the copy nothing is lost by replacing them, so
    /// no new backup is made (`previous` is `None`).
    pub fn restore(&self, id: &str) -> io::Result<RestoreOutcome> {
        let restored = self.find(id)?;
        let bytes = self.read(&restored)?;
        let root = self.open_root()?;
        let current = match read_whole(&root, CONFIG_NAME) {
            Ok(current) => Some(current),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        let already_identical = current.as_deref() == Some(bytes.as_slice());
        let previous = match current {
            Some(current) if !already_identical => Some(self.create_in(&root, &current)?),
            _ => None,
        };
        // The person is bringing back what a save had taken out: the notices
        // about this copy (and about any copy with the same bytes, which holds
        // the same items) are done. This is settled before the file is replaced,
        // so a failure here leaves the settings as they were; and if the
        // replacement itself fails the notices are put back.
        let settled = super::removed::settle_for_restore(self, &root, &restored, &bytes)?;
        if let Err(error) = root.replace(CONFIG_NAME, &bytes, 0o600) {
            return Err(match settled.undo(self, &root) {
                Ok(()) => error,
                Err(undo) => partial_failure(format!(
                    "the settings were not changed ({error}), but the notices about what a save took out of them could not be put back ({undo}); `many-ai-cli config check` shows what stands"
                )),
            });
        }
        Ok(RestoreOutcome {
            restored,
            previous,
            already_identical,
        })
    }
    /// Replace `config.yaml` with `bytes` after backing the current one up.
    pub(super) fn replace_config_in(
        &self,
        root: &Dir,
        bytes: &[u8],
    ) -> io::Result<Option<BackupRecord>> {
        let previous = self.backup_current_in(root)?;
        root.replace(CONFIG_NAME, bytes, 0o600)?;
        Ok(previous)
    }
    /// Move the single `config.yaml.bak` older versions wrote into the numbered
    /// backups, named after its last-modified time.
    pub fn migrate_legacy(&self) -> io::Result<Option<BackupRecord>> {
        let root = self.open_root()?;
        self.migrate_legacy_in(&root)
    }
    pub(super) fn migrate_legacy_in(&self, root: &Dir) -> io::Result<Option<BackupRecord>> {
        let metadata = match root.metadata(LEGACY_NAME) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let modified: SystemTime = metadata.modified()?;
        let stamp = chrono::DateTime::<chrono::Local>::from(modified)
            .naive_local()
            .format("%Y%m%d_%H%M%S")
            .to_string();
        let dir = self.open_dir(root, true)?;
        // Never overwrite: a taken name gets `_2`, `_3`, ... (the move itself
        // refuses to replace anything).
        for attempt in 1..=MAX_SAME_SECOND {
            let name = if attempt == 1 {
                format!("{PREFIX}{stamp}")
            } else {
                format!("{PREFIX}{stamp}_{attempt}")
            };
            if dir.metadata(&name).is_ok() {
                continue;
            }
            match root.rename_to(LEGACY_NAME, &dir, &name) {
                Ok(()) => return Ok(self.record(&name, metadata.len())),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::other(
            "no free backup name for the old config.yaml.bak",
        ))
    }
}

/// `YYYYMMDD_HHmmss`, optionally followed by `_N` (N >= 2).
pub(super) fn valid_id(id: &str) -> bool {
    let (stamp, suffix) = match id.get(..15) {
        Some(stamp) => (stamp, &id[15..]),
        None => return false,
    };
    let bytes = stamp.as_bytes();
    let stamp_ok = bytes[8] == b'_'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 8 || byte.is_ascii_digit());
    stamp_ok
        && (suffix.is_empty()
            || suffix.strip_prefix('_').is_some_and(|n| {
                !n.starts_with('0')
                    && n.bytes().all(|byte| byte.is_ascii_digit())
                    && n.parse::<usize>()
                        .is_ok_and(|n| (2..=MAX_SAME_SECOND).contains(&n))
            }))
}
/// Oldest first: by time, then by the same-second number.
pub(super) fn sort_key(id: &str) -> (String, usize) {
    let number = id.get(16..).and_then(|n| n.parse().ok()).unwrap_or(1);
    (id[..15].to_owned(), number)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn trial() -> (tempfile::TempDir, tempfile::TempDir, RuntimePaths) {
        let root = tempfile::tempdir().unwrap();
        let installed = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::trial(root.path(), 49211, installed.path()).unwrap();
        (root, installed, paths)
    }
    fn at(second: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, 16)
            .unwrap()
            .and_hms_opt(12, 30, second)
            .unwrap()
    }
    fn clock_a() -> NaiveDateTime {
        at(1)
    }
    fn clock_b() -> NaiveDateTime {
        at(2)
    }

    #[test]
    fn names_are_numbered_never_overwritten_and_kept() {
        let (_root, _installed, paths) = trial();
        let first = ConfigBackups::with_clock(&paths, clock_a);
        let a = first.create(b"synthetic one").unwrap();
        assert_eq!(a.file_name, "config.yaml.bak_20261016_123001");
        assert_eq!(a.created(), "2026-10-16 12:30:01");
        // Same second again: both stay, the first copy is untouched.
        let again = first.create(b"synthetic changed").unwrap();
        assert_eq!(again.file_name, "config.yaml.bak_20261016_123001_2");
        assert_eq!(again.id, "20261016_123001_2");
        assert_eq!(again.created(), "2026-10-16 12:30:01");
        assert_eq!(first.read(&a).unwrap(), b"synthetic one");
        assert_eq!(first.read(&again).unwrap(), b"synthetic changed");
        let third = first.create(b"synthetic third").unwrap();
        assert_eq!(third.id, "20261016_123001_3");
        let second = ConfigBackups::with_clock(&paths, clock_b);
        let b = second.create(b"synthetic two").unwrap();
        let listed = second.list().unwrap();
        assert_eq!(
            listed.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            [
                "20261016_123001",
                "20261016_123001_2",
                "20261016_123001_3",
                "20261016_123002"
            ]
        );
        assert_eq!(b.bytes, 13);
        // The same-second copies can be found by id.
        assert_eq!(second.find("20261016_123001_2").unwrap().id, again.id);
        assert!(second.find("20261016_123001_1").is_err());
        assert!(second.find("20261016_123001_02").is_err());
    }

    #[test]
    fn a_file_over_the_limit_is_an_error_never_a_cut_off_copy() {
        let (root, _installed, paths) = trial();
        let dir = Dir::open(paths.root()).unwrap();
        std::fs::write(root.path().join("big"), vec![b'x'; 3 * 1024 * 1024]).unwrap();
        // At the limit: every byte.
        assert_eq!(
            read_whole_limited(&dir, "big", 3 * 1024 * 1024)
                .unwrap()
                .len(),
            3 * 1024 * 1024
        );
        // One MiB under: refused, and the message says so.
        let error = read_whole_limited(&dir, "big", 2 * 1024 * 1024).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("nothing was copied or changed"));
        // The real limit is far above any settings file.
        const { assert!(COPY_LIMIT >= 64 * 1024 * 1024) };
    }

    #[cfg(unix)]
    #[test]
    fn backups_are_private_to_the_owner() {
        use std::os::unix::fs::PermissionsExt;
        let (_root, _installed, paths) = trial();
        let record = ConfigBackups::with_clock(&paths, clock_a)
            .create(b"x")
            .unwrap();
        let mode = std::fs::metadata(&record.path)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0);
    }

    #[test]
    fn find_accepts_the_short_and_the_full_name_and_refuses_paths() {
        let (_root, _installed, paths) = trial();
        let backups = ConfigBackups::with_clock(&paths, clock_a);
        backups.create(b"x").unwrap();
        assert!(backups.find("20261016_123001").is_ok());
        assert!(backups.find("config.yaml.bak_20261016_123001").is_ok());
        assert_eq!(
            backups.find("20261016_999999").unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        for bad in [
            "",
            "../x",
            "20261016-123001",
            "2026101_123001",
            "config.yaml",
        ] {
            assert_eq!(
                backups.find(bad).unwrap_err().kind(),
                io::ErrorKind::InvalidInput,
                "{bad}"
            );
        }
    }

    #[test]
    fn restore_backs_up_the_current_settings_first() {
        let (_root, _installed, paths) = trial();
        let config = paths.resource(Resource::Config);
        std::fs::write(&config, "token: synthetic-old\n").unwrap();
        let first = ConfigBackups::with_clock(&paths, clock_a);
        first.backup_current().unwrap().unwrap();
        std::fs::write(&config, "token: synthetic-newer\n").unwrap();
        let outcome = ConfigBackups::with_clock(&paths, clock_b)
            .restore("20261016_123001")
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(&config).unwrap(),
            "token: synthetic-old\n"
        );
        let previous = outcome.previous.unwrap();
        assert_eq!(previous.id, "20261016_123002");
        assert_eq!(
            std::fs::read_to_string(previous.path).unwrap(),
            "token: synthetic-newer\n"
        );
    }

    #[test]
    fn restore_in_the_same_second_keeps_both_copies() {
        let (_root, _installed, paths) = trial();
        let config = paths.resource(Resource::Config);
        std::fs::write(
            &config,
            "token: synthetic-old
",
        )
        .unwrap();
        let backups = ConfigBackups::with_clock(&paths, clock_a);
        backups.backup_current().unwrap();
        std::fs::write(
            &config,
            "token: synthetic-newer
",
        )
        .unwrap();
        // Same clock second: the safety copy gets `_2`; nothing is refused.
        let outcome = backups.restore("20261016_123001").unwrap();
        assert_eq!(outcome.previous.unwrap().id, "20261016_123001_2");
        assert_eq!(
            std::fs::read_to_string(&config).unwrap(),
            "token: synthetic-old
"
        );
    }

    #[test]
    fn the_old_single_backup_moves_to_a_name_from_its_modified_time() {
        let (_root, _installed, paths) = trial();
        let legacy = paths.root().join("config.yaml.bak");
        std::fs::write(&legacy, "token: synthetic-legacy\n").unwrap();
        let modified = std::fs::metadata(&legacy).unwrap().modified().unwrap();
        let expected = chrono::DateTime::<chrono::Local>::from(modified)
            .naive_local()
            .format("%Y%m%d_%H%M%S")
            .to_string();
        let moved = ConfigBackups::new(&paths)
            .migrate_legacy()
            .unwrap()
            .unwrap();
        assert_eq!(moved.id, expected);
        assert!(!legacy.exists());
        assert_eq!(
            std::fs::read_to_string(&moved.path).unwrap(),
            "token: synthetic-legacy\n"
        );
        assert!(
            ConfigBackups::new(&paths)
                .migrate_legacy()
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn migration_never_overwrites_an_existing_backup() {
        let (_root, _installed, paths) = trial();
        let legacy = paths.root().join("config.yaml.bak");
        std::fs::write(&legacy, "synthetic legacy\n").unwrap();
        let modified = std::fs::metadata(&legacy).unwrap().modified().unwrap();
        let stamp = chrono::DateTime::<chrono::Local>::from(modified).naive_local();
        let dir = paths.resource(Resource::ConfigBackups);
        std::fs::create_dir_all(&dir).unwrap();
        let taken = dir.join(format!("config.yaml.bak_{}", stamp.format("%Y%m%d_%H%M%S")));
        std::fs::write(&taken, "synthetic already here\n").unwrap();
        let moved = ConfigBackups::new(&paths)
            .migrate_legacy()
            .unwrap()
            .unwrap();
        assert_ne!(moved.path, taken);
        assert!(moved.id.ends_with("_2"), "{}", moved.id);
        assert_eq!(
            std::fs::read_to_string(&taken).unwrap(),
            "synthetic already here\n"
        );
        assert_eq!(
            std::fs::read_to_string(&moved.path).unwrap(),
            "synthetic legacy\n"
        );
    }
}
