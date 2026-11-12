//! The copy of a file's earlier bytes that a save keeps while it writes (U18),
//! and what happens to a copy the save never came back for (audit N16).
//!
//! A save copies the file into `<Temporary>/save-backups/<id>.bak` and writes
//! `<id>.json` beside it: the file's path, the folder the save was allowed
//! from, the time and the size. A save that finishes removes both. A Hub that
//! stops in the middle of the write leaves both, and the file half written.
//! The next Hub lists such copies for the UI, and the user puts the earlier
//! content back or deletes the copy. Nothing here is removed by age until the
//! user has been shown it.
use super::{Result, content::text_file, err, safe_fs::Dir, scope::Scope};
use crate::{
    config::paths::{Resource, RuntimePaths},
    hub::http::{Request, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs::File,
    io::{self, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard, PoisonError},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub(super) const LIST: &str = "/api/files-save-leftovers";
pub(super) const RESTORE: &str = "/api/files-save-leftover-restore";
pub(super) const DISCARD: &str = "/api/files-save-leftover-discard";

/// A copy is kept this long after the user was first shown it.
const KEEP: Duration = Duration::from_secs(7 * 24 * 3600);
/// A record whose copy is missing is removed once it is this old; younger, it
/// may belong to a save that is just starting.
const ORPHAN: Duration = Duration::from_secs(3600);
const RECORD_MAX: usize = 64 * 1024;

/// The copies saves of this process are using right now. They are not
/// leftovers: never listed, never removed by anyone but their save.
static ACTIVE: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());
fn active() -> MutexGuard<'static, BTreeSet<PathBuf>> {
    ACTIVE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What is written beside a copy, so that it can be matched to its file.
#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct Record {
    /// The file the copy was taken from.
    path: String,
    /// The folder the save was allowed from. Putting the copy back is checked
    /// against the same folder again.
    cwd: String,
    saved_at: String,
    size: u64,
    /// When the copy was first listed for the user, in Unix seconds; 0 until
    /// then.
    notified: u64,
}

fn folder(paths: &RuntimePaths) -> PathBuf {
    paths.resource(Resource::Temporary).join("save-backups")
}
fn copy_name(id: &str) -> String {
    format!("{id}.bak")
}
fn record_name(id: &str) -> String {
    format!("{id}.json")
}
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric())
}
fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
fn read_record(dir: &Dir, id: &str) -> Option<Record> {
    serde_json::from_slice(&dir.read(&record_name(id), RECORD_MAX).ok()?).ok()
}
fn remove(dir: &Dir, id: &str) {
    let _ = dir.remove_file(&copy_name(id));
    let _ = dir.remove_file(&record_name(id));
}

/// Removes the copies the user was shown more than `KEEP` ago, and records
/// whose copy is gone. A copy nobody was shown stays, however old.
fn expire(dir: &Dir) {
    let now = unix_now();
    let names = dir.entries().unwrap_or_default();
    for name in &names {
        let Some(id) = name.strip_suffix(".json") else {
            continue;
        };
        if !valid_id(id) || active().contains(&dir.path().join(id)) {
            continue;
        }
        let shown_long_ago = read_record(dir, id)
            .is_some_and(|r| r.notified != 0 && now.saturating_sub(r.notified) > KEEP.as_secs());
        let orphan = !names.contains(&copy_name(id))
            && dir
                .metadata(name)
                .ok()
                .and_then(|meta| meta.modified().ok())
                .and_then(|at| SystemTime::now().duration_since(at).ok())
                .is_some_and(|age| age > ORPHAN);
        if shown_long_ago || orphan {
            remove(dir, id);
        }
    }
}

