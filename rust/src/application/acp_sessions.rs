//! ACP sessions: a kind of Hub session that is not a PTY session.
//!
//! A PTY session is a wrapper process that registers with `SessionCore` and
//! streams terminal bytes. An ACP session is a connection from the Hub to an
//! agent that speaks the Agent Client Protocol (`crate::acp`), so it has no
//! wrapper, no VT screen and no input queue. It therefore lives here, outside
//! `SessionCore` and `SessionEngine::start_wrapped` (which refuses ACP
//! providers, plan C10), and it is reached through its own `/api/acp/` routes.
//!
//! Boundaries kept here (plan C10/C11):
//!
//! - Every operation takes a `VerifiedUiOrigin`, which only the server can
//!   build after the signed dashboard cookie, Fetch Metadata and Origin checks.
//!   A child session, relay, routine, board item or token-only caller has none.
//! - Sessions are opened one at a time by that person. Nothing here opens a
//!   session, sends a prompt, answers a permission request or reconnects by
//!   itself. There is no limit on the number of sessions; the Hub just never
//!   adds one on its own.
//! - A permission request is stored as a pending record. Only
//!   `answer_permission`, called from the dashboard, answers it. The one other
//!   way it closes is `cancel`, which the protocol requires and which is not an
//!   approval.
//! - When the process ends or the stream breaks, the session ends: pending
//!   requests finish, the running turn is recorded as ended, and nothing is
//!   restarted or resent.
//! - Sign-in stays in the agent's own runtime. When the agent asks for it the
//!   session is not created and the caller gets a pointer to the official
//!   procedure. No `authenticate` call, callback relay or credential is touched.

use std::collections::{BTreeMap, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::{Notify, mpsc};

use crate::acp::{
    AcpClient, AcpError, AcpEvent, AcpEvents, AgentCapabilities, AuthMethod, PermissionDecision,
    PermissionOption, PermissionRequest, text_block,
};
use crate::files::safe_fs::Dir;
use crate::profile::declared;
use crate::proto::core::VerifiedUiOrigin;

/// Turns the provider id and folder into a connected client. Production uses
/// `native_connector`; tests hand in the scripted agent.
pub type Connector =
    Arc<dyn Fn(&str, &Path) -> Result<(AcpClient, AcpEvents), String> + Send + Sync>;

/// How many events one session keeps in memory (the log file keeps them all).
const MEMORY_EVENTS: usize = 2000;
/// The longest a single wait for the agent's answer to a handshake step may take.
const STEP_TIMEOUT: Duration = Duration::from_secs(60);
/// The longest an events request may wait for something new.
pub const MAX_WAIT: Duration = Duration::from_secs(25);

#[derive(Debug)]
pub enum AcpSessionError {
    /// The provider is not declared `connection: acp`.
    NotAcp(String),
    BadRequest(String),
    NotFound,
    /// A turn is already running in this session.
    Busy,
    Ended(String),
    /// The agent needs the person to sign in through its own runtime.
    AuthRequired {
        message: String,
        methods: Vec<AuthMethod>,
    },
    Unsupported(String),
    /// The agent could not be started.
    Launch(String),
    Agent(String),
}

impl std::fmt::Display for AcpSessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAcp(id) => write!(f, "{id} is not an ACP provider"),
            Self::BadRequest(reason) => write!(f, "{reason}"),
            Self::NotFound => write!(f, "no such ACP session"),
            Self::Busy => write!(f, "a turn is already running in this session"),
            Self::Ended(reason) => write!(f, "the session has ended: {reason}"),
            Self::AuthRequired { .. } => write!(f, "{AUTH_GUIDANCE}"),
            Self::Unsupported(what) => write!(f, "the agent does not support {what}"),
            Self::Launch(reason) => write!(f, "the agent could not be started: {reason}"),
            Self::Agent(reason) => write!(f, "{reason}"),
        }
    }
}

