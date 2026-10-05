//! Shared relay worktrees. Every Git operation uses the existing process owner
//! and explicit environment; cleanup never merges/deletes the result branch.
use crate::{
    config::{OrchestrationConfig, RuntimePaths},
    files::safe_fs::Dir,
    orchestration::child_launch::{safe_token, worktree::WorktreeGit},
    process::{self, ExitOutcome},
    proto::core::TaskCancellation,
};
use std::{
    ffi::OsString,
    fs,
    io::{self, Write},
    path::{Component, Path, PathBuf},
    time::Duration,
};

#[derive(Clone)]
pub struct RelayGit {
    paths: RuntimePaths,
    git: WorktreeGit,
    cwd: PathBuf,
}
#[derive(Debug)]
pub struct GitError {
    pub detail: String,
    pub not_git: bool,
}
impl RelayGit {
    pub fn new(paths: RuntimePaths, git: WorktreeGit, cwd: PathBuf) -> Self {
        Self { paths, git, cwd }
    }
    pub fn root(cwd: &Path, cfg: &OrchestrationConfig) -> PathBuf {
        let configured = cfg.worktree_dir_root.trim();
        let root = if configured.is_empty() {
            PathBuf::from(".many-ai-cli").join("worktrees")
        } else {
            PathBuf::from(configured)
        };
        clean(if root.is_absolute() {
            root
        } else {
            cwd.join(root)
        })
    }
    fn check(&self, path: &Path) -> Result<(), String> {
        crate::profile::subscriptions::check_path(&self.paths, path).map_err(|e| e.to_string())
    }
    pub async fn run(
        &self,
        cwd: &Path,
        args: &[&str],
        timeout: Duration,
        cancel: &TaskCancellation,
    ) -> Result<String, String> {
        self.check(cwd)?;
        let _metadata = super::trial_git::inspect(&self.paths, cwd, args.first() == Some(&"init"))
            .map_err(|error| error.to_string())?;
        let args = args.iter().map(OsString::from).collect::<Vec<_>>();
        let mut plan = self.git.command_plan(&self.cwd, cwd, &args, timeout);
        let _policy = super::trial_git::configure(&self.paths, &mut plan)
            .map_err(|error| error.to_string())?;
        plan.output_cap = 16 * 1024 * 1024;
        let output = process::run_capped(&plan, cancel.token())
            .await
            .map_err(|e| e.to_string())?;
        if output.stdout_truncated || output.stderr_truncated || output.pipes_forced_closed {
            return Err("git output was incomplete".into());
        }
        match output.outcome {
            ExitOutcome::Exited { code: Some(0), .. } => {
                Ok(String::from_utf8_lossy(&output.stdout).into_owned())
            }
            ExitOutcome::Cancelled => Err("context canceled".into()),
            ExitOutcome::TimedOut => Err("context deadline exceeded".into()),
            other => Err(format!(
                "{}: {other:?}",
                String::from_utf8_lossy(&output.stderr).trim()
            )),
        }
    }
    pub async fn head(&self, cwd: &Path, cancel: &TaskCancellation) -> Result<String, String> {
        self.run(cwd, &["rev-parse", "HEAD"], Duration::from_secs(3), cancel)
            .await
            .map(|s| s.trim().into())
    }
    pub async fn changed(
        &self,
        cwd: &Path,
        from: &str,
        cancel: &TaskCancellation,
    ) -> Result<i64, String> {
        let range = format!("{}..HEAD", from.trim());
        let args = if from.trim().is_empty() || !valid_revision(from.trim()) {
            vec!["status", "--short"]
        } else {
            vec!["diff", "--name-only", &range]
        };
        Ok(self
            .run(cwd, &args, Duration::from_secs(3), cancel)
            .await?
            .lines()
            .filter(|s| !s.trim().is_empty())
            .count() as i64)
    }
    pub async fn prepare(
        &self,
        parent: &Path,
        id: &str,
        cfg: &OrchestrationConfig,
        cancel: &TaskCancellation,
    ) -> Result<(PathBuf, String, String), GitError> {
        let error = |detail| GitError {
            detail,
            not_git: false,
        };
        self.check(parent).map_err(error)?;
        let top=self.run(parent,&["rev-parse","--show-toplevel"],Duration::from_secs(3),cancel).await.map_err(|_|GitError{detail:"parent cwd is not a git repository; pick same-tree mode or start the relay inside a repository".into(),not_git:true})?;
        let root = Self::root(parent, cfg);
        self.check(&root).map_err(error)?;
        let token = safe_token(id);
        let path = root.join(&token).join("relay");
        let branch = format!("many-ai-cli/relay/{token}");
        if fs::metadata(&path).is_ok() {
            self.validate(parent, &path, &branch, cancel)
                .await
                .map_err(error)?;
            return Ok((path, branch, String::new()));
        }
        let base = self.head(parent, cancel).await.map_err(error)?;
        Dir::open_or_create_private(path.parent().expect("relay parent"))
            .map_err(|e| error(e.to_string()))?;
        if let Ok(relative) = root.strip_prefix(top.trim())
            && !relative.as_os_str().is_empty()
        {
            let entry = format!("/{}/", relative.to_string_lossy().replace('\\', "/"));
            self.exclude(parent, &entry, cancel).await;
        }
        let target = path.to_string_lossy();
        if let Err(first) = self
            .run(
                parent,
                &["worktree", "add", "-b", &branch, &target, &base],
                Duration::from_secs(30),
                cancel,
            )
            .await
        {
            if first.contains("already exists") {
                self.run(
                    parent,
                    &["worktree", "add", &target, &branch],
                    Duration::from_secs(30),
                    cancel,
                )
                .await
                .map_err(error)?;
            } else {
                return Err(error(first));
            }
        }
        Ok((path, branch, base))
    }
    pub async fn validate(
        &self,
        parent: &Path,
        path: &Path,
        branch: &str,
        cancel: &TaskCancellation,
    ) -> Result<(), String> {
        self.check(parent)?;
        self.check(path)?;
        if self.paths.is_trial() {
            Dir::open(path).map_err(|e| e.to_string())?;
        }
        let _parent_metadata = super::trial_git::inspect(&self.paths, parent, false)
            .map_err(|error| error.to_string())?;
        let _child_metadata = super::trial_git::inspect(&self.paths, path, false)
            .map_err(|error| error.to_string())?;
        let mut plan = self
            .git
            .command_plan(&self.cwd, parent, &[], Duration::from_secs(3));
        let _policy = super::trial_git::configure(&self.paths, &mut plan)
            .map_err(|error| error.to_string())?;
        WorktreeGit::new(plan.executable, plan.env)
            .validate_identity(parent, path, branch, cancel)
            .await
    }
    async fn exclude(&self, parent: &Path, entry: &str, cancel: &TaskCancellation) {
        let Ok(common) = self
            .run(
                parent,
                &["rev-parse", "--git-common-dir"],
                Duration::from_secs(3),
                cancel,
            )
            .await
        else {
            return;
        };
        let common = PathBuf::from(common.trim());
        let common = clean(if common.is_absolute() {
            common
        } else {
            parent.join(common)
        });
        if self.check(&common).is_err() {
            return;
        }
        let Ok(dir) = Dir::open_or_create_private(&common.join("info")) else {
            return;
        };
        let prior = dir.read("exclude", 1024 * 1024).unwrap_or_default();
        if String::from_utf8_lossy(&prior)
            .lines()
            .any(|s| s.trim() == entry)
        {
            return;
        }
        let Ok(mut file) = dir.open_append("exclude") else {
            return;
        };
        let prefix = if !prior.is_empty() && prior.last() != Some(&b'\n') {
            "\n"
        } else {
            ""
        };
        let _ = writeln!(file, "{prefix}{entry}");
    }
    pub fn validate_cleanup(
        &self,
        parent: &Path,
        path: &Path,
        cfg: &OrchestrationConfig,
    ) -> Result<(), String> {
        self.check(path)?;
        if !path.is_absolute() {
            return Err("relay worktree path must be absolute".into());
        }
        let root = Self::root(parent, cfg);
        let path = clean(path.to_owned());
        if path == root {
            return Err("relay worktree path cannot be the worktree root".into());
        }
        let actual_root = fs::canonicalize(&root).map_err(|_| {
            "relay worktree path is outside the configured worktree root".to_owned()
        })?;
        let actual_path = fs::canonicalize(&path).map_err(|_| {
            "relay worktree path is outside the configured worktree root".to_owned()
        })?;
        if actual_path == actual_root || !actual_path.starts_with(&actual_root) {
            return Err("relay worktree path is outside the configured worktree root".into());
        }
        Ok(())
    }
    pub async fn cleanup(
        &self,
        parent: &Path,
        path: &Path,
        cancel: &TaskCancellation,
    ) -> Result<(), String> {
        self.check(parent)?;
        self.check(path)?;
        let target = path.to_string_lossy();
        if let Err(error) = self
            .run(
                parent,
                &["worktree", "remove", "--force", &target],
                Duration::from_secs(30),
                cancel,
            )
            .await
        {
            let list=self.run(parent,&["worktree","list","--porcelain"],Duration::from_secs(30),cancel).await.map_err(|verify|format!("remove relay worktree: {error} (could not verify worktree registration: {verify})"))?;
            if list
                .lines()
                .filter_map(|s| s.strip_prefix("worktree "))
                .any(|s| clean(PathBuf::from(s)) == clean(path.to_owned()))
            {
                return Err(format!(
                    "relay child may still be using the worktree; close the child sessions and retry cleanup: {error}"
                ));
            }
            self.run(
                parent,
                &["worktree", "prune"],
                Duration::from_secs(30),
                cancel,
            )
            .await?;
            let parent = Dir::open(path.parent().ok_or("relay worktree has no parent")?)
                .map_err(|e| e.to_string())?;
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or("relay worktree has no basename")?;
            match parent.remove_tree(name) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(format!(
                        "relay worktree was unregistered but its directory is still in use; close child sessions and retry cleanup: {e}"
                    ));
                }
            }
        }
        // No recursive parent cleanup and no branch deletion.
        if let Some(parent) = path.parent() {
            let _ = fs::remove_dir(parent);
        }
        Ok(())
    }
    pub async fn resolve_plan(
        &self,
        parent: &Path,
        raw: &str,
        cancel: &TaskCancellation,
    ) -> Result<PathBuf, String> {
        let fail = |text: &str| format!("plan path must be an absolute path: {text}");
        if raw.trim().is_empty() {
            return Err(fail("plan_path is required"));
        }
        if parent.as_os_str().is_empty() {
            return Err(fail("parent session has no cwd"));
        }
        let parent = if parent.is_absolute() {
            parent.to_owned()
        } else {
            self.cwd.join(parent)
        };
        self.check(&parent)?;
        let base = fs::canonicalize(&parent).map_err(|_| fail("parent cwd is unavailable"))?;
        if !base.is_dir() {
            return Err(fail("parent cwd is not a directory"));
        }
        let requested = Path::new(raw.trim());
        let lexical = clean(if requested.is_absolute() {
            requested.to_owned()
        } else {
            base.join(requested)
        });
        self.check(&lexical)?;
        let path = fs::canonicalize(&lexical).map_err(|_| fail("plan file does not exist"))?;
        self.check(&path)?;
        let meta = fs::metadata(&path).map_err(|_| fail("plan file does not exist"))?;
        if !meta.is_file() {
            return Err(fail("plan_path must be a regular file"));
        }
        if !path
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("md"))
        {
            return Err(fail("plan_path must point to a .md file"));
        }
        if meta.len() > 1024 * 1024 {
            return Err(fail("plan_path must be at most 1048576 bytes"));
        }
        let git_root = self
            .run(
                &base,
                &["rev-parse", "--show-toplevel"],
                Duration::from_secs(3),
                cancel,
            )
            .await
            .unwrap_or_default();
        let resolved_root = if git_root.trim().is_empty() {
            PathBuf::new()
        } else {
            fs::canonicalize(git_root.trim()).unwrap_or_else(|_| PathBuf::from(git_root.trim()))
        };
        let root = resolved_root.as_path();
        let inside = |p: &Path| {
            p.starts_with(&base) || (!root.as_os_str().is_empty() && p.starts_with(root))
        };
        if !inside(&path)
            && (self.paths.is_trial()
                || !inside(&lexical)
                || fs::symlink_metadata(&lexical).map_or(true, |m| m.file_type().is_symlink()))
        {
            return Err(fail("plan_path is outside the parent cwd and git root"));
        }
        if self.paths.is_trial() {
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or_else(|| fail("plan file is unavailable"))?;
            Dir::open(path.parent().expect("plan parent"))
                .and_then(|dir| dir.open_file(name, false))
                .map_err(|_| fail("plan file is unavailable"))?;
        }
        Ok(path)
    }
}
pub(super) fn clean(path: PathBuf) -> PathBuf {
    let mut result = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(result.components().next_back(), Some(Component::Normal(_))) {
                    result.pop();
                } else if !result.has_root() {
                    result.push("..");
                }
            }
            other => result.push(other.as_os_str()),
        }
    }
    result
}
fn valid_revision(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('-')
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._/-".contains(c))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    struct Fixture {
        _root: tempfile::TempDir,
        paths: RuntimePaths,
        repo: PathBuf,
        git: RelayGit,
    }
    fn home_for_hooks(runtime: &Path) -> PathBuf {
        let hooks = runtime.join("empty-hooks");
        fs::create_dir_all(&hooks).unwrap();
        hooks
    }
    async fn fixture() -> Fixture {
        let root = tempfile::tempdir().unwrap();
        let runtime = root.path().join("trial");
        fs::create_dir(&runtime).unwrap();
        let paths = RuntimePaths::trial(&runtime, 49661, &root.path().join("installed")).unwrap();
        let home = runtime.join("home");
        fs::create_dir(&home).unwrap();
        let empty = runtime.join("empty-git-config");
        fs::write(&empty, "").unwrap();
        let env: BTreeMap<OsString, Option<OsString>> = BTreeMap::from([
            ("HOME".into(), Some(home.clone().into_os_string())),
            ("USERPROFILE".into(), Some(home.into_os_string())),
            ("GIT_CONFIG_NOSYSTEM".into(), Some("1".into())),
            ("GIT_CONFIG_COUNT".into(), Some("0".into())),
            ("GIT_CONFIG_GLOBAL".into(), Some(empty.into_os_string())),
            ("GIT_CONFIG_PARAMETERS".into(), None),
            ("GIT_CONFIG".into(), None),
            ("GIT_DIR".into(), None),
            ("GIT_WORK_TREE".into(), None),
            ("GIT_INDEX_FILE".into(), None),
            ("GIT_COMMON_DIR".into(), None),
            ("GIT_OBJECT_DIRECTORY".into(), None),
            ("GIT_ALTERNATE_OBJECT_DIRECTORIES".into(), None),
        ]);
        let git = RelayGit::new(
            paths.clone(),
            WorktreeGit::new("git".into(), env),
            runtime.clone(),
        );
        let repo = runtime.join("repository with spaces 日本");
        fs::create_dir(&repo).unwrap();
        let cancel = TaskCancellation::default();
        for args in [
            &["init", "-q"][..],
            &["config", "user.name", "Synthetic Relay Fixture"],
            &["config", "user.email", "relay@example.invalid"],
            &["config", "commit.gpgsign", "false"],
            &[
                "config",
                "core.hooksPath",
                home_for_hooks(&runtime).to_str().unwrap(),
            ],
        ] {
            git.run(&repo, args, Duration::from_secs(10), &cancel)
                .await
                .unwrap();
        }
        fs::write(repo.join("plan.md"), "# Synthetic plan\n").unwrap();
        git.run(&repo, &["add", "plan.md"], Duration::from_secs(10), &cancel)
            .await
            .unwrap();
        git.run(
            &repo,
            &["commit", "-qm", "synthetic base"],
            Duration::from_secs(10),
            &cancel,
        )
        .await
        .unwrap();
        Fixture {
            _root: root,
            paths,
            repo,
            git,
        }
    }
    #[tokio::test]
    async fn shared_worktree_keeps_parent_unchanged_and_cleanup_retains_result_branch() {
        let f = fixture().await;
        let cancel = TaskCancellation::default();
        let cfg = OrchestrationConfig::default();
        let initial = f.git.head(&f.repo, &cancel).await.unwrap();
        let (path, branch, base) = f
            .git
            .prepare(&f.repo, "r7-synthetic", &cfg, &cancel)
            .await
            .unwrap();
        assert_eq!(base, initial);
        assert_eq!(branch, "many-ai-cli/relay/r7-synthetic");
        assert!(path.is_dir());
        fs::write(path.join("result with spaces.txt"), "synthetic result\n").unwrap();
        f.git
            .run(
                &path,
                &["add", "result with spaces.txt"],
                Duration::from_secs(10),
                &cancel,
            )
            .await
            .unwrap();
        f.git
            .run(
                &path,
                &["commit", "-qm", "synthetic result"],
                Duration::from_secs(10),
                &cancel,
            )
            .await
            .unwrap();
        let result = f.git.head(&path, &cancel).await.unwrap();
        assert_ne!(result, base);
        assert_eq!(f.git.head(&f.repo, &cancel).await.unwrap(), initial);
        assert!(!f.repo.join("result with spaces.txt").exists());
        assert_eq!(f.git.changed(&path, &base, &cancel).await.unwrap(), 1);
        let reused = f
            .git
            .prepare(&f.repo, "r7-synthetic", &cfg, &cancel)
            .await
            .unwrap();
        assert_eq!(reused.0, path);
        assert!(reused.2.is_empty());
        f.git.validate_cleanup(&f.repo, &path, &cfg).unwrap();
        f.git.cleanup(&f.repo, &path, &cancel).await.unwrap();
        assert!(!path.exists());
        assert_eq!(
            f.git
                .run(
                    &f.repo,
                    &["rev-parse", "--verify", &branch],
                    Duration::from_secs(10),
                    &cancel
                )
                .await
                .unwrap()
                .trim(),
            result
        );
        let restored = f
            .git
            .prepare(&f.repo, "r7-synthetic", &cfg, &cancel)
            .await
            .unwrap();
        assert_eq!(restored.0, path);
        assert_eq!(f.git.head(&path, &cancel).await.unwrap(), result);
        assert_eq!(f.git.head(&f.repo, &cancel).await.unwrap(), initial);
        assert!(path.starts_with(f.paths.root()));
    }
    #[tokio::test]
    async fn worktree_reuse_rejects_wrong_branch_without_repair() {
        let f = fixture().await;
        let cancel = TaskCancellation::default();
        let cfg = OrchestrationConfig::default();
        let (path, _, _) = f
            .git
            .prepare(&f.repo, "r8-synthetic", &cfg, &cancel)
            .await
            .unwrap();
        f.git
            .run(
                &path,
                &["checkout", "-qb", "unrelated-synthetic"],
                Duration::from_secs(10),
                &cancel,
            )
            .await
            .unwrap();
        let error = f
            .git
            .prepare(&f.repo, "r8-synthetic", &cfg, &cancel)
            .await
            .unwrap_err();
        assert!(error.detail.contains("identity mismatch"));
        assert_eq!(
            f.git
                .run(
                    &path,
                    &["branch", "--show-current"],
                    Duration::from_secs(10),
                    &cancel
                )
                .await
                .unwrap()
                .trim(),
            "unrelated-synthetic"
        );
    }
    #[tokio::test]
    async fn canceled_prepare_and_non_repository_never_fall_back_to_same_tree() {
        let f = fixture().await;
        let missing = f.paths.root().join("non-repository");
        fs::create_dir(&missing).unwrap();
        let cfg = OrchestrationConfig::default();
        let error = f
            .git
            .prepare(&missing, "r9-synthetic", &cfg, &TaskCancellation::default())
            .await
            .unwrap_err();
        assert!(error.not_git);
        assert!(!RelayGit::root(&missing, &cfg).exists());
        let cancel = TaskCancellation::default();
        cancel.token().cancel();
        assert!(
            f.git
                .prepare(&f.repo, "r10-canceled", &cfg, &cancel)
                .await
                .is_err()
        );
        assert!(!RelayGit::root(&f.repo, &cfg).join("r10-canceled").exists());
    }
}

