use super::*;
use crate::proto::time::parse_rfc3339;
use serde_json::{Value, json};
use std::{collections::BTreeMap, ffi::OsStr};
use tempfile::TempDir;

struct Fixture {
    _root: TempDir,
    root: PathBuf,
    lifecycle: NormalWorktreeLifecycle,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        // Git for Windows cannot read GIT_CONFIG_GLOBAL in Rust's verbatim
        // canonical spelling. Keep the native absolute tempfile spelling for
        // external Git inputs; Unix retains canonical system-alias resolution.
        #[cfg(windows)]
        let root = temp.path().to_path_buf();
        #[cfg(not(windows))]
        let root = fs::canonicalize(temp.path()).unwrap();
        let home = root.join("home");
        fs::create_dir(&home).unwrap();
        fs::create_dir(root.join("hooks")).unwrap();
        fs::create_dir(root.join("templates")).unwrap();
        let empty = root.join("empty-git-config");
        fs::write(&empty, "").unwrap();
        let mut env = BTreeMap::<OsString, Option<OsString>>::new();
        for key in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_COMMON_DIR",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_CONFIG",
            "GIT_CONFIG_PARAMETERS",
            "GIT_CONFIG_SYSTEM",
            "GIT_EXEC_PATH",
            "GIT_NAMESPACE",
            "GIT_CEILING_DIRECTORIES",
            "GIT_AUTHOR_DATE",
            "GIT_COMMITTER_DATE",
        ] {
            env.insert(key.into(), None);
        }
        for (key, value) in [
            ("HOME", home.as_os_str()),
            ("USERPROFILE", home.as_os_str()),
            ("XDG_CONFIG_HOME", home.as_os_str()),
            ("GIT_CONFIG_GLOBAL", empty.as_os_str()),
            ("GIT_CONFIG_NOSYSTEM", OsStr::new("1")),
            ("GIT_CONFIG_COUNT", OsStr::new("0")),
            ("GIT_TERMINAL_PROMPT", OsStr::new("0")),
            ("GIT_AUTHOR_NAME", OsStr::new("Synthetic Fixture")),
            ("GIT_AUTHOR_EMAIL", OsStr::new("fixture@example.invalid")),
            ("GIT_COMMITTER_NAME", OsStr::new("Synthetic Fixture")),
            ("GIT_COMMITTER_EMAIL", OsStr::new("fixture@example.invalid")),
        ] {
            env.insert(key.into(), Some(value.to_owned()));
        }
        env.insert(
            "GIT_TEMPLATE_DIR".into(),
            Some(root.join("templates").into_os_string()),
        );
        let executable = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|path| path.join(if cfg!(windows) { "git.exe" } else { "git" }))
            .find(|path| path.is_file())
            .expect("functional normal-worktree fixtures require Git on PATH");
        let lifecycle =
            NormalWorktreeLifecycle::new(root.clone(), WorktreeGit::new(executable, env));
        Self {
            _root: temp,
            root,
            lifecycle,
        }
    }
    async fn git(&self, cwd: &Path, argv: &[&str]) -> String {
        self.lifecycle
            .run(
                cwd,
                &args(argv),
                Instant::now() + Duration::from_secs(10),
                GitOutput::Combined,
            )
            .await
            .unwrap_or_else(|error| {
                panic!(
                    "synthetic Git {:?}: {}: {}",
                    argv, error.detail, error.output
                )
            })
            .trim()
            .to_owned()
    }
    async fn repo(&self, name: &str) -> PathBuf {
        let path = self.root.join(name);
        fs::create_dir(&path).unwrap();
        self.git(&path, &["init", "-q"]).await;
        self.git(&path, &["config", "user.name", "Synthetic Fixture"])
            .await;
        self.git(&path, &["config", "user.email", "fixture@example.invalid"])
            .await;
        self.git(&path, &["config", "commit.gpgsign", "false"])
            .await;
        self.git(
            &path,
            &[
                "config",
                "core.hooksPath",
                self.root.join("hooks").to_str().unwrap(),
            ],
        )
        .await;
        fs::write(path.join("base.txt"), "base\n").unwrap();
        self.git(&path, &["add", "base.txt"]).await;
        self.git(&path, &["commit", "-qm", "synthetic base"]).await;
        path
    }
    async fn prepare(&self, cwd: &Path, label: &str) -> NormalWorktree {
        self.lifecycle
            .prepare(cwd, label, now(), 9 * 3600)
            .await
            .unwrap_or_else(|error| panic!("prepare: {error}"))
    }
    /// A clone whose `develop` tracks `origin/develop` on a bare synthetic
    /// origin, and a second clone standing in for whoever merges there.
    async fn tracked(&self, name: &str) -> Tracked {
        let origin = self.root.join(format!("{name}-origin.git"));
        fs::create_dir(&origin).unwrap();
        self.git(&origin, &["init", "-q", "--bare"]).await;
        let repo = self.repo(name).await;
        self.git(&repo, &["branch", "-M", "develop"]).await;
        self.git(
            &repo,
            &["remote", "add", "origin", origin.to_str().unwrap()],
        )
        .await;
        self.git(&repo, &["push", "-q", "-u", "origin", "develop"])
            .await;
        let other = self.root.join(format!("{name}-other"));
        self.git(
            &self.root,
            &[
                "clone",
                "-q",
                "-b",
                "develop",
                origin.to_str().unwrap(),
                other.to_str().unwrap(),
            ],
        )
        .await;
        Tracked {
            repo,
            origin,
            other,
        }
    }
    async fn commit(&self, tree: &NormalWorktree, file: &str) {
        let path = Path::new(&tree.path);
        fs::write(path.join(file), "synthetic work\n").unwrap();
        self.git(path, &["add", file]).await;
        self.git(path, &["commit", "-qm", "synthetic work"]).await;
    }
    /// Commit, then publish the branch the way a session pushes its work.
    async fn publish(&self, tree: &NormalWorktree, file: &str) {
        self.commit(tree, file).await;
        self.git(
            Path::new(&tree.path),
            &["push", "-q", "origin", &tree.branch],
        )
        .await;
    }
    /// A merge commit on origin's develop, made in the other clone, so this
    /// clone's local develop and its `origin/develop` both stay behind.
    async fn merge_on_origin(&self, tracked: &Tracked, branch: &str) {
        self.git(&tracked.other, &["fetch", "-q", "origin"]).await;
        self.git(
            &tracked.other,
            &[
                "merge",
                "-q",
                "--no-ff",
                "-m",
                "synthetic merge",
                &format!("origin/{branch}"),
            ],
        )
        .await;
        self.git(&tracked.other, &["push", "-q", "origin", "develop"])
            .await;
    }
    async fn is_ancestor(&self, repo: &Path, branch: &str, target: &str) -> bool {
        self.lifecycle
            .run(
                repo,
                &args(&["merge-base", "--is-ancestor", branch, target]),
                Instant::now() + Duration::from_secs(10),
                GitOutput::Discard,
            )
            .await
            .is_ok()
    }
    fn store(&self) -> RetainedWorktrees {
        RetainedWorktrees::new(
            crate::files::safe_fs::Dir::open_or_create_private_components(
                &self.root.join("runtime"),
            )
            .unwrap(),
        )
    }
    async fn recheck(&self, store: &RetainedWorktrees) -> RecheckReport {
        self.lifecycle
            .recheck_retained(store, &|_| false, &Cancellation::default())
            .await
    }
}
struct Tracked {
    repo: PathBuf,
    origin: PathBuf,
    other: PathBuf,
}
fn not_merged(tree: &NormalWorktree) -> String {
    format!(
        "worktree retained: branch {} is not merged",
        quote(&tree.branch)
    )
}
fn now() -> Timestamp {
    parse_rfc3339("2026-07-11T12:34:59.000000123+09:00").unwrap()
}
fn prefix<T>(result: Result<T, String>) -> String {
    match result {
        Ok(_) => String::new(),
        Err(error) => error.split(':').next().unwrap().to_owned(),
    }
}
fn relative(tree: &NormalWorktree) -> Value {
    json!({
        "path": Path::new(&tree.path).strip_prefix(&tree.parent_dir).unwrap().to_string_lossy().replace('\\', "/"),
        "branch": tree.branch,
        "created": tree.created,
    })
}
fn expected() -> Value {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/application/normal-worktree/expected.json"
    ))
    .unwrap()
}

