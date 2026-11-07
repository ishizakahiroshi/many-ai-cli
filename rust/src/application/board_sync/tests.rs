use super::*;
use crate::hub::task_owner::HubTaskOwner;
use std::sync::atomic::AtomicUsize;

const KEY: &str = "synthetic-writer-key-0123456789abcdef";

struct Board {
    calls: AtomicUsize,
    changed: AtomicBool,
    seen: Mutex<Vec<(String, String, serde_json::Value)>>,
}
async fn receive(
    axum::extract::State(state): axum::extract::State<Arc<Board>>,
    headers: axum::http::HeaderMap,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::Json<serde_json::Value> {
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let tool = body["params"]["name"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    lock(&state.seen).push((auth, tool, body["params"]["arguments"].clone()));
    state.calls.fetch_add(1, Ordering::SeqCst);
    let changed = state.changed.load(Ordering::SeqCst);
    axum::Json(serde_json::json!({
        "jsonrpc": "2.0", "id": 1,
        "result": {"structuredContent": {"ok": true, "changed": changed}},
    }))
}

struct Fixture {
    sync: Arc<BoardSync>,
    _owner: HubTaskOwner,
    board: Arc<Board>,
    root: tempfile::TempDir,
    repo: PathBuf,
    url: String,
    warnings: Arc<Mutex<Vec<String>>>,
    cancel: crate::process::Cancellation,
}
async fn fixture(trial: bool) -> Fixture {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let board = Arc::new(Board {
        calls: 0.into(),
        changed: true.into(),
        seen: Mutex::default(),
    });
    let app = axum::Router::new()
        .fallback(axum::routing::post(receive))
        .with_state(board.clone());
    let cancel = crate::process::Cancellation::default();
    let stopping = cancel.clone();
    tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move { stopping.cancelled().await })
            .await
            .unwrap();
    });
    let root = tempfile::tempdir().unwrap();
    let repo = root.path().join("repos").join("demo-repo");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(root.path().join("writer.key"), format!("{KEY}\n")).unwrap();
    let owner = HubTaskOwner::new(tokio::runtime::Handle::current());
    let warnings = Arc::new(Mutex::new(Vec::new()));
    let sink = warnings.clone();
    let sync = BoardSync::new(
        root.path(),
        trial,
        owner.handle(),
        Arc::new(move |operation: &str, detail: &str| {
            lock(&sink).push(format!("{operation}: {detail}"))
        }),
    )
    .unwrap();
    Fixture {
        sync,
        _owner: owner,
        board,
        url: format!("http://{address}/board/mcp"),
        root,
        repo,
        warnings,
        cancel,
    }
}
impl Fixture {
    fn write_settings(&self, url: &str, repos: serde_json::Value) {
        let settings = serde_json::json!({
            "schema_version": 1,
            "url": url,
            "key_file": self.root.path().join("writer.key"),
            "repos": repos,
        });
        std::fs::write(self.root.path().join(FILE_NAME), settings.to_string()).unwrap();
    }
    fn list_repo(&self) {
        self.write_settings(
            &self.url,
            serde_json::json!([{"path": self.repo, "project": "demo"}]),
        );
    }
    fn seen(&self) -> Vec<(String, String, serde_json::Value)> {
        lock(&self.board.seen).clone()
    }
}
fn details(id: i64, cwd: &Path, db: Option<i64>) -> SessionDetails {
    let binding = SessionBinding {
        session: LiveSessionId(id),
        incarnation: SessionIncarnation(1),
        wrapper: WrapperConnectionId(1),
    };
    SessionDetails {
        binding,
        last_output_at: None,
        snapshot: SessionSnapshot {
            provider: "claude".into(),
            cwd: cwd.to_string_lossy().into_owned(),
            label: "synthetic task".into(),
            ..Default::default()
        },
        db_id: db.map(DbSessionId),
        git_root: None,
        transcript: TranscriptSessionIdentity {
            started_at: "2026-01-02T03:04:05Z".into(),
            ..Default::default()
        },
        approval: ApprovalSessionSnapshot {
            session: binding.session,
            version: ApprovalStateVersion(0),
            record: None,
        },
        workflow: None,
        subagents: None,
        done: None,
        connected: true,
    }
}
fn done(id: i64, text: &str) -> DoneSummary {
    DoneSummary {
        session_id: id,
        text: text.into(),
        kind: "success".into(),
        ..Default::default()
    }
}
async fn wait_calls(board: &Board, count: usize) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while board.calls.load(Ordering::SeqCst) < count {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}
async fn settle() {
    tokio::time::sleep(Duration::from_millis(200)).await;
}

