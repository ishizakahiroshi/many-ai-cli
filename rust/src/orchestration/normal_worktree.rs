//! Ordinary-session worktree lifecycle from `internal/hub/normal_worktree.go`.
//!
//! This helper owns only the source's process-wide creation mutex. Pending
//! registration, session metadata, dismissal, and launch rollback remain with
//! their existing callers. Git identity checks used for orchestration reuse
//! are deliberately not added to this distinct source path.
use super::child_launch::{safe_token, worktree::WorktreeGit};
use crate::{
    process::{ExitOutcome, ManagedProcess, SpawnOptions},
    proto::{
        core::NormalWorktree,
        go_quote::quote,
        time::{self, Timestamp},
    },
};
use chrono::{Datelike, Timelike};
use std::{
    ffi::OsString,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::{sync::Mutex, time::Instant};

pub const WORKTREE_CLEANUP_DELETE: &str = "delete";
pub const WORKTREE_CLEANUP_KEEP: &str = "keep";
pub const WORKTREE_CLEANUP_MANUAL: &str = "manual";

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
        if !tree.created || effective_worktree_cleanup(policy) != WORKTREE_CLEANUP_DELETE {
            return Ok(());
        }
        let inspection = Instant::now() + Duration::from_secs(5);
        let status = self
            .run(
                Path::new(&tree.path),
                &args(&["status", "--porcelain"]),
                inspection,
                GitOutput::Stdout,
            )
            .await
            .map_err(|error| format!("inspect worktree status: {}", error.detail))?;
        if !status.trim().is_empty() {
            return Err("worktree retained: uncommitted changes".into());
        }
        if self
            .run(
                Path::new(&tree.parent_dir),
                &args(&["merge-base", "--is-ancestor", &tree.branch, "HEAD"]),
                inspection,
                GitOutput::Discard,
            )
            .await
            .is_err()
        {
            return Err(format!(
                "worktree retained: branch {} is not merged",
                quote(&tree.branch)
            ));
        }
        self.run(
            Path::new(&tree.parent_dir),
            &args(&["worktree", "remove", &tree.path]),
            Instant::now() + Duration::from_secs(30),
            GitOutput::Combined,
        )
        .await
        .map_err(|error| format!("remove worktree: {}: {}", error.output.trim(), error.detail))?;
        Ok(())
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
        let timeout = deadline
            .checked_duration_since(Instant::now())
            .filter(|time| !time.is_zero())
            .ok_or_else(|| GitFailure {
                output: String::new(),
                detail: "context deadline exceeded".into(),
            })?;
        let plan = self.git.command_plan(&self.process_cwd, cwd, argv, timeout);
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
        let output = process.wait().await.map_err(|error| GitFailure {
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
