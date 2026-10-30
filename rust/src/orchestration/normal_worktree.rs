//! Ordinary-session worktree lifecycle from `internal/hub/normal_worktree.go`.
//!
//! This helper owns only the source's process-wide creation mutex. Pending
//! registration, session metadata, dismissal, and launch rollback remain with
//! their existing callers. Git identity checks used for orchestration reuse
//! are deliberately not added to this distinct source path.
//!
//! Rust-only extension of the source's cleanup: a branch counts as merged when
//! the parent's local HEAD or that HEAD's upstream (`HEAD@{upstream}`, fetched
//! first) contains it, and worktrees still unmerged when their session closed
//! are remembered (`retained`) and judged again when the Hub starts. A squash
//! or rebase merge rewrites the commits, so it is never seen as merged.
use super::child_launch::{safe_token, worktree::WorktreeGit};
use crate::{
    process::{Cancellation, ExitOutcome, ManagedProcess, SpawnOptions},
    proto::{
        core::NormalWorktree,
        go_quote::quote,
        time::{self, Timestamp},
    },
};
use chrono::{Datelike, Timelike};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::{sync::Mutex, time::Instant};
mod retained;
pub use retained::{RecheckReport, RetainedWorktrees};

pub const WORKTREE_CLEANUP_DELETE: &str = "delete";
pub const WORKTREE_CLEANUP_KEEP: &str = "keep";
pub const WORKTREE_CLEANUP_MANUAL: &str = "manual";
/// Prefix of the cleanup error when the upstream could not be fetched. The
/// worktree is kept; callers warn instead of treating it as merged.
pub const UPSTREAM_FETCH_FAILED: &str = "worktree retained: upstream fetch failed";
/// Cleanup error when a live session's cwd is inside the worktree.
pub const IN_USE: &str = "worktree retained: in use by a running session";
const NOT_ON_BRANCH: &str = "worktree retained: HEAD is not on branch";
/// Answers whether a live session currently uses (has its cwd inside) a path.
pub type InUse = dyn Fn(&Path) -> bool + Send + Sync;
// A fetch must not stop at a credential prompt: the Hub has no one to answer.
const FETCH_ENV: &[(&str, &str)] = &[("GIT_TERMINAL_PROMPT", "0"), ("GCM_INTERACTIVE", "never")];

/// One cleanup pass resolves and fetches each parent's upstream at most once.
/// `Ok(None)`: no upstream, so only the local HEAD counts. `Err`: fetch failed.
#[derive(Default)]
struct UpstreamMemo(BTreeMap<String, Result<Option<String>, String>>);

// Source serializes all ordinary worktree creation, including different repos.
// Cleanup has no corresponding lock in Go.
static NORMAL_WORKTREE_CREATE: Mutex<()> = Mutex::const_new(());

pub fn valid_worktree_cleanup(value: &str) -> bool {
    matches!(value, "" | "delete" | "keep" | "manual")
}

pub fn effective_worktree_cleanup(value: &str) -> &str {
    match value {
        WORKTREE_CLEANUP_DELETE | WORKTREE_CLEANUP_KEEP => value,
        _ => WORKTREE_CLEANUP_MANUAL,
    }
}

/// Explicit process cwd; `WorktreeGit` retains the one canonical executable
/// and environment. The caller owns this future on its existing Tokio runtime;
/// no detached lifecycle job, ambient HOME read, or wall clock read is added.
#[derive(Clone)]
pub struct NormalWorktreeLifecycle {
    process_cwd: PathBuf,
    git: WorktreeGit,
}

impl NormalWorktreeLifecycle {
    pub fn new(process_cwd: PathBuf, git: WorktreeGit) -> Self {
        Self { process_cwd, git }
    }

