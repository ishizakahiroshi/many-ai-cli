//! Additive Go-compatible schema, one physical SQLite connection.
use super::*;

pub(super) fn open_connection(
    path: &Path,
    init_timeout: Duration,
    trial: bool,
) -> StorageResult<(Connection, bool)> {
    let flags = if trial {
        rusqlite::OpenFlags::default() | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW
    } else {
        rusqlite::OpenFlags::default()
    };
    let conn = Connection::open_with_flags(path, flags)
        .map_err(|e| sql_error(StorageErrorKind::Open, e))?;
    let fts = initialize(&conn, init_timeout)?;
    Ok((conn, fts))
}
fn initialize(conn: &Connection, init_timeout: Duration) -> StorageResult<bool> {
    // The timeout must precede even journal mode: the first pragma can contend.
    conn.busy_timeout(Duration::from_secs(3))
        .map_err(|e| sql_error(StorageErrorKind::Open, e))?;
    deadline(conn, Instant::now() + init_timeout)
        .map_err(|e| sql_error(StorageErrorKind::Open, e))?;
    let initialized = (|| -> rusqlite::Result<bool> {
        conn.execute_batch("PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON; PRAGMA auto_vacuum=INCREMENTAL; PRAGMA journal_mode=WAL;")?;
        conn.execute_batch(include_str!("schema.sql"))?;
        let session_columns = [
            ("title", "TEXT"), ("tags_json", "TEXT"), ("summary", "TEXT"),
            ("parent_session_id", "INTEGER NOT NULL DEFAULT 0"), ("role", "TEXT"),
            ("auto", "INTEGER NOT NULL DEFAULT 0"), ("depth", "INTEGER NOT NULL DEFAULT 0"),
            ("orchestration_id", "TEXT"), ("board_path", "TEXT"), ("worktree_branch", "TEXT"),
            ("pinned", "INTEGER NOT NULL DEFAULT 0"), ("color", "TEXT"), ("note", "TEXT"),
            ("auto_title", "TEXT"), ("archived", "INTEGER NOT NULL DEFAULT 0"), ("subscription_id", "TEXT"),
        ];
        ensure_columns(conn, "sessions", &session_columns)?;
        ensure_columns(conn, "approvals", &[("provider", "TEXT"), ("candidate_key", "TEXT"), ("source_epoch", "INTEGER NOT NULL DEFAULT 0"), ("block", "TEXT")])?;
        // Absence of FTS is a supported capability, not a failed database open.
        Ok(conn.execute_batch("CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(text, raw_text, content='messages', content_rowid='id')").is_ok())
    })().map_err(|e| sql_error(StorageErrorKind::Open, e));
    conn.progress_handler(0, None::<fn() -> bool>)
        .map_err(|e| sql_error(StorageErrorKind::Open, e))?;
    initialized
}
fn ensure_columns(
    conn: &Connection,
    table: &str,
    columns: &[(&str, &str)],
) -> rusqlite::Result<()> {
    let have = conn
        .prepare(&format!("PRAGMA table_info({table})"))?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (name, declaration) in columns {
        if !have.iter().any(|s| s == name) {
            conn.execute_batch(&format!(
                "ALTER TABLE {table} ADD COLUMN {name} {declaration}"
            ))?;
        }
    }
    Ok(())
}
pub(super) fn deadline(conn: &Connection, at: Instant) -> rusqlite::Result<()> {
    conn.progress_handler(1000, Some(move || Instant::now() >= at))
}
pub(super) fn apply_pending_reset(directory: &Dir) -> StorageResult<()> {
    match directory.metadata(directory::RESET_MARKER) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => {
            return Err(error(
                StorageErrorKind::Open,
                "database reset marker could not be read safely",
            ));
        }
    }
    // Every unlink is relative to the held directory. Renaming an ancestor
    // cannot redirect recovery to a replacement pathname's database/sidecars.
    for name in [directory::DATABASE, directory::WAL, directory::SHM] {
        match directory.remove_file(name) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                return Err(error(
                    StorageErrorKind::Open,
                    "pending database reset could not remove a database file",
                ));
            }
        }
    }
    directory.remove_file(directory::RESET_MARKER).map_err(|_| {
        error(
            StorageErrorKind::Open,
            "pending reset marker could not be removed",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    #[test]
    fn unavailable_fts_is_a_supported_initialization_capability() {
        let root = tempfile::tempdir().unwrap();
        let c = Connection::open(root.path().join("synthetic.db")).unwrap();
        c.authorizer(Some(|ctx: AuthContext<'_>| {
            if matches!(ctx.action, AuthAction::CreateVtable { .. }) {
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }))
        .unwrap();
        assert!(!initialize(&c, Duration::from_secs(30)).unwrap());
        c.execute(
            "INSERT INTO sessions(live_session_id,jsonl_path) VALUES (1,'synthetic')",
            [],
        )
        .unwrap();
        assert_eq!(
            c.query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
    #[test]
    fn failed_pending_file_removal_keeps_the_recovery_marker() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("any-ai-cli.db");
        std::fs::create_dir(&path).unwrap();
        std::fs::write(companion(&path, ".reset-pending"), "synthetic marker").unwrap();
        assert!(apply_pending_reset(&Dir::open(root.path()).unwrap()).is_err());
        assert!(companion(&path, ".reset-pending").exists());
    }
    #[test]
    fn busy_timeout_precedes_journal_initialization() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("synthetic.db");
        let c = Connection::open(&path).unwrap();
        c.execute_batch("CREATE TABLE synthetic_lock(value); BEGIN EXCLUSIVE")
            .unwrap();
        let copy = path.clone();
        let begin = Instant::now();
        let waiter = thread::spawn(move || open_connection(&copy, Duration::from_secs(30), true));
        thread::sleep(Duration::from_millis(80));
        c.execute_batch("COMMIT").unwrap();
        let (_, fts) = waiter.join().unwrap().unwrap();
        assert!(fts);
        assert!(begin.elapsed() >= Duration::from_millis(80));
    }
}
