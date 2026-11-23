use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::mock_agent::{MockConfig, connect};
use super::*;

const WAIT: Duration = Duration::from_secs(20);

async fn next(events: &mut AcpEvents) -> AcpEvent {
    tokio::time::timeout(WAIT, events.recv())
        .await
        .expect("event in time")
        .expect("event stream open")
}

async fn next_permission(events: &mut AcpEvents) -> PermissionRequest {
    loop {
        if let AcpEvent::PermissionRequest(request) = next(events).await {
            return request;
        }
    }
}

fn chunk_text(event: &AcpEvent) -> Option<String> {
    match event {
        AcpEvent::Update { update, .. } if update["sessionUpdate"] == "agent_message_chunk" => {
            update["content"]["text"].as_str().map(str::to_owned)
        }
        _ => None,
    }
}

async fn ready(
    config: MockConfig,
) -> (AcpClient, AcpEvents, super::mock_agent::MockHandle, String) {
    let (client, events, mock) = connect(config);
    let init = client
        .initialize("test-client", "0")
        .await
        .expect("initialize");
    assert_eq!(init.protocol_version, 1);
    let session = client.new_session("/work").await.expect("session/new");
    (client, events, mock, session)
}

#[test]
fn decoder_reassembles_splits_and_merges() {
    let mut decoder = LineDecoder::default();
    assert!(decoder.push(b"{\"a\":").unwrap().is_empty());
    let lines = decoder.push(b"1}\r\n{\"b\":2}\n\n{\"c\"").unwrap();
    assert_eq!(lines, vec![b"{\"a\":1}".to_vec(), b"{\"b\":2}".to_vec()]);
    assert_eq!(decoder.push(b":3}\n").unwrap(), vec![b"{\"c\":3}".to_vec()]);
}

#[test]
fn decoder_refuses_an_oversized_line() {
    let mut decoder = LineDecoder::default();
    let chunk = vec![b'x'; 1024 * 1024];
    let mut failed = false;
    for _ in 0..=(MAX_LINE_BYTES / chunk.len()) {
        if decoder.push(&chunk).is_err() {
            failed = true;
            break;
        }
    }
    assert!(failed);
}

#[test]
fn encoded_message_is_one_line() {
    let bytes = encode_message(&json!({"text": "a\nb"}));
    assert_eq!(bytes.iter().filter(|b| **b == b'\n').count(), 1);
    assert_eq!(*bytes.last().unwrap(), b'\n');
}

#[tokio::test]
async fn initialize_reports_capabilities_and_auth_methods() {
    let (client, _events, mock) = connect(MockConfig::default());
    let init = client.initialize("test-client", "0").await.unwrap();
    assert!(init.capabilities.load_session);
    assert!(init.capabilities.list_sessions);
    assert!(init.capabilities.resume_session);
    assert!(init.capabilities.close_session);
    assert!(init.capabilities.image);
    assert!(!init.capabilities.audio);
    assert_eq!(init.agent_name.as_deref(), Some("mock-agent"));
    assert_eq!(init.auth_methods.len(), 2);
    assert_eq!(init.auth_methods[0].kind, "agent");
    assert_eq!(init.auth_methods[1].kind, "terminal");
    // The client advertises nothing the agent could use to reach the machine.
    let sent = mock.find(|m| m["method"] == "initialize").unwrap();
    assert_eq!(sent["params"]["protocolVersion"], 1);
    assert_eq!(sent["params"]["clientCapabilities"], json!({}));
}

#[tokio::test]
async fn prompt_streams_updates_before_the_turn_ends() {
    let (client, mut events, _mock, session) = ready(MockConfig::default()).await;
    let stop = client
        .prompt(&session, vec![text_block("plain")])
        .await
        .unwrap();
    assert_eq!(stop, "end_turn");
    assert_eq!(
        chunk_text(&events.try_recv().unwrap()).as_deref(),
        Some("Hello ")
    );
    assert_eq!(
        chunk_text(&events.try_recv().unwrap()).as_deref(),
        Some("world")
    );
}