    /// `now` is captured by the caller before waiting for the creation mutex,
    /// as Go's time.Time argument is. The caller retains its UTC offset too.
    pub async fn prepare(
        &self,
        cwd: &Path,
        label: &str,
        now: Timestamp,
        offset_seconds: i32,
    ) -> Result<NormalWorktree, String> {
        let _creation = NORMAL_WORKTREE_CREATE.lock().await;
        let inspection = Instant::now() + Duration::from_secs(3);
        let root = self
            .run(
                cwd,
                &args(&["rev-parse", "--show-toplevel"]),
                inspection,
                GitOutput::Stdout,
            )
            .await
            .map_err(|error| format!("parent cwd is not a git repository: {}", error.detail))?;
        let parent = root.trim();
        if parent.is_empty() {
            return Err("git did not return repository root".into());
        }
        let parent = Path::new(parent);
        let name = safe_token(label);
        let stamp = minute_stamp(now, offset_seconds)?;
        let root = parent.join(".git-worktrees");
        create_dirs(&root, 0o700).map_err(|error| format!("create worktree root: {error}"))?;
        self.exclude_git_path(parent, ".git-worktrees/", inspection)
            .await;
        let mut path = root.join(format!("{name}-{stamp}"));
        let mut suffix = 2;
        loop {
            match fs::metadata(&path) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => break,
                Err(error) => return Err(format!("check worktree path: {error}")),
                Ok(_) => {
                    path = root.join(format!("{name}-{stamp}-{suffix}"));
                    suffix += 1;
                }
            }
        }
        // Keep the complete basename: dots in labels are not file extensions.
        let branch = format!(
            "many-ai/{}",
            path.file_name()
                .expect("worktree path has a basename")
                .to_string_lossy()
        );
        self.run(
            parent,
            &[
                "worktree".into(),
                "add".into(),
                "-b".into(),
                branch.clone().into(),
                path.as_os_str().to_owned(),
                "HEAD".into(),
            ],
            Instant::now() + Duration::from_secs(30),
            GitOutput::Combined,
        )
        .await
        .map_err(|error| format!("create worktree: {}: {}", error.output.trim(), error.detail))?;
        Ok(NormalWorktree {
            path: path.to_string_lossy().into_owned(),
            parent_dir: parent.to_string_lossy().into_owned(),
            branch,
            created: true,
        })
    }

    /// Only a created, clean, merged tree with the exact `delete` policy is
    /// removed. Its branch remains. Unknown policies are manual, without Git IO.
    pub async fn cleanup(&self, tree: &NormalWorktree, policy: &str) -> Result<(), String> {
        self.cleanup_guarded(tree, policy, &|_| false, &Cancellation::default())
            .await
    }

    /// `cleanup`, refusing while `in_use` reports a live session inside the
    /// tree (asked again right before removal). `cancel` stops an upstream
    /// fetch; the tree is then retained.
    pub async fn cleanup_guarded(
        &self,
        tree: &NormalWorktree,
        policy: &str,
        in_use: &InUse,
        cancel: &Cancellation,
    ) -> Result<(), String> {
        if !tree.created || effective_worktree_cleanup(policy) != WORKTREE_CLEANUP_DELETE {
            return Ok(());
        }
        self.cleanup_with(tree, &mut UpstreamMemo::default(), in_use, cancel)
            .await
    }

    /// Judge again the worktrees remembered as retained when their session
    /// closed. Records are grouped by parent so each parent's upstream is
    /// fetched at most once. A record leaves the list when its tree is removed,
    /// is gone, or no longer holds its `many-ai/` branch; nothing else is
    /// removed, and a tree a live session uses is skipped.
    pub async fn recheck_retained(
        &self,
        store: &RetainedWorktrees,
        in_use: &InUse,
        cancel: &Cancellation,
    ) -> RecheckReport {
        let mut report = RecheckReport::default();
        let Ok(mut records) = store.load() else {
            report.store_failed = true;
            return report;
        };
        records.sort_by(|a, b| (&a.parent_dir, &a.path).cmp(&(&b.parent_dir, &b.path)));
        let mut memo = UpstreamMemo::default();
        for tree in records {
            if cancel.is_cancelled() {
                break;
            }
            let forget = if !retained::well_formed(&tree) || !Path::new(&tree.path).is_dir() {
                true
            } else {
                match self.cleanup_with(&tree, &mut memo, in_use, cancel).await {
                    Ok(()) => {
                        report.removed += 1;
                        true
                    }
                    Err(error) if error.starts_with(NOT_ON_BRANCH) => true,
                    Err(error) => {
                        if error == IN_USE {
                            report.in_use += 1;
                        } else if error.starts_with(UPSTREAM_FETCH_FAILED) {
                            report.fetch_failed += 1;
                        } else {
                            report.retained += 1;
                        }
                        false
                    }
                }
            };
            if forget {
                report.forgotten += 1;
                if store.forget(&tree.path).is_err() {
                    report.store_failed = true;
                }
            }
        }
        report
    }

    async fn cleanup_with(
        &self,
        tree: &NormalWorktree,
        memo: &mut UpstreamMemo,
        in_use: &InUse,
        cancel: &Cancellation,
    ) -> Result<(), String> {
        let path = Path::new(&tree.path);
        let parent = Path::new(&tree.parent_dir);
        if in_use(path) {
            return Err(IN_USE.into());
        }
        let inspection = Instant::now() + Duration::from_secs(5);
        let status = self
            .run(
                path,
                &args(&["status", "--porcelain"]),
                inspection,
                GitOutput::Stdout,
            )
            .await
            .map_err(|error| format!("inspect worktree status: {}", error.detail))?;
        if !status.trim().is_empty() {
            return Err("worktree retained: uncommitted changes".into());
        }
        // Commits made on a detached HEAD or another branch would lose their
        // only reference (the worktree's HEAD) with the tree. `symbolic-ref -q`
        // exits 1, silently, exactly when HEAD is detached.
        let head = match self
            .run(
                path,
                &args(&["symbolic-ref", "-q", "HEAD"]),
                inspection,
                GitOutput::Stdout,
            )
            .await
        {
            Ok(head) => head,
            Err(error) if error.detail == "exit status 1" => String::new(),
            Err(error) => return Err(format!("inspect worktree HEAD: {}", error.detail)),
        };
        if head.trim() != format!("refs/heads/{}", tree.branch) {
            return Err(format!("{NOT_ON_BRANCH} {}", quote(&tree.branch)));
        }
        let not_merged = || {
            format!(
                "worktree retained: branch {} is not merged",
                quote(&tree.branch)
            )
        };
        if self
            .run(
                parent,
                &args(&["merge-base", "--is-ancestor", &tree.branch, "HEAD"]),
                inspection,
                GitOutput::Discard,
            )
            .await
            .is_err()
        {
            // The parent's branch is often behind its remote: sessions push
            // their work to the remote branch directly.
            let upstream = match self.upstream(parent, memo, cancel).await {
                Ok(Some(upstream)) => upstream,
                Ok(None) => return Err(not_merged()),
                Err(detail) => return Err(format!("{UPSTREAM_FETCH_FAILED}: {detail}")),
            };
            if self
                .run(
                    parent,
                    &args(&["merge-base", "--is-ancestor", &tree.branch, &upstream]),
                    Instant::now() + Duration::from_secs(5),
                    GitOutput::Discard,
                )
                .await
                .is_err()
            {
                return Err(not_merged());
            }
        }
        if in_use(path) {
            return Err(IN_USE.into());
        }
        self.run(
            parent,
            &args(&["worktree", "remove", &tree.path]),
            Instant::now() + Duration::from_secs(30),
            GitOutput::Combined,
        )
        .await
        .map_err(|error| format!("remove worktree: {}: {}", error.output.trim(), error.detail))?;
        Ok(())
    }

    async fn upstream(
        &self,
        parent: &Path,
        memo: &mut UpstreamMemo,
        cancel: &Cancellation,
    ) -> Result<Option<String>, String> {
        let key = parent.to_string_lossy().into_owned();
        if let Some(known) = memo.0.get(&key) {
            return known.clone();
        }
        let resolved = self.fetch_upstream(parent, cancel).await;
        memo.0.insert(key, resolved.clone());
        resolved
    }

    /// The full ref of the parent HEAD's upstream, fetched first when it is a
    /// remote-tracking branch. No upstream (or a detached HEAD) is `Ok(None)`.
    /// The error carries only Git's exit detail: fetch output can name a URL.
    async fn fetch_upstream(
        &self,
        parent: &Path,
        cancel: &Cancellation,
    ) -> Result<Option<String>, String> {
        let Ok(upstream) = self
            .run(
                parent,
                &args(&["rev-parse", "--symbolic-full-name", "HEAD@{upstream}"]),
                Instant::now() + Duration::from_secs(5),
                GitOutput::Stdout,
            )
            .await
        else {
            return Ok(None);
        };
        let upstream = upstream.trim();
        if upstream.is_empty() {
            return Ok(None);
        }
        if upstream.starts_with("refs/remotes/") {
            // Without a repository operand Git fetches the remote that the
            // current branch's upstream names.
            self.run_with(
                parent,
                &args(&["fetch", "--quiet", "--no-recurse-submodules"]),
                Instant::now() + Duration::from_secs(30),
                GitOutput::Discard,
                FETCH_ENV,
                Some(cancel),
            )
            .await
            .map_err(|error| error.detail)?;
        }
        Ok(Some(upstream.to_owned()))
    }

    // Intentionally best effort, including read, mkdir, open, write and close.
    // This is the ordinary helper's source implementation, not a tracked
    // .gitignore edit or a private-file atomic replacement.
    async fn exclude_git_path(&self, parent: &Path, entry: &str, deadline: Instant) {
        let Ok(common) = self
            .run(
                parent,
                &args(&["rev-parse", "--git-common-dir"]),
                deadline,
                GitOutput::Stdout,
            )
            .await
        else {
            return;
        };
        let common = common.trim();
        if common.is_empty() {
            return;
        }
        let common = Path::new(common);
        let common = if common.is_absolute() {
            common.to_owned()
        } else {
            parent.join(common)
        };
        let exclude = common.join("info").join("exclude");
        let existing = fs::read(&exclude).unwrap_or_default();
        if String::from_utf8_lossy(&existing)
            .split('\n')
            .any(|line| line.trim() == entry)
        {
            return;
        }
        if create_dirs(exclude.parent().expect("exclude has a parent"), 0o755).is_err() {
            return;
        }
        let mut options = OpenOptions::new();
        options.append(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o644);
        }
        let Ok(mut file) = options.open(exclude) else {
            return;
        };
        let prefix = if !existing.is_empty() && existing.last() != Some(&b'\n') {
            "\n"
        } else {
            ""
        };
        let _ = file.write_all(format!("{prefix}{entry}\n").as_bytes());
    }

    async fn run(
        &self,
        cwd: &Path,
        argv: &[OsString],
        deadline: Instant,
        mode: GitOutput,
    ) -> Result<String, GitFailure> {
        self.run_with(cwd, argv, deadline, mode, &[], None).await
    }

    /// `env` is added to the planned process only. A cancelled `cancel`
    /// drops (and so stops) the process and reports `context canceled`.
    async fn run_with(
        &self,
        cwd: &Path,
        argv: &[OsString],
        deadline: Instant,
        mode: GitOutput,
        env: &[(&str, &str)],
        cancel: Option<&Cancellation>,
    ) -> Result<String, GitFailure> {
        let canceled = || GitFailure {
            output: String::new(),
            detail: "context canceled".into(),
        };
        if cancel.is_some_and(Cancellation::is_cancelled) {
            return Err(canceled());
        }
        let timeout = deadline
            .checked_duration_since(Instant::now())
            .filter(|time| !time.is_zero())
            .ok_or_else(|| GitFailure {
                output: String::new(),
                detail: "context deadline exceeded".into(),
            })?;
        let mut plan = self
            .git
            .command_plan(&self.process_cwd, cwd, argv, timeout)
            .map_err(|error| GitFailure {
                output: String::new(),
                detail: error.to_string(),
            })?;
        for (key, value) in env {
            plan.env.insert((*key).into(), Some((*value).into()));
        }
        let (mut process, events) = ManagedProcess::spawn_owned_with_options(
            plan,
            1,
            SpawnOptions {
                combined_output: matches!(mode, GitOutput::Combined),
                stderr_null: !matches!(mode, GitOutput::Combined),
                stdout_null: matches!(mode, GitOutput::Discard),
                stdin_null: true,
                ..Default::default()
            },
        );
        drop(events);
        let waited = match cancel {
            Some(cancel) => tokio::select! {
                waited = process.wait() => waited,
                _ = cancel.cancelled() => return Err(canceled()),
            },
            None => process.wait().await,
        };
        let output = waited.map_err(|error| GitFailure {
            output: String::new(),
            detail: error.to_string(),
        })?;
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        let detail = match output.outcome {
            ExitOutcome::Exited { code: Some(0), .. }
                if output.pipes_forced_closed
                    || output.stdout_truncated
                    || output.stderr_truncated =>
            {
                // The shared owner's bounded drain must not turn incomplete
                // status output into permission to remove a worktree. This
                // follows WorktreeGit's existing incomplete-output boundary.
                "git output was incomplete".into()
            }
            ExitOutcome::Exited { code: Some(0), .. } => return Ok(text),
            ExitOutcome::Exited {
                code: Some(code), ..
            } => format!("exit status {code}"),
            ExitOutcome::Exited {
                signal: Some(signal),
                ..
            } => signal_error(signal),
            ExitOutcome::Exited { .. } => "process exited without an exit status".into(),
            ExitOutcome::Cancelled => "context canceled".into(),
            ExitOutcome::TimedOut => if cfg!(windows) {
                "exit status 1"
            } else {
                "signal: killed"
            }
            .into(),
        };
        Err(GitFailure {
            output: text,
            detail,
        })
    }
}