pub(super) struct SaveBackup {
    dir: Dir,
    id: String,
    file: Option<File>,
}
impl SaveBackup {
    pub(super) fn take(original: &mut File, path: &Path, s: &Scope) -> io::Result<Self> {
        let dir = Dir::open_or_create_private_components(&folder(&s.paths))?;
        expire(&dir);
        let id = crate::process::random_token()?[..24].to_owned();
        active().insert(dir.path().join(&id));
        let mut backup = Self {
            dir,
            id,
            file: None,
        };
        match backup.fill(original, path, s) {
            Ok(()) => Ok(backup),
            Err(error) => {
                backup.discard();
                Err(error)
            }
        }
    }
    fn fill(&mut self, original: &mut File, path: &Path, s: &Scope) -> io::Result<()> {
        let record = Record {
            path: path.to_string_lossy().into_owned(),
            cwd: s.cwd.to_string_lossy().into_owned(),
            saved_at: super::time::format_utc(SystemTime::now())
                .map_err(|_| io::Error::other("the clock cannot be written as a time"))?,
            size: original.metadata()?.len(),
            notified: 0,
        };
        // The record first: a copy is never on disk without it.
        self.dir
            .create_new(&record_name(&self.id), &serde_json::to_vec(&record)?, 0o600)?;
        self.dir.create_new(&copy_name(&self.id), &[], 0o600)?;
        let mut file = self.dir.open_file(&copy_name(&self.id), true)?;
        original.seek(SeekFrom::Start(0))?;
        io::copy(original, &mut file)?;
        file.sync_all()?;
        self.file = Some(file);
        Ok(())
    }
    pub(super) fn restore(&mut self, original: &mut File) -> io::Result<()> {
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other("the copy was not written"))?;
        file.seek(SeekFrom::Start(0))?;
        original.seek(SeekFrom::Start(0))?;
        let length = io::copy(file, original)?;
        original.set_len(length)?;
        original.sync_all()
    }
    pub(super) fn path(&self) -> PathBuf {
        self.dir.path().join(copy_name(&self.id))
    }
    pub(super) fn discard(mut self) {
        // Closed first: Windows does not remove an open file.
        self.file.take();
        remove(&self.dir, &self.id);
    }
}
impl Drop for SaveBackup {
    /// A save that ends without `discard` could not put the file back. Its
    /// copy stays on disk and is a leftover from here on.
    fn drop(&mut self) {
        active().remove(&self.dir.path().join(&self.id));
    }
}

fn list(paths: &RuntimePaths) -> io::Result<Vec<Value>> {
    let folder = folder(paths);
    if !folder.is_dir() {
        return Ok(Vec::new());
    }
    let dir = Dir::open_or_create_private_components(&folder)?;
    expire(&dir);
    let mut leftovers = Vec::new();
    for name in dir.entries()? {
        let Some(id) = name.strip_suffix(".bak") else {
            continue;
        };
        if !valid_id(id) || active().contains(&dir.path().join(id)) {
            continue;
        }
        let Ok(meta) = dir.metadata(&name) else {
            continue;
        };
        // A copy from before records were written has none: it is listed
        // without its file, and can only be deleted.
        let mut record = read_record(&dir, id).unwrap_or_else(|| Record {
            saved_at: meta
                .modified()
                .ok()
                .and_then(|at| super::time::format_utc(at).ok())
                .unwrap_or_default(),
            size: meta.len(),
            ..Default::default()
        });
        if record.notified == 0 {
            // Listed here is shown to the user: the days of `KEEP` start now.
            record.notified = unix_now();
            let _ = dir.replace(&record_name(id), &serde_json::to_vec(&record)?, 0o600);
        }
        leftovers.push(json!({
            "id": id,
            "path": record.path,
            "saved_at": record.saved_at,
            "size": record.size,
            "backup": dir.path().join(&name),
        }));
    }
    Ok(leftovers)
}

fn write_over(file: &mut File, from: &mut File) -> io::Result<()> {
    file.seek(SeekFrom::Start(0))?;
    let length = io::copy(from, file)?;
    file.set_len(length)?;
    file.sync_all()
}

