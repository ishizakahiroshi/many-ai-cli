//! Ordered session JSONL + SQLite adapter. The handle table owns writers only;
//! session lifecycle and identities live exclusively in SessionEngine.
//! Source: internal/sessionlog/sessionlog.go and hub/server.go:writeHistory.
use crate::proto::time::Timestamp;
use crate::{
    config::{Resource, RuntimePaths},
    proto::{core::*, time},
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, Ordering},
    },
};

#[cfg(test)]
#[path = "journal/failure_tests.rs"]
mod failure_tests;

#[derive(Clone, Copy, Debug, Default)]
pub struct JournalOptions {
    pub session_enabled: bool,
    /// Nonpositive means unlimited, as in Go's sessionlog.Writer.
    pub max_bytes: i64,
}
pub type JournalWarningHandler = Arc<dyn Fn(LiveSessionId, &SessionError) + Send + Sync>;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalPaths {
    pub raw: PathBuf,
    pub jsonl: PathBuf,
}

pub struct SessionJournal {
    paths: RuntimePaths,
    storage: Option<Arc<dyn SessionStorage>>,
    enabled: AtomicBool,
    warning: RwLock<JournalWarningHandler>,
    ordering: Mutex<()>,
    writers: Mutex<BTreeMap<LiveSessionId, BoundWriter>>,
}
/// Owns the binding capability for one live session. No `.jsonl` file is
/// written any more; the raw `.log` and the SQLite metadata are the only records.
struct BoundWriter {
    binding: SessionBinding,
}
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}
fn failure(detail: &str) -> SessionError {
    SessionError::Storage(StorageError {
        kind: StorageErrorKind::Write,
        detail: detail.into(),
    })
}

/// Compute the source-compatible session log names without opening a file or
/// creating another journal owner. Validated reattach timestamps retain their
/// original wall-clock offset/date rather than being reformatted in the Hub zone.
pub fn session_log_paths(
    paths: &RuntimePaths,
    id: LiveSessionId,
    provider: &str,
    cwd: &str,
    started: &str,
) -> Result<JournalPaths, SessionError> {
    time::parse_rfc3339(started).map_err(|_| failure("session log timestamp is unsupported"))?;
    let clock = started[11..].split(':').collect::<Vec<_>>();
    let hour = clock[0]
        .parse::<u8>()
        .map_err(|_| failure("invalid session log hour"))?;
    let stamp = format!("{}_{hour:02}{}{}", &started[..10], clock[1], &clock[2][..2]);
    let provider = sanitize_file_part(&provider.trim().to_lowercase());
    // filepath.Base is host-specific. Path handles the supported platform's
    // native separator; no wrapper-proposed log path is ever consulted.
    let folder = go_basename(cwd);
    let base = format!(
        "{provider}_{stamp}_{}_s{}",
        sanitize_file_part(&folder),
        id.0
    );
    let raw = paths
        .resource(Resource::Logs)
        .join("sessions")
        .join(format!("{base}.log"));
    let jsonl = raw.with_extension("jsonl");
    Ok(JournalPaths { raw, jsonl })
}