#[test]
fn policy_token_and_local_clock_match_pinned_go() {
    let observed = expected();
    for row in observed["policies"].as_array().unwrap() {
        let value = row["value"].as_str().unwrap();
        assert_eq!(
            valid_worktree_cleanup(value),
            row["valid"].as_bool().unwrap()
        );
        assert_eq!(effective_worktree_cleanup(value), row["effective"]);
    }
    for row in observed["names"].as_array().unwrap() {
        assert_eq!(safe_token(row["label"].as_str().unwrap()), row["safe"]);
    }
    assert_eq!(minute_stamp(now(), 9 * 3600).unwrap(), "20260711-1234");
    assert_eq!(minute_stamp(now(), -5 * 3600).unwrap(), "20260710-2234");
}

#[tokio::test]
async fn real_git_lifecycle_matches_pinned_go_observer() {
    let f = Fixture::new();
    let expected = expected();
    let repo = f.repo("repo with space 日本語").await;
    let sub = repo.join("subdir");
    fs::create_dir(&sub).unwrap();
    let exclude = repo.join(".git/info/exclude");
    fs::create_dir_all(exclude.parent().unwrap()).unwrap();
    fs::write(&exclude, "# keep this without newline").unwrap();
    let first = f.prepare(&sub, "a.b").await;
    let second = f.prepare(&repo, "a.b").await;
    assert_eq!(
        json!([relative(&first), relative(&second)]),
        expected["created"]
    );
    assert_eq!(fs::read_to_string(&exclude).unwrap(), expected["exclude"]);
    assert_eq!(
        f.git(&repo, &["status", "--porcelain"]).await.is_empty(),
        expected["parent_clean"]
    );
    fs::write(Path::new(&first.path).join("untracked.txt"), "work\n").unwrap();
    assert_eq!(
        f.lifecycle.cleanup(&first, "delete").await.unwrap_err(),
        expected["dirty"]
    );
    for policy in ["keep", "manual", "", "unknown"] {
        f.lifecycle.cleanup(&first, policy).await.unwrap();
    }
    assert_eq!(
        Path::new(&first.path).exists(),
        expected["retained_policies_keep_path"]
    );
    f.git(Path::new(&first.path), &["add", "untracked.txt"])
        .await;
    f.git(Path::new(&first.path), &["commit", "-qm", "synthetic work"])
        .await;
    assert_eq!(
        f.lifecycle.cleanup(&first, "delete").await.unwrap_err(),
        expected["unmerged"]
    );
    f.git(&repo, &["merge", "--ff-only", &first.branch]).await;
    f.lifecycle.cleanup(&first, "delete").await.unwrap();
    f.lifecycle.cleanup(&second, "delete").await.unwrap();
    assert_eq!(
        !Path::new(&first.path).exists() && !Path::new(&second.path).exists(),
        expected["removed_after_merge"]
    );
    assert_eq!(
        !f.git(&repo, &["branch", "--list", "many-ai/*"])
            .await
            .is_empty(),
        expected["branches_retained"]
    );
    // Each exact ref remains, not merely another matching branch.
    assert!(
        !f.git(&repo, &["rev-parse", "--verify", &first.branch])
            .await
            .is_empty()
    );
    assert!(
        !f.git(&repo, &["rev-parse", "--verify", &second.branch])
            .await
            .is_empty()
    );
    assert_eq!(
        repo.join(".git-worktrees").exists(),
        expected["root_retained"]
    );
    assert_eq!(
        prefix(f.lifecycle.prepare(&repo, "a.b", now(), 9 * 3600).await),
        expected["retained_branch_collision"]
    );
    assert_eq!(
        prefix(f.lifecycle.cleanup(&first, "delete").await),
        expected["missing_cleanup"]
    );
    let empty = f.root.join("empty");
    fs::create_dir(&empty).unwrap();
    f.git(&empty, &["init", "-q"]).await;
    assert_eq!(
        prefix(f.lifecycle.prepare(&empty, "unborn", now(), 9 * 3600).await),
        expected["unborn_create"]
    );
    assert_eq!(
        empty.join(".git-worktrees").exists(),
        expected["unborn_root_retained"]
    );
    let nongit = f.root.join("not-git");
    fs::create_dir(&nongit).unwrap();
    assert_eq!(
        prefix(f.lifecycle.prepare(&nongit, "no", now(), 9 * 3600).await),
        expected["not_git"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn separate_instances_share_complete_creation_mutex() {
    let f = Fixture::new();
    let repo = f.repo("concurrent").await;
    let a = f.lifecycle.clone();
    let b = NormalWorktreeLifecycle::new(f.root.clone(), f.lifecycle.git.clone());
    let (a, b) = tokio::join!(
        a.prepare(&repo, "review worker", now(), 9 * 3600),
        b.prepare(&repo, "review worker", now(), 9 * 3600)
    );
    let a = a.unwrap_or_else(|error| panic!("first: {error}"));
    let b = b.unwrap_or_else(|error| panic!("second: {error}"));
    let mut names = vec![
        Path::new(&a.path)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        Path::new(&b.path)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
    ];
    names.sort();
    assert_eq!(json!(names), expected()["concurrent_names"]);
    f.lifecycle.cleanup(&a, "delete").await.unwrap();
    f.lifecycle.cleanup(&b, "delete").await.unwrap();
}

#[tokio::test]
async fn existing_files_collide_and_root_mode_is_not_rewritten() {
    let f = Fixture::new();
    let repo = f.repo("collisions").await;
    let root = repo.join(".git-worktrees");
    fs::create_dir(&root).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o750)).unwrap();
    }
    fs::write(root.join("worker-20260711-1234"), "existing synthetic file").unwrap();
    fs::create_dir(root.join("worker-20260711-1234-2")).unwrap();
    let tree = f.prepare(&repo, "worker").await;
    assert_eq!(
        Path::new(&tree.path).file_name().unwrap(),
        "worker-20260711-1234-3"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&root).unwrap().permissions().mode() & 0o777,
            0o750
        );
    }
    f.lifecycle.cleanup(&tree, "delete").await.unwrap();
}

