//! Synthetic owned roots, the production core/journal/repository, and source
//! request shapes. These tests do not read real session history or launch CLIs.
use super::*;
use crate::{
    config::{Resource, RuntimePaths},
    hub::task_owner::HubTaskOwner,
    proto::{self, core::*, provider::Layers, time::Timestamp},
    storage::SqliteSessionStorage,
    terminal::{
        events::CoreEventBus,
        journal::JournalOptions,
        session::{EngineOptions, SessionEngine},
    },
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::Value;
use std::path::Path;
use std::time::Duration;

struct NoSpawn;
impl WrappedSessionSpawner for NoSpawn {
    fn spawn_and_wait<'a>(
        &'a self,
        _: WrappedSpawnSpec,
        _: Duration,
        _: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome> {
        Box::pin(async {
            SpawnWaitOutcome::Failed("synthetic fixture does not launch providers".into())
        })
    }
}
struct NoTransport;
impl WrapperTransport for NoTransport {
    fn send<'a>(
        &'a self,
        _: SessionBinding,
        _: proto::Message,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async {
            Err(SessionError::Transport(
                "synthetic fixture does not send wrapper frames".into(),
            ))
        })
    }
}
struct Sink {
    journal: Arc<SessionJournal>,
    bus: CoreEventBus,
    messages: Mutex<Vec<proto::Message>>,
}
impl CoreEffectSink for Sink {
    fn apply<'a>(&'a self, effects: CoreEffects) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
        Box::pin(async move {
            for (index, effect) in effects.0.into_iter().enumerate() {
                let result = match effect {
                    CoreEffect::Persist(effect) => self.journal.apply(effect),
                    CoreEffect::PersistBound {
                        binding,
                        scope,
                        effect,
                        order,
                    } => {
                        order.wait().await;
                        let result = self.journal.apply_bound(binding, scope, effect);
                        drop(order);
                        result
                    }
                    CoreEffect::Notify(event) => self.bus.publish(event).map(|_| ()),
                    CoreEffect::SendUi { message, .. }
                    | CoreEffect::SendUiBestEffort { message, .. } => {
                        self.messages.lock().unwrap().push(message);
                        Ok(())
                    }
                    _ => panic!("unexpected session route fixture effect"),
                };
                if let Err(error) = result {
                    return Err(CoreEffectFailure { index, error });
                }
            }
            Ok(())
        })
    }
}
struct Fixture {
    _root: tempfile::TempDir,
    _installed: tempfile::TempDir,
    paths: RuntimePaths,
    config: Config,
    store: Option<Arc<SqliteSessionStorage>>,
    core: Arc<SessionEngine>,
    sink: Arc<Sink>,
    http: Arc<SessionHttp>,
}
fn fixture(with_store: bool) -> Fixture {
    fixture_at_port(with_store, 49122)
}
fn fixture_at_port(with_store: bool, port: u16) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), port, installed.path()).unwrap();
    let store = with_store.then(|| {
        Arc::new(
            SqliteSessionStorage::open(
                &paths,
                StorageOptions::baseline(paths.resource(Resource::Logs)),
            )
            .unwrap(),
        )
    });
    let mut config = Config::default();
    config.hub.log_dir = paths
        .resource(Resource::Logs)
        .to_string_lossy()
        .into_owned();
    let journal = Arc::new(SessionJournal::new(
        paths.clone(),
        store.clone().map(|s| s as Arc<dyn SessionStorage>),
        JournalOptions {
            session_enabled: true,
            max_bytes: 0,
        },
    ));
    let bus = CoreEventBus::new(64).unwrap();
    let sink = Arc::new(Sink {
        journal: journal.clone(),
        bus: bus.clone(),
        messages: Mutex::new(Vec::new()),
    });
    let core = Arc::new(SessionEngine::new(
        EngineOptions {
            warning: Arc::new(|_, _| {}),
            ..Default::default()
        },
        journal.clone(),
        Arc::new(NoTransport),
        sink.clone(),
        Arc::new(NoSpawn),
        bus,
    ));
    let http = Arc::new(SessionHttp::new(core.clone(), journal));
    Fixture {
        _root: root,
        _installed: installed,
        paths,
        config,
        store,
        core,
        sink,
        http,
    }
}
fn request(path: &str, query: &str) -> Request {
    Request {
        method: "GET".into(),
        path: path.into(),
        query: query.into(),
        ..Default::default()
    }
}
fn body(response: Response) -> Value {
    assert_eq!(
        response.status,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    serde_json::from_slice(&response.body).unwrap()
}
fn read(f: &Fixture, path: &str, query: &str) -> Value {
    body(
        f.http
            .handle_read_authenticated(&request(path, query), &f.config),
    )
}
fn error(response: Response, status: u16, detail: &str) {
    assert_eq!(response.status, status);
    assert_eq!(
        serde_json::from_slice::<Value>(&response.body).unwrap()["detail"],
        detail
    );
}
fn now() -> Timestamp {
    Timestamp::UNIX_EPOCH + Duration::from_secs(1_700_000_000)
}
async fn register(f: &Fixture) -> LiveSessionId {
    let registration = f
        .core
        .register(
            RegisterRequest {
                message: proto::Message {
                    provider: "claude".into(),
                    cwd: f._root.path().to_string_lossy().into_owned(),
                    pid: 7,
                    cols: 80,
                    rows: 24,
                    ..Default::default()
                },
                spawn_proof: None,
            },
            WrapperConnectionId(1),
            now(),
        )
        .await
        .unwrap();
    f.sink.apply(registration.after_registered).await.unwrap();
    registration.binding.session
}

#[tokio::test]
async fn attachment_http_and_ws_use_real_bound_journal() {
    for enabled in [false, true] {
        let f = fixture(true);
        f.sink.journal.set_enabled(enabled);
        let id = register(&f).await;
        let service = crate::files::FilesService::new(f._root.path().into(), f.paths.clone())
            .with_history(f.sink.journal.clone());
        let mut saved_paths = Vec::new();
        for (filename, data) in [
            ("image.png", b"\x89PNG\r\n\x1a\nsynthetic".as_slice()),
            ("paste.txt", b"synthetic text".as_slice()),
        ] {
            let mut payload = format!("--bound\r\nContent-Disposition: form-data; name=\"session_id\"\r\n\r\n{}\r\n--bound\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n\r\n", id.0).into_bytes();
            payload.extend(data);
            payload.extend(b"\r\n--bound--\r\n");
            let response = service
                .handle(
                    &Request {
                        method: "POST".into(),
                        path: "/api/attach".into(),
                        headers: vec![(
                            "Content-Type".into(),
                            "multipart/form-data; boundary=bound".into(),
                        )],
                        body: payload,
                        ..Default::default()
                    },
                    f.core.as_ref(),
                    f.store.as_deref().map(|s| s as &dyn SessionStorage),
                    false,
                    &AttachmentWorkspace,
                )
                .await
                .unwrap();
            assert_eq!(
                response.status,
                200,
                "{}",
                String::from_utf8_lossy(&response.body)
            );
            let saved: Value = serde_json::from_slice(&response.body).unwrap();
            saved_paths.push(saved["saved_path"].clone());
            assert_eq!(
                std::fs::read(saved["saved_path"].as_str().unwrap()).unwrap(),
                data
            );
            assert_eq!(
                saved["inject"],
                format!("@{} ", saved["saved_path"].as_str().unwrap())
            );
        }
        service
            .record_ws_attachment(
                &proto::Message {
                    session_id: id.0,
                    filename: "ws.png".into(),
                    image_data: STANDARD.encode(b"synthetic ws image"),
                    ..Default::default()
                },
                f.core.as_ref(),
                None,
            )
            .unwrap();
        let log = std::path::PathBuf::from(&f.core.snapshot(id).unwrap().log_path)
            .with_extension("jsonl");
        let history = std::fs::read_to_string(log).unwrap_or_default();
        let events: Vec<Value> = history
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(
            events.iter().filter(|e| e["type"] == "attach").count(),
            if enabled { 3 } else { 0 }
        );
        if enabled {
            let attachments: Vec<_> = events.iter().filter(|e| e["type"] == "attach").collect();
            for (event, (filename, path)) in attachments
                .iter()
                .zip(["image.png", "paste.txt"].into_iter().zip(saved_paths))
            {
                assert_eq!(event["filename"], filename);
                assert_eq!(event["path"], path);
                assert_eq!(event["provider"], "claude");
                assert_eq!(event["session_id"], id.0);
            }
            assert_eq!(attachments[2]["filename"], "ws.png");
            assert_eq!(
                std::fs::read(attachments[2]["path"].as_str().unwrap()).unwrap(),
                b"synthetic ws image"
            );
        }
        let messages = f
            .store
            .as_ref()
            .unwrap()
            .chat_messages_by_live_session(id, 100)
            .unwrap();
        assert_eq!(
            messages
                .unwrap_or_default()
                .iter()
                .filter(|e| e.kind == "attach")
                .count(),
            if enabled { 3 } else { 0 }
        );
    }
}
struct AttachmentWorkspace;
impl crate::files::WorkspaceReads for AttachmentWorkspace {
    fn memo_mentions(&self) -> Vec<(String, String)> {
        vec![]
    }
    fn recorded_handoff_note(&self, _: &Path, _: &Path) -> bool {
        false
    }
}

#[tokio::test]
async fn attachment_ws_rejects_missing_ended_and_replaced_session_contexts() {
    let f = fixture(true);
    let service = crate::files::FilesService::new(f._root.path().into(), f.paths.clone())
        .with_history(f.sink.journal.clone());
    let mut message = proto::Message {
        session_id: 999,
        filename: "fixture.png".into(),
        image_data: STANDARD.encode(b"synthetic"),
        ..Default::default()
    };
    assert_eq!(
        service
            .record_ws_attachment(&message, f.core.as_ref(), None)
            .unwrap_err()
            .status,
        404
    );
    assert!(!f.paths.resource(Resource::Attachments).exists());
    let missing = Request { method: "POST".into(), path: "/api/attach".into(), headers: vec![("Content-Type".into(), "multipart/form-data; boundary=bound".into())], body: b"--bound\r\nContent-Disposition: form-data; name=\"session_id\"\r\n\r\n999\r\n--bound\r\nContent-Disposition: form-data; name=\"file\"; filename=\"fixture.txt\"\r\n\r\nsynthetic\r\n--bound--\r\n".to_vec(), ..Default::default() };
    assert_eq!(
        service
            .handle(&missing, f.core.as_ref(), None, false, &AttachmentWorkspace)
            .await
            .unwrap()
            .status,
        404
    );
    assert!(!f.paths.resource(Resource::Attachments).exists());
    let id = register(&f).await;
    let original = f.core.details(id).unwrap();
    message.session_id = id.0;
    let effects = f.core.disconnected(original.binding, now()).unwrap();
    // Deliberately leave journal cleanup unapplied: the core still rejects it.
    assert_eq!(
        service
            .record_ws_attachment(&message, f.core.as_ref(), None)
            .unwrap_err()
            .status,
        404
    );
    assert!(!f.paths.resource(Resource::Attachments).exists());
    f.sink.apply(effects).await.unwrap();
    let replacement = f
        .core
        .reattach(
            ReattachRequest {
                restored_metadata: None,
                message: proto::Message {
                    session_id: id.0,
                    provider: "claude".into(),
                    cwd: original.snapshot.cwd,
                    started_at: original.snapshot.started_at,
                    pid: 8,
                    cols: 80,
                    rows: 24,
                    ..Default::default()
                },
            },
            WrapperConnectionId(2),
            now(),
        )
        .await
        .unwrap();
    f.sink.apply(replacement.after_reattached).await.unwrap();
    assert!(!f.core.is_current(original.binding));
    assert!(f.core.is_current(replacement.binding));
    service
        .record_ws_attachment(&message, f.core.as_ref(), None)
        .unwrap();
    let invalid = proto::Message {
        image_data: "invalid base64".into(),
        ..message
    };
    assert_eq!(
        service
            .record_ws_attachment(&invalid, f.core.as_ref(), None)
            .unwrap_err()
            .status,
        400
    );
    assert_eq!(
        std::fs::read_dir(
            f.paths
                .resource(Resource::Attachments)
                .join(id.0.to_string())
        )
        .unwrap()
        .count(),
        1
    );
}
fn start(f: &Fixture, live: i64, suffix: &str) -> DbSessionId {
    let sessions = f.paths.resource(Resource::Logs).join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    f.store
        .as_ref()
        .unwrap()
        .start_session(SessionStart {
            live_session_id: LiveSessionId(live),
            provider: "synthetic".into(),
            state: "running".into(),
            started_at: "2026-01-01T00:00:00Z".into(),
            jsonl_path: sessions
                .join(format!("{suffix}.jsonl"))
                .to_string_lossy()
                .into_owned(),
            log_path: sessions
                .join(format!("{suffix}.log"))
                .to_string_lossy()
                .into_owned(),
            ..Default::default()
        })
        .unwrap()
}
fn message(f: &Fixture, live: i64, text: &str) {
    f.store
        .as_ref()
        .unwrap()
        .store_event(
            LiveSessionId(live),
            HistoryEvent(
                json!({"type":"user_input", "ts":"2026-01-01T00:00:01Z", "text":text})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .unwrap();
}
fn info_context<'a>() -> InfoContext<'a> {
    InfoContext {
        cwd: "/synthetic/project",
        version: "synthetic-version",
        git_commit: "synthetic-commit",
        build_time: "2026-01-01T00:00:00Z",
        binary_sha256: "synthetic-sha",
        binary_stale: false,
        web_src_hash: "synthetic-web-hash",
        web_dist_fresh: true,
        runtime_mode: "linux",
        user_name_fallback: "synthetic-user",
        ssh: false,
        host_ip: "192.0.2.1",
        env_kind_override: "",
        net_hint_ssh: false,
        net_hint_host: "",
        net_hint_env_kind: "",
        registry: None,
    }
}

#[test]
fn missing_store_short_circuits_invalid_identifiers_with_source_arrays() {
    let f = fixture(false);
    for (path, key) in [
        ("/api/session-chat", "messages"),
        ("/api/session-history", "sessions"),
        ("/api/session-search", "results"),
        ("/api/approval-history", "approvals"),
    ] {
        assert_eq!(
            read(&f, path, "session_db_id=not-a-number&q=word")[key],
            json!([])
        );
    }
    error(
        f.http
            .handle_read_authenticated(&request("/api/session-log", "session_id=bad"), &f.config),
        400,
        "invalid session_id",
    );
}
#[tokio::test]
async fn maintenance_resets_archived_history_and_preserves_canonical_live_session() {
    let f = fixture(true);
    let id = register(&f).await;
    message(&f, id.0, "retained current user message");
    let old = start(&f, 99, "archived-reset");
    message(&f, 99, "archived message");
    let response = f.http.handle_maintenance_authenticated(&Request {
        method: "POST".into(),
        path: "/api/session-store/reset".into(),
        body: b"ignored body".to_vec(),
        ..Default::default()
    });
    let got = body(response);
    assert_eq!(got["ok"], true);
    assert_eq!(got["result"]["preserved_sessions"], 1);
    let store = f.store.as_ref().unwrap();
    assert!(store.session_overview_by_session_id(old).unwrap() == SessionOverview::default());
    assert_eq!(
        store
            .chat_messages_by_live_session(id, 100)
            .unwrap()
            .unwrap()
            .len(),
        0
    );
    assert!(f.core.snapshot(id).is_some());
    assert!(store.session_overview_by_live_session(id).unwrap().id.0 > 0);
    message(&f, id.0, "new current user message after reset");
    assert_eq!(
        store
            .chat_messages_by_live_session(id, 100)
            .unwrap()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        body(f.http.handle_maintenance_authenticated(&Request {
            method: "POST".into(),
            path: "/api/session-store/prune-transcript-noise".into(),
            ..Default::default()
        }))["deleted_messages"],
        0
    );
}
#[tokio::test]
async fn log_purge_keeps_active_files_and_legacy_notice_publishes_before_failed_save() {
    let f = fixture(true);
    let id = register(&f).await;
    let active = f.core.snapshot(id).unwrap();
    // Registration publishes paths; files are materialized only on output.
    // Create the active artifacts so this fixture proves purge protection.
    std::fs::write(&active.log_path, b"synthetic active").unwrap();
    std::fs::write(&active.jsonl_path, b"synthetic active jsonl").unwrap();
    let sessions = crate::files::safe_fs::Dir::open(Path::new(&f.config.hub.log_dir))
        .unwrap()
        .child_dir("sessions", true)
        .unwrap();
    sessions
        .create_new("old.log", b"synthetic old", 0o600)
        .unwrap();
    sessions
        .create_new("keep.bin", b"synthetic other", 0o600)
        .unwrap();
    let spawn = crate::files::safe_fs::Dir::open(Path::new(&f.config.hub.log_dir))
        .unwrap()
        .child_dir("spawn", true)
        .unwrap();
    spawn
        .create_new("old-spawn", b"synthetic spawn", 0o600)
        .unwrap();
    let config = crate::config::ConfigStore::new(f.paths.clone(), f.config.clone()).unwrap();
    let response = f.http.handle_log_maintenance_authenticated(
        &Request {
            method: "POST".into(),
            path: "/api/logs/purge".into(),
            ..Default::default()
        },
        &config,
        &f.paths,
    );
    let result = body(response);
    assert_eq!(result["session_files"], 1);
    assert_eq!(result["spawn_files"], 1);
    assert!(Path::new(&active.log_path).exists());
    assert!(Path::new(&active.jsonl_path).exists());
    assert_eq!(sessions.read("keep.bin", 20).unwrap(), b"synthetic other");
    std::fs::create_dir(f.paths.resource(Resource::Config)).unwrap();
    let response = f.http.handle_log_maintenance_authenticated(
        &Request {
            method: "POST".into(),
            path: "/api/logs/legacy-notice".into(),
            body: br#"{"enable_logging":true,"ENABLE_LOGGING":"bad"} ignored"#.to_vec(),
            ..Default::default()
        },
        &config,
        &f.paths,
    );
    assert_eq!(response.status, 500);
    let published = config.snapshot().unwrap();
    assert!(published.config.log.legacy_logs_notice_shown);
    assert!(published.config.log.session_enabled);
}
#[test]
fn empty_existing_store_preserves_null_and_approval_array_distinctions() {
    let f = fixture(true);
    assert_eq!(
        read(&f, "/api/session-chat", "session_id=1")["messages"],
        Value::Null
    );
    assert_eq!(
        read(&f, "/api/session-history", "")["sessions"],
        Value::Null
    );
    assert_eq!(
        read(&f, "/api/session-search", "q=missing")["results"],
        Value::Null
    );
    assert_eq!(
        read(&f, "/api/session-search", "q=%20")["results"],
        json!([])
    );
    assert_eq!(
        read(&f, "/api/approval-history", "")["approvals"],
        json!([])
    );
    start(&f, 1, "empty-session");
    assert_eq!(
        read(&f, "/api/session-chat", "session_id=1")["messages"],
        json!([])
    );
}
#[tokio::test]
async fn approval_profile_migration_and_failed_save_update_real_private_mirror() {
    let f = fixture(true);
    let config =
        Arc::new(crate::config::ConfigStore::new(f.paths.clone(), f.config.clone()).unwrap());
    let dir = crate::files::safe_fs::Dir::open(f.paths.root())
        .unwrap()
        .child_dir("approval-patterns", true)
        .unwrap();
    dir.create_new("claude.json", br#"["synthetic legacy phrase"]"#, 0o600)
        .unwrap();
    let owner = crate::application::approval_patterns::ApprovalPatterns::new(
        &f.paths,
        config.clone(),
        Arc::downgrade(&f.core),
        Arc::new(|_| {}),
    )
    .unwrap();
    owner.sync().unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(
            &dir.read(&format!("{}.{}.json", "claude", "custom"), 4096)
                .unwrap()
        )
        .unwrap(),
        json!(["synthetic legacy phrase"])
    );
    assert!(owner.active().unwrap()["claude"].contains(&"do you want to".into()));
    let http = crate::hub::approval_pattern_routes::ApprovalPatternHttp::new(
        owner.clone(),
        config.clone(),
    );
    let response = http.handle_authenticated(&Request {
        method: "PUT".into(),
        path: "/api/approval-patterns/claude".into(),
        body: br#"[null,"  synthetic changed  ",""] trailing"#.to_vec(),
        ..Default::default()
    });
    assert_eq!(response.status, 204);
    std::fs::create_dir(f.paths.resource(Resource::Config)).unwrap();
    let response = http.handle_authenticated(&Request {
        method: "POST".into(),
        path: "/api/approval-patterns/profile".into(),
        body: br#"{"provider":"claude","profile":"custom"}"#.to_vec(),
        ..Default::default()
    });
    assert_eq!(response.status, 204);
    assert_eq!(
        config.snapshot().unwrap().config.approval_profiles.claude,
        "custom"
    );
    assert_eq!(owner.active().unwrap()["claude"], vec!["synthetic changed"]);
    assert_eq!(
        serde_json::from_slice::<Value>(&owner.asset("claude.json").unwrap()).unwrap(),
        json!(["synthetic changed"])
    );
    assert!(owner.asset("../claude.json").is_err());
    let response = http.handle_authenticated(&Request {
        method: "DELETE".into(),
        path: "/api/approval-patterns/missing".into(),
        ..Default::default()
    });
    assert_eq!(response.status, 404);
    let response = http.handle_authenticated(&Request {
        method: "GET".into(),
        path: "/approval-patterns/claude.json".into(),
        ..Default::default()
    });
    assert_eq!(response.status, 200);
    assert_eq!(
        serde_json::from_slice::<Value>(&response.body).unwrap(),
        json!(["synthetic changed"])
    );
    let response = http.handle_authenticated(&Request {
        method: "POST".into(),
        path: "/api/approval-patterns/copy-official".into(),
        body: br#"{"provider":"claude","profile":9}"#.to_vec(),
        ..Default::default()
    });
    assert_eq!(response.status, 204);
    assert!(owner.active().unwrap()["claude"].contains(&"do you want to".into()));
}
#[test]
fn chat_query_priority_whitespace_signs_and_overflow_follow_go() {
    let f = fixture(true);
    let db = start(&f, 1, "first");
    message(&f, 1, "source message");
    let result = read(
        &f,
        "/api/session-chat",
        &format!("session_db_id=%20{}%20&session_id=invalid", db.0),
    );
    assert_eq!(result["messages"][0]["rawText"], "source message");
    assert_eq!(
        read(&f, "/api/session-chat", "session_id=%2B1")["messages"],
        result["messages"]
    );
    for q in [
        "session_id=%201%20",
        "session_id=0",
        "session_id=9223372036854775808",
    ] {
        error(
            f.http
                .handle_read_authenticated(&request("/api/session-chat", q), &f.config),
            400,
            "session_id required",
        );
    }
    error(
        f.http.handle_read_authenticated(
            &request("/api/session-chat", "session_db_id=bad&session_id=1"),
            &f.config,
        ),
        400,
        "invalid session_db_id",
    );
}
#[test]
fn historical_database_id_stays_distinct_from_reused_live_id() {
    let f = fixture(true);
    let old = start(&f, 8, "old");
    message(&f, 8, "old request");
    f.store
        .as_ref()
        .unwrap()
        .end_session(LiveSessionId(8), "completed", "done", now());
    let new = start(&f, 8, "new");
    message(&f, 8, "new request");
    assert_ne!(old, new);
    assert_eq!(
        read(&f, "/api/session-chat", &format!("session_db_id={}", old.0))["messages"][0]["rawText"],
        "old request"
    );
    assert_eq!(
        read(&f, "/api/session-chat", "session_id=8")["messages"][0]["rawText"],
        "new request"
    );
}
#[test]
fn history_includes_archived_and_storage_pagination_is_retained() {
    let f = fixture(true);
    start(&f, 1, "one");
    start(&f, 2, "two");
    f.store
        .as_ref()
        .unwrap()
        .update_session_meta(LiveSessionId(1), "saved title", &[], "summary", true)
        .unwrap();
    let history = read(&f, "/api/session-history", "limit=500");
    let rows = history["sessions"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().any(|row| row["archived"] == true));
    assert_eq!(
        read(&f, "/api/session-history", "limit=1")["sessions"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        read(&f, "/api/session-history", "limit=invalid")["sessions"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}
#[test]
fn chat_returns_latest_limit_in_forward_order_and_search_uses_same_store() {
    let f = fixture(true);
    start(&f, 1, "one");
    for text in ["searchneedle one", "searchneedle two", "searchneedle three"] {
        message(&f, 1, text);
    }
    let rows = read(&f, "/api/session-chat", "session_id=1&limit=2")["messages"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["rawText"], "searchneedle two");
    assert_eq!(rows[1]["rawText"], "searchneedle three");
    assert_eq!(
        read(&f, "/api/session-chat", "session_id=1&limit=1001")["messages"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        read(&f, "/api/session-search", "q=%20searchneedle%20&limit=2")["results"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}
#[test]
fn approval_filters_prefer_database_id_and_pending_is_case_insensitive() {
    let f = fixture(true);
    let db = start(&f, 1, "one");
    let store = f.store.as_ref().unwrap();
    for (sig, step) in [("first", 1), ("second", 2)] {
        store.store_approval_detected(ApprovalDetected {
            live_session_id: LiveSessionId(1),
            sig: sig.into(),
            source: "native".into(),
            kind: "shell".into(),
            provider: "synthetic".into(),
            question: "Run synthetic task?".into(),
            context: String::new(),
            block: String::new(),
            candidate_key: sig.into(),
            source_epoch: ApprovalSourceEpoch(1),
            options: vec![],
            detected_at: Some(now() + Duration::from_secs(step)),
        });
    }
    store.store_approval_consumed(
        LiveSessionId(1),
        "first",
        "yes",
        now() + Duration::from_secs(3),
    );
    let rows = read(
        &f,
        "/api/approval-history",
        &format!(
            "session_db_id=%20{}%20&session_id=bad&state=%20PeNdInG%20",
            db.0
        ),
    );
    assert_eq!(rows["approvals"].as_array().unwrap().len(), 1);
    assert_eq!(rows["approvals"][0]["sig"], "second");
    assert_eq!(
        read(&f, "/api/approval-history", "session_id=%201%20")["approvals"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        read(&f, "/api/approval-history", "limit=1")["approvals"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    error(
        f.http.handle_read_authenticated(
            &request("/api/approval-history", "session_id=x"),
            &f.config,
        ),
        400,
        "invalid session_id",
    );
}
#[test]
fn closed_repository_errors_are_not_successful_empty_history() {
    let f = fixture(true);
    f.store.as_ref().unwrap().close().unwrap();
    for (path, query, code) in [
        ("/api/session-chat", "session_id=1", "session_chat_failed"),
        ("/api/session-history", "", "session_history_failed"),
        ("/api/session-search", "q=word", "session_search_failed"),
        ("/api/approval-history", "", "approval_history_failed"),
    ] {
        let response = f
            .http
            .handle_read_authenticated(&request(path, query), &f.config);
        assert_eq!(response.status, 500);
        assert_eq!(
            serde_json::from_slice::<Value>(&response.body).unwrap()["error"],
            code
        );
    }
}
#[tokio::test]
async fn current_and_persisted_logs_share_tail_range_and_masking_contract() {
    let f = fixture(true);
    let id = register(&f).await;
    let snapshot = f.core.snapshot(id).unwrap();
    let bytes = b"\x1b[31mhello\xff\x1b[0m API_KEY=synthetic-value tail";
    std::fs::write(&snapshot.log_path, bytes).unwrap();
    let full = read(
        &f,
        "/api/session-log",
        &format!("session_id={}&offset=0", id.0),
    );
    assert_eq!(full["size"], bytes.len());
    let decoded = STANDARD.decode(full["data_b64"].as_str().unwrap()).unwrap();
    assert_eq!(decoded, b"\x1b[31mhello\xff\x1b[0m API_KEY=*** tail");
    assert_eq!(full["length"], decoded.len());
    let db = f.core.details(id).unwrap().db_id.unwrap();
    assert_eq!(
        read(
            &f,
            "/api/session-log",
            &format!("session_db_id={}&limit=4", db.0)
        )["data_b64"],
        STANDARD.encode(b"tail")
    );
    assert_eq!(
        read(
            &f,
            "/api/session-log",
            &format!("session_id={}&offset=99999", id.0)
        )["length"],
        0
    );
}
#[test]
fn persisted_log_lookup_precedes_bad_range_and_does_not_trim_ids() {
    let f = fixture(true);
    error(
        f.http.handle_read_authenticated(
            &request("/api/session-log", "session_db_id=%201%20"),
            &f.config,
        ),
        400,
        "invalid session_db_id",
    );
    error(
        f.http.handle_read_authenticated(
            &request("/api/session-log", "session_id=1&limit=bad&offset=bad"),
            &f.config,
        ),
        404,
        "session log not found",
    );
    let db = start(&f, 1, "range");
    let path = f
        .store
        .as_ref()
        .unwrap()
        .session_overview_by_session_id(db)
        .unwrap()
        .log_path;
    std::fs::write(path, vec![b'x'; 600_000]).unwrap();
    assert_eq!(
        read(&f, "/api/session-log", "session_id=1")["length"],
        128 * 1024
    );
    assert_eq!(
        read(&f, "/api/session-log", "session_id=1&limit=999999")["length"],
        512 * 1024
    );
    error(
        f.http.handle_read_authenticated(
            &request("/api/session-log", "session_id=1&limit=0"),
            &f.config,
        ),
        400,
        "invalid limit",
    );
    error(
        f.http.handle_read_authenticated(
            &request("/api/session-log", "session_id=1&offset=bad"),
            &f.config,
        ),
        400,
        "invalid offset",
    );
    assert_eq!(
        read(
            &f,
            "/api/session-log",
            "session_id=1&limit=1&offset=-9223372036854775808"
        )["offset"],
        599999
    );
}
#[test]
fn metadata_path_source_shapes_and_signed_ids() {
    for path in [
        "/api/session/1/meta",
        "/api/session/+1/meta/",
        "/api/session///1/meta///",
        "/api/sessions/1/meta",
    ] {
        assert_eq!(metadata_path(path).unwrap(), LiveSessionId(1));
    }
    error(
        metadata_path("/api/session/0/meta").unwrap_err(),
        400,
        "invalid session id",
    );
    error(
        metadata_path("/api/session/x/unknown").unwrap_err(),
        404,
        "not found",
    );
    error(
        metadata_path("/api/sessions/x/unknown").unwrap_err(),
        400,
        "invalid session id",
    );
    error(
        metadata_path("/api/session/1/meta/extra").unwrap_err(),
        404,
        "not found",
    );
    assert!(!is_metadata_route("/api/sessions/1/inject"));
}
async fn patch(f: &Fixture, owner: &HubTaskOwner, id: LiveSessionId, bytes: &[u8]) -> Response {
    f.http
        .handle_metadata_authenticated(
            &Request {
                method: "PATCH".into(),
                path: format!("/api/session/{}/meta", id.0),
                body: bytes.to_vec(),
                ..Default::default()
            },
            f.sink.clone(),
            &owner.handle(),
        )
        .await
}
#[tokio::test]
async fn metadata_decodes_first_value_case_duplicates_null_and_explicit_clears() {
    let f = fixture(true);
    let id = register(&f).await;
    let owner = HubTaskOwner::new(tokio::runtime::Handle::current());
    let update=body(patch(&f,&owner,id,br#"{"LABEL":"first","label":" last\nline  name ","pinned":true,"color":" PURPLE ","note":" memo "} trailing junk"#).await);
    assert_eq!(update["session_meta"]["label"], "lastline name");
    assert_eq!(update["session_meta"]["color"], "purple");
    let meta = f
        .store
        .as_ref()
        .unwrap()
        .session_card_meta_by_live_session(id)
        .unwrap();
    assert_eq!(meta.label, "lastline name");
    assert!(meta.pinned);
    let cleared = body(
        patch(
            &f,
            &owner,
            id,
            br#"{"label":"","pinned":false,"color":"","note":"","auto_title":"not writable"}"#,
        )
        .await,
    );
    assert_eq!(
        cleared["session_meta"],
        json!({"label":"","pinned":false,"color":"","note":"","auto_title":""})
    );
    for empty in [
        b"{}".as_slice(),
        b"null",
        br#"{"label":null,"pinned":null}"#,
    ] {
        error(
            patch(&f, &owner, id, empty).await,
            400,
            "at least one meta field is required",
        );
    }
    error(
        patch(&f, &owner, id, br#"{"pinned":"true"}"#).await,
        400,
        "invalid json",
    );
}
#[tokio::test]
async fn metadata_invalid_color_retains_early_memory_changes_without_persistence() {
    let f = fixture(true);
    let id = register(&f).await;
    let owner = HubTaskOwner::new(tokio::runtime::Handle::current());
    body(
        patch(
            &f,
            &owner,
            id,
            br#"{"label":"stored","note":"stored note"}"#,
        )
        .await,
    );
    error(
        patch(
            &f,
            &owner,
            id,
            br#"{"label":"memory only","pinned":true,"color":"invalid","note":"must not apply"}"#,
        )
        .await,
        400,
        "invalid session color",
    );
    let snapshot = f.core.snapshot(id).unwrap();
    assert_eq!(snapshot.label, "memory only");
    assert!(snapshot.pinned);
    assert_eq!(snapshot.note, "stored note");
    assert_eq!(
        f.store
            .as_ref()
            .unwrap()
            .session_card_meta_by_live_session(id)
            .unwrap()
            .label,
        "stored"
    );
    error(
        patch(&f, &owner, LiveSessionId(999), br#"{"color":"invalid"}"#).await,
        404,
        "session not found",
    );
}
#[tokio::test]
async fn metadata_storage_failure_keeps_memory_and_suppresses_ui_broadcast() {
    let f = fixture(true);
    let id = register(&f).await;
    let owner = HubTaskOwner::new(tokio::runtime::Handle::current());
    let ui = UiBinding {
        connection: UiConnectionId(1),
        auth_epoch: f.core.auth_epoch(),
    };
    f.core.attach_ui(ui, None, None).unwrap();
    f.core.finish_ui_priming(ui).unwrap();
    f.store.as_ref().unwrap().close().unwrap();
    error(
        patch(&f, &owner, id, br#"{"label":"visible in memory"}"#).await,
        500,
        "failed to save session metadata",
    );
    assert_eq!(f.core.snapshot(id).unwrap().label, "visible in memory");
    assert!(f.sink.messages.lock().unwrap().is_empty());
}
#[tokio::test]
async fn metadata_unicode_limits_and_unrelated_fields_are_preserved() {
    let f = fixture(false);
    let id = register(&f).await;
    let owner = HubTaskOwner::new(tokio::runtime::Handle::current());
    let input = json!({"label":"日".repeat(125),"note":"語".repeat(170),"pinned":true});
    body(patch(&f, &owner, id, &serde_json::to_vec(&input).unwrap()).await);
    body(patch(&f, &owner, id, br#"{"color":"blue"}"#).await);
    let snapshot = f.core.snapshot(id).unwrap();
    assert_eq!(snapshot.label.chars().count(), 120);
    assert_eq!(snapshot.note.chars().count(), 160);
    assert!(snapshot.pinned);
}
#[test]
fn metadata_json_body_limit_uses_first_value_without_eof_requirement() {
    use crate::hub::http::{JSON_BODY_LIMIT, decode_json};
    let mut bytes = br#"{"label":"safe"}"#.to_vec();
    bytes.resize(JSON_BODY_LIMIT + 50, b'x');
    let _: super::metadata::MetadataBody = decode_json(&Request {
        body: bytes,
        ..Default::default()
    })
    .unwrap();
    let bytes = format!("{{\"label\":\"{}\"}}", "x".repeat(JSON_BODY_LIMIT));
    assert!(
        decode_json::<super::metadata::MetadataBody>(&Request {
            body: bytes.into_bytes(),
            ..Default::default()
        })
        .is_err()
    );
}
#[tokio::test]
async fn info_uses_explicit_identity_preferences_and_transition_only_stale_effects() {
    let mut f = fixture(false);
    register(&f).await;
    let ui = UiBinding {
        connection: UiConnectionId(1),
        auth_epoch: f.core.auth_epoch(),
    };
    f.core.attach_ui(ui, None, None).unwrap();
    f.core.finish_ui_priming(ui).unwrap();
    f.config.user_prefs.avatar = "/synthetic/avatar.bin".into();
    let mut context = info_context();
    let initial = f.http.handle_info_authenticated(&f.config, &context);
    assert_eq!(
        initial.response.headers.get("Cache-Control").unwrap(),
        "no-store"
    );
    assert!(initial.effects.0.is_empty());
    let value = body(initial.response);
    assert_eq!(value["cwd"], "/synthetic/project");
    assert_eq!(value["active_sessions"], 1);
    assert_eq!(value["userAvatar"], "/api/avatar");
    assert_eq!(value["userDisplayName"], "synthetic-user");
    assert_eq!(value["custom_providers"], json!([]));
    assert_eq!(value["role_permission"], json!({}));
    assert_eq!(
        value["child_permission_preview"]["claude"][""]["tier"],
        "attended"
    );
    assert_eq!(value["child_permission_default"], "full");
    context.binary_stale = true;
    let stale = f.http.handle_info_authenticated(&f.config, &context);
    assert_eq!(stale.effects.0.len(), 1);
    f.sink.apply(stale.effects).await.unwrap();
    assert!(
        f.http
            .handle_info_authenticated(&f.config, &context)
            .effects
            .0
            .is_empty()
    );
    context.binary_stale = false;
    f.sink
        .apply(
            f.http
                .handle_info_authenticated(&f.config, &context)
                .effects,
        )
        .await
        .unwrap();
    let messages = f.sink.messages.lock().unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].binary_stale, Some(true));
    assert_eq!(messages[1].binary_stale, Some(false));
}
#[test]
fn info_environment_precedence_and_registry_capabilities_match_source() {
    let mut f = fixture(false);
    let mut context = info_context();
    context.runtime_mode = "wsl";
    context.ssh = true;
    context.net_hint_ssh = true;
    context.net_hint_host = "203.0.113.10";
    assert_eq!(
        body(
            f.http
                .handle_info_authenticated(&f.config, &context)
                .response
        )["env_kind"],
        "remote-tunnel"
    );
    f.config.hub.env_kind = "remote".into();
    assert_eq!(
        body(
            f.http
                .handle_info_authenticated(&f.config, &context)
                .response
        )["env_kind"],
        "remote"
    );
    context.env_kind_override = "unknown explicit";
    let explicit = body(
        f.http
            .handle_info_authenticated(&f.config, &context)
            .response,
    );
    assert_eq!(explicit["env_kind"], "local");
    assert_eq!(explicit["host_ip"], "203.0.113.10");
    assert_eq!(explicit["runtime_label"], "WSL Linux");
    f.config
        .user_prefs
        .spawn
        .role_permission
        .insert("review".into(), "bounded".into());
    f.config
        .user_prefs
        .spawn
        .role_permission
        .insert("bad".into(), "bogus".into());
    let mut definitions = crate::profile::registry::embedded_definitions().unwrap();
    let codex = definitions.iter_mut().find(|d| d.id == "codex").unwrap();
    codex.launch.as_mut().unwrap().effort_levels = vec!["synthetic-effort".into()];
    let registry = crate::profile::registry::Registry::build(
        Layers {
            embedded: Some(definitions),
            ..Default::default()
        },
        &crate::profile::registry::default_adapters(),
    );
    context.registry = Some(&registry);
    let value = body(
        f.http
            .handle_info_authenticated(&f.config, &context)
            .response,
    );
    assert_eq!(value["effort_levels"]["codex"], json!(["synthetic-effort"]));
    assert_eq!(value["role_permission"], json!({"review":"bounded"}));
}

struct PausedSink {
    inner: Arc<Sink>,
    pause_once: std::sync::atomic::AtomicBool,
    entered: tokio::sync::Notify,
    resume: tokio::sync::Notify,
}
impl CoreEffectSink for PausedSink {
    fn apply<'a>(&'a self, effects: CoreEffects) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
        Box::pin(async move {
            if self
                .pause_once
                .swap(false, std::sync::atomic::Ordering::SeqCst)
            {
                self.entered.notify_one();
                self.resume.notified().await;
            }
            self.inner.apply(effects).await
        })
    }
}
#[tokio::test]
async fn dropping_http_waiter_cannot_cancel_accepted_metadata_persistence() {
    let f = fixture(true);
    let id = register(&f).await;
    let owner = HubTaskOwner::new(tokio::runtime::Handle::current());
    let sink = Arc::new(PausedSink {
        inner: f.sink.clone(),
        pause_once: std::sync::atomic::AtomicBool::new(true),
        entered: tokio::sync::Notify::new(),
        resume: tokio::sync::Notify::new(),
    });
    let ui = UiBinding {
        connection: UiConnectionId(1),
        auth_epoch: f.core.auth_epoch(),
    };
    f.core.attach_ui(ui, None, None).unwrap();
    f.core.finish_ui_priming(ui).unwrap();
    let waiter = tokio::spawn({
        let http = f.http.clone();
        let sink = sink.clone();
        let tasks = owner.handle();
        async move {
            http.handle_metadata_authenticated(
                &Request {
                    method: "PATCH".into(),
                    path: format!("/api/sessions/{}/meta", id.0),
                    body: br#"{"label":"survives disconnect"}"#.to_vec(),
                    ..Default::default()
                },
                sink,
                &tasks,
            )
            .await
        }
    });
    tokio::time::timeout(Duration::from_secs(2), sink.entered.notified())
        .await
        .unwrap();
    assert_eq!(f.core.snapshot(id).unwrap().label, "survives disconnect");
    assert_eq!(
        f.store
            .as_ref()
            .unwrap()
            .session_card_meta_by_live_session(id)
            .unwrap()
            .label,
        ""
    );
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    sink.resume.notify_one();
    owner.stop_requests();
    owner.drain_requests().await;
    owner.stop_effects().unwrap();
    tokio::time::timeout(Duration::from_secs(2), owner.drain_effects())
        .await
        .unwrap();
    assert_eq!(
        f.store
            .as_ref()
            .unwrap()
            .session_card_meta_by_live_session(id)
            .unwrap()
            .label,
        "survives disconnect"
    );
    assert_eq!(
        f.sink.messages.lock().unwrap()[0]
            .session_meta
            .as_ref()
            .unwrap()
            .label,
        "survives disconnect"
    );
}

#[tokio::test]
async fn failed_metadata_never_enters_priming_ui_queue() {
    let f = fixture(true);
    let id = register(&f).await;
    let owner = HubTaskOwner::new(tokio::runtime::Handle::current());
    let ui = UiBinding {
        connection: UiConnectionId(2),
        auth_epoch: f.core.auth_epoch(),
    };
    f.core.attach_ui(ui, None, None).unwrap();
    f.store.as_ref().unwrap().close().unwrap();
    error(
        patch(&f, &owner, id, br#"{"label":"memory change only"}"#).await,
        500,
        "failed to save session metadata",
    );
    assert!(f.core.finish_ui_priming(ui).unwrap().0.is_empty());
}

#[path = "loopback_tests.rs"]
mod loopback;
