//! Optional progress writes to an MCP board (for example Many Iris).
//!
//! Off unless `<many-ai-cli root>/board.json` exists, so people who use only
//! many-ai-cli see no change. Only sessions whose repository (or working
//! directory) is inside a listed path are written; nothing else leaves the
//! machine. The writer key is read from `key_file` for every send and is never
//! logged or stored in the many-ai-cli config.
//!
//! ```json
//! {
//!   "schema_version": 1,
//!   "url": "https://board.example.com/board/mcp",
//!   "key_file": "C:\\keys\\board-writer.key",
//!   "repos": [{ "path": "C:\\work\\my-app", "project": "my-app" }]
//! }
//! ```
//!
//! A session's work is one item (`session/<id>`, kind `task`): a
//! `session_start` event when it starts, a `note` with the DONE summary after
//! each finished turn, and `session_end` (state `done`) when it ends. The first
//! summary names the item (date, provider, first sentence).
//!
//! A summary may end with `計画: <path or URL>` and `成果: <https URL>` (the
//! rules in `wrapper::approval_injection` ask for them). The plan is matched
//! against the `source_ref` of the board's items and the work is put under the
//! match; a plan file that exists in the repository but has no item yet gets
//! one (title from its first heading). When later summaries name another plan,
//! the work continues in a new item (`session/<id>/2`, ...), so each plan shows
//! the work done for it. The outcome URL becomes the link of that note. A
//! summary without one, which names a commit by its hash, gets that commit's
//! GitHub page as its link once the commit is on `origin` (asked of Git, see
//! `commit_link`). What leaves the machine stays the same kind of thing: titles,
//! states, summaries and references, never file contents.
//!
//! Writes are best effort, in order, one at a time; a failure is logged and
//! dropped.
use crate::{
    files::branch::GitBranchSource,
    hub::task_owner::HubTaskHandle,
    proto::{DoneSummary, core::*},
    storage::mask_secrets,
};
use serde::Deserialize;
use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

pub const FILE_NAME: &str = "board.json";
const TITLE_MAX: usize = 200;
const TEXT_MAX: usize = 2000;
const PROJECT_MAX: usize = 80;
const QUEUE_MAX: usize = 256;
/// Characters of the first sentence kept in a work title.
const SENTENCE_MAX: usize = 80;
/// The board's limit for a reference (path or URL).
const REF_MAX: usize = 500;
/// Hashes in one summary that are asked of Git, in the order written.
const HASHES_MAX: usize = 3;
/// A plan file is read this far for its first heading.
const PLAN_HEAD_MAX: u64 = 64 * 1024;
/// `board_list` returns only events after `since_seq`; this asks for none, so
/// the reply holds the items alone.
const NO_EVENTS: u64 = 9_007_199_254_740_991;
/// A tools/call reply is a few KiB. A longer one is refused rather than read,
/// so a wrong or hostile board cannot exhaust the Hub's memory (U29).
const RESPONSE_MAX: usize = 1024 * 1024;