#[tokio::test]
async fn keep_manual_and_uncreated_do_not_start_git() {
    let f = Fixture::new();
    let missing_git = WorktreeGit::new(f.root.join("missing-git-executable"), BTreeMap::new());
    let lifecycle = NormalWorktreeLifecycle::new(f.root.clone(), missing_git);
    let mut tree = NormalWorktree {
        path: "absent".into(),
        created: true,
        ..Default::default()
    };
    for policy in ["", "keep", "manual", "wrong", "Delete"] {
        lifecycle.cleanup(&tree, policy).await.unwrap();
    }
    tree.created = false;
    lifecycle.cleanup(&tree, "delete").await.unwrap();
    let error = lifecycle
        .run(
            &f.root,
            &args(&["status"]),
            Instant::now(),
            GitOutput::Stdout,
        )
        .await
        .err()
        .unwrap();
    assert_eq!(error.detail, "context deadline exceeded");
}

#[tokio::test]
async fn tracked_dirty_and_staged_work_are_retained() {
    let f = Fixture::new();
    let repo = f.repo("dirty").await;
    let tree = f.prepare(&repo, "worker").await;
    let path = Path::new(&tree.path);
    fs::write(path.join("base.txt"), "changed\n").unwrap();
    assert_eq!(
        f.lifecycle.cleanup(&tree, "delete").await.unwrap_err(),
        "worktree retained: uncommitted changes"
    );
    f.git(path, &["add", "base.txt"]).await;
    assert_eq!(
        f.lifecycle.cleanup(&tree, "delete").await.unwrap_err(),
        "worktree retained: uncommitted changes"
    );
    assert!(path.exists());
}