/// Writes the copy over its file, under the rules of a save: the path is
/// checked against the folder the save was allowed from, the file is locked,
/// and the content being replaced is itself copied until the write is done.
fn put_back(dir: &Dir, id: &str, paths: &RuntimePaths) -> Result<()> {
    let kept_at = dir.path().join(copy_name(id));
    let record = read_record(dir, id)
        .filter(|r| !r.path.is_empty() && !r.cwd.is_empty())
        .ok_or_else(|| {
            err(
                409,
                "no_record",
                &format!(
                    "this copy has no record of its file; it is kept at {}",
                    kept_at.display()
                ),
            )
        })?;
    let path = PathBuf::from(&record.path);
    let scope = Scope::new(PathBuf::from(&record.cwd), paths.clone());
    let (parent, name) = scope.write_parent(&path)?;
    if !text_file(&path) {
        return Err(err(403, "forbidden", "not a previewable text file"));
    }
    let mut file = parent.open_file(&name, true).map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            err(
                404,
                "file_gone",
                &format!(
                    "the file is no longer there; its earlier content is kept at {}",
                    kept_at.display()
                ),
            )
        } else {
            super::io_error(e, "open_failed")
        }
    })?;
    super::mutation::lock_for_save(&file)?;
    let mut earlier = dir
        .open_file(&copy_name(id), false)
        .map_err(|e| super::io_error(e, "open_failed"))?;
    let mut replaced = SaveBackup::take(&mut file, &path, &scope)
        .map_err(|e| super::io_error(e, "backup_failed"))?;
    match write_over(&mut file, &mut earlier) {
        Ok(()) => {
            replaced.discard();
            Ok(())
        }
        Err(error) => {
            if replaced.restore(&mut file).is_ok() {
                replaced.discard();
                return Err(super::io_error(error, "write_failed"));
            }
            Err(err(
                500,
                "write_failed",
                &format!(
                    "write_failed: {error}; the file could not be put back, and its content from before this attempt is kept at {}",
                    replaced.path().display()
                ),
            ))
        }
    }
}