pub type Warning = dyn Fn(&str, &str) + Send + Sync;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    schema_version: u32,
    url: String,
    key_file: PathBuf,
    repos: Vec<RepoEntry>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RepoEntry {
    path: PathBuf,
    #[serde(default)]
    project: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
struct Target {
    url: String,
    key_file: PathBuf,
    project: String,
    /// The session's first item; later pieces of its work append `/<n>`.
    item_id: String,
}

/// One listed session: where its writes go and what names and links its work.
#[derive(Clone)]
struct Seat {
    target: Target,
    /// Repository root (or working directory) a declared plan path is relative to.
    root: PathBuf,
    provider: String,
    /// Title until the first summary names the work.
    opening: String,
}

/// The piece of work a session is on. Owned by the worker.
#[derive(Clone, Debug, PartialEq)]
struct Piece {
    item_id: String,
    number: u32,
    /// The plan this piece's summaries named.
    plan: Option<String>,
    /// The plan has been looked up (`false` while the board could not be read).
    settled: bool,
    /// The item sits under a plan's item on the board.
    linked: bool,
    /// The item is on the board (`false` for a new piece until its first write).
    exists: bool,
    titled: bool,
}
impl Piece {
    fn first(item_id: &str) -> Self {
        Self {
            item_id: item_id.to_owned(),
            number: 1,
            plan: None,
            settled: true,
            linked: false,
            exists: true,
            titled: false,
        }
    }
}

/// An item as `board_list` returns it (only what linking needs).
#[derive(Deserialize)]
struct Listed {
    item_id: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    source_ref: Option<String>,
    #[serde(default)]
    parent_id: Option<String>,
}

enum Lookup {
    Found(String),
    /// Nothing to put the work under; do not ask again for this piece.
    Nothing,
    /// The board could not be read; ask again with the next summary.
    Unread,
}

enum Failed {
    /// No usable reply (key, network, size).
    Unreached,
    /// The board answered and said no.
    Refused,
}

enum Job {
    Start {
        session: LiveSessionId,
        seat: Seat,
        text: String,
    },
    Note {
        session: LiveSessionId,
        seat: Seat,
        kind: &'static str,
        summary: String,
    },
    End {
        session: LiveSessionId,
        target: Target,
        text: String,
    },
}

fn lock<T>(value: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    value.lock().unwrap_or_else(|error| error.into_inner())
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Compares paths as written, ignoring separators and (on Windows) case.
fn normalized(path: &Path) -> String {
    let text = path.to_string_lossy().replace('/', "\\");
    let text = text.trim_end_matches('\\').to_owned();
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text
    }
}
fn inside(dir: &Path, root: &Path) -> bool {
    let (dir, root) = (normalized(dir), normalized(root));
    !root.is_empty() && (dir == root || dir.starts_with(&format!("{root}\\")))
}

fn url_allowed(url: &str) -> bool {
    reqwest::Url::parse(url).is_ok_and(|u| {
        let loopback = matches!(u.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
        (u.scheme() == "https" || (u.scheme() == "http" && loopback))
            && u.host_str().is_some()
            && u.username().is_empty()
            && u.password().is_none()
            && u.query().is_none()
            && u.fragment().is_none()
    })
}

fn project_ok(name: &str) -> bool {
    !name.trim().is_empty() && name.chars().count() <= PROJECT_MAX
}

/// Reads and checks the settings. `Ok(None)` means switched off (no file).
fn load(file: &Path) -> Result<Option<FileConfig>, &'static str> {
    let raw = match std::fs::read_to_string(file) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("board settings unreadable"),
    };
    let config: FileConfig = serde_json::from_str(&raw).map_err(|_| "board settings invalid")?;
    if config.schema_version != 1 {
        return Err("board settings: unsupported schema_version");
    }
    if !url_allowed(&config.url) {
        return Err("board settings: url must be https (http only for loopback)");
    }
    if !config.key_file.is_absolute() {
        return Err("board settings: key_file must be an absolute path");
    }
    if config.repos.is_empty()
        || config
            .repos
            .iter()
            .any(|r| !r.path.is_absolute() || r.project.as_deref().is_some_and(|p| !project_ok(p)))
    {
        return Err(
            "board settings: repos need absolute paths (and projects of 1 to 80 characters)",
        );
    }
    Ok(Some(config))
}

fn read_key(path: &Path) -> Option<String> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.is_file() || meta.len() > 1024 {
        return None;
    }
    let key = std::fs::read_to_string(path).ok()?.trim().to_owned();
    (!key.is_empty() && key.len() <= 512 && key.chars().all(|c| c.is_ascii_graphic()))
        .then_some(key)
}

fn item_id(details: &SessionDetails) -> String {
    match details.db_id {
        Some(id) => format!("session/{}", id.0),
        None => {
            let started: String = details
                .transcript
                .started_at
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .take(20)
                .collect();
            format!("session/live-{started}-{}", details.binding.session.0)
        }
    }
}