#[tokio::test]
async fn ignored_files_do_not_block_source_git_remove() {
    let f = Fixture::new();
    let repo = f.repo("ignored").await;
    let tree = f.prepare(&repo, "worker").await;
    let exclude = repo.join(".git/info/exclude");
    let mut file = OpenOptions::new().append(true).open(&exclude).unwrap();
    writeln!(file, "ignored.txt").unwrap();
    fs::write(
        Path::new(&tree.path).join("ignored.txt"),
        "synthetic ignored file\n",
    )
    .unwrap();
    f.lifecycle.cleanup(&tree, "delete").await.unwrap();
    assert_eq!(
        !Path::new(&tree.path).exists(),
        expected()["ignored_removed"]
    );
}

#[tokio::test]
async fn existing_exclude_line_is_byte_preserved_and_failed_query_is_best_effort() {
    let f = Fixture::new();
    let repo = f.repo("excluded").await;
    let exclude = repo.join(".git/info/exclude");
    fs::create_dir_all(exclude.parent().unwrap()).unwrap();
    let original = b"# synthetic\r\n  .git-worktrees/ \r\n\xff";
    fs::write(&exclude, original).unwrap();
    let tree = f.prepare(&repo, "worker").await;
    assert_eq!(fs::read(&exclude).unwrap(), original);
    f.lifecycle.cleanup(&tree, "delete").await.unwrap();
    let nongit = f.root.join("not-git");
    fs::create_dir(&nongit).unwrap();
    f.lifecycle
        .exclude_git_path(
            &nongit,
            "synthetic/",
            Instant::now() + Duration::from_secs(3),
        )
        .await;
    assert_eq!(fs::read_dir(&nongit).unwrap().count(), 0);
}

