// MCP framing shared from the MIT-licensed Many Iris local candidate b54249c.
// Domain tools remain outside this module.
//! MCP 2026-07-28, without implicit connection-scoped authorization.
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value, json};
use std::{collections::HashSet, fmt};

pub const VERSION: &str = "2026-07-28";
pub const MAX_BYTES: usize = 65_536;
pub const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolError {
    pub code: i64,
    pub message: String,
}
impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}
impl std::error::Error for ProtocolError {}
fn error(code: i64, message: &str) -> ProtocolError {
    ProtocolError {
        code,
        message: message.into(),
    }
}

struct StrictValue(usize);
impl<'de> DeserializeSeed<'de> for StrictValue {
    type Value = Value;
    fn deserialize<D: de::Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
        if self.0 > MAX_DEPTH {
            return Err(de::Error::custom("JSON nesting limit exceeded"));
        }
        d.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for StrictValue {
    type Value = Value;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("bounded JSON with unique object keys")
    }
    fn visit_bool<E: de::Error>(self, v: bool) -> Result<Value, E> {
        Ok(Value::Bool(v))
    }
    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Value, E> {
        Ok(v.into())
    }
    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Value, E> {
        Ok(v.into())
    }
    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Value, E> {
        Number::from_f64(v)
            .map(Value::Number)
            .ok_or_else(|| E::custom("non-finite number"))
    }
    fn visit_str<E: de::Error>(self, v: &str) -> Result<Value, E> {
        Ok(v.into())
    }
    fn visit_string<E: de::Error>(self, v: String) -> Result<Value, E> {
        Ok(Value::String(v))
    }
    fn visit_unit<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_none<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut values = Vec::new();
        while let Some(v) = seq.next_element_seed(StrictValue(self.0 + 1))? {
            values.push(v);
        }
        Ok(Value::Array(values))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut values = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(de::Error::custom("duplicate JSON object key"));
            }
            let value = map.next_value_seed(StrictValue(self.0 + 1))?;
            values.insert(key, value);
        }
        Ok(Value::Object(values))
    }
}

pub fn parse_json(bytes: &[u8]) -> Result<Value, ProtocolError> {
    if bytes.len() > MAX_BYTES {
        return Err(error(-32700, "JSON byte limit exceeded"));
    }
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let value = StrictValue(0)
        .deserialize(&mut decoder)
        .map_err(|_| error(-32700, "Invalid bounded JSON"))?;
    decoder
        .end()
        .map_err(|_| error(-32700, "Trailing JSON data"))?;
    Ok(value)
}

fn valid_id(v: &Value) -> bool {
    v.is_string() || v.is_i64() || v.is_u64()
}
pub fn validate_request(request: &Value) -> Result<(), ProtocolError> {
    if !request.is_object()
        || request["jsonrpc"] != "2.0"
        || !valid_id(&request["id"])
        || !request["method"].is_string()
    {
        return Err(error(-32600, "Invalid JSON-RPC request"));
    }
    if request.get("result").is_some() || request.get("error").is_some() {
        return Err(error(-32600, "Not a client request"));
    }
    let params = request
        .get("params")
        .and_then(Value::as_object)
        .ok_or_else(|| error(-32602, "Object params required"))?;
    let meta = params
        .get("_meta")
        .and_then(Value::as_object)
        .ok_or_else(|| error(-32602, "Per-request metadata required"))?;
    let version = meta
        .get("io.modelcontextprotocol/protocolVersion")
        .and_then(Value::as_str)
        .ok_or_else(|| error(-32602, "Protocol version metadata required"))?;
    if version != VERSION {
        return Err(error(-32022, "Unsupported protocol version"));
    }
    if let Some(info) = meta.get("io.modelcontextprotocol/clientInfo")
        && (!info.is_object()
            || !info.get("name").is_some_and(Value::is_string)
            || !info.get("version").is_some_and(Value::is_string))
    {
        return Err(error(
            -32602,
            "Client name and version required when clientInfo is supplied",
        ));
    }
    if !meta
        .get("io.modelcontextprotocol/clientCapabilities")
        .is_some_and(Value::is_object)
    {
        return Err(error(-32602, "Client capability metadata required"));
    }
    Ok(())
}

