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
    wait_calls(&f.board, 7).await;
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
            "board_list:", // is the work already on the board?
            "board_upsert_item:task",
            "board_add_event:session_start",
            "board_upsert_item:", // the first summary names the work
            "board_add_event:note",
            "board_upsert_item:done",
            "board_add_event:session_end",
        ]
    );
    assert!(
        seen.iter()
            .all(|(auth, _, _)| auth == &format!("Bearer {KEY}"))
    );
    assert_eq!(seen[0].2["project"], "demo");
    assert!(
        seen[1..]
            .iter()
            .all(|(_, _, a)| a["item_id"] == "session/42")
    );
    assert_eq!(seen[1].2["project"], "demo");
    assert_eq!(seen[1].2["title"], "claude: synthetic task");
    assert_eq!(seen[1].2["state"], "in_progress");
    let named = seen[3].2["title"].as_str().unwrap();
    assert!(
        regex::Regex::new(r"^\d\d-\d\d claude: wrote the parser ")
            .unwrap()
            .is_match(named),
        "{named}"
    );
    assert!(!named.contains("abc123secretvalue"), "{named}");
    assert!(seen[3].2.get("parent_id").is_none() && seen[3].2.get("state").is_none());
    let note = seen[4].2["text"].as_str().unwrap();
    assert!(note.starts_with("[完了] wrote the parser"), "{note}");
    assert!(
        !note.contains("abc123secretvalue"),
        "secrets are masked: {note}"
    );
    assert!(seen[4].2.get("ref").is_none());
    assert!(seen[6].2["text"].as_str().unwrap().contains("exited"));
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
    wait_calls(&f.board, 4).await;
    settle().await;
    let seen = f.seen();
    let tools: Vec<_> = seen.iter().map(|(_, tool, _)| tool.as_str()).collect();
    assert_eq!(
        tools,
        [
            "board_list",
            "board_upsert_item",
            "board_upsert_item",
            "board_add_event"
        ]
    );
    assert_eq!(seen[3].2["kind"], "note");
    assert_eq!(seen[1].2["item_id"], "session/live-20260102T030405Z-8");
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

// ---- Linking work to plans: a board that keeps what it is told. ----