#[tokio::test]
async fn linked_parent_uses_shared_info_exclude_and_relative_cwd_uses_process_cwd() {
    let f = Fixture::new();
    let repo = f.repo("main").await;
    let linked = f.root.join("linked parent");
    f.git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "linked",
            linked.to_str().unwrap(),
            "HEAD",
        ],
    )
    .await;
    let tree = f.prepare(Path::new("linked parent"), "nested").await;
    assert_eq!(
        fs::canonicalize(Path::new(&tree.parent_dir)).unwrap(),
        fs::canonicalize(&linked).unwrap(),
        "Git and the fixture must identify the same linked parent directory"
    );
    assert_eq!(
        fs::canonicalize(Path::new(&tree.path).parent().unwrap()).unwrap(),
        fs::canonicalize(linked.join(".git-worktrees")).unwrap(),
        "worktree path must remain below the linked parent"
    );
    assert!(
        fs::read_to_string(repo.join(".git/info/exclude"))
            .unwrap()
            .lines()
            .any(|line| line == ".git-worktrees/")
    );
    assert!(f.git(&linked, &["status", "--porcelain"]).await.is_empty());
    f.lifecycle.cleanup(&tree, "delete").await.unwrap();
}

#[tokio::test]
async fn root_create_failure_and_invalid_git_token_preserve_error_prefixes() {
    let f = Fixture::new();
    let blocked = f.repo("blocked").await;
    fs::write(blocked.join(".git-worktrees"), "ordinary synthetic file").unwrap();
    assert_eq!(
        prefix(f.lifecycle.prepare(&blocked, "worker", now(), 0).await),
        "create worktree root"
    );
    let repo = f.repo("invalid-branch").await;
    assert_eq!(
        prefix(f.lifecycle.prepare(&repo, "a..b", now(), 0).await),
        "create worktree"
    );
    assert!(repo.join(".git-worktrees").exists());
    assert!(
        fs::read_to_string(repo.join(".git/info/exclude"))
            .unwrap()
            .contains(".git-worktrees/")
    );
}

