//! A scripted ACP agent for tests. It speaks the same framing as the real
//! protocol (newline-delimited JSON-RPC) over in-memory channels, so every
//! test runs without the real agent and without signing in.
//!
//! The prompt text picks the behaviour:
//! - `plain`: two message chunks, then `end_turn`
//! - `permission`: a tool call, then `session/request_permission` (id 900),
//!   and the turn continues according to the answer
//! - `hang`: no answer until `session/cancel` arrives
//! - `garbage`: one line that is not JSON, then a chunk and `end_turn`
//! - `crash`: one update, then the stream ends abruptly

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::client::{AcpClient, AcpEvents, Incoming};
use super::framing::{LineDecoder, encode_message};

#[derive(Clone, Default)]
pub struct MockConfig {
    /// Deliver every message in two pieces, cut in the middle.
    pub fragment: bool,
    /// `session/new` fails with "Authentication required".
    pub auth_required: bool,
    /// Advertise no optional session capability.
    pub bare_capabilities: bool,
    /// Advertise `session/load` but not `session/resume`.
    pub no_resume: bool,
}

#[derive(Clone, Default)]
pub struct MockHandle {
    /// Every message the client wrote, in order.
    pub received: Arc<Mutex<Vec<Value>>>,
}

impl MockHandle {
    pub fn methods(&self) -> Vec<String> {
        self.received
            .lock()
            .unwrap()
            .iter()
            .filter_map(|m| m.get("method").and_then(Value::as_str).map(str::to_owned))
            .collect()
    }
    pub fn find(&self, predicate: impl Fn(&Value) -> bool) -> Option<Value> {
        self.received
            .lock()
            .unwrap()
            .iter()
            .find(|m| predicate(m))
            .cloned()
    }
}

enum Waiting {
    Permission { prompt_id: Value },
    Cancel { prompt_id: Value },
}