#[derive(Default)]
struct Shelf {
    items: Mutex<Vec<serde_json::Value>>,
    events: Mutex<Vec<serde_json::Value>>,
    calls: AtomicUsize,
}
impl Shelf {
    fn put(&self, item: serde_json::Value) {
        lock(&self.items).push(item);
    }
    fn item(&self, id: &str) -> Option<serde_json::Value> {
        lock(&self.items)
            .iter()
            .find(|item| item["item_id"] == id)
            .cloned()
    }
    fn events_of(&self, id: &str, kind: &str) -> Vec<serde_json::Value> {
        lock(&self.events)
            .iter()
            .filter(|event| event["item_id"] == id && event["kind"] == kind)
            .cloned()
            .collect()
    }
    fn work_ids(&self) -> Vec<String> {
        lock(&self.items)
            .iter()
            .filter_map(|item| item["item_id"].as_str())
            .filter(|id| id.starts_with("session/"))
            .map(str::to_owned)
            .collect()
    }
}
fn plan_item(id: &str, source: &str) -> serde_json::Value {
    serde_json::json!({
        "item_id": id, "project": "demo", "kind": "plan", "title": id,
        "state": "in_progress", "source_ref": source, "parent_id": "demo",
    })
}
async fn shelve(
    axum::extract::State(shelf): axum::extract::State<Arc<Shelf>>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::Json<serde_json::Value> {
    shelf.calls.fetch_add(1, Ordering::SeqCst);
    let args = body["params"]["arguments"].clone();
    let reply = |content: serde_json::Value| {
        axum::Json(serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "result": {"structuredContent": content},
        }))
    };
    let refused = serde_json::json!({"ok": false, "error": {"code": "INVALID"}});
    match body["params"]["name"].as_str().unwrap_or_default() {
        "board_list" => {
            let items: Vec<_> = lock(&shelf.items)
                .iter()
                .filter(|item| item["project"] == args["project"])
                .cloned()
                .collect();
            reply(serde_json::json!({"ok": true, "items": items, "events": []}))
        }
        "board_upsert_item" => {
            let mut items = lock(&shelf.items);
            if let Some(parent) = args["parent_id"].as_str()
                && !items.iter().any(|item| item["item_id"] == parent)
            {
                return reply(refused);
            }
            match items
                .iter()
                .position(|item| item["item_id"] == args["item_id"])
            {
                Some(index) => {
                    let before = items[index].clone();
                    for (key, value) in args.as_object().unwrap() {
                        items[index][key] = value.clone();
                    }
                    let changed = items[index] != before;
                    reply(serde_json::json!({"ok": true, "changed": changed}))
                }
                None => {
                    if ["project", "kind", "title", "state"]
                        .iter()
                        .any(|key| args[*key].is_null())
                    {
                        return reply(refused);
                    }
                    items.push(args);
                    reply(serde_json::json!({"ok": true, "changed": true}))
                }
            }
        }
        "board_add_event" => {
            let known = lock(&shelf.items)
                .iter()
                .any(|item| item["item_id"] == args["item_id"]);
            let link_ok = args.get("ref").is_none_or(|link| {
                link.as_str()
                    .is_some_and(|link| link.starts_with("https://") && !link.contains("refused"))
            });
            if !known || !link_ok {
                return reply(refused);
            }
            lock(&shelf.events).push(args);
            reply(serde_json::json!({"ok": true}))
        }
        _ => reply(refused),
    }
}
/// A fixture whose listed repository writes to a `Shelf` holding project `demo`.
async fn linked() -> (Fixture, Arc<Shelf>) {
    let mut f = fixture(false).await;
    let shelf = Arc::new(Shelf::default());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = axum::Router::new()
        .fallback(axum::routing::post(shelve))
        .with_state(shelf.clone());
    let stopping = f.cancel.clone();
    tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move { stopping.cancelled().await })
            .await
            .unwrap();
    });
    f.url = format!("http://{address}/board/mcp");
    f.list_repo();
    shelf.put(serde_json::json!({
        "item_id": "demo", "project": "demo", "kind": "project", "title": "Demo",
        "state": "in_progress",
    }));
    (f, shelf)
}
/// Waits until `item` has `count` notes (a note is the last write of a
/// summary), then until the worker has stopped calling the board.
async fn noted(shelf: &Shelf, item: &str, count: usize) {
    tokio::time::timeout(Duration::from_secs(20), async {
        while shelf.events_of(item, "note").len() < count {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        loop {
            let before = shelf.calls.load(Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(150)).await;
            if shelf.calls.load(Ordering::SeqCst) == before {
                break;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{item} never got {count} notes: {:?}", lock(&shelf.items)));
}
fn named(title: &serde_json::Value, sentence: &str) -> bool {
    regex::Regex::new(&format!(r"^\d\d-\d\d claude: {}$", regex::escape(sentence)))
        .unwrap()
        .is_match(title.as_str().unwrap_or_default())
}

#[tokio::test]
async fn work_goes_under_the_plan_its_summary_names() {
    let (f, shelf) = linked().await;
    // A step that shares the plan's file comes first in the list: the plan wins.
    shelf.put(serde_json::json!({
        "item_id": "demo/plan/a/c1", "project": "demo", "kind": "context", "title": "C1",
        "state": "done", "source_ref": "docs/plan_a.md", "parent_id": "demo",
    }));
    shelf.put(plan_item("demo/plan/a", "docs/plan_a.md"));
    let d = details(21, &f.repo, Some(100));
    f.sync.session_started(&d);
    f.sync.turn_done(
        Some(&d),
        &done(
            21,
            "Fixed the list order. 次: review 計画: docs/plan_a.md 成果: https://example.com/pull/1",
        ),
    );
    noted(&shelf, "session/100", 1).await;
    let work = shelf.item("session/100").unwrap();
    assert_eq!(work["parent_id"], "demo/plan/a");
    assert!(named(&work["title"], "Fixed the list order"), "{work}");
    let notes = shelf.events_of("session/100", "note");
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0]["text"], "[完了] Fixed the list order. 次: review");
    assert_eq!(notes[0]["ref"], "https://example.com/pull/1");
    assert_eq!(shelf.work_ids(), ["session/100"]);
    assert!(lock(&f.warnings).is_empty(), "{:?}", lock(&f.warnings));
    f.cancel.cancel();
}

#[tokio::test]
async fn work_that_moves_to_another_plan_continues_in_a_new_item() {
    let (f, shelf) = linked().await;
    shelf.put(plan_item("demo/plan/a", "docs/plan_a.md"));
    shelf.put(plan_item("demo/plan/b", "docs/plan_b.md"));
    let d = details(22, &f.repo, Some(100));
    f.sync.session_started(&d);
    f.sync
        .turn_done(Some(&d), &done(22, "Did A. 計画: docs/plan_a.md"));
    f.sync.turn_done(Some(&d), &done(22, "More of it."));
    f.sync
        .turn_done(Some(&d), &done(22, "Did B. 計画: docs\\plan_b.md。"));
    f.sync.session_ended(LiveSessionId(22), "exited");
    noted(&shelf, "session/100/2", 1).await;
    tokio::time::timeout(Duration::from_secs(20), async {
        while shelf.events_of("session/100/2", "session_end").is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let first = shelf.item("session/100").unwrap();
    assert_eq!(first["parent_id"], "demo/plan/a");
    assert_eq!(first["state"], "done");
    assert!(named(&first["title"], "Did A"), "{first}");
    assert_eq!(shelf.events_of("session/100", "note").len(), 2);
    let second = shelf.item("session/100/2").unwrap();
    assert_eq!(second["parent_id"], "demo/plan/b");
    assert_eq!(second["kind"], "task");
    assert_eq!(second["state"], "done");
    assert!(named(&second["title"], "Did B"), "{second}");
    assert_eq!(shelf.events_of("session/100/2", "note").len(), 1);
    assert_eq!(shelf.events_of("session/100/2", "session_end").len(), 1);
    assert!(shelf.events_of("session/100", "session_end").is_empty());
    assert_eq!(shelf.work_ids(), ["session/100", "session/100/2"]);
    f.cancel.cancel();
}

#[tokio::test]
async fn work_without_a_named_plan_stays_unlinked_and_is_named_once() {
    let (f, shelf) = linked().await;
    let d = details(23, &f.repo, Some(100));
    f.sync.session_started(&d);
    f.sync
        .turn_done(Some(&d), &done(23, "Looked around。次は実装"));
    // Words that only look like the tails stay in the note and link nothing.
    f.sync.turn_done(
        Some(&d),
        &done(23, "Second. 成果: tests pass now 計画: next phase"),
    );
    noted(&shelf, "session/100", 2).await;
    let work = shelf.item("session/100").unwrap();
    assert!(work.get("parent_id").is_none(), "{work}");
    assert!(named(&work["title"], "Looked around"), "{work}");
    let notes = shelf.events_of("session/100", "note");
    assert_eq!(notes.len(), 2);
    assert_eq!(
        notes[1]["text"],
        "[完了] Second. 成果: tests pass now 計画: next phase"
    );
    assert!(notes[1].get("ref").is_none());
    assert_eq!(shelf.work_ids(), ["session/100"]);
    f.cancel.cancel();
}

#[tokio::test]
async fn a_plan_file_without_an_item_gets_one_but_a_mistyped_path_does_not() {
    let (f, shelf) = linked().await;
    std::fs::create_dir_all(f.repo.join("docs")).unwrap();
    std::fs::write(
        f.repo.join("docs").join("plan_new.md"),
        "---\ntype: plan\ntitle: not this\n---\n\n# [計画] New plan title\n\nbody stays here\n",
    )
    .unwrap();
    std::fs::write(f.repo.join("docs").join("notes.txt"), "# Not a plan\n").unwrap();
    let d = details(24, &f.repo, Some(100));
    f.sync.session_started(&d);
    f.sync
        .turn_done(Some(&d), &done(24, "Started it. 計画: docs/plan_new.md"));
    noted(&shelf, "session/100", 1).await;
    let plan = shelf.item("plan/demo/docs/plan_new.md").unwrap();
    assert_eq!(plan["kind"], "plan");
    assert_eq!(plan["title"], "New plan title");
    assert_eq!(plan["source_ref"], "docs/plan_new.md");
    assert_eq!(plan["parent_id"], "demo");
    assert_eq!(
        shelf.item("session/100").unwrap()["parent_id"],
        "plan/demo/docs/plan_new.md"
    );
    // The same file written in full is the same plan: no second item.
    let full = f.repo.join("docs").join("plan_new.md");
    let again = details(25, &f.repo, Some(101));
    f.sync.session_started(&again);
    f.sync.turn_done(
        Some(&again),
        &done(25, &format!("Went on. 計画: {}", full.display())),
    );
    // A path that is not there, and a file that is not Markdown, create nothing.
    let other = details(26, &f.repo, Some(102));
    f.sync.session_started(&other);
    f.sync
        .turn_done(Some(&other), &done(26, "Typo. 計画: docs/plan_mising.md"));
    f.sync
        .turn_done(Some(&other), &done(26, "Text. 計画: docs/notes.txt"));
    noted(&shelf, "session/101", 1).await;
    noted(&shelf, "session/102", 2).await;
    assert_eq!(
        shelf.item("session/101").unwrap()["parent_id"],
        "plan/demo/docs/plan_new.md"
    );
    assert!(
        shelf.item("session/102/2").is_none(),
        "work under no plan is not split"
    );
    assert!(
        shelf
            .item("session/102")
            .unwrap()
            .get("parent_id")
            .is_none()
    );
    let plans = lock(&shelf.items)
        .iter()
        .filter(|item| item["kind"] == "plan")
        .count();
    assert_eq!(plans, 1);
    let sent = serde_json::to_string(&*lock(&shelf.items)).unwrap()
        + &serde_json::to_string(&*lock(&shelf.events)).unwrap();
    assert!(
        !sent.contains("body stays here"),
        "file contents never leave"
    );
    f.cancel.cancel();
}

#[tokio::test]
async fn a_link_the_board_refuses_does_not_cost_the_note() {
    let (f, shelf) = linked().await;
    let d = details(27, &f.repo, Some(100));
    f.sync.session_started(&d);
    f.sync.turn_done(
        Some(&d),
        &done(27, "Opened it. 成果: https://example.com/refused/1"),
    );
    f.sync.turn_done(
        Some(&d),
        &done(27, "Plain link. 成果: http://example.com/pull/2"),
    );
    noted(&shelf, "session/100", 2).await;
    let notes = shelf.events_of("session/100", "note");
    assert_eq!(notes.len(), 2);
    assert_eq!(notes[0]["text"], "[完了] Opened it.");
    assert!(notes[0].get("ref").is_none());
    assert_eq!(
        notes[1]["text"],
        "[完了] Plain link. 成果: http://example.com/pull/2"
    );
    assert!(notes[1].get("ref").is_none());
    f.cancel.cancel();
}

#[tokio::test]
async fn after_a_hub_restart_named_work_is_continued_not_reset() {
    let (f, shelf) = linked().await;
    shelf.put(plan_item("demo/plan/a", "docs/plan_a.md"));
    shelf.put(plan_item("demo/plan/b", "docs/plan_b.md"));
    // What an earlier Hub left: two pieces, both named, the second still open.
    shelf.put(serde_json::json!({
        "item_id": "session/100", "project": "demo", "kind": "task",
        "title": "01-02 claude: Earlier", "state": "done", "parent_id": "demo/plan/a",
    }));
    shelf.put(serde_json::json!({
        "item_id": "session/100/2", "project": "demo", "kind": "task",
        "title": "01-02 claude: Later", "state": "in_progress", "parent_id": "demo/plan/b",
    }));
    let d = details(28, &f.repo, Some(100));
    f.sync
        .turn_done(Some(&d), &done(28, "More B. 計画: docs/plan_b.md"));
    noted(&shelf, "session/100/2", 1).await;
    assert_eq!(
        shelf.item("session/100/2").unwrap()["title"],
        "01-02 claude: Later"
    );
    assert_eq!(shelf.item("session/100").unwrap()["state"], "done");
    assert_eq!(shelf.events_of("session/100/2", "note").len(), 1);
    assert!(shelf.events_of("session/100/2", "session_start").is_empty());
    assert!(shelf.events_of("session/100", "session_start").is_empty());
    f.sync
        .turn_done(Some(&d), &done(28, "Back to A. 計画: docs/plan_a.md"));
    noted(&shelf, "session/100/3", 1).await;
    let third = shelf.item("session/100/3").unwrap_or_else(|| {
        panic!(
            "no third piece: {:?} / {:?}",
            lock(&shelf.items),
            lock(&f.warnings)
        )
    });
    assert_eq!(third["parent_id"], "demo/plan/a");
    assert!(named(&third["title"], "Back to A"), "{third}");
    assert_eq!(shelf.item("session/100/2").unwrap()["state"], "done");
    f.cancel.cancel();
}

#[test]
fn declared_plans_are_kept_as_the_board_keeps_a_reference() {
    let root = Path::new(if cfg!(windows) {
        "C:\\work\\repo"
    } else {
        "/work/repo"
    });
    let full = if cfg!(windows) {
        "c:\\WORK\\repo\\docs\\plan_x.md"
    } else {
        "/work/repo/docs/plan_x.md"
    };
    assert_eq!(
        plan_ref("docs/plan_x.md", root).as_deref(),
        Some("docs/plan_x.md")
    );
    assert_eq!(
        plan_ref("`./docs\\plan_x.md`。", root).as_deref(),
        Some("docs/plan_x.md")
    );
    assert_eq!(plan_ref(full, root).as_deref(), Some("docs/plan_x.md"));
    assert_eq!(
        plan_ref("https://example.com/issues/3.", root).as_deref(),
        Some("https://example.com/issues/3")
    );
    for refused in [
        "../outside.md",
        "docs/../../outside.md",
        "/etc/plan.md",
        "~/plan.md",
        "D:\\elsewhere\\plan.md",
        "http://example.com/issues/3",
        "https://user:pw@example.com/issues/3",
        // A query or fragment can carry a secret, and the link is cut out of
        // the text before the text is masked.
        "https://example.com/issues/3?token=abc",
        "https://example.com/issues/3#access",
        "docs//plan.md",
    ] {
        assert_eq!(plan_ref(refused, root), None, "{refused}");
    }
    // A URL holding something `mask_secrets` hides is not sent as a link, and the
    // outcome stays in the (masked) note text instead.
    let secret_url = format!("https://example.com/pull/{}{}", "ghp_", "a".repeat(24));
    assert_eq!(plan_ref(&secret_url, root), None);
    let found = tails(&format!("Done. 成果: {secret_url}"), root);
    assert_eq!(found.outcome, None);
    assert!(found.rest.contains("成果"), "{}", found.rest);
    assert_eq!(
        plan_item_id("demo", "docs/plan_x.md"),
        "plan/demo/docs/plan_x.md"
    );
    let other = plan_item_id("demo", "docs/計画.md");
    assert!(
        other.starts_with("plan/demo/docs/--.md-") && other.len() <= 128,
        "{other}"
    );
    assert!(
        other
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._/-".contains(c))
    );
    assert_ne!(other, plan_item_id("demo", "docs/作業.md"));
}

#[test]
fn work_titles_are_one_clean_line_with_the_first_sentence() {
    assert_eq!(
        first_sentence("看板を直しました。変更: a.rs。"),
        "看板を直しました"
    );
    assert_eq!(
        first_sentence("\n  Fixed v1.2 parsing. More.\nsecond line"),
        "Fixed v1.2 parsing"
    );
    assert_eq!(first_sentence("no stop here"), "no stop here");
    assert_eq!(first_sentence(""), "");
    assert_eq!(
        work_title("11-11", "", "tab\there\u{200b} now").as_deref(),
        Some("11-11 AI: tab here now")
    );
    assert_eq!(work_title("11-11", "claude", "  \n"), None);
    let long = work_title("11-11", "claude", &"あ".repeat(300)).unwrap();
    assert!(
        long.chars().count() <= TITLE_MAX && long.ends_with('…'),
        "{long}"
    );
}