#[tokio::test]
async fn locked_worktree_is_retained_with_source_remove_error() {
    let f = Fixture::new();
    let repo = f.repo("locked").await;
    let tree = f.prepare(&repo, "worker").await;
    f.git(&repo, &["worktree", "lock", &tree.path]).await;
    let error = f.lifecycle.cleanup(&tree, "delete").await.unwrap_err();
    assert!(error.ends_with(": exit status 128"), "{error}");
    assert_eq!(prefix::<()>(Err(error)), expected()["locked_remove"]);
    assert_eq!(
        Path::new(&tree.path).exists(),
        expected()["locked_retained"]
    );
    f.git(&repo, &["worktree", "unlock", &tree.path]).await;
    f.lifecycle.cleanup(&tree, "delete").await.unwrap();
}
#[tokio::test]
async fn cleanup_clean_check_leaves_the_worktree_index_and_its_lock_alone() {
    let f = Fixture::new();
    let repo = f.repo("parent").await;
    let tree = f.prepare(&repo, "stat-only").await;
    let path = PathBuf::from(&tree.path);
    let gitdir = PathBuf::from(f.git(&path, &["rev-parse", "--absolute-git-dir"]).await);
    // Same content, newer stat data: a refreshing status rewrites the index.
    fs::File::options()
        .write(true)
        .open(path.join("base.txt"))
        .unwrap()
        .set_modified(std::time::SystemTime::now() + Duration::from_secs(30))
        .unwrap();
    // An untracked file keeps the tree, so its index remains to compare.
    fs::write(path.join("untracked.txt"), "new\n").unwrap();
    let index = gitdir.join("index");
    let before = fs::read(&index).unwrap();
    let error = f.lifecycle.cleanup(&tree, "delete").await.unwrap_err();
    assert_eq!(error, "worktree retained: uncommitted changes");
    assert_eq!(fs::read(&index).unwrap(), before);
    assert!(!gitdir.join("index.lock").exists());
    // Control: the same status outside the entry rewrites it in this state.
    let control = std::process::Command::new("git")
        .arg("-C")
        .arg(&path)
        .args(["status", "--porcelain"])
        .env_remove("GIT_OPTIONAL_LOCKS")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", f.root.join("empty-git-config"))
        .output()
        .unwrap();
    assert!(control.status.success());
    assert_ne!(fs::read(&index).unwrap(), before);
}

// (a) Sessions push their work to the remote branch; the parent's own develop
// stays old until someone pulls. Closing must still see the merge.
#[tokio::test]
async fn upstream_merge_removes_the_tree_while_local_develop_stays_behind() {
    let f = Fixture::new();
    let t = f.tracked("pushed").await;
    let tree = f.prepare(&t.repo, "worker").await;
    f.publish(&tree, "work.txt").await;
    f.merge_on_origin(&t, &tree.branch).await;
    let develop = f.git(&t.repo, &["rev-parse", "develop"]).await;
    // Neither the local HEAD nor the not-yet-fetched origin/develop has it.
    assert!(!f.is_ancestor(&t.repo, &tree.branch, "HEAD").await);
    assert!(
        !f.is_ancestor(&t.repo, &tree.branch, "refs/remotes/origin/develop")
            .await
    );
    f.lifecycle.cleanup(&tree, "delete").await.unwrap();
    assert!(!Path::new(&tree.path).exists());
    assert_eq!(f.git(&t.repo, &["rev-parse", "develop"]).await, develop);
    assert!(
        !f.git(&t.repo, &["rev-parse", "--verify", &tree.branch])
            .await
            .is_empty()
    );
    assert!(
        f.is_ancestor(&t.repo, &tree.branch, "refs/remotes/origin/develop")
            .await
    );
}

// (b) Unmerged when its session closed, merged afterwards: the next start's
// recheck removes it and drops it from the list.
#[tokio::test]
async fn merged_after_close_is_removed_by_the_startup_recheck() {
    let f = Fixture::new();
    let t = f.tracked("later").await;
    let tree = f.prepare(&t.repo, "worker").await;
    f.publish(&tree, "work.txt").await;
    let store = f.store();
    assert_eq!(
        f.lifecycle.cleanup(&tree, "delete").await.unwrap_err(),
        not_merged(&tree)
    );
    store.remember(&tree).unwrap();
    assert_eq!(f.recheck(&store).await.retained, 1);
    f.merge_on_origin(&t, &tree.branch).await;
    assert_eq!(
        f.recheck(&store).await,
        RecheckReport {
            removed: 1,
            forgotten: 1,
            ..Default::default()
        }
    );
    assert!(!Path::new(&tree.path).exists());
    assert!(store.load().unwrap().is_empty());
}

