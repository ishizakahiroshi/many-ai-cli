//! The ACP client: request/response correlation, the update stream, and the
//! rules for what happens when the connection ends.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

use super::framing::{LineDecoder, encode_message};
use super::types::{
    AcpError, AcpEvent, AgentCapabilities, AuthMethod, InitializeResult, PermissionDecision,
    PermissionOption, PermissionRequest, SessionInfo, SessionList,
};

/// The only ACP protocol version this client speaks.
const PROTOCOL_VERSION: u64 = 1;

/// What the transport hands to the client.
#[derive(Debug)]
pub enum Incoming {
    Bytes(Vec<u8>),
    /// The stream ended (process exit, read failure). Always the last item.
    Closed(String),
}

type Reply = Result<Value, AcpError>;

struct OpenPermission {
    agent_id: Value,
    session_id: String,
    options: Vec<PermissionOption>,
}

#[derive(Default)]
struct State {
    next_id: u64,
    next_key: u64,
    pending: HashMap<u64, oneshot::Sender<Reply>>,
    permissions: HashMap<u64, OpenPermission>,
    closed: Option<String>,
    capabilities: Option<AgentCapabilities>,
}

struct Core {
    state: Mutex<State>,
    out: mpsc::Sender<Vec<u8>>,
    events: mpsc::UnboundedSender<AcpEvent>,
}

struct Inner {
    core: Arc<Core>,
    /// Dropped together with the last client handle; the transport reads that
    /// as "stop the process".
    _stop: Option<oneshot::Sender<()>>,
}

/// Cloneable handle. Requests can run concurrently from several tasks.
#[derive(Clone)]
pub struct AcpClient {
    inner: Arc<Inner>,
}

/// Receiver of everything that is not a response to a request.
pub struct AcpEvents {
    receiver: mpsc::UnboundedReceiver<AcpEvent>,
}

impl AcpEvents {
    pub async fn recv(&mut self) -> Option<AcpEvent> {
        self.receiver.recv().await
    }
    pub fn try_recv(&mut self) -> Option<AcpEvent> {
        self.receiver.try_recv().ok()
    }
}

struct PendingGuard {
    core: Arc<Core>,
    id: u64,
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        self.core.lock().pending.remove(&self.id);
    }
}

impl Core {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Ends the connection once: fails every pending request, forgets the
    /// open permission requests and tells the event stream.
    fn close(&self, reason: String) {
        let pending = {
            let mut state = self.lock();
            if state.closed.is_some() {
                return;
            }
            state.closed = Some(reason.clone());
            state.permissions.clear();
            std::mem::take(&mut state.pending)
        };
        for (_, sender) in pending {
            let _ = sender.send(Err(AcpError::Closed(reason.clone())));
        }
        let _ = self.events.send(AcpEvent::Exited(reason));
    }

    async fn send(&self, message: &Value) -> Result<(), AcpError> {
        if let Some(reason) = self.lock().closed.clone() {
            return Err(AcpError::Closed(reason));
        }
        if self.out.send(encode_message(message)).await.is_err() {
            self.close("the agent's input is closed".into());
            return Err(AcpError::Closed("the agent's input is closed".into()));
        }
        Ok(())
    }

    async fn respond(&self, id: Value, result: Value) {
        let _ = self
            .send(&json!({"jsonrpc":"2.0","id":id,"result":result}))
            .await;
    }

