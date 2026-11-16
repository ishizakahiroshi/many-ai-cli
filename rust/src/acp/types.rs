//! Public types of the ACP client.

use std::fmt;

use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub enum AcpError {
    /// The agent answered with a JSON-RPC error.
    Rpc {
        code: i64,
        message: String,
        data: Option<Value>,
    },
    /// The agent needs the user to sign in through its own runtime.
    AuthRequired { message: String },
    /// The process ended, the stream broke or the client was closed. Every
    /// request still waiting finishes with this.
    Closed(String),
    /// The agent sent something the protocol does not allow.
    Protocol(String),
    /// The agent did not advertise the capability the call needs.
    Unsupported(&'static str),
    /// A caller mistake (for example an unknown permission option).
    Invalid(String),
}

impl fmt::Display for AcpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AcpError::Rpc { code, message, .. } => write!(f, "agent error {code}: {message}"),
            AcpError::AuthRequired { message } => write!(f, "sign-in required: {message}"),
            AcpError::Closed(reason) => write!(f, "connection closed: {reason}"),
            AcpError::Protocol(reason) => write!(f, "protocol error: {reason}"),
            AcpError::Unsupported(what) => write!(f, "not supported by the agent: {what}"),
            AcpError::Invalid(reason) => write!(f, "invalid request: {reason}"),
        }
    }
}

impl std::error::Error for AcpError {}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentCapabilities {
    pub load_session: bool,
    pub list_sessions: bool,
    pub resume_session: bool,
    pub close_session: bool,
    pub image: bool,
    pub audio: bool,
    pub embedded_context: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthMethod {
    pub id: String,
    pub name: String,
    /// `agent` when the entry has no `type`, otherwise the given type
    /// (for example `terminal`).
    pub kind: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InitializeResult {
    pub protocol_version: u64,
    pub capabilities: AgentCapabilities,
    pub auth_methods: Vec<AuthMethod>,
    pub agent_name: Option<String>,
    pub agent_version: Option<String>,
    /// The untouched `result`, for callers that need a field not parsed here.
    pub raw: Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionInfo {
    pub session_id: String,
    pub cwd: Option<String>,
    pub title: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionList {
    pub sessions: Vec<SessionInfo>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionOption {
    pub option_id: String,
    pub name: String,
    /// `allow_once`, `allow_always`, `reject_once` or `reject_always`.
    pub kind: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PermissionRequest {
    /// Local handle; pass it to `AcpClient::answer_permission`.
    pub request_key: u64,
    pub session_id: String,
    pub tool_call: Value,
    pub options: Vec<PermissionOption>,
}

/// What the person at the screen chose. Never produced by this crate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PermissionDecision {
    Select(String),
    Cancelled,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AcpEvent {
    /// `session/update` notification; `update` is the `params.update` object
    /// (its `sessionUpdate` field names the kind).
    Update { session_id: String, update: Value },
    /// The agent asks the person to allow or reject a tool call.
    PermissionRequest(PermissionRequest),
    /// The agent withdrew or the client cancelled the request.
    PermissionClosed { request_key: u64 },
    /// A notification this client does not interpret.
    Notification { method: String, params: Value },
    /// A line that was not a valid message (the stream continues).
    Malformed(String),
    /// The connection ended. Last event; pending requests already failed.
    Exited(String),
}
