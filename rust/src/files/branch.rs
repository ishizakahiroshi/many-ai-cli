//! Session Git observations, matching the fixed Go branch/project helpers.
//! Each lookup owns one budget, including its fallback command: 250 ms, or
//! `CHANGES_TIMEOUT` for the worktree statistics.
use super::FilesService;
use crate::{
    process::{
        Cancellation, ExitOutcome, ProcessOutput, ProcessPlan,
        execpath::{NativeFs, Platform, Resolver},
        git::GitCommand,
    },
    proto::core::CoreFuture,
};
use std::{
    ffi::OsString,
    io,
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};

const LOOKUP_TIMEOUT: Duration = Duration::from_millis(250);
/// `status` and `diff --numstat` walk the whole worktree and routinely exceed
/// 250 ms on large or cloud-backed trees. On Windows a timed-out lookup
/// terminates its Job, which stops the real Git mid-scan (Go killed only the
/// `cmd\git.exe` launcher). A Git stopped while reading a streaming cloud
/// folder was observed to stay terminated but never torn down, leaking one
/// process per slow refresh. Refreshes already run per-cwd single-flight in
/// Hub-owned workers, so the longer budget does not delay the UI.
const CHANGES_TIMEOUT: Duration = Duration::from_secs(5);
const OUTPUT_CAP: usize = 16 * 1024 * 1024;

/// `check-ignore` for a batch of changed paths (`files::change_watch`).
const IGNORE_TIMEOUT: Duration = Duration::from_secs(2);

/// `Some("")` is a definitive absence of a project; `None` must be retried.
pub trait GitBranchSource: Send + Sync {
    fn branch<'a>(&'a self, cwd: &'a str) -> CoreFuture<'a, String>;
    fn changes<'a>(&'a self, cwd: &'a str) -> CoreFuture<'a, (i64, i64, i64)>;
    fn project<'a>(&'a self, cwd: &'a str) -> CoreFuture<'a, Option<String>>;
    /// `changes`, and whether Git answered both counts. Only answered counts
    /// are reused until the repository changes.
    fn changes_counted<'a>(&'a self, cwd: &'a str) -> CoreFuture<'a, ((i64, i64, i64), bool)> {
        Box::pin(async move { (self.changes(cwd).await, true) })
    }
    /// Which of `paths` (relative to the work tree `root`; directories end in
    /// `/`) Git ignores. `None` when Git did not answer.
    fn ignored<'a>(
        &'a self,
        _root: &'a Path,
        _paths: &'a [String],
    ) -> CoreFuture<'a, Option<std::collections::BTreeSet<String>>> {
        Box::pin(async { None })
    }
}

impl FilesService {
    /// Go creates each display lookup from the command name and current PATH.
    /// Keep that behavior separate from the existing Git API's selected binary;
    /// explicit FilesService implementations remain useful for isolated runners.
    pub fn branch_reader(&self) -> std::sync::Arc<dyn GitBranchSource> {
        let mut reader = Self::new(self.hub_cwd.clone(), self.paths.clone());
        reader.git_executable = PathBuf::from("git");
        reader.git_environment = self.git_environment.clone();
        std::sync::Arc::new(reader)
    }
}

/// 一時観測 `rust-branch-probe` 用: `branch()` が空を返した理由と所要時間。
/// 返り値や `GitBranchSource` は変えず、`branch()` が内部で作って観測へ渡すだけで、動作には使わない。
/// 撤去するときは、この 3 つの型と `branch_detailed` を `branch()` の本体へ戻す。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(not(debug_assertions), allow(dead_code))]
pub(crate) struct BranchLookupDetail {
    pub blank_cwd: bool,
    /// `rev-parse --abbrev-ref HEAD`。
    pub first: Option<LookupStep>,
    /// `rev-parse --short HEAD`。1 回目が `HEAD`（detached）だったときだけ打つ。
    pub second: Option<LookupStep>,
    pub elapsed: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(not(debug_assertions), allow(dead_code))]
pub(crate) struct LookupStep {
    pub outcome: StepOutcome,
    pub elapsed: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(not(debug_assertions), allow(dead_code))]
pub(crate) enum StepOutcome {
    Ok,
    MissingExecutable,
    Start,
    Expired,
    Output {
        exit_code: Option<i32>,
        signal: Option<i32>,
        timed_out: bool,
        cancelled: bool,
        stdout_truncated: bool,
        stderr_truncated: bool,
        pipes_forced_closed: bool,
    },
}

impl LookupStep {
    fn new(result: &Result<String, LookupError>, elapsed: Duration) -> Self {
        let outcome = match result {
            Ok(_) => StepOutcome::Ok,
            Err(LookupError::MissingExecutable) => StepOutcome::MissingExecutable,
            Err(LookupError::Start) => StepOutcome::Start,
            Err(LookupError::Expired) => StepOutcome::Expired,
            Err(LookupError::Output(output)) => {
                let (exit_code, signal) = match output.outcome {
                    ExitOutcome::Exited { code, signal } => (code, signal),
                    _ => (None, None),
                };
                StepOutcome::Output {
                    exit_code,
                    signal,
                    timed_out: matches!(output.outcome, ExitOutcome::TimedOut),
                    cancelled: matches!(output.outcome, ExitOutcome::Cancelled),
                    stdout_truncated: output.stdout_truncated,
                    stderr_truncated: output.stderr_truncated,
                    pipes_forced_closed: output.pipes_forced_closed,
                }
            }
        };
        Self { outcome, elapsed }
    }
}