// (c) Uncommitted work, or a branch merged nowhere (the upstream is fetched
// and checked too), is kept at close and by every recheck.
#[tokio::test]
async fn dirty_or_unmerged_everywhere_is_kept_and_stays_listed() {
    let f = Fixture::new();
    let t = f.tracked("kept").await;
    let dirty = f.prepare(&t.repo, "dirty").await;
    fs::write(Path::new(&dirty.path).join("base.txt"), "changed\n").unwrap();
    let unmerged = f.prepare(&t.repo, "unmerged").await;
    f.publish(&unmerged, "work.txt").await;
    let store = f.store();
    assert_eq!(
        f.lifecycle.cleanup(&dirty, "delete").await.unwrap_err(),
        "worktree retained: uncommitted changes"
    );
    assert_eq!(
        f.lifecycle.cleanup(&unmerged, "delete").await.unwrap_err(),
        not_merged(&unmerged)
    );
    store.remember(&dirty).unwrap();
    store.remember(&unmerged).unwrap();
    assert_eq!(
        f.recheck(&store).await,
        RecheckReport {
            retained: 2,
            ..Default::default()
        }
    );
    assert!(Path::new(&dirty.path).is_dir() && Path::new(&unmerged.path).is_dir());
    assert_eq!(store.load().unwrap().len(), 2);
}

// (d) An unreachable remote is never taken as "merged": the tree stays, stays
// listed, and the failure is reported for a warning.
#[tokio::test]
async fn failed_fetch_keeps_the_tree_and_reports_it() {
    let f = Fixture::new();
    let t = f.tracked("offline").await;
    let tree = f.prepare(&t.repo, "worker").await;
    f.publish(&tree, "work.txt").await;
    f.merge_on_origin(&t, &tree.branch).await;
    fs::rename(&t.origin, f.root.join("offline-moved.git")).unwrap();
    let error = f.lifecycle.cleanup(&tree, "delete").await.unwrap_err();
    assert!(error.starts_with(UPSTREAM_FETCH_FAILED), "{error}");
    assert!(Path::new(&tree.path).is_dir());
    let store = f.store();
    store.remember(&tree).unwrap();
    assert_eq!(
        f.recheck(&store).await,
        RecheckReport {
            fetch_failed: 1,
            ..Default::default()
        }
    );
    assert!(Path::new(&tree.path).is_dir());
    assert_eq!(store.load().unwrap().len(), 1);
}

// (e) No upstream: only the local HEAD decides, exactly as before, and no
// fetch is attempted (this remote would fail one).
#[tokio::test]
async fn without_upstream_only_the_local_head_decides() {
    let f = Fixture::new();
    let repo = f.repo("local").await;
    let missing = f.root.join("missing-origin.git");
    f.git(
        &repo,
        &["remote", "add", "origin", missing.to_str().unwrap()],
    )
    .await;
    let tree = f.prepare(&repo, "worker").await;
    f.commit(&tree, "work.txt").await;
    assert_eq!(
        f.lifecycle.cleanup(&tree, "delete").await.unwrap_err(),
        not_merged(&tree)
    );
    f.git(&repo, &["merge", "--ff-only", &tree.branch]).await;
    f.lifecycle.cleanup(&tree, "delete").await.unwrap();
    assert!(!Path::new(&tree.path).exists());
}

// (f) A live session inside the tree (here: in a subdirectory) blocks removal
// at recheck; the record waits for a later start.
#[tokio::test]
async fn a_tree_a_live_session_uses_is_not_removed_by_the_recheck() {
    let f = Fixture::new();
    let repo = f.repo("busy").await;
    let tree = f.prepare(&repo, "worker").await;
    let store = f.store();
    store.remember(&tree).unwrap();
    let cwd = Path::new(&tree.path).join("sub");
    fs::create_dir(&cwd).unwrap();
    let in_use = move |path: &Path| path_within(&cwd, path);
    assert_eq!(
        f.lifecycle
            .recheck_retained(&store, &in_use, &Cancellation::default())
            .await,
        RecheckReport {
            in_use: 1,
            ..Default::default()
        }
    );
    assert!(Path::new(&tree.path).is_dir());
    assert_eq!(store.load().unwrap().len(), 1);
    assert_eq!(f.recheck(&store).await.removed, 1);
    assert!(!Path::new(&tree.path).exists());
}

