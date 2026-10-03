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
fn schema(kind: &str) -> Option<&'static Schema> {
    super::generated::WIRE_SCHEMA
        .iter()
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
fn merge(kind: &str, old: Option<Decoded>, raw: &RawValue) -> Result<Decoded, serde_json::Error> {
    let source = raw.get();
    if let Some(inner) = kind.strip_prefix('*') {
        if source == "null" {
            return Ok(Decoded::Leaf(Value::Null));
        }
        return merge(inner, old, raw);
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
            let next = merge(inner, previous, item)?;
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
            map.insert(key, merge(inner, None, &v)?);
        }
        return Ok(Decoded::Object(map));
    }
    if let Some(schema) = schema(kind) {
        if source == "null" {
            return Ok(old.unwrap_or_else(|| Decoded::Object(BTreeMap::new())));
        }
        let Object(fields) = serde_json::from_str(source)?;
        let mut map = old.and_then(Decoded::object).unwrap_or_default();
        for (key, v) in fields {
            if let Some(field) = schema.fields.iter().find(|f| fold(f.name) == fold(&key)) {
                let previous = map.remove(field.name);
                map.insert(field.name.into(), merge(field.kind, previous, &v)?);
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
    serde_json::from_value(merge(T::GO_TYPE, None, &raw)?.finish())
}