impl FilesService {
    /// `branch()` の本体。値は従来と同じで、空になった理由と時間を一緒に返す。
    async fn branch_detailed(&self, cwd: &str) -> (String, BranchLookupDetail) {
        let started = Instant::now();
        let mut detail = BranchLookupDetail::default();
        if cwd.trim().is_empty() {
            detail.blank_cwd = true;
            detail.elapsed = started.elapsed();
            return (String::new(), detail);
        }
        // Read from HEAD: no Git step ran, so `first` stays empty.
        if let Some(name) = head_file_branch(cwd).await {
            detail.elapsed = started.elapsed();
            return (name, detail);
        }
        let git = Lookup::new(self);
        let first_started = Instant::now();
        let first = git.run(cwd, &["rev-parse", "--abbrev-ref", "HEAD"]).await;
        detail.first = Some(LookupStep::new(&first, first_started.elapsed()));
        let Ok(output) = first else {
            detail.elapsed = started.elapsed();
            return (String::new(), detail);
        };
        let branch = output.trim();
        if branch != "HEAD" {
            detail.elapsed = started.elapsed();
            return (branch.into(), detail);
        }
        let second_started = Instant::now();
        let second = git.run(cwd, &["rev-parse", "--short", "HEAD"]).await;
        detail.second = Some(LookupStep::new(&second, second_started.elapsed()));
        let branch = match second {
            Ok(hash) if !hash.trim().is_empty() => format!("detached:{}", hash.trim()),
            _ => String::new(),
        };
        detail.elapsed = started.elapsed();
        (branch, detail)
    }
}

/// The branch the repository's HEAD file names, read without starting Git.
/// Every card refresh asks for the branch, and starting Git for it took
/// longer than the 250 ms budget often enough to show "(no git)" for a
/// repository (2026-10-16). `None` leaves the answer to Git: no repository
/// above `cwd`, a directory that does not exist, a detached HEAD (Git's
/// abbreviation length decides how it is shown), the reftable placeholder
/// `refs/heads/.invalid`, or anything else unexpected.
///
/// Unlike Git, this names the branch of a repository with no commit yet.
async fn head_file_branch(cwd: &str) -> Option<String> {
    let cwd = PathBuf::from(cwd);
    tokio::task::spawn_blocking(move || {
        if !cwd.is_dir() {
            return None;
        }
        let (_, git_dir) = super::git_lock::locate(&cwd)?;
        let head = git_dir.join("HEAD");
        let meta = std::fs::metadata(&head).ok()?;
        if !meta.is_file() || meta.len() > 4096 {
            return None;
        }
        head_branch(&std::fs::read_to_string(&head).ok()?)
    })
    .await
    .ok()
    .flatten()
}

/// `ref: refs/heads/<name>` gives `<name>` when it is a plain branch name.
fn head_branch(content: &str) -> Option<String> {
    let line = content.strip_suffix('\n').unwrap_or(content);
    let line = line.strip_suffix('\r').unwrap_or(line);
    let name = line
        .strip_prefix("ref:")?
        .trim_start_matches([' ', '\t'])
        .strip_prefix("refs/heads/")?;
    let plain = !name.is_empty()
        && name != ".invalid"
        && !name.starts_with(['/', '-', '.'])
        && !name.ends_with(['/', '.'])
        && !name.ends_with(".lock")
        && !name.contains("..")
        && !name.contains("//")
        && !name.contains("@{")
        && !name
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || "~^:?*[\\".contains(c));
    plain.then(|| name.to_owned())
}

impl GitBranchSource for FilesService {
    fn branch<'a>(&'a self, cwd: &'a str) -> CoreFuture<'a, String> {
        Box::pin(async move {
            let (branch, detail) = self.branch_detailed(cwd).await;
            crate::logging::branch_probe::note_lookup(&detail);
            branch
        })
    }

    fn changes<'a>(&'a self, cwd: &'a str) -> CoreFuture<'a, (i64, i64, i64)> {
        Box::pin(async move { self.count_changes(cwd).await.0 })
    }

    fn changes_counted<'a>(&'a self, cwd: &'a str) -> CoreFuture<'a, ((i64, i64, i64), bool)> {
        Box::pin(self.count_changes(cwd))
    }

    fn ignored<'a>(
        &'a self,
        root: &'a Path,
        paths: &'a [String],
    ) -> CoreFuture<'a, Option<std::collections::BTreeSet<String>>> {
        Box::pin(async move {
            if paths.is_empty() {
                return Some(Default::default());
            }
            let git = Lookup::with_budget(self, IGNORE_TIMEOUT);
            let mut plan = git
                .plan(&root.to_string_lossy(), &["check-ignore", "--stdin", "-z"])
                .ok()?;
            plan.stdin = paths
                .iter()
                .flat_map(|path| path.bytes().chain([0]))
                .collect();
            let output = crate::process::run_capped(&plan, &Cancellation::default())
                .await
                .ok()?;
            if output.stdout_truncated || output.stderr_truncated || output.pipes_forced_closed {
                return None;
            }
            // 0: some are ignored (listed), 1: none are.
            match output.outcome {
                ExitOutcome::Exited { code: Some(0), .. } => Some(
                    String::from_utf8_lossy(&output.stdout)
                        .split('\0')
                        .filter(|path| !path.is_empty())
                        .map(str::to_owned)
                        .collect(),
                ),
                ExitOutcome::Exited { code: Some(1), .. } => Some(Default::default()),
                _ => None,
            }
        })
    }

    fn project<'a>(&'a self, cwd: &'a str) -> CoreFuture<'a, Option<String>> {
        Box::pin(async move {
            if cwd.trim().is_empty() {
                return Some(String::new());
            }
            let git = Lookup::new(self);
            let output = match git
                .run(
                    cwd,
                    &["rev-parse", "--path-format=absolute", "--git-common-dir"],
                )
                .await
            {
                Ok(output) => output,
                Err(_) => match git.run(cwd, &["rev-parse", "--git-common-dir"]).await {
                    Ok(output) => output,
                    Err(error) => {
                        return (!git.expired() && error.answered()).then(String::new);
                    }
                },
            };
            project_path(cwd, &output)
        })
    }
}

impl FilesService {
    /// Changed files and line counts, and whether both Git answers arrived.
    async fn count_changes(&self, cwd: &str) -> ((i64, i64, i64), bool) {
        if cwd.trim().is_empty() {
            return ((0, 0, 0), false);
        }
        let git = Lookup::with_budget(self, CHANGES_TIMEOUT);
        let Ok(status) = git.run(cwd, &["status", "--porcelain"]).await else {
            return ((0, 0, 0), false);
        };
        let files = status
            .split('\n')
            .filter(|line| !line.trim().is_empty())
            .count() as i64;
        // Porcelain `git diff` rewrites the index (taking index.lock) to
        // record refreshed stat data even with GIT_OPTIONAL_LOCKS=0. The
        // plumbing equivalent compares the same HEAD/worktree pair without
        // writing; -M keeps porcelain's default rename pairing.
        let Ok(numstat) = git
            .run(cwd, &["diff-index", "-M", "--numstat", "HEAD"])
            .await
        else {
            return ((files, 0, 0), false);
        };
        let (added, deleted) = change_lines(&numstat);
        ((files, added, deleted), true)
    }
}

