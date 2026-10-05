use super::*;
use serde::Deserialize;
use std::{io::Cursor, sync::Mutex};
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Case {
    name: String,
    provider: String,
    input: String,
    rollout: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Golden {
    name: String,
    stdout: String,
    payload: Option<Value>,
}
fn now() -> Timestamp {
    crate::proto::time::parse_rfc3339("2026-10-05T03:00:00Z").unwrap()
}
struct FakeIo {
    rollout: Vec<u8>,
    posts: Mutex<Vec<Value>>,
    fail_post: bool,
}
impl RelayIo for FakeIo {
    fn now(&self) -> Timestamp {
        now()
    }
    fn rollout(&self, _: &str) -> io::Result<Box<dyn BufRead + Send>> {
        Ok(Box::new(Cursor::new(self.rollout.clone())))
    }
    fn post<'a>(
        &'a self,
        hub: &'a str,
        token: &'a str,
        payload: &'a Value,
    ) -> CoreFuture<'a, io::Result<()>> {
        Box::pin(async move {
            assert_eq!(hub, "http://127.0.0.1:1");
            assert_eq!(token, "synthetic");
            self.posts.lock().unwrap().push(payload.clone());
            if self.fail_post {
                Err(io::Error::other("synthetic failure"))
            } else {
                Ok(())
            }
        })
    }
}
fn args(provider: &str) -> Vec<String> {
    vec![
        "--provider".into(),
        provider.into(),
        "--hub=http://127.0.0.1:1".into(),
        "--session=7".into(),
    ]
}
#[tokio::test]
async fn pinned_go_52_input_and_rollout_cases() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("cases.json")).unwrap();
    let golden: Vec<Golden> = serde_json::from_str(include_str!("golden.json")).unwrap();
    assert_eq!(cases.len(), 52);
    assert_eq!(cases.len(), golden.len());
    for (case, mut expected) in cases.iter().zip(golden) {
        assert_eq!(case.name, expected.name);
        let io = FakeIo {
            rollout: case.rollout.as_bytes().to_vec(),
            posts: Mutex::new(vec![]),
            fail_post: false,
        };
        let input = if case.provider == "codex" {
            r#"{"model":"gpt-test","transcript_path":"TRIAL_TRANSCRIPT"}"#
        } else {
            &case.input
        };
        let mut output = vec![];
        run_with_io(
            &args(&case.provider),
            &["MANY_AI_CLI_HUB_TOKEN=synthetic".into()],
            &mut Cursor::new(input),
            &mut output,
            &io,
            &|_| {},
        )
        .await
        .unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            expected.stdout,
            "{} stdout",
            case.name
        );
        let posts = io.posts.lock().unwrap();
        let mut actual = posts.first().cloned();
        if let Some(value) = actual.as_mut() {
            let fields = value.as_object_mut().unwrap();
            assert_eq!(
                fields.remove("started_at"),
                Some(Value::String(
                    crate::proto::time::format_rfc3339(now()).unwrap()
                )),
                "{} started_at",
                case.name
            );
            if case.provider == "claude" {
                assert_eq!(
                    fields.remove("usage_observed_at"),
                    Some(Value::String("2026-10-05T03:00:00Z".into())),
                    "{} received_at",
                    case.name
                );
            }
        }
        // JSON numbers have no int/float type. Normalize only Go-declared
        // float64 fields so integer counters retain their full 64-bit precision.
        if let Some(Value::Object(fields)) = expected.payload.as_mut() {
            use crate::proto::wire::GoWire;
            for field in UsageRequest::SCHEMAS[0].fields {
                if field.kind == "float64"
                    && let Some(value) = fields.get_mut(field.name)
                {
                    *value = Value::from(value.as_f64().unwrap());
                }
            }
        }
        assert_eq!(actual, expected.payload, "{} payload", case.name);
        assert!(posts.len() <= 1);
    }
}
#[tokio::test]
async fn statusline_and_success_exit_survive_post_failure_without_body_leak() {
    let io = FakeIo {
        rollout: vec![],
        posts: Mutex::new(vec![]),
        fail_post: true,
    };
    let warnings = Mutex::new(vec![]);
    let mut output = vec![];
    run_with_io(&args("claude"),&["MANY_AI_CLI_HUB_TOKEN=synthetic".into()],&mut Cursor::new(br#"{"prompt":"PRIVATE_BODY_SENTINEL","cost":{"total_cost_usd":2},"model":{"display_name":"Example"}}"#),&mut output,&io,&|v|warnings.lock().unwrap().push(v)).await.unwrap();
    assert_eq!(output, b"$2.0000  Example  \xe2\x86\x910 \xe2\x86\x930\n");
    assert_eq!(*warnings.lock().unwrap(), vec!["usage POST failed"]);
    assert!(
        !io.posts.lock().unwrap()[0]
            .to_string()
            .contains("PRIVATE_BODY_SENTINEL")
    );
}
#[tokio::test]
async fn missing_auth_keeps_claude_status_and_never_posts() {
    let io = FakeIo {
        rollout: vec![],
        posts: Mutex::new(vec![]),
        fail_post: false,
    };
    let mut output = vec![];
    run_with_io(
        &args("claude"),
        &[],
        &mut Cursor::new(b"{}"),
        &mut output,
        &io,
        &|_| {},
    )
    .await
    .unwrap();
    assert!(!output.is_empty());
    assert!(io.posts.lock().unwrap().is_empty());
}
#[test]
fn flag_integer_source_and_environment_token_precedence() {
    let warnings = Mutex::new(vec![]);
    let parsed = parse(
        &[
            "--provider=codex".into(),
            "--session=0x_7".into(),
            "--token=deprecated".into(),
        ],
        &["MANY_AI_CLI_HUB_TOKEN=synthetic".into()],
        &|v| warnings.lock().unwrap().push(v),
    )
    .unwrap();
    assert_eq!(parsed.session, 7);
    assert_eq!(parsed.token, "synthetic");
    assert!(warnings.lock().unwrap().is_empty());
    for (raw, expected) in [
        ("0", Some(0)),
        ("077", Some(63)),
        ("0o_7", Some(7)),
        ("-9223372036854775808", Some(i64::MIN)),
        ("9223372036854775808", None),
        ("09", None),
        ("7_", None),
        ("_7", None),
        ("0x__7", None),
    ] {
        assert_eq!(integer(raw), expected, "{raw}");
    }
}
#[test]
fn loopback_allowlist_does_not_canonicalize_unapproved_host_spellings() {
    for raw in [
        "http://127.0.0.1:1",
        "https://localhost",
        "http://[::1]:3/base",
        "HTTP://localhost",
    ] {
        assert!(validate_hub_url(raw).is_ok(), "{raw}");
    }
    for raw in [
        "http://LOCALHOST",
        "http://127.1",
        "http://2130706433",
        "http://127.0.0.2",
        "http://example.test",
        "file://localhost/",
        " http://localhost",
    ] {
        assert!(validate_hub_url(raw).is_err(), "{raw}");
    }
}
#[tokio::test]
async fn native_trial_port_and_redirect_never_reach_another_hub() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let primary = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let other = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = primary.local_addr().unwrap().port();
    let other_port = other.local_addr().unwrap().port();
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("trial")).unwrap();
    let paths = RuntimePaths::trial(
        &root.path().join("trial"),
        port,
        &root.path().join("installed"),
    )
    .unwrap();
    let io = NativeRelayIo::new(Some(paths));
    assert!(
        io.validate_target(&format!("http://127.0.0.1:{other_port}"))
            .is_err()
    );
    let server = tokio::spawn(async move {
        let (mut socket, _) = primary.accept().await.unwrap();
        let mut request = vec![];
        let mut chunk = [0u8; 2048];
        while !request.windows(4).any(|v| v == b"\r\n\r\n") {
            let n = socket.read(&mut chunk).await.unwrap();
            assert!(n > 0);
            request.extend_from_slice(&chunk[..n]);
        }
        // Synthetic credential is checked inside the task and never printed.
        assert!(
            String::from_utf8_lossy(&request)
                .to_ascii_lowercase()
                .contains("authorization: bearer synthetic")
        );
        socket.write_all(format!("HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{other_port}/other\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
    });
    let posted = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        io.post(
            &format!("http://127.0.0.1:{port}"),
            "synthetic",
            &serde_json::json!({"provider":"claude","session_id":7,"cost_from_relay":true}),
        ),
    )
    .await
    .unwrap();
    assert!(posted.is_err());
    server.await.unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), other.accept())
            .await
            .is_err()
    );
}