#[cfg(test)]
mod trial_boundary_tests {
    use super::*;
    use std::collections::BTreeMap;
    fn fixture() -> (tempfile::TempDir, RuntimePaths, RelayGit, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let trial = root.path().join("trial");
        fs::create_dir(&trial).unwrap();
        let paths = RuntimePaths::trial(&trial, 49663, &root.path().join("installed")).unwrap();
        let repo = trial.join("repository");
        fs::create_dir(&repo).unwrap();
        let git = RelayGit::new(
            paths.clone(),
            WorktreeGit::new(trial.join("must-not-execute"), BTreeMap::new()),
            trial,
        );
        (root, paths, git, repo)
    }
    #[tokio::test]
    async fn trial_gitdir_and_commondir_cannot_touch_outside_metadata() {
        for direct in [true, false] {
            let (root, _paths, git, repo) = fixture();
            let outside = root.path().join("outside");
            fs::create_dir(&outside).unwrap();
            fs::write(outside.join("HEAD"), "FORBIDDEN SYNTHETIC SENTINEL\n").unwrap();
            if direct {
                fs::write(
                    repo.join(".git"),
                    format!("gitdir: {}\n", outside.display()),
                )
                .unwrap();
            } else {
                fs::create_dir(repo.join(".git")).unwrap();
                fs::write(
                    repo.join(".git/commondir"),
                    outside.to_string_lossy().as_bytes(),
                )
                .unwrap();
            }
            let error = git
                .head(&repo, &TaskCancellation::default())
                .await
                .unwrap_err();
            assert!(!error.contains("FORBIDDEN SYNTHETIC SENTINEL"));
            assert_eq!(
                fs::read_to_string(outside.join("HEAD")).unwrap(),
                "FORBIDDEN SYNTHETIC SENTINEL\n"
            );
            assert!(!outside.join("HEAD.lock").exists());
        }
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn trial_git_metadata_symlinks_and_configuration_includes_fail_closed() {
        use std::os::unix::fs::symlink;
        let (root, _paths, git, repo) = fixture();
        let outside = root.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("sentinel"), "untouched").unwrap();
        symlink(&outside, repo.join(".git")).unwrap();
        assert!(git.head(&repo, &TaskCancellation::default()).await.is_err());
        fs::remove_file(repo.join(".git")).unwrap();
        fs::create_dir(repo.join(".git")).unwrap();
        fs::write(
            repo.join(".git/config"),
            format!("[include]\npath={}\n", outside.join("sentinel").display()),
        )
        .unwrap();
        let error = git
            .head(&repo, &TaskCancellation::default())
            .await
            .unwrap_err();
        assert!(!error.contains("untouched"));
        assert_eq!(
            fs::read_to_string(outside.join("sentinel")).unwrap(),
            "untouched"
        );
    }
    #[tokio::test]
    async fn oversized_trial_git_metadata_is_rejected_before_git_execution() {
        for name in [
            "config",
            "config.worktree",
            "commondir",
            "gitdir",
            "objects/info/alternates",
        ] {
            let (_root, paths, git, repo) = fixture();
            let metadata = repo.join(".git");
            fs::create_dir_all(metadata.join("objects/info")).unwrap();
            let cap = if matches!(name, "commondir" | "gitdir") {
                4096
            } else {
                1024 * 1024
            };
            let mut value = if name.starts_with("config") {
                b"#\n".repeat(cap / 2)
            } else {
                vec![b' '; cap]
            };
            value.extend_from_slice(b"[include]\npath = /synthetic-outside/not-read\n");
            fs::write(metadata.join(name), value).unwrap();
            let result = super::super::trial_git::inspect(&paths, &repo, false);
            assert!(
                result.is_err(),
                "oversized {name} must never be prefix-accepted"
            );
            let error = git
                .head(&repo, &TaskCancellation::default())
                .await
                .unwrap_err();
            assert!(
                error.contains("trial Git metadata"),
                "unexpected pre-exec error: {error}"
            );
        }
        let (_root, paths, _git, repo) = fixture();
        let metadata = paths.root().join("selected-metadata");
        fs::create_dir(&metadata).unwrap();
        let prefix = format!("gitdir: {}\n", metadata.display());
        let mut bytes = prefix.into_bytes();
        bytes.resize(4097, b' ');
        fs::write(repo.join(".git"), bytes).unwrap();
        assert!(super::super::trial_git::inspect(&paths, &repo, false).is_err());
    }