impl SessionJournal {
    pub fn new(
        paths: RuntimePaths,
        storage: Option<Arc<dyn SessionStorage>>,
        options: JournalOptions,
    ) -> Self {
        Self {
            paths,
            storage,
            enabled: AtomicBool::new(options.session_enabled),
            warning: RwLock::new(Arc::new(|id, error| {
                eprintln!("session journal {}: {error:?}", id.0)
            })),
            ordering: Mutex::new(()),
            writers: Mutex::new(BTreeMap::new()),
        }
    }
    pub fn set_warning_handler(&self, handler: JournalWarningHandler) {
        *self.warning.write().unwrap_or_else(|p| p.into_inner()) = handler;
    }
    fn warn(&self, session: LiveSessionId, error: &SessionError) {
        let handler = self
            .warning
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        handler(session, error);
    }
    pub fn storage(&self) -> Option<&Arc<dyn SessionStorage>> {
        self.storage.as_ref()
    }
    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }
    /// Changing the body gate does not create missing writers or alter metadata.
    /// Existing writers are retained until their lifecycle ends, matching Go.
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Release);
    }
    pub fn paths(
        &self,
        id: LiveSessionId,
        provider: &str,
        cwd: &str,
        started: Timestamp,
    ) -> Result<JournalPaths, SessionError> {
        let started = time::format_rfc3339(started)
            .map_err(|_| failure("session log timestamp is unsupported"))?;
        self.paths_for_timestamp(id, provider, cwd, &started)
    }
    /// The frozen wrapper reattach timestamp carries a wall-clock offset. Keep
    /// that date/time for log naming rather than reformatting in the Hub's zone.
    pub fn paths_for_timestamp(
        &self,
        id: LiveSessionId,
        provider: &str,
        cwd: &str,
        started: &str,
    ) -> Result<JournalPaths, SessionError> {
        session_log_paths(&self.paths, id, provider, cwd, started)
    }
    /// Called during registration, before its first history effect is produced.
    /// Failure does not erase the previous writer or database session metadata.
    pub fn open(
        &self,
        id: LiveSessionId,
        provider: &str,
        cwd: &str,
        started: Timestamp,
        append: bool,
    ) -> Result<JournalPaths, SessionError> {
        let paths = self.paths(id, provider, cwd, started)?;
        self.open_paths(id, paths, append)
    }
    pub fn open_for_timestamp(
        &self,
        id: LiveSessionId,
        provider: &str,
        cwd: &str,
        started: &str,
        append: bool,
    ) -> Result<JournalPaths, SessionError> {
        let paths = self.paths_for_timestamp(id, provider, cwd, started)?;
        self.open_paths(id, paths, append)
    }
    fn open_paths(
        &self,
        id: LiveSessionId,
        paths: JournalPaths,
        append: bool,
    ) -> Result<JournalPaths, SessionError> {
        let _order = lock(&self.ordering);
        let binding = SessionBinding {
            session: id,
            incarnation: SessionIncarnation(0),
            wrapper: WrapperConnectionId(0),
        };
        self.bind_writer(binding, &paths, append)?;
        Ok(paths)
    }
    fn bind_writer(
        &self,
        binding: SessionBinding,
        _paths: &JournalPaths,
        _append: bool,
    ) -> Result<(), SessionError> {
        // Replacing a capability closes its old writer now. A later stale
        // cleanup must not close this newly-bound writer or end its DB row.
        lock(&self.writers).insert(binding.session, BoundWriter { binding });
        Ok(())
    }
    /// Registration is one persistence transaction boundary: writer binding,
    /// optional DB start, and restored card metadata cannot interleave with an
    /// old binding's EndSession. The caller holds no session-state lock.
    pub fn attach_session(
        &self,
        binding: SessionBinding,
        start: SessionStart,
        mut card: SessionCardMeta,
        started: &str,
        append: bool,
        persist_metadata: bool,
    ) -> Result<(JournalPaths, Option<DbSessionId>, SessionCardMeta), SessionError> {
        let paths =
            self.paths_for_timestamp(binding.session, &start.provider, &start.cwd, started)?;
        let _order = lock(&self.ordering);
        if let Err(error) = self.bind_writer(binding, &paths, append) {
            self.warn(binding.session, &error);
        }
        let mut db_id = None;
        if persist_metadata && let Some(storage) = &self.storage {
            match storage.start_session(start) {
                Ok(id) if id.0 != 0 => {
                    db_id = Some(id);
                    match storage.session_card_meta_by_live_session(binding.session) {
                        Ok(saved) => card = saved,
                        Err(error) => self.warn(binding.session, &SessionError::Storage(error)),
                    }
                }
                Ok(_) => {}
                Err(error) => self.warn(binding.session, &SessionError::Storage(error)),
            }
        }
        Ok((paths, db_id, card))
    }
    pub fn close_writer(&self, id: LiveSessionId) {
        let _order = lock(&self.ordering);
        lock(&self.writers).remove(&id);
    }
    pub fn close_all(&self) {
        let _order = lock(&self.ordering);
        lock(&self.writers).clear();
    }
    fn event(&self, id: LiveSessionId, event: HistoryEvent) -> Result<(), SessionError> {
        if !self.enabled() {
            return Ok(());
        }
        if let Some(storage) = &self.storage {
            match storage.store_event_async(id, event) {
                EnqueueOutcome::Dropped { cumulative_count } if cumulative_count % 1000 == 1 => {
                    self.warn(
                        id,
                        &failure("session history queue is full; event was dropped"),
                    )
                }
                EnqueueOutcome::Closed => {
                    self.warn(id, &failure("session history database is closed"))
                }
                _ => {}
            }
        }
        // Go history failures are best effort and do not prevent live output.
        Ok(())
    }
}
impl PersistenceEffectSink for SessionJournal {
    fn apply(&self, _effect: PersistenceEffect) -> Result<(), SessionError> {
        Err(SessionError::InvalidRequest(
            "session persistence requires a binding".into(),
        ))
    }
    fn apply_bound(
        &self,
        binding: SessionBinding,
        scope: PersistenceBindingScope,
        effect: PersistenceEffect,
    ) -> Result<(), SessionError> {
        let _order = lock(&self.ordering);
        if persistence_session(&effect) != binding.session {
            return Err(SessionError::InvalidRequest(
                "persistence binding does not match effect target".into(),
            ));
        }
        let current = lock(&self.writers)
            .get(&binding.session)
            .map(|slot| slot.binding);
        let valid = current.is_some_and(|current| match scope {
            PersistenceBindingScope::Incarnation => current.incarnation == binding.incarnation,
            PersistenceBindingScope::ExactWrapper => current == binding,
        });
        if !valid {
            return Err(SessionError::StaleBinding);
        }
        self.apply_ordered(effect)
    }
}

