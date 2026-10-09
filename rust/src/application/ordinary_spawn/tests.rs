use super::*;
use crate::{
    config::{Config, RuntimePaths},
    orchestration::child_launch::worktree::WorktreeGit,
    profile::registry::{Registry, default_adapters, embedded_definitions},
    proto::{self, provider::Layers, time::UNIX_EPOCH},
    terminal::{
        events::CoreEventBus,
        journal::{JournalOptions, SessionJournal},
        session::EngineOptions,
    },
};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

struct Capture {
    requests: Mutex<Vec<WrappedStartRequest>>,
    outcome: SpawnStartOutcome,
    gate: Mutex<Option<Arc<tokio::sync::Notify>>>,
    entered: tokio::sync::Notify,
}
impl WrappedSessionSpawner for Capture {
    fn start_wrapped<'a>(
        &'a self,
        request: WrappedStartRequest,
        _: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnStartOutcome> {
        Box::pin(async move {
            self.requests.lock().unwrap().push(request);
            let gate = self.gate.lock().unwrap().clone();
            self.entered.notify_one();
            if let Some(gate) = gate {
                gate.notified().await;
            }
            self.outcome.clone()
        })
    }
    fn spawn_and_wait<'a>(
        &'a self,
        _: WrappedSpawnSpec,
        _: Duration,
        _: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome> {
        Box::pin(async { panic!("ordinary HTTP launch must use native Start") })
    }
}
#[derive(Default)]
struct Policy(AtomicUsize);
impl SpawnLaunchPolicy for Policy {
    fn resolve(
        &self,
        spec: &WrappedSpawnSpec,
        base: &[String],
    ) -> std::io::Result<ResolvedSpawnPolicy> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(ResolvedSpawnPolicy {
            base_environment: base.to_vec(),
            route_environment: vec![],
            subscription_environment: vec![],
            effective_route: String::new(),
            current_model: spec.model.clone(),
            resolved_model: spec.model.clone(),
            effort_args: vec![],
            ordinary_model_args: vec![],
        })
    }
    fn folder_trust(&self, _: &WrappedSpawnSpec, _: &[String]) -> std::io::Result<()> {
        panic!("caller cannot perform native folder trust")
    }
    fn registered_model(&self, _: &str, _: &str) -> std::io::Result<()> {
        panic!("caller cannot record native model")
    }
}
#[derive(Default)]
struct Hooks {
    registered: AtomicUsize,
    warnings: AtomicUsize,
}
impl OrdinarySpawnHooks for Hooks {
    fn registered<'a>(
        &'a self,
        _: SessionBinding,
        _: SpawnRegistrationMetadata,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            self.registered.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }
    fn warning(&self, _: &'static str) {
        self.warnings.fetch_add(1, Ordering::SeqCst);
    }
}
struct Boundary;
impl WrapperTransport for Boundary {
    fn send<'a>(
        &'a self,
        _: SessionBinding,
        _: proto::Message,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async { panic!("native acceptance does not emit wrapper commands") })
    }
}
impl CoreEffectSink for Boundary {
    fn apply<'a>(&'a self, _: CoreEffects) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
        Box::pin(async { panic!("native acceptance does not create sessions") })
    }
}
struct Fixture {
    _root: tempfile::TempDir,
    paths: RuntimePaths,
    service: Arc<OrdinarySpawnService>,
    capture: Arc<Capture>,
    policy: Arc<Policy>,
    hooks: Arc<Hooks>,
    repo: PathBuf,
    _tasks: crate::hub::task_owner::HubTaskOwner,
}
impl Fixture {
    fn new(outcome: SpawnStartOutcome) -> Self {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("isolated-home");
        let repo = root.path().join("repository");
        let trial = root.path().join("trial");
        let installed = root.path().join("installed");
        for dir in [&home, &repo, &trial, &installed] {
            std::fs::create_dir(dir).unwrap();
        }
        let empty = root.path().join("empty-config");
        std::fs::write(&empty, "").unwrap();
        let template = root.path().join("empty-templates");
        std::fs::create_dir(&template).unwrap();
        let git = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|p| p.join(if cfg!(windows) { "git.exe" } else { "git" }))
            .find(|p| p.is_file())
            .expect("ordinary worktree tests require Git on PATH");
        let mut env = BTreeMap::<OsString, Option<OsString>>::new();
        for key in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_COMMON_DIR",
            "GIT_CONFIG",
            "GIT_CONFIG_PARAMETERS",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_CONFIG_SYSTEM",
            "GIT_EXEC_PATH",
            "GIT_NAMESPACE",
        ] {
            env.insert(key.into(), None);
        }
        for (key, value) in [
            ("HOME", home.as_os_str()),
            ("USERPROFILE", home.as_os_str()),
            ("GIT_CONFIG_GLOBAL", empty.as_os_str()),
            ("GIT_TEMPLATE_DIR", template.as_os_str()),
        ] {
            env.insert(key.into(), Some(value.to_owned()));
        }
        for (key, value) in [
            ("GIT_CONFIG_NOSYSTEM", "1"),
            ("GIT_CONFIG_COUNT", "0"),
            ("GIT_TERMINAL_PROMPT", "0"),
            ("GIT_AUTHOR_NAME", "Synthetic"),
            ("GIT_AUTHOR_EMAIL", "fixture@example.invalid"),
            ("GIT_COMMITTER_NAME", "Synthetic"),
            ("GIT_COMMITTER_EMAIL", "fixture@example.invalid"),
        ] {
            env.insert(key.into(), Some(value.into()));
        }
        for argv in [
            vec!["init", "-q"],
            vec![
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "synthetic",
            ],
        ] {
            let mut command = std::process::Command::new(&git);
            command.current_dir(&repo).args(argv);
            for (key, value) in &env {
                if let Some(value) = value {
                    command.env(key, value);
                } else {
                    command.env_remove(key);
                }
            }
            let output = command.output().unwrap();
            assert!(
                output.status.success(),
                "synthetic Git init: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let paths = RuntimePaths::trial(&trial, 49181, &installed).unwrap();
        let config = Arc::new(ConfigStore::new(paths.clone(), Config::defaults(&paths)).unwrap());
        let bus = CoreEventBus::new(32).unwrap();
        let capture = Arc::new(Capture {
            requests: Mutex::new(vec![]),
            outcome,
            gate: Mutex::new(None),
            entered: tokio::sync::Notify::new(),
        });
        let core = Arc::new(SessionEngine::new(
            EngineOptions::default(),
            Arc::new(SessionJournal::new(
                paths.clone(),
                None,
                JournalOptions {
                    session_enabled: false,
                    max_bytes: 0,
                },
            )),
            Arc::new(Boundary),
            Arc::new(Boundary),
            capture.clone(),
            bus,
        ));
        let registry = Registry::build(
            Layers {
                embedded: Some(embedded_definitions().unwrap()),
                ..Default::default()
            },
            &default_adapters(),
        );
        let policy = Arc::new(Policy::default());
        let hooks = Arc::new(Hooks::default());
        let tasks = crate::hub::task_owner::HubTaskOwner::new(tokio::runtime::Handle::current());
        let service = OrdinarySpawnService::new(OrdinarySpawnDependencies {
            core,
            config,
            policy: policy.clone(),
            registry: Arc::new(move || Ok(registry.clone())),
            worktrees: NormalWorktreeLifecycle::new(repo.clone(), WorktreeGit::new(git, env)),
            hooks: hooks.clone(),
            hub_cwd: repo.clone(),
            home,
            environment: vec![],
            tasks: tasks.handle(),
            orchestration: None,
        })
        .unwrap();
        Self {
            _root: root,
            paths,
            service: Arc::new(service),
            capture,
            policy,
            hooks,
            repo,
            _tasks: tasks,
        }
    }
    async fn spawn(
        &self,
        body: OrdinarySpawnRequest,
    ) -> Result<Option<OrchestrationId>, SpawnError> {
        self.service
            .spawn(
                body,
                &VerifiedConfirmationRequest::after_server_authentication(
                    self.service.core.auth_epoch(),
                ),
                &HttpWaitCancellation::default(),
                UNIX_EPOCH + Duration::from_secs(1767323045),
                0,
            )
            .await
    }
    fn body(&self) -> OrdinarySpawnRequest {
        OrdinarySpawnRequest {
            provider: "copilot".into(),
            cwd: self.repo.to_string_lossy().into(),
            label: "synthetic-ordinary".into(),
            isolate_worktree: Some(true),
            initial_prompt: "synthetic injection".into(),
            ..Default::default()
        }
    }
}