    async fn respond_error(&self, id: Value, code: i64, message: &str) {
        let _ = self
            .send(&json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}}))
            .await;
    }

    async fn handle_line(&self, line: &[u8]) {
        let value: Value = match serde_json::from_slice(line) {
            Ok(value) => value,
            Err(error) => {
                let _ = self
                    .events
                    .send(AcpEvent::Malformed(format!("invalid JSON: {error}")));
                return;
            }
        };
        let Some(object) = value.as_object() else {
            let _ = self
                .events
                .send(AcpEvent::Malformed("message is not an object".into()));
            return;
        };
        let method = object.get("method").and_then(Value::as_str);
        let id = object.get("id").filter(|id| !id.is_null());
        let params = object.get("params").cloned().unwrap_or(Value::Null);
        match (method, id) {
            (Some(method), Some(id)) => self.handle_request(method, id.clone(), params).await,
            (Some(method), None) => self.handle_notification(method, params),
            (None, Some(id)) => self.handle_response(id, object),
            (None, None) => {
                let _ = self
                    .events
                    .send(AcpEvent::Malformed("message has no method or id".into()));
            }
        }
    }

    fn handle_response(&self, id: &Value, object: &serde_json::Map<String, Value>) {
        let Some(id) = id.as_u64() else {
            let _ = self
                .events
                .send(AcpEvent::Malformed("response id is not one of ours".into()));
            return;
        };
        let Some(sender) = self.lock().pending.remove(&id) else {
            let _ = self.events.send(AcpEvent::Malformed(format!(
                "response for unknown request {id}"
            )));
            return;
        };
        let reply = if let Some(error) = object.get("error") {
            Err(map_error(error))
        } else if let Some(result) = object.get("result") {
            Ok(result.clone())
        } else {
            Err(AcpError::Protocol(
                "response has neither result nor error".into(),
            ))
        };
        let _ = sender.send(reply);
    }

    fn handle_notification(&self, method: &str, params: Value) {
        if method == "session/update" {
            let session_id = params.get("sessionId").and_then(Value::as_str);
            let update = params.get("update");
            if let (Some(session_id), Some(update)) = (session_id, update) {
                let _ = self.events.send(AcpEvent::Update {
                    session_id: session_id.to_owned(),
                    update: update.clone(),
                });
            } else {
                let _ = self.events.send(AcpEvent::Malformed(
                    "session/update without sessionId or update".into(),
                ));
            }
            return;
        }
        let _ = self.events.send(AcpEvent::Notification {
            method: method.to_owned(),
            params,
        });
    }

    async fn handle_request(&self, method: &str, id: Value, params: Value) {
        if method != "session/request_permission" {
            // We advertise no file, terminal or elicitation capability, so any
            // other request from the agent is answered "method not found".
            self.respond_error(id, -32601, "method not found").await;
            return;
        }
        let session_id = params.get("sessionId").and_then(Value::as_str);
        let options = params.get("options").and_then(Value::as_array).map(|list| {
            list.iter()
                .filter_map(|option| {
                    Some(PermissionOption {
                        option_id: option.get("optionId")?.as_str()?.to_owned(),
                        name: option
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        kind: option
                            .get("kind")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                    })
                })
                .collect::<Vec<_>>()
        });
        let (Some(session_id), Some(options)) = (session_id, options) else {
            self.respond_error(id, -32602, "invalid params").await;
            return;
        };
        if options.is_empty() {
            self.respond_error(id, -32602, "no options").await;
            return;
        }
        let request = {
            let mut state = self.lock();
            if state.closed.is_some() {
                return;
            }
            let key = state.next_key;
            state.next_key += 1;
            state.permissions.insert(
                key,
                OpenPermission {
                    agent_id: id,
                    session_id: session_id.to_owned(),
                    options: options.clone(),
                },
            );
            PermissionRequest {
                request_key: key,
                session_id: session_id.to_owned(),
                tool_call: params.get("toolCall").cloned().unwrap_or(Value::Null),
                options,
            }
        };
        // Only a caller-supplied decision answers it (`answer_permission`).
        let _ = self.events.send(AcpEvent::PermissionRequest(request));
    }
}