fn title(details: &SessionDetails) -> String {
    let snapshot = &details.snapshot;
    let name = [&snapshot.label, &snapshot.display]
        .into_iter()
        .find(|s| !s.trim().is_empty())
        .cloned()
        .unwrap_or_else(|| {
            Path::new(&snapshot.cwd)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
    let provider = if snapshot.provider.is_empty() {
        "AI"
    } else {
        &snapshot.provider
    };
    let text = if name.is_empty() {
        format!("{provider} の作業")
    } else {
        format!("{provider}: {name}")
    };
    truncate(&mask_secrets(&text), TITLE_MAX)
}

fn kind_label(kind: &str) -> &'static str {
    match kind.trim().to_ascii_lowercase().as_str() {
        "success" => "完了",
        "failure" => "失敗",
        "aborted" => "中断",
        "needs_action" => "判断待ち",
        _ => "完了",
    }
}

/// The optional tails of a DONE summary this module reads.
struct Tails {
    /// `計画:` as the board keeps it (see `plan_ref`).
    plan: Option<String>,
    /// `成果:`, an https URL.
    outcome: Option<String>,
    /// The summary without the tails that were understood. Words that only
    /// look like a tail ("成果: 試験が通った") stay in the text.
    rest: String,
}
fn tails(summary: &str, root: &Path) -> Tails {
    static PLAN: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?:^|\s)計画[:：]\s*(\S+)").unwrap());
    static OUTCOME: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?:^|\s)成果[:：]\s*(\S+)").unwrap());
    let mut rest = summary.to_owned();
    let mut take = |pattern: &regex::Regex, read: &dyn Fn(&str) -> Option<String>| {
        let (range, value) = pattern.captures_iter(&rest).find_map(|found| {
            read(&found[1]).map(|value| (found.get(0).expect("whole match").range(), value))
        })?;
        rest.replace_range(range, "");
        Some(value)
    };
    let plan = take(&PLAN, &|raw| {
        // A path has a separator or an extension; plain words are not a plan.
        (raw.contains(['/', '\\', '.']))
            .then(|| plan_ref(raw, root))
            .flatten()
    });
    let outcome = take(&OUTCOME, &|raw| {
        let url = unwrapped(raw);
        https_ok(url).then(|| url.to_owned())
    });
    Tails {
        plan,
        outcome,
        rest: rest.trim().to_owned(),
    }
}
/// A value as written in a sentence: without quotes and closing punctuation.
fn unwrapped(value: &str) -> &str {
    value
        .trim_matches(['`', '"', '\'', '<', '>'])
        .trim_end_matches(['。', '、', '.', ',', ')', '）', '」'])
        .trim_matches(['`', '"', '\'', '<', '>'])
}

/// What may be a commit hash in a summary: 7 to 40 hexadecimal characters
/// standing alone (ASCII letters, digits and `_` on neither side; Japanese
/// text may touch them), with a letter among them (a date or a count is
/// digits only).
/// At most `HASHES_MAX`, without repeats, in the order written.
fn commit_hashes(summary: &str) -> Vec<String> {
    static HASH: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?-u:\b)[0-9a-f]{7,40}(?-u:\b)").unwrap());
    let mut found: Vec<String> = Vec::new();
    for hash in HASH.find_iter(summary).map(|m| m.as_str()) {
        if hash.bytes().any(|b| b.is_ascii_alphabetic()) && !found.iter().any(|h| h == hash) {
            found.push(hash.to_owned());
            if found.len() == HASHES_MAX {
                break;
            }
        }
    }
    found
}

/// What the board accepts as a link: `https://host/...` without user info,
/// query or fragment, and without anything `mask_secrets` would hide. The link
/// is cut out of the summary text before that text is masked, so it has to be
/// checked here; a URL refused here stays in the (masked) note text instead.
fn https_ok(url: &str) -> bool {
    url.starts_with("https://")
        && url.chars().count() <= REF_MAX
        && !url
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
        && mask_secrets(url) == url
        && reqwest::Url::parse(url).is_ok_and(|u| {
            u.host_str().is_some()
                && u.username().is_empty()
                && u.password().is_none()
                && u.query().is_none()
                && u.fragment().is_none()
        })
}

