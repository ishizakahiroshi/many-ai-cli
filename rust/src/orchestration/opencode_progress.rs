//! Read-only progress snapshots of OpenCode's shared session store.
//!
//! Only session identity, part/message kinds, states, tool names and timestamps
//! are selected. Message text, tool input/output, metadata and every other body
//! column stay in SQLite. Failures (missing file/table/column, busy database,
//! malformed JSON that cannot be skipped, unexpected value types) yield `None`
//! so callers can fall back to their existing idle detection.

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use std::{path::Path, time::Duration};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgressPhase {
    Thinking,
    Tool,
    Responding,
    BetweenSteps,
    Idle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenCodeProgress {
    pub phase: ProgressPhase,
    pub phase_started_ms: Option<i64>,
    pub tool: Option<String>,
    pub last_activity_ms: Option<i64>,
}

#[derive(Clone, Copy, Debug)]
pub enum SessionBinding<'a> {
    Title(&'a str),
    FirstInDirectory {
        directory: &'a str,
        created_after_ms: i64,
    },
}

const SESSION_BY_TITLE_SQL: &str =
    "SELECT id FROM session WHERE title = ?1 ORDER BY time_created DESC, id DESC LIMIT 1";
const SESSION_CANDIDATES_SQL: &str = "SELECT id, directory, time_created FROM session \
     WHERE time_created >= ?1 ORDER BY time_created ASC, id ASC";
// The newest assistant message decides whether a turn is in progress. An older
// unfinished one (an aborted turn) must not keep the session looking busy.
const LATEST_ASSISTANT_SQL: &str = "SELECT id, json_extract(data, '$.time.completed') IS NULL \
     FROM message WHERE session_id = ?1 \
     AND json_valid(data) AND json_extract(data, '$.role') = 'assistant' \
     ORDER BY time_created DESC, id DESC LIMIT 1";
// OpenCode keeps a tool part's times under `state.time` (the part itself has no `time`),
// while reasoning and text parts keep them under `time`. `time_updated` is the part's
// last write, so a tool that is still `pending` (no `state.time` yet) still counts as activity.
const SESSION_ACTIVITY_SQL: &str = "SELECT \
     max(COALESCE(json_extract(data, '$.state.time.start'), json_extract(data, '$.time.start'))), \
     max(COALESCE(json_extract(data, '$.state.time.end'), json_extract(data, '$.time.end'))), \
     max(time_updated) \
     FROM part WHERE session_id = ?1 AND json_valid(data)";
const MESSAGE_PARTS_SQL: &str = "SELECT json_extract(data, '$.type'), \
     json_extract(data, '$.state.status'), json_extract(data, '$.tool'), \
     COALESCE(json_extract(data, '$.state.time.start'), json_extract(data, '$.time.start')), \
     COALESCE(json_extract(data, '$.state.time.end'), json_extract(data, '$.time.end')), \
     time_created \
     FROM part WHERE session_id = ?1 AND message_id = ?2 AND json_valid(data)";

#[cfg(test)]
const SQL_STATEMENTS: &[&str] = &[
    SESSION_BY_TITLE_SQL,
    SESSION_CANDIDATES_SQL,
    LATEST_ASSISTANT_SQL,
    SESSION_ACTIVITY_SQL,
    MESSAGE_PARTS_SQL,
];

struct PartSnapshot {
    kind: Option<String>,
    status: Option<String>,
    tool: Option<String>,
    start: Option<i64>,
    end: Option<i64>,
    created: Option<i64>,
}

struct Candidate {
    start: Option<i64>,
    tool: Option<String>,
}

pub fn read_progress(db_path: &Path, binding: &SessionBinding<'_>) -> Option<OpenCodeProgress> {
    if !db_path.is_file() {
        return None;
    }
    let connection = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    connection.busy_timeout(Duration::from_millis(200)).ok()?;
    connection.execute_batch("PRAGMA query_only=ON").ok()?;
    let session_id = resolve_session(&connection, binding)?;
    let (last_activity_ms, last_end_ms) = session_activity(&connection, &session_id)?;
    let latest_message = connection
        .query_row(LATEST_ASSISTANT_SQL, params![session_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?))
        })
        .optional()
        .ok()?;
    let Some((message_id, true)) = latest_message else {
        return Some(OpenCodeProgress {
            phase: ProgressPhase::Idle,
            phase_started_ms: last_end_ms,
            tool: None,
            last_activity_ms,
        });
    };
    let parts = message_parts(&connection, &session_id, &message_id)?;
    Some(settle_progress(parts, last_activity_ms))
}