/// `-32000` with an "authentication required" message is how the agent says
/// the user has not signed in (official ACP page for the agent runtime).
fn map_error(error: &Value) -> AcpError {
    let code = error.get("code").and_then(Value::as_i64).unwrap_or(0);
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let lower = message.to_lowercase();
    if lower.contains("authentication required")
        || lower.contains("auth_required")
        || (code == -32000 && lower.contains("auth"))
    {
        return AcpError::AuthRequired { message };
    }
    AcpError::Rpc {
        code,
        message,
        data: error.get("data").cloned(),
    }
}

async fn read_loop(core: Arc<Core>, mut incoming: mpsc::Receiver<Incoming>) {
    let mut decoder = LineDecoder::default();
    loop {
        match incoming.recv().await {
            Some(Incoming::Bytes(bytes)) => match decoder.push(&bytes) {
                Ok(lines) => {
                    for line in lines {
                        core.handle_line(&line).await;
                    }
                }
                Err(reason) => {
                    core.close(reason);
                    return;
                }
            },
            Some(Incoming::Closed(reason)) => {
                core.close(reason);
                return;
            }
            None => {
                core.close("the transport ended".into());
                return;
            }
        }
    }
}

/// Parses an `initialize` result. Capability flags count only when present
/// and not `false`/`null`.
fn parse_initialize(raw: Value) -> Result<InitializeResult, AcpError> {
    let version = raw
        .get("protocolVersion")
        .and_then(Value::as_u64)
        .ok_or_else(|| AcpError::Protocol("initialize result has no protocolVersion".into()))?;
    if version != PROTOCOL_VERSION {
        return Err(AcpError::Protocol(format!(
            "the agent speaks protocol version {version}; this client speaks {PROTOCOL_VERSION}"
        )));
    }
    let present = |value: Option<&Value>| match value {
        None | Some(Value::Null) | Some(Value::Bool(false)) => false,
        Some(_) => true,
    };
    let caps = raw.get("agentCapabilities").unwrap_or(&Value::Null);
    let session = caps.get("sessionCapabilities");
    let prompt = caps.get("promptCapabilities");
    let capabilities = AgentCapabilities {
        load_session: present(caps.get("loadSession")),
        list_sessions: present(session.and_then(|s| s.get("list"))),
        resume_session: present(session.and_then(|s| s.get("resume"))),
        close_session: present(session.and_then(|s| s.get("close"))),
        image: present(prompt.and_then(|p| p.get("image"))),
        audio: present(prompt.and_then(|p| p.get("audio"))),
        embedded_context: present(prompt.and_then(|p| p.get("embeddedContext"))),
    };
    let auth_methods = raw
        .get("authMethods")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|method| {
                    Some(AuthMethod {
                        id: method.get("id")?.as_str()?.to_owned(),
                        name: method
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        kind: method
                            .get("type")
                            .and_then(Value::as_str)
                            .unwrap_or("agent")
                            .to_owned(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let info = raw.get("agentInfo");
    Ok(InitializeResult {
        protocol_version: version,
        capabilities,
        auth_methods,
        agent_name: info
            .and_then(|i| i.get("name"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        agent_version: info
            .and_then(|i| i.get("version"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        raw,
    })
}

/// A `text` content block for `AcpClient::prompt`.
pub fn text_block(text: &str) -> Value {
    json!({"type":"text","text":text})
}

impl AcpClient {
    /// Builds a client over a transport: `out` receives encoded lines to write
    /// to the agent, `incoming` delivers what the agent wrote. `stop` is held
    /// until the last handle is dropped.
    pub fn from_transport(
        out: mpsc::Sender<Vec<u8>>,
        incoming: mpsc::Receiver<Incoming>,
        stop: Option<oneshot::Sender<()>>,
    ) -> (AcpClient, AcpEvents) {
        let (events, receiver) = mpsc::unbounded_channel();
        let core = Arc::new(Core {
            state: Mutex::new(State::default()),
            out,
            events,
        });
        tokio::spawn(read_loop(core.clone(), incoming));
        (
            AcpClient {
                inner: Arc::new(Inner { core, _stop: stop }),
            },
            AcpEvents { receiver },
        )
    }

    fn core(&self) -> &Arc<Core> {
        &self.inner.core
    }

    /// True once the connection has ended.
    pub fn is_closed(&self) -> bool {
        self.core().lock().closed.is_some()
    }

    async fn request(&self, method: &str, params: Value) -> Reply {
        let core = self.core();
        let (sender, receiver) = oneshot::channel();
        let id = {
            let mut state = core.lock();
            if let Some(reason) = &state.closed {
                return Err(AcpError::Closed(reason.clone()));
            }
            let id = state.next_id;
            state.next_id += 1;
            state.pending.insert(id, sender);
            id
        };
        let _guard = PendingGuard {
            core: core.clone(),
            id,
        };
        core.send(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
            .await?;
        match receiver.await {
            Ok(reply) => reply,
            Err(_) => Err(AcpError::Closed("the request was dropped".into())),
        }
    }

    fn require(
        &self,
        pick: impl Fn(&AgentCapabilities) -> bool,
        what: &'static str,
    ) -> Result<(), AcpError> {
        match &self.core().lock().capabilities {
            None => Err(AcpError::Invalid("initialize has not completed".into())),
            Some(caps) if pick(caps) => Ok(()),
            Some(_) => Err(AcpError::Unsupported(what)),
        }
    }

    /// Exchanges versions and capabilities. Advertises no file, terminal,
    /// elicitation or terminal-auth capability, so the agent cannot ask for them.
    pub async fn initialize(
        &self,
        client_name: &str,
        client_version: &str,
    ) -> Result<InitializeResult, AcpError> {
        let raw = self
            .request(
                "initialize",
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "clientCapabilities": {},
                    "clientInfo": {
                        "name": client_name,
                        "title": client_name,
                        "version": client_version,
                    },
                }),
            )
            .await?;
        let result = parse_initialize(raw)?;
        self.core().lock().capabilities = Some(result.capabilities.clone());
        Ok(result)
    }

    /// Creates a session. Returns `AcpError::AuthRequired` when the user has
    /// not signed in through the agent's own runtime.
    pub async fn new_session(&self, cwd: &str) -> Result<String, AcpError> {
        let result = self
            .request("session/new", json!({"cwd": cwd, "mcpServers": []}))
            .await?;
        result
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| AcpError::Protocol("session/new result has no sessionId".into()))
    }

    /// Sends a prompt and waits for the end of the turn. Updates for the turn
    /// arrive on the event stream before this returns. Returns the
    /// `stopReason` (`end_turn`, `cancelled`, ...).
    pub async fn prompt(&self, session_id: &str, blocks: Vec<Value>) -> Result<String, AcpError> {
        let result = self
            .request(
                "session/prompt",
                json!({"sessionId": session_id, "prompt": blocks}),
            )
            .await?;
        result
            .get("stopReason")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| AcpError::Protocol("session/prompt result has no stopReason".into()))
    }

    /// Answers one open permission request with the person's decision.
    pub async fn answer_permission(
        &self,
        request_key: u64,
        decision: PermissionDecision,
    ) -> Result<(), AcpError> {
        let core = self.core();
        let open = {
            let mut state = core.lock();
            if let Some(reason) = &state.closed {
                return Err(AcpError::Closed(reason.clone()));
            }
            let Some(open) = state.permissions.get(&request_key) else {
                return Err(AcpError::Invalid(
                    "unknown or already answered permission request".into(),
                ));
            };
            if let PermissionDecision::Select(option_id) = &decision
                && !open.options.iter().any(|o| &o.option_id == option_id)
            {
                return Err(AcpError::Invalid("unknown permission option".into()));
            }
            state
                .permissions
                .remove(&request_key)
                .expect("checked above")
        };
        let outcome = match decision {
            PermissionDecision::Select(option_id) => {
                json!({"outcome":"selected","optionId":option_id})
            }
            PermissionDecision::Cancelled => json!({"outcome":"cancelled"}),
        };
        core.respond(open.agent_id, json!({"outcome": outcome}))
            .await;
        Ok(())
    }

    /// Cancels the running turn of a session. The protocol also requires that
    /// the session's open permission requests are answered `cancelled`; that is
    /// done here and is not an approval.
    pub async fn cancel(&self, session_id: &str) -> Result<(), AcpError> {
        let core = self.core();
        core.send(&json!({
            "jsonrpc":"2.0",
            "method":"session/cancel",
            "params":{"sessionId":session_id},
        }))
        .await?;
        let open: Vec<(u64, OpenPermission)> = {
            let mut state = core.lock();
            let keys: Vec<u64> = state
                .permissions
                .iter()
                .filter(|(_, open)| open.session_id == session_id)
                .map(|(key, _)| *key)
                .collect();
            keys.into_iter()
                .filter_map(|key| state.permissions.remove(&key).map(|open| (key, open)))
                .collect()
        };
        for (key, open) in open {
            core.respond(open.agent_id, json!({"outcome":{"outcome":"cancelled"}}))
                .await;
            let _ = core
                .events
                .send(AcpEvent::PermissionClosed { request_key: key });
        }
        Ok(())
    }

    /// Needs the `sessionCapabilities.list` capability.
    pub async fn list_sessions(
        &self,
        cwd: Option<&str>,
        cursor: Option<&str>,
    ) -> Result<SessionList, AcpError> {
        self.require(|c| c.list_sessions, "session/list")?;
        let mut params = serde_json::Map::new();
        if let Some(cwd) = cwd {
            params.insert("cwd".into(), json!(cwd));
        }
        if let Some(cursor) = cursor {
            params.insert("cursor".into(), json!(cursor));
        }
        let result = self.request("session/list", Value::Object(params)).await?;
        let text =
            |item: &Value, key: &str| item.get(key).and_then(Value::as_str).map(str::to_owned);
        let sessions = result
            .get("sessions")
            .and_then(Value::as_array)
            .ok_or_else(|| AcpError::Protocol("session/list result has no sessions".into()))?
            .iter()
            .filter_map(|item| {
                Some(SessionInfo {
                    session_id: text(item, "sessionId")?,
                    cwd: text(item, "cwd"),
                    title: text(item, "title"),
                    updated_at: text(item, "updatedAt"),
                })
            })
            .collect();
        Ok(SessionList {
            sessions,
            next_cursor: text(&result, "nextCursor"),
        })
    }

    /// Needs `loadSession`. The history arrives as `session/update` events
    /// before this returns.
    pub async fn load_session(&self, session_id: &str, cwd: &str) -> Result<(), AcpError> {
        self.require(|c| c.load_session, "session/load")?;
        self.request(
            "session/load",
            json!({"sessionId": session_id, "cwd": cwd, "mcpServers": []}),
        )
        .await
        .map(|_| ())
    }

    /// Needs `sessionCapabilities.resume`. Reopens without replaying history.
    pub async fn resume_session(&self, session_id: &str, cwd: &str) -> Result<(), AcpError> {
        self.require(|c| c.resume_session, "session/resume")?;
        self.request(
            "session/resume",
            json!({"sessionId": session_id, "cwd": cwd, "mcpServers": []}),
        )
        .await
        .map(|_| ())
    }

    /// Needs `sessionCapabilities.close`. The protocol says a `-32002` error
    /// (unknown or inactive session) counts as closed.
    pub async fn close_session(&self, session_id: &str) -> Result<(), AcpError> {
        self.require(|c| c.close_session, "session/close")?;
        match self
            .request("session/close", json!({"sessionId": session_id}))
            .await
        {
            Ok(_) => Ok(()),
            Err(AcpError::Rpc { code: -32002, .. }) => Ok(()),
            Err(error) => Err(error),
        }
    }
}
