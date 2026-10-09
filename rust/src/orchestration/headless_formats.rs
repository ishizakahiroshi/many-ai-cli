//! `internal/headless/{event,format_text,format_claude_stream_json}.go` at 21d0bc7.
//! Format dispatch, never provider dispatch. A malformed structured record is text.
use crate::proto::{
    decode_wire,
    wire::{
        Field, GoWire, Schema, decode_go_json_array, decode_go_json_members, decode_go_json_object,
        last_go_raw_field,
    },
};
use serde::{Deserialize, Deserializer};

pub const MAX_EVENT_TEXT_BYTES: usize = 8192;
pub const KNOWN_FORMATS: &[&str] = &["text", "claude-stream-json"];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Event {
    pub kind: String,
    pub tool: String,
    pub text: String,
    pub is_error: bool,
}
impl Event {
    pub fn new(kind: &str, text: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            text: text.into(),
            ..Self::default()
        }
    }
    pub fn line(&self) -> String {
        let text = sanitize_event_text(&self.text);
        let tag = match self.kind.as_str() {
            "output" => return text,
            "run.started" => "[run]",
            "assistant.message" => "[assistant]",
            "tool.started" => "[tool]",
            "tool.completed" if self.is_error => "[tool error]",
            "tool.completed" => "[tool result]",
            "run.completed" => "[result]",
            "run.failed" => "[error]",
            "stderr" => "[stderr]",
            _ => return join_tag(&format!("[{}]", sanitize_event_text(&self.kind)), &text),
        };
        if matches!(self.kind.as_str(), "tool.started" | "tool.completed") {
            return join_tag(
                tag,
                format!("{} {text}", sanitize_event_text(&self.tool)).trim(),
            );
        }
        join_tag(tag, &text)
    }
}
fn join_tag(tag: &str, text: &str) -> String {
    if text.is_empty() {
        tag.into()
    } else {
        format!("{tag} {text}")
    }
}
pub fn sanitize_event_text(text: &str) -> String {
    let text: String = text
        .chars()
        .filter_map(|c| match c {
            '\t' => Some(' '),
            '\u{0}'..='\u{1f}' | '\u{7f}' | '\u{fffd}' => None,
            _ => Some(c),
        })
        .collect();
    let mut text = text.trim_end_matches(' ').to_owned();
    if text.len() > MAX_EVENT_TEXT_BYTES {
        let mut end = MAX_EVENT_TEXT_BYTES;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str(" …(truncated)");
    }
    text
}
fn first_line(text: &str) -> &str {
    text.split('\n')
        .map(str::trim)
        .find(|s| !s.is_empty())
        .unwrap_or("")
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Parser {
    Text,
    ClaudeStreamJson,
}
pub fn parser_for(format: &str) -> Option<Parser> {
    match format {
        "text" => Some(Parser::Text),
        "claude-stream-json" => Some(Parser::ClaudeStreamJson),
        _ => None,
    }
}
impl Parser {
    pub fn parse(self, line: &[u8]) -> Vec<Event> {
        if self == Self::Text {
            return vec![Event::new("output", String::from_utf8_lossy(line))];
        }
        let text = String::from_utf8_lossy(line);
        let text = text.trim();
        if text.is_empty() {
            return vec![];
        }
        let parsed = match parse_stream(line) {
            Ok(parsed) => parsed,
            Err(_) => return vec![Event::new("output", text)],
        };
        match parsed.kind.as_str() {
            "system" if parsed.subtype == "init" => {
                let mut text = "started".to_owned();
                if !parsed.model.trim().is_empty() {
                    text.push_str(&format!(" model={}", parsed.model.trim()));
                }
                vec![Event::new("run.started", text)]
            }
            "assistant" => assistant_events(parsed.message),
            "user" => tool_results(parsed.message),
            "result" => {
                let failed =
                    parsed.is_error || (!parsed.subtype.is_empty() && parsed.subtype != "success");
                let mut event = Event::new(
                    if failed {
                        "run.failed"
                    } else {
                        "run.completed"
                    },
                    result_text(&parsed),
                );
                event.is_error = failed;
                vec![event]
            }
            _ => vec![],
        }
    }
}
fn null_default<'de, D: Deserializer<'de>, T: Deserialize<'de> + Default>(
    d: D,
) -> Result<T, D::Error> {
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct StreamLine {
    #[serde(rename = "type", deserialize_with = "null_default")]
    kind: String,
    #[serde(deserialize_with = "null_default")]
    subtype: String,
    #[serde(deserialize_with = "null_default")]
    is_error: bool,
    #[serde(deserialize_with = "null_default")]
    result: String,
    #[serde(deserialize_with = "null_default")]
    model: String,
    message: Option<Message>,
    error: Option<Vec<u8>>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Message {
    #[serde(rename = "role", deserialize_with = "null_default")]
    _role: String,
    #[serde(rename = "model", deserialize_with = "null_default")]
    _model: String,
    content: Option<Vec<u8>>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Block {
    #[serde(rename = "type", deserialize_with = "null_default")]
    kind: String,
    #[serde(deserialize_with = "null_default")]
    text: String,
    #[serde(deserialize_with = "null_default")]
    name: String,
    input: Option<Vec<u8>>,
    content: Option<Vec<u8>>,
    #[serde(deserialize_with = "null_default")]
    is_error: bool,
}
// Scalar/message metadata is decoded through the shared Go boundary; raw content
// is preserved separately so duplicate pointer-object merges and invalid UTF-8
// keep the Go behavior without eagerly decoding unrelated tool payloads.
impl GoWire for StreamLine {
    const GO_TYPE: &'static str = "HeadlessStreamLine";
    const SCHEMAS: &'static [Schema] = &[
        Schema {
            name: "HeadlessStreamLine",
            fields: &[
                Field {
                    name: "type",
                    kind: "string",
                },
                Field {
                    name: "subtype",
                    kind: "string",
                },
                Field {
                    name: "is_error",
                    kind: "bool",
                },
                Field {
                    name: "result",
                    kind: "string",
                },
                Field {
                    name: "model",
                    kind: "string",
                },
                Field {
                    name: "message",
                    kind: "*HeadlessMessage",
                },
            ],
        },
        Schema {
            name: "HeadlessMessage",
            fields: &[
                Field {
                    name: "role",
                    kind: "string",
                },
                Field {
                    name: "model",
                    kind: "string",
                },
            ],
        },
    ];
}
impl GoWire for Block {
    const GO_TYPE: &'static str = "HeadlessBlock";
    const SCHEMAS: &'static [Schema] = &[Schema {
        name: "HeadlessBlock",
        fields: &[
            Field {
                name: "type",
                kind: "string",
            },
            Field {
                name: "text",
                kind: "string",
            },
            Field {
                name: "name",
                kind: "string",
            },
            Field {
                name: "is_error",
                kind: "bool",
            },
        ],
    }];
}
#[derive(Deserialize)]
#[serde(transparent)]
struct GoString(String);
impl GoWire for GoString {
    const GO_TYPE: &'static str = "string";
}
fn string(raw: &[u8]) -> Result<String, serde_json::Error> {
    decode_wire::<GoString>(raw).map(|s| s.0)
}
fn parse_stream(raw: &[u8]) -> Result<StreamLine, serde_json::Error> {
    let mut parsed = decode_wire::<StreamLine>(raw)?;
    let members = decode_go_json_members(raw)?.unwrap_or_default();
    parsed.error = last_go_raw_field(&members, "error").map(<[u8]>::to_vec);
    let mut content = None;
    for member in &members {
        if let Some(raw) = last_go_raw_field(std::slice::from_ref(member), "message") {
            if let Some(message) = decode_go_json_members(raw)? {
                if let Some(raw) = last_go_raw_field(&message, "content") {
                    content = Some(raw.to_vec());
                }
            } else {
                content = None;
            }
        }
    }
    if let Some(message) = &mut parsed.message {
        message.content = content;
    }
    Ok(parsed)
}
fn parse_blocks(raw: &[u8]) -> Result<Vec<Block>, serde_json::Error> {
    decode_go_json_array(raw)?
        .unwrap_or_default()
        .into_iter()
        .map(|raw| {
            let mut block = decode_wire::<Block>(&raw)?;
            let members = decode_go_json_members(&raw)?.unwrap_or_default();
            block.input = last_go_raw_field(&members, "input").map(<[u8]>::to_vec);
            block.content = last_go_raw_field(&members, "content").map(<[u8]>::to_vec);
            Ok(block)
        })
        .collect()
}
fn content(raw: Option<&[u8]>) -> (Vec<Block>, String) {
    let Some(raw) = raw else {
        return (vec![], String::new());
    };
    if let Ok(blocks) = parse_blocks(raw) {
        let text = blocks
            .iter()
            .filter(|b| b.kind == "text")
            .map(|b| b.text.trim())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        return (blocks, text);
    }
    (
        vec![],
        string(raw).map(|s| s.trim().into()).unwrap_or_default(),
    )
}
fn assistant_events(message: Option<Message>) -> Vec<Event> {
    let Some(message) = message else {
        return vec![];
    };
    let (blocks, text) = content(message.content.as_deref());
    if blocks.is_empty() {
        return if text.is_empty() {
            vec![]
        } else {
            vec![Event::new("assistant.message", text)]
        };
    }
    blocks
        .into_iter()
        .filter_map(|b| match b.kind.as_str() {
            "text" if !b.text.trim().is_empty() => {
                Some(Event::new("assistant.message", b.text.trim()))
            }
            "tool_use" => Some(Event {
                kind: "tool.started".into(),
                tool: b.name,
                text: tool_summary(b.input.as_deref()),
                is_error: false,
            }),
            _ => None,
        })
        .collect()
}
fn tool_results(message: Option<Message>) -> Vec<Event> {
    let Some(message) = message else {
        return vec![];
    };
    content(message.content.as_deref())
        .0
        .into_iter()
        .filter(|b| b.kind == "tool_result")
        .map(|b| {
            let text = content(b.content.as_deref()).1;
            Event {
                kind: "tool.completed".into(),
                text: first_line(&text).into(),
                is_error: b.is_error,
                ..Event::default()
            }
        })
        .collect()
}
/// Go's map[string]any path rejects an overflowing JSON number even in a
/// non-selected field. Validate numeric tokens without decoding unrelated bodies.
fn finite_numbers(raw: &[u8]) -> bool {
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'"' {
            i += 1;
            while i < raw.len() {
                if raw[i] == b'\\' {
                    i += 2;
                } else if raw[i] == b'"' {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
        } else if raw[i] == b'-' || raw[i].is_ascii_digit() {
            let start = i;
            i += 1;
            while i < raw.len()
                && !matches!(raw[i], b',' | b']' | b'}' | b' ' | b'\n' | b'\r' | b'\t')
            {
                i += 1;
            }
            if std::str::from_utf8(&raw[start..i])
                .ok()
                .and_then(|s| s.parse::<f64>().ok())
                .is_none_or(|n| !n.is_finite())
            {
                return false;
            }
        } else {
            i += 1;
        }
    }
    true
}
fn tool_summary(raw: Option<&[u8]>) -> String {
    let Some(raw) = raw else {
        return String::new();
    };
    let Ok(Some(input)) = decode_go_json_object(raw) else {
        return String::new();
    };
    if !finite_numbers(raw) {
        return String::new();
    }
    for key in [
        "command",
        "file_path",
        "path",
        "pattern",
        "url",
        "query",
        "description",
    ] {
        if let Some(raw) = input.get(key)
            && let Ok(value) = string(raw)
        {
            let summary = first_line(&value);
            if !summary.is_empty() {
                return summary.into();
            }
        }
    }
    String::new()
}
fn result_text(parsed: &StreamLine) -> String {
    let mut parts: Vec<String> = vec![];
    if !parsed.subtype.trim().is_empty() {
        parts.push(parsed.subtype.trim().into());
    }
    if !first_line(&parsed.result).is_empty() {
        parts.push(first_line(&parsed.result).into());
    } else if let Some(error) = &parsed.error {
        if let Ok(text) = string(error) {
            if !first_line(&text).is_empty() {
                parts.push(first_line(&text).into());
            }
        } else if let Ok(Some(object)) = decode_go_json_object(error)
            && finite_numbers(error)
            && let Some(message) = object.get("message")
            && message.first() == Some(&b'"')
            && let Ok(text) = string(message)
        {
            parts.push(first_line(&text).into());
        }
    }
    if parts.is_empty() {
        format!("is_error={}", parsed.is_error)
    } else {
        parts.join(": ")
    }
}
