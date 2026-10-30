//! Local hold record for this Hub only. Deskly's stage-4 ledger and one-time
//! authority stay canonical: this table never grants, retries or re-sends. It
//! exists so a crash, disconnect or lost reply leaves `(request, generation)`
//! visibly held instead of looking new to the next poll. It stores no
//! instruction text, wire body, preview token or key.
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    OwnerPending,
    OwnerDeclined,
    Approved,
    ClaimSent,
    Claimed,
    HandoffSent,
    Issued,
    LaunchSent,
    Launched,
    Refused,
    Held,
}
impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OwnerPending => "owner_pending",
            Self::OwnerDeclined => "owner_declined",
            Self::Approved => "approved",
            Self::ClaimSent => "claim_sent",
            Self::Claimed => "claimed",
            Self::HandoffSent => "handoff_sent",
            Self::Issued => "issued",
            Self::LaunchSent => "launch_sent",
            Self::Launched => "launched",
            Self::Refused => "refused",
            Self::Held => "held",
        }
    }
    fn parse(raw: &str) -> Option<Self> {
        [
            Self::OwnerPending,
            Self::OwnerDeclined,
            Self::Approved,
            Self::ClaimSent,
            Self::Claimed,
            Self::HandoffSent,
            Self::Issued,
            Self::LaunchSent,
            Self::Launched,
            Self::Refused,
            Self::Held,
        ]
        .into_iter()
        .find(|state| state.as_str() == raw)
    }
}
/// States in which a request may have left this process with an unknown
/// result. A restart turns each into `Held`; only a person reconciling with
/// Deskly (and local session evidence) may resolve it.
const IN_FLIGHT: [State; 5] = [
    State::Approved,
    State::ClaimSent,
    State::Claimed,
    State::HandoffSent,
    State::Issued,
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub request_id: String,
    pub generation: u64,
    pub attempt_id: String,
    pub state: State,
    pub reason: String,
    pub revision: u64,
    pub authorization_epoch: u64,
    pub session_id: String,
    pub repo: String,
    pub pull_request: u64,
    pub head_sha: String,
    pub client_request_id: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalError(pub String);
impl From<rusqlite::Error> for JournalError {
    fn from(error: rusqlite::Error) -> Self {
        Self(error.to_string())
    }
}

pub struct Journal {
    db: std::sync::Mutex<Connection>,
}
impl Journal {
    /// Opening is recovery: every in-flight row from a previous process,
    /// including a launch that may have started, becomes `Held`.
    pub fn open(path: &Path) -> Result<(Self, usize), JournalError> {
        let db = Connection::open(path)?;
        db.pragma_update(None, "journal_mode", "WAL")?;
        db.pragma_update(None, "synchronous", "FULL")?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS deskly_intake_attempt(
                request_id TEXT NOT NULL,
                generation INTEGER NOT NULL,
                attempt_id TEXT NOT NULL UNIQUE,
                state TEXT NOT NULL,
                reason TEXT NOT NULL DEFAULT '',
                revision INTEGER NOT NULL,
                authorization_epoch INTEGER NOT NULL,
                session_id TEXT NOT NULL,
                repo TEXT NOT NULL,
                pull_request INTEGER NOT NULL,
                head_sha TEXT NOT NULL,
                client_request_id TEXT NOT NULL DEFAULT '',
                updated_ms INTEGER NOT NULL,
                PRIMARY KEY(request_id, generation)
            ) STRICT;",
        )?;
        let mut held = 0;
        for state in IN_FLIGHT.into_iter().chain([State::LaunchSent]) {
            held += db.execute(
                "UPDATE deskly_intake_attempt SET state = 'held', reason = ?1
                 WHERE state = ?2",
                params![format!("restart_during_{}", state.as_str()), state.as_str()],
            )?;
        }
        Ok((
            Self {
                db: std::sync::Mutex::new(db),
            },
            held,
        ))
    }
    fn db(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.db.lock().unwrap_or_else(|poison| poison.into_inner())
    }
    /// Insert a new `OwnerPending` row. Returns false (and changes nothing)
    /// when this Hub has already seen `(request_id, generation)` in any state.
    pub fn reserve(&self, record: &Record, now_ms: i64) -> Result<bool, JournalError> {
        let inserted = self.db().execute(
            "INSERT OR IGNORE INTO deskly_intake_attempt(request_id, generation, attempt_id,
                state, revision, authorization_epoch, session_id, repo, pull_request, head_sha,
                updated_ms)
             VALUES (?1, ?2, ?3, 'owner_pending', ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                record.request_id,
                record.generation as i64,
                record.attempt_id,
                record.revision as i64,
                record.authorization_epoch as i64,
                record.session_id,
                record.repo,
                record.pull_request as i64,
                record.head_sha,
                now_ms
            ],
        )?;
        Ok(inserted == 1)
    }
    /// Compare-and-set; the row is durable (synchronous=FULL) before return,
    /// so callers write the intent before sending the matching request.
    pub fn transition(
        &self,
        request_id: &str,
        generation: u64,
        from: State,
        to: State,
        reason: &str,
        now_ms: i64,
    ) -> Result<bool, JournalError> {
        let changed = self.db().execute(
            "UPDATE deskly_intake_attempt SET state = ?1, reason = ?2, updated_ms = ?3
             WHERE request_id = ?4 AND generation = ?5 AND state = ?6",
            params![
                to.as_str(),
                reason,
                now_ms,
                request_id,
                generation as i64,
                from.as_str()
            ],
        )?;
        Ok(changed == 1)
    }
    pub fn launched(
        &self,
        request_id: &str,
        generation: u64,
        client_request_id: &str,
        now_ms: i64,
    ) -> Result<bool, JournalError> {
        let changed = self.db().execute(
            "UPDATE deskly_intake_attempt SET state = 'launched', client_request_id = ?1,
                updated_ms = ?2
             WHERE request_id = ?3 AND generation = ?4 AND state = 'launch_sent'",
            params![client_request_id, now_ms, request_id, generation as i64],
        )?;
        Ok(changed == 1)
    }
    pub fn get(&self, request_id: &str, generation: u64) -> Result<Option<Record>, JournalError> {
        let db = self.db();
        let row = db
            .query_row(
                "SELECT request_id, generation, attempt_id, state, reason, revision,
                    authorization_epoch, session_id, repo, pull_request, head_sha,
                    client_request_id
                 FROM deskly_intake_attempt WHERE request_id = ?1 AND generation = ?2",
                params![request_id, generation as i64],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, i64>(9)?,
                        row.get::<_, String>(10)?,
                        row.get::<_, String>(11)?,
                    ))
                },
            )
            .optional()?;
        let Some(r) = row else { return Ok(None) };
        let state = State::parse(&r.3).ok_or_else(|| JournalError("unknown state".into()))?;
        Ok(Some(Record {
            request_id: r.0,
            generation: r.1 as u64,
            attempt_id: r.2,
            state,
            reason: r.4,
            revision: r.5 as u64,
            authorization_epoch: r.6 as u64,
            session_id: r.7,
            repo: r.8,
            pull_request: r.9 as u64,
            head_sha: r.10,
            client_request_id: r.11,
        }))
    }
}

/// Deskly-compatible random UUIDv4 for `client_attempt_id`.
pub fn new_attempt_id() -> Result<String, JournalError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| JournalError("random".into()))?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    ))
}
