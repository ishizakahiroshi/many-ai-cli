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
