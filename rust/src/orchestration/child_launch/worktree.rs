//! Actual argv-only Git preparation, ported from orchestration.go and
//! worktree_identity.go. Existing worktrees are checked read-only, never repaired.
use super::{board::private_dirs, clean_path, safe_token};
use crate::{
    config::OrchestrationConfig,
    process::{self, ExitOutcome, ProcessPlan},
    proto::core::TaskCancellation,
};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::time::Instant;

#[derive(Clone)]
pub struct WorktreeGit {
    executable: PathBuf,
    /// Explicit additions/removals, useful for isolated synthetic repositories.
    /// Never changes HOME or the process-wide environment.
    env: BTreeMap<OsString, Option<OsString>>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorktreePreparation {
    pub cwd: PathBuf,
    pub branch: String,
    pub note: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorktreeError {
    IdentityMismatch(String),
    Cancelled,
}
struct GitFailure {
    output: String,
    detail: String,
}
impl WorktreeGit {
    pub fn new(executable: PathBuf, env: BTreeMap<OsString, Option<OsString>>) -> Self {
        Self { executable, env }
    }
    /// Keep the process directory independent of Git's `-C` operand so a
    /// relative repository spelling is resolved exactly once by Git.
    pub fn command_plan(
        &self,
        process_cwd: &Path,
        git_cwd: &Path,
        argv: &[OsString],
        timeout: Duration,
    ) -> ProcessPlan {
        ProcessPlan {
            executable: self.executable.clone(),
            args: [
                vec![OsString::from("-C"), git_cwd.as_os_str().to_owned()],
                argv.to_vec(),
            ]
            .concat(),
            cwd: process_cwd.to_owned(),
            env: self.env.clone(),
            stdin: Vec::new(),
            timeout,
            output_cap: usize::MAX,
            pipe_drain_timeout: Duration::from_secs(1),
        }
    }
    async fn run(
        &self,
        cwd: &Path,
        args: &[OsString],
        timeout: Duration,
        cancel: &TaskCancellation,
    ) -> Result<String, GitFailure> {
        let plan = ProcessPlan {
            executable: self.executable.clone(),
            args: [
                vec![OsString::from("-C"), cwd.as_os_str().to_owned()],
                args.to_vec(),
            ]
            .concat(),
            cwd: cwd.to_owned(),
            env: self.env.clone(),
            stdin: Vec::new(),
            timeout,
            output_cap: usize::MAX,
            pipe_drain_timeout: Duration::from_secs(1),
        };
        let output = process::run_capped(&plan, cancel.token())
            .await
            .map_err(|e| GitFailure {
                output: String::new(),
                detail: e.to_string(),
            })?;
        // Git's fixed commands report diagnostics on stderr; successful query
        // results are stdout. Both are retained in failed add notes.
        let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
        combined.push_str(&String::from_utf8_lossy(&output.stderr));
        let detail = match output.outcome {
            ExitOutcome::Exited { code: Some(0), .. }
                if !output.stdout_truncated
                    && !output.stderr_truncated
                    && !output.pipes_forced_closed =>
            {
                return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
            }
            ExitOutcome::Exited { code: Some(0), .. } => "git output was incomplete".into(),
            ExitOutcome::Exited {
                code: Some(code), ..
            } => format!("exit status {code}"),
            ExitOutcome::Exited { signal, .. } => format!("git terminated by signal {signal:?}"),
            ExitOutcome::Cancelled => "context canceled".into(),
            ExitOutcome::TimedOut => "signal: killed".into(),
        };
        Err(GitFailure {
            output: combined,
            detail,
        })
    }
    pub async fn prepare(
        &self,
        cwd: &Path,
        orchestration: &str,
        role: &str,
        cfg: &OrchestrationConfig,
        cancel: &TaskCancellation,
    ) -> Result<WorktreePreparation, WorktreeError> {
        let skip = |note: String| WorktreePreparation {
            cwd: cwd.to_owned(),
            branch: String::new(),
            note,
        };
        check_cancel(cancel)?;
        if !cfg.worktree_enabled() {
            return Ok(skip("worktree skip: disabled by config".into()));
        }
        let is_git = self
            .run(
                cwd,
                &args(&["rev-parse", "--show-toplevel"]),
                Duration::from_secs(3),
                cancel,
            )
            .await;
        check_cancel(cancel)?;
        if is_git.is_err() {
            return Ok(skip(
                "worktree skip: parent cwd is not a git repository".into(),
            ));
        }
        let branch = format!("orch/{}/{}", safe_token(orchestration), safe_token(role));
        let configured_root = Path::new(&cfg.worktree_dir_root);
        let root = if configured_root.is_absolute() {
            configured_root.to_owned()
        } else {
            cwd.join(configured_root)
        };
        let child = clean_path(&root.join(safe_token(orchestration)).join(safe_token(role)));
        if let Err(error) = private_dirs(child.parent().expect("child worktree has parent")) {
            return Ok(skip(format!("worktree skip: {error}")));
        }
        if fs::metadata(&child).is_ok() {
            let identity = self.validate_identity(cwd, &child, &branch, cancel).await;
            check_cancel(cancel)?;
            identity.map_err(|detail| {
                WorktreeError::IdentityMismatch(format!("worktree reuse rejected: {detail}"))
            })?;
            return Ok(WorktreePreparation {
                note: format!("worktree reuse: {} branch={branch}", child.display()),
                cwd: child,
                branch,
            });
        }
        let result = self
            .run(
                cwd,
                &[
                    "worktree".into(),
                    "add".into(),
                    "-b".into(),
                    branch.clone().into(),
                    child.as_os_str().to_owned(),
                ],
                Duration::from_secs(30),
                cancel,
            )
            .await;
        check_cancel(cancel)?;
        if let Err(error) = result {
            return Ok(skip(format!(
                "worktree skip: {} {}",
                error.output.trim(),
                error.detail
            )));
        }
        Ok(WorktreePreparation {
            note: format!("worktree created: {} branch={branch}", child.display()),
            cwd: child,
            branch,
        })
    }
    pub async fn validate_identity(
        &self,
        parent: &Path,
        child: &Path,
        branch: &str,
        cancel: &TaskCancellation,
    ) -> Result<(), String> {
        let fail = |detail: String| {
            format!(
                "worktree identity mismatch: path={} expected_branch={branch}: {detail}",
                child.display()
            )
        };
        if parent.as_os_str().is_empty() {
            return Err(fail("parent cwd is empty".into()));
        }
        if child.as_os_str().is_empty() {
            return Err(fail("worktree path is empty".into()));
        }
        if branch.trim().is_empty() {
            return Err(fail("expected branch is empty".into()));
        }
        let meta =
            fs::metadata(child).map_err(|e| fail(format!("worktree path is unavailable: {e}")))?;
        if !meta.is_dir() {
            return Err(fail("worktree path is not a directory".into()));
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        let parent_common = self
            .common_dir(parent, deadline, cancel)
            .await
            .map_err(|e| fail(format!("parent git common directory unavailable: {e}")))?;
        let child_common = self
            .common_dir(child, deadline, cancel)
            .await
            .map_err(|e| fail(format!("worktree git common directory unavailable: {e}")))?;
        if !same_path(&parent_common, &child_common) {
            return Err(fail(format!(
                "git common directory changed: parent={} worktree={}",
                parent_common.display(),
                child_common.display()
            )));
        }
        let target = fs::canonicalize(child)
            .map_err(|e| fail(format!("worktree registration unavailable: {e}")))?;
        let list = self
            .query(
                parent,
                &["worktree", "list", "--porcelain"],
                deadline,
                cancel,
            )
            .await
            .map_err(|e| fail(format!("worktree registration unavailable: {e}")))?;
        let registered = list
            .lines()
            .filter_map(|line| line.strip_prefix("worktree "))
            .filter_map(|path| fs::canonicalize(path.trim()).ok())
            .any(|listed| same_path(&target, &listed));
        if !registered {
            return Err(fail(
                "directory is not registered by the parent repository".into(),
            ));
        }
        let actual = self
            .query(child, &["branch", "--show-current"], deadline, cancel)
            .await
            .map_err(|e| fail(format!("checked-out branch unavailable: {e}")))?;
        if actual.trim() != branch {
            return Err(fail(format!("checked-out branch is {:?}", actual.trim())));
        }
        Ok(())
    }
    async fn query(
        &self,
        cwd: &Path,
        argv: &[&str],
        deadline: Instant,
        cancel: &TaskCancellation,
    ) -> Result<String, String> {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or("context deadline exceeded")?;
        self.run(cwd, &args(argv), remaining, cancel)
            .await
            .map_err(|e| e.detail)
    }
    async fn common_dir(
        &self,
        cwd: &Path,
        deadline: Instant,
        cancel: &TaskCancellation,
    ) -> Result<PathBuf, String> {
        let output = match self
            .query(
                cwd,
                &["rev-parse", "--path-format=absolute", "--git-common-dir"],
                deadline,
                cancel,
            )
            .await
        {
            Ok(output) => output,
            Err(_) => {
                self.query(cwd, &["rev-parse", "--git-common-dir"], deadline, cancel)
                    .await?
            }
        };
        let common = output.trim();
        if common.is_empty() {
            return Err("git returned an empty common directory".into());
        }
        let path = Path::new(common);
        let path = if path.is_absolute() {
            path.to_owned()
        } else {
            cwd.join(path)
        };
        fs::canonicalize(clean_path(&path)).map_err(|e| e.to_string())
    }
}
fn check_cancel(cancel: &TaskCancellation) -> Result<(), WorktreeError> {
    if cancel.token().is_cancelled() {
        Err(WorktreeError::Cancelled)
    } else {
        Ok(())
    }
}
fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}
fn same_path(left: &Path, right: &Path) -> bool {
    if cfg!(windows) {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    } else {
        left == right
    }
}
impl From<io::Error> for WorktreeError {
    fn from(error: io::Error) -> Self {
        Self::IdentityMismatch(error.to_string())
    }
}