pub(super) fn handle(r: &Request, paths: &RuntimePaths) -> Result<Response> {
    if r.path == LIST {
        let leftovers = list(paths).map_err(|e| super::io_error(e, "list_failed"))?;
        return Ok(Response::json(200, &json!({"leftovers": leftovers})));
    }
    #[derive(Deserialize)]
    struct Body {
        id: String,
    }
    let body: Body = serde_json::from_slice(&r.body[..r.body.len().min(4096)])
        .map_err(|_| err(400, "bad_request", "id is required"))?;
    if !valid_id(&body.id) {
        return Err(err(400, "bad_request", "invalid id"));
    }
    let gone = || err(404, "leftover_gone", "the copy is no longer there");
    let folder = folder(paths);
    if !folder.is_dir() {
        return Err(gone());
    }
    let dir = Dir::open_or_create_private_components(&folder)
        .map_err(|e| super::io_error(e, "open_failed"))?;
    if active().contains(&dir.path().join(&body.id)) {
        return Err(err(409, "save_running", "a save is using this copy"));
    }
    if dir.metadata(&copy_name(&body.id)).is_err() {
        return Err(gone());
    }
    if r.path == RESTORE {
        put_back(&dir, &body.id, paths)?;
    }
    remove(&dir, &body.id);
    Ok(Response::json(200, &json!({"ok": true})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    struct Fixture {
        scope: Scope,
        project: PathBuf,
        backups: PathBuf,
        _temp: tempfile::TempDir,
    }
    fn fixture() -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        let runtime = temp.path().join("runtime");
        std::fs::create_dir(&project).unwrap();
        std::fs::create_dir(&runtime).unwrap();
        let paths = RuntimePaths::trial(&runtime, 49123, &temp.path().join("installed")).unwrap();
        let backups = folder(&paths);
        Fixture {
            scope: Scope::new(project.clone(), paths),
            project,
            backups,
            _temp: temp,
        }
    }
    /// What a Hub that stopped in the middle of a save leaves: the file half
    /// written, and the copy and its record on disk.
    fn stopped_mid_save(f: &Fixture, name: &str, earlier: &[u8], half: &[u8]) -> PathBuf {
        let path = f.project.join(name);
        std::fs::write(&path, earlier).unwrap();
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        let backup = SaveBackup::take(&mut file, &path, &f.scope).unwrap();
        assert_eq!(
            leftovers(f),
            Vec::<Value>::new(),
            "a running save is not a leftover"
        );
        file.seek(SeekFrom::Start(0)).unwrap();
        file.write_all(half).unwrap();
        // The process is gone: nothing discards the copy.
        drop(backup);
        path
    }
    fn call(f: &Fixture, route: &str, body: Value) -> Response {
        handle(
            &Request {
                method: if route == LIST { "GET" } else { "POST" }.into(),
                path: route.into(),
                body: serde_json::to_vec(&body).unwrap(),
                ..Default::default()
            },
            &f.scope.paths,
        )
        .unwrap_or_else(|e| e)
    }
    fn leftovers(f: &Fixture) -> Vec<Value> {
        let response = call(f, LIST, json!({}));
        assert_eq!(response.status, 200);
        serde_json::from_slice::<Value>(&response.body).unwrap()["leftovers"]
            .as_array()
            .unwrap()
            .clone()
    }
    fn kept(f: &Fixture) -> usize {
        std::fs::read_dir(&f.backups).map_or(0, |d| d.count())
    }
    fn age(path: &Path, days: u64) {
        let at = SystemTime::now() - Duration::from_secs(days * 24 * 3600);
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(at)
            .unwrap();
    }
    fn save(f: &Fixture, name: &str) {
        let path = f.project.join(name);
        std::fs::write(&path, b"another synthetic file").unwrap();
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        SaveBackup::take(&mut file, &path, &f.scope)
            .unwrap()
            .discard();
    }

    #[test]
    fn audit_n16_a_save_a_stopped_hub_left_half_done_is_listed_with_its_file_and_put_back() {
        let f = fixture();
        let earlier = b"earlier synthetic content, longer than the half that was written";
        let path = stopped_mid_save(&f, "fixture.md", earlier, b"half of the new");
        assert_ne!(std::fs::read(&path).unwrap(), earlier);

        let listed = leftovers(&f);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["path"], json!(path));
        assert_eq!(listed[0]["size"], json!(earlier.len()));
        assert!(!listed[0]["saved_at"].as_str().unwrap().is_empty());
        let copy = PathBuf::from(listed[0]["backup"].as_str().unwrap());
        assert_eq!(std::fs::read(&copy).unwrap(), earlier);

        let id = listed[0]["id"].clone();
        assert_eq!(call(&f, RESTORE, json!({"id": id})).status, 200);
        assert_eq!(std::fs::read(&path).unwrap(), earlier);
        assert_eq!(kept(&f), 0);
        assert_eq!(leftovers(&f), Vec::<Value>::new());
        assert_eq!(call(&f, RESTORE, json!({"id": id})).status, 404);
    }

    #[test]
    fn audit_n16_a_copy_is_removed_by_age_only_after_the_user_was_shown_it() {
        let f = fixture();
        stopped_mid_save(&f, "fixture.md", b"earlier synthetic content", b"half");
        for entry in std::fs::read_dir(&f.backups).unwrap() {
            age(&entry.unwrap().path(), 30);
        }
        // Nobody has been shown the copy: a later save leaves it, however old.
        save(&f, "other.md");
        assert_eq!(kept(&f), 2);

        let listed = leftovers(&f);
        assert_eq!(listed.len(), 1);
        let id = listed[0]["id"].as_str().unwrap().to_owned();
        // Shown a moment ago: still kept.
        save(&f, "other.md");
        assert_eq!(kept(&f), 2);

        // Shown eight days ago.
        let record_path = f.backups.join(record_name(&id));
        let mut record: Record =
            serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
        assert_ne!(record.notified, 0);
        record.notified = unix_now() - 8 * 24 * 3600;
        std::fs::write(&record_path, serde_json::to_vec(&record).unwrap()).unwrap();
        save(&f, "other.md");
        assert_eq!(kept(&f), 0);
    }

    #[test]
    fn audit_n16_a_copy_can_be_deleted_and_one_without_a_record_is_not_written_anywhere() {
        let f = fixture();
        let path = stopped_mid_save(&f, "fixture.md", b"earlier synthetic content", b"half");
        let id = leftovers(&f)[0]["id"].clone();
        assert_eq!(call(&f, DISCARD, json!({"id": "../outside"})).status, 400);
        assert_eq!(call(&f, DISCARD, json!({"id": "unknown0"})).status, 404);
        assert_eq!(call(&f, DISCARD, json!({"id": id})).status, 200);
        assert_eq!(kept(&f), 0);
        assert_eq!(std::fs::read(&path).unwrap()[..4], *b"half");

        // A copy an earlier version left: no record says which file it was.
        std::fs::write(f.backups.join("0123456789abcdef01234567.bak"), b"old copy").unwrap();
        let listed = leftovers(&f);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["path"], "");
        assert_eq!(listed[0]["size"], 8);
        let id = listed[0]["id"].clone();
        let refused = call(&f, RESTORE, json!({"id": id}));
        assert_eq!(refused.status, 409);
        assert_eq!(
            serde_json::from_slice::<Value>(&refused.body).unwrap()["error"],
            "no_record"
        );
        assert_eq!(kept(&f), 2, "the copy and the record of its listing");
        assert_eq!(call(&f, DISCARD, json!({"id": id})).status, 200);
        assert_eq!(kept(&f), 0);
    }
}