/// Shown to the person; many-ai-cli never signs in on their behalf.
pub const AUTH_GUIDANCE: &str = "Sign-in is required. Sign in with this AI's own official procedure (its login command or its own app), then start the session again. many-ai-cli does not sign in for you and does not relay any sign-in.";

fn from_acp(error: AcpError) -> AcpSessionError {
    match error {
        AcpError::AuthRequired { message } => AcpSessionError::AuthRequired {
            message,
            methods: Vec::new(),
        },
        AcpError::Unsupported(what) => AcpSessionError::Unsupported(what.to_owned()),
        AcpError::Closed(reason) => AcpSessionError::Ended(reason),
        other => AcpSessionError::Agent(other.to_string()),
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct StoredEvent {
    pub seq: u64,
    pub at: String,
    pub kind: &'static str,
    pub data: Value,
}

#[derive(Clone, Debug, Serialize)]
pub struct PendingPermission {
    pub request_key: u64,
    pub tool_call: Value,
    pub options: Vec<Value>,
    pub at: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Idle,
    Running,
    Ended,
}

enum TurnResult {
    Done(String),
    Failed(String),
}

struct State {
    seq: u64,
    events: VecDeque<StoredEvent>,
    pending: BTreeMap<u64, PendingPermission>,
    phase: Phase,
    ended_reason: Option<String>,
    log: Option<std::fs::File>,
    /// While a saved conversation is being loaded its history arrives as
    /// updates; they are marked so a screen does not show them as new.
    replaying: bool,
}

pub struct Session {
    id: u64,
    provider: String,
    cwd: PathBuf,
    agent_session_id: String,
    started_at: String,
    log_name: Option<String>,
    capabilities: AgentCapabilities,
    client: Mutex<Option<AcpClient>>,
    turns: mpsc::UnboundedSender<TurnResult>,
    state: Mutex<State>,
    changed: Notify,
}

#[derive(Clone, Debug, Serialize)]
pub struct SessionView {
    pub id: u64,
    pub provider: String,
    pub cwd: String,
    pub agent_session_id: String,
    pub started_at: String,
    pub phase: Phase,
    pub ended_reason: Option<String>,
    pub log_file: Option<String>,
    pub can_list_agent_sessions: bool,
    pub can_resume: bool,
    pub can_close: bool,
    pub accepts_images: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct EventsView {
    pub session: SessionView,
    /// Sequence number of the oldest event still in memory.
    pub first_seq: u64,
    pub last_seq: u64,
    pub events: Vec<StoredEvent>,
    /// Hub's own record of the unanswered permission requests.
    pub pending_permissions: Vec<PendingPermission>,
}

/// A conversation the Hub saved, read back from its log's first line.
#[derive(Clone, Debug, Serialize)]
pub struct SavedSession {
    pub file: String,
    pub provider: String,
    pub cwd: String,
    pub agent_session_id: String,
    pub started_at: String,
}

fn now_text() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn mask(value: &mut Value) {
    match value {
        Value::String(text) => *text = crate::storage::mask_secrets(text),
        Value::Array(items) => items.iter_mut().for_each(mask),
        Value::Object(map) => map.values_mut().for_each(mask),
        _ => {}
    }
}

fn permission_options(options: &[PermissionOption]) -> Vec<Value> {
    options
        .iter()
        .map(|o| json!({"option_id": o.option_id, "name": o.name, "kind": o.kind}))
        .collect()
}

impl Session {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn view(&self) -> SessionView {
        let state = self.lock();
        self.view_locked(&state)
    }

    fn view_locked(&self, state: &State) -> SessionView {
        SessionView {
            id: self.id,
            provider: self.provider.clone(),
            cwd: self.cwd.to_string_lossy().into_owned(),
            agent_session_id: self.agent_session_id.clone(),
            started_at: self.started_at.clone(),
            phase: state.phase,
            ended_reason: state.ended_reason.clone(),
            log_file: self.log_name.clone(),
            can_list_agent_sessions: self.capabilities.list_sessions,
            can_resume: self.capabilities.resume_session || self.capabilities.load_session,
            can_close: self.capabilities.close_session,
            accepts_images: self.capabilities.image,
        }
    }

    fn record(&self, kind: &'static str, data: Value) {
        let mut state = self.lock();
        self.record_locked(&mut state, kind, data);
        drop(state);
        self.changed.notify_waiters();
    }

    fn record_locked(&self, state: &mut State, kind: &'static str, mut data: Value) {
        if state.replaying && kind == "update" {
            data["replay"] = json!(true);
        }
        state.seq += 1;
        let event = StoredEvent {
            seq: state.seq,
            at: now_text(),
            kind,
            data,
        };
        if let Some(file) = state.log.as_mut() {
            let mut line = serde_json::to_value(&event).unwrap_or(Value::Null);
            mask(&mut line);
            let mut bytes = serde_json::to_vec(&line).unwrap_or_default();
            bytes.push(b'\n');
            if file.write_all(&bytes).is_err() {
                // Keep running without the file rather than failing the turn.
                state.log = None;
            }
        }
        state.events.push_back(event);
        while state.events.len() > MEMORY_EVENTS {
            state.events.pop_front();
        }
    }

    fn client(&self) -> Result<AcpClient, AcpSessionError> {
        self.client
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or_else(|| {
                AcpSessionError::Ended(
                    self.lock()
                        .ended_reason
                        .clone()
                        .unwrap_or_else(|| "ended".into()),
                )
            })
    }

    /// Ends the session once: no client, no pending requests, no running turn.
    fn finish(&self, reason: &str) {
        let mut state = self.lock();
        if state.phase == Phase::Ended {
            return;
        }
        state.phase = Phase::Ended;
        state.ended_reason = Some(reason.to_owned());
        state.replaying = false;
        let dropped: Vec<u64> = state.pending.keys().copied().collect();
        state.pending.clear();
        for request_key in dropped {
            self.record_locked(
                &mut state,
                "permission_closed",
                json!({"request_key": request_key, "reason": "session ended"}),
            );
        }
        self.record_locked(&mut state, "ended", json!({"reason": reason}));
        state.log = None;
        drop(state);
        self.client.lock().unwrap_or_else(|e| e.into_inner()).take();
        self.changed.notify_waiters();
    }

    fn handle_event(&self, event: AcpEvent) {
        match event {
            AcpEvent::Update { session_id, update } => self.record(
                "update",
                json!({"session_id": session_id, "update": update}),
            ),
            AcpEvent::PermissionRequest(PermissionRequest {
                request_key,
                session_id,
                tool_call,
                options,
            }) => {
                let pending = PendingPermission {
                    request_key,
                    tool_call: tool_call.clone(),
                    options: permission_options(&options),
                    at: now_text(),
                };
                let mut state = self.lock();
                state.pending.insert(request_key, pending.clone());
                self.record_locked(
                    &mut state,
                    "permission_request",
                    json!({
                        "request_key": request_key,
                        "session_id": session_id,
                        "tool_call": tool_call,
                        "options": pending.options,
                    }),
                );
                drop(state);
                self.changed.notify_waiters();
            }
            AcpEvent::PermissionClosed { request_key } => {
                let mut state = self.lock();
                state.pending.remove(&request_key);
                self.record_locked(
                    &mut state,
                    "permission_closed",
                    json!({"request_key": request_key, "reason": "cancelled"}),
                );
                drop(state);
                self.changed.notify_waiters();
            }
            AcpEvent::Notification { method, params } => {
                self.record("notification", json!({"method": method, "params": params}))
            }
            AcpEvent::Malformed(reason) => self.record("notice", json!({"message": reason})),
            AcpEvent::Exited(reason) => self.finish(&reason),
        }
    }

    fn handle_turn(&self, result: TurnResult) {
        let mut state = self.lock();
        if state.phase == Phase::Ended {
            return;
        }
        state.phase = Phase::Idle;
        match result {
            TurnResult::Done(stop_reason) => {
                self.record_locked(&mut state, "turn_end", json!({"stop_reason": stop_reason}))
            }
            TurnResult::Failed(message) => {
                self.record_locked(&mut state, "turn_error", json!({"message": message}))
            }
        }
        drop(state);
        self.changed.notify_waiters();
    }
}

/// Reads what the agent sends and what the turn tasks report, in order: every
/// update that arrived before a turn's answer is recorded before the turn's end.
async fn pump(
    session: Arc<Session>,
    mut events: AcpEvents,
    mut turns: mpsc::UnboundedReceiver<TurnResult>,
) {
    loop {
        tokio::select! {
            biased;
            event = events.recv() => match event {
                Some(event) => {
                    let exited = matches!(event, AcpEvent::Exited(_));
                    session.handle_event(event);
                    if exited {
                        return;
                    }
                }
                None => {
                    session.finish("the connection to the agent ended");
                    return;
                }
            },
            result = turns.recv() => {
                let Some(result) = result else { continue };
                while let Some(event) = events.try_recv() {
                    let exited = matches!(event, AcpEvent::Exited(_));
                    session.handle_event(event);
                    if exited {
                        return;
                    }
                }
                session.handle_turn(result);
            }
        }
    }
}

struct Inner {
    connector: Connector,
    /// `<logs>/sessions`; `None` keeps sessions in memory only.
    log_dir: Option<Arc<Dir>>,
    sessions: Mutex<BTreeMap<u64, Arc<Session>>>,
    next_id: AtomicU64,
}

#[derive(Clone)]
pub struct AcpSessions {
    inner: Arc<Inner>,
}

fn folder_label(cwd: &Path) -> String {
    let name = cwd
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(24)
        .collect();
    if cleaned.is_empty() {
        "root".into()
    } else {
        cleaned
    }
}

impl AcpSessions {
    pub fn new(connector: Connector, logs: Option<&Path>) -> Self {
        let log_dir = logs.and_then(|logs| {
            Dir::open_or_create_private_components(logs)
                .and_then(|dir| dir.child_dir("sessions", true))
                .ok()
                .map(Arc::new)
        });
        Self {
            inner: Arc::new(Inner {
                connector,
                log_dir,
                sessions: Mutex::new(BTreeMap::new()),
                next_id: AtomicU64::new(1),
            }),
        }
    }

    fn get(&self, id: u64) -> Result<Arc<Session>, AcpSessionError> {
        self.inner
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&id)
            .cloned()
            .ok_or(AcpSessionError::NotFound)
    }

    /// Opens one session because the person asked for it. With
    /// `resume_agent_session` it reopens that conversation through the agent's
    /// own `session/resume` (or `session/load`) when the agent offers one.
    pub async fn open(
        &self,
        _human: &VerifiedUiOrigin,
        provider: &str,
        cwd: &str,
        resume_agent_session: Option<&str>,
    ) -> Result<SessionView, AcpSessionError> {
        if !declared::is_acp(provider) {
            return Err(AcpSessionError::NotAcp(provider.to_owned()));
        }
        let cwd = Path::new(cwd);
        if !cwd.is_absolute() || !cwd.is_dir() {
            return Err(AcpSessionError::BadRequest(
                "cwd must be an existing absolute folder".into(),
            ));
        }
        let (client, mut events) =
            (self.inner.connector)(provider, cwd).map_err(AcpSessionError::Launch)?;
        let init = tokio::time::timeout(
            STEP_TIMEOUT,
            client.initialize("many-ai-cli", env!("CARGO_PKG_VERSION")),
        )
        .await
        .map_err(|_| AcpSessionError::Agent("the agent did not answer initialize in time".into()))?
        .map_err(from_acp)?;
        let cwd_text = cwd.to_string_lossy().into_owned();
        let agent_session_id = match resume_agent_session {
            None => tokio::time::timeout(STEP_TIMEOUT, client.new_session(&cwd_text))
                .await
                .map_err(|_| {
                    AcpSessionError::Agent("the agent did not answer session/new in time".into())
                })?
                .map_err(|error| match from_acp(error) {
                    AcpSessionError::AuthRequired { message, .. } => {
                        AcpSessionError::AuthRequired {
                            message,
                            methods: init.auth_methods.clone(),
                        }
                    }
                    other => other,
                })?,
            Some(agent_id) => {
                if agent_id.is_empty() {
                    return Err(AcpSessionError::BadRequest(
                        "agent_session_id is empty".into(),
                    ));
                }
                let resumed = if init.capabilities.resume_session {
                    client.resume_session(agent_id, &cwd_text).await
                } else if init.capabilities.load_session {
                    client.load_session(agent_id, &cwd_text).await
                } else {
                    return Err(AcpSessionError::Unsupported(
                        "reopening a conversation (session/resume, session/load)".into(),
                    ));
                };
                resumed.map_err(|error| match from_acp(error) {
                    AcpSessionError::AuthRequired { message, .. } => {
                        AcpSessionError::AuthRequired {
                            message,
                            methods: init.auth_methods.clone(),
                        }
                    }
                    other => other,
                })?;
                agent_id.to_owned()
            }
        };
        let id = self.inner.next_id.fetch_add(1, Ordering::SeqCst);
        let started_at = now_text();
        let (log_name, log) = self.open_log(provider, cwd, id);
        let (turn_tx, turn_rx) = mpsc::unbounded_channel();
        let session = Arc::new(Session {
            id,
            provider: provider.to_owned(),
            cwd: cwd.to_path_buf(),
            agent_session_id: agent_session_id.clone(),
            started_at: started_at.clone(),
            log_name,
            capabilities: init.capabilities.clone(),
            client: Mutex::new(Some(client)),
            turns: turn_tx,
            state: Mutex::new(State {
                seq: 0,
                events: VecDeque::new(),
                pending: BTreeMap::new(),
                phase: Phase::Idle,
                ended_reason: None,
                log,
                replaying: resume_agent_session.is_some()
                    && !init.capabilities.resume_session
                    && init.capabilities.load_session,
            }),
            changed: Notify::new(),
        });
        session.record(
            "meta",
            json!({
                "provider": provider,
                "cwd": cwd_text,
                "agent_session_id": agent_session_id,
                "started_at": started_at,
                "resumed": resume_agent_session.is_some(),
                "agent": {"name": init.agent_name, "version": init.agent_version},
            }),
        );
        self.inner
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, session.clone());
        // Whatever the agent sent while the conversation was being opened (a
        // loaded conversation's history) is recorded first, marked as replay.
        while let Some(event) = events.try_recv() {
            session.handle_event(event);
        }
        session.lock().replaying = false;
        tokio::spawn(pump(session.clone(), events, turn_rx));
        Ok(session.view())
    }

    fn open_log(
        &self,
        provider: &str,
        cwd: &Path,
        id: u64,
    ) -> (Option<String>, Option<std::fs::File>) {
        let Some(dir) = &self.inner.log_dir else {
            return (None, None);
        };
        let safe_provider: String = provider
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let name = format!(
            "acp_{safe_provider}_{}_{}_s{id}.jsonl",
            chrono::Local::now().format("%Y%m%d_%H%M%S"),
            folder_label(cwd)
        );
        match dir.open_append(&name) {
            Ok(file) => (Some(name), Some(file)),
            Err(_) => (None, None),
        }
    }

    pub fn list(&self) -> Vec<SessionView> {
        self.inner
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .map(|s| s.view())
            .collect()
    }

    /// Conversations saved by earlier sessions (first line of each log).
    pub fn saved(&self) -> Vec<SavedSession> {
        let Some(dir) = &self.inner.log_dir else {
            return Vec::new();
        };
        let mut found: Vec<SavedSession> = dir
            .entries()
            .unwrap_or_default()
            .into_iter()
            .filter(|name| name.starts_with("acp_") && name.ends_with(".jsonl"))
            .filter_map(|name| {
                let bytes = dir.read(&name, 64 * 1024).ok()?;
                let first = bytes.split(|b| *b == b'\n').next()?;
                let line: Value = serde_json::from_slice(first).ok()?;
                if line["kind"] != "meta" {
                    return None;
                }
                let data = &line["data"];
                let text = |key: &str| data[key].as_str().map(str::to_owned);
                Some(SavedSession {
                    file: name,
                    provider: text("provider")?,
                    cwd: text("cwd")?,
                    agent_session_id: text("agent_session_id")?,
                    started_at: text("started_at").unwrap_or_default(),
                })
            })
            .collect();
        found.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        found
    }

    /// Events after `after`, waiting up to `wait` when there are none yet.
    pub async fn events(
        &self,
        _human: &VerifiedUiOrigin,
        id: u64,
        after: u64,
        wait: Duration,
    ) -> Result<EventsView, AcpSessionError> {
        let session = self.get(id)?;
        let wait = wait.min(MAX_WAIT);
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            let notified = session.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let state = session.lock();
                let fresh: Vec<StoredEvent> = state
                    .events
                    .iter()
                    .filter(|event| event.seq > after)
                    .cloned()
                    .collect();
                if !fresh.is_empty()
                    || wait.is_zero()
                    || state.phase == Phase::Ended
                    || tokio::time::Instant::now() >= deadline
                {
                    let view = EventsView {
                        first_seq: state.events.front().map_or(0, |e| e.seq),
                        last_seq: state.seq,
                        events: fresh,
                        pending_permissions: state.pending.values().cloned().collect(),
                        session: session.view_locked(&state),
                    };
                    return Ok(view);
                }
            }
            let _ = tokio::time::timeout_at(deadline, notified).await;
        }
    }

    /// Sends the person's prompt. Returns once the turn has started; its
    /// updates and its end arrive as events. One turn at a time. Never retried.
    pub async fn prompt(
        &self,
        _human: &VerifiedUiOrigin,
        id: u64,
        text: &str,
    ) -> Result<SessionView, AcpSessionError> {
        if text.trim().is_empty() {
            return Err(AcpSessionError::BadRequest("the prompt is empty".into()));
        }
        let session = self.get(id)?;
        let client = session.client()?;
        {
            let mut state = session.lock();
            match state.phase {
                Phase::Ended => {
                    return Err(AcpSessionError::Ended(
                        state.ended_reason.clone().unwrap_or_default(),
                    ));
                }
                Phase::Running => return Err(AcpSessionError::Busy),
                Phase::Idle => {}
            }
            state.phase = Phase::Running;
            session.record_locked(&mut state, "user_prompt", json!({"text": text}));
        }
        session.changed.notify_waiters();
        let agent_id = session.agent_session_id.clone();
        let blocks = vec![text_block(text)];
        let runner = session.clone();
        tokio::spawn(async move {
            let result = match client.prompt(&agent_id, blocks).await {
                Ok(stop) => TurnResult::Done(stop),
                Err(error) => TurnResult::Failed(error.to_string()),
            };
            let _ = runner.turns.send(result);
        });
        Ok(session.view())
    }

    /// The person's answer to one stored permission request.
    pub async fn answer_permission(
        &self,
        _human: &VerifiedUiOrigin,
        id: u64,
        request_key: u64,
        decision: PermissionDecision,
    ) -> Result<(), AcpSessionError> {
        let session = self.get(id)?;
        let client = session.client()?;
        if !session.lock().pending.contains_key(&request_key) {
            return Err(AcpSessionError::BadRequest(
                "no such pending permission request".into(),
            ));
        }
        let chosen = match &decision {
            PermissionDecision::Select(option) => json!({"option_id": option}),
            PermissionDecision::Cancelled => json!({"cancelled": true}),
        };
        client
            .answer_permission(request_key, decision)
            .await
            .map_err(|error| match error {
                AcpError::Invalid(reason) => AcpSessionError::BadRequest(reason),
                other => from_acp(other),
            })?;
        let mut state = session.lock();
        state.pending.remove(&request_key);
        session.record_locked(
            &mut state,
            "permission_answered",
            json!({"request_key": request_key, "answer": chosen, "by": "dashboard"}),
        );
        drop(state);
        session.changed.notify_waiters();
        Ok(())
    }

    /// Stops the running turn. Open permission requests of the session are
    /// closed as cancelled by the protocol, not approved.
    pub async fn cancel(&self, _human: &VerifiedUiOrigin, id: u64) -> Result<(), AcpSessionError> {
        let session = self.get(id)?;
        let client = session.client()?;
        client
            .cancel(&session.agent_session_id)
            .await
            .map_err(from_acp)
    }

    /// The agent's own list of conversations, when it offers one.
    pub async fn agent_sessions(
        &self,
        _human: &VerifiedUiOrigin,
        id: u64,
        cursor: Option<&str>,
    ) -> Result<Value, AcpSessionError> {
        let session = self.get(id)?;
        let client = session.client()?;
        let cwd = session.cwd.to_string_lossy().into_owned();
        let list = client
            .list_sessions(Some(&cwd), cursor)
            .await
            .map_err(from_acp)?;
        Ok(json!({
            "sessions": list.sessions.iter().map(|s| json!({
                "agent_session_id": s.session_id,
                "cwd": s.cwd,
                "title": s.title,
                "updated_at": s.updated_at,
            })).collect::<Vec<_>>(),
            "next_cursor": list.next_cursor,
        }))
    }

    /// Ends the session at the person's request: asks the agent to close the
    /// conversation when it can, then drops the connection (which stops the
    /// agent process). The saved log stays for a later resume.
    pub async fn end(
        &self,
        _human: &VerifiedUiOrigin,
        id: u64,
    ) -> Result<SessionView, AcpSessionError> {
        let session = self.get(id)?;
        if session.lock().phase != Phase::Ended
            && session.capabilities.close_session
            && let Ok(client) = session.client()
        {
            let _ = tokio::time::timeout(
                Duration::from_secs(5),
                client.close_session(&session.agent_session_id),
            )
            .await;
        }
        session.finish("ended by the person");
        Ok(session.view())
    }
}