    #[test]
    fn trial_policy_removes_ambient_repository_and_hook_execution_overrides() {
        let (_root, paths, git, repo) = fixture();
        let mut plan = git
            .git
            .command_plan(&repo, &repo, &[], Duration::from_secs(1));
        plan.env.insert("GIT_DIR".into(), Some("outside".into()));
        plan.env
            .insert("GIT_CONFIG_PARAMETERS".into(), Some("unsafe".into()));
        let _pins = super::super::trial_git::configure(&paths, &mut plan).unwrap();
        assert_eq!(plan.env[&OsString::from("GIT_DIR")], None);
        assert_eq!(plan.env[&OsString::from("GIT_CONFIG_PARAMETERS")], None);
        assert_eq!(
            plan.env[&OsString::from("GIT_ALLOW_PROTOCOL")],
            Some("".into())
        );
        assert_eq!(
            plan.env[&OsString::from("GIT_TERMINAL_PROMPT")],
            Some("0".into())
        );
        assert_eq!(
            plan.env[&OsString::from("GIT_CONFIG_COUNT")],
            Some("2".into())
        );
        assert_eq!(
            plan.env[&OsString::from("GIT_CONFIG_VALUE_1")],
            Some("false".into())
        );
        let hooks = PathBuf::from(
            plan.env[&OsString::from("GIT_CONFIG_VALUE_0")]
                .clone()
                .unwrap(),
        );
        assert!(hooks.starts_with(paths.root()));
        assert!(fs::read_dir(hooks).unwrap().next().is_none());
    }
}