#[derive(Clone, Copy)]
enum GitOutput {
    Stdout,
    Combined,
    Discard,
}
struct GitFailure {
    output: String,
    detail: String,
}

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}
fn minute_stamp(now: Timestamp, offset_seconds: i32) -> Result<String, String> {
    let shifted = time::utc(now)
        .map_err(|error| error.to_string())?
        .checked_add_signed(chrono::Duration::seconds(i64::from(offset_seconds)))
        .ok_or_else(|| "timestamp is invalid or outside the supported range".to_owned())?;
    let year = if shifted.year() < 0 {
        format!("-{:04}", shifted.year().unsigned_abs())
    } else {
        format!("{:04}", shifted.year())
    };
    Ok(format!(
        "{year}{:02}{:02}-{:02}{:02}",
        shifted.month(),
        shifted.day(),
        shifted.hour(),
        shifted.minute()
    ))
}
fn create_dirs(path: &Path, mode: u32) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(mode);
    }
    #[cfg(not(unix))]
    let _ = mode;
    builder.create(path)
}
/// Whether `path` is `root` or lies below it. True if either the resolved
/// paths (links and junctions followed) or the spellings say so, so a session
/// cwd written differently from the recorded tree still counts as inside it.
pub fn path_within(path: &Path, root: &Path) -> bool {
    if let (Ok(path), Ok(root)) = (fs::canonicalize(path), fs::canonicalize(root))
        && path.starts_with(&root)
    {
        return true;
    }
    let (path, root) = (spelling(path), spelling(root));
    path == root || path.starts_with(&format!("{root}/"))
}
/// Separator-, trailing-slash- and (on Windows) case-insensitive spelling.
fn spelling(path: &Path) -> String {
    let mut text = path.to_string_lossy().replace('\\', "/");
    if let Some(rest) = text.strip_prefix("//?/") {
        text = rest.to_owned();
    }
    while text.len() > 1 && text.ends_with('/') {
        text.pop();
    }
    if cfg!(windows) {
        text = text.to_lowercase();
    }
    text
}
fn signal_error(signal: i32) -> String {
    // Git normally exits with a code; preserve Go's common Unix signal wording.
    match signal {
        2 => "signal: interrupt".into(),
        9 => "signal: killed".into(),
        15 => "signal: terminated".into(),
        _ => format!("signal: {signal}"),
    }
}
#[cfg(test)]
mod tests;
