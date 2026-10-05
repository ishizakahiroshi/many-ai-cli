//! Wire-facing JSON decoding with Go encoding/json object merge semantics.
//! Raw Serde DTO decoding is for canonical internal values only; HTTP/WS callers
//! must use this boundary for case-folded fields, duplicate keys and null defaults.
use serde::{
    Deserialize, Deserializer,
    de::{DeserializeOwned, MapAccess, Visitor},
};
use serde_json::{Value, value::RawValue};
use std::collections::BTreeMap;
use std::fmt;

pub trait GoWire: DeserializeOwned {
    const GO_TYPE: &'static str;
    /// Additional service schemas. Shared protocol schemas remain available.
    const SCHEMAS: &'static [Schema] = &[];
}
pub struct Field {
    pub name: &'static str,
    pub kind: &'static str,
}
pub struct Schema {
    pub name: &'static str,
    pub fields: &'static [Field],
}
struct Object(Vec<(String, Box<RawValue>)>);
impl<'de> Deserialize<'de> for Object {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ObjectVisitor;
        impl<'de> Visitor<'de> for ObjectVisitor {
            type Value = Object;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("JSON object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Object, A::Error> {
                let mut out = vec![];
                while let Some(pair) = map.next_entry()? {
                    out.push(pair);
                }
                Ok(Object(out))
            }
        }
        deserializer.deserialize_map(ObjectVisitor)
    }
}
fn error(message: &str) -> serde_json::Error {
    <serde_json::Error as serde::de::Error>::custom(message)
}
fn fold(name: &str) -> String {
    // Go foldName uses Unicode simple-fold equivalence. These are the only
    // non-ASCII runes whose class reaches the ASCII JSON field alphabet.
    name.chars()
        .map(|c| match c {
            '\u{017f}' => 's',
            '\u{212a}' => 'k',
            c => c.to_ascii_lowercase(),
        })
        .collect()
}
fn schema(kind: &str, extra: &'static [Schema]) -> Option<&'static Schema> {
    extra
        .iter()
        .chain(super::generated::WIRE_SCHEMA.iter())
        .find(|s| s.name == kind)
}
// Keep slice backing elements while repeated fields are decoded. Go reuses
// them even when an intermediate occurrence shortens the visible slice.
enum Decoded {
    Leaf(Value),
    Array { values: Vec<Decoded>, len: usize },
    Object(BTreeMap<String, Decoded>),
}
impl Decoded {
    fn finish(self) -> Value {
        match self {
            Self::Leaf(v) => v,
            Self::Array { values, len } => {
                Value::Array(values.into_iter().take(len).map(Self::finish).collect())
            }
            Self::Object(values) => {
                Value::Object(values.into_iter().map(|(k, v)| (k, v.finish())).collect())
            }
        }
    }
    fn object(self) -> Option<BTreeMap<String, Decoded>> {
        if let Self::Object(v) = self {
            Some(v)
        } else {
            None
        }
    }
}
fn merge(
    kind: &str,
    old: Option<Decoded>,
    raw: &RawValue,
    extra: &'static [Schema],
) -> Result<Decoded, serde_json::Error> {
    let source = raw.get();
    // RawMessage copies this occurrence's JSON bytes rather than decoding its
    // contents. The internal DTO projection is a string containing those bytes;
    // a later typed decoder must still see duplicate keys and invalid earlier
    // occurrences. JSON null is the four raw bytes "null", not a nil pointer.
    if kind == "json.RawMessage" {
        return Ok(Decoded::Leaf(Value::String(source.into())));
    }
    if let Some(inner) = kind.strip_prefix('*') {
        if source == "null" {
            return Ok(Decoded::Leaf(Value::Null));
        }
        return merge(inner, old, raw, extra);
    }
    if let Some(inner) = kind.strip_prefix("[]") {
        if source == "null" {
            return Ok(Decoded::Leaf(Value::Null));
        }
        if inner == "byte" && source.starts_with('"') {
            #[derive(Deserialize)]
            struct Bytes(#[serde(with = "super::base64_bytes")] Vec<u8>);
            // Validate every occurrence before a later duplicate can overwrite it.
            let Bytes(bytes) = serde_json::from_str(source)?;
            let len = bytes.len();
            return Ok(Decoded::Array {
                values: bytes.into_iter().map(|v| Decoded::Leaf(v.into())).collect(),
                len,
            });
        }
        let items: Vec<Box<RawValue>> = serde_json::from_str(source)?;
        let len = items.len();
        let mut values = match old {
            Some(Decoded::Array { values, .. }) if len > 0 => values,
            _ => Vec::new(),
        };
        for (i, item) in items.iter().enumerate() {
            let previous = if i < values.len() {
                Some(std::mem::replace(
                    &mut values[i],
                    Decoded::Leaf(Value::Null),
                ))
            } else {
                None
            };
            let next = merge(inner, previous, item, extra)?;
            if i < values.len() {
                values[i] = next;
            } else {
                values.push(next);
            }
        }
        return Ok(Decoded::Array { values, len });
    }
    if let Some(inner) = kind.strip_prefix("map[string]") {
        if source == "null" {
            return Ok(Decoded::Leaf(Value::Null));
        }
        let Object(fields) = serde_json::from_str(source)?;
        let mut map = old.and_then(Decoded::object).unwrap_or_default();
        for (key, v) in fields {
            map.insert(key, merge(inner, None, &v, extra)?);
        }
        return Ok(Decoded::Object(map));
    }
    if let Some(schema) = schema(kind, extra) {
        if source == "null" {
            return Ok(old.unwrap_or_else(|| Decoded::Object(BTreeMap::new())));
        }
        let Object(fields) = serde_json::from_str(source)?;
        let mut map = old.and_then(Decoded::object).unwrap_or_default();
        for (key, v) in fields {
            if let Some(field) = schema.fields.iter().find(|f| fold(f.name) == fold(&key)) {
                let previous = map.remove(field.name);
                map.insert(field.name.into(), merge(field.kind, previous, &v, extra)?);
            }
            // Unknown fields are syntax-checked raw values, not numbers or DTOs.
            // In particular an ignored 1e1000 must not overflow an f64 here.
        }
        return Ok(Decoded::Object(map));
    }
    if source == "null" {
        return Ok(old.unwrap_or_else(|| {
            Decoded::Leaf(match kind {
                "bool" => false.into(),
                "int" | "int64" | "uint64" | "byte" => 0.into(),
                "float64" => 0.0.into(),
                "time.Time" => "0001-01-01T00:00:00Z".into(),
                _ => "".into(),
            })
        }));
    }
    // Decode with the Go destination type before merging. An invalid earlier
    // duplicate remains an error even if the last occurrence would be valid.
    match kind {
        "bool" => serde_json::from_str::<bool>(source).map(Value::from),
        "int" | "int64" => source
            .parse::<i64>()
            .map(Value::from)
            .map_err(|_| error("invalid Go int64")),
        "uint64" => source
            .parse::<u64>()
            .map(Value::from)
            .map_err(|_| error("invalid Go uint64")),
        "byte" => source
            .parse::<u8>()
            .map(Value::from)
            .map_err(|_| error("invalid Go byte")),
        "float64" => serde_json::from_str::<f64>(source).map(Value::from),
        "time.Time" => {
            let text = serde_json::from_str::<String>(source)?;
            super::time::parse_rfc3339(&text).map_err(|_| error("invalid Go time.Time"))?;
            Ok(Value::String(text))
        }
        "string" | "ApprovalRiskTier" => serde_json::from_str::<String>(source).map(Value::from),
        _ => Err(error("unsupported Go wire schema kind")),
    }
    .map(Decoded::Leaf)
}

fn hex_quad(bytes: &[u8]) -> Option<u16> {
    if bytes.len() < 6 || &bytes[..2] != b"\\u" {
        return None;
    }
    bytes[2..6].iter().try_fold(0u16, |v, b| {
        (*b as char).to_digit(16).map(|digit| v * 16 + digit as u16)
    })
}
fn append_go_utf8(out: &mut Vec<u8>, mut bytes: &[u8]) {
    // encoding/json replaces each invalid UTF-8 byte, not each invalid sequence.
    while !bytes.is_empty() {
        match std::str::from_utf8(bytes) {
            Ok(_) => {
                out.extend_from_slice(bytes);
                break;
            }
            Err(e) => {
                out.extend_from_slice(&bytes[..e.valid_up_to()]);
                out.extend_from_slice("\u{fffd}".as_bytes());
                bytes = &bytes[e.valid_up_to() + 1..];
            }
        }
    }
}
/// Go string-to-JSON text projection: each invalid UTF-8 byte becomes one
/// replacement rune, unlike Rust's grouping of some incomplete byte sequences.
pub fn go_utf8_lossy(bytes: &[u8]) -> String {
    let mut out = Vec::with_capacity(bytes.len());
    append_go_utf8(&mut out, bytes);
    String::from_utf8(out).expect("Go UTF-8 projection is valid")
}
fn go_unicode(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    let mut in_string = false;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                in_string = !in_string;
                out.push(b'"');
                i += 1;
            }
            b'\\' if in_string => {
                if let Some(unit) = hex_quad(&bytes[i..])
                    && (0xd800..=0xdfff).contains(&unit)
                {
                    if unit <= 0xdbff
                        && hex_quad(&bytes[i + 6..]).is_some_and(|v| (0xdc00..=0xdfff).contains(&v))
                    {
                        out.extend_from_slice(&bytes[i..i + 12]);
                        i += 12;
                    } else {
                        out.extend_from_slice(b"\\ufffd");
                        i += 6;
                    }
                } else {
                    // Leave malformed escapes to JSON syntax validation; consume
                    // the escaped character so an escaped quote does not toggle.
                    let end = (i + 2).min(bytes.len());
                    out.extend_from_slice(&bytes[i..end]);
                    i = end;
                }
            }
            _ if in_string => {
                let start = i;
                while i < bytes.len() && !matches!(bytes[i], b'"' | b'\\') {
                    i += 1;
                }
                append_go_utf8(&mut out, &bytes[start..i]);
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    out
}
fn check_depth(bytes: &[u8]) -> Result<(), serde_json::Error> {
    // encoding/json's scanner rejects the 10,001st opening array/object, even
    // inside an ignored field. RawValue validates syntax without this limit.
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for &byte in bytes {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
        } else {
            match byte {
                b'"' => in_string = true,
                b'[' | b'{' => {
                    depth += 1;
                    if depth > 10_000 {
                        return Err(error("exceeded Go JSON maximum depth"));
                    }
                }
                b']' | b'}' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
    }
    Ok(())
}
pub fn decode<T: GoWire>(bytes: &[u8]) -> Result<T, serde_json::Error> {
    let bytes = go_unicode(bytes);
    check_depth(&bytes)?;
    let raw: Box<RawValue> = serde_json::from_slice(&bytes)?;
    serde_json::from_value(merge(T::GO_TYPE, None, &raw, T::SCHEMAS)?.finish())
}

/// Match one encoding/json Decoder.Decode call: validate/decode the first value,
/// ignoring subsequent values or trailing data. HTTP callers enforce their
/// route-specific body limit before calling; WS callers instead use `decode`.
pub fn decode_http_json<T: GoWire>(bytes: &[u8]) -> Result<T, serde_json::Error> {
    decode_http_schema(bytes, T::GO_TYPE, T::SCHEMAS)
}

/// Shared first-value syntax boundary for HTTP structs with service schemas.
pub fn first_http_raw(bytes: &[u8]) -> Result<Box<RawValue>, serde_json::Error> {
    let bytes = go_unicode(bytes);
    let mut deserializer = serde_json::Deserializer::from_slice(&bytes);
    let raw = Box::<RawValue>::deserialize(&mut deserializer)?;
    check_depth(raw.get().as_bytes())?;
    Ok(raw)
}

pub fn decode_http_schema<T: DeserializeOwned>(
    bytes: &[u8],
    kind: &str,
    schemas: &'static [Schema],
) -> Result<T, serde_json::Error> {
    let raw = first_http_raw(bytes)?;
    serde_json::from_value(merge(kind, None, &raw, schemas)?.finish())
}

/// First-value map/interface boundary (not a struct decoder): exact object keys,
/// last duplicate wins, every number decoded as Go float64, and Go Unicode
/// replacement. The bounded generic projection accepts at most 128 containers;
/// this explicit resource boundary is narrower than Go's syntax depth limit.
pub fn decode_http_value(bytes: &[u8]) -> Result<Value, serde_json::Error> {
    fn project(raw: &RawValue, depth: usize) -> Result<Value, serde_json::Error> {
        if depth > 128 {
            return Err(error("generic JSON projection exceeds safe depth"));
        }
        let source = raw.get();
        match source.as_bytes().first() {
            Some(b'{') => {
                let Object(fields) = serde_json::from_str(source)?;
                let mut out = serde_json::Map::new();
                for (name, value) in fields {
                    // Validate earlier duplicate numbers too: Go retains their
                    // error even if a subsequent assignment would be valid.
                    out.insert(name, project(&value, depth + 1)?);
                }
                Ok(Value::Object(out))
            }
            Some(b'[') => {
                let values: Vec<Box<RawValue>> = serde_json::from_str(source)?;
                values
                    .iter()
                    .map(|v| project(v, depth + 1))
                    .collect::<Result<Vec<_>, _>>()
                    .map(Value::Array)
            }
            Some(b'"' | b'n' | b't' | b'f') => serde_json::from_str(source),
            _ => {
                let number: f64 = serde_json::from_str(source)?;
                serde_json::Number::from_f64(number)
                    .map(Value::Number)
                    .ok_or_else(|| error("invalid Go float64"))
            }
        }
    }
    let bytes = go_unicode(bytes);
    let mut deserializer = serde_json::Deserializer::from_slice(&bytes);
    let raw = Box::<RawValue>::deserialize(&mut deserializer)?;
    check_depth(raw.get().as_bytes())?;
    project(&raw, 0)
}

fn json_whitespace(bytes: &[u8], i: &mut usize) {
    while bytes
        .get(*i)
        .is_some_and(|v| matches!(v, b' ' | b'\t' | b'\n' | b'\r'))
    {
        *i += 1;
    }
}
fn json_string_end(bytes: &[u8], mut i: usize) -> usize {
    i += 1;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => return i + 1,
            b'\\' => i += 2,
            _ => i += 1,
        }
    }
    i
}

/// A Go map[string]json.RawMessage boundary: normalize keys, retain raw value
/// bytes (including invalid UTF-8 / lone escapes), retaining duplicate order.
/// The whole document is syntax checked; typed callers decode each owned value
/// through the appropriate shared boundary rather than coercing it into Value.
pub type GoJsonMembers = Vec<(String, Vec<u8>)>;

pub fn decode_go_json_members(bytes: &[u8]) -> Result<Option<GoJsonMembers>, serde_json::Error> {
    let normalized = go_unicode(bytes);
    check_depth(&normalized)?;
    let raw: Box<RawValue> = serde_json::from_slice(&normalized)?;
    if raw.get() == "null" {
        return Ok(None);
    }
    if !raw.get().starts_with('{') {
        return Err(error("Go raw-message map requires an object"));
    }
    let mut out = Vec::new();
    let mut i = 0;
    json_whitespace(bytes, &mut i);
    i += 1; // validated opening object
    loop {
        json_whitespace(bytes, &mut i);
        if bytes.get(i) == Some(&b'}') {
            return Ok(Some(out));
        }
        let end = json_string_end(bytes, i);
        let key: String = serde_json::from_slice(&go_unicode(&bytes[i..end]))?;
        i = end;
        json_whitespace(bytes, &mut i);
        i += 1; // validated colon
        json_whitespace(bytes, &mut i);
        let start = i;
        let mut depth = 0usize;
        while i < bytes.len() {
            match bytes[i] {
                b'"' => i = json_string_end(bytes, i),
                b'[' | b'{' => {
                    depth += 1;
                    i += 1;
                }
                b']' | b'}' if depth > 0 => {
                    depth -= 1;
                    i += 1;
                }
                b',' | b'}' if depth == 0 => break,
                _ => i += 1,
            }
        }
        let mut end = i;
        while end > start && matches!(bytes[end - 1], b' ' | b'\t' | b'\n' | b'\r') {
            end -= 1;
        }
        out.push((key, bytes[start..end].to_vec()));
        if bytes.get(i) == Some(&b',') {
            i += 1;
        }
    }
}

/// Ordered members of the first Go HTTP JSON value, after Go Unicode repair.
/// Callers that deliberately ignore field type errors can retain earlier values.
pub fn decode_http_go_members(bytes: &[u8]) -> Result<Option<GoJsonMembers>, serde_json::Error> {
    let normalized = go_unicode(bytes);
    check_depth(&normalized)?;
    let mut decoder = serde_json::Deserializer::from_slice(&normalized);
    let first = Box::<RawValue>::deserialize(&mut decoder)?;
    decode_go_json_members(first.get().as_bytes())
}

/// Last-exact-key-wins projection matching Go map[string]json.RawMessage.
pub fn decode_go_json_object(
    bytes: &[u8],
) -> Result<Option<BTreeMap<String, Vec<u8>>>, serde_json::Error> {
    decode_go_json_members(bytes).map(|members| members.map(|pairs| pairs.into_iter().collect()))
}
/// Select the last occurrence of a declared Go struct field, including folded
/// spellings. None distinguishes absence from Some(b"null") raw content.
pub fn last_go_raw_field<'a>(members: &'a [(String, Vec<u8>)], field: &str) -> Option<&'a [u8]> {
    let key = fold(field);
    members
        .iter()
        .rev()
        .find(|(name, _)| fold(name) == key)
        .map(|(_, raw)| raw.as_slice())
}