#[tokio::test]
async fn one_fetch_per_parent_serves_every_record_of_the_recheck() {
    let f = Fixture::new();
    let t = f.tracked("shared").await;
    let first = f.prepare(&t.repo, "a-first").await;
    let second = f.prepare(&t.repo, "b-second").await;
    f.publish(&first, "first.txt").await;
    f.publish(&second, "second.txt").await;
    f.merge_on_origin(&t, &first.branch).await;
    f.merge_on_origin(&t, &second.branch).await;
    let store = f.store();
    store.remember(&first).unwrap();
    store.remember(&second).unwrap();
    // Once the first record is removed, the remote vanishes: a second fetch
    // for the same parent would fail and keep the second tree.
    let (origin, moved) = (t.origin.clone(), f.root.join("shared-moved.git"));
    let first_path = PathBuf::from(&first.path);
    let in_use = move |_: &Path| {
        if !first_path.exists() && origin.exists() {
            fs::rename(&origin, &moved).unwrap();
        }
        false
    };
    assert_eq!(
        f.lifecycle
            .recheck_retained(&store, &in_use, &Cancellation::default())
            .await,
        RecheckReport {
            removed: 2,
            forgotten: 2,
            ..Default::default()
        }
    );
    assert!(!t.origin.exists());
}

#[tokio::test]
async fn a_tree_moved_off_its_branch_is_forgotten_not_removed() {
    let f = Fixture::new();
    let repo = f.repo("detached").await;
    let tree = f.prepare(&repo, "worker").await;
    let path = Path::new(&tree.path);
    f.git(path, &["checkout", "-q", "--detach"]).await;
    f.commit(&tree, "only-on-detached-head.txt").await;
    let error = f.lifecycle.cleanup(&tree, "delete").await.unwrap_err();
    assert!(error.starts_with(NOT_ON_BRANCH), "{error}");
    let store = f.store();
    store.remember(&tree).unwrap();
    assert_eq!(
        f.recheck(&store).await,
        RecheckReport {
            forgotten: 1,
            ..Default::default()
        }
    );
    assert!(path.join("only-on-detached-head.txt").is_file());
    assert!(store.load().unwrap().is_empty());
}

#[tokio::test]
async fn unreadable_list_is_left_alone_and_removes_nothing() {
    let f = Fixture::new();
    let repo = f.repo("corrupt").await;
    let tree = f.prepare(&repo, "worker").await;
    let store = f.store();
    let file = f
        .root
        .join("runtime")
        .join("normal-worktrees-retained.json");
    fs::write(&file, "{ not json").unwrap();
    assert!(store.remember(&tree).is_err());
    assert_eq!(fs::read_to_string(&file).unwrap(), "{ not json");
    assert_eq!(
        f.recheck(&store).await,
        RecheckReport {
            store_failed: true,
            ..Default::default()
        }
    );
    assert!(Path::new(&tree.path).is_dir());
    // Records outside the ordinary layout are never listed.
    fs::remove_file(&file).unwrap();
    let mut foreign = tree.clone();
    foreign.branch = "feature/not-ours".into();
    store.remember(&foreign).unwrap();
    assert!(store.load().unwrap().is_empty());
}

#[test]
fn path_within_follows_spelling_and_component_boundaries() {
    let root = Path::new("/synthetic/repo/.git-worktrees/worker");
    assert!(path_within(
        Path::new("/synthetic/repo/.git-worktrees/worker/sub/"),
        root
    ));
    assert!(path_within(
        Path::new("\\synthetic\\repo\\.git-worktrees\\worker"),
        root
    ));
    assert!(!path_within(
        Path::new("/synthetic/repo/.git-worktrees/worker-2"),
        root
    ));
    assert!(!path_within(Path::new("/synthetic/repo"), root));
    #[cfg(windows)]
    assert!(path_within(
        Path::new("C:\\Synthetic\\Repo\\.git-worktrees\\Worker"),
        Path::new("c:/synthetic/repo/.git-worktrees/worker")
    ));
}