fn header<'a>(headers: &'a [(&str, &str)], name: &str) -> Result<Option<&'a str>, ProtocolError> {
    let values: Vec<_> = headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| *v)
        .collect();
    if values.len() > 1 {
        return Err(error(-32020, "Duplicate MCP HTTP header"));
    }
    Ok(values.first().copied())
}
fn decode_header(value: &str) -> Result<String, ProtocolError> {
    if let Some(encoded) = value
        .strip_prefix("=?base64?")
        .and_then(|v| v.strip_suffix("?="))
    {
        let bytes = STANDARD
            .decode(encoded)
            .map_err(|_| error(-32020, "Invalid MCP header encoding"))?;
        if STANDARD.encode(&bytes) != encoded {
            return Err(error(-32020, "Noncanonical MCP header encoding"));
        }
        return String::from_utf8(bytes).map_err(|_| error(-32020, "MCP header is not UTF-8"));
    }
    if value.trim() != value || !value.bytes().all(|v| (0x20..=0x7e).contains(&v)) {
        return Err(error(-32020, "Unsafe unencoded MCP header value"));
    }
    Ok(value.into())
}

/// Validates HTTP envelope mirrors. The HTTP owner must separately validate
/// configured Host/Origin, bearer identity, method, body limit and endpoint.
pub fn validate_http(headers: &[(&str, &str)], request: &Value) -> Result<(), ProtocolError> {
    let version = header(headers, "MCP-Protocol-Version")?
        .ok_or_else(|| error(-32020, "Protocol version header required"))?;
    let body_version = request["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"]
        .as_str()
        .unwrap_or("");
    if version != body_version {
        return Err(error(-32020, "Protocol version header mismatch"));
    }
    let method =
        header(headers, "Mcp-Method")?.ok_or_else(|| error(-32020, "Method header required"))?;
    if decode_header(method)? != request["method"].as_str().unwrap_or("") {
        return Err(error(-32020, "Method header mismatch"));
    }
    let method_name = request["method"].as_str().unwrap_or("");
    let name_field = match method_name {
        "tools/call" | "prompts/get" => Some("name"),
        "resources/read" => Some("uri"),
        _ => None,
    };
    if let Some(field) = name_field {
        let name =
            header(headers, "Mcp-Name")?.ok_or_else(|| error(-32020, "Name header required"))?;
        if decode_header(name)? != request["params"][field].as_str().unwrap_or("") {
            return Err(error(-32020, "Name header mismatch"));
        }
    }
    // No published tool uses x-mcp-header. Unrecognized extension headers do
    // not establish a routing decision or override any body argument.
    let accept =
        header(headers, "Accept")?.ok_or_else(|| error(-32600, "Accept header required"))?;
    let media: HashSet<_> = accept
        .split(',')
        .map(|s| s.trim().split(';').next().unwrap_or(""))
        .collect();
    if !media.contains("application/json") || !media.contains("text/event-stream") {
        return Err(error(-32600, "Both MCP response media types required"));
    }
    let content_type =
        header(headers, "Content-Type")?.ok_or_else(|| error(-32600, "Content type required"))?;
    if !content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .eq_ignore_ascii_case("application/json")
    {
        return Err(error(-32600, "JSON request content type required"));
    }
    validate_request(request)
}