struct Lookup<'a> {
    service: &'a FilesService,
    until: Instant,
}

#[derive(Debug)]
enum LookupError {
    MissingExecutable,
    Start,
    Expired,
    Output(ProcessOutput),
}

impl LookupError {
    fn answered(&self) -> bool {
        match self {
            Self::MissingExecutable => true,
            Self::Output(output) => {
                matches!(output.outcome, ExitOutcome::Exited { .. })
                    && !output.stdout_truncated
                    && !output.stderr_truncated
                    && !output.pipes_forced_closed
            }
            Self::Start | Self::Expired => false,
        }
    }
}

impl<'a> Lookup<'a> {
    fn new(service: &'a FilesService) -> Self {
        Self::with_budget(service, LOOKUP_TIMEOUT)
    }

    fn with_budget(service: &'a FilesService, budget: Duration) -> Self {
        Self {
            service,
            until: Instant::now() + budget,
        }
    }

    fn expired(&self) -> bool {
        Instant::now() >= self.until
    }

    fn plan(&self, cwd: &str, args: &[&str]) -> Result<ProcessPlan, LookupError> {
        if self.expired() {
            return Err(LookupError::Expired);
        }
        let executable = lookup_executable(self.service)?;
        let timeout = self.until.saturating_duration_since(Instant::now());
        if timeout.is_zero() {
            return Err(LookupError::Expired);
        }
        let mut argv = vec![OsString::from("-C"), OsString::from(cwd)];
        argv.extend(args.iter().map(OsString::from));
        // Display lookups only observe. The entry skips optional locks
        // (status's opportunistic index refresh), so a stopped lookup cannot
        // strand `.git/index.lock` and block the user's own Git commands.
        GitCommand {
            executable,
            args: argv,
            // Go passes only -C; a nonexistent target must reach Git, rather
            // than becoming an indistinguishable pre-exec chdir failure.
            cwd: self.service.hub_cwd.clone(),
            env: self.service.git_environment.clone(),
            stdin: vec![],
            timeout,
            output_cap: OUTPUT_CAP,
            pipe_drain_timeout: LOOKUP_TIMEOUT,
        }
        .plan()
        .map_err(|_| LookupError::Start)
    }

    async fn run(&self, cwd: &str, args: &[&str]) -> Result<String, LookupError> {
        let plan = self.plan(cwd, args)?;
        let output = crate::process::run_capped(&plan, &Cancellation::default())
            .await
            .map_err(|_| LookupError::Start)?;
        if output.stdout_truncated || output.stderr_truncated || output.pipes_forced_closed {
            return Err(LookupError::Output(output));
        }
        match output.outcome {
            ExitOutcome::Exited { code: Some(0), .. } => {
                Ok(String::from_utf8_lossy(&output.stdout).into_owned())
            }
            _ => Err(LookupError::Output(output)),
        }
    }
}

/// Only PATH lookup absence corresponds to Go exec.ErrNotFound. A path that
/// existed during lookup can still fail to spawn (including raw ENOENT from a
/// missing interpreter or working directory); that failure stays unresolved.
fn lookup_executable(service: &FilesService) -> Result<PathBuf, LookupError> {
    let executable = &service.git_executable;
    let Some(name) = executable.to_str() else {
        return Ok(executable.clone());
    };
    if name.contains('/') || (cfg!(windows) && name.contains(['\\', ':'])) {
        return Ok(executable.clone());
    }
    let mut environment: Vec<(OsString, OsString)> = std::env::vars_os().collect();
    for (key, value) in &service.git_environment {
        environment.retain(|(existing, _)| {
            if cfg!(windows) {
                !existing
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&key.to_string_lossy())
            } else {
                existing != key
            }
        });
        if let Some(value) = value {
            environment.push((key.clone(), value.clone()));
        }
    }
    let environment: Vec<String> = environment
        .into_iter()
        .map(|(key, value)| format!("{}={}", key.to_string_lossy(), value.to_string_lossy()))
        .collect();
    Resolver::new(
        Platform::native(),
        &environment,
        &service.hub_cwd,
        &NativeFs,
    )
    .look_path(name)
    .map(PathBuf::from)
    .map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            LookupError::MissingExecutable
        } else {
            LookupError::Start
        }
    })
}

fn change_lines(output: &str) -> (i64, i64) {
    let (mut added, mut deleted) = (0_i64, 0_i64);
    for line in output.split('\n') {
        let mut parts = line.trim().splitn(3, '\t');
        let Some(add) = parts.next() else { continue };
        let Some(del) = parts.next() else { continue };
        // Go Atoi accepts signs, rejects overflow, and leaves each invalid
        // field at zero. Its signed additions wrap rather than panic.
        if let Ok(value) = add.parse::<i64>() {
            added = added.wrapping_add(value);
        }
        if let Ok(value) = del.parse::<i64>() {
            deleted = deleted.wrapping_add(value);
        }
    }
    (added, deleted)
}

fn project_path(cwd: &str, output: &str) -> Option<String> {
    let common = output.trim();
    if common.is_empty() {
        return None;
    }
    let mut common = lexical_clean(Path::new(common));
    if !common.is_absolute() {
        // Go filepath.Join concatenates before cleaning. In particular a
        // Windows rooted, drive-less result must not replace the cwd prefix.
        common = lexical_clean(Path::new(&format!(
            "{cwd}{}{}",
            std::path::MAIN_SEPARATOR,
            common.to_string_lossy()
        )));
    }
    if common.file_name().is_some_and(|name| name == ".git") {
        common = lexical_clean(common.parent().unwrap_or_else(|| Path::new(".")));
    }
    Some(common.to_string_lossy().into_owned())
}