/// A declared plan in the form the board keeps as `source_ref`: an https URL,
/// or a path relative to the repository with `/` separators. A path written in
/// full is accepted when it lies inside `root`.
fn plan_ref(raw: &str, root: &Path) -> Option<String> {
    let raw = unwrapped(raw.trim());
    if raw.starts_with("https://") {
        return https_ok(raw).then(|| raw.to_owned());
    }
    let mut path = raw.replace('\\', "/");
    let base = root.to_string_lossy().replace('\\', "/");
    let base = base.trim_end_matches('/');
    let under = if cfg!(windows) {
        path.to_lowercase()
            .starts_with(&format!("{}/", base.to_lowercase()))
    } else {
        path.starts_with(&format!("{base}/"))
    };
    if !base.is_empty() && under && path.is_char_boundary(base.len() + 1) {
        path = path[base.len() + 1..].to_owned();
    }
    while let Some(rest) = path.strip_prefix("./") {
        path = rest.to_owned();
    }
    let ok = !path.is_empty()
        && path.chars().count() <= REF_MAX
        && !path.starts_with(['/', '~'])
        && !path.contains(':')
        && !path.chars().any(|c| c.is_whitespace() || c.is_control())
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..");
    ok.then_some(path)
}

/// One clean line: no control or invisible characters, single spaces.
fn clean_line(text: &str) -> String {
    let visible: String = text
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .filter(|c| {
            !c.is_control()
                && !matches!(*c as u32, 0x200b..=0x200f | 0x2028..=0x202e | 0x2060..=0x2064 | 0x2066..=0x2069 | 0xfeff)
        })
        .collect();
    visible
        .split(' ')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The first sentence of a summary's first line.
fn first_sentence(summary: &str) -> String {
    let line = summary
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    let mut end = line.len();
    let mut chars = line.char_indices().peekable();
    while let Some((index, c)) = chars.next() {
        let closes = match c {
            '。' => true,
            '.' | '!' | '?' => chars.peek().is_none_or(|(_, next)| next.is_whitespace()),
            _ => false,
        };
        if closes && index > 0 {
            end = index;
            break;
        }
    }
    truncate(&clean_line(&line[..end]), SENTENCE_MAX)
}

/// `MM-DD provider: first sentence`, or `None` when the summary says nothing.
fn work_title(day: &str, provider: &str, summary: &str) -> Option<String> {
    let sentence = first_sentence(summary);
    if sentence.is_empty() {
        return None;
    }
    let provider = if provider.is_empty() { "AI" } else { provider };
    let text = clean_line(&mask_secrets(&format!("{day} {provider}: {sentence}")));
    (!text.is_empty()).then(|| truncate(&text, TITLE_MAX))
}

fn today() -> String {
    chrono::Local::now().format("%m-%d").to_string()
}

/// Title for a plan that has no item yet: the first heading of its Markdown
/// file, without `#` and a leading `[status]`. `None` unless `plan` is a
/// Markdown file in the repository, so a mistyped path never creates an item
/// (items cannot be removed from the board).
fn plan_title(root: &Path, plan: &str) -> Option<String> {
    use std::io::Read;
    let file = root.join(plan);
    let markdown = file
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("markdown"));
    if !markdown || !file.is_file() {
        return None;
    }
    let mut head = Vec::new();
    std::fs::File::open(&file)
        .ok()?
        .take(PLAN_HEAD_MAX)
        .read_to_end(&mut head)
        .ok()?;
    let text = String::from_utf8_lossy(&head);
    let mut lines = text.lines().peekable();
    if lines.peek().is_some_and(|line| line.trim() == "---") {
        lines.next();
        for line in lines.by_ref() {
            if line.trim() == "---" {
                break;
            }
        }
    }
    let heading = lines
        .map(str::trim)
        .find(|line| line.starts_with('#'))
        .map(|line| line.trim_start_matches('#').trim())
        .map(
            |line| match line.strip_prefix('[').and_then(|rest| rest.split_once(']')) {
                Some((_, after)) if !after.trim().is_empty() => after.trim(),
                _ => line,
            },
        )
        .map(|line| clean_line(&mask_secrets(line)))
        .filter(|line| !line.is_empty());
    let name = heading.or_else(|| {
        file.file_stem()
            .map(|stem| clean_line(&stem.to_string_lossy()))
            .filter(|stem| !stem.is_empty())
    })?;
    Some(truncate(&name, TITLE_MAX))
}