#[tokio::test]
async fn caller_native_acceptance_retains_exact_metadata_and_startup_failure_rolls_back() {
    for outcome in [
        SpawnStartOutcome::Started { pid: 42 },
        SpawnStartOutcome::WaiterCancelled,
    ] {
        let f = Fixture::new(outcome.clone());
        let result = f.spawn(f.body()).await;
        assert_eq!(
            result.as_ref().err().map(|e| e.status),
            if outcome == SpawnStartOutcome::WaiterCancelled {
                Some(499)
            } else {
                None
            }
        );
        let request = f.capture.requests.lock().unwrap().pop().unwrap();
        assert_eq!(f.policy.0.load(Ordering::SeqCst), 1);
        assert_eq!(f.hooks.registered.load(Ordering::SeqCst), 0);
        let attempt = request.spec.spawn_attempt.unwrap();
        let metadata = f
            .service
            .core
            .peek_spawn_metadata(attempt, "copilot")
            .unwrap();
        assert_eq!(metadata.initial_prompt, "synthetic injection");
        assert!(Path::new(&metadata.normal_worktree.path).is_dir());
        assert!(
            f.service
                .core
                .peek_spawn_metadata(attempt, "claude")
                .is_none()
        );
        assert!(
            f.service
                .core
                .peek_spawn_metadata(SpawnAttemptId(attempt.0 + 1), "copilot")
                .is_none()
        );
        assert!(f.service.core.begin_provider_update("copilot").is_err());
        f.service.startup_failed(&metadata).await;
        f.service.core.spawn_failed(attempt, "copilot").unwrap();
        assert!(!Path::new(&metadata.normal_worktree.path).exists());
        assert!(
            f.service
                .core
                .peek_spawn_metadata(attempt, "copilot")
                .is_none()
        );
        assert_eq!(f.hooks.warnings.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn native_creation_failure_removes_only_created_worktree_and_releases_admission() {
    let f = Fixture::new(SpawnStartOutcome::Failed("synthetic failure".into()));
    assert_eq!(f.spawn(f.body()).await.unwrap_err().status, 500);
    let request = f.capture.requests.lock().unwrap().pop().unwrap();
    assert!(!Path::new(&request.spec.registration_metadata.normal_worktree.path).exists());
    assert!(f.repo.join(".git").is_dir());
    assert!(f.service.core.begin_provider_update("copilot").is_ok());
    assert_eq!(f.policy.0.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn authenticated_http_success_decode_failure_and_shell_early_path() {
    let f = Fixture::new(SpawnStartOutcome::Started { pid: 42 });
    let core = f.service.core.clone();
    let http = crate::hub::spawn_routes::SpawnHttp::new(f.service.clone());
    let proof = VerifiedConfirmationRequest::after_server_authentication(core.auth_epoch());
    let request=crate::hub::http::Request{body:br#"{"provider":"shell","model":"ignored-model","permission_mode":"bypassPermissions","sandbox":"danger-full-access","utf8_session":true,"isolate_worktree":false}"#.to_vec(),..Default::default()};
    let response = http
        .spawn_authenticated(
            &request,
            &proof,
            &HttpWaitCancellation::default(),
            UNIX_EPOCH,
            0,
        )
        .await;
    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"{\"ok\":true}\n");
    assert_eq!(f.policy.0.load(Ordering::SeqCst), 0);
    {
        let captured = f.capture.requests.lock().unwrap();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].kind, WrappedStartKind::Bare);
        assert!(captured[0].policy.resolved_model.is_empty());
    }
    let invalid = crate::hub::http::Request {
        body: br#"{"provider":true}"#.to_vec(),
        ..Default::default()
    };
    assert_eq!(
        http.spawn_authenticated(
            &invalid,
            &proof,
            &HttpWaitCancellation::default(),
            UNIX_EPOCH,
            0
        )
        .await
        .status,
        400
    );
    assert_eq!(f.capture.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn dropped_http_future_does_not_cancel_owned_worktree_or_start_workflow() {
    let f = Fixture::new(SpawnStartOutcome::Started { pid: 42 });
    let gate = Arc::new(tokio::sync::Notify::new());
    *f.capture.gate.lock().unwrap() = Some(gate.clone());
    let service = f.service.clone();
    let body = f.body();
    let observer = tokio::spawn(async move {
        service
            .spawn(
                body,
                &VerifiedConfirmationRequest::after_server_authentication(
                    service.core.auth_epoch(),
                ),
                &HttpWaitCancellation::default(),
                UNIX_EPOCH + Duration::from_secs(1767323045),
                0,
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(15), f.capture.entered.notified())
        .await
        .unwrap();
    observer.abort();
    assert!(observer.await.unwrap_err().is_cancelled());
    let attempt = f.capture.requests.lock().unwrap()[0]
        .spec
        .spawn_attempt
        .unwrap();
    assert!(
        f.service
            .core
            .peek_spawn_metadata(attempt, "copilot")
            .is_some()
    );
    gate.notify_one();
    tokio::task::yield_now().await;
    let metadata = f
        .service
        .core
        .peek_spawn_metadata(attempt, "copilot")
        .unwrap();
    assert!(Path::new(&metadata.normal_worktree.path).is_dir());
    f.service.startup_failed(&metadata).await;
    f.service.core.spawn_failed(attempt, "copilot").unwrap();
}

#[test]
fn ordinary_request_decode_retains_go_case_null_duplicate_and_typed_values() {
    let body: OrdinarySpawnRequest = crate::proto::decode_http_json(br#"{"provider":"claude","PROVIDER":"codex","risk_confirmed":true,"risk_confirmed":null,"isolate_worktree":false,"delegation":null,"unknown":1e1000} trailing"#).unwrap();
    assert_eq!(body.provider, "codex");
    assert!(body.risk_confirmed);
    assert_eq!(body.isolate_worktree, Some(false));
    assert_eq!(body.delegation, None);
    for invalid in [
        br#"{"risk_confirmed":"true"}"#.as_slice(),
        br#"{"handoff_from":9223372036854775808}"#.as_slice(),
        br#"{"orchestration_roles":{"worker":{"provider":true}}}"#.as_slice(),
    ] {
        assert!(crate::proto::decode_http_json::<OrdinarySpawnRequest>(invalid).is_err());
    }
}

#[test]
fn validation_preserves_source_risk_fields_and_rejects_control_route_and_cleanup() {
    let mut body = OrdinarySpawnRequest {
        provider: "claude".into(),
        ..Default::default()
    };
    assert!(body.validate().is_ok());
    body.permission_mode = "dontAsk".into();
    assert!(body.validate().is_ok());
    body.worktree_cleanup = "force".into();
    assert_eq!(
        body.validate().unwrap_err().detail,
        "invalid worktree cleanup policy"
    );
    body.worktree_cleanup.clear();
    body.model = "safe\n--flag".into();
    assert!(body.validate().is_err());
    body.model = "model with spaces".into();
    assert_eq!(
        body.validate().unwrap_err().detail,
        "invalid model or label value"
    );
    body.model.clear();
    body.label = "label with spaces".into();
    assert_eq!(
        body.validate().unwrap_err().detail,
        "invalid model or label value"
    );
    body.label = "label-with-dashes".into();
    assert!(body.validate().is_ok());
    body.route = "nvidia-nim".into();
    assert_eq!(
        body.validate().unwrap_err().detail,
        "route is not available for this provider"
    );
}

#[test]
fn ordinary_risk_matches_pinned_provider_switch_without_preset_confirmation() {
    let policy = ResolvedSpawnPolicy {
        base_environment: vec![],
        route_environment: vec![],
        subscription_environment: vec![],
        effective_route: String::new(),
        current_model: "old".into(),
        resolved_model: "new".into(),
        effort_args: vec![],
        ordinary_model_args: vec![],
    };
    for (provider, expected) in [
        ("claude", true),
        ("codex", true),
        ("copilot", false),
        ("cursor-agent", false),
        ("grok", false),
        ("opencode", true),
        ("command-code", true),
        ("synthetic-custom", false),
        ("shell", false),
    ] {
        let body = OrdinarySpawnRequest {
            provider: provider.into(),
            permission_mode: "bypassPermissions".into(),
            ..Default::default()
        };
        let mut spec = body.spec(
            PathBuf::from("/synthetic"),
            SpawnAttemptId(1),
            InternalSpawnGrants::default(),
        );
        assert_eq!(risk(&spec, &policy), expected, "{provider}");
        spec.risk_confirmed = true;
        assert!(!risk(&spec, &policy));
    }
    let cfg = config::OrchestrationConfig {
        child_full_bypass: Some(false),
        ..Default::default()
    };
    let preset =
        crate::orchestration::child_options::permission_preset_approval("codex", "full", &cfg);
    assert_eq!(preset.sandbox, "danger-full-access");
    let body = OrdinarySpawnRequest {
        provider: "codex".into(),
        sandbox: preset.sandbox,
        ..Default::default()
    };
    let spec = body.spec(
        PathBuf::from("/synthetic"),
        SpawnAttemptId(1),
        InternalSpawnGrants::default(),
    );
    assert!(!spec.risk_confirmed);
    assert!(risk(&spec, &policy));
}

fn conductor_fixture(outcome: SpawnStartOutcome) -> Fixture {
    let mut f = Fixture::new(outcome);
    let service = Arc::get_mut(&mut f.service).unwrap();
    let workers = super::super::session_workers::SessionWorkers::new(
        service.config.clone(),
        f.paths.clone(),
        Arc::new(crate::files::FilesService::new(
            f.paths.root().to_owned(),
            f.paths.clone(),
        )),
        f._tasks.handle(),
        Arc::new(|_, _| {}),
    );
    let effects: Arc<dyn CoreEffectSink> = Arc::new(Boundary);
    service.orchestration = Some(
        OrchestrationProgram::new(
            super::super::orchestration_program::OrchestrationDependencies {
                core: Arc::downgrade(&service.core),
                effects: Arc::downgrade(&effects),
                config: service.config.clone(),
                paths: f.paths.clone(),
                registry: service.registry.clone(),
                environment: vec![],
                hub_cwd: f.paths.root().to_owned(),
                workers,
                tasks: f._tasks.handle(),
                warning: Arc::new(|_, _| {}),
            },
        )
        .unwrap(),
    );
    f
}

#[tokio::test]
async fn conductor_source_roles_response_and_launch_prompt_share_one_reservation() {
    let f = conductor_fixture(SpawnStartOutcome::Started { pid: 42 });
    let mut body = f.body();
    body.provider = "claude".into();
    body.execution_mode = "headless".into();
    body.orchestration = true;
    body.initial_prompt = "screen instruction superseded by source conductor guide".into();
    body.orchestration_roles = Some(BTreeMap::from([(
        "review".into(),
        Some(request::RoleAssignment {
            provider: "codex".into(),
            model: "gpt-5.2-codex".into(),
            subscription: "synthetic-profile".into(),
            effort: "high".into(),
            execution_mode: "headless".into(),
            permission_preset: "full".into(),
        }),
    )]));
    let id = f.spawn(body).await.unwrap().unwrap();
    let request = f.capture.requests.lock().unwrap().pop().unwrap();
    assert_eq!(request.spec.registration_metadata.orchestration, id);
    assert!(request.spec.registration_metadata.prompt_at_launch);
    assert!(request.spec.registration_metadata.initial_prompt.is_empty());
    assert_eq!(
        request.spec.initial_prompt,
        f.service
            .orchestration
            .as_ref()
            .unwrap()
            .conductor_prompt(&id)
    );
    assert!(
        !request
            .spec
            .initial_prompt
            .contains("screen instruction superseded")
    );
    let role = &f.service.orchestration.as_ref().unwrap().roles(&id)["review"];
    assert_eq!(
        (
            &role.provider,
            &role.model,
            &role.subscription,
            &role.effort,
            &role.execution_mode,
            &role.permission_preset
        ),
        (
            &"codex".to_owned(),
            &"gpt-5.2-codex".to_owned(),
            &"synthetic-profile".to_owned(),
            &"high".to_owned(),
            &"headless".to_owned(),
            &"full".to_owned()
        )
    );
    assert_eq!(f.policy.0.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn conductor_invalid_role_after_worktree_preparation_rolls_back_before_response() {
    let f = conductor_fixture(SpawnStartOutcome::Started { pid: 42 });
    let mut body = f.body();
    body.orchestration = true;
    body.orchestration_roles = Some(BTreeMap::from([(
        "review".into(),
        Some(request::RoleAssignment {
            provider: "invalid-provider".into(),
            ..Default::default()
        }),
    )]));
    let error = f.spawn(body).await.unwrap_err();
    assert_eq!(error.detail, "invalid orchestration role provider");
    assert!(f.capture.requests.lock().unwrap().is_empty());
    assert!(f.service.core.begin_provider_update("copilot").is_ok());
    let trees = f.repo.join(".git-worktrees");
    assert!(trees.is_dir());
    assert_eq!(std::fs::read_dir(trees).unwrap().count(), 0);
}

#[test]
fn spawn_correlation_schema_validation_and_legacy_compatibility() {
    for raw in [
        br#"{}"#.as_slice(),
        br#"{"client_request_id":null}"#,
        br#"{"client_request_id":""}"#,
    ] {
        let body: OrdinarySpawnRequest = crate::proto::decode_http_json(raw).unwrap();
        assert!(body.client_request_id.is_empty());
        assert!(body.validate().is_ok());
    }
    let body: OrdinarySpawnRequest =
        crate::proto::decode_http_json(br#"{"client_request_id":"browser_a-123"}"#).unwrap();
    assert_eq!(body.client_request_id, "browser_a-123");
    assert!(body.validate().is_ok());
    for id in [
        "x".repeat(129),
        "bad space".into(),
        "非ASCII".into(),
        "bad\nvalue".into(),
    ] {
        let body = OrdinarySpawnRequest {
            client_request_id: id,
            ..Default::default()
        };
        assert_eq!(
            body.validate().unwrap_err().detail,
            "invalid client_request_id"
        );
    }
    assert!(
        crate::proto::decode_http_json::<OrdinarySpawnRequest>(br#"{"client_request_id":true}"#)
            .is_err()
    );
}

#[tokio::test]
async fn spawn_correlation_ordinary_cancellation_and_invalid_input_before_side_effects() {
    for outcome in [
        SpawnStartOutcome::Started { pid: 42 },
        SpawnStartOutcome::WaiterCancelled,
    ] {
        let f = Fixture::new(outcome);
        let mut body = f.body();
        body.client_request_id = "request_ordinary-1".into();
        let _ = f.spawn(body).await;
        let request = f.capture.requests.lock().unwrap().pop().unwrap();
        assert_eq!(
            request.spec.registration_metadata.client_request_id,
            "request_ordinary-1"
        );
        assert_eq!(
            f.service
                .core
                .peek_spawn_metadata(request.spec.spawn_attempt.unwrap(), "copilot")
                .unwrap()
                .client_request_id,
            "request_ordinary-1"
        );
    }
    let f = Fixture::new(SpawnStartOutcome::Started { pid: 42 });
    let mut body = f.body();
    body.client_request_id = "invalid value".into();
    assert_eq!(f.spawn(body).await.unwrap_err().status, 400);
    assert!(f.capture.requests.lock().unwrap().is_empty());
    assert_eq!(f.policy.0.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn spawn_correlation_orchestration_keeps_authenticated_request_metadata() {
    let f = conductor_fixture(SpawnStartOutcome::Started { pid: 42 });
    let mut body = f.body();
    body.provider = "claude".into();
    body.execution_mode = "headless".into();
    body.orchestration = true;
    body.client_request_id = "request_conductor-1".into();
    let id = f.spawn(body).await.unwrap().unwrap();
    let request = f.capture.requests.lock().unwrap().pop().unwrap();
    assert_eq!(request.spec.registration_metadata.orchestration, id);
    assert_eq!(
        request.spec.registration_metadata.client_request_id,
        "request_conductor-1"
    );
}