/// Ordered raw array elements, without eager Unicode/number conversion.
pub fn decode_go_json_array(bytes: &[u8]) -> Result<Option<Vec<Vec<u8>>>, serde_json::Error> {
    let normalized = go_unicode(bytes);
    check_depth(&normalized)?;
    let raw: Box<RawValue> = serde_json::from_slice(&normalized)?;
    if raw.get() == "null" {
        return Ok(None);
    }
    if !raw.get().starts_with('[') {
        return Err(error("Go raw-message slice requires an array"));
    }
    let mut out = Vec::new();
    let mut i = 0;
    json_whitespace(bytes, &mut i);
    i += 1;
    loop {
        json_whitespace(bytes, &mut i);
        if bytes.get(i) == Some(&b']') {
            return Ok(Some(out));
        }
        let start = i;
        let mut depth = 0usize;
        while i < bytes.len() {
            match bytes[i] {
                b'"' => i = json_string_end(bytes, i),
                b'[' | b'{' => {
                    depth += 1;
                    i += 1;
                }
                b']' | b'}' if depth > 0 => {
                    depth -= 1;
                    i += 1;
                }
                b',' | b']' if depth == 0 => break,
                _ => i += 1,
            }
        }
        let mut end = i;
        while end > start && matches!(bytes[end - 1], b' ' | b'\t' | b'\n' | b'\r') {
            end -= 1;
        }
        out.push(bytes[start..end].to_vec());
        if bytes.get(i) == Some(&b',') {
            i += 1;
        }
    }
}
// Primitive leaves use the same destination-typed/null/Unicode rules as struct
// fields. Avoid a generic Serde entry that would imply Go struct compatibility.
impl GoWire for String {
    const GO_TYPE: &'static str = "string";
}
impl GoWire for bool {
    const GO_TYPE: &'static str = "bool";
}
impl GoWire for i64 {
    const GO_TYPE: &'static str = "int64";
}
impl GoWire for u64 {
    const GO_TYPE: &'static str = "uint64";
}
impl GoWire for u8 {
    const GO_TYPE: &'static str = "byte";
}
impl GoWire for f64 {
    const GO_TYPE: &'static str = "float64";
}
#[cfg(test)]
mod raw_message_tests {
    use super::*;
    #[derive(serde::Deserialize, Default)]
    #[serde(default)]
    struct Envelope {
        raw: Option<String>,
    }
    #[derive(serde::Deserialize, Default)]
    #[serde(default)]
    struct Payload {
        count: i64,
    }
    const SCHEMAS: &[Schema] = &[
        Schema {
            name: "RawProjectionEnvelope",
            fields: &[Field {
                name: "raw",
                kind: "json.RawMessage",
            }],
        },
        Schema {
            name: "RawProjectionPayload",
            fields: &[Field {
                name: "count",
                kind: "int64",
            }],
        },
    ];
    impl GoWire for Envelope {
        const GO_TYPE: &'static str = "RawProjectionEnvelope";
        const SCHEMAS: &'static [Schema] = SCHEMAS;
    }
    impl GoWire for Payload {
        const GO_TYPE: &'static str = "RawProjectionPayload";
        const SCHEMAS: &'static [Schema] = SCHEMAS;
    }
    #[test]
    fn raw_projection_preserves_duplicates_for_later_destination_validation() {
        let envelope =
            decode::<Envelope>(br#"{"raw":{"count":"invalid","count":2,"ignored":1e1000}}"#)
                .unwrap();
        let raw = envelope.raw.unwrap();
        assert_eq!(raw, r#"{"count":"invalid","count":2,"ignored":1e1000}"#);
        assert!(decode::<Payload>(raw.as_bytes()).is_err());
        let valid = decode::<Envelope>(br#"{"raw":{"count":1,"count":2}}"#).unwrap();
        assert_eq!(
            decode::<Payload>(valid.raw.unwrap().as_bytes())
                .unwrap()
                .count,
            2
        );
    }
    #[test]
    fn raw_null_is_copied_and_a_later_occurrence_replaces_the_whole_raw_value() {
        assert_eq!(
            decode::<Envelope>(br#"{"raw":null}"#)
                .unwrap()
                .raw
                .as_deref(),
            Some("null")
        );
        assert!(decode::<Envelope>(b"{}").unwrap().raw.is_none());
        assert_eq!(
            decode::<Envelope>(br#"{"raw":{"count":1},"raw":null}"#)
                .unwrap()
                .raw
                .as_deref(),
            Some("null")
        );
        assert_eq!(
            decode::<Envelope>(br#"{"raw":null,"raw":{"count":2}}"#)
                .unwrap()
                .raw
                .as_deref(),
            Some(r#"{"count":2}"#)
        );
    }
}
