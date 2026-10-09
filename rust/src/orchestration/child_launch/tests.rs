use super::*;
use crate::{
    proto::{self, time::UNIX_EPOCH},
    terminal::{
        events::CoreEventBus,
        journal::{JournalOptions, SessionJournal},
        session::EngineOptions,
    },
};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    process::Command,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use tempfile::TempDir;

fn now() -> Timestamp {
    UNIX_EPOCH + Duration::from_secs(1767323045)
}
fn roots() -> (TempDir, RuntimePaths) {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("runtime")).unwrap();
    let paths = RuntimePaths::trial(
        &temp.path().join("runtime"),
        49231,
        &temp.path().join("installed"),
    )
    .unwrap();
    (temp, paths)
}
fn git_env(temp: &Path) -> BTreeMap<OsString, Option<OsString>> {
    let empty = temp.join("empty-git-config");
    fs::write(&empty, "").unwrap();
    BTreeMap::from([
        ("GIT_CONFIG_NOSYSTEM".into(), Some("1".into())),
        ("GIT_CONFIG_COUNT".into(), Some("0".into())),
        ("GIT_CONFIG_GLOBAL".into(), Some(empty.into_os_string())),
        ("GIT_DIR".into(), None),
        ("GIT_WORK_TREE".into(), None),
        ("GIT_INDEX_FILE".into(), None),
        ("GIT_COMMON_DIR".into(), None),
    ])
}
fn git(temp: &Path) -> WorktreeGit {
    WorktreeGit::new("git".into(), git_env(temp))
}
fn command(repo: &Path, env: &BTreeMap<OsString, Option<OsString>>, args: &[&str]) -> String {
    let mut command = Command::new("git");
    command.arg("-C").arg(repo).args(args);
    for (key, value) in env {
        if let Some(value) = value {
            command.env(key, value);
        } else {
            command.env_remove(key);
        }
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "synthetic git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}
fn repo(root: &Path, name: &str, env: &BTreeMap<OsString, Option<OsString>>) -> PathBuf {
    let repo = root.join(name);
    fs::create_dir_all(&repo).unwrap();
    command(&repo, env, &["init", "-q"]);
    command(&repo, env, &["config", "user.name", "Synthetic Fixture"]);
    command(
        &repo,
        env,
        &["config", "user.email", "fixture@example.invalid"],
    );
    command(&repo, env, &["config", "commit.gpgsign", "false"]);
    fs::write(repo.join("README.md"), "synthetic child launch fixture\n").unwrap();
    command(&repo, env, &["add", "README.md"]);
    command(&repo, env, &["commit", "-qm", "synthetic base"]);
    repo
}
fn parent(cwd: &Path) -> SessionSnapshot {
    SessionSnapshot {
        id: LiveSessionId(7),
        provider: "codex".into(),
        model: "synthetic".into(),
        cwd: cwd.to_string_lossy().into_owned(),
        ..Default::default()
    }
}
fn prep(paths: &RuntimePaths, root: &Path) -> ChildPreparer {
    ChildPreparer::new(paths, root.to_owned(), Some(root.join("home")), git(root)).unwrap()
}
fn worktree_config() -> OrchestrationConfig {
    OrchestrationConfig {
        worktree_auto: Some(true),
        worktree_dir_root: ".many-ai-cli/worktrees".into(),
        ..Default::default()
    }
}

#[test]
fn prompts_tokens_and_control_bytes_match_extracted_fixed_go() {
    let fixture: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../tests/fixtures/core/orchestration/child-launch/prompt-go-21d0bc7.json"
    ))
    .unwrap();
    for case in fixture {
        let input = &case["Input"];
        let get = |key: &str| input[key].as_str().unwrap();
        let actual = child_initial_prompt(
            get("Base"),
            Path::new(get("Board")),
            get("Role"),
            get("Branch"),
            get("ID"),
        );
        let expected = case["Prompt"].as_str().unwrap();
        // The fixture is Linux Go; only filepath.Join's progress-file separator
        // is platform-specific. The board text itself is passed through unchanged.
        let expected = if cfg!(windows) {
            expected.replace(
                &format!(
                    "Your progress file (write here): {}/child-{}.md",
                    Path::new(get("Board")).parent().unwrap().display(),
                    get("ID")
                ),
                &format!(
                    "Your progress file (write here): {}",
                    prompt::child_progress_path(Path::new(get("Board")), get("ID")).display()
                ),
            )
        } else {
            expected.to_owned()
        };
        assert_eq!(actual, expected, "{}", get("Name"));
        assert_eq!(prompt::sanitize_inject_text(get("Base")), case["Sanitized"]);
        assert_eq!(safe_token(get("Token")), case["Safe"]);
        assert_eq!(prompt::normalize_role(get("Role")).unwrap(), case["Role"]);
    }
    assert!(prompt::normalize_role(" \n ").is_err());
    assert_eq!(prompt::normalize_role("... 💡").unwrap(), "item");
}
#[test]
fn exactly_one_initial_prompt_route_and_no_fabricated_session_zero() {
    for mode in ["", "interactive", "headless"] {
        for via_arg in [false, true] {
            let delivery =
                child_launch_prompt(mode, via_arg, "review", Path::new("board.md"), "review", "");
            match delivery {
                PromptDelivery::AfterRegistration => assert!(mode != "headless" && !via_arg),
                PromptDelivery::AtLaunch(prompt) => {
                    assert!(mode == "headless" || via_arg);
                    assert!(!prompt.contains("child-0.md"));
                    assert_eq!(
                        crate::orchestration::headless::expand_prompt(&prompt, 42),
                        child_initial_prompt("review", Path::new("board.md"), "review", "", "42")
                    );
                }
            }
        }
    }
}
#[test]
fn cwd_resolution_uses_parent_then_hub_and_preserves_source_lexical_paths() {
    let (root, paths) = roots();
    let parent_dir = root.path().join("project");
    fs::create_dir_all(parent_dir.join("child")).unwrap();
    fs::create_dir_all(root.path().join("child")).unwrap();
    let preparer = prep(&paths, root.path());
    let mut parent = parent(&parent_dir);
    assert_eq!(
        preparer.resolve_cwd(&parent, " child/../child ").unwrap(),
        parent_dir.join("child")
    );
    assert_eq!(preparer.resolve_cwd(&parent, "").unwrap(), parent_dir);
    parent.cwd.clear();
    assert_eq!(
        preparer.resolve_cwd(&parent, "child").unwrap(),
        root.path().join("child")
    );
    assert!(matches!(
        preparer.resolve_cwd(&parent, "missing"),
        Err(ChildLaunchError { status: 400, .. })
    ));
    for path in ["/", r"c:\windows"] {
        assert!(cwd_too_broad(Path::new(path), None), "{path}");
    }
    // Fixed Go cleans these spellings to backslashes on Windows before its
    // Unix-only literal lookup. Assert the native result on each OS.
    for path in ["/home", "/Users", "/etc"] {
        assert_eq!(
            cwd_too_broad(Path::new(path), None),
            !cfg!(windows),
            "{path}"
        );
    }
    #[cfg(windows)]
    for path in [
        r"C:\",
        r"c:\program files",
        r"C:\Program Files (x86)",
        r"C:\Users",
    ] {
        assert!(cwd_too_broad(Path::new(path), None), "{path}");
    }
    assert!(cwd_too_broad(
        Path::new("/synthetic/home"),
        Some(Path::new("/synthetic/home"))
    ));
    assert!(!cwd_too_broad(
        Path::new("/synthetic/home/project"),
        Some(Path::new("/synthetic/home"))
    ));
}
#[test]
fn boards_create_once_append_serially_and_preserve_ordinary_hardlink_identity() {
    let (root, paths) = roots();
    let boards = Arc::new(BoardStore::new(&paths));
    let parent = parent(root.path());
    let path = boards
        .ensure(".. Board / β ..", &parent, "  purpose\n ", now())
        .unwrap();
    let initial = fs::read_to_string(&path).unwrap();
    assert!(initial.contains("- purpose: purpose\n"));
    boards
        .ensure(".. Board / β ..", &parent, "should not replace", now())
        .unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), initial);
    let alias = root.path().join("ordinary-board-copy.md");
    fs::hard_link(&path, &alias).unwrap();
    let handles: Vec<_> = (0..16)
        .map(|i| {
            let boards = boards.clone();
            let path = path.clone();
            std::thread::spawn(move || {
                boards
                    .append(&path, "hub", &format!("item={i}\n"), now())
                    .unwrap()
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }
    let text = fs::read_to_string(&path).unwrap();
    assert_eq!(text, fs::read_to_string(alias).unwrap());
    assert_eq!(text.matches("\n## hub ").count(), 16);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
}
#[tokio::test]
async fn actual_git_create_reuse_and_changed_branch_rejection_leave_evidence() {
    let (root, paths) = roots();
    let env = git_env(root.path());
    let repo = repo(root.path(), "repo", &env);
    let preparer = prep(&paths, root.path());
    let mut parent = parent(&repo);
    parent.orchestration_id = OrchestrationId("same-board".into());
    let body = ChildSpawnRequest {
        role: "review".into(),
        ..Default::default()
    };
    let cfg = worktree_config();
    let cancel = TaskCancellation::default();
    let first = preparer
        .prepare(&parent, &body, &cfg, now(), &cancel, |_, board| {
            assert!(board.exists());
            std::future::ready(Ok(()))
        })
        .await
        .unwrap();
    assert_eq!(first.absolute_cwd, repo);
    assert_eq!(
        first.child_cwd,
        repo.join(".many-ai-cli/worktrees/same-board/review")
    );
    assert_eq!(first.branch, "orch/same-board/review");
    let reused = preparer
        .prepare(&parent, &body, &cfg, now(), &cancel, |_, _| {
            std::future::ready(Ok(()))
        })
        .await
        .unwrap();
    assert_eq!(first, reused);
    command(
        &first.child_cwd,
        &env,
        &["switch", "-c", "synthetic-manual-branch"],
    );
    let before = fs::read(first.child_cwd.join("README.md")).unwrap();
    let marked = AtomicBool::new(false);
    let error = preparer
        .prepare(&parent, &body, &cfg, now(), &cancel, |_, _| {
            marked.store(true, Ordering::SeqCst);
            std::future::ready(Ok(()))
        })
        .await
        .unwrap_err();
    assert!(
        matches!(error, SessionError::ChildLaunch { status: 409, code, .. } if code == "worktree_identity_mismatch")
    );
    assert!(marked.load(Ordering::SeqCst));
    assert_eq!(
        command(&first.child_cwd, &env, &["branch", "--show-current"]).trim(),
        "synthetic-manual-branch"
    );
    assert_eq!(before, fs::read(first.child_cwd.join("README.md")).unwrap());
    let text = fs::read_to_string(first.board_path).unwrap();
    assert!(text.contains("worktree created:"));
    assert!(text.contains("worktree reuse:"));
    assert!(text.contains("worktree reuse rejected:"));
}
#[tokio::test]
async fn same_tree_tristate_and_non_git_fallback_use_requested_cwd() {
    let (root, paths) = roots();
    let env = git_env(root.path());
    let repo_a = repo(root.path(), "repo-a", &env);
    let repo_b = repo(root.path(), "repo-b", &env);
    let preparer = prep(&paths, root.path());
    let parent = parent(&repo_a);
    let cfg = worktree_config();
    let cancel = TaskCancellation::default();
    for same_tree in [None, Some(false), Some(true)] {
        let body = ChildSpawnRequest {
            role: format!("role-{same_tree:?}"),
            cwd: repo_b.to_string_lossy().into_owned(),
            same_tree,
            ..Default::default()
        };
        let out = preparer
            .prepare(&parent, &body, &cfg, now(), &cancel, |_, _| {
                std::future::ready(Ok(()))
            })
            .await
            .unwrap();
        assert_eq!(out.absolute_cwd, repo_b);
        if same_tree == Some(true) {
            assert_eq!(out.child_cwd, repo_b);
            assert!(out.branch.is_empty());
        } else {
            assert!(out.child_cwd.starts_with(&repo_b));
            assert_ne!(out.child_cwd, repo_b);
        }
    }
    let non_git = root.path().join("plain");
    fs::create_dir(&non_git).unwrap();
    let out = preparer
        .git
        .prepare(&non_git, "id", "review", &cfg, &cancel)
        .await
        .unwrap();
    assert_eq!(out.cwd, non_git);
    assert_eq!(
        out.note,
        "worktree skip: parent cwd is not a git repository"
    );
    let mut disabled = cfg;
    disabled.worktree_auto = Some(false);
    let out = preparer
        .git
        .prepare(&repo_a, "id", "review", &disabled, &cancel)
        .await
        .unwrap();
    assert_eq!(out.cwd, repo_a);
    assert_eq!(out.note, "worktree skip: disabled by config");
}
#[tokio::test]
async fn missing_existing_worktree_identity_is_rejected_and_cancel_is_not_a_skip() {
    let (root, _) = roots();
    let env = git_env(root.path());
    let repo = repo(root.path(), "repo", &env);
    let git = git(root.path());
    let cfg = worktree_config();
    let child = repo.join(".many-ai-cli/worktrees/id/review");
    fs::create_dir_all(&child).unwrap();
    assert!(
        matches!(git.prepare(&repo, "id", "review", &cfg, &TaskCancellation::default()).await, Err(WorktreeError::IdentityMismatch(note)) if note.contains("not registered"))
    );
    let cancel = TaskCancellation::default();
    cancel.cancel();
    assert_eq!(
        git.prepare(&repo, "cancel", "review", &cfg, &cancel).await,
        Err(WorktreeError::Cancelled)
    );
}

#[derive(Default)]
struct TestServices {
    children: Mutex<Vec<RegisteredChild>>,
    notices: Mutex<Vec<String>>,
    warnings: AtomicUsize,
    effects: AtomicUsize,
    role_lookups: AtomicUsize,
}
impl ChildLaunchServices for TestServices {
    fn launch_arg_usable(&self, _: &str) -> Result<bool, SessionError> {
        Ok(true)
    }
    fn role_subscription(
        &self,
        _: &OrchestrationId,
        _: &str,
    ) -> Result<Option<String>, SessionError> {
        self.role_lookups.fetch_add(1, Ordering::SeqCst);
        Ok(None)
    }
    fn trust_grant_providers(&self) -> Result<Vec<String>, SessionError> {
        Ok(vec!["claude".into(), "codex".into()])
    }
    fn apply_effects<'a>(&'a self, _: CoreEffects) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async {
            self.effects.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }
    fn register_conductor(
        &self,
        _: SessionBinding,
        _: &OrchestrationId,
        board: &Path,
    ) -> Result<(), SessionError> {
        assert!(board.exists());
        Ok(())
    }
    fn child_registered<'a>(
        &'a self,
        child: RegisteredChild,
        _: TaskCancellation,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            self.children.lock().unwrap().push(child);
            Ok(())
        })
    }
    fn notify_board_event<'a>(
        &'a self,
        _: SessionBinding,
        _: &'a OrchestrationId,
        text: String,
        _: TaskCancellation,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            self.notices.lock().unwrap().push(text);
            Ok(())
        })
    }
    fn notify_orchestration_error<'a>(
        &'a self,
        _: SessionBinding,
        limit: &'a str,
        detail: &'a str,
        _: TaskCancellation,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            self.notices
                .lock()
                .unwrap()
                .push(format!("{limit}: {detail}"));
            Ok(())
        })
    }
    fn confirmation_changed(
        &self,
        _: SessionBinding,
        _: &str,
        provider_note: &str,
        option_note: &str,
    ) {
        self.notices
            .lock()
            .unwrap()
            .push(format!("{provider_note}; {option_note}"));
    }
    fn warning(&self, _: &'static str, _: &SessionError) {
        self.warnings.fetch_add(1, Ordering::SeqCst);
    }
}
struct NoTransport;
impl WrapperTransport for NoTransport {
    fn send<'a>(
        &'a self,
        _: SessionBinding,
        _: proto::Message,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async { Err(SessionError::Transport("fixture has no PTY".into())) })
    }
}
struct TestSink;
impl CoreEffectSink for TestSink {
    fn apply<'a>(&'a self, _: CoreEffects) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
        Box::pin(async { Ok(()) })
    }
}
#[derive(Default)]
struct RegisteredSpawner {
    engine: Mutex<Weak<SessionEngine>>,
    fail: AtomicBool,
    starts: AtomicUsize,
    seen: Mutex<Vec<WrappedSpawnSpec>>,
}
impl WrappedSessionSpawner for RegisteredSpawner {
    fn spawn_and_wait<'a>(
        &'a self,
        spec: WrappedSpawnSpec,
        _: Duration,
        _: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome> {
        Box::pin(async move {
            let n = self.starts.fetch_add(1, Ordering::SeqCst);
            self.seen.lock().unwrap().push(spec.clone());
            if self.fail.load(Ordering::SeqCst) {
                return SpawnWaitOutcome::Failed("synthetic start failure".into());
            }
            let engine = self.engine.lock().unwrap().upgrade().unwrap();
            let registration = engine
                .register(
                    RegisterRequest {
                        message: proto::Message {
                            provider: spec.provider,
                            model: spec.model,
                            cwd: spec.cwd.to_string_lossy().into_owned(),
                            label: spec.label,
                            pid: 100 + n as i64,
                            cols: 80,
                            rows: 24,
                            ..Default::default()
                        },
                        spawn_proof: spec
                            .registration_proof
                            .map(|proof| proof.as_header_value().to_owned()),
                    },
                    WrapperConnectionId(100 + n as u64),
                    now(),
                )
                .await
                .unwrap();
            SpawnWaitOutcome::Registered(registration.binding)
        })
    }
}
struct ExecutorFixture {
    _root: TempDir,
    paths: RuntimePaths,
    executor: ChildLaunchExecutor,
    engine: Arc<SessionEngine>,
    config: Arc<ConfigStore>,
    services: Arc<TestServices>,
    spawner: Arc<RegisteredSpawner>,
    parent: SessionBinding,
}
async fn executor_fixture() -> ExecutorFixture {
    let (root, paths) = roots();
    let project = root.path().join("project");
    fs::create_dir(&project).unwrap();
    let config = Arc::new(ConfigStore::new(paths.clone(), Config::default()).unwrap());
    let spawner = Arc::new(RegisteredSpawner::default());
    let services = Arc::new(TestServices::default());
    let journal = Arc::new(SessionJournal::new(
        paths.clone(),
        None,
        JournalOptions {
            session_enabled: false,
            max_bytes: 0,
        },
    ));
    let engine = Arc::new(SessionEngine::new(
        EngineOptions {
            warning: Arc::new(|_, _| {}),
            ..Default::default()
        },
        journal,
        Arc::new(NoTransport),
        Arc::new(TestSink),
        spawner.clone(),
        CoreEventBus::new(16).unwrap(),
    ));
    *spawner.engine.lock().unwrap() = Arc::downgrade(&engine);
    let parent = engine
        .register(
            RegisterRequest {
                message: proto::Message {
                    provider: "copilot".into(),
                    cwd: project.to_string_lossy().into_owned(),
                    pid: 1,
                    cols: 80,
                    rows: 24,
                    ..Default::default()
                },
                spawn_proof: None,
            },
            WrapperConnectionId(1),
            now(),
        )
        .await
        .unwrap()
        .binding;
    let executor = ChildLaunchExecutor::new(
        Arc::downgrade(&engine),
        config.clone(),
        prep(&paths, root.path()),
        services.clone(),
    );
    ExecutorFixture {
        _root: root,
        paths,
        executor,
        engine,
        config,
        services,
        spawner,
        parent,
    }
}
fn launch_request(f: &ExecutorFixture, mut body: ChildSpawnRequest) -> ConfirmedChildRequest {
    body.role = if body.role.is_empty() {
        "review".into()
    } else {
        body.role
    };
    body.provider = if body.provider.is_empty() {
        "copilot".into()
    } else {
        body.provider
    };
    body.claimed_origin = "ui".into();
    body.same_tree = Some(true);
    let resolved = ResolvedChildSpawn::from_request(
        body,
        Some(VerifiedUiOrigin::after_server_verification(AuthEpoch(0))),
        InternalSpawnGrants::default(),
    )
    .unwrap();
    let admission = f
        .engine
        .reserve_children(
            AdmissionRequest {
                parent: f.parent.session,
                slots: 1,
                origin: resolved.origin().clone(),
                replace: None,
            },
            AdmissionLimits {
                max_children_per_parent: 8,
                max_total_sessions: 16,
            },
        )
        .unwrap()
        .id;
    ConfirmedChildRequest {
        original_body: resolved.clone(),
        parent: f.parent,
        requested_provider: String::new(),
        body: resolved,
        admission,
    }
}
#[tokio::test]
async fn real_core_registration_receives_metadata_then_restart_prompt_and_memory() {
    let f = executor_fixture().await;
    let request = launch_request(
        &f,
        ChildSpawnRequest {
            initial_prompt: "perform review".into(),
            subscription_profile_id: "explicit-synthetic-profile".into(),
            permission_preset: "attended".into(),
            remember_permission: Some(true),
            ..Default::default()
        },
    );
    let reservation = request.admission.clone();
    assert!(
        f.config
            .snapshot()
            .unwrap()
            .config
            .user_prefs
            .spawn
            .role_provider
            .is_empty()
    );
    let result = f
        .executor
        .spawn(request, TaskCancellation::default())
        .await
        .unwrap();
    assert!(
        !f.engine
            .admission_matches(&reservation, f.parent.session, 1)
    );
    let child = f.engine.snapshot(result.id).unwrap();
    assert_eq!(child.parent_session_id, f.parent.session);
    assert_eq!(child.role, "review");
    assert!(child.auto);
    assert_eq!(child.depth, 1);
    assert_eq!(child.board_path, result.board_path);
    assert_eq!(f.services.role_lookups.load(Ordering::SeqCst), 0);
    assert_eq!(
        f.spawner.seen.lock().unwrap()[0].subscription_profile_id,
        "explicit-synthetic-profile"
    );
    let children = f.services.children.lock().unwrap();
    assert_eq!(children.len(), 1);
    assert!(
        children[0]
            .inject_after_registration
            .as_ref()
            .unwrap()
            .contains(&format!("Session ID: {}", result.id.0))
    );
    assert!(children[0].restart_spec.initial_prompt.is_empty());
    assert!(
        !children[0]
            .restart_spec
            .registration_metadata
            .prompt_at_launch
    );
    let saved = f.config.snapshot().unwrap().config;
    assert_eq!(saved.user_prefs.spawn.role_provider["review"], "copilot");
    assert_eq!(saved.user_prefs.spawn.role_permission["review"], "attended");
    assert!(f.paths.resource(config::Resource::Config).exists());
}
#[tokio::test]
async fn failed_spawn_does_not_remember_and_duplicate_guard_runs_before_preparation() {
    let f = executor_fixture().await;
    f.spawner.fail.store(true, Ordering::SeqCst);
    let request = launch_request(&f, ChildSpawnRequest::default());
    let admission = request.admission.clone();
    assert!(
        matches!(f.executor.spawn(request, TaskCancellation::default()).await, Err(SessionError::ChildLaunch { code, .. }) if code == "spawn_error")
    );
    assert!(
        f.config
            .snapshot()
            .unwrap()
            .config
            .user_prefs
            .spawn
            .role_provider
            .is_empty()
    );
    assert!(!f.engine.admission_matches(&admission, f.parent.session, 1));
    f.spawner.fail.store(false, Ordering::SeqCst);
    f.executor
        .spawn(
            launch_request(&f, ChildSpawnRequest::default()),
            TaskCancellation::default(),
        )
        .await
        .unwrap();
    let board = f.engine.snapshot(f.parent.session).unwrap().board_path;
    let before = fs::read(&board).unwrap();
    let request = launch_request(&f, ChildSpawnRequest::default());
    let admission = request.admission.clone();
    assert!(
        matches!(f.executor.spawn(request, TaskCancellation::default()).await, Err(SessionError::ChildLaunch { status: 409, code, .. }) if code == "duplicate_role_child")
    );
    assert!(!f.engine.admission_matches(&admission, f.parent.session, 1));
    assert_eq!(before, fs::read(board).unwrap());
    assert_eq!(f.spawner.starts.load(Ordering::SeqCst), 2);
    f.executor
        .spawn(
            launch_request(
                &f,
                ChildSpawnRequest {
                    force: true,
                    ..Default::default()
                },
            ),
            TaskCancellation::default(),
        )
        .await
        .unwrap();
    assert_eq!(f.spawner.starts.load(Ordering::SeqCst), 3);
}
#[tokio::test]
async fn launch_argument_and_headless_prompts_are_not_injected() {
    let f = executor_fixture().await;
    for (role, provider, mode) in [
        ("interactive", "codex", "interactive"),
        ("headless", "claude", "headless"),
    ] {
        f.executor
            .spawn(
                launch_request(
                    &f,
                    ChildSpawnRequest {
                        role: role.into(),
                        provider: provider.into(),
                        execution_mode: mode.into(),
                        initial_prompt: "hello".into(),
                        ..Default::default()
                    },
                ),
                TaskCancellation::default(),
            )
            .await
            .unwrap();
    }
    let children = f.services.children.lock().unwrap();
    for child in children.iter() {
        assert!(child.inject_after_registration.is_none());
        assert!(child.restart_spec.initial_prompt.is_empty());
        assert!(
            child
                .restart_spec
                .registration_metadata
                .orchestration
                .0
                .is_empty()
        );
        assert!(child.restart_spec.label.is_empty());
        assert!(!child.restart_spec.grants.grant_folder_trust());
    }
    let launches = f.spawner.seen.lock().unwrap();
    for launch in launches.iter() {
        assert!(launch.registration_metadata.prompt_at_launch);
        assert!(
            launch
                .initial_prompt
                .contains(crate::orchestration::headless::SESSION_ID_PLACEHOLDER)
        );
    }
    assert!(children[0].prompt_via_launch_arg);
    assert!(!children[1].prompt_via_launch_arg);
}
#[tokio::test]
async fn refusal_writes_once_without_launch_and_gone_waiter_records_real_outcome() {
    let f = executor_fixture().await;
    let request = launch_request(&f, ChildSpawnRequest::default());
    let pending = PendingSpawnConfirmation {
        id: SpawnConfirmationId("synthetic-confirmation".into()),
        parent: f.parent.session,
        requested_provider: String::new(),
        body: request.body,
        requested_at: now(),
        admission: request.admission,
        waiter_gone: true,
    };
    f.executor
        .record_refusal(&pending, TaskCancellation::default())
        .await
        .unwrap();
    let parent = f.engine.snapshot(f.parent.session).unwrap();
    assert!(
        fs::read_to_string(&parent.board_path)
            .unwrap()
            .contains("refused: role=review reason=user_refusal")
    );
    assert_eq!(f.spawner.starts.load(Ordering::SeqCst), 0);
    let failure =
        Err(ChildLaunchError::new(409, "duplicate_role_child", "synthetic failure").into());
    f.executor
        .notify_waiter_gone(&pending, &failure, TaskCancellation::default())
        .await
        .unwrap();
    assert!(f.services.notices.lock().unwrap()[0].ends_with("detail=synthetic failure"));
    f.engine.release_children(&pending.admission);
}
#[test]
fn subscription_and_memory_helpers_preserve_fixed_go_precedence_and_explicit_clear() {
    let mut cfg = Config::default();
    cfg.user_prefs
        .spawn
        .defaults
        .insert("subscription_codex".into(), " saved ".into());
    let mut body = ChildSpawnRequest {
        role: "review".into(),
        provider: "codex".into(),
        ..Default::default()
    };
    let mut parent = SessionSnapshot {
        provider: "codex".into(),
        subscription_profile_id: "parent".into(),
        ..Default::default()
    };
    assert_eq!(
        resolve_subscription(&body, &parent, Some("role".into()), &cfg),
        "role"
    );
    assert_eq!(resolve_subscription(&body, &parent, None, &cfg), "parent");
    parent.provider = "claude".into();
    assert_eq!(resolve_subscription(&body, &parent, None, &cfg), "saved");
    body.subscription_profile_id = " explicit ".into();
    assert_eq!(
        resolve_subscription(&body, &parent, Some("role".into()), &cfg),
        "explicit"
    );
    cfg.user_prefs
        .spawn
        .role_effort
        .insert("review".into(), "high".into());
    assert!(remember_effort(&mut cfg, &body));
    assert!(!cfg.user_prefs.spawn.role_effort.contains_key("review"));
    cfg.user_prefs
        .spawn
        .role_permission
        .insert("review".into(), "full".into());
    assert!(!remember_permission(&mut cfg, &body));
    body.remember_permission = Some(false);
    assert!(remember_permission(&mut cfg, &body));
    assert!(!cfg.user_prefs.spawn.role_permission.contains_key("review"));
}

