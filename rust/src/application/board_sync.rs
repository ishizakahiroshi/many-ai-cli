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
//! Per session the board gets one item (`session/<id>`, kind `task`): a
//! `session_start` event when it starts, a `note` with the DONE summary after
//! each finished turn, and `session_end` (state `done`) when it ends. Writes
//! are best effort, in order, one at a time; a failure is logged and dropped.
use crate::{
    hub::task_owner::HubTaskHandle,
    proto::{DoneSummary, core::*},
    storage::mask_secrets,
};
use serde::Deserialize;
use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

pub const FILE_NAME: &str = "board.json";
const TITLE_MAX: usize = 200;
const TEXT_MAX: usize = 2000;
const PROJECT_MAX: usize = 80;
const QUEUE_MAX: usize = 256;

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
    item_id: String,
}

enum Job {
    Start {
        target: Target,
        title: String,
        text: String,
    },
    Note {
        target: Target,
        text: String,
    },
    End {
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

pub struct BoardSync {
    file: PathBuf,
    enabled: bool,
    client: reqwest::Client,
    tasks: HubTaskHandle,
    warning: Arc<Warning>,
    /// `None` = this session is not in a listed repository.
    sessions: Mutex<HashMap<LiveSessionId, Option<Target>>>,
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
        Ok(Arc::new(Self {
            file: root.join(FILE_NAME),
            enabled: !trial,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            tasks,
            warning,
            sessions: Mutex::default(),
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

    fn resolve(&self, details: &SessionDetails) -> Option<Target> {
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
        Some(Target {
            url: config.url,
            key_file: config.key_file,
            project,
            item_id: item_id(details),
        })
    }

    /// Session registered (also called again on reattach; the board item is
    /// only re-announced when the board reports a change).
    pub fn session_started(self: &Arc<Self>, details: &SessionDetails) {
        let session = details.binding.session;
        if lock(&self.sessions).contains_key(&session) {
            return;
        }
        let target = self.resolve(details);
        lock(&self.sessions).insert(session, target.clone());
        if let Some(target) = target {
            let cwd = Path::new(&details.snapshot.cwd)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            self.enqueue(Job::Start {
                target,
                title: title(details),
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
        let target = match (known, details) {
            (Some(target), _) => target,
            (None, Some(details)) => {
                // Hub restarted after the session began: announce it first.
                self.session_started(details);
                lock(&self.sessions).get(&session).cloned().flatten()
            }
            (None, None) => None,
        };
        let Some(target) = target else { return };
        let text = format!("[{}] {}", kind_label(&summary.kind), summary.text.trim());
        self.enqueue(Job::Note {
            target,
            text: truncate(&mask_secrets(&text), TEXT_MAX),
        });
    }

    pub fn session_ended(self: &Arc<Self>, session: LiveSessionId, state: &str) {
        let Some(Some(target)) = lock(&self.sessions).remove(&session) else {
            return;
        };
        let state = if state.trim().is_empty() {
            "ended"
        } else {
            state.trim()
        };
        self.enqueue(Job::End {
            target,
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
                target,
                title,
                text,
            } => {
                let args = serde_json::json!({
                    "item_id": target.item_id, "project": target.project, "kind": "task",
                    "title": title, "state": "in_progress",
                });
                if let Some(result) = self.call(&target, "board_upsert_item", args).await
                    && result.get("changed").and_then(|v| v.as_bool()) != Some(false)
                {
                    self.event(&target, "session_start", text).await;
                }
            }
            Job::Note { target, text } => self.event(&target, "note", text).await,
            Job::End { target, text } => {
                let args = serde_json::json!({"item_id": target.item_id, "state": "done"});
                if self
                    .call(&target, "board_upsert_item", args)
                    .await
                    .is_some()
                {
                    self.event(&target, "session_end", text).await;
                }
            }
        }
    }

    async fn event(&self, target: &Target, kind: &str, text: String) {
        let args = serde_json::json!({"item_id": target.item_id, "kind": kind, "text": text});
        self.call(target, "board_add_event", args).await;
    }

    async fn call(
        &self,
        target: &Target,
        tool: &str,
        arguments: serde_json::Value,
    ) -> Option<serde_json::Value> {
        let Some(key) = read_key(&target.key_file) else {
            self.warn_once("board key file unreadable");
            return None;
        };
        let body = serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": tool, "arguments": arguments},
        });
        let response = match self
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
                return None;
            }
        };
        if !response.status().is_success() {
            self.warn_once("board refused the write (HTTP status)");
            return None;
        }
        let value: serde_json::Value = response.json().await.ok()?;
        let result = value.get("result")?.get("structuredContent")?.clone();
        if result.get("ok").and_then(|v| v.as_bool()) == Some(false) {
            self.warn_once("board refused the write");
            return None;
        }
        Some(result)
    }
}

#[cfg(test)]
mod tests;