pub fn request(id: u64, method: &str, mut params: Value) -> Value {
    params["_meta"] = json!({"io.modelcontextprotocol/protocolVersion":VERSION,"io.modelcontextprotocol/clientInfo":{"name":"iris-synthetic-client","version":"0.1"},"io.modelcontextprotocol/clientCapabilities":{}});
    json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
}
pub fn error_response(id: Option<&Value>, e: &ProtocolError) -> Value {
    let mut response = json!({"jsonrpc":"2.0","error":{"code":e.code,"message":e.message}});
    if let Some(id) = id.filter(|id| valid_id(id)) {
        response["id"] = id.clone();
    }
    if e.code == -32022 {
        response["error"]["data"] = json!({"supported":[VERSION]});
    }
    response
}
pub fn tool(name: &str, properties: Value, required: &[&str]) -> Value {
    json!({"name":name,"description":format!("Synthetic Iris operation {name}"),"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false}})
}
pub type ToolHandler<'a> = dyn FnMut(&str, &Value) -> Result<Value, (String, String)> + 'a;

pub fn handle_with_tools(
    request: &Value,
    server_name: &str,
    tools: &[Value],
    context: &mut ToolHandler<'_>,
) -> Value {
    if request.is_object()
        && request["jsonrpc"] == "2.0"
        && request.get("id").is_none()
        && request["method"]
            .as_str()
            .is_some_and(|m| m.starts_with("notifications/"))
    {
        // Callers must omit a wire response for this sentinel. No notification
        // is allowed to invoke an application mutation through this function.
        return Value::Null;
    }
    if let Err(e) = validate_request(request) {
        return error_response(request.get("id"), &e);
    }
    let id = &request["id"];
    let result = match request["method"].as_str().unwrap_or("") {
        "server/discover" => {
            json!({"resultType":"complete","supportedVersions":[VERSION],"capabilities":{"tools":{}},"_meta":{"io.modelcontextprotocol/serverInfo":{"name":server_name,"version":"0.1.0-dev.1"}}})
        }
        "tools/list" => {
            if request["params"].get("cursor").is_some() {
                return error_response(
                    Some(id),
                    &error(-32602, "Pagination cursor is not supported"),
                );
            }
            json!({"resultType":"complete","tools":tools})
        }
        "tools/call" => {
            let name = match request["params"]["name"].as_str() {
                Some(v) => v,
                None => return error_response(Some(id), &error(-32602, "Tool name required")),
            };
            if name == "iris_submit_decision" || name == "submit_decision" {
                return tool_response(
                    id,
                    Err((
                        "HUMAN_DECISION_REQUIRED".into(),
                        "Use the human decision UI".into(),
                    )),
                );
            }
            if name.is_empty()
                || name.len() > 128
                || !name
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
                || !tools.iter().any(|t| t["name"] == name)
            {
                return error_response(Some(id), &error(-32602, "Unknown or invalid tool"));
            }
            let args = request["params"]
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            if !args.is_object() {
                return error_response(
                    Some(id),
                    &error(-32602, "Tool arguments must be an object"),
                );
            }
            let definition = tools.iter().find(|t| t["name"] == name).unwrap();
            if let Some(fields) = definition["inputSchema"]["required"].as_array()
                && fields
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|field| args.get(field).is_none())
            {
                return error_response(Some(id), &error(-32602, "Required tool argument missing"));
            }
            let props = &definition["inputSchema"]["properties"];
            if args.as_object().unwrap().iter().any(|(k, v)| {
                props.get(k).is_none()
                    || match props[k]["type"].as_str() {
                        Some("string") => v.as_str().is_none_or(|s| s.is_empty()),
                        Some("integer") => !v.is_u64(),
                        Some("object") => !v.is_object(),
                        _ => false,
                    }
            }) {
                return error_response(Some(id), &error(-32602, "Invalid tool argument schema"));
            }
            return tool_response(
                id,
                context(name.strip_prefix("iris_").unwrap_or(name), &args),
            );
        }
        "ping" => json!({"resultType":"complete"}),
        _ => return error_response(Some(id), &error(-32601, "Method not found")),
    };
    json!({"jsonrpc":"2.0","id":id,"result":result})
}
fn tool_response(id: &Value, result: Result<Value, (String, String)>) -> Value {
    let (data, is_error) = match result {
        Ok(v) => (v, false),
        Err((code, detail)) => (
            json!({"ok":false,"error":{"code":code,"detail":detail}}),
            true,
        ),
    };
    json!({"jsonrpc":"2.0","id":id,"result":{"resultType":"complete","content":[{"type":"text","text":data.to_string()}],"structuredContent":data,"isError":is_error}})
}
