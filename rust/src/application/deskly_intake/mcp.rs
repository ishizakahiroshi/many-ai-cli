//! Authenticated, stateless JSON-RPC client for Deskly's existing machine MCP
//! (`POST /mcp/development-runtime`). One request per call: no MCP session id,
//! no redirects (the bearer key never follows a Location), bounded bodies.
use super::wire::{
    self, Discovery, HandoffApplied, IssuedHandoff, Preview, PreviewToken, Receipt, WireError,
};
use serde_json::{Value, json};
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

pub const MCP_PATH: &str = "/mcp/development-runtime";
pub const DISCOVER_TOOL: &str = "development_runtime_discover_approved_relay";
pub const CLAIM_TOOL: &str = "development_runtime_relay_claim";
pub const HANDOFF_TOOL: &str = "development_runtime_relay_handoff";
pub const VALIDATE_TOOL: &str = "development_runtime_relay_validate_issued";
/// Deskly bounds its own responses at 128 KiB (handoff output at 131072 bytes
/// of tool text); the JSON-RPC envelope adds escaping, so allow headroom.
const MAX_RESPONSE_BYTES: usize = 512 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// Same rule as Deskly's `targetUrl`: HTTPS, or literal-loopback HTTP for a
/// local harness; exact path; no userinfo, query or fragment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint(url::Url);
impl Endpoint {
    pub fn parse(raw: &str) -> Result<Self, &'static str> {
        let url = url::Url::parse(raw).map_err(|_| "invalid endpoint")?;
        let loopback = matches!(
            url.host(),
            Some(url::Host::Ipv4(ip)) if ip.is_loopback()
        ) || matches!(url.host(), Some(url::Host::Ipv6(ip)) if ip.is_loopback());
        let scheme_ok = url.scheme() == "https" || (url.scheme() == "http" && loopback);
        if !scheme_ok
            || url.path() != MCP_PATH
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || raw.contains('?')
            || raw.contains('#')
        {
            return Err("invalid endpoint");
        }
        Ok(Self(url))
    }
}

/// The registered orchestrator key. Never printed: Debug is redacted and there
/// is no Display or accessor outside the request builder.
pub struct MachineKey(String);
impl MachineKey {
    pub fn new(value: String) -> Result<Self, &'static str> {
        let ok = (16..=4096).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_graphic());
        if !ok {
            return Err("invalid machine key");
        }
        Ok(Self(value))
    }
}
impl std::fmt::Debug for MachineKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MachineKey([redacted])")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallError {
    /// 401: key missing, revoked or not registered. Nothing reached the tool.
    Unauthorized,
    /// The server answered and the call was not executed (4xx, JSON-RPC error).
    Rejected(u16),
    /// The request may have reached the server but the reply is unusable
    /// (timeout after send, 5xx, truncated or malformed body, oversized).
    /// For a mutating tool this is "outcome unknown" and must not be retried.
    Unknown(&'static str),
    /// Connection never established; the request was not delivered.
    NotDelivered,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolReply {
    Data(Value),
    /// Deskly's `{code, retryable:false}` refusal (`isError: true`).
    Refused(String),
}

pub struct McpClient {
    http: reqwest::Client,
    endpoint: Endpoint,
    key: MachineKey,
    next_id: AtomicU64,
}
impl McpClient {
    pub fn new(endpoint: Endpoint, key: MachineKey) -> Result<Self, &'static str> {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|_| "http client")?;
        Ok(Self {
            http,
            endpoint,
            key,
            next_id: AtomicU64::new(1),
        })
    }
    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<ToolReply, CallError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let body = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments },
        });
        let response = self
            .http
            .post(self.endpoint.0.clone())
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .header("authorization", format!("Bearer {}", self.key.0))
            .json(&body)
            .send()
            .await
            .map_err(|error| {
                if error.is_connect() || error.is_builder() {
                    CallError::NotDelivered
                } else {
                    CallError::Unknown("transport")
                }
            })?;
        let status = response.status().as_u16();
        match status {
            200 => {}
            401 => return Err(CallError::Unauthorized),
            400..=499 => return Err(CallError::Rejected(status)),
            _ => return Err(CallError::Unknown("server status")),
        }
        let bytes = read_bounded(response).await?;
        parse_reply(&bytes, id)
    }
}
async fn read_bounded(mut response: reqwest::Response) -> Result<Vec<u8>, CallError> {
    if response
        .content_length()
        .is_some_and(|len| len as usize > MAX_RESPONSE_BYTES)
    {
        return Err(CallError::Unknown("response too large"));
    }
    let mut out = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| CallError::Unknown("response body"))?
    {
        if out.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(CallError::Unknown("response too large"));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}
