//! Wire-facing JSON decoding with Go encoding/json object merge semantics.
//! Raw Serde DTO decoding is for canonical internal values only; HTTP/WS callers
//! must use this boundary for case-folded fields, duplicate keys and null defaults.
use serde::{
    Deserialize, Deserializer,
    de::{DeserializeOwned, MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, Number, Value};
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
#[derive(Clone)]
enum Raw {
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    Array(Vec<Raw>),
    Object(Vec<(String, Raw)>),
}
impl<'de> Deserialize<'de> for Raw {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct RawVisitor;
        impl<'de> Visitor<'de> for RawVisitor {
            type Value = Raw;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("JSON value")
            }
            fn visit_unit<E>(self) -> Result<Raw, E> {
                Ok(Raw::Null)
            }
            fn visit_none<E>(self) -> Result<Raw, E> {
                Ok(Raw::Null)
            }
            fn visit_bool<E>(self, v: bool) -> Result<Raw, E> {
                Ok(Raw::Bool(v))
            }
            fn visit_i64<E>(self, v: i64) -> Result<Raw, E> {
                Ok(Raw::Number(v.into()))
            }
            fn visit_u64<E>(self, v: u64) -> Result<Raw, E> {
                Ok(Raw::Number(v.into()))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Raw, E> {
                Number::from_f64(v)
                    .map(Raw::Number)
                    .ok_or_else(|| E::custom("non-finite number"))
            }
            fn visit_str<E>(self, v: &str) -> Result<Raw, E> {
                Ok(Raw::String(v.into()))
            }
            fn visit_string<E>(self, v: String) -> Result<Raw, E> {
                Ok(Raw::String(v))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Raw, A::Error> {
                let mut out = vec![];
                while let Some(v) = seq.next_element()? {
                    out.push(v);
                }
                Ok(Raw::Array(out))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Raw, A::Error> {
                let mut out = vec![];
                while let Some(pair) = map.next_entry()? {
                    out.push(pair);
                }
                Ok(Raw::Object(out))
            }
        }
        deserializer.deserialize_any(RawVisitor)
    }
}
fn value(raw: Raw) -> Value {
    match raw {
        Raw::Null => Value::Null,
        Raw::Bool(v) => v.into(),
        Raw::Number(v) => v.into(),
        Raw::String(v) => v.into(),
        Raw::Array(v) => Value::Array(v.into_iter().map(value).collect()),
        Raw::Object(v) => Value::Object(v.into_iter().map(|(k, v)| (k, value(v))).collect()),
    }
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
fn merge(kind: &str, old: Option<Value>, raw: Raw) -> Value {
    if let Some(inner) = kind.strip_prefix('*') {
        if matches!(raw, Raw::Null) {
            return Value::Null;
        }
        return merge(inner, old, raw);
    }
    if let Some(inner) = kind.strip_prefix("[]") {
        return match raw {
            Raw::Null => Value::Null,
            Raw::Array(items) => {
                Value::Array(items.into_iter().map(|v| merge(inner, None, v)).collect())
            }
            other => value(other),
        };
    }
    if let Some(inner) = kind.strip_prefix("map[string]") {
        return match raw {
            Raw::Null => Value::Null,
            Raw::Object(fields) => {
                let mut map = old.and_then(|v| v.as_object().cloned()).unwrap_or_default();
                for (key, v) in fields {
                    map.insert(key, merge(inner, None, v));
                }
                Value::Object(map)
            }
            other => value(other),
        };
    }
    if let Some(schema) = schema(kind) {
        if matches!(raw, Raw::Null) {
            return old.unwrap_or_else(|| Value::Object(Map::new()));
        }
        let Raw::Object(fields) = raw else {
            return value(raw);
        };
        let mut map = old.and_then(|v| v.as_object().cloned()).unwrap_or_default();
        for (key, v) in fields {
            if let Some(field) = schema.fields.iter().find(|f| fold(f.name) == fold(&key)) {
                let previous = map.remove(field.name);
                map.insert(field.name.into(), merge(field.kind, previous, v));
            }
        }
        return Value::Object(map);
    }
    if matches!(raw, Raw::Null) {
        return old.unwrap_or_else(|| match kind {
            "bool" => false.into(),
            "int" | "int64" | "uint64" | "byte" => 0.into(),
            "float64" => 0.0.into(),
            _ => "".into(),
        });
    }
    value(raw)
}
pub fn decode<T: GoWire>(bytes: &[u8]) -> Result<T, serde_json::Error> {
    let raw: Raw = serde_json::from_slice(bytes)?;
    serde_json::from_value(merge(T::GO_TYPE, None, raw))
}