fn resolve_session(connection: &Connection, binding: &SessionBinding<'_>) -> Option<String> {
    match binding {
        SessionBinding::Title(title) => connection
            .query_row(SESSION_BY_TITLE_SQL, params![title], |row| {
                row.get::<_, String>(0)
            })
            .optional()
            .ok()?,
        SessionBinding::FirstInDirectory {
            directory,
            created_after_ms,
        } => {
            let wanted = normalize_directory(directory);
            let mut statement = connection.prepare(SESSION_CANDIDATES_SQL).ok()?;
            let rows = statement
                .query_map(params![created_after_ms], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                })
                .ok()?;
            let mut first: Option<(String, i64)> = None;
            for row in rows {
                let (id, directory, created) = row.ok()?;
                if normalize_directory(&directory) != wanted {
                    continue;
                }
                match &first {
                    None => first = Some((id, created)),
                    Some((_, first_created)) => {
                        if created - *first_created <= 2000 {
                            return None;
                        }
                        break;
                    }
                }
            }
            Some(first?.0)
        }
    }
}

fn normalize_directory(directory: &str) -> String {
    let normalized = directory
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_string();
    // Windows paths differ only by case (drive letter, folder names) between callers.
    if cfg!(windows) {
        normalized.to_lowercase()
    } else {
        normalized
    }
}

fn session_activity(
    connection: &Connection,
    session_id: &str,
) -> Option<(Option<i64>, Option<i64>)> {
    let row = connection
        .query_row(SESSION_ACTIVITY_SQL, params![session_id], |row| {
            let start: Option<i64> = row.get(0)?;
            let end: Option<i64> = row.get(1)?;
            let updated: Option<i64> = row.get(2)?;
            Ok((start, end, updated))
        })
        .optional()
        .ok()?;
    let (start, end, updated) = row.unwrap_or((None, None, None));
    Some((max_option(max_option(start, end), updated), end))
}

fn message_parts(
    connection: &Connection,
    session_id: &str,
    message_id: &str,
) -> Option<Vec<PartSnapshot>> {
    let mut statement = connection.prepare(MESSAGE_PARTS_SQL).ok()?;
    let rows = statement
        .query_map(params![session_id, message_id], |row| {
            Ok(PartSnapshot {
                kind: row.get(0)?,
                status: row.get(1)?,
                tool: row.get(2)?,
                start: row.get(3)?,
                end: row.get(4)?,
                created: row.get(5)?,
            })
        })
        .ok()?;
    let mut parts = Vec::new();
    for row in rows {
        parts.push(row.ok()?);
    }
    Some(parts)
}

fn settle_progress(parts: Vec<PartSnapshot>, last_activity_ms: Option<i64>) -> OpenCodeProgress {
    let mut tools = Vec::new();
    let mut thinking = Vec::new();
    let mut responding = Vec::new();
    let mut message_last_end = None;
    for part in parts {
        message_last_end = max_option(message_last_end, part.end);
        // A part that has not recorded its own start yet (a `pending` tool) began no
        // later than the row was created.
        let start = part.start.or(part.created);
        match part.kind.as_deref() {
            Some("tool") if matches!(part.status.as_deref(), Some("pending" | "running")) => {
                tools.push(Candidate {
                    start,
                    tool: clean_tool_name(part.tool),
                });
            }
            Some("reasoning") if part.end.is_none() => {
                thinking.push(Candidate { start, tool: None })
            }
            Some("text") if part.end.is_none() => responding.push(Candidate { start, tool: None }),
            _ => {}
        }
    }
    let (phase, chosen) = if !tools.is_empty() {
        (ProgressPhase::Tool, pick_latest(tools))
    } else if !thinking.is_empty() {
        (ProgressPhase::Thinking, pick_latest(thinking))
    } else if !responding.is_empty() {
        (ProgressPhase::Responding, pick_latest(responding))
    } else {
        (ProgressPhase::BetweenSteps, None)
    };
    let (phase_started_ms, tool) = match chosen {
        Some(candidate) => (candidate.start, candidate.tool),
        None => (message_last_end, None),
    };
    OpenCodeProgress {
        phase,
        phase_started_ms,
        tool,
        last_activity_ms,
    }
}