pub fn connect(config: MockConfig) -> (AcpClient, AcpEvents, MockHandle) {
    let (out_tx, mut out_rx) = mpsc::channel::<Vec<u8>>(64);
    let (in_tx, in_rx) = mpsc::channel::<Incoming>(256);
    let handle = MockHandle::default();
    let received = handle.received.clone();
    tokio::spawn(async move {
        let mut decoder = LineDecoder::default();
        let mut waiting: Option<Waiting> = None;
        let say = |value: Value| {
            let in_tx = in_tx.clone();
            let fragment = config.fragment;
            async move {
                let bytes = encode_message(&value);
                if fragment {
                    let cut = bytes.len() / 2;
                    let _ = in_tx.send(Incoming::Bytes(bytes[..cut].to_vec())).await;
                    let _ = in_tx.send(Incoming::Bytes(bytes[cut..].to_vec())).await;
                } else {
                    let _ = in_tx.send(Incoming::Bytes(bytes)).await;
                }
            }
        };
        let update = |session: &str, body: Value| json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":session,"update":body}});
        let ok = |id: Value, result: Value| json!({"jsonrpc":"2.0","id":id,"result":result});
        let err = |id: Value, code: i64, message: &str| json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}});
        while let Some(bytes) = out_rx.recv().await {
            let lines = decoder.push(&bytes).expect("client framing");
            for line in lines {
                let message: Value = serde_json::from_slice(&line).expect("client sent JSON");
                received.lock().unwrap().push(message.clone());
                let method = message
                    .get("method")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let id = message.get("id").cloned();
                let params = message.get("params").cloned().unwrap_or(Value::Null);
                match (method.as_deref(), id) {
                    (Some("initialize"), Some(id)) => {
                        let mut caps = json!({
                            "loadSession": true,
                            "promptCapabilities": {"image": true, "embeddedContext": true},
                            "sessionCapabilities": {"list": {}, "resume": {}, "close": {}},
                        });
                        if config.no_resume {
                            caps["sessionCapabilities"] = json!({"list": {}, "close": {}});
                        }
                        if config.bare_capabilities {
                            caps = json!({"promptCapabilities": {}});
                        }
                        say(ok(
                            id,
                            json!({
                                "protocolVersion": 1,
                                "agentCapabilities": caps,
                                "agentInfo": {"name": "mock-agent", "version": "0.0.1"},
                                "authMethods": [
                                    {"id": "mock-login", "name": "Mock login"},
                                    {"id": "mock-terminal", "name": "Terminal", "type": "terminal"},
                                ],
                            }),
                        ))
                        .await;
                    }
                    (Some("session/new"), Some(id)) => {
                        if config.auth_required {
                            say(err(id, -32000, "Authentication required")).await;
                        } else {
                            say(ok(id, json!({"sessionId": "sess_mock_1"}))).await;
                        }
                    }
                    (Some("session/prompt"), Some(id)) => {
                        let session = params["sessionId"].as_str().unwrap_or_default().to_owned();
                        let text = params["prompt"][0]["text"]
                            .as_str()
                            .unwrap_or_default()
                            .to_owned();
                        match text.as_str() {
                            "plain" => {
                                for part in ["Hello ", "world"] {
                                    say(update(
                                        &session,
                                        json!({
                                            "sessionUpdate": "agent_message_chunk",
                                            "content": {"type": "text", "text": part},
                                        }),
                                    ))
                                    .await;
                                }
                                say(ok(id, json!({"stopReason": "end_turn"}))).await;
                            }
                            "permission" => {
                                say(update(
                                    &session,
                                    json!({
                                        "sessionUpdate": "tool_call",
                                        "toolCallId": "call_001",
                                        "title": "Run a command",
                                        "kind": "execute",
                                        "status": "pending",
                                    }),
                                ))
                                .await;
                                say(json!({
                                    "jsonrpc": "2.0",
                                    "id": 900,
                                    "method": "session/request_permission",
                                    "params": {
                                        "sessionId": session,
                                        "toolCall": {"toolCallId": "call_001"},
                                        "options": [
                                            {"optionId": "allow-once", "name": "Allow once", "kind": "allow_once"},
                                            {"optionId": "reject-once", "name": "Reject", "kind": "reject_once"},
                                        ],
                                    },
                                }))
                                .await;
                                waiting = Some(Waiting::Permission { prompt_id: id });
                            }
                            "hang" => waiting = Some(Waiting::Cancel { prompt_id: id }),
                            "garbage" => {
                                let _ = in_tx
                                    .send(Incoming::Bytes(b"this is not json\n".to_vec()))
                                    .await;
                                say(update(
                                    &session,
                                    json!({
                                        "sessionUpdate": "agent_message_chunk",
                                        "content": {"type": "text", "text": "after garbage"},
                                    }),
                                ))
                                .await;
                                say(ok(id, json!({"stopReason": "end_turn"}))).await;
                            }
                            "crash" => {
                                say(update(
                                    &session,
                                    json!({
                                        "sessionUpdate": "agent_message_chunk",
                                        "content": {"type": "text", "text": "partial"},
                                    }),
                                ))
                                .await;
                                let _ = in_tx
                                    .send(Incoming::Closed("mock agent crashed".into()))
                                    .await;
                                return;
                            }
                            _ => say(err(id, -32602, "unknown mock prompt")).await,
                        }
                    }
                    (Some("session/list"), Some(id)) => {
                        say(ok(id, json!({
                            "sessions": [
                                {"sessionId": "sess_a", "cwd": "/work/a", "title": "First", "updatedAt": "2026-01-01T00:00:00Z"},
                                {"sessionId": "sess_b", "cwd": "/work/b"},
                            ],
                            "nextCursor": "page2",
                        })))
                        .await;
                    }
                    (Some("session/load"), Some(id)) => {
                        let session = params["sessionId"].as_str().unwrap_or_default().to_owned();
                        say(update(
                            &session,
                            json!({
                                "sessionUpdate": "user_message_chunk",
                                "content": {"type": "text", "text": "history"},
                            }),
                        ))
                        .await;
                        say(ok(id, json!({}))).await;
                    }
                    (Some("session/resume"), Some(id)) => say(ok(id, json!({}))).await,
                    (Some("session/close"), Some(id)) => {
                        if params["sessionId"] == "sess_gone" {
                            say(err(id, -32002, "unknown session")).await;
                        } else {
                            say(ok(id, json!({}))).await;
                        }
                    }
                    (Some("session/cancel"), None) => {
                        let prompt_id = match waiting.take() {
                            Some(Waiting::Cancel { prompt_id })
                            | Some(Waiting::Permission { prompt_id }) => prompt_id,
                            None => continue,
                        };
                        say(ok(prompt_id, json!({"stopReason": "cancelled"}))).await;
                    }
                    (Some(_), Some(id)) => say(err(id, -32601, "method not found")).await,
                    (None, Some(id)) if id == 900 => {
                        let Some(Waiting::Permission { prompt_id }) = waiting.take() else {
                            continue;
                        };
                        let outcome = &message["result"]["outcome"];
                        let stop = match (outcome["outcome"].as_str(), outcome["optionId"].as_str())
                        {
                            (Some("selected"), Some("allow-once")) => {
                                say(update(
                                    "sess_mock_1",
                                    json!({
                                        "sessionUpdate": "tool_call_update",
                                        "toolCallId": "call_001",
                                        "status": "completed",
                                    }),
                                ))
                                .await;
                                "end_turn"
                            }
                            (Some("selected"), _) => "end_turn",
                            _ => "cancelled",
                        };
                        say(ok(prompt_id, json!({"stopReason": stop}))).await;
                    }
                    _ => {}
                }
            }
        }
    });
    let (client, events) = AcpClient::from_transport(out_tx, in_rx, None);
    (client, events, handle)
}