/// The stateless SDK answers with either plain JSON or one SSE `data:` frame.
pub(super) fn parse_reply(bytes: &[u8], id: u64) -> Result<ToolReply, CallError> {
    let text = std::str::from_utf8(bytes).map_err(|_| CallError::Unknown("utf8"))?;
    let frame = if text.trim_start().starts_with('{') {
        text.trim()
    } else {
        let mut data = text.lines().filter_map(|line| line.strip_prefix("data: "));
        match (data.next(), data.next()) {
            (Some(line), None) => line,
            _ => return Err(CallError::Unknown("sse frame")),
        }
    };
    let message: Value = serde_json::from_str(frame).map_err(|_| CallError::Unknown("json"))?;
    if message.get("jsonrpc") != Some(&json!("2.0")) || message.get("id") != Some(&json!(id)) {
        return Err(CallError::Unknown("json-rpc envelope"));
    }
    if message.get("error").is_some() {
        return Err(CallError::Rejected(200));
    }
    let result = message
        .get("result")
        .and_then(Value::as_object)
        .ok_or(CallError::Unknown("json-rpc result"))?;
    let content = result
        .get("content")
        .and_then(Value::as_array)
        .filter(|items| items.len() == 1)
        .ok_or(CallError::Unknown("tool content"))?;
    let item = content[0]
        .as_object()
        .filter(|item| item.get("type") == Some(&json!("text")))
        .and_then(|item| item.get("text"))
        .and_then(Value::as_str)
        .ok_or(CallError::Unknown("tool content"))?;
    let is_error = match result.get("isError") {
        None | Some(Value::Bool(false)) => false,
        Some(Value::Bool(true)) => true,
        Some(_) => return Err(CallError::Unknown("isError")),
    };
    if is_error {
        // Argument-schema failures may arrive as plain text, not JSON.
        let code = serde_json::from_str::<Value>(item)
            .ok()
            .and_then(|value| value.get("code").and_then(Value::as_str).map(str::to_owned))
            .unwrap_or_else(|| "refused".to_owned());
        return Ok(ToolReply::Refused(code));
    }
    serde_json::from_str(item)
        .map(ToolReply::Data)
        .map_err(|_| CallError::Unknown("tool json"))
}

/// Result of one Deskly authority step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step<T> {
    Done(T),
    /// The server definitely did not apply the step.
    Refused(String),
    /// The step may or may not have been applied. Hold; never resend.
    Unknown(&'static str),
}
fn step<T>(
    reply: Result<ToolReply, CallError>,
    decode: impl FnOnce(Value) -> Result<T, WireError>,
) -> Step<T> {
    match reply {
        Ok(ToolReply::Data(value)) => match decode(value) {
            Ok(value) => Step::Done(value),
            // The server answered "ok" with a shape we refuse: it may have applied.
            Err(WireError(reason)) => Step::Unknown(reason),
        },
        Ok(ToolReply::Refused(code)) => Step::Refused(code),
        Err(CallError::Unauthorized) => Step::Refused("unauthorized".into()),
        Err(CallError::Rejected(status)) => Step::Refused(format!("rejected_{status}")),
        Err(CallError::NotDelivered) => Step::Refused("not_delivered".into()),
        Err(CallError::Unknown(reason)) => Step::Unknown(reason),
    }
}