impl SessionJournal {
    fn apply_ordered(&self, effect: PersistenceEffect) -> Result<(), SessionError> {
        if let PersistenceEffect::Event { session, event } = effect {
            return self.event(session, event);
        }
        let storage = self.storage.as_ref();
        match effect {
            PersistenceEffect::Event { .. } => unreachable!(),
            PersistenceEffect::OrchestrationMeta { database, meta, .. } => {
                if let Some(storage) = storage {
                    storage
                        .update_session_orchestration(database, &meta)
                        .map_err(SessionError::Storage)?;
                }
            }
            PersistenceEffect::ApprovalDetected(d) => {
                if let Some(s) = storage {
                    s.store_approval_detected(d);
                }
            }
            PersistenceEffect::ApprovalConsumed {
                session,
                sig,
                selected_text,
                resolved_at,
            } => {
                if let Some(s) = storage {
                    s.store_approval_consumed(session, &sig, &selected_text, resolved_at);
                }
            }
            PersistenceEffect::CardMeta { session, meta } => {
                if let Some(s) = storage {
                    s.update_session_card_meta(session, meta)
                        .map_err(SessionError::Storage)?;
                }
            }
            PersistenceEffect::CardMetaBestEffort { session, meta } => {
                if let Some(s) = storage
                    && let Err(error) = s.update_session_card_meta(session, meta)
                {
                    self.warn(session, &SessionError::Storage(error));
                }
            }
            PersistenceEffect::SessionState {
                session,
                state,
                last_output_at,
            } => {
                if let Some(s) = storage {
                    s.update_session_state(session, &state, &last_output_at);
                }
            }
            PersistenceEffect::SessionMessages {
                session,
                first,
                last,
            } => {
                if let Some(s) = storage {
                    s.update_session_messages(session, &first, &last);
                }
            }
            PersistenceEffect::EndSession {
                binding,
                state,
                reason,
                ended_at,
            } => {
                let session = binding.session;
                if !lock(&self.writers)
                    .get(&session)
                    .is_some_and(|slot| slot.binding == binding)
                {
                    return Ok(());
                }
                // Source disconnect closes JSONL directly (no duplicate SQLite
                // event); dismiss records only lifecycle metadata even when the
                // normal user-body logging gate is disabled.
                let kind = if state == "disconnected" {
                    Some("session_end")
                } else if state == "dismissed" {
                    Some("session_dismiss")
                } else {
                    None
                };
                if let Some(kind) = kind {
                    let stamp = time::format_rfc3339(ended_at)
                        .map_err(|_| failure("session end timestamp is unsupported"))?;
                    let mut data =
                        serde_json::json!({"type":kind,"session_id":session.0,"ts":stamp})
                            .as_object()
                            .expect("history object")
                            .clone();
                    if kind == "session_end" {
                        data.insert("state".into(), state.clone().into());
                        data.insert("exit_code".into(), 0.into());
                        if !reason.is_empty() {
                            data.insert("reason".into(), reason.clone().into());
                        }
                    }
                    let event = HistoryEvent(data);
                    if kind == "session_dismiss"
                        && let Some(s) = storage
                    {
                        s.store_event_async(session, event);
                    }
                }
                if let Some(s) = storage {
                    s.end_session(session, &state, &reason, ended_at);
                }
            }
            PersistenceEffect::ClearSessionHistory(id) => {
                if let Some(s) = storage
                    && let Err(error) = s.clear_session_history(id)
                {
                    self.warn(id, &SessionError::Storage(error));
                }
            }
        }
        Ok(())
    }
}
/// Baseline filename normalization is byte-bounded, not character-bounded.
pub fn sanitize_file_part(text: &str) -> String {
    let mut out = String::new();
    let mut whitespace = false;
    for c in text.trim().chars() {
        if c.is_control() || "<>:\"/\\|?*".contains(c) {
            out.push('_');
            whitespace = false;
        } else if matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c') {
            if !whitespace {
                out.push('_');
            }
            whitespace = true;
        } else {
            out.push(c);
            whitespace = false;
        }
    }
    let mut out = out.trim_matches(['.', ' ']).to_owned();
    if out.len() > 80 {
        let mut end = 80;
        while !out.is_char_boundary(end) {
            end -= 1;
        }
        out.truncate(end);
        out = out.trim_matches(['.', ' ']).to_owned();
    }
    if out.is_empty() {
        "no-project".into()
    } else {
        out
    }
}