fn pick_latest(candidates: Vec<Candidate>) -> Option<Candidate> {
    candidates
        .into_iter()
        .max_by_key(|candidate| candidate.start.unwrap_or(i64::MIN))
}

fn clean_tool_name(raw: Option<String>) -> Option<String> {
    let name = raw?;
    if name.is_empty() || name.chars().count() > 64 {
        return None;
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-'))
    {
        return None;
    }
    Some(name)
}

fn max_option(a: Option<i64>, b: Option<i64>) -> Option<i64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, b) => b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{Connection, params};
    use serde_json::{Value, json};
    use std::path::PathBuf;
    use tempfile::TempDir;

    struct Fixture {
        path: PathBuf,
        connection: Connection,
        // Last: fields drop in this order, and the folder cannot be removed
        // on Windows while the connection still has the database open.
        dir: TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("opencode.db");
            let connection = Connection::open(&path).unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE session (id TEXT PRIMARY KEY, directory TEXT NOT NULL, \
                     title TEXT NOT NULL, time_created INTEGER NOT NULL);\
                     CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, \
                     time_created INTEGER NOT NULL, data TEXT NOT NULL);\
                     CREATE TABLE part (id TEXT PRIMARY KEY, message_id TEXT NOT NULL, \
                     session_id TEXT NOT NULL, time_created INTEGER NOT NULL, \
                     time_updated INTEGER NOT NULL, data TEXT NOT NULL);",
                )
                .unwrap();
            Self {
                dir,
                path,
                connection,
            }
        }

        fn session(&self, id: &str, directory: &str, title: &str, created: i64) {
            self.connection
                .execute(
                    "INSERT INTO session(id, directory, title, time_created) VALUES(?1,?2,?3,?4)",
                    params![id, directory, title, created],
                )
                .unwrap();
        }

        fn message(&self, id: &str, session: &str, created: i64, completed: Option<i64>) {
            let data = match completed {
                Some(completed) => json!({"role": "assistant", "time": {"completed": completed}}),
                None => json!({"role": "assistant", "time": {}}),
            };
            self.connection
                .execute(
                    "INSERT INTO message(id, session_id, time_created, data) VALUES(?1,?2,?3,?4)",
                    params![id, session, created, data.to_string()],
                )
                .unwrap();
        }

        fn part(&self, id: &str, session: &str, message: &str, data: &Value) {
            self.part_at(id, session, message, (0, 0), data);
        }

        /// `rows` is the part row's (time_created, time_updated).
        fn part_at(&self, id: &str, session: &str, message: &str, rows: (i64, i64), data: &Value) {
            self.connection
                .execute(
                    "INSERT INTO part(id, message_id, session_id, time_created, time_updated, data) \
                     VALUES(?1,?2,?3,?4,?5,?6)",
                    params![id, message, session, rows.0, rows.1, data.to_string()],
                )
                .unwrap();
        }

        fn part_raw(&self, id: &str, session: &str, message: &str, data: &str) {
            self.connection
                .execute(
                    "INSERT INTO part(id, message_id, session_id, time_created, time_updated, data) \
                     VALUES(?1,?2,?3,?4,?5,?6)",
                    params![id, message, session, 0, 0, data],
                )
                .unwrap();
        }

        fn read(&self, binding: &SessionBinding<'_>) -> Option<OpenCodeProgress> {
            read_progress(&self.path, binding)
        }
    }

    fn part_data(
        kind: &str,
        status: Option<&str>,
        tool: Option<&str>,
        start: Option<i64>,
        end: Option<i64>,
    ) -> Value {
        let mut data = json!({"type": kind, "time": {}});
        if let Some(start) = start {
            data["time"]["start"] = json!(start);
        }
        if let Some(end) = end {
            data["time"]["end"] = json!(end);
        }
        if let Some(tool) = tool {
            data["tool"] = json!(tool);
        }
        if let Some(status) = status {
            data["state"] = json!({"status": status});
        }
        data
    }

    /// A tool part in the shape OpenCode 1.19.5 writes: no part-level `time`, the times live
    /// under `state.time`, and a `pending` tool has no times at all.
    fn tool_data(status: &str, tool: &str, start: Option<i64>, end: Option<i64>) -> Value {
        let mut state = json!({"status": status});
        if start.is_some() || end.is_some() {
            state["time"] = json!({});
            if let Some(start) = start {
                state["time"]["start"] = json!(start);
            }
            if let Some(end) = end {
                state["time"]["end"] = json!(end);
            }
        }
        json!({"type": "tool", "tool": tool, "state": state})
    }

    fn base() -> Fixture {
        let fixture = Fixture::new();
        fixture.session("s-1", "/work/alpha", "title-001", 1_000);
        fixture.message("m-1", "s-1", 1_100, None);
        fixture
    }

    fn title_progress(fixture: &Fixture) -> Option<OpenCodeProgress> {
        fixture.read(&SessionBinding::Title("title-001"))
    }

    #[test]
    fn reasoning_without_end_is_thinking() {
        let fixture = base();
        fixture.part(
            "p-1",
            "s-1",
            "m-1",
            &part_data("reasoning", None, None, Some(1_200), None),
        );
        let progress = title_progress(&fixture).unwrap();
        assert_eq!(progress.phase, ProgressPhase::Thinking);
        assert_eq!(progress.phase_started_ms, Some(1_200));
        assert_eq!(progress.tool, None);
        assert_eq!(progress.last_activity_ms, Some(1_200));
    }

    #[test]
    fn pending_tool_is_tool() {
        let fixture = base();
        fixture.part(
            "p-1",
            "s-1",
            "m-1",
            &part_data("tool", Some("pending"), Some("bash"), Some(1_300), None),
        );
        let progress = title_progress(&fixture).unwrap();
        assert_eq!(progress.phase, ProgressPhase::Tool);
        assert_eq!(progress.phase_started_ms, Some(1_300));
        assert_eq!(progress.tool.as_deref(), Some("bash"));
    }

    #[test]
    fn running_tool_is_tool() {
        let fixture = base();
        fixture.part(
            "p-1",
            "s-1",
            "m-1",
            &part_data("tool", Some("running"), Some("read"), Some(1_400), None),
        );
        let progress = title_progress(&fixture).unwrap();
        assert_eq!(progress.phase, ProgressPhase::Tool);
        assert_eq!(progress.tool.as_deref(), Some("read"));
    }

    #[test]
    fn unfinished_text_is_responding() {
        let fixture = base();
        fixture.part(
            "p-1",
            "s-1",
            "m-1",
            &part_data("text", None, None, Some(1_500), None),
        );
        let progress = title_progress(&fixture).unwrap();
        assert_eq!(progress.phase, ProgressPhase::Responding);
        assert_eq!(progress.phase_started_ms, Some(1_500));
    }

    #[test]
    fn finished_parts_are_between_steps() {
        let fixture = base();
        fixture.part(
            "p-1",
            "s-1",
            "m-1",
            &part_data("reasoning", None, None, Some(1_200), Some(1_300)),
        );
        fixture.part(
            "p-2",
            "s-1",
            "m-1",
            &part_data(
                "tool",
                Some("completed"),
                Some("bash"),
                Some(1_300),
                Some(1_400),
            ),
        );
        fixture.part(
            "p-3",
            "s-1",
            "m-1",
            &part_data(
                "tool",
                Some("error"),
                Some("read"),
                Some(1_400),
                Some(1_450),
            ),
        );
        fixture.part(
            "p-4",
            "s-1",
            "m-1",
            &part_data("text", None, None, Some(1_450), Some(1_500)),
        );
        let progress = title_progress(&fixture).unwrap();
        assert_eq!(progress.phase, ProgressPhase::BetweenSteps);
        assert_eq!(progress.phase_started_ms, Some(1_500));
        assert_eq!(progress.tool, None);
        assert_eq!(progress.last_activity_ms, Some(1_500));
    }

    #[test]
    fn completed_assistant_message_is_idle() {
        let fixture = Fixture::new();
        fixture.session("s-1", "/work/alpha", "title-001", 1_000);
        fixture.message("m-1", "s-1", 1_100, Some(2_000));
        fixture.part(
            "p-1",
            "s-1",
            "m-1",
            &part_data("text", None, None, Some(1_500), Some(1_600)),
        );
        let progress = title_progress(&fixture).unwrap();
        assert_eq!(progress.phase, ProgressPhase::Idle);
        assert_eq!(progress.phase_started_ms, Some(1_600));
        assert_eq!(progress.last_activity_ms, Some(1_600));
    }

    #[test]
    fn no_messages_at_all_is_idle() {
        let fixture = Fixture::new();
        fixture.session("s-1", "/work/alpha", "title-001", 1_000);
        let progress = title_progress(&fixture).unwrap();
        assert_eq!(progress.phase, ProgressPhase::Idle);
        assert_eq!(progress.phase_started_ms, None);
        assert_eq!(progress.last_activity_ms, None);
    }

    #[test]
    fn unknown_title_returns_none() {
        let fixture = base();
        assert!(
            fixture
                .read(&SessionBinding::Title("title-absent"))
                .is_none()
        );
    }

    #[test]
    fn title_must_match_exactly() {
        let fixture = base();
        fixture.session("s-2", "/work/other", "title-001-extra", 1_200);
        assert!(fixture.read(&SessionBinding::Title("title-001")).is_some());
        assert!(
            fixture
                .read(&SessionBinding::Title("title-001-extr"))
                .is_none()
        );
    }

    #[test]
    fn missing_table_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("opencode.db");
        let _connection = Connection::open(&path).unwrap();
        assert!(read_progress(&path, &SessionBinding::Title("title-001")).is_none());
    }

    #[test]
    fn missing_column_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("opencode.db");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE session (id TEXT PRIMARY KEY, directory TEXT NOT NULL, \
                 title TEXT NOT NULL, time_created INTEGER NOT NULL);\
                 CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, \
                 time_created INTEGER NOT NULL, data TEXT NOT NULL);\
                 CREATE TABLE part (id TEXT PRIMARY KEY, message_id TEXT NOT NULL, \
                 session_id TEXT NOT NULL, time_created INTEGER NOT NULL);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO session(id, directory, title, time_created) VALUES(?1,?2,?3,?4)",
                params!["s-1", "/work/alpha", "title-001", 1_000],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO message(id, session_id, time_created, data) VALUES(?1,?2,?3,?4)",
                params![
                    "m-1",
                    "s-1",
                    1_100,
                    json!({"role": "assistant"}).to_string()
                ],
            )
            .unwrap();
        assert!(read_progress(&path, &SessionBinding::Title("title-001")).is_none());
    }

    #[test]
    fn missing_file_returns_none_without_creating_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("opencode.db");
        assert!(read_progress(&path, &SessionBinding::Title("title-001")).is_none());
        assert!(!path.exists());
    }

    #[test]
    fn broken_json_row_is_skipped() {
        let fixture = base();
        fixture.part_raw("p-bad", "s-1", "m-1", "{broken");
        fixture.part(
            "p-1",
            "s-1",
            "m-1",
            &part_data("reasoning", None, None, Some(1_200), None),
        );
        let progress = title_progress(&fixture).unwrap();
        assert_eq!(progress.phase, ProgressPhase::Thinking);
        assert_eq!(progress.last_activity_ms, Some(1_200));
    }

    #[test]
    fn duplicate_title_prefers_latest_session() {
        let fixture = Fixture::new();
        fixture.session("s-old", "/work/old", "same-title", 1_000);
        fixture.session("s-new", "/work/new", "same-title", 2_000);
        fixture.message("m-old", "s-old", 1_100, None);
        fixture.message("m-new", "s-new", 2_100, None);
        fixture.part(
            "p-old",
            "s-old",
            "m-old",
            &part_data("tool", Some("running"), Some("old-tool"), Some(1_200), None),
        );
        fixture.part(
            "p-new",
            "s-new",
            "m-new",
            &part_data("reasoning", None, None, Some(2_200), None),
        );
        let progress = fixture.read(&SessionBinding::Title("same-title")).unwrap();
        assert_eq!(progress.phase, ProgressPhase::Thinking);
        assert_eq!(progress.phase_started_ms, Some(2_200));
    }

    #[test]
    fn first_in_directory_is_ambiguous_within_two_seconds() {
        let fixture = Fixture::new();
        fixture.session("s-1", "/work/alpha", "t-1", 1_000);
        fixture.session("s-2", "/work/alpha", "t-2", 3_000);
        fixture.message("m-1", "s-1", 1_100, None);
        assert!(
            fixture
                .read(&SessionBinding::FirstInDirectory {
                    directory: "/work/alpha",
                    created_after_ms: 0,
                })
                .is_none()
        );
    }

    #[test]
    fn first_in_directory_skips_other_directories_and_earlier_sessions() {
        let fixture = Fixture::new();
        fixture.session("s-before", "/work/alpha", "t-before", 800);
        fixture.session("s-first", "\\work\\alpha\\", "t-first", 1_000);
        fixture.session("s-other", "/work/beta", "t-other", 1_200);
        fixture.session("s-late", "/work/alpha", "t-late", 5_000);
        fixture.message("m-before", "s-before", 810, None);
        fixture.message("m-first", "s-first", 1_100, None);
        fixture.message("m-other", "s-other", 1_210, None);
        fixture.message("m-late", "s-late", 5_100, None);
        fixture.part(
            "p-before",
            "s-before",
            "m-before",
            &part_data(
                "tool",
                Some("running"),
                Some("before-tool"),
                Some(820),
                None,
            ),
        );
        fixture.part(
            "p-first",
            "s-first",
            "m-first",
            &part_data("reasoning", None, None, Some(1_100), None),
        );
        fixture.part(
            "p-other",
            "s-other",
            "m-other",
            &part_data("text", None, None, Some(1_210), None),
        );
        fixture.part(
            "p-late",
            "s-late",
            "m-late",
            &part_data(
                "tool",
                Some("running"),
                Some("late-tool"),
                Some(5_100),
                None,
            ),
        );
        let progress = fixture
            .read(&SessionBinding::FirstInDirectory {
                directory: "/work/alpha/",
                created_after_ms: 900,
            })
            .unwrap();
        assert_eq!(progress.phase, ProgressPhase::Thinking);
        assert_eq!(progress.phase_started_ms, Some(1_100));
    }

    #[test]
    fn first_in_directory_without_candidates_returns_none() {
        let fixture = Fixture::new();
        fixture.session("s-1", "/work/alpha", "t-1", 1_000);
        assert!(
            fixture
                .read(&SessionBinding::FirstInDirectory {
                    directory: "/work/alpha",
                    created_after_ms: 2_000,
                })
                .is_none()
        );
        assert!(
            fixture
                .read(&SessionBinding::FirstInDirectory {
                    directory: "/work/gamma",
                    created_after_ms: 0,
                })
                .is_none()
        );
    }

    #[test]
    fn tool_wins_over_thinking() {
        let fixture = base();
        fixture.part(
            "p-1",
            "s-1",
            "m-1",
            &part_data("reasoning", None, None, Some(1_200), None),
        );
        fixture.part(
            "p-2",
            "s-1",
            "m-1",
            &part_data("tool", Some("running"), Some("bash"), Some(1_300), None),
        );
        let progress = title_progress(&fixture).unwrap();
        assert_eq!(progress.phase, ProgressPhase::Tool);
        assert_eq!(progress.phase_started_ms, Some(1_300));
        assert_eq!(progress.tool.as_deref(), Some("bash"));
    }

    #[test]
    fn tool_name_is_validated() {
        let long_name = "a".repeat(65);
        for (name, expected) in [
            ("bash", Some("bash")),
            ("build.rs:1-x_y", Some("build.rs:1-x_y")),
            (long_name.as_str(), None),
            ("bash sh", None),
            ("bash;rm", None),
            ("", None),
        ] {
            let fixture = base();
            fixture.part(
                "p-1",
                "s-1",
                "m-1",
                &part_data("tool", Some("running"), Some(name), Some(1_200), None),
            );
            let progress = title_progress(&fixture).unwrap();
            assert_eq!(progress.phase, ProgressPhase::Tool);
            assert_eq!(progress.tool.as_deref(), expected, "tool name {name:?}");
        }
    }

    #[test]
    fn sentinels_do_not_appear_in_debug_output() {
        let fixture = Fixture::new();
        fixture.session("s-1", "/work/alpha", "SENTINEL-TITLE-alpha", 1_000);
        fixture.message("m-1", "s-1", 1_100, None);
        let mut data = part_data("reasoning", None, None, Some(1_200), None);
        data["text"] = json!("SENTINEL-BODY-text");
        data["state"] = json!({
            "input": "SENTINEL-BODY-input",
            "output": "SENTINEL-BODY-output",
            "metadata": "SENTINEL-BODY-metadata",
        });
        fixture.part("p-1", "s-1", "m-1", &data);
        let progress = fixture
            .read(&SessionBinding::Title("SENTINEL-TITLE-alpha"))
            .unwrap();
        let debug = format!("{progress:?}");
        assert!(!debug.contains("SENTINEL"), "{debug}");
    }

    #[test]
    fn last_activity_is_session_wide() {
        let fixture = base();
        fixture.message("m-0", "s-1", 900, Some(1_000));
        fixture.part(
            "p-older",
            "s-1",
            "m-0",
            &part_data("text", None, None, Some(950), Some(5_000)),
        );
        fixture.part(
            "p-1",
            "s-1",
            "m-1",
            &part_data("reasoning", None, None, Some(1_200), None),
        );
        let progress = title_progress(&fixture).unwrap();
        assert_eq!(progress.phase, ProgressPhase::Thinking);
        assert_eq!(progress.phase_started_ms, Some(1_200));
        assert_eq!(progress.last_activity_ms, Some(5_000));
    }

    #[test]
    fn latest_open_assistant_message_is_used() {
        let fixture = base();
        fixture.message("m-2", "s-1", 1_500, None);
        fixture.part(
            "p-1",
            "s-1",
            "m-1",
            &part_data("reasoning", None, None, Some(1_200), None),
        );
        fixture.part(
            "p-2",
            "s-1",
            "m-2",
            &part_data("text", None, None, Some(1_600), None),
        );
        let progress = title_progress(&fixture).unwrap();
        assert_eq!(progress.phase, ProgressPhase::Responding);
        assert_eq!(progress.phase_started_ms, Some(1_600));
    }

    #[test]
    fn running_tool_start_is_read_from_state_time() {
        let fixture = base();
        fixture.part(
            "p-1",
            "s-1",
            "m-1",
            &tool_data("completed", "read", Some(1_200), Some(1_250)),
        );
        fixture.part(
            "p-2",
            "s-1",
            "m-1",
            &tool_data("running", "bash", Some(1_300), None),
        );
        let progress = title_progress(&fixture).unwrap();
        assert_eq!(progress.phase, ProgressPhase::Tool);
        assert_eq!(progress.tool.as_deref(), Some("bash"));
        assert_eq!(progress.phase_started_ms, Some(1_300));
        assert_eq!(progress.last_activity_ms, Some(1_300));
    }

    #[test]
    fn pending_tool_without_times_falls_back_to_row_times() {
        let fixture = base();
        fixture.part_at(
            "p-1",
            "s-1",
            "m-1",
            (1_350, 1_360),
            &tool_data("pending", "bash", None, None),
        );
        let progress = title_progress(&fixture).unwrap();
        assert_eq!(progress.phase, ProgressPhase::Tool);
        assert_eq!(progress.tool.as_deref(), Some("bash"));
        assert_eq!(progress.phase_started_ms, Some(1_350));
        assert_eq!(progress.last_activity_ms, Some(1_360));
    }

    #[test]
    fn completed_tools_in_state_time_are_between_steps() {
        let fixture = base();
        fixture.part(
            "p-1",
            "s-1",
            "m-1",
            &tool_data("completed", "read", Some(1_200), Some(1_250)),
        );
        fixture.part(
            "p-2",
            "s-1",
            "m-1",
            &tool_data("error", "bash", Some(1_260), Some(1_400)),
        );
        let progress = title_progress(&fixture).unwrap();
        assert_eq!(progress.phase, ProgressPhase::BetweenSteps);
        assert_eq!(progress.tool, None);
        assert_eq!(progress.phase_started_ms, Some(1_400));
        assert_eq!(progress.last_activity_ms, Some(1_400));
    }

    #[test]
    fn older_unfinished_message_does_not_beat_a_newer_completed_one() {
        let fixture = Fixture::new();
        fixture.session("s-1", "/work/alpha", "title-001", 1_000);
        fixture.message("m-old", "s-1", 1_100, None);
        fixture.message("m-new", "s-1", 1_500, Some(1_900));
        fixture.part(
            "p-old",
            "s-1",
            "m-old",
            &part_data("reasoning", None, None, Some(1_200), None),
        );
        fixture.part(
            "p-new",
            "s-1",
            "m-new",
            &part_data("text", None, None, Some(1_600), Some(1_800)),
        );
        let progress = title_progress(&fixture).unwrap();
        assert_eq!(progress.phase, ProgressPhase::Idle);
    }

    #[cfg(windows)]
    #[test]
    fn first_in_directory_ignores_case_on_windows() {
        let fixture = Fixture::new();
        fixture.session("s-1", "F:\\Work\\Alpha", "t-1", 1_000);
        fixture.message("m-1", "s-1", 1_100, None);
        fixture.part(
            "p-1",
            "s-1",
            "m-1",
            &part_data("reasoning", None, None, Some(1_200), None),
        );
        let progress = fixture
            .read(&SessionBinding::FirstInDirectory {
                directory: "f:/work/alpha",
                created_after_ms: 0,
            })
            .unwrap();
        assert_eq!(progress.phase, ProgressPhase::Thinking);
    }

    #[test]
    fn sql_only_projects_json_fields() {
        for sql in SQL_STATEMENTS {
            assert!(sql.trim_start().starts_with("SELECT"), "{sql}");
            assert!(!sql.contains("SELECT *"), "{sql}");
            for (index, _) in sql.match_indices("data") {
                let prefix = &sql[..index];
                assert!(
                    prefix.ends_with("json_extract(") || prefix.ends_with("json_valid("),
                    "raw data column escaped json_extract/json_valid in: {sql}"
                );
            }
            for forbidden in [
                "$.text",
                "$.state.input",
                "$.state.output",
                "$.state.metadata",
                "state.input",
                "state.output",
                "state.metadata",
            ] {
                assert!(!sql.contains(forbidden), "{forbidden} in {sql}");
            }
        }
    }

    #[test]
    fn database_file_is_not_modified() {
        let fixture = base();
        fixture.part(
            "p-1",
            "s-1",
            "m-1",
            &part_data("reasoning", None, None, Some(1_200), None),
        );
        let before = std::fs::read(&fixture.path).unwrap();
        let entries_before = std::fs::read_dir(fixture.dir.path()).unwrap().count();
        let progress = title_progress(&fixture).unwrap();
        assert_eq!(progress.phase, ProgressPhase::Thinking);
        let after = std::fs::read(&fixture.path).unwrap();
        let entries_after = std::fs::read_dir(fixture.dir.path()).unwrap().count();
        assert_eq!(before, after);
        assert_eq!(entries_before, entries_after);
    }
}
