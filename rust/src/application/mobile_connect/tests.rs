use super::*;
use crate::config::{Config, RuntimePaths};
use std::{collections::VecDeque, sync::Mutex};
struct FakeIo {
    windows: bool,
    available: Vec<Program>,
    replies: Mutex<VecDeque<CommandOutput>>,
    calls: Mutex<Vec<String>>,
}
impl MobileIo for FakeIo {
    fn windows(&self) -> bool {
        self.windows
    }
    fn available(&self, program: Program) -> bool {
        self.available.contains(&program)
    }
    fn run<'a>(
        &'a self,
        program: Program,
        args: &'a [&'a str],
        combined: bool,
        _: &'a Cancellation,
    ) -> CoreFuture<'a, CommandOutput> {
        Box::pin(async move {
            self.calls.lock().unwrap().push(format!(
                "{program:?} {} combined={combined}",
                args.join(" ")
            ));
            self.replies
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected command")
        })
    }
}
fn output(stdout: &str, stderr: &str, success: bool) -> CommandOutput {
    CommandOutput {
        stdout: stdout.into(),
        stderr: stderr.into(),
        success,
    }
}
type MobileFixture = (
    tempfile::TempDir,
    Arc<ConfigStore>,
    Arc<FakeIo>,
    Arc<MobileConnect>,
    Arc<Mutex<Vec<&'static str>>>,
);
fn setup(replies: Vec<CommandOutput>) -> MobileFixture {
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49327, installed.path()).unwrap();
    let mut config = Config::defaults(&paths);
    config.token = "synthetic +&/~".into();
    let config = Arc::new(ConfigStore::new(paths, config).unwrap());
    let io = Arc::new(FakeIo {
        windows: true,
        available: vec![Program::Tailscale],
        replies: Mutex::new(replies.into()),
        calls: Mutex::new(vec![]),
    });
    let identity = MobileIdentity {
        export: ExportIdentity {
            username: "actor".into(),
            hostname: "HOST.".into(),
            public_host: " host ".into(),
            ipv4: vec![
                "169.254.1.1".parse().unwrap(),
                "192.0.2.17".parse().unwrap(),
            ],
            ..Default::default()
        },
        host_label: String::new(),
    };
    let warnings = Arc::new(Mutex::new(vec![]));
    let captured = warnings.clone();
    let owner = MobileConnect::new(
        config.clone(),
        io.clone(),
        identity,
        49327,
        Arc::new(move |operation| captured.lock().unwrap().push(operation)),
    );
    (root, config, io, owner, warnings)
}
#[tokio::test]
async fn enable_preserves_probe_order_escapes_token_and_idempotent_source_host() {
    let replies=(0..2).flat_map(|_|[output(r#"{"BackendState":"Running","Self":{"DNSName":" Node.Example. ","Online":true}}"#,"",true),output("127.0.0.1:49327","",true),output("","",true)]).collect();
    let (_root, config, io, owner, warnings) = setup(replies);
    let cancel = Cancellation::default();
    let first = owner.enable(&cancel).await.unwrap();
    assert!(first.no_store);
    assert_eq!(first.value["allowed_host_added"], true);
    assert_eq!(
        first.value["https_url"],
        "https://Node.Example/?token=synthetic+%2B%26%2F~"
    );
    let second = owner.enable(&cancel).await.unwrap();
    assert_eq!(second.value["allowed_host_added"], false);
    assert_eq!(
        config.snapshot().unwrap().config.hub.allowed_hosts,
        vec!["Node.Example"]
    );
    assert_eq!(
        *io.calls.lock().unwrap(),
        [
            "Tailscale status --json combined=false",
            "Tailscale serve status combined=false",
            "Tailscale serve --bg 49327 combined=false"
        ]
        .repeat(2)
    );
    assert!(warnings.lock().unwrap().is_empty());
    assert!(!io.calls.lock().unwrap().join("\n").contains("synthetic"));
}
#[tokio::test]
async fn persistence_failure_retains_published_host_and_reports_manual_hint() {
    let (root, config, _io, owner, warnings) = setup(vec![
        output(
            r#"{"BackendState":"Running","Self":{"DNSName":"node.example."}}"#,
            "",
            true,
        ),
        output("", "", true),
        output("", "", true),
    ]);
    // Force replace(config.yaml) failure without permissions assumptions on Windows.
    std::fs::create_dir(root.path().join("config.yaml")).unwrap();
    let response = owner.enable(&Cancellation::default()).await.unwrap();
    assert_eq!(response.value["ok"], true);
    assert_eq!(response.value["allowed_host_added"], false);
    assert_eq!(response.value["allowed_host_hint"], "node.example");
    assert_eq!(
        config.snapshot().unwrap().config.hub.allowed_hosts,
        vec!["node.example"]
    );
    assert_eq!(
        *warnings.lock().unwrap(),
        vec!["mobile_allowed_host_persist"]
    );
}
#[tokio::test]
async fn status_typed_errors_and_logged_out_do_not_issue_serve_or_include_token() {
    for raw in [
        r#"{"BackendState":"NeedsLogin"}"#,
        r#"{"BackendState":"Running","Self":{"Online":"yes"}}"#,
        r#"{"BackendState":9}"#,
        r#"{"BackendState":"Running"} trailing"#,
    ] {
        let (_root, _config, io, owner, _warnings) = setup(vec![output(raw, "", true)]);
        let response = owner.status(&Cancellation::default()).await.unwrap();
        assert!(response.no_store);
        assert!(response.value.get("https_url").is_none());
        assert_eq!(io.calls.lock().unwrap().len(), 1);
    }
}
#[tokio::test]
async fn tailnet_admin_failure_is_200_state_and_never_exposes_stderr() {
    let (_root, _config, _io, owner, _warnings) = setup(vec![
        output(
            r#"{"BackendState":"Running","Self":{"DNSName":"node."}}"#,
            "",
            true,
        ),
        output("", "", false),
        output(
            "",
            "not enabled secret=synthetic https://login.tailscale.com/admin/dns",
            false,
        ),
    ]);
    let response = owner.enable(&Cancellation::default()).await.unwrap();
    assert!(!response.no_store);
    assert_eq!(response.value["state"], "serve_disabled_on_tailnet");
    assert_eq!(
        response.value["admin_url"],
        "https://login.tailscale.com/admin/dns"
    );
    assert!(!response.value.to_string().contains("secret="));
}
#[tokio::test]
async fn ssh_metadata_uses_one_source_lan_and_defined_user_priority() {
    let (_root, _config, _io, owner, _warnings) =
        setup(vec![output("[SC] OpenService FAILED 1060:", "", false)]);
    let response = owner.metadata(&Cancellation::default()).await.unwrap();
    assert!(response.no_store);
    assert_eq!(
        response.value["host_candidates"],
        json!(["host", "192.0.2.17"])
    );
    assert_eq!(response.value["sshd_state"], "not_installed");
    assert_eq!(response.value["sshd_installed"], false);
    assert_eq!(response.value["ssh_user"], "actor");
    assert_eq!(
        response.value["ssh_command"],
        "ssh -L 49327:127.0.0.1:49327 actor@192.0.2.17"
    );
}
#[tokio::test]
async fn native_trial_rejects_daemon_access_before_lookup_and_spawn() {
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49327, installed.path()).unwrap();
    let io = native::NativeMobileIo::new(
        paths,
        vec!["PATH=synthetic".into()],
        root.path().into(),
        root.path().join("actor"),
        Cancellation::default(),
    )
    .unwrap();
    for program in [
        Program::Tailscale,
        Program::ServiceControl,
        Program::Systemctl,
        Program::Pgrep,
        Program::Sshd,
    ] {
        assert!(!io.available(program));
        assert!(
            !io.run(
                program,
                &["must-not-execute"],
                false,
                &Cancellation::default()
            )
            .await
            .success
        );
    }
}
#[derive(Deserialize)]
struct OracleCase {
    name: String,
    dns: String,
    token: String,
    stderr: String,
    hosts: Vec<String>,
    status: String,
}
#[derive(Deserialize)]
struct OracleGolden {
    name: String,
    normalized: String,
    url: String,
    admin: String,
    hosts: Vec<String>,
    decoded: bool,
    dns: String,
    online: bool,
    backend: String,
}
#[test]
fn pinned_go_typed_status_url_host_and_admin_oracle() {
    let cases: Vec<OracleCase> = serde_json::from_str(include_str!("cases.json")).unwrap();
    let goldens: Vec<OracleGolden> = serde_json::from_str(include_str!("golden.json")).unwrap();
    assert_eq!(cases.len(), goldens.len());
    for (case, golden) in cases.into_iter().zip(goldens) {
        assert_eq!(case.name, golden.name);
        let decoded = crate::proto::wire::decode::<Status>(case.status.as_bytes());
        assert_eq!(decoded.is_ok(), golden.decoded, "{}", case.name);
        if let Ok(value) = decoded {
            assert_eq!(value.own.dns, golden.dns, "{}", case.name);
            assert_eq!(value.own.online, golden.online, "{}", case.name);
            assert_eq!(value.backend, golden.backend, "{}", case.name);
        }
        assert_eq!(
            normalize_host(&case.dns),
            golden.normalized,
            "{}",
            case.name
        );
        assert_eq!(
            https_url(&case.dns, &case.token),
            golden.url,
            "{}",
            case.name
        );
        assert_eq!(admin_url(&case.stderr), golden.admin, "{}", case.name);
        assert_eq!(
            reachable_hosts(&case.hosts.iter().map(String::as_str).collect::<Vec<_>>()),
            golden.hosts,
            "{}",
            case.name
        );
    }
}

struct HangingUnixIo {
    calls: std::sync::atomic::AtomicUsize,
}
impl MobileIo for HangingUnixIo {
    fn windows(&self) -> bool {
        false
    }
    fn available(&self, program: Program) -> bool {
        matches!(program, Program::Systemctl | Program::Pgrep | Program::Sshd)
    }
    fn run<'a>(
        &'a self,
        _: Program,
        _: &'a [&'a str],
        _: bool,
        _: &'a Cancellation,
    ) -> CoreFuture<'a, CommandOutput> {
        Box::pin(async move {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            std::future::pending().await
        })
    }
}
#[tokio::test(start_paused = true)]
async fn unix_shared_probe_deadline_keeps_source_installed_fallback_without_starting_later_commands()
 {
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49329, installed.path()).unwrap();
    let config = Arc::new(
        ConfigStore::new(
            paths.clone(),
            Config {
                token: "synthetic".into(),
                ..Config::defaults(&paths)
            },
        )
        .unwrap(),
    );
    let io = Arc::new(HangingUnixIo {
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    let owner = MobileConnect::new(
        config,
        io.clone(),
        MobileIdentity::from_export(ExportIdentity::default(), &[]),
        49329,
        Arc::new(|_| {}),
    );
    let started = tokio::time::Instant::now();
    let response = owner.metadata(&Cancellation::default()).await.unwrap();
    assert_eq!(started.elapsed(), Duration::from_secs(5));
    assert_eq!(response.value["sshd_state"], "stopped");
    assert_eq!(response.value["sshd_installed"], true);
    assert_eq!(io.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}
#[test]
fn windows_service_output_precedence_matches_source_even_when_command_exits_nonzero() {
    for (text, success, expected) in [
        ("STATE : 4 RUNNING", true, "running"),
        ("STATE : 1 STOPPED", true, "stopped"),
        ("STATE : 4 RUNNING", false, "running"),
        ("STATE RUNNING FAILED 1060", false, "not_installed"),
        ("service does not exist", true, "not_installed"),
        ("", false, "unknown"),
        ("unexpected", true, "unknown"),
    ] {
        assert_eq!(classify_windows_sshd(text, success), expected);
    }
}