#[tokio::test]
async fn messages_split_in_the_middle_still_work() {
    let (client, mut events, _mock, session) = ready(MockConfig {
        fragment: true,
        ..Default::default()
    })
    .await;
    assert_eq!(
        client
            .prompt(&session, vec![text_block("plain")])
            .await
            .unwrap(),
        "end_turn"
    );
    assert_eq!(
        chunk_text(&events.try_recv().unwrap()).as_deref(),
        Some("Hello ")
    );
}

#[tokio::test]
async fn permission_is_only_answered_by_the_caller() {
    let (client, mut events, mock, session) = ready(MockConfig::default()).await;
    let prompt = tokio::spawn({
        let client = client.clone();
        let session = session.clone();
        async move {
            client
                .prompt(&session, vec![text_block("permission")])
                .await
        }
    });
    let request = next_permission(&mut events).await;
    assert_eq!(request.session_id, session);
    assert_eq!(request.options.len(), 2);
    assert_eq!(request.tool_call["toolCallId"], "call_001");
    // Nothing answers it on its own.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        mock.find(|m| m["id"] == 900 && m.get("result").is_some())
            .is_none()
    );
    assert!(!prompt.is_finished());
    // An option the agent did not offer is refused and the request stays open.
    let wrong = client
        .answer_permission(
            request.request_key,
            PermissionDecision::Select("allow-always".into()),
        )
        .await;
    assert!(matches!(wrong, Err(AcpError::Invalid(_))));
    client
        .answer_permission(
            request.request_key,
            PermissionDecision::Select("allow-once".into()),
        )
        .await
        .unwrap();
    assert_eq!(prompt.await.unwrap().unwrap(), "end_turn");
    let answer = mock
        .find(|m| m["id"] == 900 && m.get("result").is_some())
        .unwrap();
    assert_eq!(
        answer["result"]["outcome"],
        json!({"outcome":"selected","optionId":"allow-once"})
    );
    // The same request cannot be answered twice.
    let again = client
        .answer_permission(request.request_key, PermissionDecision::Cancelled)
        .await;
    assert!(matches!(again, Err(AcpError::Invalid(_))));
    let mut saw_completed = false;
    while let Some(event) = events.try_recv() {
        if let AcpEvent::Update { update, .. } = event {
            saw_completed |= update["status"] == "completed";
        }
    }
    assert!(saw_completed);
}

#[tokio::test]
async fn a_rejection_reaches_the_agent() {
    let (client, mut events, mock, session) = ready(MockConfig::default()).await;
    let prompt = tokio::spawn({
        let client = client.clone();
        let session = session.clone();
        async move {
            client
                .prompt(&session, vec![text_block("permission")])
                .await
        }
    });
    let request = next_permission(&mut events).await;
    client
        .answer_permission(
            request.request_key,
            PermissionDecision::Select("reject-once".into()),
        )
        .await
        .unwrap();
    assert_eq!(prompt.await.unwrap().unwrap(), "end_turn");
    let answer = mock
        .find(|m| m["id"] == 900 && m.get("result").is_some())
        .unwrap();
    assert_eq!(answer["result"]["outcome"]["optionId"], "reject-once");
}

#[tokio::test]
async fn cancel_ends_the_turn_and_closes_open_permission_requests() {
    let (client, mut events, mock, session) = ready(MockConfig::default()).await;
    let prompt = tokio::spawn({
        let client = client.clone();
        let session = session.clone();
        async move {
            client
                .prompt(&session, vec![text_block("permission")])
                .await
        }
    });
    let request = next_permission(&mut events).await;
    client.cancel(&session).await.unwrap();
    assert_eq!(prompt.await.unwrap().unwrap(), "cancelled");
    let answer = mock
        .find(|m| m["id"] == 900 && m.get("result").is_some())
        .unwrap();
    assert_eq!(answer["result"]["outcome"], json!({"outcome":"cancelled"}));
    assert!(mock.methods().contains(&"session/cancel".to_string()));
    loop {
        if let AcpEvent::PermissionClosed { request_key } = next(&mut events).await {
            assert_eq!(request_key, request.request_key);
            break;
        }
    }
}

