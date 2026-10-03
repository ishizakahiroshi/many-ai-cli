use super::*;
use std::fs;
async fn repo() -> (tempfile::TempDir, FilesService, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("synthetic repo 日本語");
    fs::create_dir(&root).unwrap();
    let app = dir.path().join("runtime");
    fs::create_dir(&app).unwrap();
    let paths =
        crate::config::RuntimePaths::trial(&app, 49222, &dir.path().join("installed")).unwrap();
    let mut service = FilesService::new(root.clone(), paths);
    let empty = dir.path().join("empty-git-config");
    fs::write(&empty, b"").unwrap();
    service.git_environment = BTreeMap::from([
        ("GIT_CONFIG_GLOBAL".into(), Some(empty.into_os_string())),
        ("GIT_CONFIG_NOSYSTEM".into(), Some("1".into())),
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
    ]);
    Git::new(&service, 5)
        .run(&root, &["init", "-b", "synthetic"])
        .await
        .unwrap();
    (dir, service, root)
}
#[test]
fn revision_and_process_error_guards() {
    for bad in ["", "--help", "HEAD^{tree}", "a b", "a\nb", "$(id)"] {
        assert!(!valid_revision(bad));
    }
    for good in ["HEAD", "refs/heads/develop", "abc012", "feature/new-work"] {
        assert!(valid_revision(good));
    }
    assert_eq!(
        classify_error("push", "rejected (non-fast-forward)"),
        (409, "rejected_non_fast_forward")
    );
    assert_eq!(
        classify_error("pull", "Please commit your changes or stash them"),
        (409, "local_changes")
    );
    assert_eq!(
        classify_error("push", "Permission denied (publickey)"),
        (502, "auth_failed")
    );
    assert_eq!(
        classify_error("commit", "Author identity unknown"),
        (400, "commit_identity_missing")
    );
    let message = sanitize_error(
        "git fetch: https://name:synthetic@example.invalid/private /home/synthetic/file C:\\fake\\file", // secrets-scan: allow /home/synthetic/ -- synthetic home-path redaction fixture
    );
    assert!(!message.contains("synthetic@"));
    assert!(!message.contains("/home/synthetic"));
}
#[test]
fn porcelain_z_preserves_spaces_unicode_and_rename_source_consumption() {
    let files = parse_status("R  new file 日本語.md\0old.md\0?? a\tb.txt\0 M other.rs\0");
    assert_eq!(files.len(), 3);
    assert_eq!(files[0].path, "new file 日本語.md");
    assert_eq!(files[0].status, "R");
    assert_eq!(files[1].path, "a\tb.txt");
    assert_eq!(files[2].status, "M");
    assert_eq!(rename_path("src/{old => new}/file.rs"), "src/new/file.rs");
    assert_eq!(rename_path("src/{old => }/file.rs"), "src/file.rs");
    assert_eq!(
        parse_numstat("-\t-\tbinary.png\n2\t3\told => new")["new"],
        (2, 3)
    );
}
#[test]
fn decorations_and_github_urls_preserve_shapes() {
    assert!(parse_decorate("").is_none());
    let refs =
        parse_decorate("HEAD -> refs/heads/main, tag: refs/tags/v1, refs/remotes/origin/main")
            .unwrap();
    assert_eq!(refs.len(), 3);
    assert_eq!(refs[0].kind, "local");
    assert_eq!(refs[1].name, "v1");
    assert_eq!(
        github_url("git@github.com:owner/repo.git"), // secrets-scan: allow git@github.com -- public SSH URL fixture
        Some("https://github.com/owner/repo".into())
    );
    assert_eq!(
        github_url("https://synthetic:git@github.com/owner/repo.git"), // secrets-scan: allow git@github.com -- synthetic URL userinfo is stripped
        Some("https://github.com/owner/repo".into())
    );
    assert!(github_url("https://not-github.invalid/owner/repo").is_none());
}
#[tokio::test]
async fn empty_repository_log_is_successful_empty_array() {
    let (_dir, service, root) = repo().await;
    let git = Git::new(&service, 5);
    let r = Request {
        query: "limit=2000&skip=-1".into(),
        ..Default::default()
    };
    let value = git.log(&r, &root, &root).await.unwrap();
    assert_eq!(value["commits"], json!([]));
    assert_eq!(value["limit"], 1000);
    assert_eq!(value["skip"], 0);
    assert_eq!(value["has_more"], false);
}
#[tokio::test]
async fn actual_synthetic_commit_status_show_log_and_refs() {
    let (_dir, service, root) = repo().await;
    fs::write(root.join("hello 日本語.md"), "one\ntwo\n").unwrap();
    let git = Git::new(&service, 20);
    let before = git.status(&root, &root).await.unwrap();
    assert_eq!(before["files"][0]["added"], Value::Null);
    let diff = git.diff(&root, &root, None).await.unwrap();
    assert_eq!(diff["files"][0]["added"], 2);
    let commit = git
        .commit(
            &root,
            &root,
            &GitRequest {
                subject: "docs: synthetic fixture".into(),
                body: "Only owned temporary data.".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(commit["files_changed"], 1);
    let hash = commit["hash"].as_str().unwrap();
    assert!(!hash.is_empty());
    let show = git.show(hash, &root, &root).await.unwrap();
    assert_eq!(show["subject"], "docs: synthetic fixture");
    assert_eq!(show["body"], "Only owned temporary data.");
    assert_eq!(show["files"][0]["path"], "hello 日本語.md");
    assert_eq!(show["files"][0]["added"], 2);
    let log = git.log(&Request::default(), &root, &root).await.unwrap();
    assert_eq!(log["commits"].as_array().unwrap().len(), 1);
    assert_eq!(log["commits"][0]["parents"], json!([]));
    assert_eq!(git.refs(&root).await.unwrap()["head"], "synthetic");
    assert_eq!(
        git.status(&root, &root).await.unwrap()["has_changes"],
        false
    );
    let no = git
        .commit(
            &root,
            &root,
            &GitRequest {
                subject: "empty".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(no.status, 400);
}
#[tokio::test]
async fn git_diff_from_subdirectory_uses_repo_root_pathspec() {
    let (_dir, service, root) = repo().await;
    fs::create_dir(root.join("sub")).unwrap();
    fs::write(root.join("top.md"), "old\n").unwrap();
    let git = Git::new(&service, 20);
    git.commit(
        &root,
        &root,
        &GitRequest {
            subject: "initial".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    fs::write(root.join("top.md"), "new\n").unwrap();
    let diff = git.diff(&root, &root.join("sub"), None).await.unwrap();
    assert!(diff["files"][0]["diff"].as_str().unwrap().contains("+new"));
}
#[tokio::test]
async fn turn_snapshot_uses_private_temporary_index_and_preserves_real_index() {
    let (_dir, service, root) = repo().await;
    fs::write(root.join("a.md"), "one\n").unwrap();
    let git = Git::new(&service, 20);
    git.commit(
        &root,
        &root,
        &GitRequest {
            subject: "initial".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let before = fs::read(root.join(".git/index")).unwrap();
    fs::write(root.join("a.md"), "two\n").unwrap();
    let tree = git.worktree_tree(&root).await.unwrap();
    assert!(valid_revision(&tree));
    assert_eq!(fs::read(root.join(".git/index")).unwrap(), before);
    let tmp = service.paths.root().join("tmp");
    assert_eq!(fs::read_dir(tmp).unwrap().count(), 0);
    let current = git
        .run(&root, &["diff", "--cached", "--name-only"])
        .await
        .unwrap();
    assert!(current.is_empty());
}
#[cfg(unix)]
#[tokio::test]
async fn untracked_symlink_diff_exposes_link_text_never_target_bytes() {
    use std::os::unix::fs::symlink;
    let (dir, service, root) = repo().await;
    let external = dir.path().join("outside-fixture");
    fs::write(&external, b"never appear in diff").unwrap();
    symlink(&external, root.join("link.md")).unwrap();
    let diff = Git::new(&service, 5)
        .diff(&root, &root, None)
        .await
        .unwrap();
    let text = diff["files"][0]["diff"].as_str().unwrap();
    assert!(text.contains("new file mode 120000"));
    assert!(!text.contains("never appear"));
}
#[cfg(unix)]
#[tokio::test]
async fn fake_network_command_receives_fixed_args_and_noninteractive_environment() {
    use std::os::unix::fs::PermissionsExt;
    let (_dir, mut service, root) = repo().await;
    let fake = root.join("fake-git");
    fs::write(&fake,"#!/bin/sh\nprintf '%s\\n' \"$GIT_TERMINAL_PROMPT\" \"$GIT_ASKPASS\"\nprintf '<%s>\\n' \"$@\"\n").unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o700)).unwrap();
    service.git_executable = fake;
    let env = BTreeMap::from([
        ("GIT_TERMINAL_PROMPT".into(), Some("0".into())),
        ("GIT_ASKPASS".into(), Some("echo".into())),
    ]);
    let out = Git::new(&service, 5)
        .env(&root, &["push"], env, true)
        .await
        .unwrap();
    assert!(out.starts_with("0\necho\n"));
    assert!(out.ends_with("<push>\n"));
    assert!(!out.contains("--force"));
    assert!(out.contains(&format!("<{}>", root.display())));
}
#[test]
fn deterministic_suggestion_uses_symbols_scopes_weights_and_languages() {
    let files = vec![StatusFile {
        status: "M".into(),
        path: "internal/hub/example.go".into(),
        added: None,
        removed: None,
    }];
    let diff = "+++ b/internal/hub/example.go\n+func NewEndpoint() {\n+ mux.HandleFunc(\"/api/new\", handler)\n";
    let (subject, body) = suggest::suggest(&files, "", "", "", "en", &BTreeMap::new());
    assert_eq!(subject, "refactor(hub): rework example.go");
    assert_eq!(body, "1 file(s): 0 added / 1 modified / 0 deleted.");
    let (subject, body) = suggest::suggest(&files, "", "", "", "ja", &BTreeMap::new());
    assert_eq!(subject, "refactor(hub): example.go を整理");
    assert!(body.starts_with("ファイル 1 件"));
    let (subject, body) = suggest::suggest(&files, "", diff, "", "en", &BTreeMap::new());
    assert_eq!(subject, "feat(hub): add /api/new");
    assert!(body.contains("- API added: /api/new"));
}
#[test]
fn marker_scanner_tui_reflow_echo_latest_block_and_prompt_bytes() {
    use super::turns::{ai_prompt, extract_markers};
    assert!(extract_markers(&ai_prompt(false)).is_none());
    assert!(extract_markers(&ai_prompt(true)).is_none());
    let pair =
        extract_markers("●[MANY-AI-CLI-COMMIT]feat: synthetic\n\n│ body line[/MANY-AI-CLI-COMMIT]")
            .unwrap();
    assert_eq!(pair, ("feat: synthetic".into(), "body line".into()));
    assert_eq!(extract_markers("[MANY-AI-CLI-COMMIT]one[/MANY-AI-CLI-COMMIT]\n[MANY-AI-CLI-COMMIT]two[/MANY-AI-CLI-COMMIT]").unwrap().0,"two");
}
#[test]
fn per_route_git_bodies_ignore_fields_from_other_endpoints() {
    let body = br#"{"SESSION":3,"subject":false,"token":"synthetic","unknown":1e999}"#;
    assert_eq!(
        crate::proto::decode_http_json::<GitActionBody>(body)
            .unwrap()
            .0
            .session,
        3
    );
    assert!(crate::proto::decode_http_json::<GitCommitBody>(body).is_err());
}

#[cfg(unix)]
#[test]
fn git_turn_timestamps_preserve_source_local_offset() {
    const CHILD: &str = "MANY_AI_TEST_GIT_TURN_TIMEZONE";
    if std::env::var(CHILD).as_deref() == Ok("1") {
        let at = std::time::UNIX_EPOCH + std::time::Duration::new(1_700_000_000, 123_456_789);
        assert_eq!(
            super::turns::format_turn_timestamp(at).unwrap(),
            "2023-11-15T07:13:20+09:00"
        );
        return;
    }
    // Change TZ only in an owned test subprocess; concurrent tests never share
    // mutated process-global timezone state. Asia/Tokyo has no modern DST delta.
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "files::git::tests::git_turn_timestamps_preserve_source_local_offset",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env("TZ", "Asia/Tokyo")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("1 passed"));
}
