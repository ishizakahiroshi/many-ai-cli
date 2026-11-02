//! Real JSONL and SQLite owners with independently failing sinks. Every path
//! and database belongs to this fixture; no product runtime is started.
use super::*;
use crate::storage::SqliteSessionStorage;
use std::{fs, sync::mpsc, time::Duration};

struct Fixture {
    _root: tempfile::TempDir,
    _installed: tempfile::TempDir,
    journal: SessionJournal,
    store: Arc<SqliteSessionStorage>,
    logs: JournalPaths,
    warnings: Arc<Mutex<Vec<(LiveSessionId, SessionError)>>>,
}
fn instant() -> Timestamp {
    Timestamp::UNIX_EPOCH + Duration::from_secs(1_700_000_000)
}
fn fixture(capacity: usize) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49125, installed.path()).unwrap();
    let mut options = StorageOptions::baseline(paths.resource(Resource::Logs));
    options.queue_capacity = capacity;
    let store = Arc::new(SqliteSessionStorage::open(&paths, options).unwrap());
    store
        .start_session(SessionStart {
            live_session_id: LiveSessionId(1),
            provider: "copilot".into(),
            started_at: "2023-11-14T22:13:20Z".into(),
            ..Default::default()
        })
        .unwrap();
    let journal = SessionJournal::new(
        paths,
        Some(store.clone()),
        JournalOptions {
            session_enabled: true,
            max_bytes: 0,
        },
    );
    let warnings = Arc::new(Mutex::new(Vec::new()));
    let captured = warnings.clone();
    journal.set_warning_handler(Arc::new(move |session, error| {
        lock(&captured).push((session, error.clone()));
    }));
    let logs = journal
        .open(
            LiveSessionId(1),
            "copilot",
            "/synthetic/project",
            instant(),
            false,
        )
        .unwrap();
    Fixture {
        _root: root,
        _installed: installed,
        journal,
        store,
        logs,
        warnings,
    }
}
fn attachment(name: &str) -> HistoryEvent {
    HistoryEvent(
        serde_json::json!({"type":"attach","filename":name,
            "path":format!("/synthetic/{name}"),"ts":"2023-11-14T22:13:20Z"})
        .as_object()
        .unwrap()
        .clone(),
    )
}
fn append(f: &Fixture, name: &str) {
    f.journal
        .apply_bound(
            SessionBinding {
                session: LiveSessionId(1),
                incarnation: SessionIncarnation(0),
                wrapper: WrapperConnectionId(0),
            },
            PersistenceBindingScope::Incarnation,
            PersistenceEffect::Event {
                session: LiveSessionId(1),
                event: attachment(name),
            },
        )
        .unwrap();
}
fn warning_details(f: &Fixture) -> Vec<String> {
    lock(&f.warnings)
        .iter()
        .map(|(session, error)| {
            assert_eq!(*session, LiveSessionId(1));
            match error {
                SessionError::Storage(error) => {
                    assert_eq!(error.kind, StorageErrorKind::Write);
                    error.detail.clone()
                }
                other => panic!("expected a storage warning, got {other:?}"),
            }
        })
        .collect()
}
fn jsonl_names(f: &Fixture) -> Vec<String> {
    fs::read_to_string(&f.logs.jsonl)
        .unwrap()
        .lines()
        .map(|line| {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(value["type"], "attach");
            value["filename"].as_str().unwrap().to_owned()
        })
        .collect()
}
fn stored_names(f: &Fixture) -> Vec<String> {
    let connection = rusqlite::Connection::open(f.store.database_path()).unwrap();
    let mut statement = connection
        .prepare("SELECT filename FROM attachments ORDER BY id")
        .unwrap();
    statement
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

async fn drain(f: &Fixture) -> ShutdownReport {
    let cancellation = HubShutdownCancellation::default();
    let report = tokio::time::timeout(
        Duration::from_secs(3),
        f.store.shutdown(
            ShutdownPolicy::Drain {
                timeout: Duration::from_secs(2),
            },
            &cancellation,
        ),
    )
    .await
    .unwrap();
    assert!(!report.timed_out);
    assert!(!report.cancelled);
    assert_eq!(report.remaining, 0);
    report
}

#[tokio::test]
async fn jsonl_write_failure_keeps_independent_sqlite_attachment_writes_and_reopen_control() {
    let f = fixture(4);
    let bytes_before = fs::read(&f.logs.jsonl).unwrap();
    {
        let mut writers = lock(&f.journal.writers);
        let writer = writers
            .get_mut(&LiveSessionId(1))
            .unwrap()
            .writer
            .as_mut()
            .unwrap();
        // Replace only the test-owned descriptor. Actual File::write_all on a
        // read-only OS handle fails; no mocked writer or runtime seam is used.
        writer.file = Some(File::open(&f.logs.jsonl).unwrap());
    }
    append(&f, "first-accepted-by-sqlite.txt");
    append(&f, "second-accepted-by-sqlite.txt");
    assert_eq!(fs::read(&f.logs.jsonl).unwrap(), bytes_before);
    assert_eq!(
        warning_details(&f),
        [
            "session history write failed",
            "session history writer is closed"
        ]
    );
    f.journal
        .open(
            LiveSessionId(1),
            "copilot",
            "/synthetic/project",
            instant(),
            true,
        )
        .unwrap();
    append(&f, "positive-reopened-writer.txt");
    f.journal.close_all();
    let report = drain(&f).await;
    assert_eq!(
        stored_names(&f),
        [
            "first-accepted-by-sqlite.txt",
            "second-accepted-by-sqlite.txt",
            "positive-reopened-writer.txt"
        ]
    );
    assert_eq!(jsonl_names(&f), ["positive-reopened-writer.txt"]);
    assert_eq!(warning_details(&f).len(), 2);
    assert_eq!(report.written, 3);
}

#[tokio::test]
async fn closed_sqlite_enqueue_has_its_own_warning_while_jsonl_keeps_the_attachment() {
    let f = fixture(4);
    append(&f, "positive-before-close.txt");
    let report = drain(&f).await;
    assert!(warning_details(&f).is_empty());
    append(&f, "jsonl-only-after-close.txt");
    f.journal.close_all();
    assert_eq!(warning_details(&f), ["session history database is closed"]);
    assert_eq!(stored_names(&f), ["positive-before-close.txt"]);
    assert_eq!(
        jsonl_names(&f),
        ["positive-before-close.txt", "jsonl-only-after-close.txt"]
    );
    assert_eq!(report.written, 1);
}

struct WriterGate(Option<mpsc::Sender<()>>);
impl WriterGate {
    fn release(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}
impl Drop for WriterGate {
    fn drop(&mut self) {
        // Always release before the fixture closes its actual writer thread,
        // including when a test assertion panics while the channel is full.
        self.release();
    }
}

#[tokio::test]
async fn full_sqlite_queue_warns_and_drops_only_database_copy_while_jsonl_keeps_both_attachments() {
    let f = fixture(1);
    {
        let connection = rusqlite::Connection::open(f.store.database_path()).unwrap();
        connection.execute_batch(
            "CREATE TRIGGER fail_gate_event BEFORE INSERT ON events WHEN NEW.type='synthetic_gate' BEGIN SELECT RAISE(ABORT,'synthetic bounded gate'); END"
        ).unwrap();
    }
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let receiver = Mutex::new(release_rx);
    let mut gate = WriterGate(Some(release_tx));
    f.store
        .set_on_write_error(Some(Arc::new(move |session, error| {
            assert_eq!(session, LiveSessionId(1));
            assert_eq!(error.kind, StorageErrorKind::Write);
            entered_tx.send(()).unwrap();
            lock(&receiver)
                .recv_timeout(Duration::from_secs(3))
                .expect("the test-owned writer gate must be released");
        })));
    let event = HistoryEvent(
        serde_json::json!({"type":"synthetic_gate"})
            .as_object()
            .unwrap()
            .clone(),
    );
    assert!(matches!(
        f.store.store_event_async(LiveSessionId(1), event),
        EnqueueOutcome::Queued { .. }
    ));
    entered_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    // The real writer is now in its error callback, outside SQLite/history
    // locks. One accepted entry fills the actual one-slot channel exactly.
    append(&f, "queued-in-both-sinks.txt");
    append(&f, "dropped-only-from-database.txt");
    assert_eq!(
        warning_details(&f),
        ["session history queue is full; event was dropped"]
    );
    assert_eq!(
        jsonl_names(&f),
        ["queued-in-both-sinks.txt", "dropped-only-from-database.txt"]
    );
    gate.release();
    f.store.set_on_write_error(None);
    f.journal.close_all();
    let report = drain(&f).await;
    assert_eq!(stored_names(&f), ["queued-in-both-sinks.txt"]);
    assert_eq!(report.written, 1);
    assert_eq!(report.dropped, 1);
    assert_eq!(report.remaining, 0);
    assert_eq!(warning_details(&f).len(), 1);
}