/// The existing stage-4 authority surface the intake reuses. Implementations
/// must not add a lease, cursor or second ledger.
#[allow(async_fn_in_trait)]
pub trait DesklyRelay {
    async fn discover(&self) -> Result<Discovery, String>;
    async fn claim_preview(
        &self,
        request_id: &str,
        revision: u64,
        session_id: &str,
    ) -> Step<Preview>;
    async fn claim_apply(
        &self,
        request_id: &str,
        revision: u64,
        session_id: &str,
        token: &PreviewToken,
    ) -> Step<Receipt>;
    async fn handoff_preview(
        &self,
        request_id: &str,
        revision: u64,
        attempt_id: &str,
    ) -> Step<Preview>;
    async fn handoff_apply(
        &self,
        request_id: &str,
        revision: u64,
        attempt_id: &str,
        token: &PreviewToken,
    ) -> Step<HandoffApplied>;
    async fn validate_issued(&self, revision: u64, issued: &IssuedHandoff) -> Step<i64>;
}
impl DesklyRelay for McpClient {
    async fn discover(&self) -> Result<Discovery, String> {
        match self.call_tool(DISCOVER_TOOL, json!({})).await {
            Ok(ToolReply::Data(value)) => {
                wire::decode_discovery(value).map_err(|WireError(reason)| reason.to_owned())
            }
            Ok(ToolReply::Refused(code)) => Err(code),
            Err(error) => Err(format!("{error:?}")),
        }
    }
    async fn claim_preview(
        &self,
        request_id: &str,
        revision: u64,
        session_id: &str,
    ) -> Step<Preview> {
        let args = json!({"request_id": request_id, "expected_revision": revision, "session_id": session_id});
        step(self.call_tool(CLAIM_TOOL, args).await, |value| {
            wire::decode_preview(value, request_id, false)
        })
    }
    async fn claim_apply(
        &self,
        request_id: &str,
        revision: u64,
        session_id: &str,
        token: &PreviewToken,
    ) -> Step<Receipt> {
        let args = json!({"request_id": request_id, "expected_revision": revision,
            "session_id": session_id, "apply": true, "preview_token": token});
        step(self.call_tool(CLAIM_TOOL, args).await, |value| {
            wire::decode_claim_applied(value, request_id)
        })
    }
    async fn handoff_preview(
        &self,
        request_id: &str,
        revision: u64,
        attempt_id: &str,
    ) -> Step<Preview> {
        let args = json!({"request_id": request_id, "expected_revision": revision,
            "generation": 1, "client_attempt_id": attempt_id});
        step(self.call_tool(HANDOFF_TOOL, args).await, |value| {
            wire::decode_preview(value, request_id, true)
        })
    }
    async fn handoff_apply(
        &self,
        request_id: &str,
        revision: u64,
        attempt_id: &str,
        token: &PreviewToken,
    ) -> Step<HandoffApplied> {
        let args = json!({"request_id": request_id, "expected_revision": revision,
            "generation": 1, "client_attempt_id": attempt_id, "apply": true, "preview_token": token});
        step(self.call_tool(HANDOFF_TOOL, args).await, |value| {
            wire::decode_handoff_applied(value, request_id, attempt_id)
        })
    }
    async fn validate_issued(&self, revision: u64, issued: &IssuedHandoff) -> Step<i64> {
        let args = json!({
            "request_id": issued.receipt.request_id, "expected_revision": revision,
            "generation": 1, "authorization_epoch": issued.authorization_epoch,
            "client_attempt_id": issued.attempt_id, "authority_id": issued.authority_id,
            "consumption_id": issued.consumption_id, "wire_id": issued.wire_id,
            "wire_digest": issued.wire_digest, "session_id": issued.session_id,
        });
        step(self.call_tool(VALIDATE_TOOL, args).await, |value| {
            wire::decode_validation(value, issued)
        })
    }
}
