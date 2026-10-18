//! Recount a repository's changes only when its files change (Windows;
//! plan_git-invocation-index-lock.md C7).
//!
//! Card refreshes asked Git for `status` and `diff-index` every time, changed
//! or not: 1,446 counts in 10 minutes for 8 repositories, against 31 when only
//! a change triggers one (measured 2026-10-18). Each watched repository gets
//! the operating system's change notifications. Its counts are reused until a
//! change that can alter them arrives, and taken again at least every
//! [`SAFETY`] in case a notification was lost. Inside `.git` only `index`,
//! `HEAD` and refs matter. Paths Git ignores (build output) do not; one
//! `git check-ignore` per batch decides them and the answers are remembered
//! until an exclude file changes. A directory Git reports as not ignored
//! because it holds tracked files keeps those files counted.
//!
//! A watched directory and the folders above it cannot be renamed (measured
//! 2026-10-18), the same as a running session's working directory. So only
//! repositories a live session works in are watched ([`ChangeWatch::keep`]);
//! the watch ends at the next refresh tick after the last one ends.
//!
//! Only Windows has a watcher. Elsewhere [`ChangeWatch::native`] is `None`
//! and the refresh counts every time, as before.
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// A changed repository is counted this long after its first change, once.
pub const SETTLE: Duration = Duration::from_secs(2);
/// Counts are taken again at least this often.
pub const SAFETY: Duration = Duration::from_secs(60);
const RAW_LIMIT: usize = 1024;
const KNOWN_LIMIT: usize = 8192;