/// Item id for a plan made from its path: `plan/<project>/<path>`, in the
/// board's id alphabet, with a digest when the name had to be changed or cut.
fn plan_item_id(project: &str, plan: &str) -> String {
    use sha2::{Digest, Sha256};
    let safe = |text: &str, slash: bool| -> String {
        text.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') || (slash && c == '/')
                {
                    c
                } else {
                    '-'
                }
            })
            .collect()
    };
    let mut id = format!("plan/{}/{}", safe(project, false), safe(plan, true));
    if id.len() > 110 || !project.is_ascii() || !plan.is_ascii() {
        let digest: String = Sha256::digest(format!("{project}\n{plan}").as_bytes())
            .iter()
            .take(4)
            .map(|byte| format!("{byte:02x}"))
            .collect();
        id.truncate(110);
        id = format!("{id}-{digest}");
    }
    id
}

/// Where a session's work stood on the board (after a Hub restart): its
/// latest piece, the plan that piece is under, and whether it has been named.
fn resume(items: &[Listed], base: &str, opening: &str) -> Option<Piece> {
    let number = |id: &str| -> Option<u32> {
        if id == base {
            return Some(1);
        }
        id.strip_prefix(base)?
            .strip_prefix('/')?
            .parse()
            .ok()
            .filter(|n| *n >= 2)
    };
    let (item, number) = items
        .iter()
        .filter_map(|item| number(&item.item_id).map(|n| (item, n)))
        .max_by_key(|(_, n)| *n)?;
    let plan = item
        .parent_id
        .as_deref()
        .and_then(|parent| items.iter().find(|other| other.item_id == parent))
        .and_then(|parent| parent.source_ref.clone());
    Some(Piece {
        item_id: item.item_id.clone(),
        number,
        linked: plan.is_some(),
        plan,
        settled: true,
        exists: true,
        titled: item.title != opening,
    })
}

pub struct BoardSync {
    file: PathBuf,
    enabled: bool,
    client: reqwest::Client,
    tasks: HubTaskHandle,
    warning: Arc<Warning>,
    /// Answers where a commit named in a summary can be opened; `None` = no
    /// commit links.
    git: Option<Arc<dyn GitBranchSource>>,
    /// `None` = this session is not in a listed repository.
    sessions: Mutex<HashMap<LiveSessionId, Option<Seat>>>,
    /// Written by the worker only, in job order.
    pieces: Mutex<HashMap<LiveSessionId, Piece>>,
    queue: Mutex<VecDeque<Job>>,
    running: AtomicBool,
    warned: Mutex<Option<&'static str>>,
}

impl BoardSync {
    pub fn new(
        root: &Path,
        trial: bool,
        tasks: HubTaskHandle,
        warning: Arc<Warning>,
    ) -> Result<Arc<Self>, reqwest::Error> {
        Self::with_git(root, trial, tasks, warning, None)
    }

    /// `new`, with the reader that turns a commit named in a summary into its
    /// GitHub page.
    pub fn with_git(
        root: &Path,
        trial: bool,
        tasks: HubTaskHandle,
        warning: Arc<Warning>,
        git: Option<Arc<dyn GitBranchSource>>,
    ) -> Result<Arc<Self>, reqwest::Error> {
        Ok(Arc::new(Self {
            file: root.join(FILE_NAME),
            enabled: !trial,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            tasks,
            warning,
            git,
            sessions: Mutex::default(),
            pieces: Mutex::default(),
            queue: Mutex::default(),
            running: AtomicBool::new(false),
            warned: Mutex::new(None),
        }))
    }

    fn warn_once(&self, message: &'static str) {
        let mut last = lock(&self.warned);
        if *last != Some(message) {
            *last = Some(message);
            (self.warning)("board sync", message);
        }
    }

