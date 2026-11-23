use super::*;
use crate::terminal::session::EngineOptions;
use crate::{
    config::Config,
    proto::{self, time::Timestamp},
    terminal::{
        events::CoreEventBus,
        journal::{JournalOptions, SessionJournal},
    },
};
struct Unused;
impl WrapperTransport for Unused {
    fn send<'a>(
        &'a self,
        _: SessionBinding,
        _: proto::Message,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async { panic!("instruction owner must not write PTY") })
    }
}
impl CoreEffectSink for Unused {
    fn apply<'a>(&'a self, _: CoreEffects) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
        Box::pin(async { panic!("instruction owner must not invent effects") })
    }
}
impl WrappedSessionSpawner for Unused {
    fn spawn_and_wait<'a>(
        &'a self,
        _: WrappedSpawnSpec,
        _: std::time::Duration,
        _: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome> {
        Box::pin(async { panic!("instruction owner must not spawn provider") })
    }
}
struct Root(PathBuf, Arc<std::sync::atomic::AtomicUsize>);
impl InstructionRootResolver for Root {
    fn root<'a>(&'a self, _: &'a Path, _: &'a Cancellation) -> CoreFuture<'a, PathBuf> {
        self.1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Box::pin(async { self.0.clone() })
    }
}
struct Fixture {
    home: PathBuf,
    paths: RuntimePaths,
    owner: Arc<InstructionRules>,
    core: Arc<SessionEngine>,
    config: Arc<ConfigStore>,
    warnings: Arc<std::sync::Mutex<Vec<&'static str>>>,
    root_calls: Arc<std::sync::atomic::AtomicUsize>,
    // Last: fields drop in this order, and on Windows the folder cannot be
    // removed while a field above still holds a file inside it.
    _root: tempfile::TempDir,
    _installed: tempfile::TempDir,
}
fn fixture() -> Fixture {
    build(false)
}
/// `production` uses the installed-style runtime, where instruction targets are
/// written through the user's links instead of the trial's lexical walk.
fn build(production: bool) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    // Production walks from the drive root without following links, so start
    // from the real spelling of the temporary folder (macOS reaches it via /var).
    let base = if production {
        std::fs::canonicalize(root.path()).unwrap()
    } else {
        root.path().to_path_buf()
    };
    let home = base.join("actor");
    std::fs::create_dir(&home).unwrap();
    let paths = if production {
        RuntimePaths::production(&home).unwrap()
    } else {
        RuntimePaths::trial(root.path(), 49337, installed.path()).unwrap()
    };
    let cwd = base.join("project");
    std::fs::create_dir(&cwd).unwrap();
    let core = Arc::new(SessionEngine::new(
        EngineOptions::default(),
        Arc::new(SessionJournal::new(
            paths.clone(),
            None,
            JournalOptions::default(),
        )),
        Arc::new(Unused),
        Arc::new(Unused),
        Arc::new(Unused),
        CoreEventBus::new(64).unwrap(),
    ));
    let config = Arc::new(ConfigStore::new(paths.clone(), Config::default()).unwrap());
    let warnings = Arc::new(std::sync::Mutex::new(Vec::new()));
    let root_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let w = warnings.clone();
    let owner = InstructionRules::new(
        paths.clone(),
        home.clone(),
        vec![],
        base,
        config.clone(),
        core.clone(),
        Arc::new(Root(cwd, root_calls.clone())),
        Arc::new(move |v| w.lock().unwrap().push(v)),
    )
    .unwrap();
    Fixture {
        _root: root,
        _installed: installed,
        home,
        paths,
        owner,
        core,
        config,
        warnings,
        root_calls,
    }
}
fn at() -> Timestamp {
    Timestamp::from_unix(1791158400, 0).unwrap()
}
async fn register(f: &Fixture, provider: &str, pid: i64) -> SessionBinding {
    f.core
        .register(
            RegisterRequest {
                message: proto::Message {
                    provider: provider.into(),
                    pid,
                    cwd: f
                        ._root
                        .path()
                        .join("project/subdirectory")
                        .to_string_lossy()
                        .into_owned(),
                    home_dir: f.home.to_string_lossy().into_owned(),
                    ..Default::default()
                },
                spawn_proof: None,
            },
            WrapperConnectionId(pid as u64),
            at(),
        )
        .await
        .unwrap()
        .binding
}
fn link_file(source: &Path, alias: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(source, alias).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_file(source, alias).unwrap();
}
async fn register_claude(f: &Fixture, pid: i64, claude_dir: &Path) -> SessionBinding {
    f.core
        .register(
            RegisterRequest {
                message: proto::Message {
                    provider: "claude".into(),
                    pid,
                    cwd: f.home.to_string_lossy().into_owned(),
                    home_dir: f.home.to_string_lossy().into_owned(),
                    claude_dir: claude_dir.to_string_lossy().into_owned(),
                    ..Default::default()
                },
                spawn_proof: None,
            },
            WrapperConnectionId(pid as u64),
            at(),
        )
        .await
        .unwrap()
        .binding
}
#[tokio::test]
async fn two_profiles_linking_one_claude_md_keep_the_import_until_the_last_session_ends() {
    let f = build(true);
    let base = f.home.parent().unwrap().to_path_buf();
    std::fs::create_dir(base.join("dotfiles")).unwrap();
    let shared = base.join("dotfiles").join("CLAUDE.md");
    std::fs::write(&shared, b"custom\n").unwrap();
    let mut bindings = vec![];
    for (pid, profile) in [(2041, "first"), (2042, "second")] {
        let dir = f.home.join("subscriptions").join(profile);
        std::fs::create_dir_all(&dir).unwrap();
        link_file(&shared, &dir.join("CLAUDE.md"));
        bindings.push(register_claude(&f, pid, &dir).await);
    }
    let cancel = Cancellation::default();
    f.owner.change("enable", &cancel).await.unwrap();
    // The exact import bytes are pinned by the approval_injection tests.
    let injected =
        crate::wrapper::approval_injection::inject_rules_content("claude", b"custom\n", b"")
            .unwrap();
    assert_ne!(injected, b"custom\n");
    assert_eq!(std::fs::read(&shared).unwrap(), injected);
    let second = bindings.pop().unwrap();
    let first = bindings.pop().unwrap();
    f.core.disconnected(first, at()).unwrap();
    f.owner.ended(&cancel).await;
    assert_eq!(std::fs::read(&shared).unwrap(), injected);
    f.core.disconnected(second, at()).unwrap();
    f.owner.ended(&cancel).await;
    assert_eq!(std::fs::read(&shared).unwrap(), b"custom\n");
    assert!(!f.paths.root().join("approval-rule-targets.json").exists());
    assert!(f.warnings.lock().unwrap().is_empty());
}
#[tokio::test]
async fn a_linked_project_agents_file_is_left_alone_for_the_whole_session() {
    let f = build(true);
    let base = f.home.parent().unwrap().to_path_buf();
    // A cloned repository ships its AGENTS.md as a link to a file of the user's
    // outside the repository.
    let outside = f.home.join("secret.txt");
    std::fs::write(&outside, b"outside\n").unwrap();
    link_file(&outside, &base.join("project").join("AGENTS.md"));
    let session = register(&f, "copilot", 2051).await;
    let cancel = Cancellation::default();
    f.owner.change("enable", &cancel).await.unwrap();
    assert_eq!(std::fs::read(&outside).unwrap(), b"outside\n");
    assert!(
        f.warnings
            .lock()
            .unwrap()
            .contains(&"approval instruction injection failed")
    );
    let journal = f.paths.root().join("approval-rule-targets.json");
    let recorded: Journal = serde_json::from_slice(&std::fs::read(&journal).unwrap()).unwrap();
    assert!(recorded.targets.iter().all(|t| t.project));
    f.core.disconnected(session, at()).unwrap();
    f.owner.ended(&cancel).await;
    assert_eq!(std::fs::read(&outside).unwrap(), b"outside\n");
    // Nothing was written through the link, so the target leaves the journal
    // instead of failing again at every sweep and every start.
    assert!(!journal.exists());
    assert!(
        !f.warnings
            .lock()
            .unwrap()
            .contains(&"approval instruction removal failed")
    );
}
#[tokio::test]
async fn registered_cli_gets_the_short_block_only_when_it_declares_the_agents_file() {
    let f = fixture();
    let session = register(&f, "devin-cli", 2021).await;
    let path = f._root.path().join("project/AGENTS.md");
    std::fs::write(&path, b"original").unwrap();
    let cancel = Cancellation::default();
    // Not declared: the user's file is left alone and nothing is journalled.
    f.owner.change("enable", &cancel).await.unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"original");
    assert!(!f.paths.root().join("approval-rule-targets.json").exists());
    // Declared: the same sweep writes the short rules, never the full ones.
    f.owner
        .set_agents_file_providers(Arc::new(|| vec!["devin-cli".to_string()]));
    f.owner.registered(&cancel).await;
    let body = std::fs::read(&path).unwrap();
    assert!(body.starts_with(b"original"));
    assert!(
        body.windows(b"<!-- many-ai-cli:approval-rules -->".len())
            .any(|v| v == b"<!-- many-ai-cli:approval-rules -->")
    );
    assert!(
        body.len()
            <= b"original".len()
                + crate::wrapper::approval_injection::SHORT_LIMIT
                + 2 * "<!-- /many-ai-cli:approval-rules -->".len()
    );
    assert!(f.paths.root().join("approval-rule-targets.json").exists());
    f.core.disconnected(session, at()).unwrap();
    f.owner.ended(&cancel).await;
    assert_eq!(std::fs::read(&path).unwrap(), b"original");
    assert!(!f.paths.root().join("approval-rule-targets.json").exists());
    assert!(f.warnings.lock().unwrap().is_empty());
}
#[tokio::test]
async fn source_shared_provider_lifetime_keeps_block_until_last_actual_session_ends() {
    let f = fixture();
    let first = register(&f, "copilot", 2011).await;
    let second = register(&f, "cursor-agent", 2012).await;
    let path = f._root.path().join("project/AGENTS.md");
    std::fs::write(&path, b"original").unwrap();
    let cancel = Cancellation::default();
    f.owner.change("enable", &cancel).await.unwrap();
    assert_eq!(f.root_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    let body = std::fs::read(&path).unwrap();
    assert!(
        body.windows(b"<!-- many-ai-cli:approval-rules -->".len())
            .any(|v| v == b"<!-- many-ai-cli:approval-rules -->")
    );
    assert!(f.paths.root().join("approval-rule-targets.json").exists());
    f.core.disconnected(first, at()).unwrap();
    f.owner.ended(&cancel).await;
    assert_eq!(std::fs::read(&path).unwrap(), body);
    f.core.disconnected(second, at()).unwrap();
    f.owner.ended(&cancel).await;
    assert_eq!(std::fs::read(&path).unwrap(), b"original");
    assert!(!f.paths.root().join("approval-rule-targets.json").exists());
    assert!(f.warnings.lock().unwrap().is_empty());
}
/// A registry with the built-ins and three registered CLIs: one that declares
/// the project file it reads, one that declares nothing, and one reached over
/// ACP that declares a file all the same.
fn registered_clis() -> crate::profile::declared::TestRegistryGuard {
    use crate::{
        profile::registry,
        proto::provider::{Definition, InstructionsDefinition, LaunchDefinition, Layers},
    };
    let cli = |id: &str, connection: &str, rules_file: &str| Definition {
        schema_version: 1,
        id: id.into(),
        display_name: id.into(),
        connection: connection.into(),
        launch: Some(LaunchDefinition {
            executable: format!("{id}-bin"),
            ..Default::default()
        }),
        instructions: (!rules_file.is_empty()).then(|| InstructionsDefinition {
            rules_file: rules_file.into(),
        }),
        ..Default::default()
    };
    crate::profile::declared::install_test_registry(registry::Registry::build(
        Layers {
            embedded: Some(registry::embedded_definitions().unwrap()),
            user: Some(vec![
                cli("fixture-reads-rules", "", "RULES.md"),
                cli("fixture-silent", "", ""),
                cli("fixture-acp", "acp", "RULES.md"),
            ]),
            ..Default::default()
        },
        &registry::default_adapters(),
    ))
}
fn has_rules(path: &Path) -> bool {
    const START: &[u8] = b"<!-- many-ai-cli:approval-rules -->";
    std::fs::read(path).is_ok_and(|body| body.windows(START.len()).any(|v| v == START))
}
#[tokio::test]
async fn a_registered_cli_gets_the_rules_in_the_project_file_it_declares() {
    let _registry = registered_clis();
    let f = fixture();
    let session = register(&f, "fixture-reads-rules", 2051).await;
    let path = f._root.path().join("project/RULES.md");
    std::fs::write(&path, b"original").unwrap();
    let cancel = Cancellation::default();
    f.owner.change("enable", &cancel).await.unwrap();
    assert!(has_rules(&path));
    assert!(!f._root.path().join("project/AGENTS.md").exists());
    assert!(f.paths.root().join("approval-rule-targets.json").exists());
    f.core.disconnected(session, at()).unwrap();
    f.owner.ended(&cancel).await;
    assert_eq!(std::fs::read(&path).unwrap(), b"original");
    assert!(!f.paths.root().join("approval-rule-targets.json").exists());
    assert!(f.warnings.lock().unwrap().is_empty());
}
#[tokio::test]
async fn a_cli_that_declares_no_file_or_is_reached_over_acp_is_left_alone() {
    let _registry = registered_clis();
    let f = fixture();
    let _silent = register(&f, "fixture-silent", 2061).await;
    let _acp = register(&f, "fixture-acp", 2062).await;
    let _unknown = register(&f, "nobody-declares-this", 2063).await;
    let cancel = Cancellation::default();
    f.owner.change("enable", &cancel).await.unwrap();
    let project = f._root.path().join("project");
    assert_eq!(std::fs::read_dir(&project).unwrap().count(), 0);
    assert!(!f.paths.root().join("approval-rule-targets.json").exists());
    assert_eq!(f.root_calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert!(f.warnings.lock().unwrap().is_empty());
}
#[tokio::test]
async fn the_built_in_project_file_clis_still_get_the_rules_from_their_declaration() {
    for (provider, pid) in [
        ("copilot", 2071),
        ("cursor-agent", 2072),
        ("grok", 2073),
        ("opencode", 2074),
    ] {
        let f = fixture();
        let _session = register(&f, provider, pid).await;
        let cancel = Cancellation::default();
        f.owner.change("enable", &cancel).await.unwrap();
        assert!(
            has_rules(&f._root.path().join("project/AGENTS.md")),
            "{provider}"
        );
    }
    let f = fixture();
    let _session = register(&f, "command-code", 2075).await;
    f.owner
        .change("enable", &Cancellation::default())
        .await
        .unwrap();
    assert!(!f._root.path().join("project/AGENTS.md").exists());
}
#[tokio::test]
async fn crash_recovery_removes_only_approved_rules_and_retains_failed_journal_entries() {
    let f = fixture();
    let _binding = register(&f, "codex", 2021).await;
    let cancel = Cancellation::default();
    f.owner.change("enable", &cancel).await.unwrap();
    let target = f.home.join(".codex/AGENTS.md");
    assert!(target.exists());
    let journal = f.paths.root().join("approval-rule-targets.json");
    let files = InstructionFiles::new(f.paths.clone(), f.home.clone()).unwrap();
    let bad = f.paths.root().join("directory/AGENTS.md");
    std::fs::create_dir_all(&bad).unwrap();
    let state = Journal {
        version: 1,
        targets: vec![
            Target {
                path: target.clone(),
                providers: vec!["codex".into()],
                mode: "shared_block".into(),
                project: false,
            },
            Target {
                path: bad.clone(),
                providers: vec!["copilot".into()],
                mode: "shared_block".into(),
                project: false,
            },
        ],
    };
    files
        .write(&journal, &serde_json::to_vec(&state).unwrap(), true)
        .unwrap();
    f.owner.recover().await;
    assert!(std::fs::read(&target).unwrap().is_empty());
    let retained: Journal = serde_json::from_slice(&std::fs::read(&journal).unwrap()).unwrap();
    assert_eq!(retained.targets.len(), 1);
    assert_eq!(retained.targets[0].path, bad);
}
#[tokio::test]
async fn corrupt_journal_never_authorizes_mutation_and_http_dismiss_ignores_body() {
    let f = fixture();
    let target = f._root.path().join("AGENTS.md");
    std::fs::write(&target, b"original").unwrap();
    std::fs::write(f.paths.root().join("approval-rule-targets.json"), b"{bad").unwrap();
    f.owner.recover().await;
    assert_eq!(std::fs::read(&target).unwrap(), b"original");
    assert!(
        f.warnings
            .lock()
            .unwrap()
            .contains(&"approval instruction journal corrupt")
    );
    let http = crate::hub::approval_status_routes::ApprovalStatusHttp::new(f.owner.clone());
    let mut request = crate::hub::http::Request {
        method: "POST".into(),
        path: "/api/approval/dismiss".into(),
        ..Default::default()
    };
    request.body = b"invalid json".to_vec();
    let response = http
        .handle_authenticated(&request, &Cancellation::default())
        .await;
    assert_eq!(response.status, 204);
    assert!(response.body.is_empty());
    assert!(
        f.config
            .snapshot()
            .unwrap()
            .config
            .approval
            .first_launch_shown
    );
}
#[tokio::test]
async fn save_failure_retains_source_visible_enable_and_real_injection() {
    let f = fixture();
    register(&f, "claude", 2031).await;
    std::fs::create_dir(f.paths.root().join("config.yaml")).unwrap();
    f.owner
        .change("enable", &Cancellation::default())
        .await
        .unwrap();
    assert!(f.config.snapshot().unwrap().config.approval.enabled);
    let body = std::fs::read(f.home.join(".claude/CLAUDE.md")).unwrap();
    assert_eq!(body, b"\n@~/.many-ai-cli/approval-rules.md\n");
    assert!(
        f.warnings
            .lock()
            .unwrap()
            .contains(&"approval instruction settings persistence failed")
    );
}

#[tokio::test]
async fn oversize_journal_valid_prefix_with_invalid_suffix_cannot_authorize_recovery() {
    use std::io::{Seek, Write};
    // Corrupt recovery cannot authorize mutation of either a previous or current rules block.
    for version in ["24", "25"] {
        let f = fixture();
        let target = f._root.path().join("AGENTS.md");
        let body = format!(
            "original\n<!-- many-ai-cli:approval-rules -->\n<!-- version: {version} -->\nprivate synthetic\n<!-- /many-ai-cli:approval-rules -->\n"
        );
        std::fs::write(&target, body.as_bytes()).unwrap();
        let state = Journal {
            version: 1,
            targets: vec![Target {
                path: target.clone(),
                providers: vec!["codex".into()],
                mode: "shared_block".into(),
                project: false,
            }],
        };
        let journal = f.paths.root().join("approval-rule-targets.json");
        {
            let mut file = std::fs::File::create(&journal).unwrap();
            file.write_all(&serde_json::to_vec(&state).unwrap())
                .unwrap();
            let chunk = vec![b' '; 1024 * 1024];
            while file.stream_position().unwrap() < 32 * 1024 * 1024 {
                let remaining = (32 * 1024 * 1024 - file.stream_position().unwrap()) as usize;
                file.write_all(&chunk[..remaining.min(chunk.len())])
                    .unwrap();
            }
            file.write_all(b"invalid suffix must reject the whole journal")
                .unwrap();
        }
        f.owner.recover().await;
        assert_eq!(std::fs::read(&target).unwrap(), body.as_bytes());
        assert!(std::fs::metadata(&journal).unwrap().len() > 32 * 1024 * 1024);
        assert!(
            f.warnings
                .lock()
                .unwrap()
                .contains(&"approval instruction journal read failed")
        );
    }
}
