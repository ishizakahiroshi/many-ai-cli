use super::*;
use std::sync::Mutex;
#[derive(Clone)]
struct Receipt {
    port: u16,
    path: String,
    body: Option<Value>,
    timeout: Duration,
}
struct Fake {
    receipts: Mutex<Vec<Receipt>>,
    response: Mutex<Result<HttpReply, RequestError>>,
}
impl OrchestrateIo for Fake {
    fn exchange<'a>(
        &'a self,
        request: HttpRequest<'a>,
        _: &'a Cancellation,
    ) -> CoreFuture<'a, Result<HttpReply, RequestError>> {
        Box::pin(async move {
            assert_eq!(request.token, "synthetic-private-hub-token");
            self.receipts.lock().unwrap().push(Receipt {
                port: request.port,
                path: request.path,
                body: request.body,
                timeout: request.timeout,
            });
            match &*self.response.lock().unwrap() {
                Ok(reply) => Ok(HttpReply {
                    status: reply.status,
                    body: reply.body.clone(),
                }),
                Err(error) => Err(*error),
            }
        })
    }
}
struct Fixture {
    _temp: tempfile::TempDir,
    cli: OrchestrateCli,
    io: Arc<Fake>,
}
fn fixture() -> Fixture {
    let t = tempfile::tempdir().unwrap();
    let run = t.path().join("runtime");
    let installed = t.path().join("installed");
    std::fs::create_dir(&run).unwrap();
    std::fs::create_dir(&installed).unwrap();
    let paths = RuntimePaths::trial(&run, 49381, &installed).unwrap();
    let io = Arc::new(Fake {
        receipts: Mutex::new(vec![]),
        response: Mutex::new(Ok(HttpReply {
            status: 200,
            body: br#"{"ok":true,"session_id":12,"board_path":"board.md","cwd":"actual-worktree"}"#
                .to_vec(),
        })),
    });
    let cli = OrchestrateCli {
        paths,
        cwd: run,
        environment: vec![
            "MANY_AI_CLI_SESSION_ID=11".into(),
            "MANY_AI_CLI_HUB_PORT=49381".into(),
            "MANY_AI_CLI_HUB_TOKEN=synthetic-private-hub-token".into(),
        ],
        io: io.clone(),
    };
    Fixture { _temp: t, cli, io }
}
fn args(v: &[&str]) -> Vec<String> {
    v.iter().map(|v| v.to_string()).collect()
}
#[tokio::test]
async fn spawn_actual_payload_omission_header_boundary_and_source_reply_text() {
    let f = fixture();
    let output = f
        .cli
        .run(
            &args(&["spawn", "--role", "implementation", "prompt"]),
            &Cancellation::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        output,
        "spawned child session #12 role=implementation board=board.md cwd=actual-worktree\n"
    );
    let receipts = f.io.receipts.lock().unwrap();
    let receipt = &receipts[0];
    assert_eq!(receipt.port, 49381);
    assert_eq!(receipt.path, "/api/sessions/11/spawn-child");
    assert_eq!(receipt.timeout, Duration::from_secs(300));
    assert_eq!(
        receipt.body.as_ref().unwrap(),
        &json!({"role":"implementation","initial_prompt":"prompt"})
    );
    assert!(
        !receipt
            .body
            .as_ref()
            .unwrap()
            .to_string()
            .contains("synthetic-private-hub-token")
    );
}
#[tokio::test]
async fn flags_and_source_pending_timeout_do_not_indicate_refusal_or_retry() {
    let f = fixture();
    *f.io.response.lock().unwrap() = Err(RequestError::Timeout);
    let error = f
        .cli
        .run(
            &args(&[
                "spawn",
                "--role",
                "review",
                "--same-tree",
                "--force",
                "--cwd",
                "requested",
                "--execution-mode",
                "headless",
                "prompt",
            ]),
            &Cancellation::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert!(error.to_string().contains("DO NOT retry spawn"));
    assert!(!error.to_string().contains("spawn refused"));
    assert_eq!(f.io.receipts.lock().unwrap().len(), 1);
    let body = f.io.receipts.lock().unwrap()[0].body.clone().unwrap();
    assert_eq!(body["same_tree"], true);
    assert_eq!(body["execution_mode"], "headless");
}
#[tokio::test]
async fn invalid_session_and_trial_port_are_rejected_before_any_http_call() {
    let mut f = fixture();
    f.cli.environment[0] = "MANY_AI_CLI_SESSION_ID=invalid".into();
    assert!(
        f.cli
            .run(
                &args(&["send", "--role", "review", "text"]),
                &Cancellation::default()
            )
            .await
            .is_err()
    );
    assert!(f.io.receipts.lock().unwrap().is_empty());
    f.cli.environment[0] = "MANY_AI_CLI_SESSION_ID=11".into();
    f.cli.environment[1] = "MANY_AI_CLI_HUB_PORT=47777".into();
    assert!(
        f.cli
            .run(
                &args(&["send", "--role", "review", "text"]),
                &Cancellation::default()
            )
            .await
            .is_err()
    );
    assert!(f.io.receipts.lock().unwrap().is_empty());
}
#[tokio::test]
async fn relay_roles_plan_absolute_ack_and_filter_use_existing_source_routes() {
    let f = fixture();
    *f.io.response.lock().unwrap() = Ok(HttpReply {
        status: 200,
        body:
            br#"{"ok":true,"relay":{"orchestration_id":"relay1","mode":"worktree","max_rounds":3}}"#
                .to_vec(),
    });
    let result = f
        .cli
        .run(
            &args(&[
                "relay",
                "--plan",
                "plan.md",
                "--impl",
                "codex/model@high",
                "--review",
                "claude",
                "--permission",
                "bounded",
            ]),
            &Cancellation::default(),
        )
        .await
        .unwrap();
    assert!(result.starts_with("relay started orchestration=relay1"));
    let body = f.io.receipts.lock().unwrap()[0].body.clone().unwrap();
    assert!(PathBuf::from(body["plan_path"].as_str().unwrap()).is_absolute());
    assert_eq!(body["acknowledge_child_full_bypass"], true);
    assert_eq!(body["roles"]["implementation"]["effort"], "high");
    assert_eq!(body["roles"]["review"]["permission_preset"], "bounded");
    assert!(body.get("max_rounds").is_none());
    *f.io.response.lock().unwrap() = Ok(HttpReply {
        status: 200,
        body: br#"{"ok":true,"relays":[]}"#.to_vec(),
    });
    assert_eq!(
        f.cli
            .run(&args(&["relay", "status"]), &Cancellation::default())
            .await
            .unwrap(),
        "no relays\n"
    );
    assert_eq!(
        f.io.receipts.lock().unwrap()[1].timeout,
        Duration::from_secs(30)
    );
}
#[test]
fn source_duration_and_role_parser_keep_fractional_compounds_and_reject_invalid_inputs() {
    assert_eq!(timeout_value("1m2.5s"), Duration::from_millis(62500));
    assert_eq!(timeout_value("+0.5ms"), Duration::from_micros(500));
    assert_eq!(timeout_value("100µs"), Duration::from_micros(100));
    for raw in [
        "",
        "-1s",
        "0",
        "0s",
        "nonsense",
        "999999999999999999999999999999999999999999999999s",
    ] {
        assert_eq!(timeout_value(raw), Duration::from_secs(300));
    }
    assert_eq!(
        parse_role("codex/vendor/model@high").unwrap(),
        ("codex".into(), "vendor/model".into(), "high".into())
    );
    assert!(parse_role("claude@").is_err());
    assert!(parse_role("codex/bad model").is_err());
}

#[tokio::test]
async fn native_owned_loopback_request_carries_credentials_only_in_header() {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("runtime");
    let installed = t.path().join("installed");
    std::fs::create_dir(&root).unwrap();
    std::fs::create_dir(&installed).unwrap();
    let paths = RuntimePaths::trial(&root, port, &installed).unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = vec![0; 8192];
        let count = socket.read(&mut bytes).await.unwrap();
        let received = String::from_utf8_lossy(&bytes[..count]).into_owned();
        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}",
            )
            .await
            .unwrap();
        received
    });
    let io = NativeOrchestrateIo { paths };
    let reply = io
        .exchange(
            HttpRequest {
                port,
                path: "/api/sessions/11/relay".into(),
                token: "synthetic-private-hub-token",
                body: None,
                timeout: Duration::from_secs(1),
            },
            &Cancellation::default(),
        )
        .await
        .unwrap();
    assert_eq!(reply.status, 200);
    let received = server.await.unwrap();
    assert!(
        received
            .to_lowercase()
            .contains("authorization: bearer synthetic-private-hub-token")
    );
    assert!(
        !received
            .lines()
            .next()
            .unwrap()
            .contains("synthetic-private-hub-token")
    );
}
#[tokio::test]
async fn relay_get_timeout_is_not_reported_as_a_pending_spawn_confirmation() {
    let f = fixture();
    *f.io.response.lock().unwrap() = Err(RequestError::Timeout);
    let error = f
        .cli
        .run(&args(&["relay", "status"]), &Cancellation::default())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert!(!error.to_string().contains("spawn confirmation pending"));
}
