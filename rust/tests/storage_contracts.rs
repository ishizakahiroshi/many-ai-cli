//! Synthetic-only repository contracts against Go 21d0bc7; Hub/native acceptance is separate.
use many_ai_cli::{
    config::{Resource, RuntimePaths},
    proto::core::*,
    storage::SqliteSessionStorage,
};
use rusqlite::{Connection, params};
use serde_json::json;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

struct Fixture {
    _trial: tempfile::TempDir,
    _installed: tempfile::TempDir,
    paths: RuntimePaths,
}
impl Fixture {
    fn new() -> Self {
        let trial = tempfile::tempdir().unwrap();
        let installed = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::trial(trial.path(), 49111, installed.path()).unwrap();
        Self {
            _trial: trial,
            _installed: installed,
            paths,
        }
    }
    fn open(&self) -> SqliteSessionStorage {
        SqliteSessionStorage::open(
            &self.paths,
            StorageOptions::baseline(self.paths.resource(Resource::Logs)),
        )
        .unwrap()
    }
    fn sql(&self) -> Connection {
        Connection::open(self.paths.resource(Resource::Database)).unwrap()
    }
}
fn start(id: i64, provider: &str) -> SessionStart {
    SessionStart {
        live_session_id: LiveSessionId(id),
        provider: provider.into(),
        display: provider.into(),
        cwd: "/synthetic/project".into(),
        started_at: "2025-01-01T00:00:00Z".into(),
        ..Default::default()
    }
}
fn event(kind: &str, body: &str) -> HistoryEvent {
    HistoryEvent(
        json!({"type":kind,"text":body,"ts":"2025-01-01T01:00:00Z"})
            .as_object()
            .unwrap()
            .clone(),
    )
}
fn detected(id: i64, sig: &str, seconds: u64) -> ApprovalDetected {
    ApprovalDetected {
        live_session_id: LiveSessionId(id),
        sig: sig.into(),
        source: "transcript".into(),
        kind: "marker".into(),
        provider: String::new(),
        question: "Continue?".into(),
        context: "synthetic".into(),
        block: "Q1 Continue?\n1. yes\n2. no".into(),
        candidate_key: format!("key-{sig}"),
        source_epoch: ApprovalSourceEpoch(3),
        options: vec![],
        detected_at: Some(UNIX_EPOCH + Duration::from_secs(seconds)),
    }
}
fn scalar(c: &Connection, sql: &str) -> i64 {
    c.query_row(sql, [], |r| r.get(0)).unwrap()
}