/// Starts the agent named by the provider's own `launch` declaration and
/// connects to it over stdio. `environment` is the Hub's `KEY=value` list, used
/// only to find the executable; the child gets the allow-listed environment of
/// `crate::acp::spawn_process`.
pub fn native_connector(environment: Vec<String>) -> Connector {
    use crate::process::execpath::{NativeFs, Platform, Resolver};
    Arc::new(move |provider, cwd| {
        let launch = declared::with_declared(provider, |d| d.launch.clone())
            .flatten()
            .ok_or_else(|| format!("provider {provider} has no launch definition"))?;
        let resolver = Resolver::new(Platform::native(), &environment, cwd, &NativeFs);
        let candidates = std::iter::once(&launch.executable)
            .filter(|s| !s.is_empty())
            .chain(launch.executable_candidates.iter());
        let mut found = None;
        for candidate in candidates {
            if let Ok(path) = resolver.look_path(candidate) {
                found = Some(path);
                break;
            }
        }
        let path = found.ok_or_else(|| format!("the executable of {provider} was not found"))?;
        let command = resolver.resolve(&path, &launch.args);
        let (client, events) = crate::acp::spawn_process(crate::acp::AcpProcessSpec {
            executable: PathBuf::from(command.executable),
            args: command.args.into_iter().map(Into::into).collect(),
            cwd: cwd.to_path_buf(),
        })
        .map_err(|error| error.to_string())?;
        Ok((client, events))
    })
}

#[cfg(test)]
pub(crate) mod tests;