#[tokio::test]
async fn without_settings_nothing_is_sent() {
    let f = fixture(false).await;
    let d = details(1, &f.repo, Some(10));
    f.sync.session_started(&d);
    f.sync.turn_done(Some(&d), &done(1, "summary"));
    f.sync.session_ended(LiveSessionId(1), "exited");
    settle().await;
    assert_eq!(f.board.calls.load(Ordering::SeqCst), 0);
    assert!(lock(&f.warnings).is_empty());
    f.cancel.cancel();
}

#[tokio::test]
async fn listed_repository_writes_start_summary_and_end_in_order_with_the_key() {
    let f = fixture(false).await;
    f.list_repo();
    let d = details(3, &f.repo.join("src"), Some(42));
    f.sync.session_started(&d);
    f.sync.session_started(&d); // reattach: no second announcement
    f.sync.turn_done(
        Some(&d),
        &done(3, "wrote the parser API_TOKEN=abc123secretvalue"),
    );
    f.sync.session_ended(LiveSessionId(3), "exited");
    f.sync.session_ended(LiveSessionId(3), "exited");
    wait_calls(&f.board, 5).await;
    settle().await;
    let seen = f.seen();
    let tools: Vec<_> = seen
        .iter()
        .map(|(_, t, a)| {
            format!(
                "{t}:{}",
                a["kind"].as_str().or(a["state"].as_str()).unwrap_or("")
            )
        })
        .collect();
    assert_eq!(
        tools,
        [
            "board_upsert_item:task",
            "board_add_event:session_start",
            "board_add_event:note",
            "board_upsert_item:done",
            "board_add_event:session_end",
        ]
    );
    assert!(
        seen.iter()
            .all(|(auth, _, _)| auth == &format!("Bearer {KEY}"))
    );
    assert!(seen.iter().all(|(_, _, a)| a["item_id"] == "session/42"));
    assert_eq!(seen[0].2["project"], "demo");
    assert_eq!(seen[0].2["title"], "claude: synthetic task");
    assert_eq!(seen[0].2["state"], "in_progress");
    let note = seen[2].2["text"].as_str().unwrap();
    assert!(note.starts_with("[完了] wrote the parser"), "{note}");
    assert!(
        !note.contains("abc123secretvalue"),
        "secrets are masked: {note}"
    );
    assert!(seen[4].2["text"].as_str().unwrap().contains("exited"));
    assert!(lock(&f.warnings).is_empty(), "{:?}", lock(&f.warnings));
    f.cancel.cancel();
}

#[tokio::test]
async fn other_repositories_are_not_sent() {
    let f = fixture(false).await;
    f.list_repo();
    let elsewhere = f.root.path().join("repos").join("demo-repo-other");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let d = details(4, &elsewhere, Some(11));
    f.sync.session_started(&d);
    f.sync.turn_done(Some(&d), &done(4, "summary"));
    f.sync.session_ended(LiveSessionId(4), "exited");
    settle().await;
    assert_eq!(f.board.calls.load(Ordering::SeqCst), 0);
    f.cancel.cancel();
}

#[tokio::test]
async fn trial_runtime_never_sends() {
    let f = fixture(true).await;
    f.list_repo();
    let d = details(5, &f.repo, Some(12));
    f.sync.session_started(&d);
    f.sync.turn_done(Some(&d), &done(5, "summary"));
    settle().await;
    assert_eq!(f.board.calls.load(Ordering::SeqCst), 0);
    f.cancel.cancel();
}

#[tokio::test]
async fn remote_plain_http_and_unknown_fields_are_refused_with_a_warning() {
    let f = fixture(false).await;
    f.write_settings(
        "http://board.example.com/board/mcp",
        serde_json::json!([{"path": f.repo}]),
    );
    let d = details(6, &f.repo, Some(13));
    f.sync.session_started(&d);
    settle().await;
    assert_eq!(f.board.calls.load(Ordering::SeqCst), 0);
    assert_eq!(lock(&f.warnings).len(), 1);
    let mut extra: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(f.root.path().join(FILE_NAME)).unwrap())
            .unwrap();
    extra["url"] = f.url.clone().into();
    extra["token"] = KEY.into();
    std::fs::write(f.root.path().join(FILE_NAME), extra.to_string()).unwrap();
    f.sync.session_started(&details(7, &f.repo, Some(14)));
    settle().await;
    assert_eq!(f.board.calls.load(Ordering::SeqCst), 0);
    assert!(lock(&f.warnings).iter().all(|w| !w.contains(KEY)));
    f.cancel.cancel();
}

