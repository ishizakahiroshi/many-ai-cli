use super::*;
use crate::{
    application::subscriptions::cli::Status,
    config::{Config, SubscriptionProfile},
    proto::core::*,
    terminal::{
        events::CoreEventBus,
        journal::{JournalOptions, SessionJournal},
        session::{EngineOptions, SessionEngine},
    },
};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
struct Transport;
impl WrapperTransport for Transport {
    fn send<'a>(
        &'a self,
        _: SessionBinding,
        _: crate::proto::Message,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async { Ok(()) })
    }
}
struct Effects;
impl CoreEffectSink for Effects {
    fn apply<'a>(&'a self, _: CoreEffects) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
        Box::pin(async { Ok(()) })
    }
}
struct Spawn;
impl WrappedSessionSpawner for Spawn {
    fn spawn_and_wait<'a>(
        &'a self,
        _: WrappedSpawnSpec,
        _: Duration,
        _: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome> {
        Box::pin(async { panic!("diagnostic fixture must never spawn a provider") })
    }
}
pub(super) struct FakeIo {
    pub calls: Mutex<Vec<(String, Vec<String>, Duration)>>,
    pub statuses: AtomicUsize,
}
impl DiagnosticIo for FakeIo {
    fn look_path(&self, name: &str) -> io::Result<String> {
        if name == "codex" {
            Ok("synthetic-codex".into())
        } else {
            Err(io::Error::new(io::ErrorKind::NotFound, "synthetic absent"))
        }
    }
    fn command<'a>(
        &'a self,
        exe: &'a str,
        args: Vec<String>,
        _: &'a Path,
        timeout: Duration,
        cancel: &'a Cancellation,
    ) -> CoreFuture<'a, io::Result<ProbeOutput>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push((exe.into(), args, timeout));
            if cancel.is_cancelled() {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
            }
            Ok(ProbeOutput {
                bytes: b"synthetic v1\nsecret-not-first-line".to_vec(),
                success: true,
            })
        })
    }
    fn http_status<'a>(
        &'a self,
        _: &'a str,
        timeout: Duration,
        _: &'a Cancellation,
    ) -> CoreFuture<'a, io::Result<u16>> {
        Box::pin(async move {
            assert_eq!(timeout, Duration::from_secs(2));
            Ok(200)
        })
    }
    fn pid_alive(&self, _: u32) -> bool {
        false
    }
}
impl SubscriptionCli for FakeIo {
    fn status<'a>(
        &'a self,
        _: &'a str,
        _: &'a Path,
        timeout: Duration,
        _: &'a TaskCancellation,
    ) -> CoreFuture<'a, io::Result<Status>> {
        Box::pin(async move {
            assert_eq!(timeout, Duration::from_secs(5));
            self.statuses.fetch_add(1, Ordering::SeqCst);
            Ok(Status {
                logged_in: true,
                ..Default::default()
            })
        })
    }
}
pub(super) struct Fixture {
    pub _temp: tempfile::TempDir,
    pub paths: RuntimePaths,
    pub config: Arc<ConfigStore>,
    pub core: Arc<SessionEngine>,
    pub doctor: Arc<Diagnostics>,
    pub io: Arc<FakeIo>,
}
pub(super) fn fixture() -> Fixture {
    let t = tempfile::tempdir().unwrap();
    let run = t.path().join("runtime");
    let old = t.path().join("installed");
    std::fs::create_dir_all(&run).unwrap();
    std::fs::create_dir_all(&old).unwrap();
    let paths = RuntimePaths::trial(&run, 49614, &old).unwrap();
    let home = run.join("home");
    std::fs::create_dir(&home).unwrap();
    let mut cfg = Config::defaults(&paths);
    cfg.token = "synthetic-private-token".into();
    cfg.ollama.base_url = "http://192.0.2.42:11434".into();
    cfg.subscriptions.insert(
        "codex".into(),
        vec![SubscriptionProfile {
            id: "disabled".into(),
            enabled: Some(false),
            ..Default::default()
        }],
    );
    let config = Arc::new(ConfigStore::new(paths.clone(), cfg).unwrap());
    let io = Arc::new(FakeIo {
        calls: Mutex::new(vec![]),
        statuses: AtomicUsize::new(0),
    });
    let doctor = Diagnostics::new(DiagnosticsDependencies {
        config: config.clone(),
        paths: paths.clone(),
        cwd: run,
        home,
        environment: vec![],
        platform: "windows".into(),
        io: io.clone(),
        subscription_cli: io.clone(),
        nvidia_key_configured: Arc::new(|| Ok(false)),
    });
    let core = Arc::new(SessionEngine::new(
        EngineOptions::default(),
        Arc::new(SessionJournal::new(
            paths.clone(),
            None,
            JournalOptions::default(),
        )),
        Arc::new(Transport),
        Arc::new(Effects),
        Arc::new(Spawn),
        CoreEventBus::new(32).unwrap(),
    ));
    Fixture {
        _temp: t,
        paths,
        config,
        core,
        doctor,
        io,
    }
}
#[tokio::test]
async fn real_checks_have_source_order_bounded_probes_and_disabled_profile_never_auth_probes() {
    let f = fixture();
    let result = f.doctor.run(&Cancellation::default()).await.unwrap();
    let names = result
        .checks
        .iter()
        .map(|c| c.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        &names[..11],
        &[
            "provider",
            "port",
            "token",
            "ACL",
            "Ollama",
            "Whisper",
            "NVIDIA NIM",
            "Tailscale",
            "log",
            "session log",
            "handoff"
        ]
    );
    let wire = serde_json::to_value(&result).unwrap();
    assert!(wire.get("Checks").is_some());
    assert!(wire.get("checks").is_none());
    assert!(result.checks[0].message.contains("codex (synthetic v1)"));
    assert!(!result.checks[0].message.contains("secret-not-first-line"));
    assert_eq!(f.io.calls.lock().unwrap()[0].2, Duration::from_secs(3));
    assert_eq!(f.io.statuses.load(Ordering::SeqCst), 0);
    assert_eq!(result.checks.last().unwrap().level, "OK");
    assert!(result.checks.last().unwrap().message.contains("無効化"));
    assert!(
        !serde_json::to_string(&result)
            .unwrap()
            .contains("synthetic-private-token")
    );
}
#[tokio::test]
async fn cancellation_propagates_without_launching_new_work() {
    let f = fixture();
    let cancel = Cancellation::default();
    cancel.cancel();
    assert_eq!(
        f.doctor.run(&cancel).await.unwrap_err(),
        SessionError::Cancelled
    );
    assert_eq!(f.io.statuses.load(Ordering::SeqCst), 0);
}
#[test]
fn privacy_scrubs_named_secrets_private_paths_ips_but_keeps_loopback_and_public_contact() {
    let private_ip = std::net::Ipv4Addr::new(10, 2, 3, 4).to_string();
    let input = format!(
        "auth_cookie_secret=hidden remote_pin_hash=hashval vapid_private_key=keyval Bearer bearerval https://user:pass@example.com ?token=queryval D:\\dev\\github\\private\\repo {private_ip} 127.0.0.1 ishiz.private.example private@example.com ishizakahiroshi.dev@gmail.com"
    );
    let output = report::redact(&input);
    for secret in [
        "hidden",
        "hashval",
        "keyval",
        "bearerval",
        "pass@",
        "queryval",
        "D:\\dev",
        private_ip.as_str(),
        "ishiz.private.example",
        "private@example.com",
    ] {
        assert!(!output.contains(secret), "escaped {secret}");
    }
    assert!(output.contains("127.0.0.1"));
    assert!(output.contains("ishizakahiroshi.dev@gmail.com"));
    assert_eq!(report::redact(&output), output);
}
#[test]
fn report_collect_is_allowlisted_and_urls_have_fixed_destination_and_byte_limit() {
    let t = tempfile::tempdir().unwrap();
    let cfg = Config::default();
    let env = report::collect(
        &cfg,
        report::EnvironmentInput {
            version: "v1",
            platform: "windows",
            arch: "amd64",
            runtime_version: "rustc 1.90.0",
            provider: "custom-secret-provider",
            model: "",
            user_agent: "agent",
        },
    );
    assert!(env.provider.is_empty());
    assert_eq!(env.allowed.len(), 1);
    let body = report::render_markdown(
        "ja",
        "token=secret",
        "",
        &report::render_environment(&env, "ja"),
    );
    assert!(!body.contains("token=secret"));
    assert!(body.contains("再現手順"));
    let url = report::issue_url("token=secret", &body).unwrap();
    assert!(url.starts_with("https://github.com/ishizakahiroshi/many-ai-cli/issues/new?body="));
    assert!(report::issue_url("test", &"long".repeat(3000)).is_none());
    assert!(!t.path().join("unused").exists());
}
#[tokio::test]
async fn native_trial_refuses_actual_host_command_and_network_before_effect() {
    let f = fixture();
    let native = NativeDiagnosticIo {
        paths: f.paths.clone(),
        cwd: f.paths.root().into(),
        environment: vec![],
    };
    let command = native
        .command(
            "C:/outside-installed/provider.exe",
            vec!["--version".into()],
            f.paths.root(),
            Duration::from_secs(3),
            &Cancellation::default(),
        )
        .await;
    assert_eq!(command.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(
        native
            .http_status(
                "http://127.0.0.1:11434/api/tags",
                Duration::from_secs(2),
                &Cancellation::default()
            )
            .await
            .unwrap_err()
            .kind(),
        io::ErrorKind::PermissionDenied
    );
}

struct NoGist;
impl bug_report::BugReportIo for NoGist {
    fn look_path(&self, _: &str) -> io::Result<String> {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "synthetic gh absent",
        ))
    }
    fn create_secret_gist<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
        _: &'a Cancellation,
    ) -> CoreFuture<'a, io::Result<String>> {
        Box::pin(async { panic!("default HTTP preview must not publish") })
    }
}
#[tokio::test]
async fn authenticated_diagnostic_routes_keep_method_and_json_guards_before_actor_work() {
    use crate::hub::{diagnostic_routes::DiagnosticHttp, http::Request};
    let f = fixture();
    let reports = bug_report::BugReport::new(bug_report::BugReportDependencies {
        config: f.config.clone(),
        paths: f.paths.clone(),
        core: f.core.clone(),
        version: "fixture".into(),
        platform: "windows".into(),
        arch: "amd64".into(),
        runtime_version: "rustc 1.90.0".into(),
        io: Arc::new(NoGist),
    });
    let routes = DiagnosticHttp::new(f.doctor.clone(), reports);
    let cancellation = Cancellation::default();
    let rejected = routes
        .handle_authenticated(
            &Request {
                method: "POST".into(),
                path: "/api/doctor".into(),
                body: b"invalid JSON".to_vec(),
                ..Default::default()
            },
            &cancellation,
        )
        .await
        .unwrap();
    assert_eq!(rejected.status, 405);
    assert!(f.io.calls.lock().unwrap().is_empty());
    let malformed = routes
        .handle_authenticated(
            &Request {
                method: "POST".into(),
                path: "/api/bug-report/preview".into(),
                body: br#"{"include_recent_log_lines":"200"}"#.to_vec(),
                ..Default::default()
            },
            &cancellation,
        )
        .await
        .unwrap();
    assert_eq!(malformed.status, 400);
    let preview = routes
        .handle_authenticated(
            &Request {
                method: "POST".into(),
                path: "/api/bug-report/preview".into(),
                body: b"{} trailing ignored".to_vec(),
                ..Default::default()
            },
            &cancellation,
        )
        .await
        .unwrap();
    assert_eq!(preview.status, 200);
    assert!(!f.paths.root().join("reports").exists());
    assert!(f.io.calls.lock().unwrap().is_empty());
    let unknown = routes
        .handle_authenticated(
            &Request {
                method: "GET".into(),
                path: "/api/not-doctor".into(),
                ..Default::default()
            },
            &cancellation,
        )
        .await;
    assert!(unknown.is_none());
}