fn go_basename(path: &str) -> String {
    if path.is_empty() {
        return ".".into();
    }
    #[cfg(not(windows))]
    {
        let trimmed = path.trim_end_matches('/');
        if trimmed.is_empty() {
            "/".into()
        } else {
            trimmed.rsplit('/').next().unwrap_or_default().into()
        }
    }
    #[cfg(windows)]
    {
        let trimmed = path.trim_end_matches(['/', '\\']);
        if trimmed.is_empty() {
            "\\".into()
        } else {
            trimmed
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or_default()
                .into()
        }
    }
}

pub(crate) fn persistence_session(effect: &PersistenceEffect) -> LiveSessionId {
    match effect {
        PersistenceEffect::OrchestrationMeta { session, .. }
        | PersistenceEffect::Event { session, .. }
        | PersistenceEffect::ApprovalConsumed { session, .. }
        | PersistenceEffect::CardMeta { session, .. }
        | PersistenceEffect::CardMetaBestEffort { session, .. }
        | PersistenceEffect::SessionState { session, .. }
        | PersistenceEffect::SessionMessages { session, .. } => *session,
        PersistenceEffect::ApprovalDetected(d) => d.live_session_id,
        PersistenceEffect::EndSession { binding, .. } => binding.session,
        PersistenceEffect::ClearSessionHistory(session) => *session,
    }
}
