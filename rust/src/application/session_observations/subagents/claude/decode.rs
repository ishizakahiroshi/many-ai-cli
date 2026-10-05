use super::*;
macro_rules! fields {($($name:literal => $kind:literal),* $(,)?) => {&[$(Field{name:$name,kind:$kind}),*]};}
const SCHEMAS: &[Schema] = &[
    Schema {
        name: "Meta",
        fields: fields!("agentType"=>"string","description"=>"string","toolUseId"=>"string","spawnDepth"=>"int","parentAgentId"=>"string","model"=>"string"),
    },
    Schema {
        name: "Input",
        fields: fields!("command"=>"string","pattern"=>"string","path"=>"string","file_path"=>"string"),
    },
    Schema {
        name: "ToolResult",
        fields: fields!("isAsync"=>"bool","status"=>"string"),
    },
    Schema {
        name: "Transcript",
        fields: fields!("type"=>"string","sessionId"=>"string","timestamp"=>"string","isSidechain"=>"bool","message"=>"Message"),
    },
    Schema {
        name: "Message",
        fields: fields!("role"=>"string"),
    },
    Schema {
        name: "Result",
        fields: fields!("timestamp"=>"string","message"=>"RawMessageObject"),
    },
    Schema {
        name: "RawMessageObject",
        fields: &[],
    },
    Schema {
        name: "Sniff",
        fields: fields!("type"=>"string"),
    },
    Schema {
        name: "Queue",
        fields: fields!("type"=>"string","operation"=>"string","timestamp"=>"string","content"=>"string"),
    },
    Schema {
        name: "Block",
        fields: fields!("type"=>"string","text"=>"string","thinking"=>"string","id"=>"string","name"=>"string","tool_use_id"=>"string"),
    },
];
impl GoWire for Meta {
    const GO_TYPE: &'static str = "Meta";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
impl GoWire for Input {
    const GO_TYPE: &'static str = "Input";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
impl GoWire for ToolResult {
    const GO_TYPE: &'static str = "ToolResult";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
pub(super) fn decode_schema<T: serde::de::DeserializeOwned>(
    raw: &[u8],
    kind: &str,
) -> Result<T, serde_json::Error> {
    // Require the whole JSON value, unlike an HTTP first-value decoder.
    wire::decode_go_json_members(raw)?;
    wire::decode_http_schema(raw, kind, SCHEMAS)
}
pub(super) fn raw_field(raw: &[u8], field: &str) -> Option<Box<RawValue>> {
    let fields = wire::decode_go_json_members(raw).ok().flatten()?;
    serde_json::from_slice(wire::last_go_raw_field(&fields, field)?).ok()
}
fn nested_content(raw: &[u8]) -> Option<Box<RawValue>> {
    let fields = wire::decode_go_json_members(raw).ok().flatten()?;
    let mut content = None;
    for (name, value) in fields {
        if name.eq_ignore_ascii_case("message")
            && value != b"null"
            && let Some(value) = raw_field(&value, "content")
        {
            content = Some(value)
        }
    }
    content
}
pub(super) fn decode_record(raw: &[u8], kind: &str) -> Result<Record, serde_json::Error> {
    let kind = if kind == "Parent" {
        let sniff: Record = decode_schema(raw, "Sniff")?;
        match sniff.kind.as_str() {
            "assistant" => "Transcript",
            "user" => "Result",
            "queue-operation" => "Queue",
            _ => return Ok(sniff),
        }
    } else {
        kind
    };
    // Queue's content is validated as a Go string independently, then retained
    // as RawMessage for the source notification parser only.
    let mut record: Record = if kind == "Queue" {
        let decoded: serde_json::Value = decode_schema(raw, kind)?;
        let mut decoded = decoded.as_object().cloned().unwrap_or_default();
        decoded.remove("content");
        serde_json::from_value(serde_json::Value::Object(decoded))?
    } else {
        decode_schema(raw, kind)?
    };
    if kind == "Result" {
        record.kind = "user".into();
        record.tool_use_result = raw_field(raw, "toolUseResult")
    }
    record.message.content = nested_content(raw);
    if kind == "Queue" {
        record.content = raw_field(raw, "content")
    }
    Ok(record)
}