    fn resolve(&self, details: &SessionDetails) -> Option<Seat> {
        if !self.enabled {
            return None;
        }
        let config = match load(&self.file) {
            Ok(Some(config)) => config,
            Ok(None) => return None,
            Err(message) => {
                self.warn_once(message);
                return None;
            }
        };
        let dir = details
            .git_root
            .clone()
            .unwrap_or_else(|| PathBuf::from(&details.snapshot.cwd));
        let cwd = PathBuf::from(&details.snapshot.cwd);
        let repo = config
            .repos
            .iter()
            .find(|r| inside(&dir, &r.path) || inside(&cwd, &r.path))?;
        let project = repo.project.clone().unwrap_or_else(|| {
            repo.path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
        if !project_ok(&project) {
            self.warn_once("board settings: project name unusable");
            return None;
        }
        Some(Seat {
            target: Target {
                url: config.url,
                key_file: config.key_file,
                project,
                item_id: item_id(details),
            },
            root: dir,
            provider: details.snapshot.provider.clone(),
            opening: title(details),
        })
    }

    /// Session registered (also called again on reattach; the board item is
    /// only re-announced when the board reports a change).
    pub fn session_started(self: &Arc<Self>, details: &SessionDetails) {
        let session = details.binding.session;
        if lock(&self.sessions).contains_key(&session) {
            return;
        }
        let seat = self.resolve(details);
        lock(&self.sessions).insert(session, seat.clone());
        if let Some(seat) = seat {
            let cwd = Path::new(&details.snapshot.cwd)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            self.enqueue(Job::Start {
                session,
                seat,
                text: truncate(
                    &format!("開始（{}、{cwd}）", details.snapshot.provider),
                    TEXT_MAX,
                ),
            });
        }
    }

    /// One confirmed DONE summary (not the fallback).
    pub fn turn_done(self: &Arc<Self>, details: Option<&SessionDetails>, summary: &DoneSummary) {
        let session = LiveSessionId(summary.session_id);
        let known = lock(&self.sessions).get(&session).cloned();
        let seat = match (known, details) {
            (Some(seat), _) => seat,
            (None, Some(details)) => {
                // Hub restarted after the session began: announce it first.
                self.session_started(details);
                lock(&self.sessions).get(&session).cloned().flatten()
            }
            (None, None) => None,
        };
        let Some(seat) = seat else { return };
        self.enqueue(Job::Note {
            session,
            seat,
            kind: kind_label(&summary.kind),
            summary: summary.text.trim().to_owned(),
        });
    }

    pub fn session_ended(self: &Arc<Self>, session: LiveSessionId, state: &str) {
        let Some(Some(seat)) = lock(&self.sessions).remove(&session) else {
            return;
        };
        let state = if state.trim().is_empty() {
            "ended"
        } else {
            state.trim()
        };
        self.enqueue(Job::End {
            session,
            target: seat.target,
            text: truncate(&format!("終了（{state}）"), TEXT_MAX),
        });
    }

    fn enqueue(self: &Arc<Self>, job: Job) {
        {
            let mut queue = lock(&self.queue);
            if queue.len() >= QUEUE_MAX {
                self.warn_once("board sync queue full; dropped a write");
                return;
            }
            queue.push_back(job);
            if self.running.swap(true, Ordering::SeqCst) {
                return;
            }
        }
        let permit = match self.tasks.effect_permit() {
            Ok(permit) => permit,
            Err(_) => {
                let mut queue = lock(&self.queue);
                queue.clear();
                self.running.store(false, Ordering::SeqCst);
                return;
            }
        };
        let owner = self.clone();
        let cancellation = permit.cancellation();
        let _receipt = permit.start(async move {
            loop {
                let job = {
                    let mut queue = lock(&owner.queue);
                    match queue.pop_front() {
                        Some(job) => job,
                        None => {
                            owner.running.store(false, Ordering::SeqCst);
                            break;
                        }
                    }
                };
                tokio::select! {
                    _ = cancellation.token().cancelled() => break,
                    _ = owner.run(job) => {}
                }
            }
        });
    }

    async fn run(&self, job: Job) {
        match job {
            Job::Start {
                session,
                seat,
                text,
            } => self.start(session, &seat, text).await,
            Job::Note {
                session,
                seat,
                kind,
                summary,
            } => self.note(session, &seat, kind, &summary).await,
            Job::End {
                session,
                target,
                text,
            } => {
                let item_id = lock(&self.pieces)
                    .remove(&session)
                    .map(|piece| piece.item_id)
                    .unwrap_or_else(|| target.item_id.clone());
                let args = serde_json::json!({"item_id": item_id, "state": "done"});
                if self
                    .call(&target, "board_upsert_item", args)
                    .await
                    .is_some()
                {
                    self.event(&target, &item_id, "session_end", text, None)
                        .await;
                }
            }
        }
    }

    async fn start(&self, session: LiveSessionId, seat: &Seat, text: String) {
        let target = &seat.target;
        // After a Hub restart the work may already be on the board, named and
        // in a later piece: continue there instead of resetting it.
        let known = self
            .items(target)
            .await
            .and_then(|items| resume(&items, &target.item_id, &seat.opening));
        let piece = known
            .clone()
            .unwrap_or_else(|| Piece::first(&target.item_id));
        let args = if known.is_some() {
            serde_json::json!({"item_id": piece.item_id, "state": "in_progress"})
        } else {
            serde_json::json!({
                "item_id": piece.item_id, "project": target.project, "kind": "task",
                "title": seat.opening, "state": "in_progress",
            })
        };
        if let Some(result) = self.call(target, "board_upsert_item", args).await
            && result.get("changed").and_then(|v| v.as_bool()) != Some(false)
        {
            self.event(target, &piece.item_id, "session_start", text, None)
                .await;
        }
        lock(&self.pieces).entry(session).or_insert(piece);
    }

    async fn note(&self, session: LiveSessionId, seat: &Seat, kind: &str, summary: &str) {
        let target = &seat.target;
        let tails = tails(summary, &seat.root);
        let declared = tails.plan.clone();
        let mut piece = lock(&self.pieces)
            .get(&session)
            .cloned()
            .unwrap_or_else(|| Piece::first(&target.item_id));
        if let Some(plan) = declared
            && piece.plan.as_ref() != Some(&plan)
        {
            if piece.linked {
                // The work moved on from the plan it is under: close this
                // piece, open the next. (Work that is under no plan yet takes
                // the plan named now, so a mistyped path costs no extra item.)
                let args = serde_json::json!({"item_id": piece.item_id, "state": "done"});
                self.call(target, "board_upsert_item", args).await;
                piece = Piece {
                    item_id: format!("{}/{}", target.item_id, piece.number + 1),
                    number: piece.number + 1,
                    plan: None,
                    settled: true,
                    linked: false,
                    exists: false,
                    titled: false,
                };
            }
            piece.plan = Some(plan);
            piece.settled = false;
        }
        let mut parent = None;
        if !piece.settled
            && let Some(plan) = piece.plan.clone()
        {
            match self.parent_for(target, &plan, &seat.root).await {
                Lookup::Found(item_id) => {
                    parent = Some(item_id);
                    piece.settled = true;
                }
                Lookup::Nothing => piece.settled = true,
                Lookup::Unread => {}
            }
        }
        let name = if piece.titled {
            None
        } else {
            work_title(&today(), &seat.provider, &tails.rest)
        };
        if !piece.exists || parent.is_some() || name.is_some() {
            let mut args = serde_json::json!({"item_id": piece.item_id});
            if !piece.exists {
                args["project"] = target.project.clone().into();
                args["kind"] = "task".into();
                args["state"] = "in_progress".into();
                args["title"] = name.clone().unwrap_or_else(|| seat.opening.clone()).into();
            } else if let Some(name) = &name {
                args["title"] = name.clone().into();
            }
            if let Some(parent) = &parent {
                args["parent_id"] = parent.clone().into();
            }
            let mut written = self
                .request(target, "board_upsert_item", args.clone())
                .await;
            let mut under = parent.is_some();
            if matches!(written, Err(Failed::Refused))
                && under
                && let Some(fields) = args.as_object_mut()
            {
                // A parent the board will not take (depth, another project)
                // must not cost the work its item or its name.
                fields.remove("parent_id");
                under = false;
                written = self.request(target, "board_upsert_item", args).await;
            }
            if written.is_ok() {
                piece.exists = true;
                piece.linked = piece.linked || under;
                piece.titled = piece.titled || name.is_some();
            }
        }
        let link = match tails.outcome {
            Some(outcome) => Some(outcome),
            None => self.commit_link(&tails.rest, &seat.root).await,
        };
        let text = truncate(&mask_secrets(&format!("[{kind}] {}", tails.rest)), TEXT_MAX);
        self.event(target, &piece.item_id, "note", text, link).await;
        lock(&self.pieces).insert(session, piece);
    }

    /// The GitHub page of the first commit the summary names by hash that is on
    /// `origin`. Only what the summary itself names is linked: the repository's
    /// newest commit may be another session's work, and a commit that is not
    /// pushed has no page to open.
    async fn commit_link(&self, summary: &str, root: &Path) -> Option<String> {
        let git = self.git.as_ref()?;
        let root = root.to_string_lossy();
        for hash in commit_hashes(summary) {
            if let Some(url) = git.commit_page(&root, &hash).await
                && https_ok(&url)
            {
                return Some(url);
            }
        }
        None
    }

    /// The items of the target's project (no events).
    async fn items(&self, target: &Target) -> Option<Vec<Listed>> {
        let args = serde_json::json!({"project": target.project, "since_seq": NO_EVENTS});
        let result = self.call(target, "board_list", args).await?;
        serde_json::from_value(result.get("items")?.clone()).ok()
    }

    /// The item a piece of work for `plan` belongs under.
    async fn parent_for(&self, target: &Target, plan: &str, root: &Path) -> Lookup {
        let Some(items) = self.items(target).await else {
            return Lookup::Unread;
        };
        // A plan, issue or PR that points at the same place; a plan first when
        // several do (its steps often share the plan's file).
        let found = items
            .iter()
            .filter(|item| item.source_ref.as_deref() == Some(plan) && item.kind != "task")
            .min_by_key(|item| u8::from(item.kind != "plan"));
        if let Some(item) = found {
            return Lookup::Found(item.item_id.clone());
        }
        if plan.starts_with("https://") {
            return Lookup::Nothing;
        }
        let Some(title) = plan_title(root, plan) else {
            return Lookup::Nothing;
        };
        let item_id = plan_item_id(&target.project, plan);
        if items.iter().any(|item| item.item_id == item_id) {
            return Lookup::Nothing;
        }
        let mut args = serde_json::json!({
            "item_id": item_id, "project": target.project, "kind": "plan",
            "title": title, "state": "in_progress", "source_ref": plan,
        });
        let mut projects = items.iter().filter(|item| item.kind == "project");
        if let (Some(project), None) = (projects.next(), projects.next()) {
            args["parent_id"] = project.item_id.clone().into();
        }
        match self.call(target, "board_upsert_item", args).await {
            Some(_) => Lookup::Found(item_id),
            None => Lookup::Nothing,
        }
    }

    async fn event(
        &self,
        target: &Target,
        item_id: &str,
        kind: &str,
        text: String,
        link: Option<String>,
    ) {
        let args = serde_json::json!({"item_id": item_id, "kind": kind, "text": text});
        if let Some(link) = link {
            let mut linked = args.clone();
            linked["ref"] = link.into();
            match self.request(target, "board_add_event", linked).await {
                // A link the board will not take must not cost the note.
                Err(Failed::Refused) => {}
                _ => return,
            }
        }
        self.call(target, "board_add_event", args).await;
    }

    async fn call(
        &self,
        target: &Target,
        tool: &str,
        arguments: serde_json::Value,
    ) -> Option<serde_json::Value> {
        self.request(target, tool, arguments).await.ok()
    }

    async fn request(
        &self,
        target: &Target,
        tool: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, Failed> {
        let Some(key) = read_key(&target.key_file) else {
            self.warn_once("board key file unreadable");
            return Err(Failed::Unreached);
        };
        let body = serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": tool, "arguments": arguments},
        });
        let mut response = match self
            .client
            .post(&target.url)
            .bearer_auth(key)
            .header("accept", "application/json")
            .json(&body)
            .send()
            .await
        {
            Ok(response) => response,
            Err(_) => {
                self.warn_once("board request failed");
                return Err(Failed::Unreached);
            }
        };
        if !response.status().is_success() {
            self.warn_once("board refused the write (HTTP status)");
            return Err(Failed::Unreached);
        }
        if response
            .content_length()
            .is_some_and(|length| length > RESPONSE_MAX as u64)
        {
            self.warn_once("board reply too large");
            return Err(Failed::Unreached);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| Failed::Unreached)? {
            if bytes.len() + chunk.len() > RESPONSE_MAX {
                self.warn_once("board reply too large");
                return Err(Failed::Unreached);
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| Failed::Unreached)?;
        let result = value
            .get("result")
            .and_then(|result| result.get("structuredContent"))
            .cloned()
            .ok_or(Failed::Unreached)?;
        if result.get("ok").and_then(|v| v.as_bool()) == Some(false) {
            self.warn_once("board refused the write");
            return Err(Failed::Refused);
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests;