fn lexical_clean(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if result.file_name().is_some_and(|name| name != "..") {
                    result.pop();
                } else if !result.has_root() {
                    result.push("..");
                }
            }
            component => result.push(component.as_os_str()),
        }
    }
    if result.as_os_str().is_empty() {
        result.push(".");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::BTreeMap, fmt::Debug, fs, future::Future};

    // Go's project_id.go documents cold Git exceeding 250ms, and
    // branch_refresh.go retries on later cycles. Wait for fixture observations
    // to converge without changing any production lookup's budget.
    async fn observe_git<T, F, Fut>(
        description: &str,
        expected: &T,
        mut read: F,
        retry: impl Fn(&T) -> bool,
    ) -> T
    where
        T: Debug + PartialEq,
        F: FnMut() -> Fut,
        Fut: Future<Output = T>,
    {
        let started = Instant::now();
        let mut attempts = 0;
        let mut last = None;
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                attempts += 1;
                let actual = read().await;
                if &actual == expected {
                    return actual;
                }
                assert!(
                    retry(&actual),
                    "{description}: expected {expected:?}, got {actual:?} on attempt {attempts}"
                );
                last = Some(actual);
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "{description} did not converge after {attempts} attempts in {:?}: expected {expected:?}, last observation {last:?}",
                started.elapsed()
            )
        })
    }

    struct Fixture {
        owned: tempfile::TempDir,
        service: FilesService,
        root: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let owned = tempfile::tempdir().unwrap();
            let root = owned.path().join("synthetic repo 日本語");
            let runtime = owned.path().join("runtime");
            fs::create_dir(&root).unwrap();
            fs::create_dir(&runtime).unwrap();
            let paths = crate::config::RuntimePaths::trial(
                &runtime,
                49223,
                &owned.path().join("installed"),
            )
            .unwrap();
            let mut service = FilesService::new(owned.path().into(), paths);
            let empty = owned.path().join("empty-config");
            let hooks = owned.path().join("empty-hooks");
            fs::write(&empty, b"").unwrap();
            fs::create_dir(&hooks).unwrap();
            service.git_environment = BTreeMap::from([
                ("GIT_CONFIG_GLOBAL".into(), Some(empty.into_os_string())),
                ("GIT_CONFIG_NOSYSTEM".into(), Some("1".into())),
                ("GIT_CONFIG_COUNT".into(), Some("3".into())),
                ("GIT_CONFIG_KEY_0".into(), Some("core.hooksPath".into())),
                ("GIT_CONFIG_VALUE_0".into(), Some(hooks.into_os_string())),
                ("GIT_CONFIG_KEY_1".into(), Some("commit.gpgSign".into())),
                ("GIT_CONFIG_VALUE_1".into(), Some("false".into())),
                ("GIT_CONFIG_KEY_2".into(), Some("core.autocrlf".into())),
                ("GIT_CONFIG_VALUE_2".into(), Some("false".into())),
                ("GIT_AUTHOR_NAME".into(), Some("Synthetic Author".into())),
                (
                    "GIT_AUTHOR_EMAIL".into(),
                    Some("synthetic@example.invalid".into()),
                ),
                ("GIT_COMMITTER_NAME".into(), Some("Synthetic Author".into())),
                (
                    "GIT_COMMITTER_EMAIL".into(),
                    Some("synthetic@example.invalid".into()),
                ),
                ("GIT_INDEX_FILE".into(), None),
                ("GIT_DIR".into(), None),
                ("GIT_WORK_TREE".into(), None),
                ("GIT_COMMON_DIR".into(), None),
                ("GIT_OBJECT_DIRECTORY".into(), None),
                ("GIT_ALTERNATE_OBJECT_DIRECTORIES".into(), None),
                ("GIT_NAMESPACE".into(), None),
                ("GIT_SHALLOW_FILE".into(), None),
                ("GIT_TEMPLATE_DIR".into(), None),
                ("GIT_CONFIG".into(), None),
                ("GIT_CONFIG_PARAMETERS".into(), None),
            ]);
            Self {
                owned,
                service,
                root,
            }
        }

        async fn git(&self, cwd: &Path, args: &[&str]) -> String {
            let mut argv = vec![OsString::from("-C"), cwd.as_os_str().into()];
            argv.extend(args.iter().map(OsString::from));
            let plan = ProcessPlan {
                executable: self.service.git_executable.clone(),
                args: argv,
                cwd: self.owned.path().into(),
                env: self.service.git_environment.clone(),
                stdin: vec![],
                // Fixture creation is outside the production lookup budget.
                timeout: Duration::from_secs(20),
                output_cap: OUTPUT_CAP,
                pipe_drain_timeout: LOOKUP_TIMEOUT,
            };
            let output = crate::process::run_capped(&plan, &Cancellation::default())
                .await
                .unwrap();
            assert_eq!(
                output.outcome,
                ExitOutcome::Exited {
                    code: Some(0),
                    signal: None
                },
                "owned fixture Git {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                !output.stdout_truncated && !output.stderr_truncated && !output.pipes_forced_closed
            );
            String::from_utf8(output.stdout).unwrap().trim().into()
        }

        async fn init(&self) {
            self.git(&self.root, &["init", "-b", "synthetic"]).await;
        }

        async fn commit(&self) {
            self.git(&self.root, &["add", "--all"]).await;
            self.git(&self.root, &["commit", "-m", "synthetic fixture"])
                .await;
        }

        async fn observed_branch(&self, cwd: &str, expected: &str) -> String {
            observe_git(
                "branch",
                &expected.to_owned(),
                || self.service.branch(cwd),
                String::is_empty,
            )
            .await
        }

        async fn observed_project(&self, cwd: &str, expected: &str) -> Option<String> {
            observe_git(
                "project",
                &Some(expected.to_owned()),
                || self.service.project(cwd),
                Option::is_none,
            )
            .await
        }

        async fn observed_changes(&self, cwd: &str, expected: (i64, i64, i64)) -> (i64, i64, i64) {
            observe_git(
                "changes",
                &expected,
                || self.service.changes(cwd),
                // status failure returns all zeroes; diff failure preserves
                // the successful status count but returns no line counts.
                |actual| *actual == (0, 0, 0) || *actual == (expected.0, 0, 0),
            )
            .await
        }

        fn cwd(&self) -> &str {
            self.root.to_str().unwrap()
        }
    }

    #[tokio::test]
    async fn display_reader_resolves_command_path_without_reusing_git_api_selection() {
        let mut f = Fixture::new();
        let empty_path = f.owned.path().join("empty-path");
        fs::create_dir(&empty_path).unwrap();
        let configured = f.owned.path().join("configured-but-absent-git");
        f.service.git_executable = configured.clone();
        f.service
            .git_environment
            .insert("PATH".into(), Some(empty_path.into_os_string()));
        // An explicit missing path is a startup failure. The production display
        // reader uses Go's bare command lookup, whose PATH absence is definitive.
        assert_eq!(f.service.project(f.root.to_str().unwrap()).await, None);
        assert_eq!(
            f.service
                .branch_reader()
                .project(f.root.to_str().unwrap())
                .await,
            Some(String::new())
        );
        assert_eq!(f.service.git_executable, configured);
    }

    #[tokio::test]
    async fn project_roots_cover_repository_subdirectory_worktree_bare_and_nonrepo() {
        let fixture = Fixture::new();
        fixture.init().await;
        fs::write(fixture.root.join("readme.txt"), "base\n").unwrap();
        fixture.commit().await;
        let expected = project_path(
            fixture.cwd(),
            &fixture
                .git(
                    &fixture.root,
                    &["rev-parse", "--path-format=absolute", "--git-common-dir"],
                )
                .await,
        )
        .unwrap();
        assert_eq!(
            fixture.observed_project(fixture.cwd(), &expected).await,
            Some(expected.clone())
        );

        let subdir = fixture.root.join("inside").join("deep");
        fs::create_dir_all(&subdir).unwrap();
        assert_eq!(
            fixture
                .observed_project(subdir.to_str().unwrap(), &expected)
                .await,
            Some(expected.clone())
        );

        let worktree = fixture.owned.path().join("synthetic worktree");
        fixture
            .git(
                &fixture.root,
                &["worktree", "add", "-b", "side", worktree.to_str().unwrap()],
            )
            .await;
        assert_eq!(
            fixture
                .observed_project(worktree.to_str().unwrap(), &expected)
                .await,
            Some(expected)
        );
        assert_eq!(
            fixture
                .observed_branch(worktree.to_str().unwrap(), "side")
                .await,
            "side"
        );

        let bare = fixture.owned.path().join("synthetic bare.git");
        fs::create_dir(&bare).unwrap();
        fixture.git(&bare, &["init", "--bare"]).await;
        let bare_expected = lexical_clean(Path::new(
            &fixture
                .git(
                    &bare,
                    &["rev-parse", "--path-format=absolute", "--git-common-dir"],
                )
                .await,
        ));
        let bare_expected = bare_expected.to_string_lossy().into_owned();
        assert_eq!(
            fixture
                .observed_project(bare.to_str().unwrap(), &bare_expected)
                .await,
            Some(bare_expected)
        );

        let outside = fixture.owned.path().join("nonrepo");
        fs::create_dir(&outside).unwrap();
        // Establish nonrepo with a definitive project answer. Empty branch
        // and statistics values alone are not readiness evidence.
        assert_eq!(
            fixture
                .observed_project(outside.to_str().unwrap(), "")
                .await,
            Some(String::new())
        );
        assert_eq!(fixture.service.branch(outside.to_str().unwrap()).await, "");
        assert_eq!(
            fixture.service.changes(outside.to_str().unwrap()).await,
            (0, 0, 0)
        );
        let missing = fixture.owned.path().join("missing-target");
        assert_eq!(
            fixture
                .observed_project(missing.to_str().unwrap(), "")
                .await,
            Some(String::new())
        );
    }

    #[tokio::test]
    async fn branches_and_stats_cover_unborn_detached_switch_text_binary_and_untracked() {
        let fixture = Fixture::new();
        fixture.init().await;
        fs::write(fixture.root.join("text.txt"), "one\ntwo\n").unwrap();
        // HEAD already names the branch before the first commit; Git's
        // `rev-parse` failed there and the card showed "(no git)".
        assert_eq!(fixture.service.branch(fixture.cwd()).await, "synthetic");
        assert_eq!(
            fixture.observed_changes(fixture.cwd(), (1, 0, 0)).await,
            (1, 0, 0)
        );
        fs::write(fixture.root.join("binary.dat"), b"\0old\0").unwrap();
        fixture.commit().await;
        assert_eq!(
            fixture.observed_branch(fixture.cwd(), "synthetic").await,
            "synthetic"
        );
        assert_eq!(fixture.service.changes(fixture.cwd()).await, (0, 0, 0));

        fixture
            .git(&fixture.root, &["checkout", "-b", "changed"])
            .await;
        assert_eq!(
            fixture.observed_branch(fixture.cwd(), "changed").await,
            "changed"
        );
        fixture
            .git(&fixture.root, &["checkout", "--detach", "HEAD"])
            .await;
        let short = fixture
            .git(&fixture.root, &["rev-parse", "--short", "HEAD"])
            .await;
        let detached = format!("detached:{short}");
        assert_eq!(
            fixture.observed_branch(fixture.cwd(), &detached).await,
            detached
        );

        fs::write(fixture.root.join("text.txt"), "one\nthree\nfour\n").unwrap();
        fs::write(fixture.root.join("binary.dat"), b"\0new\0").unwrap();
        fs::write(fixture.root.join("untracked.txt"), "not counted as added\n").unwrap();
        assert_eq!(
            fixture.observed_changes(fixture.cwd(), (3, 2, 1)).await,
            (3, 2, 1)
        );
    }

    #[tokio::test]
    async fn head_file_names_the_branch_without_starting_git() {
        let mut fixture = Fixture::new();
        fixture.init().await;
        fs::write(fixture.root.join("readme.txt"), "base\n").unwrap();
        fixture.commit().await;
        let worktree = fixture.owned.path().join("synthetic worktree");
        fixture
            .git(
                &fixture.root,
                &[
                    "worktree",
                    "add",
                    "-b",
                    "side/feature",
                    worktree.to_str().unwrap(),
                ],
            )
            .await;
        let subdir = fixture.root.join("inside");
        fs::create_dir(&subdir).unwrap();
        let outside = fixture.owned.path().join("nonrepo");
        fs::create_dir(&outside).unwrap();
        // From here Git cannot start, so every name below came from HEAD.
        fixture.service.git_executable = fixture.owned.path().join("absent-git");
        assert_eq!(fixture.service.branch(fixture.cwd()).await, "synthetic");
        assert_eq!(
            fixture.service.branch(subdir.to_str().unwrap()).await,
            "synthetic"
        );
        // A linked worktree's `.git` file points at its own HEAD.
        assert_eq!(
            fixture.service.branch(worktree.to_str().unwrap()).await,
            "side/feature"
        );
        // Left to Git (which cannot start here): outside a repository, a
        // directory that does not exist inside one, and a detached HEAD.
        assert_eq!(fixture.service.branch(outside.to_str().unwrap()).await, "");
        let missing = fixture.root.join("missing");
        assert_eq!(fixture.service.branch(missing.to_str().unwrap()).await, "");
        let head = fixture.root.join(".git").join("HEAD");
        fs::write(&head, format!("{}\n", "a".repeat(40))).unwrap();
        assert_eq!(fixture.service.branch(fixture.cwd()).await, "");
    }

    #[test]
    fn head_branch_accepts_only_plain_branch_names() {
        for (content, expected) in [
            ("ref: refs/heads/develop\n", Some("develop")),
            ("ref: refs/heads/feature/日本語\r\n", Some("feature/日本語")),
            ("ref:\trefs/heads/x", Some("x")),
            // reftable keeps this placeholder in HEAD and the real ref elsewhere.
            ("ref: refs/heads/.invalid\n", None),
            ("ref: refs/remotes/origin/main\n", None),
            ("ref: refs/heads/\n", None),
            ("ref: refs/heads/a..b\n", None),
            ("ref: refs/heads/a b\n", None),
            ("ref: refs/heads/x.lock\n", None),
            ("ref: refs/heads/x\nextra\n", None),
            ("0123456789abcdef0123456789abcdef01234567\n", None),
            ("", None),
        ] {
            assert_eq!(head_branch(content).as_deref(), expected, "{content:?}");
        }
    }

    #[tokio::test]
    async fn reftable_repository_is_left_to_git() {
        let fixture = Fixture::new();
        let mut init = std::process::Command::new(&fixture.service.git_executable);
        init.args(["init", "--ref-format=reftable", "-b", "synthetic"])
            .arg(&fixture.root);
        for (key, value) in &fixture.service.git_environment {
            match value {
                Some(value) => init.env(key, value),
                None => init.env_remove(key),
            };
        }
        if !init.output().is_ok_and(|out| out.status.success()) {
            // Git older than 2.45 has no reftable; nothing to check.
            return;
        }
        fs::write(fixture.root.join("readme.txt"), "base\n").unwrap();
        fixture.commit().await;
        assert_eq!(
            head_branch(&fs::read_to_string(fixture.root.join(".git").join("HEAD")).unwrap()),
            None
        );
        assert_eq!(
            fixture.observed_branch(fixture.cwd(), "synthetic").await,
            "synthetic"
        );
    }

    #[tokio::test]
    async fn project_distinguishes_missing_path_lookup_from_spawn_failures() {
        let mut fixture = Fixture::new();
        fixture.service.git_executable = "synthetic-nonexistent-git-command".into();
        fixture
            .service
            .git_environment
            .insert("PATH".into(), Some(fixture.owned.path().into()));
        assert_eq!(
            fixture.service.project(fixture.cwd()).await,
            Some(String::new())
        );
        fixture.service.git_executable = fixture.owned.path().join("nonexistent-absolute-git");
        assert_eq!(fixture.service.project(fixture.cwd()).await, None);
        fixture.service.git_executable = super::super::resolve_git();
        fixture.service.hub_cwd = fixture.owned.path().join("missing-process-cwd");
        assert_eq!(fixture.service.project(fixture.cwd()).await, None);
        for blank in ["", " \n\t"] {
            assert_eq!(fixture.service.project(blank).await, Some(String::new()));
            assert_eq!(fixture.service.branch(blank).await, "");
            assert_eq!(fixture.service.changes(blank).await, (0, 0, 0));
        }
    }

    #[tokio::test]
    async fn branch_detail_explains_an_empty_result_without_changing_the_value() {
        let mut fixture = Fixture::new();
        // 空白だけの作業フォルダは git を呼ばない。
        let (value, detail) = fixture.service.branch_detailed(" \t").await;
        assert_eq!(value, "");
        assert!(detail.blank_cwd && detail.first.is_none() && detail.second.is_none());

        // git のリポジトリの外では 1 回目の rev-parse が 0 以外で終わる。
        // （2026-10-18 から最初のコミット前の repo は HEAD から名前が取れるので、外のフォルダで確かめる。
        //   plan_git-invocation-index-lock.md の C5）
        // 250ms を超えて時間切れになる回もあるので、git 自身の答えが出るまで繰り返す。
        let mut answered = None;
        for _ in 0..40 {
            let (value, detail) = fixture.service.branch_detailed(fixture.cwd()).await;
            assert_eq!(value, "");
            assert!(detail.second.is_none());
            // 時間切れは Expired ではなく timed_out の Output で届く。ここでは git 自身の答えだけを待つ。
            if let Some(StepOutcome::Output {
                exit_code: Some(_), ..
            }) = detail.first.as_ref().map(|s| &s.outcome)
            {
                answered = detail.first;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let Some(LookupStep {
            outcome:
                StepOutcome::Output {
                    exit_code,
                    timed_out,
                    cancelled,
                    ..
                },
            ..
        }) = answered
        else {
            panic!("non-repository folder never produced an answered lookup");
        };
        assert_ne!(exit_code, Some(0));
        assert!(!timed_out && !cancelled);
        // 観測を通す trait 経由の branch() も同じ空を返す。
        assert_eq!(fixture.service.branch(fixture.cwd()).await, "");

        // HEAD から名前が取れた回は git を打たないので、手順はどちらも空のまま届く。
        fixture.init().await;
        let (value, detail) = fixture.service.branch_detailed(fixture.cwd()).await;
        assert_eq!(value, "synthetic");
        assert!(detail.first.is_none() && detail.second.is_none() && !detail.blank_cwd);

        // detached の HEAD は git に聞く。1 回目（--abbrev-ref）と 2 回目（--short）が成功する。
        fs::write(fixture.root.join("a.txt"), "x\n").unwrap();
        fixture.commit().await;
        fixture
            .git(&fixture.root, &["checkout", "--detach", "HEAD"])
            .await;
        let mut seen = None;
        for _ in 0..40 {
            let (value, detail) = fixture.service.branch_detailed(fixture.cwd()).await;
            if value.starts_with("detached:") {
                seen = Some(detail);
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let detail = seen.expect("detached repo never reported its commit");
        assert!(matches!(
            detail.first.map(|s| s.outcome),
            Some(StepOutcome::Ok)
        ));
        assert!(matches!(
            detail.second.map(|s| s.outcome),
            Some(StepOutcome::Ok)
        ));
        assert!(!detail.blank_cwd);

        // PATH に git が無いとき（Go の exec.ErrNotFound 相当）。HEAD は detached のままなので git に聞きに行く。
        let empty_path = fixture.owned.path().join("empty-path");
        fs::create_dir(&empty_path).unwrap();
        fixture.service.git_executable = "synthetic-nonexistent-git-command".into();
        fixture
            .service
            .git_environment
            .insert("PATH".into(), Some(empty_path.into_os_string()));
        let (value, detail) = fixture.service.branch_detailed(fixture.cwd()).await;
        assert_eq!(value, "");
        assert_eq!(
            detail.first.map(|s| s.outcome),
            Some(StepOutcome::MissingExecutable)
        );
    }

    #[test]
    fn lookup_steps_keep_timeout_exit_code_and_pipe_state_apart() {
        let base = ProcessOutput {
            stdout: vec![],
            stderr: vec![],
            stdout_truncated: false,
            stderr_truncated: false,
            pipes_forced_closed: false,
            outcome: ExitOutcome::Exited {
                code: Some(128),
                signal: None,
            },
        };
        let step = |output: ProcessOutput| {
            LookupStep::new(&Err(LookupError::Output(output)), Duration::from_millis(7)).outcome
        };
        let output = |exit_code, timed_out, cancelled, forced| StepOutcome::Output {
            exit_code,
            signal: None,
            timed_out,
            cancelled,
            stdout_truncated: false,
            stderr_truncated: false,
            pipes_forced_closed: forced,
        };
        assert_eq!(step(base.clone()), output(Some(128), false, false, false));
        assert_eq!(
            step(ProcessOutput {
                outcome: ExitOutcome::TimedOut,
                ..base.clone()
            }),
            output(None, true, false, false)
        );
        assert_eq!(
            step(ProcessOutput {
                outcome: ExitOutcome::Cancelled,
                pipes_forced_closed: true,
                ..base
            }),
            output(None, false, true, true)
        );
        let ms = Duration::from_millis(7);
        assert_eq!(
            LookupStep::new(&Ok(String::new()), ms).outcome,
            StepOutcome::Ok
        );
        assert_eq!(
            LookupStep::new(&Err(LookupError::Expired), ms).outcome,
            StepOutcome::Expired
        );
        assert_eq!(
            LookupStep::new(&Err(LookupError::Start), ms).outcome,
            StepOutcome::Start
        );
    }

    #[test]
    fn typed_outcomes_do_not_turn_transient_or_incomplete_output_into_nonrepo() {
        let output = ProcessOutput {
            stdout: vec![],
            stderr: vec![],
            stdout_truncated: false,
            stderr_truncated: false,
            pipes_forced_closed: false,
            outcome: ExitOutcome::Exited {
                code: Some(128),
                signal: None,
            },
        };
        assert!(LookupError::Output(output.clone()).answered());
        assert!(LookupError::MissingExecutable.answered());
        assert!(!LookupError::Start.answered());
        assert!(!LookupError::Expired.answered());
        for outcome in [ExitOutcome::TimedOut, ExitOutcome::Cancelled] {
            assert!(
                !LookupError::Output(ProcessOutput {
                    outcome,
                    ..output.clone()
                })
                .answered()
            );
        }
        assert!(
            !LookupError::Output(ProcessOutput {
                stdout_truncated: true,
                ..output.clone()
            })
            .answered()
        );
        assert!(
            !LookupError::Output(ProcessOutput {
                stderr_truncated: true,
                ..output.clone()
            })
            .answered()
        );
        assert!(
            !LookupError::Output(ProcessOutput {
                pipes_forced_closed: true,
                ..output
            })
            .answered()
        );
    }

    #[test]
    fn command_plan_keeps_argv_environment_and_one_lookup_budget() {
        let fixture = Fixture::new();
        let lookup = Lookup::new(&fixture.service);
        let first = lookup
            .plan("target with spaces", &["rev-parse", "--abbrev-ref", "HEAD"])
            .unwrap();
        let second = lookup
            .plan("target with spaces", &["rev-parse", "--short", "HEAD"])
            .unwrap();
        assert_eq!(
            first.args,
            [
                "-C",
                "target with spaces",
                "rev-parse",
                "--abbrev-ref",
                "HEAD"
            ]
            .map(OsString::from)
        );
        assert_eq!(first.cwd, fixture.service.hub_cwd);
        let mut expected_env = fixture.service.git_environment.clone();
        expected_env.insert("GIT_OPTIONAL_LOCKS".into(), Some("0".into()));
        assert_eq!(first.env, expected_env);
        assert_eq!(first.output_cap, OUTPUT_CAP);
        assert_eq!(first.pipe_drain_timeout, LOOKUP_TIMEOUT);
        assert!(first.timeout <= LOOKUP_TIMEOUT && second.timeout <= first.timeout);
        let stats = Lookup::with_budget(&fixture.service, CHANGES_TIMEOUT)
            .plan("target with spaces", &["status", "--porcelain"])
            .unwrap();
        assert!(stats.timeout > LOOKUP_TIMEOUT && stats.timeout <= CHANGES_TIMEOUT);
        assert_eq!(stats.env, expected_env);
        let expired = Lookup {
            service: &fixture.service,
            until: Instant::now(),
        };
        assert!(matches!(
            expired.plan("cwd", &["status"]),
            Err(LookupError::Expired)
        ));
    }

    #[tokio::test]
    async fn changes_lookup_leaves_the_index_and_its_lock_alone() {
        let fixture = Fixture::new();
        fixture.init().await;
        let tracked = fixture.root.join("tracked.txt");
        fs::write(&tracked, "same\n").unwrap();
        fixture.commit().await;
        // Same content, newer stat data: a refresh would rewrite the index.
        fs::File::options()
            .write(true)
            .open(&tracked)
            .unwrap()
            .set_modified(std::time::SystemTime::now() + Duration::from_secs(10))
            .unwrap();
        // One untracked file makes a successful status distinguishable from
        // the all-zero failure result.
        fs::write(fixture.root.join("untracked.txt"), "new\n").unwrap();
        let index = fixture.root.join(".git").join("index");
        let before = fs::read(&index).unwrap();
        assert_eq!(
            fixture.observed_changes(fixture.cwd(), (1, 0, 0)).await,
            (1, 0, 0)
        );
        assert_eq!(fs::read(&index).unwrap(), before);
        assert!(!fixture.root.join(".git").join("index.lock").exists());
        // Control: the porcelain numstat used before does rewrite it in the
        // same state, so the unchanged index above is the lookup's doing.
        fixture
            .git(&fixture.root, &["diff", "--numstat", "HEAD"])
            .await;
        assert_ne!(fs::read(&index).unwrap(), before);
    }

    #[tokio::test]
    async fn changes_line_counts_match_porcelain_diff_for_renames_and_edits() {
        let fixture = Fixture::new();
        fixture.init().await;
        fs::write(fixture.root.join("moved.txt"), "a\nb\nc\nd\ne\n").unwrap();
        fs::write(fixture.root.join("edited.txt"), "one\ntwo\n").unwrap();
        fixture.commit().await;
        fixture
            .git(&fixture.root, &["mv", "moved.txt", "renamed.txt"])
            .await;
        fs::write(fixture.root.join("edited.txt"), "one\nthree\nfour\n").unwrap();
        let (added, deleted) = change_lines(
            &fixture
                .git(&fixture.root, &["diff", "--numstat", "HEAD"])
                .await,
        );
        // A rename paired as one entry contributes no lines; unpaired it
        // would add and delete the whole file.
        assert_eq!((added, deleted), (2, 1));
        assert_eq!(
            fixture.observed_changes(fixture.cwd(), (2, 2, 1)).await,
            (2, 2, 1)
        );
    }

    #[test]
    fn numstat_matches_go_independent_signed_parsing_binary_and_overflow() {
        assert_eq!(
            change_lines("2\t3\ttext\n-\t-\tbinary\n\nmalformed\n+4\t-2\tother\nno\t7\n8\tbad\n"),
            (14, 8)
        );
        assert_eq!(
            change_lines("9223372036854775808\t-9223372036854775809\tbad\n"),
            (0, 0)
        );
        assert_eq!(
            change_lines("9223372036854775807\t-9223372036854775808\ta\n1\t-1\tb\n"),
            (i64::MIN, i64::MAX)
        );
    }

    #[test]
    fn project_paths_are_lexically_cleaned_without_case_folding_or_io() {
        let root = if cfg!(windows) {
            r"C:\Synthetic\Repo"
        } else {
            "/Synthetic/Repo"
        };
        let cwd = Path::new(root).join("inside");
        assert_eq!(
            project_path(cwd.to_str().unwrap(), "../.git/./\n"),
            Some(root.into())
        );
        assert_eq!(project_path(root, ".git"), Some(root.into()));
        assert_eq!(project_path(root, " \n"), None);
        assert_eq!(
            project_path("relative/inside", "../../bare.git"),
            Some(
                lexical_clean(Path::new("bare.git"))
                    .to_string_lossy()
                    .into_owned()
            )
        );
        assert_eq!(project_path("relative", "../.GIT"), Some(".GIT".into()));
        assert_eq!(
            lexical_clean(Path::new("../../a/../b")),
            PathBuf::from("../../b")
        );
    }

    #[cfg(unix)]
    fn script(fixture: &mut Fixture, content: &str) {
        use std::os::unix::fs::PermissionsExt;
        let executable = fixture.owned.path().join("owned-git-helper");
        fs::write(&executable, format!("#!/bin/sh\n{content}\n")).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        fixture.service.git_executable = executable;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn owned_helper_exercises_legacy_project_fallback_empty_output_and_timeout() {
        let mut fixture = Fixture::new();
        script(
            &mut fixture,
            "if [ \"$4\" = --path-format=absolute ]; then exit 129; fi\nprintf '../.git\\n'",
        );
        assert_eq!(
            fixture.service.project(fixture.cwd()).await,
            Some(fixture.owned.path().to_string_lossy().into_owned())
        );
        script(&mut fixture, "exit 0");
        assert_eq!(fixture.service.project(fixture.cwd()).await, None);
        script(&mut fixture, "while :; do :; done");
        assert_eq!(fixture.service.project(fixture.cwd()).await, None);
        script(
            &mut fixture,
            "if [ \"$4\" = --abbrev-ref ]; then printf 'HEAD\\n'; else exit 1; fi",
        );
        assert_eq!(fixture.service.branch(fixture.cwd()).await, "");
        script(
            &mut fixture,
            "if [ \"$3\" = status ]; then printf ' M one\\n\\n?? two\\n'; else exit 128; fi",
        );
        assert_eq!(fixture.service.changes(fixture.cwd()).await, (2, 0, 0));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn changes_wait_past_the_short_lookup_budget_instead_of_stopping_git() {
        let mut fixture = Fixture::new();
        // Each command takes longer than the 250 ms lookup budget but stays
        // well inside the statistics budget.
        script(
            &mut fixture,
            "sleep 0.6\nif [ \"$3\" = status ]; then printf ' M one\\n'; else printf '2\\t1\\tone\\n'; fi",
        );
        assert_eq!(fixture.service.changes(fixture.cwd()).await, (1, 2, 1));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn existing_helper_with_missing_interpreter_is_unresolved() {
        let mut fixture = Fixture::new();
        script(&mut fixture, "exit 0");
        fs::write(
            &fixture.service.git_executable,
            format!(
                "#!{}\n",
                fixture.owned.path().join("missing-interpreter").display()
            ),
        )
        .unwrap();
        assert_eq!(fixture.service.project(fixture.cwd()).await, None);
    }
}