#[test]
fn restore_search_prune_and_all_session_metadata() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let id = store.start_session(start(1, "copilot")).unwrap();
    for (kind, body) in [
        ("session_start", ""),
        ("user_input", "調査して\r"),
        ("pty_output", "SQLite retention search result\n"),
        ("pty_output", "second chunk\n"),
    ] {
        store
            .store_event(LiveSessionId(1), event(kind, body))
            .unwrap();
    }
    let messages = store
        .chat_messages_by_live_session(LiveSessionId(1), 100)
        .unwrap()
        .unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, "user");
    assert_eq!(
        messages[1].raw_text,
        "SQLite retention search result\nsecond chunk"
    );
    assert_eq!(
        store
            .chat_messages_by_session_id(id, 100)
            .unwrap()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        store
            .search_messages("retention", 10)
            .unwrap()
            .unwrap()
            .len(),
        1
    );
    let meta = store
        .update_session_meta(
            LiveSessionId(1),
            " SQLite work ",
            &["sqlite".into(), "retention".into(), "sqlite".into()],
            " summary text ",
            false,
        )
        .unwrap();
    assert_eq!(meta.tags, vec!["sqlite", "retention"]);
    assert_eq!(meta.title, "SQLite work");
    assert_eq!(meta.summary, "summary text");
    let timeline = store
        .timeline_by_live_session(LiveSessionId(1), 10)
        .unwrap()
        .unwrap();
    assert_eq!(timeline.len(), 2);
    assert_eq!(timeline[1].r#type, "user_input");
    let list = store.list_sessions(10, false).unwrap().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].message_count, 3);
    assert_eq!(list[0].event_count, 2);
    let usage = store.usage_summary().unwrap();
    assert_eq!(usage.total_sessions, 1);
    assert_eq!(usage.total_messages, 3);
    let bucket = &usage.providers.unwrap()[0];
    assert_eq!(bucket.ai_msgs, 2);
    assert_eq!(bucket.user_msgs, 1);
    store.update_session_messages(LiveSessionId(1), "do not overwrite first", "last updated");
    store.update_session_state(LiveSessionId(1), "running", "2025-01-02T01:00:00Z");
    let overview = store.session_overview_by_session_id(id).unwrap();
    assert_eq!(overview.first_message, "調査して");
    assert_eq!(overview.last_message, "last updated");
    assert_eq!(overview.state, "running");
    store.end_session(
        LiveSessionId(1),
        "completed",
        "done",
        UNIX_EPOCH + Duration::from_secs(1735862400),
    );
    assert_eq!(
        store
            .stale_sessions(SystemTime::now(), 10)
            .unwrap()
            .unwrap()
            .len(),
        1
    );
    store
        .store_event(LiveSessionId(1), event("late", "not recorded"))
        .unwrap();
    assert_eq!(
        store
            .timeline_by_live_session(LiveSessionId(1), 10)
            .unwrap()
            .unwrap()
            .len(),
        2
    );
    store.prune_older_than(SystemTime::now()).unwrap();
    assert!(
        store
            .chat_messages_by_live_session(LiveSessionId(1), 100)
            .unwrap()
            .is_none()
    );
    assert_eq!(store.usage_summary().unwrap().total_sessions, 0);
}
#[test]
fn stable_database_identity_reattach_stale_close_card_and_virtual_paths() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let mut original = start(8, "claude");
    original.jsonl_path = "synthetic-one.jsonl".into();
    original.label = "wrapper label".into();
    let one = store.start_session(original.clone()).unwrap();
    store
        .update_session_card_meta(
            LiveSessionId(8),
            SessionCardMeta {
                label: "edited".into(),
                pinned: true,
                color: "blue".into(),
                note: "note".into(),
                auto_title: "title".into(),
            },
        )
        .unwrap();
    store
        .store_event(LiveSessionId(8), event("user_input", "old row"))
        .unwrap();
    assert_eq!(
        store
            .close_stale_sessions(SystemTime::now(), "restart")
            .unwrap(),
        1
    );
    let revived = store.start_session(original).unwrap();
    assert_eq!(revived, one);
    let card = store
        .session_card_meta_by_live_session(LiveSessionId(8))
        .unwrap();
    assert_eq!(card.label, "edited");
    assert!(card.pinned);
    assert_eq!(card.note, "note");
    store.end_session(LiveSessionId(8), "completed", "", SystemTime::now());
    let mut fresh = start(8, "codex");
    fresh.jsonl_path = "synthetic-two.jsonl".into();
    let two = store.start_session(fresh).unwrap();
    assert_ne!(one, two);
    store
        .store_event(LiveSessionId(8), event("user_input", "new row"))
        .unwrap();
    assert_eq!(
        store.chat_messages_by_session_id(one, 10).unwrap().unwrap()[0].raw_text,
        "old row"
    );
    assert_eq!(
        store
            .chat_messages_by_live_session(LiveSessionId(8), 10)
            .unwrap()
            .unwrap()[0]
            .raw_text,
        "new row"
    );
    assert_eq!(
        store
            .session_overview_by_live_session(LiveSessionId(8))
            .unwrap()
            .id,
        two
    );
    let a = store.start_session(start(21, "claude")).unwrap();
    let b = store.start_session(start(22, "claude")).unwrap();
    assert_ne!(a, b);
    assert_eq!(
        store.session_overview_by_session_id(a).unwrap().jsonl_path,
        "virtual-live-21"
    );
    assert_eq!(
        store
            .session_card_meta_by_live_session(LiveSessionId(999))
            .unwrap()
            .label,
        ""
    );
}
#[test]
fn approval_ledger_marker_resolution_latest_and_redetection() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let id = store.start_session(start(4, "claude")).unwrap();
    store.start_session(start(5, "codex")).unwrap();
    store.store_approval_detected(detected(4, "old", 100));
    store.store_approval_detected(detected(5, "new", 200));
    store.store_approval_consumed(
        LiveSessionId(4),
        "old",
        "2",
        UNIX_EPOCH + Duration::from_secs(110),
    );
    let rows = store.recent_approvals(0, false).unwrap().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].sig, "new");
    assert_eq!(rows[1].state, "resolved");
    assert_eq!(rows[1].selected_text, "2");
    assert_eq!(rows[1].provider, "claude");
    assert_eq!(rows[1].source_epoch, ApprovalSourceEpoch(3));
    assert_eq!(
        store
            .approvals_by_session_id(id, 0, true)
            .unwrap()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        store
            .approvals_by_live_session(LiveSessionId(4), 0, false)
            .unwrap()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(store.recent_approvals(0, true).unwrap().unwrap().len(), 1);
    assert_eq!(
        store
            .latest_approval_of_kinds(LiveSessionId(4), "transcript", &["marker".into()])
            .unwrap()
            .unwrap()
            .sig,
        "old"
    );
    assert!(
        store
            .latest_approval_of_kinds(LiveSessionId(4), "native", &["marker".into()])
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .latest_approval_of_kinds(LiveSessionId(4), "transcript", &[])
            .unwrap()
            .is_none()
    );
    store.store_approval_detected(detected(4, "old", 300));
    let row = store
        .approvals_by_session_id(id, 1, true)
        .unwrap()
        .unwrap()
        .remove(0);
    assert_eq!(row.state, "pending");
    assert_eq!(row.resolved_at, "");
    assert_eq!(row.selected_text, "2");
    assert!(row.block.starts_with("Q1"));
    assert!(
        store
            .approvals_by_live_session(LiveSessionId(999), 0, false)
            .unwrap()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn transcript_selection_noise_masking_and_user_only_mentions() {
    let fixture = Fixture::new();
    let store = fixture.open();
    for (id, provider) in [
        (1, "claude"),
        (2, "codex"),
        (3, "copilot"),
        (4, "command-code"),
    ] {
        store.start_session(start(id, provider)).unwrap();
        store
            .store_event(LiveSessionId(id), event("pty_output", "real output"))
            .unwrap();
        let rows = store
            .chat_messages_by_live_session(LiveSessionId(id), 0)
            .unwrap()
            .unwrap();
        assert_eq!(rows.len(), usize::from(id > 2));
    }
    for noise in [
        "",
        "·",
        "Thinking",
        "Working",
        "✳ Imploring… (12s · ↑3.2k tokens · esc to interrupt)",
        "thinking with medium effort✳3thinking with medium effort\n✶✷✻ still thinking with medium effort",
        "1.8924 Opus ↑111.0k ↓764\nauto mode on (shift+tab to cycle)",
    ] {
        store
            .store_event(LiveSessionId(3), event("pty_output", noise))
            .unwrap();
    }
    for body in ["OK", "No", "はい", "\x1b[31mcolored\x1b[0m\r\ntext"] {
        store
            .store_event(LiveSessionId(3), event("pty_output", body))
            .unwrap();
    }
    assert_eq!(
        store
            .session_overview_by_live_session(LiveSessionId(3))
            .unwrap()
            .message_count,
        5
    );
    assert!(
        store
            .timeline_by_live_session(LiveSessionId(3), 0)
            .unwrap()
            .is_none()
    );
    store
        .store_event(
            LiveSessionId(3),
            event("pty_output", "/synthetic/ai-only.md"),
        )
        .unwrap();
    assert!(
        !store
            .messages_mention_text(LiveSessionId(3), &["/synthetic/ai-only.md".into()])
            .unwrap()
    );
    store
        .store_event(
            LiveSessionId(3),
            event("user_input", "read /synthetic/user.md"),
        )
        .unwrap();
    assert!(
        store
            .messages_mention_text(LiveSessionId(3), &["".into(), "/synthetic/user.md".into()])
            .unwrap()
    );
    store
        .store_event(
            LiveSessionId(1),
            event("user_input", "OPENAI_API_KEY=synthetic-key-value\r"),
        )
        .unwrap();
    assert_eq!(
        store
            .chat_messages_by_live_session(LiveSessionId(1), 10)
            .unwrap()
            .unwrap()[0]
            .raw_text,
        "OPENAI_API_KEY=***"
    );
}
#[test]
fn clear_and_reset_preserve_only_latest_metadata_and_remove_all_children() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let id = store.start_session(start(1, "copilot")).unwrap();
    store.start_session(start(2, "codex")).unwrap();
    for live in [1, 2] {
        store
            .store_event(LiveSessionId(live), event("user_input", "before reset"))
            .unwrap();
        store
            .store_event(
                LiveSessionId(live),
                HistoryEvent(
                    json!({"type":"attach","path":"/synthetic/file.png","filename":"file.png"})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            )
            .unwrap();
        store.store_approval_detected(detected(live, "approval", 1));
        store
            .update_session_meta(
                LiveSessionId(live),
                "title",
                &["tag".into()],
                "summary",
                false,
            )
            .unwrap();
    }
    store.clear_session_history(LiveSessionId(1)).unwrap();
    let overview = store.session_overview_by_session_id(id).unwrap();
    assert_eq!(overview.message_count, 0);
    assert_eq!(overview.title, "");
    assert_eq!(overview.first_message, "");
    assert_eq!(
        scalar(&fixture.sql(), "SELECT COUNT(*) FROM attachments"),
        1
    );
    store.clear_session_history(LiveSessionId(999)).unwrap();
    let result = store
        .reset_history(&[LiveSessionId(1), LiveSessionId(1), LiveSessionId(-1)])
        .unwrap();
    assert_eq!(result.sessions, 2);
    assert_eq!(result.messages, 2);
    assert_eq!(result.attachments, 1);
    assert_eq!(result.preserved, 1);
    assert_eq!(store.history_generation(), HistoryGeneration(1));
    assert_eq!(store.list_sessions(0, true).unwrap().unwrap().len(), 1);
    assert_eq!(store.session_overview_by_session_id(id).unwrap().id, id);
    assert!(store.search_messages("before", 10).unwrap().is_none());
    store
        .store_event(LiveSessionId(1), event("user_input", "after reset"))
        .unwrap();
    assert_eq!(
        store.search_messages("after", 10).unwrap().unwrap().len(),
        1
    );
    assert!(!store.file_reset_pending());
}
#[test]
fn old_schema_migrations_and_pending_file_reset_recovery() {
    let fixture = Fixture::new();
    let c = fixture.sql();
    c.execute_batch(include_str!("fixtures/core/storage/old-schema.sql"))
        .unwrap();
    drop(c);
    let store = fixture.open();
    assert_eq!(
        store
            .session_overview_by_session_id(DbSessionId(1))
            .unwrap()
            .provider,
        "copilot"
    );
    let approval = store
        .approvals_by_session_id(DbSessionId(1), 10, false)
        .unwrap()
        .unwrap();
    assert_eq!(approval.len(), 1);
    assert_eq!(approval[0].source_epoch, ApprovalSourceEpoch(0));
    let c = fixture.sql();
    assert_eq!(
        scalar(
            &c,
            "SELECT COUNT(*) FROM pragma_table_info('sessions') WHERE name IN ('subscription_id','auto_title','pinned','summary')"
        ),
        4
    );
    drop(c);
    store.schedule_file_reset().unwrap();
    assert!(store.file_reset_pending());
    store.close().unwrap();
    let reopened = fixture.open();
    assert!(!reopened.file_reset_pending());
    assert_eq!(reopened.usage_summary().unwrap().total_sessions, 0);
    assert_eq!(
        reopened.database_path().file_name().unwrap(),
        "any-ai-cli.db"
    );
}
#[test]
fn fts_query_error_falls_back_to_literal_like_and_index_failure_keeps_message() {
    let fixture = Fixture::new();
    let store = fixture.open();
    store.start_session(start(1, "copilot")).unwrap();
    assert!(store.fts_enabled());
    fixture
        .sql()
        .execute_batch("DROP TABLE messages_fts")
        .unwrap();
    for body in [
        "go_test",
        "goXtest",
        "100% match",
        "1000 match",
        "folder\\path",
        "folderXpath",
    ] {
        store
            .store_event(LiveSessionId(1), event("user_input", body))
            .unwrap();
    }
    for needle in ["go_test", "100%", "folder\\path"] {
        let rows = store.search_messages(needle, 10).unwrap().unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].text.contains(needle));
    }
    assert_eq!(store.usage_summary().unwrap().total_messages, 6);
}
#[test]
fn full_close_drains_every_accepted_event_and_is_idempotent() {
    let fixture = Fixture::new();
    let store = fixture.open();
    store.start_session(start(1, "copilot")).unwrap();
    let mut queued = 0;
    for n in 0..4096 {
        if matches!(
            store.store_event_async(
                LiveSessionId(1),
                event("user_input", &format!("queued-{n}"))
            ),
            EnqueueOutcome::Queued { .. }
        ) {
            queued += 1;
        }
    }
    store.close().unwrap();
    store.close().unwrap();
    assert_eq!(
        scalar(&fixture.sql(), "SELECT COUNT(*) FROM messages"),
        queued
    );
    assert_eq!(
        store.store_event_async(LiveSessionId(1), event("late", "")),
        EnqueueOutcome::Closed
    );
    assert_eq!(
        store.usage_summary().err().unwrap().kind,
        StorageErrorKind::Closed
    );
}
#[tokio::test]
async fn shutdown_reports_write_errors_without_discarding_later_work() {
    let fixture = Fixture::new();
    let store = fixture.open();
    store.start_session(start(1, "copilot")).unwrap();
    let failures = Arc::new(AtomicUsize::new(0));
    let received = Arc::new(Mutex::new(Vec::new()));
    let count = failures.clone();
    let seen = received.clone();
    store.set_on_write_error(Some(Arc::new(move |id, error| {
        count.fetch_add(1, Ordering::SeqCst);
        seen.lock().unwrap().push((id, error));
    })));
    fixture.sql().execute_batch("CREATE TRIGGER reject_synthetic_event BEFORE INSERT ON events WHEN NEW.type='reject' BEGIN SELECT RAISE(ABORT,'synthetic-rejection'); END").unwrap();
    store.store_event_async(
        LiveSessionId(1),
        event("reject", "sensitive synthetic payload never in error"),
    );
    store.store_event_async(LiveSessionId(1), event("user_input", "accepted"));
    let report = store
        .shutdown(
            ShutdownPolicy::Drain {
                timeout: Duration::from_secs(10),
            },
            &HubShutdownCancellation::default(),
        )
        .await;
    assert_eq!(report.remaining, 0);
    assert_eq!(report.written, 1);
    assert_eq!(failures.load(Ordering::SeqCst), 1);
    assert!(report.error.is_some());
    assert!(!report.error.unwrap().detail.contains("payload"));
    assert_eq!(scalar(&fixture.sql(), "SELECT COUNT(*) FROM messages"), 1);
    store.set_on_write_error(None);
}
#[test]
fn chunked_prune_and_legacy_transcript_noise() {
    let fixture = Fixture::new();
    let store = fixture.open();
    let old = store.start_session(start(1, "claude")).unwrap();
    let live = store.start_session(start(2, "copilot")).unwrap();
    let c = fixture.sql();
    for (id, text) in [
        (old.0, "Thinking"),
        (old.0, "real answer"),
        (live.0, "Thinking"),
    ] {
        c.execute(
            "INSERT INTO messages(session_id,role,kind,text,raw_text) VALUES (?,'ai','text',?,?)",
            params![id, text, text],
        )
        .unwrap();
        let row = c.last_insert_rowid();
        c.execute(
            "INSERT INTO messages_fts(rowid,text,raw_text) VALUES (?,?,?)",
            params![row, text, text],
        )
        .unwrap();
    }
    assert_eq!(store.prune_transcript_noise().unwrap(), 1);
    assert_eq!(
        store
            .session_overview_by_session_id(old)
            .unwrap()
            .message_count,
        1
    );
    assert_eq!(
        store
            .session_overview_by_session_id(live)
            .unwrap()
            .message_count,
        1
    );
    c.execute_batch("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<4501) INSERT INTO events(session_id,type,payload_json) SELECT 1,'synthetic','{}' FROM n").unwrap();
    drop(c);
    store.end_session(LiveSessionId(1), "completed", "", UNIX_EPOCH);
    store.prune_older_than(SystemTime::now()).unwrap();
    assert_eq!(store.usage_summary().unwrap().total_sessions, 1);
    assert_eq!(store.session_overview_by_session_id(live).unwrap().id, live);
    assert_eq!(scalar(&fixture.sql(), "SELECT COUNT(*) FROM events"), 0);
}
#[test]
fn explicit_runtime_paths_reject_empty_and_outside_trial_logs() {
    let fixture = Fixture::new();
    assert!(
        SqliteSessionStorage::open(&fixture.paths, StorageOptions::baseline(Default::default()))
            .is_err()
    );
    assert!(
        SqliteSessionStorage::open(
            &fixture.paths,
            StorageOptions::baseline(fixture._installed.path().join("logs"))
        )
        .is_err()
    );
    assert!(!fixture._installed.path().join("any-ai-cli.db").exists());
}