pub type Counts = (i64, i64, i64);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Which watched directory a notification came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Origin {
    /// The work tree, recursively (a `.git` directory inside it included).
    WorkTree,
    /// A linked worktree's own Git directory, not recursively.
    GitDir,
    /// The common Git directory of a linked worktree, recursively.
    GitCommon,
    /// A Git directory outside the work tree with no common directory (a
    /// submodule's), recursively.
    GitFull,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Kind {
    /// Cannot change the counts.
    Skip,
    Relevant,
    /// An exclude file: relevant, and the remembered answers are void.
    Excludes,
    /// A work-tree path that matters unless Git ignores it.
    Candidate(String),
}

fn git_internal(rest: &str) -> Kind {
    match rest {
        "index" | "HEAD" | "packed-refs" | "refs" => Kind::Relevant,
        "info/exclude" => Kind::Excludes,
        _ if rest.starts_with("refs/") => Kind::Relevant,
        _ => Kind::Skip,
    }
}

/// `path` is relative to the watched directory, with `/` separators.
pub fn classify(origin: Origin, path: &str) -> Kind {
    let path = path.trim_matches('/');
    if path.is_empty() {
        return Kind::Skip;
    }
    match origin {
        Origin::WorkTree => {
            if path == ".git" {
                Kind::Skip
            } else if let Some(rest) = path.strip_prefix(".git/") {
                git_internal(rest)
            } else if path.rsplit('/').next() == Some(".gitignore") {
                Kind::Excludes
            } else {
                Kind::Candidate(path.to_owned())
            }
        }
        Origin::GitDir => match path {
            "index" | "HEAD" => Kind::Relevant,
            _ => Kind::Skip,
        },
        // The common directory's own index and HEAD belong to the main worktree.
        Origin::GitCommon if matches!(path, "index" | "HEAD") => Kind::Skip,
        Origin::GitCommon | Origin::GitFull => git_internal(path),
    }
}

/// The directories above `path` (each ending in `/`), then `path` itself.
fn candidates(path: &str) -> Vec<String> {
    let mut out: Vec<String> = path
        .match_indices('/')
        .map(|(at, _)| path[..=at].to_owned())
        .collect();
    out.push(path.to_owned());
    out
}

#[derive(Debug, PartialEq, Eq)]
enum Decision {
    Ignored,
    Kept,
    /// Git must answer for these candidates.
    Ask(Vec<String>),
}

/// Remembered `check-ignore` answers for one repository.
#[derive(Default)]
struct Excluded {
    ignored: BTreeSet<String>,
    kept: BTreeSet<String>,
}

impl Excluded {
    /// Git cannot re-include anything below an ignored directory, so the
    /// first ignored candidate from the top decides.
    fn decide(&self, path: &str) -> Decision {
        let all = candidates(path);
        for (at, candidate) in all.iter().enumerate() {
            if self.ignored.contains(candidate) {
                return Decision::Ignored;
            }
            if !self.kept.contains(candidate) {
                return Decision::Ask(all[at..].to_vec());
            }
        }
        Decision::Kept
    }

    fn learn(&mut self, asked: &BTreeSet<String>, ignored: &BTreeSet<String>) {
        if self.ignored.len() + self.kept.len() + asked.len() > KNOWN_LIMIT {
            *self = Self::default();
        }
        for candidate in asked {
            if ignored.contains(candidate) {
                self.ignored.insert(candidate.clone());
            } else {
                self.kept.insert(candidate.clone());
            }
        }
    }
}

/// Notifications not yet sorted, filled by the watcher threads.
#[derive(Default)]
pub struct Raw(Mutex<RawPaths>);

#[derive(Default)]
struct RawPaths {
    paths: BTreeSet<(Origin, String)>,
    /// Some were lost; treat the repository as changed.
    overflow: bool,
}

impl Raw {
    pub fn push(&self, origin: Origin, path: String) {
        let mut raw = lock(&self.0);
        if raw.paths.len() >= RAW_LIMIT {
            raw.overflow = true;
        } else {
            raw.paths.insert((origin, path));
        }
    }
    pub fn overflow(&self) {
        lock(&self.0).overflow = true;
    }
    fn take(&self) -> RawPaths {
        std::mem::take(&mut lock(&self.0))
    }
}

/// A directory to watch.
pub struct Dir {
    pub path: PathBuf,
    pub origin: Origin,
    pub subtree: bool,
}

/// A running watcher. Dropping it stops it; `alive` turns false if it ends
/// on its own (the directory went away), and then the counts are taken
/// every time again.
pub struct Watching {
    pub alive: Arc<AtomicBool>,
    pub stop: Box<dyn Send>,
}

pub type Start = Box<dyn Fn(&[Dir], Arc<Raw>) -> io::Result<Watching> + Send + Sync>;

struct Repo {
    raw: Arc<Raw>,
    watching: Option<Watching>,
    generation: u64,
    changed_at: Option<Instant>,
    counted: Option<(u64, Instant, Counts)>,
    excluded: Excluded,
}

impl Repo {
    fn watched(&self) -> bool {
        self.watching
            .as_ref()
            .is_some_and(|watching| watching.alive.load(Ordering::Relaxed))
    }
    fn change(&mut self, now: Instant) {
        self.generation += 1;
        self.changed_at.get_or_insert(now);
    }
}

/// What the refresh does about a repository's counts.
#[derive(Debug, PartialEq, Eq)]
pub enum Lease {
    /// Nothing that can change them happened since they were taken.
    Reuse(Counts),
    /// Count, and store the answer with the ticket when there is one.
    Count(Option<Ticket>),
}

#[derive(Debug, PartialEq, Eq)]
pub struct Ticket {
    root: PathBuf,
    generation: u64,
}

/// Paths of one repository whose ignore status Git must decide.
pub struct Pending {
    root: PathBuf,
    asked: BTreeSet<String>,
    paths: Vec<String>,
}

impl Pending {
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn asked(&self) -> Vec<String> {
        self.asked.iter().cloned().collect()
    }
}

pub struct ChangeWatch {
    repos: Mutex<BTreeMap<PathBuf, Repo>>,
    roots: Mutex<BTreeMap<String, (Option<PathBuf>, Instant)>>,
    settle: Duration,
    safety: Duration,
    start: Start,
}

impl ChangeWatch {
    /// The watcher for this system, or `None` where there is none.
    pub fn native() -> Option<std::sync::Arc<Self>> {
        #[cfg(windows)]
        {
            Some(Arc::new(Self::new(
                SETTLE,
                SAFETY,
                Box::new(platform::start),
            )))
        }
        #[cfg(not(windows))]
        {
            None
        }
    }

    pub fn new(settle: Duration, safety: Duration, start: Start) -> Self {
        Self {
            repos: Mutex::new(BTreeMap::new()),
            roots: Mutex::new(BTreeMap::new()),
            settle,
            safety,
            start,
        }
    }

    /// The work tree root holding `cwd`, looked up again after [`SAFETY`].
    pub fn root_of(&self, cwd: &str) -> Option<PathBuf> {
        let now = Instant::now();
        if let Some((root, at)) = lock(&self.roots).get(cwd)
            && now.duration_since(*at) < self.safety
        {
            return root.clone();
        }
        let path = Path::new(cwd);
        let root = path
            .is_dir()
            .then(|| super::git_lock::locate(path))
            .flatten()
            .map(|(root, _)| root);
        let mut roots = lock(&self.roots);
        if roots.len() >= 1024 {
            roots.clear();
        }
        roots.insert(cwd.to_owned(), (root.clone(), now));
        root
    }

    /// The directories that hold a repository's counts: the work tree, and
    /// the Git directories a linked worktree or submodule keeps elsewhere.
    fn dirs(root: &Path) -> Option<Vec<Dir>> {
        let (root, git_dir) = super::git_lock::locate(root)?;
        let mut dirs = vec![Dir {
            path: root.clone(),
            origin: Origin::WorkTree,
            subtree: true,
        }];
        if git_dir.starts_with(&root) {
            return Some(dirs);
        }
        let common = std::fs::read_to_string(git_dir.join("commondir"))
            .ok()
            .map(|text| git_dir.join(text.trim()));
        match common {
            Some(common) => {
                dirs.push(Dir {
                    path: git_dir,
                    origin: Origin::GitDir,
                    subtree: false,
                });
                if !common.starts_with(&root) {
                    dirs.push(Dir {
                        path: common,
                        origin: Origin::GitCommon,
                        subtree: true,
                    });
                }
            }
            None => dirs.push(Dir {
                path: git_dir,
                origin: Origin::GitFull,
                subtree: true,
            }),
        }
        Some(dirs)
    }

    fn open(&self, root: &Path) -> Repo {
        let raw = Arc::new(Raw::default());
        let watching = Self::dirs(root).and_then(|dirs| (self.start)(&dirs, raw.clone()).ok());
        Repo {
            raw,
            watching,
            generation: 0,
            changed_at: None,
            counted: None,
            excluded: Excluded::default(),
        }
    }

    /// Whether the counts for `cwd` must be taken. Starts watching its
    /// repository the first time, and again after a watch could not start
    /// or ended.
    pub fn lease(&self, cwd: &str) -> Lease {
        let Some(root) = self.root_of(cwd) else {
            return Lease::Count(None);
        };
        let now = Instant::now();
        let mut repos = lock(&self.repos);
        let repo = repos
            .entry(root.clone())
            .or_insert_with(|| self.open(&root));
        if !repo.watched() {
            repos.remove(&root);
            return Lease::Count(None);
        }
        if let Some((generation, at, counts)) = repo.counted
            && generation == repo.generation
            && now.duration_since(at) < self.safety
        {
            return Lease::Reuse(counts);
        }
        Lease::Count(Some(Ticket {
            root,
            generation: repo.generation,
        }))
    }

    /// Stop watching repositories no live session works in. Dropping a
    /// watch releases its directories at once.
    pub fn keep(&self, roots: &BTreeSet<PathBuf>) {
        lock(&self.repos).retain(|root, _| roots.contains(root));
    }

    /// Keep counts Git answered, for the repository state the ticket saw.
    pub fn store(&self, ticket: Ticket, counts: Counts) {
        if let Some(repo) = lock(&self.repos).get_mut(&ticket.root)
            && repo
                .counted
                .is_none_or(|(generation, ..)| generation <= ticket.generation)
        {
            repo.counted = Some((ticket.generation, Instant::now(), counts));
        }
    }

    /// Sort the notifications that arrived. Returns what Git must decide,
    /// one batch per repository; everything else is settled here.
    pub fn take(&self) -> Vec<Pending> {
        let now = Instant::now();
        let mut out = Vec::new();
        let mut repos = lock(&self.repos);
        for (root, repo) in repos.iter_mut() {
            let raw = repo.raw.take();
            let mut changed = raw.overflow;
            let mut asked = BTreeSet::new();
            let mut paths = Vec::new();
            for (origin, path) in raw.paths {
                if changed {
                    break;
                }
                match classify(origin, &path) {
                    Kind::Skip => {}
                    Kind::Relevant => changed = true,
                    Kind::Excludes => {
                        repo.excluded = Excluded::default();
                        changed = true;
                    }
                    Kind::Candidate(path) => match repo.excluded.decide(&path) {
                        Decision::Ignored => {}
                        Decision::Kept => changed = true,
                        Decision::Ask(candidates) => {
                            asked.extend(candidates);
                            paths.push(path);
                        }
                    },
                }
            }
            if changed {
                repo.change(now);
            } else if !paths.is_empty() {
                out.push(Pending {
                    root: root.clone(),
                    asked,
                    paths,
                });
            }
        }
        out
    }

    /// Git's answer for a batch from [`take`](Self::take); `None` when Git
    /// did not answer, which counts as a change.
    pub fn learn(&self, pending: Pending, ignored: Option<BTreeSet<String>>) {
        let now = Instant::now();
        let mut repos = lock(&self.repos);
        let Some(repo) = repos.get_mut(&pending.root) else {
            return;
        };
        let Some(ignored) = ignored else {
            repo.change(now);
            return;
        };
        repo.excluded.learn(&pending.asked, &ignored);
        if pending
            .paths
            .iter()
            .any(|path| repo.excluded.decide(path) != Decision::Ignored)
        {
            repo.change(now);
        }
    }

    /// Repositories whose first change since the last call is [`SETTLE`]
    /// old. Their sessions should be counted now.
    pub fn due(&self) -> Vec<PathBuf> {
        let now = Instant::now();
        lock(&self.repos)
            .iter_mut()
            .filter_map(|(root, repo)| {
                let at = repo.changed_at?;
                (now.duration_since(at) >= self.settle).then(|| {
                    repo.changed_at = None;
                    root.clone()
                })
            })
            .collect()
    }
}

/// One `FILE_NOTIFY_INFORMATION` list: offset of the next entry, action,
/// name length in bytes, then the UTF-16 name relative to the directory.
#[cfg_attr(not(windows), allow(dead_code))]
fn parse_notifications(bytes: &[u8], origin: Origin, raw: &Raw) {
    let word = |at: usize| {
        bytes
            .get(at..at + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
    };
    let mut at = 0;
    loop {
        let (Some(next), Some(length)) = (word(at), word(at + 8)) else {
            raw.overflow();
            return;
        };
        let Some(name) = bytes.get(at + 12..at + 12 + length) else {
            raw.overflow();
            return;
        };
        let units: Vec<u16> = name
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        raw.push(origin, String::from_utf16_lossy(&units).replace('\\', "/"));
        if next == 0 {
            return;
        }
        at += next;
    }
}

#[cfg(windows)]
mod platform {
    use super::{Dir, Origin, Raw, Watching};
    use std::{
        io,
        os::windows::{ffi::OsStrExt, io::AsRawHandle},
        path::Path,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0},
        Storage::FileSystem::{
            CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED, FILE_LIST_DIRECTORY,
            FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME,
            FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE, FILE_SHARE_DELETE,
            FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, ReadDirectoryChangesW,
        },
        System::{
            IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED},
            Threading::{
                CreateEventW, INFINITE, ResetEvent, SetEvent, WaitForMultipleObjects,
                WaitForSingleObject,
            },
        },
    };

    /// 64 KiB, the most a network share returns.
    const BUFFER_WORDS: usize = 16 * 1024;
    const FILTER: u32 = FILE_NOTIFY_CHANGE_FILE_NAME
        | FILE_NOTIFY_CHANGE_DIR_NAME
        | FILE_NOTIFY_CHANGE_SIZE
        | FILE_NOTIFY_CHANGE_LAST_WRITE;

    struct Handle(HANDLE);
    // SAFETY: a kernel handle may be used and closed from any thread.
    unsafe impl Send for Handle {}
    // SAFETY: as above; the only shared use is SetEvent/WaitForMultipleObjects.
    unsafe impl Sync for Handle {}
    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: the handle came from a successful create and is closed once.
            unsafe { CloseHandle(self.0) };
        }
    }

    fn event() -> io::Result<Handle> {
        // SAFETY: an unnamed manual-reset event with default security.
        let event = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
        if event.is_null() {
            Err(io::Error::last_os_error())
        } else {
            Ok(Handle(event))
        }
    }

    /// Stops the watcher threads of one repository when dropped, and waits
    /// (up to a second each) until they closed their directories.
    struct Stop {
        event: Arc<Handle>,
        threads: Vec<std::thread::JoinHandle<()>>,
    }
    impl Drop for Stop {
        fn drop(&mut self) {
            // SAFETY: the event outlives this call through the Arc.
            unsafe { SetEvent(self.event.0) };
            for thread in &self.threads {
                // SAFETY: the thread handle stays open while the JoinHandle lives.
                unsafe { WaitForSingleObject(thread.as_raw_handle() as HANDLE, 1000) };
            }
        }
    }

    /// One directory and its outstanding change request. The buffer and the
    /// OVERLAPPED stay on the heap, so moving the reader to its thread does
    /// not move what the kernel writes to.
    struct Reader {
        dir: Handle,
        event: Handle,
        overlapped: Box<OVERLAPPED>,
        buffer: Vec<u32>,
        subtree: bool,
        pending: bool,
    }
    // SAFETY: the handles and the heap buffers are owned by the reader and
    // used by one thread at a time.
    unsafe impl Send for Reader {}

    impl Reader {
        fn open(path: &Path, subtree: bool) -> io::Result<Self> {
            let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
            // SAFETY: `wide` is NUL-terminated; the directory is opened for
            // change requests only and shares everything with other users.
            let dir = unsafe {
                CreateFileW(
                    wide.as_ptr(),
                    FILE_LIST_DIRECTORY,
                    FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                    std::ptr::null(),
                    OPEN_EXISTING,
                    FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
                    std::ptr::null_mut(),
                )
            };
            if dir == INVALID_HANDLE_VALUE {
                return Err(io::Error::last_os_error());
            }
            let dir = Handle(dir);
            let event = event()?;
            // SAFETY: an all-zero OVERLAPPED is its documented initial state.
            let mut overlapped: Box<OVERLAPPED> = Box::new(unsafe { std::mem::zeroed() });
            overlapped.hEvent = event.0;
            let mut reader = Self {
                dir,
                event,
                overlapped,
                buffer: vec![0; BUFFER_WORDS],
                subtree,
                pending: false,
            };
            // File systems without change notifications fail here.
            reader.arm()?;
            Ok(reader)
        }

        fn arm(&mut self) -> io::Result<()> {
            // SAFETY: the event belongs to this reader.
            unsafe { ResetEvent(self.event.0) };
            // SAFETY: the buffer and OVERLAPPED live on the heap until the
            // request completes or is cancelled (see Drop).
            let ok = unsafe {
                ReadDirectoryChangesW(
                    self.dir.0,
                    self.buffer.as_mut_ptr().cast(),
                    (self.buffer.len() * 4) as u32,
                    self.subtree.into(),
                    FILTER,
                    std::ptr::null_mut(),
                    &mut *self.overlapped,
                    None,
                )
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            self.pending = true;
            Ok(())
        }

        fn run(mut self, origin: Origin, raw: Arc<Raw>, stop: Arc<Handle>, alive: Arc<AtomicBool>) {
            loop {
                let handles = [self.event.0, stop.0];
                // SAFETY: both handles are open for the duration of the wait.
                let woke = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, INFINITE) };
                if woke != WAIT_OBJECT_0 {
                    break;
                }
                let mut bytes = 0u32;
                // SAFETY: the request signalled completion; no wait.
                let done =
                    unsafe { GetOverlappedResult(self.dir.0, &*self.overlapped, &mut bytes, 0) };
                self.pending = false;
                if done == 0 {
                    break;
                }
                if bytes == 0 {
                    raw.overflow();
                } else {
                    // SAFETY: the kernel filled `bytes` bytes of the buffer.
                    let filled = unsafe {
                        std::slice::from_raw_parts(
                            self.buffer.as_ptr().cast::<u8>(),
                            (bytes as usize).min(self.buffer.len() * 4),
                        )
                    };
                    super::parse_notifications(filled, origin, &raw);
                }
                if self.arm().is_err() {
                    break;
                }
            }
            alive.store(false, Ordering::Relaxed);
        }
    }

    impl Drop for Reader {
        fn drop(&mut self) {
            if self.pending {
                let mut bytes = 0u32;
                // SAFETY: cancel the outstanding request and wait for it to
                // finish before the buffer and OVERLAPPED are freed.
                unsafe {
                    CancelIoEx(self.dir.0, &*self.overlapped);
                    GetOverlappedResult(self.dir.0, &*self.overlapped, &mut bytes, 1);
                }
            }
        }
    }

    pub fn start(dirs: &[Dir], raw: Arc<Raw>) -> io::Result<Watching> {
        let stop = Arc::new(event()?);
        let alive = Arc::new(AtomicBool::new(true));
        let readers = dirs
            .iter()
            .map(|dir| Reader::open(&dir.path, dir.subtree).map(|reader| (reader, dir.origin)))
            .collect::<io::Result<Vec<_>>>()?;
        let mut guard = Stop {
            event: stop.clone(),
            threads: Vec::new(),
        };
        for (reader, origin) in readers {
            let (raw, stop, alive) = (raw.clone(), stop.clone(), alive.clone());
            // On failure the guard stops the threads already started.
            guard.threads.push(
                std::thread::Builder::new()
                    .name("git-change-watch".into())
                    .spawn(move || reader.run(origin, raw, stop, alive))?,
            );
        }
        Ok(Watching {
            alive,
            stop: Box::new(guard),
        })
    }
}

#[cfg(test)]
#[path = "change_watch_tests.rs"]
mod tests;
