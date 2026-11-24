//! Exercise the normal outbound synchronizer with an isolated MCP endpoint and
//! the real session engine. Only the provider PTY and the board are simulated.
use super::*;
use many_ai_cli::{
    application::board_sync::BoardSync, approval::marker, hub::task_owner::HubTaskOwner,
};
use serde_json::{Value, json};
use tokio::sync::mpsc;

struct BoardHarness {
    calls: mpsc::UnboundedSender<(String, Value)>,
}

async fn serve_board(
    axum::extract::State(state): axum::extract::State<Arc<BoardHarness>>,
    headers: axum::http::HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> axum::Json<Value> {
    let tool = body["params"]["name"].as_str().unwrap();
    let args = body["params"]["arguments"].clone();
    let granted =
        headers.get("authorization").unwrap().to_str().unwrap() == "Bearer fixture-device";
    let outcome = match tool {
        "iris_request_confirmation" => "pending",
        "iris_confirmation_status" => "approved",
        "iris_close_confirmation" => args["outcome"].as_str().unwrap(),
        _ => panic!("unexpected tool"),
    };
    let result = json!({"ok":granted,"request_id":args["request_id"],"state":outcome});
    state.calls.send((tool.into(), args)).unwrap();
    axum::Json(json!({"jsonrpc":"2.0","id":1,"result":{"structuredContent":result}}))
}

async fn start_fixture(
    f: &Fixture,
    root: &std::path::Path,
    local: &std::path::Path,
    device_key: &std::path::Path,
    url: &str,
) -> (HubTaskOwner, SessionBinding) {
    std::fs::write(root.join("ordinary.key"), "fixture-ordinary").unwrap();
    let config = json!({"schema_version":1, "url":url,
    "key_file":root.join("ordinary.key"), "repos":[
        {"path":local,"project":"iris-fixture","confirmation_key_file":device_key}
    ]});
    std::fs::write(root.join("board.json"), config.to_string()).unwrap();

    let registered = f
        .engine
        .register(
            RegisterRequest {
                message: proto::Message {
                    provider: "copilot".into(),
                    cwd: local.to_string_lossy().into_owned(),
                    pid: 7,
                    cols: 120,
                    rows: 30,
                    ..Default::default()
                },
                spawn_proof: None,
            },
            WrapperConnectionId(1),
            now(),
        )
        .await
        .unwrap();
    let binding = registered.binding;
    f.sink.apply(registered.after_registered).await.unwrap();
    let block = format!(
        "{}\nRead the synthetic fixture? (Y:1/N:0)\n{}",
        marker::OPEN,
        marker::CLOSE
    );
    f.sink
        .apply(
            f.engine
                .observe_transcript_marker(binding, marker::extract(&block), now())
                .unwrap(),
        )
        .await
        .unwrap();

    let engine = f.engine.clone();
    *f.transport.on_send.lock().unwrap() = Some(Arc::new(move |bound, message| {
        let engine = engine.clone();
        Box::pin(async move {
            engine.acknowledge(bound, InputSeq(message.input_seq));
        })
    }));
    let tasks = HubTaskOwner::new(tokio::runtime::Handle::current());
    let board = BoardSync::new(root, false, tasks.handle(), Arc::new(|_, _| {})).unwrap();
    board.start_confirmations(Arc::downgrade(&f.engine));
    (tasks, binding)
}

#[tokio::test]
async fn iris_confirmation_round_trip_over_the_device_channel() {
    let f = fixture();
    let root = tempfile::tempdir().unwrap();
    let local = root.path().join("project");
    std::fs::create_dir(&local).unwrap();
    let device_key = root.path().join("device.key");
    std::fs::write(&device_key, "fixture-device").unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (calls, mut observed) = mpsc::unbounded_channel();
    let app = axum::Router::new()
        .fallback(axum::routing::post(serve_board))
        .with_state(Arc::new(BoardHarness { calls }));
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (tasks, binding) = start_fixture(
        &f,
        root.path(),
        &local,
        &device_key,
        &format!("http://{addr}/board/mcp"),
    )
    .await;
    tokio::time::timeout(Duration::from_secs(35), async {
        let (tool, registered) = observed.recv().await.unwrap();
        assert_eq!(tool, "iris_request_confirmation");
        assert_eq!(registered["project"], "iris-fixture");
        assert_eq!(registered["question"], "Read the synthetic fixture?");
        assert_eq!(registered["local_only"], false);
        assert_eq!(registered["request_id"].as_str().unwrap().len(), 32);
        let (tool, read) = observed.recv().await.unwrap();
        assert_eq!(tool, "iris_confirmation_status");
        assert_eq!(read["request_id"], registered["request_id"]);
        let (tool, closed) = observed.recv().await.unwrap();
        assert_eq!(tool, "iris_close_confirmation");
        assert_eq!(closed["outcome"], "applied");
        assert_eq!(closed["request_id"], registered["request_id"]);
    })
    .await
    .expect("round trip through the board");
    let frames = f.transport.frames.lock().unwrap();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].0, binding);
    assert_eq!(frames[0].1.data, b"1\r");
    drop(frames);
    tasks.stop_requests();
    tasks.stop_effects().unwrap();
    tasks.abort_effects().unwrap();
    server.abort();
}

/// Run with an isolated workerd and its owner's browser open. Only a localhost
/// port and a fixture key file can be supplied; the normal Hub never starts.
#[tokio::test]
#[ignore = "requires an isolated Many Iris worker and its owner approval"]
async fn iris_confirmation_real_worker_round_trip() {
    let port: u16 = std::env::var("IRIS_FIXTURE_PORT").unwrap().parse().unwrap();
    assert_ne!(port, 0);
    let key = std::path::PathBuf::from(std::env::var_os("IRIS_FIXTURE_KEY_FILE").unwrap());
    assert!(key.is_absolute());
    let root = tempfile::tempdir().unwrap();
    let local = root.path().join("project");
    std::fs::create_dir(&local).unwrap();
    let f = fixture();
    let (tasks, binding) = start_fixture(
        &f,
        root.path(),
        &local,
        &key,
        &format!("http://127.0.0.1:{port}/board/mcp"),
    )
    .await;
    let path = root.path().join("iris-confirmation-receipts.json");
    tokio::time::timeout(Duration::from_secs(180), async {
        loop {
            let applied = !f.transport.frames.lock().unwrap().is_empty();
            let acknowledged = std::fs::read(&path)
                .ok()
                .and_then(|buf| serde_json::from_slice::<Value>(&buf).ok())
                .is_some_and(|r| r.as_object().is_some_and(|obj| obj.is_empty()));
            if applied && acknowledged {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("owner decision, PTY acknowledgement and board receipt");
    let frames = f.transport.frames.lock().unwrap();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].0, binding);
    assert_eq!(frames[0].1.data, b"1\r");
    drop(frames);
    tasks.stop_requests();
    tasks.stop_effects().unwrap();
    tasks.abort_effects().unwrap();
}
