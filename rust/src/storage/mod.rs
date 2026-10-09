//! Go-compatible SQLite history repository (Go oracle 21d0bc7).
//! This is a repository implementation, not proof of Hub caller integration.
use crate::proto::time::Timestamp;
mod directory;
mod history;
mod repository;
mod schema;
mod text;
pub use text::{mask_secret_bytes, mask_secrets};
mod writer;

use crate::{
    config::{Resource, RuntimePaths},
    files::safe_fs::Dir,
    proto::core::*,
};
use rusqlite::{Connection, OptionalExtension, Row, params, params_from_iter};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering},
        mpsc::{self, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// Every handle uses one physical connection. All persisted paths are derived
/// from RuntimePaths, including the recovery marker and SQLite WAL companions.
pub struct SqliteSessionStorage {
    inner: Arc<Inner>,
    writer: Mutex<Option<JoinHandle<()>>>,
    close_lock: Mutex<()>,
    writer_id: thread::ThreadId,
}
struct Inner {
    connection: Mutex<Option<Connection>>,
    path: PathBuf,
    directory: Arc<Dir>,
    fts: bool,
    query_timeout: Duration,
    history: RwLock<()>,
    generation: AtomicU64,
    sender: Mutex<Option<SyncSender<QueuedHistoryEvent>>>,
    closing: AtomicBool,
    done: AtomicBool,
    stop: AtomicBool,
    pending: AtomicU64,
    written: AtomicU64,
    discarded: AtomicU64,
    dropped: AtomicI64,
    handler: RwLock<Option<WriteErrorHandler>>,
    last_error: Mutex<Option<StorageError>>,
}
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
fn error(kind: StorageErrorKind, detail: &str) -> StorageError {
    StorageError {
        kind,
        detail: detail.to_owned(),
    }
}
fn sql_error(kind: StorageErrorKind, e: rusqlite::Error) -> StorageError {
    // Extended codes are diagnostic without disclosing SQL, paths or user data.
    match e {
        rusqlite::Error::SqliteFailure(code, _) => StorageError {
            kind: if code.code == rusqlite::ErrorCode::OperationInterrupted {
                StorageErrorKind::Timeout
            } else {
                kind
            },
            detail: format!("SQLite failure {}", code.extended_code),
        },
        rusqlite::Error::ToSqlConversionFailure(_) => error(
            StorageErrorKind::InvalidData,
            "database value is outside the supported range",
        ),
        rusqlite::Error::QueryReturnedNoRows => error(kind, "database row was not found"),
        _ => error(kind, "database operation failed"),
    }
}
fn companion(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}
impl Inner {
    fn with_conn<T>(
        &self,
        kind: StorageErrorKind,
        f: impl FnOnce(&mut Connection) -> rusqlite::Result<T>,
    ) -> StorageResult<T> {
        let until = Instant::now() + self.query_timeout;
        let mut guard = loop {
            match self.connection.try_lock() {
                Ok(g) => break g,
                Err(std::sync::TryLockError::Poisoned(e)) => break e.into_inner(),
                Err(std::sync::TryLockError::WouldBlock) => {
                    if Instant::now() >= until {
                        return Err(error(
                            StorageErrorKind::Timeout,
                            "database connection wait expired",
                        ));
                    }
                    thread::sleep(Duration::from_millis(1));
                }
            }
        };
        let conn = guard
            .as_mut()
            .ok_or_else(|| error(StorageErrorKind::Closed, "database is closed"))?;
        let remaining = until.saturating_duration_since(Instant::now());
        // busy_timeout is connection-local and SQLite does not invoke its VM
        // progress hook while waiting for another writer. Include lock-wait time
        // in the same query deadline, rounding up the final millisecond.
        let busy_ms = remaining.as_millis().saturating_add(1).min(3000) as u64;
        conn.busy_timeout(Duration::from_millis(busy_ms))
            .map_err(|e| sql_error(kind.clone(), e))?;
        schema::deadline(conn, until).map_err(|e| sql_error(kind.clone(), e))?;
        let result = f(conn).map_err(|e| sql_error(kind.clone(), e));
        conn.progress_handler(0, None::<fn() -> bool>)
            .map_err(|e| sql_error(kind.clone(), e))?;
        conn.busy_timeout(Duration::from_secs(3))
            .map_err(|e| sql_error(kind, e))?;
        result
    }
    fn notify(&self, session: LiveSessionId, result: StorageResult<()>) {
        if let Err(err) = result {
            *lock(&self.last_error) = Some(err.clone());
            let callback = self
                .handler
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            if let Some(handler) = callback {
                handler(session, err);
            }
        }
    }
}
impl SqliteSessionStorage {
    /// Stable path is useful to an explicitly authorized backup/export caller.
    pub fn database_path(&self) -> &Path {
        &self.inner.path
    }
    pub fn fts_enabled(&self) -> bool {
        self.inner.fts
    }
    fn with_conn<T>(
        &self,
        kind: StorageErrorKind,
        f: impl FnOnce(&mut Connection) -> rusqlite::Result<T>,
    ) -> StorageResult<T> {
        if self.inner.closing.load(Ordering::Acquire) {
            return Err(error(StorageErrorKind::Closed, "database is closing"));
        }
        self.inner.with_conn(kind, f)
    }
    fn execute(&self, session: LiveSessionId, sql: &str, p: impl rusqlite::Params) {
        let result = self.with_conn(StorageErrorKind::Write, |c| c.execute(sql, p).map(|_| ()));
        self.inner.notify(session, result);
    }
}
impl Drop for SqliteSessionStorage {
    fn drop(&mut self) {
        let _ = self.close();
    }
}
/// Shared Go-compatible local-offset formatter; unsupported input is an error.
fn timestamp(time: Timestamp) -> rusqlite::Result<String> {
    crate::proto::time::format_rfc3339(time)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
}
fn now() -> String {
    // Supported operating system clocks are inside the RFC3339 year range.
    crate::proto::time::format_rfc3339(Timestamp::now())
        .expect("system clock outside RFC3339 range")
}
fn normalized_limit(value: i64, max: i64, default: i64) -> i64 {
    if value <= 0 || value > max {
        default
    } else {
        value
    }
}
fn nullable<T>(rows: Vec<T>) -> StoredRows<T> {
    if rows.is_empty() { None } else { Some(rows) }
}
fn resolve(c: &Connection, id: LiveSessionId, active: bool) -> rusqlite::Result<Option<i64>> {
    if id.0 <= 0 {
        return Ok(None);
    }
    c.query_row(if active { "SELECT id FROM sessions WHERE live_session_id=? AND ended_at IS NULL ORDER BY id DESC LIMIT 1" } else { "SELECT id FROM sessions WHERE live_session_id=? ORDER BY id DESC LIMIT 1" }, [id.0], |r| r.get(0)).optional()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fixture(capacity: usize) -> (tempfile::TempDir, tempfile::TempDir, SqliteSessionStorage) {
        let root = tempfile::tempdir().unwrap();
        let installed = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::trial(root.path(), 49113, installed.path()).unwrap();
        let mut options = StorageOptions::baseline(paths.resource(Resource::Logs));
        options.queue_capacity = capacity;
        let store = SqliteSessionStorage::open(&paths, options).unwrap();
        store
            .start_session(SessionStart {
                live_session_id: LiveSessionId(1),
                provider: "copilot".into(),
                ..Default::default()
            })
            .unwrap();
        (root, installed, store)
    }
    fn event(n: u64) -> HistoryEvent {
        HistoryEvent(
            json!({"type":"user_input","text":format!("synthetic-{n}")})
                .as_object()
                .unwrap()
                .clone(),
        )
    }
    #[test]
    fn bounded_queue_drop_count_is_nonblocking_under_database_backpressure() {
        let (_root, _installed, store) = fixture(2);
        let barrier = store.inner.history.write().unwrap();
        // Writer may remove one event and wait on barrier. Subsequent sends fill
        // the two-element channel without waiting for SQLite or the barrier.
        let started = Instant::now();
        let mut queued = 0;
        let mut dropped = 0;
        for n in 0..100 {
            match store.store_event_async(LiveSessionId(1), event(n)) {
                EnqueueOutcome::Queued { .. } => queued += 1,
                EnqueueOutcome::Dropped { cumulative_count } => {
                    dropped += 1;
                    assert_eq!(cumulative_count, dropped);
                }
                other => panic!("unexpected enqueue {other:?}"),
            }
        }
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(queued <= 3);
        assert_eq!(queued + dropped, 100);
        drop(barrier);
        store.close().unwrap();
        assert_eq!(store.shutdown_report().written, queued as u64);
        assert_eq!(store.shutdown_report().dropped, dropped as u64);
    }
    #[test]
    fn reset_barrier_discards_old_queue_and_accepts_the_new_generation() {
        let (_root, _installed, store) = fixture(128);
        let barrier = store.inner.history.write().unwrap();
        for n in 0..100 {
            assert!(matches!(
                store.store_event_async(LiveSessionId(1), event(n)),
                EnqueueOutcome::Queued {
                    generation: HistoryGeneration(0)
                }
            ));
        }
        // Exercise the same reset body under its actual write barrier so no
        // scheduler-dependent write can hide a queued-generation regression.
        let result = store.reset_under_barrier(&[LiveSessionId(1)]).unwrap();
        assert_eq!(result.preserved, 1);
        assert!(matches!(
            store.store_event_async(LiveSessionId(1), event(100)),
            EnqueueOutcome::Queued {
                generation: HistoryGeneration(1)
            }
        ));
        drop(barrier);
        store.close().unwrap();
        let report = store.shutdown_report();
        assert_eq!(report.discarded_old_generation, 100);
        assert_eq!(report.written, 1);
        assert_eq!(report.remaining, 0);
        let c = Connection::open(store.database_path()).unwrap();
        let body: String = c
            .query_row("SELECT text FROM messages", [], |r| r.get(0))
            .unwrap();
        assert_eq!(body, "synthetic-100");
    }
    #[test]
    fn single_connection_pragmas_reapply_on_every_explicit_open() {
        let (root, installed, store) = fixture(2);
        for _ in 0..2 {
            store
                .inner
                .with_conn(StorageErrorKind::Query, |c| {
                    for (pragma, expected) in [
                        ("foreign_keys", 1),
                        ("busy_timeout", 3000),
                        ("synchronous", 1),
                        ("auto_vacuum", 2),
                    ] {
                        assert_eq!(
                            c.query_row(&format!("PRAGMA {pragma}"), [], |r| r.get::<_, i64>(0))?,
                            expected
                        );
                    }
                    assert_eq!(
                        c.query_row("PRAGMA journal_mode", [], |r| r.get::<_, String>(0))?,
                        "wal"
                    );
                    Ok(())
                })
                .unwrap();
        }
        store.close().unwrap();
        let paths = RuntimePaths::trial(root.path(), 49113, installed.path()).unwrap();
        let reopened = SqliteSessionStorage::open(
            &paths,
            StorageOptions::baseline(paths.resource(Resource::Logs)),
        )
        .unwrap();
        reopened
            .inner
            .with_conn(StorageErrorKind::Query, |c| {
                assert_eq!(
                    c.query_row("PRAGMA busy_timeout", [], |r| r.get::<_, i64>(0))?,
                    3000
                );
                assert_eq!(
                    c.query_row("PRAGMA foreign_keys", [], |r| r.get::<_, i64>(0))?,
                    1
                );
                Ok(())
            })
            .unwrap();
    }
    #[test]
    fn sqlite_vm_deadline_interrupts_expensive_queries() {
        let (_root, _installed, store) = fixture(2);
        let started = Instant::now();
        let err=store.inner.with_conn(StorageErrorKind::Query,|c|{
            schema::deadline(c,Instant::now()+Duration::from_millis(10))?;
            c.query_row("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<1000000000) SELECT sum(x) FROM n",[],|r|r.get::<_,i64>(0))
        }).unwrap_err();
        assert_eq!(err.kind, StorageErrorKind::Timeout);
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(store.usage_summary().unwrap().total_sessions, 1);
    }
    #[tokio::test]
    #[expect(
        clippy::readonly_write_lock,
        reason = "An exclusive barrier must block the reader-owned writer while shutdown timeout and cancellation are asserted"
    )]
    #[allow(
        clippy::await_holding_lock,
        reason = "The fixture intentionally stalls the writer to test shutdown timeout and cancellation"
    )]
    async fn drain_timeout_and_cancellation_leave_accepted_work_owned_until_close() {
        let (_root, _installed, store) = fixture(2);
        let barrier = store.inner.history.write().unwrap();
        store.store_event_async(LiveSessionId(1), event(1));
        let report = store
            .shutdown(
                ShutdownPolicy::Drain {
                    timeout: Duration::from_millis(10),
                },
                &HubShutdownCancellation::default(),
            )
            .await;
        assert!(report.timed_out);
        assert_eq!(report.remaining, 1);
        let cancelled = HubShutdownCancellation::default();
        cancelled.cancel();
        let report = store
            .shutdown(
                ShutdownPolicy::Drain {
                    timeout: Duration::from_secs(1),
                },
                &cancelled,
            )
            .await;
        assert!(report.cancelled);
        drop(barrier);
        store.close().unwrap();
        assert_eq!(store.shutdown_report().written, 1);
    }
    #[test]
    fn failed_reset_retains_recovery_marker() {
        let (_root, _installed, store) = fixture(2);
        store.inner.with_conn(StorageErrorKind::Write,|c|c.execute_batch("CREATE TRIGGER refuse_reset BEFORE DELETE ON sessions BEGIN SELECT RAISE(ABORT,'synthetic reset failure'); END")).unwrap();
        assert!(store.reset_history(&[]).is_err());
        assert!(store.file_reset_pending());
        assert_eq!(store.history_generation(), HistoryGeneration(1));
    }
    #[test]
    fn sqlite_busy_wait_uses_the_same_query_deadline() {
        let root = tempfile::tempdir().unwrap();
        let installed = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::trial(root.path(), 49114, installed.path()).unwrap();
        let mut options = StorageOptions::baseline(paths.resource(Resource::Logs));
        options.query_timeout = Duration::from_millis(25);
        let store = SqliteSessionStorage::open(&paths, options).unwrap();
        store
            .start_session(SessionStart {
                live_session_id: LiveSessionId(1),
                ..Default::default()
            })
            .unwrap();
        let blocker = Connection::open(store.database_path()).unwrap();
        blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
        let began = Instant::now();
        assert!(store.store_event(LiveSessionId(1), event(1)).is_err());
        assert!(began.elapsed() < Duration::from_millis(250));
        blocker.execute_batch("ROLLBACK").unwrap();
        store.store_event(LiveSessionId(1), event(2)).unwrap();
        assert_eq!(store.usage_summary().unwrap().total_messages, 1);
    }
    #[test]
    fn recovery_metadata_targets_database_identity_without_reviving_or_replacing_a_row() {
        let (_root, _installed, store) = fixture(8);
        let first = store
            .session_overview_by_live_session(LiveSessionId(1))
            .unwrap()
            .id;
        store.end_session(
            LiveSessionId(1),
            "completed",
            "synthetic-finished",
            Timestamp::now(),
        );
        let second = store
            .start_session(SessionStart {
                live_session_id: LiveSessionId(1),
                jsonl_path: "distinct-synthetic-log".into(),
                state: "running".into(),
                ..Default::default()
            })
            .unwrap();
        store
            .update_session_orchestration(
                first,
                &SessionOrchestrationMeta {
                    parent: LiveSessionId(9),
                    role: "review".into(),
                    auto: true,
                    depth: 1,
                    orchestration: OrchestrationId("synthetic-relay".into()),
                    board_path: "synthetic-board".into(),
                },
            )
            .unwrap();
        let old = store.session_overview_by_session_id(first).unwrap();
        let current = store.session_overview_by_session_id(second).unwrap();
        assert_eq!(old.parent_session_id, LiveSessionId(9));
        assert_eq!(old.state, "completed");
        assert_eq!(current.parent_session_id, LiveSessionId(0));
        assert_eq!(current.state, "running");
        let ended: bool = store
            .with_conn(StorageErrorKind::Query, |connection| {
                connection.query_row(
                    "SELECT ended_at IS NOT NULL FROM sessions WHERE id=?",
                    [first.0],
                    |r| r.get(0),
                )
            })
            .unwrap();
        assert!(ended);
    }
}
