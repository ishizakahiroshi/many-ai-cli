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
struct Root(PathBuf);
impl InstructionRootResolver for Root {
    fn root<'a>(&'a self, _: &'a Path, _: &'a Cancellation) -> CoreFuture<'a, PathBuf> {
        Box::pin(async { self.0.clone() })
    }
}
struct Fixture {
    _root: tempfile::TempDir,
    _installed: tempfile::TempDir,
    home: PathBuf,
    paths: RuntimePaths,
    owner: Arc<InstructionRules>,
    core: Arc<SessionEngine>,
    config: Arc<ConfigStore>,
    warnings: Arc<std::sync::Mutex<Vec<&'static str>>>,
}
fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49337, installed.path()).unwrap();
    let home = root.path().join("actor");
    std::fs::create_dir(&home).unwrap();
    let cwd = root.path().join("project");
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
    let w = warnings.clone();
    let owner = InstructionRules::new(
        paths.clone(),
        home.clone(),
        vec![],
        root.path().to_path_buf(),
        config.clone(),
        core.clone(),
        Arc::new(Root(cwd)),
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
#[tokio::test]
async fn source_shared_provider_lifetime_keeps_block_until_last_actual_session_ends() {
    let f = fixture();
    let first = register(&f, "copilot", 2011).await;
    let second = register(&f, "cursor-agent", 2012).await;
    let path = f._root.path().join("project/AGENTS.md");
    std::fs::write(&path, b"original").unwrap();
    let cancel = Cancellation::default();
    f.owner.change("enable", &cancel).await.unwrap();
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
            },
            Target {
                path: bad.clone(),
                providers: vec!["copilot".into()],
                mode: "shared_block".into(),
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