#[tokio::test]
async fn summary_after_hub_restart_announces_once_without_duplicate_start_event() {
    let f = fixture(false).await;
    f.list_repo();
    f.board.changed.store(false, Ordering::SeqCst); // the item already exists unchanged
    let d = details(8, &f.repo, None);
    f.sync.turn_done(Some(&d), &done(8, "after restart"));
    wait_calls(&f.board, 2).await;
    settle().await;
    let seen = f.seen();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].1, "board_upsert_item");
    assert_eq!(seen[1].2["kind"], "note");
    assert_eq!(seen[0].2["item_id"], "session/live-20260102T030405Z-8");
    f.cancel.cancel();
}

#[tokio::test]
async fn missing_key_file_sends_nothing_and_warns() {
    let f = fixture(false).await;
    f.list_repo();
    std::fs::remove_file(f.root.path().join("writer.key")).unwrap();
    f.sync.session_started(&details(9, &f.repo, Some(15)));
    settle().await;
    assert_eq!(f.board.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        *lock(&f.warnings),
        ["board sync: board key file unreadable"]
    );
    f.cancel.cancel();
}

/// A board that answers one request with `head` and `body` as raw bytes.
async fn raw_board(head: &str, body: Vec<u8>) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let head = head.to_owned();
    tokio::spawn(async move {
        let Ok((mut socket, _)) = listener.accept().await else {
            return;
        };
        let mut request = [0; 8192];
        let _ = socket.read(&mut request).await;
        let _ = socket.write_all(head.as_bytes()).await;
        let _ = socket.write_all(&body).await;
        let _ = socket.shutdown().await;
        // Hold the socket until the client lets go.
        while matches!(socket.read(&mut request).await, Ok(read) if read > 0) {}
    });
    format!("http://{address}/board/mcp")
}

#[tokio::test]
async fn audit_u29_a_reply_past_the_size_limit_is_refused_without_reading_it() {
    let reply = br#"{"jsonrpc":"2.0","id":1,"result":{"structuredContent":{"ok":true}}}"#;
    let streamed = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n";
    let declared = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        1u64 << 30
    );
    // Padding past the limit before a reply that would otherwise be accepted.
    let mut padded = vec![b' '; RESPONSE_MAX + 1];
    padded.extend_from_slice(reply);
    let cases = [
        (declared.as_str(), Vec::new(), false),
        (streamed, padded, false),
        (streamed, reply.to_vec(), true),
    ];
    for (head, body, accepted) in cases {
        let f = fixture(false).await;
        let target = Target {
            url: raw_board(head, body).await,
            key_file: f.root.path().join("writer.key"),
            project: "demo".into(),
            item_id: "synthetic".into(),
        };
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            f.sync
                .call(&target, "board_upsert_item", serde_json::json!({})),
        )
        .await
        .expect("an oversized reply must not be waited for");
        assert_eq!(result.is_some(), accepted);
        let expected: &[&str] = if accepted {
            &[]
        } else {
            &["board sync: board reply too large"]
        };
        assert_eq!(*lock(&f.warnings), expected);
        f.cancel.cancel();
    }
}

#[test]
fn paths_match_by_prefix_on_whole_names_only() {
    let root = Path::new(if cfg!(windows) {
        "C:\\work\\repo"
    } else {
        "/dev/repo"
    });
    let sub = Path::new(if cfg!(windows) {
        "c:/WORK/repo/src"
    } else {
        "/dev/repo/src"
    });
    let sibling = Path::new(if cfg!(windows) {
        "C:\\work\\repo-two"
    } else {
        "/dev/repo-two"
    });
    assert!(inside(root, root));
    assert!(inside(sub, root));
    assert!(!inside(sibling, root));
    assert!(url_allowed("https://board.example.com/board/mcp"));
    assert!(url_allowed("http://127.0.0.1:9/board/mcp"));
    assert!(!url_allowed("http://board.example.com/board/mcp"));
    assert!(!url_allowed("https://board.example.com/board/mcp?key=x"));
    assert!(!url_allowed("https://user:pw@board.example.com/"));
}