/// Requires the Go toolchain on PATH or MANY_AI_GO_BINARY. The Go oracle is
/// invoked by exact filename with its build-ignore guard; it never starts a Hub.
#[test]
fn fixed_go_cross_read_and_rollback_continuation() {
    let root = tempfile::Builder::new()
        .prefix("many-ai-storage-oracle-")
        .tempdir()
        .unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49119, installed.path()).unwrap();
    let store = SqliteSessionStorage::open(
        &paths,
        StorageOptions::baseline(paths.resource(Resource::Logs)),
    )
    .unwrap();
    store.start_session(start(1, "copilot")).unwrap();
    store
        .update_session_card_meta(
            LiveSessionId(1),
            SessionCardMeta {
                label: "edited card".into(),
                ..Default::default()
            },
        )
        .unwrap();
    store
        .store_event(
            LiveSessionId(1),
            event("user_input", "synthetic rollback query"),
        )
        .unwrap();
    store
        .store_event(LiveSessionId(1), event("pty_output", "synthetic answer"))
        .unwrap();
    store.store_approval_detected(detected(1, "rollback-approval", 100));
    store.store_approval_consumed(
        LiveSessionId(1),
        "rollback-approval",
        "yes",
        UNIX_EPOCH + Duration::from_secs(110),
    );
    store.close().unwrap();
    std::fs::write(
        root.path().join("synthetic-storage-fixture"),
        "Go/Rust storage compatibility only\n",
    )
    .unwrap();
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    // Build only a verified snapshot of the baseline package files. This works
    // from immutable archives and shallow checkouts without any Git executable
    // or historical object, and excludes unlisted local Go files/go.work.
    let oracle_source = root.path().join("oracle-source");
    copy_verified_go_sources(repo, &oracle_source).unwrap();
    let helper = oracle_source.join("rollback-oracle.go");
    std::fs::write(
        &helper,
        include_bytes!("fixtures/core/storage/rollback-oracle.go"),
    )
    .unwrap();
    let go = std::env::var_os("MANY_AI_GO_BINARY").unwrap_or_else(|| "go".into());
    let mut command = std::process::Command::new(go);
    command
        .current_dir(&oracle_source)
        .args([
            "run",
            "-mod=readonly",
            "-buildvcs=false",
            "rollback-oracle.go",
        ])
        .arg(root.path())
        .env("GOTOOLCHAIN", "local")
        .env("GOWORK", "off")
        .env_remove("GOFLAGS");
    for variable in ["GOCACHE", "GOMODCACHE", "GOPATH", "XDG_CACHE_HOME"] {
        let value = std::env::var_os(variable).unwrap_or_else(|| {
            root.path()
                .join(variable.to_ascii_lowercase())
                .into_os_string()
        });
        command.env(variable, value);
    }
    let result = command
        .output()
        .expect("Go toolchain required: set MANY_AI_GO_BINARY or include Go on PATH");
    assert!(
        result.status.success(),
        "Go oracle failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let receipt: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(
        receipt,
        json!({"go_baseline":"21d0bc7935a2c4696fb89ccff2e324157a528c2d","sessions":1,"messages":2,"approvals":1,"search":1,"go_write":true})
    );
    let reopened = SqliteSessionStorage::open(
        &paths,
        StorageOptions::baseline(paths.resource(Resource::Logs)),
    )
    .unwrap();
    let messages = reopened
        .chat_messages_by_live_session(LiveSessionId(1), 100)
        .unwrap()
        .unwrap();
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[2].raw_text, "synthetic Go rollback continuation");
    assert_eq!(
        reopened
            .search_messages("continuation", 10)
            .unwrap()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn read_limits_ordering_and_nil_empty_shapes_match_baseline() {
    let fixture = Fixture::new();
    let store = fixture.open();
    assert!(store.list_sessions(0, true).unwrap().is_none());
    assert!(store.search_messages("", 0).unwrap().is_none());
    assert!(
        store
            .recent_approvals(0, false)
            .unwrap()
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .chat_messages_by_live_session(LiveSessionId(999), 0)
            .unwrap()
            .is_none()
    );
    let id = store.start_session(start(1, "copilot")).unwrap();
    assert!(
        store
            .chat_messages_by_session_id(id, 0)
            .unwrap()
            .unwrap()
            .is_empty()
    );
    let mut c = fixture.sql();
    let tx = c.transaction().unwrap();
    for n in 0..1005 {
        tx.execute("INSERT INTO messages(session_id,ts,role,kind,text,raw_text) VALUES (?,'2025-01-01T00:00:00Z','user','text',?,?)",params![id.0,format!("message-{n}"),format!("message-{n}")]).unwrap();
        tx.execute(
            "INSERT INTO events(session_id,type,payload_json) VALUES (?,'event','{}')",
            [id.0],
        )
        .unwrap();
        if n < 505 {
            tx.execute(
                "INSERT INTO approvals(session_id,sig,state,detected_at) VALUES (?,?,'pending',?)",
                params![id.0, format!("sig-{n}"), format!("{:04}", n)],
            )
            .unwrap();
        }
    }
    tx.commit().unwrap();
    drop(c);
    let messages = store
        .chat_messages_by_session_id(id, 1001)
        .unwrap()
        .unwrap();
    assert_eq!(messages.len(), 400);
    assert_eq!(messages[0].raw_text, "message-605");
    assert_eq!(messages[399].raw_text, "message-1004");
    assert_eq!(
        store
            .chat_messages_by_session_id(id, 1000)
            .unwrap()
            .unwrap()
            .len(),
        1000
    );
    assert_eq!(
        store
            .timeline_by_live_session(LiveSessionId(1), 2001)
            .unwrap()
            .unwrap()
            .len(),
        400
    );
    assert_eq!(
        store
            .approvals_by_session_id(id, 1000, false)
            .unwrap()
            .unwrap()
            .len(),
        500
    );
    let approvals = store.recent_approvals(0, false).unwrap().unwrap();
    assert_eq!(approvals.len(), 100);
    assert_eq!(approvals[0].sig, "sig-504");
    store
        .update_session_meta(
            LiveSessionId(1),
            &"😀".repeat(200),
            &(0..20).map(|i| format!(" tag-{i} ")).collect::<Vec<_>>(),
            &"界".repeat(4001),
            true,
        )
        .unwrap();
    let row = store.session_overview_by_session_id(id).unwrap();
    assert_eq!(row.title.chars().count(), 160);
    assert_eq!(row.summary.chars().count(), 4000);
    assert_eq!(row.tags.len(), 12);
    assert!(store.list_sessions(0, false).unwrap().is_none());
}
#[test]
fn event_and_clear_failures_rollback_all_related_rows() {
    let fixture = Fixture::new();
    let store = fixture.open();
    store.start_session(start(1, "copilot")).unwrap();
    fixture.sql().execute_batch("CREATE TRIGGER reject_message BEFORE INSERT ON messages BEGIN SELECT RAISE(ABORT,'synthetic'); END").unwrap();
    assert!(
        store
            .store_event(LiveSessionId(1), event("user_input", "must rollback"))
            .is_err()
    );
    assert_eq!(scalar(&fixture.sql(), "SELECT COUNT(*) FROM events"), 0);
    assert_eq!(
        store
            .session_overview_by_live_session(LiveSessionId(1))
            .unwrap()
            .first_message,
        ""
    );
    fixture
        .sql()
        .execute_batch("DROP TRIGGER reject_message")
        .unwrap();
    store
        .store_event(LiveSessionId(1), event("user_input", "keep on failure"))
        .unwrap();
    fixture.sql().execute_batch("CREATE TRIGGER reject_clear BEFORE DELETE ON events BEGIN SELECT RAISE(ABORT,'synthetic'); END").unwrap();
    assert!(store.clear_session_history(LiveSessionId(1)).is_err());
    assert_eq!(
        store
            .chat_messages_by_live_session(LiveSessionId(1), 10)
            .unwrap()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(store.search_messages("keep", 10).unwrap().unwrap().len(), 1);
}
#[cfg(unix)]
#[test]
fn trial_database_and_companions_refuse_outside_symlinks() {
    use std::os::unix::fs::symlink;
    for suffix in ["", "-wal", "-shm", ".reset-pending"] {
        let fixture = Fixture::new();
        let outside = fixture._installed.path().join("synthetic-outside");
        std::fs::write(&outside, b"untouched").unwrap();
        symlink(
            &outside,
            fixture._trial.path().join(format!("any-ai-cli.db{suffix}")),
        )
        .unwrap();
        assert!(
            SqliteSessionStorage::open(
                &fixture.paths,
                StorageOptions::baseline(fixture.paths.resource(Resource::Logs))
            )
            .is_err()
        );
        assert_eq!(std::fs::read(outside).unwrap(), b"untouched");
    }
}

/// Verify and copy exactly the committed source manifest. Extra checkout files
/// never enter the oracle module. LF normalization supports Git text checkouts
/// on Windows without changing the canonical pinned source digest.
fn copy_verified_go_sources(
    source: &std::path::Path,
    destination: &std::path::Path,
) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    #[derive(serde::Deserialize)]
    struct Source {
        path: String,
        sha256: String,
    }
    #[derive(serde::Deserialize)]
    struct Manifest {
        baseline: String,
        files: Vec<Source>,
    }
    let manifest: Manifest = serde_json::from_str(include_str!(
        "fixtures/core/storage/go-source-manifest.json"
    ))
    .map_err(|e| e.to_string())?;
    if manifest.baseline != "21d0bc7935a2c4696fb89ccff2e324157a528c2d" {
        return Err("unexpected Go baseline manifest".into());
    }
    for file in manifest.files {
        let relative = std::path::Path::new(&file.path);
        if relative
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
        {
            return Err("invalid source manifest path".into());
        }
        let raw = std::fs::read(source.join(relative))
            .map_err(|_| format!("missing Go oracle source {}", file.path))?;
        let canonical = raw
            .iter()
            .enumerate()
            .filter_map(|(i, b)| {
                if *b == b'\r' && raw.get(i + 1) == Some(&b'\n') {
                    None
                } else {
                    Some(*b)
                }
            })
            .collect::<Vec<_>>();
        let digest = Sha256::digest(&canonical)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        if digest != file.sha256 {
            return Err(format!(
                "Go oracle source differs from the fixed manifest: {}",
                file.path
            ));
        }
        let target = destination.join(relative);
        std::fs::create_dir_all(target.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(target, canonical).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[test]
fn oracle_manifest_works_without_git_and_rejects_source_changes() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let archive = tempfile::tempdir().unwrap();
    copy_verified_go_sources(repo, archive.path()).unwrap();
    assert!(!archive.path().join(".git").exists());
    // A local extra Go file cannot change what gets compiled by the oracle.
    std::fs::write(
        archive.path().join("internal/sessionstore/unlisted.go"),
        "package sessionstore\nfunc init() { panic(\"must never compile\") }\n",
    )
    .unwrap();
    let snapshot = tempfile::tempdir().unwrap();
    copy_verified_go_sources(archive.path(), snapshot.path()).unwrap();
    assert!(
        !snapshot
            .path()
            .join("internal/sessionstore/unlisted.go")
            .exists()
    );
    let source = archive.path().join("internal/sessionstore/store.go");
    let original = std::fs::read_to_string(&source).unwrap();
    std::fs::write(&source, original.replace('\n', "\r\n")).unwrap();
    let windows_copy = tempfile::tempdir().unwrap();
    copy_verified_go_sources(archive.path(), windows_copy.path()).unwrap();
    std::fs::write(&source, "package sessionstore\n// synthetic mutation\n").unwrap();
    let rejected = tempfile::tempdir().unwrap();
    assert!(
        copy_verified_go_sources(archive.path(), rejected.path())
            .unwrap_err()
            .contains("differs")
    );
}