#[tokio::test]
async fn successful_launch_publishes_memory_despite_save_failure_and_explicit_provider_does_not_replace_default()
 {
    let f = executor_fixture().await;
    fs::create_dir_all(f.paths.resource(config::Resource::Config)).unwrap();
    let result = f
        .executor
        .spawn(
            launch_request(&f, ChildSpawnRequest::default()),
            TaskCancellation::default(),
        )
        .await
        .unwrap();
    assert!(f.engine.snapshot(result.id).is_some());
    assert_eq!(
        f.config
            .snapshot()
            .unwrap()
            .config
            .user_prefs
            .spawn
            .role_provider["review"],
        "copilot"
    );
    assert_eq!(f.services.warnings.load(Ordering::SeqCst), 1);
    let mut request = launch_request(
        &f,
        ChildSpawnRequest {
            provider: "claude".into(),
            force: true,
            ..Default::default()
        },
    );
    request.requested_provider = "claude".into();
    f.executor
        .spawn(request, TaskCancellation::default())
        .await
        .unwrap();
    assert_eq!(
        f.config
            .snapshot()
            .unwrap()
            .config
            .user_prefs
            .spawn
            .role_provider["review"],
        "copilot"
    );
}

#[tokio::test]
async fn confirmation_changes_record_original_resolved_provider_on_the_real_board() {
    let f = executor_fixture().await;
    let mut request = launch_request(
        &f,
        ChildSpawnRequest {
            provider: "codex".into(),
            permission_preset: "attended".into(),
            ..Default::default()
        },
    );
    request.original_body.request_mut().provider = "claude".into();
    request.original_body.request_mut().permission_preset = "full".into();
    let out = f
        .executor
        .spawn(request, TaskCancellation::default())
        .await
        .unwrap();
    assert!(
        fs::read_to_string(out.board_path)
            .unwrap()
            .contains("provider changed at confirmation: requested=claude decided=codex")
    );
    let notes = f.services.notices.lock().unwrap();
    assert!(notes[0].contains("permission_preset: requested=\"full\" decided=\"attended\""));
}