#[tokio::test]
async fn cancel_stops_a_running_turn() {
    let (client, _events, _mock, session) = ready(MockConfig::default()).await;
    let prompt = tokio::spawn({
        let client = client.clone();
        let session = session.clone();
        async move { client.prompt(&session, vec![text_block("hang")]).await }
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!prompt.is_finished());
    client.cancel(&session).await.unwrap();
    assert_eq!(prompt.await.unwrap().unwrap(), "cancelled");
}

#[tokio::test]
async fn abnormal_exit_finishes_every_pending_request() {
    let (client, mut events, _mock, session) = ready(MockConfig::default()).await;
    let waiting: Vec<_> = (0..3)
        .map(|_| {
            let client = client.clone();
            let session = session.clone();
            tokio::spawn(async move { client.prompt(&session, vec![text_block("hang")]).await })
        })
        .collect();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let crash = client.prompt(&session, vec![text_block("crash")]).await;
    assert!(matches!(crash, Err(AcpError::Closed(_))), "{crash:?}");
    for task in waiting {
        let result = tokio::time::timeout(WAIT, task)
            .await
            .expect("finished")
            .unwrap();
        assert!(matches!(result, Err(AcpError::Closed(_))), "{result:?}");
    }
    assert!(client.is_closed());
    // The partial output arrived, then the last event says the connection ended.
    let mut last = None;
    while let Ok(Some(event)) =
        tokio::time::timeout(Duration::from_millis(500), events.recv()).await
    {
        last = Some(event);
    }
    assert!(matches!(last, Some(AcpEvent::Exited(reason)) if reason.contains("crashed")));
    // Nothing is retried or resent: new requests fail at once.
    assert!(matches!(
        client.new_session("/work").await,
        Err(AcpError::Closed(_))
    ));
    assert!(matches!(
        client.cancel(&session).await,
        Err(AcpError::Closed(_))
    ));
}

#[tokio::test]
async fn missing_sign_in_is_reported_as_auth_required() {
    let (client, _events, _mock) = connect(MockConfig {
        auth_required: true,
        ..Default::default()
    });
    client.initialize("test-client", "0").await.unwrap();
    let error = client.new_session("/work").await.unwrap_err();
    assert!(matches!(error, AcpError::AuthRequired { .. }), "{error:?}");
}

#[tokio::test]
async fn optional_calls_need_the_advertised_capability() {
    let (client, _events, _mock) = connect(MockConfig {
        bare_capabilities: true,
        ..Default::default()
    });
    assert!(matches!(
        client.list_sessions(None, None).await,
        Err(AcpError::Invalid(_))
    ));
    client.initialize("test-client", "0").await.unwrap();
    assert!(matches!(
        client.list_sessions(None, None).await,
        Err(AcpError::Unsupported(_))
    ));
    assert!(matches!(
        client.load_session("s", "/w").await,
        Err(AcpError::Unsupported(_))
    ));
    assert!(matches!(
        client.resume_session("s", "/w").await,
        Err(AcpError::Unsupported(_))
    ));
    assert!(matches!(
        client.close_session("s").await,
        Err(AcpError::Unsupported(_))
    ));
}

#[tokio::test]
async fn session_management_when_advertised() {
    let (client, mut events, mock, _session) = ready(MockConfig::default()).await;
    let list = client.list_sessions(Some("/work"), None).await.unwrap();
    assert_eq!(list.sessions.len(), 2);
    assert_eq!(list.sessions[0].title.as_deref(), Some("First"));
    assert_eq!(list.next_cursor.as_deref(), Some("page2"));
    let sent = mock.find(|m| m["method"] == "session/list").unwrap();
    assert_eq!(sent["params"], json!({"cwd": "/work"}));
    client.load_session("sess_a", "/work").await.unwrap();
    let mut saw_history = false;
    while let Some(event) = events.try_recv() {
        if let AcpEvent::Update { update, .. } = event {
            saw_history |= update["sessionUpdate"] == "user_message_chunk";
        }
    }
    assert!(saw_history);
    client.resume_session("sess_a", "/work").await.unwrap();
    client.close_session("sess_a").await.unwrap();
    // The protocol counts "unknown session" on close as closed.
    client.close_session("sess_gone").await.unwrap();
}

#[tokio::test]
async fn a_line_that_is_not_json_does_not_end_the_stream() {
    let (client, mut events, _mock, session) = ready(MockConfig::default()).await;
    assert_eq!(
        client
            .prompt(&session, vec![text_block("garbage")])
            .await
            .unwrap(),
        "end_turn"
    );
    assert!(matches!(next(&mut events).await, AcpEvent::Malformed(_)));
    assert_eq!(
        chunk_text(&next(&mut events).await).as_deref(),
        Some("after garbage")
    );
}

#[tokio::test]
async fn agent_requests_other_than_permission_get_method_not_found() {
    let (out_tx, mut out_rx) = mpsc::channel(8);
    let (in_tx, in_rx) = mpsc::channel(8);
    let (_client, mut events) = AcpClient::from_transport(out_tx, in_rx, None);
    let request =
        json!({"jsonrpc":"2.0","id":"x1","method":"fs/read_text_file","params":{"path":"/a"}});
    in_tx
        .send(Incoming::Bytes(encode_message(&request)))
        .await
        .unwrap();
    let reply: Value = serde_json::from_slice(&out_rx.recv().await.unwrap()).unwrap();
    assert_eq!(reply["id"], "x1");
    assert_eq!(reply["error"]["code"], -32601);
    // Unknown notifications are passed on; unmatched responses are reported.
    let note = json!({"jsonrpc":"2.0","method":"vendor/ping","params":{"a":1}});
    in_tx
        .send(Incoming::Bytes(encode_message(&note)))
        .await
        .unwrap();
    assert!(
        matches!(next(&mut events).await, AcpEvent::Notification { method, .. } if method == "vendor/ping")
    );
    let stray = json!({"jsonrpc":"2.0","id":77,"result":{}});
    in_tx
        .send(Incoming::Bytes(encode_message(&stray)))
        .await
        .unwrap();
    assert!(matches!(next(&mut events).await, AcpEvent::Malformed(_)));
}

#[tokio::test]
async fn an_unsupported_protocol_version_is_refused() {
    let (out_tx, mut out_rx) = mpsc::channel(8);
    let (in_tx, in_rx) = mpsc::channel(8);
    let (client, _events) = AcpClient::from_transport(out_tx, in_rx, None);
    let call = tokio::spawn({
        let client = client.clone();
        async move { client.initialize("test-client", "0").await }
    });
    let sent: Value = serde_json::from_slice(&out_rx.recv().await.unwrap()).unwrap();
    let reply = json!({"jsonrpc":"2.0","id":sent["id"],"result":{"protocolVersion":2}});
    in_tx
        .send(Incoming::Bytes(encode_message(&reply)))
        .await
        .unwrap();
    assert!(matches!(call.await.unwrap(), Err(AcpError::Protocol(_))));
}

#[tokio::test]
async fn a_closed_transport_finishes_pending_requests() {
    let (out_tx, mut out_rx) = mpsc::channel(8);
    let (in_tx, in_rx) = mpsc::channel(8);
    let (client, mut events) = AcpClient::from_transport(out_tx, in_rx, None);
    let call = tokio::spawn({
        let client = client.clone();
        async move { client.initialize("test-client", "0").await }
    });
    let _ = out_rx.recv().await.unwrap();
    drop(in_tx);
    assert!(matches!(call.await.unwrap(), Err(AcpError::Closed(_))));
    assert!(matches!(next(&mut events).await, AcpEvent::Exited(_)));
}

#[tokio::test]
async fn a_dead_input_pipe_finishes_the_request() {
    let (out_tx, out_rx) = mpsc::channel(8);
    let (_in_tx, in_rx) = mpsc::channel(8);
    drop(out_rx);
    let (client, _events) = AcpClient::from_transport(out_tx, in_rx, None);
    assert!(matches!(
        client.initialize("test-client", "0").await,
        Err(AcpError::Closed(_))
    ));
}

#[test]
fn only_allow_listed_variables_reach_the_child() {
    let ambient = [
        ("PATH", "/bin"),
        ("Path", "C:\\bin"),
        ("USERPROFILE", "/home/u"),
        ("MANY_AI_CLI", "1"),
        ("HUB_SECRET_TOKEN", "synthetic"),
        ("GEMINI_API_KEY", "synthetic"),
        ("GOOGLE_APPLICATION_CREDENTIALS", "/x"),
    ]
    .map(|(k, v)| (OsString::from(k), OsString::from(v)));
    let kept = minimal_env(ambient);
    let names: Vec<String> = kept
        .keys()
        .map(|k| k.to_string_lossy().to_uppercase())
        .collect();
    assert_eq!(names.len(), 3, "{names:?}");
    assert!(names.iter().all(|n| n == "PATH" || n == "USERPROFILE"));
}

fn exit_immediately() -> AcpProcessSpec {
    #[cfg(windows)]
    let (executable, args): (PathBuf, Vec<OsString>) = (
        PathBuf::from(std::env::var_os("SystemRoot").expect("Windows directory"))
            .join("System32")
            .join("cmd.exe"),
        vec!["/D".into(), "/C".into(), "exit 3".into()],
    );
    #[cfg(unix)]
    let (executable, args): (PathBuf, Vec<OsString>) =
        (PathBuf::from("/bin/sh"), vec!["-c".into(), "exit 3".into()]);
    AcpProcessSpec {
        executable,
        args,
        cwd: std::env::temp_dir(),
    }
}

#[tokio::test]
async fn a_child_that_exits_finishes_the_request() {
    let (client, mut events) = spawn_process(exit_immediately()).unwrap();
    let result = tokio::time::timeout(WAIT, client.initialize("test-client", "0"))
        .await
        .expect("finished");
    assert!(matches!(result, Err(AcpError::Closed(_))), "{result:?}");
    loop {
        if let AcpEvent::Exited(_) = next(&mut events).await {
            break;
        }
    }
}

#[tokio::test]
async fn a_missing_executable_finishes_the_request() {
    let spec = AcpProcessSpec {
        executable: std::env::temp_dir().join("no-such-acp-agent-executable"),
        args: vec![],
        cwd: std::env::temp_dir(),
    };
    let (client, _events) = spawn_process(spec).unwrap();
    let result = tokio::time::timeout(WAIT, client.initialize("test-client", "0"))
        .await
        .expect("finished");
    assert!(matches!(result, Err(AcpError::Closed(_))), "{result:?}");
}

#[tokio::test]
async fn a_launch_that_approves_tools_by_itself_is_refused() {
    let mut spec = exit_immediately();
    spec.args.push("--dangerously-skip-permissions".into());
    let error = spawn_process(spec).err().expect("refused");
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    // The same flag inside one `cmd.exe /c` string (a shim wrapped) is refused too.
    let mut spec = exit_immediately();
    spec.args
        .push("\"synthetic-agent.cmd\" --acp --dangerously-skip-permissions".into());
    let error = spawn_process(spec).err().expect("refused");
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
}

/// The client talks protocol only. Nothing in this directory may name a
/// credential file, a sign-in store or token handling. This file is excluded
/// because it holds the list itself; comments are not scanned.
#[test]
fn acp_sources_do_not_touch_credentials() {
    const FORBIDDEN: &[&str] = &[
        "token",
        "credential",
        "oauth_creds",
        "keyring",
        "keychain",
        "secret",
        "password",
        "api_key",
        "apikey",
        "bearer",
        "authorization",
        ".gemini",
        "dpapi",
        "refresh",
    ];
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("acp");
    let mut scanned = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs")
            || path.file_name().and_then(|n| n.to_str()) == Some("tests.rs")
        {
            continue;
        }
        scanned += 1;
        let text = std::fs::read_to_string(&path).unwrap();
        for (number, line) in text.lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            let lower = line.to_lowercase();
            for word in FORBIDDEN {
                assert!(
                    !lower.contains(word),
                    "{}:{} mentions `{word}`",
                    path.display(),
                    number + 1
                );
            }
        }
    }
    assert!(scanned >= 5, "scanned only {scanned} files");
}
