//! Schema-directed YAML bridge for the frozen gopkg.in/yaml.v3 config reader.
//!
//! Never deserialize the entire document into JSON first: doing so loses the
//! original spelling of string settings and rejects ignored YAML-only values.
//! This arena retains decoded scalar text, notation and tags until the Go field
//! type is known. Unknown subtrees are parsed for YAML syntax but not resolved.
//! None of these private nodes implement Debug; errors contain no input bytes.
use super::model::schema;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Map, Number, Value};
use serde_saphyr::granit_parser::{ErrorKind, Event, Parser, ScalarStyle, ScanError, Span};
use std::collections::{BTreeMap, BTreeSet};

const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_NODES: usize = 200_000;
const MAX_DEPTH: usize = 128;
const MAX_EXPANSION_STEPS: usize = 1_000_000;
const CORE_TAG: &str = "tag:yaml.org,2002:";

/// Resource exhaustion must not trigger corrupt-file backup or token rotation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum YamlDecodeError {
    Syntax,
    ResourceLimit,
}
impl std::fmt::Display for YamlDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Syntax => "config YAML syntax is invalid (content redacted)",
            Self::ResourceLimit => "config YAML exceeds safe decoding limits (content redacted)",
        })
    }
}

struct Scalar {
    text: String,
    style: ScalarStyle,
    tag: Option<String>,
}
enum Node {
    Pending,
    Scalar(Scalar),
    Sequence(Vec<usize>),
    Mapping(Vec<(usize, usize)>),
    Alias(usize),
}
#[derive(Default)]
struct Document {
    nodes: Vec<Node>,
    anchors: BTreeMap<usize, usize>,
}

fn parser_error(error: ScanError) -> YamlDecodeError {
    match error.kind() {
        ErrorKind::RecursionLimitExceeded
        | ErrorKind::InputByteLimitExceeded { .. }
        | ErrorKind::DirectiveByteLimitExceeded { .. }
        | ErrorKind::TooManyReservedDirectiveParams { .. }
        | ErrorKind::TooManyComments
        | ErrorKind::AnchorCountOverflow
        | ErrorKind::YamlVersionTooLong => YamlDecodeError::ResourceLimit,
        _ => YamlDecodeError::Syntax,
    }
}
fn next_event<'a>(
    events: &mut impl Iterator<Item = Result<(Event<'a>, Span), ScanError>>,
) -> Result<Option<Event<'a>>, YamlDecodeError> {
    loop {
        match events.next() {
            Some(Ok((Event::Comment(..), _))) => {}
            Some(Ok((event, _))) => return Ok(Some(event)),
            Some(Err(error)) => return Err(parser_error(error)),
            None => return Ok(None),
        }
    }
}
impl Document {
    fn parse_node<'a>(
        &mut self,
        first: Event<'a>,
        events: &mut impl Iterator<Item = Result<(Event<'a>, Span), ScanError>>,
        depth: usize,
    ) -> Result<usize, YamlDecodeError> {
        if depth > MAX_DEPTH || self.nodes.len() >= MAX_NODES {
            return Err(YamlDecodeError::ResourceLimit);
        }
        let index = self.nodes.len();
        self.nodes.push(Node::Pending);
        let anchor = match &first {
            Event::Scalar(_, _, anchor, _)
            | Event::SequenceStart(_, anchor, _)
            | Event::MappingStart(_, anchor, _) => *anchor,
            _ => 0,
        };
        if anchor != 0 {
            self.anchors.insert(anchor, index);
        }
        let node = match first {
            Event::Scalar(text, style, _, tag) => Node::Scalar(Scalar {
                text: text.into_owned(),
                style,
                tag: tag.map(|tag| format!("{}{}", tag.handle(), tag.suffix())),
            }),
            Event::Alias(anchor) => {
                Node::Alias(*self.anchors.get(&anchor).ok_or(YamlDecodeError::Syntax)?)
            }
            Event::SequenceStart(..) => {
                let mut children = Vec::new();
                loop {
                    let event = next_event(events)?.ok_or(YamlDecodeError::Syntax)?;
                    if matches!(event, Event::SequenceEnd) {
                        break;
                    }
                    children.push(self.parse_node(event, events, depth + 1)?);
                }
                Node::Sequence(children)
            }
            Event::MappingStart(..) => {
                let mut pairs = Vec::new();
                loop {
                    let event = next_event(events)?.ok_or(YamlDecodeError::Syntax)?;
                    if matches!(event, Event::MappingEnd) {
                        break;
                    }
                    let key = self.parse_node(event, events, depth + 1)?;
                    let event = next_event(events)?.ok_or(YamlDecodeError::Syntax)?;
                    let value = self.parse_node(event, events, depth + 1)?;
                    pairs.push((key, value));
                }
                Node::Mapping(pairs)
            }
            _ => return Err(YamlDecodeError::Syntax),
        };
        self.nodes[index] = node;
        Ok(index)
    }
}

pub(super) fn decode_config(text: &str) -> Result<Value, YamlDecodeError> {
    if text.len() > MAX_BYTES {
        return Err(YamlDecodeError::ResourceLimit);
    }
    let mut options = serde_saphyr::granit_parser::Options::default();
    options.emit_comments = false;
    let mut events = Parser::new_from_str_with_options(text, options);
    let mut document = Document::default();
    let root = loop {
        match next_event(&mut events)? {
            Some(Event::StreamStart | Event::DocumentStart(..)) => {}
            Some(Event::StreamEnd) | None => return Ok(Value::Null),
            Some(event) => break document.parse_node(event, &mut events, 0)?,
        }
    };
    // yaml.Unmarshal reads one document. Do not interpret later documents.
    if !matches!(next_event(&mut events)?, Some(Event::DocumentEnd)) {
        return Err(YamlDecodeError::Syntax);
    }
    Decoder {
        document: &document,
        steps: 0,
    }
    .project(root, "Config", 0)
}

struct Decoder<'a> {
    document: &'a Document,
    steps: usize,
}
impl Decoder<'_> {
    fn charge(&mut self, depth: usize) -> Result<(), YamlDecodeError> {
        self.steps += 1;
        if depth > MAX_DEPTH || self.steps > MAX_EXPANSION_STEPS {
            Err(YamlDecodeError::ResourceLimit)
        } else {
            Ok(())
        }
    }
    fn resolve_alias(&mut self, mut id: usize, depth: usize) -> Result<usize, YamlDecodeError> {
        self.charge(depth)?;
        while let Node::Alias(target) = self.document.nodes[id] {
            self.charge(depth)?;
            id = target;
        }
        Ok(id)
    }
    fn is_null(&mut self, id: usize, depth: usize) -> Result<bool, YamlDecodeError> {
        let id = self.resolve_alias(id, depth)?;
        Ok(matches!(&self.document.nodes[id], Node::Scalar(scalar) if scalar.is_null()))
    }
    /// Resolve mapping merges after direct entries. Earlier maps in a merge
    /// sequence win; explicit entries win regardless of where `<<` appears.
    fn pairs(
        &mut self,
        id: usize,
        depth: usize,
    ) -> Result<Option<Vec<(String, usize)>>, YamlDecodeError> {
        let id = self.resolve_alias(id, depth)?;
        let Node::Mapping(entries) = &self.document.nodes[id] else {
            return Ok(None);
        };
        let mut seen = BTreeSet::new();
        let mut result = Vec::new();
        let mut merge = None;
        for &(key, value) in entries {
            self.charge(depth)?;
            let key = self.resolve_alias(key, depth + 1)?;
            let Node::Scalar(scalar) = &self.document.nodes[key] else {
                return Ok(None);
            };
            // yaml.v3 duplicate checks compare node kind/value, even when tags differ.
            if !seen.insert(scalar.text.clone()) {
                return Ok(None);
            }
            if scalar.is_merge() {
                merge = Some(value);
                continue;
            }
            if scalar.is_null() {
                continue;
            }
            let Some(name) = scalar.string_value()? else {
                return Ok(None);
            };
            result.push((name, value));
        }
        if let Some(merge) = merge {
            let merge = self.resolve_alias(merge, depth + 1)?;
            let merge_ids: Vec<usize> = match &self.document.nodes[merge] {
                Node::Mapping(_) => vec![merge],
                Node::Sequence(ids) => ids.clone(),
                _ => return Ok(None),
            };
            let mut names: BTreeSet<String> = result.iter().map(|(name, _)| name.clone()).collect();
            for merge in merge_ids {
                let Some(pairs) = self.pairs(merge, depth + 1)? else {
                    return Ok(None);
                };
                for (name, value) in pairs {
                    if names.insert(name.clone()) {
                        result.push((name, value));
                    }
                }
            }
        }
        Ok(Some(result))
    }
    // SessionOrderIDs first decodes []any in Go. A bad generic descendant
    // invalidates that entire optional list, even if the item itself would be
    // discarded later by the numeric-ID filter.
    fn valid_generic(&mut self, id: usize, depth: usize) -> Result<bool, YamlDecodeError> {
        let id = self.resolve_alias(id, depth)?;
        match &self.document.nodes[id] {
            Node::Scalar(scalar) => Ok(!matches!(scalar.resolve(), Resolved::Invalid)
                && (scalar.tag() != Some("tag:yaml.org,2002:binary")
                    || STANDARD
                        .decode(scalar.text.replace(['\r', '\n'], ""))
                        .is_ok())),
            Node::Sequence(items) => {
                for &item in items {
                    if !self.valid_generic(item, depth + 1)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Node::Mapping(entries) => {
                let mut seen = BTreeSet::new();
                for &(key, value) in entries {
                    let key = self.resolve_alias(key, depth + 1)?;
                    let Node::Scalar(scalar) = &self.document.nodes[key] else {
                        return Ok(false);
                    };
                    if !seen.insert(scalar.text.clone())
                        || !self.valid_generic(key, depth + 1)?
                        || !self.valid_generic(value, depth + 1)?
                    {
                        return Ok(false);
                    }
                    if scalar.is_merge() && self.pairs(value, depth + 1)?.is_none() {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn project(&mut self, id: usize, kind: &str, depth: usize) -> Result<Value, YamlDecodeError> {
        let id = self.resolve_alias(id, depth)?;
        if self.is_null(id, depth)? {
            return Ok(Value::Null);
        }
        let kind = kind.strip_prefix('*').unwrap_or(kind);
        if kind == "SessionOrderIDs" && !self.valid_generic(id, depth + 1)? {
            return Ok(Value::Array(Vec::new()));
        }
        let kind = match kind {
            "CustomProviders" => "[]CustomProvider",
            "SubscriptionProfiles" => "map[string][]SubscriptionProfile",
            "SessionOrderIDs" => "[]any-session-id",
            other => other,
        };
        if let Some(inner) = kind.strip_prefix("[]") {
            let Node::Sequence(items) = &self.document.nodes[id] else {
                return Ok(incompatible(kind));
            };
            let mut output = Vec::new();
            for &item in items {
                // yaml.v3 does not append null to a slice of scalar/struct values.
                if self.is_null(item, depth + 1)?
                    && inner != "any-session-id"
                    && !inner.starts_with('*')
                    && !inner.starts_with("[]")
                    && !inner.starts_with("map[")
                {
                    continue;
                }
                output.push(self.project(item, inner, depth + 1)?);
            }
            return Ok(Value::Array(output));
        }
        if let Some(inner) = kind.strip_prefix("map[string]") {
            let Some(pairs) = self.pairs(id, depth + 1)? else {
                return Ok(incompatible(kind));
            };
            let mut output = Map::new();
            for (name, value) in pairs {
                output.insert(name, self.project(value, inner, depth + 1)?);
            }
            return Ok(Value::Object(output));
        }
        let fields = schema(kind);
        if !fields.is_empty() {
            let Some(pairs) = self.pairs(id, depth + 1)? else {
                return Ok(incompatible(kind));
            };
            let mut output = Map::new();
            for (name, value) in pairs {
                // Critically, no JSON conversion, resolution or alias expansion of
                // unknown values occurs here, even for non-string keys/NaN below.
                if let Some(field) = fields.iter().find(|field| field.yaml == name) {
                    output.insert(name, self.project(value, field.kind, depth + 1)?);
                }
            }
            return Ok(Value::Object(output));
        }
        let Node::Scalar(scalar) = &self.document.nodes[id] else {
            return Ok(incompatible(kind));
        };
        Ok(match kind {
            "int" => scalar
                .integer_value()
                .map(Value::from)
                .unwrap_or_else(|| incompatible(kind)),
            "bool" => scalar
                .boolean_value()
                .map(Value::Bool)
                .unwrap_or_else(|| incompatible(kind)),
            "any-session-id" => scalar.session_id_value(),
            _ => scalar
                .string_value()?
                .map(Value::String)
                .unwrap_or_else(|| incompatible(kind)),
        })
    }
}

fn incompatible(kind: &str) -> Value {
    // Shapes deliberately incompatible with the requested schema type. The
    // existing normalizer retains its source-grounded optional-section policy.
    if kind.starts_with("[]") || kind.starts_with("map[") || !schema(kind).is_empty() {
        Value::Bool(false)
    } else {
        Value::Array(Vec::new())
    }
}

enum Resolved {
    Null,
    String,
    Boolean(bool),
    Integer(i64),
    Unsigned(u64),
    Float(f64),
    Invalid,
}
impl Scalar {
    fn tag(&self) -> Option<&str> {
        self.tag.as_deref().filter(|tag| *tag != "!")
    }
    fn is_null(&self) -> bool {
        matches!(self.resolve(), Resolved::Null)
    }
    fn is_merge(&self) -> bool {
        self.text == "<<"
            && (self.tag() == Some("tag:yaml.org,2002:merge")
                || (self.tag().is_none() && self.style == ScalarStyle::Plain))
    }
    fn resolve(&self) -> Resolved {
        let tag = self.tag();
        if tag == Some("tag:yaml.org,2002:str")
            || (tag.is_none() && self.style != ScalarStyle::Plain)
        {
            return Resolved::String;
        }
        if tag.is_some_and(|tag| {
            !matches!(
                tag.strip_prefix(CORE_TAG),
                Some("null" | "bool" | "int" | "float" | "timestamp")
            )
        }) {
            return Resolved::String;
        }
        let resolved = infer_scalar(&self.text);
        match tag.and_then(|tag| tag.strip_prefix(CORE_TAG)) {
            None => resolved,
            Some("null") if matches!(resolved, Resolved::Null) => resolved,
            Some("bool") if matches!(resolved, Resolved::Boolean(_)) => resolved,
            Some("int") if matches!(resolved, Resolved::Integer(_) | Resolved::Unsigned(_)) => {
                resolved
            }
            Some("float") => match resolved {
                Resolved::Integer(value) => Resolved::Float(value as f64),
                Resolved::Float(_) => resolved,
                _ => Resolved::Invalid,
            },
            Some("timestamp") => {
                // yaml.v3 validates an explicit timestamp tag before assigning
                // even a string destination, while preserving its source text.
                if valid_timestamp(&self.text) {
                    Resolved::String
                } else {
                    Resolved::Invalid
                }
            }
            _ => Resolved::Invalid,
        }
    }
    fn string_value(&self) -> Result<Option<String>, YamlDecodeError> {
        if self.tag() == Some("tag:yaml.org,2002:binary") {
            let bytes = match STANDARD.decode(self.text.replace(['\r', '\n'], "")) {
                Ok(bytes) => bytes,
                Err(_) => return Ok(None),
            };
            // Rust strings cannot preserve arbitrary Go byte strings. Refuse
            // non-UTF8 binary settings without invoking corrupt-file recovery.
            return String::from_utf8(bytes)
                .map(Some)
                .map_err(|_| YamlDecodeError::ResourceLimit);
        }
        Ok(match self.resolve() {
            Resolved::Invalid | Resolved::Null => None,
            _ => Some(self.text.clone()),
        })
    }
    fn boolean_value(&self) -> Option<bool> {
        match self.resolve() {
            Resolved::Boolean(value) => Some(value),
            Resolved::String => match self.text.as_str() {
                "y" | "Y" | "yes" | "Yes" | "YES" | "on" | "On" | "ON" => Some(true),
                "n" | "N" | "no" | "No" | "NO" | "off" | "Off" | "OFF" => Some(false),
                _ => None,
            },
            _ => None,
        }
    }
    fn integer_value(&self) -> Option<i64> {
        match self.resolve() {
            Resolved::Integer(value) => Some(value),
            Resolved::Unsigned(value) => i64::try_from(value).ok(),
            Resolved::Float(value) if value <= i64::MAX as f64 => {
                // yaml.v3 on supported 64-bit targets casts negative overflow
                // and -Inf to MinInt64; positive overflow and NaN fail.
                if value >= -(i64::MIN as f64) {
                    Some(i64::MIN)
                } else {
                    Some(value as i64)
                }
            }
            _ => None,
        }
    }
    fn session_id_value(&self) -> Value {
        match self.resolve() {
            Resolved::Null => Value::Null,
            Resolved::Boolean(value) => Value::Bool(value),
            Resolved::Integer(value) => Value::from(value),
            Resolved::Unsigned(_) => incompatible("any-session-id"),
            Resolved::Float(value) => Number::from_f64(value)
                .map(Value::Number)
                .unwrap_or(Value::Null),
            Resolved::String => Value::String(self.text.clone()),
            Resolved::Invalid => incompatible("any-session-id"),
        }
    }
}

fn infer_scalar(text: &str) -> Resolved {
    match text {
        "" | "~" | "null" | "Null" | "NULL" => return Resolved::Null,
        "true" | "True" | "TRUE" => return Resolved::Boolean(true),
        "false" | "False" | "FALSE" => return Resolved::Boolean(false),
        ".nan" | ".NaN" | ".NAN" => return Resolved::Float(f64::NAN),
        ".inf" | ".Inf" | ".INF" | "+.inf" | "+.Inf" | "+.INF" => {
            return Resolved::Float(f64::INFINITY);
        }
        "-.inf" | "-.Inf" | "-.INF" => return Resolved::Float(f64::NEG_INFINITY),
        _ => {}
    }
    if !text.starts_with(|character: char| {
        character.is_ascii_digit() || matches!(character, '+' | '-' | '.')
    }) {
        return Resolved::String;
    }
    let plain = text.replace('_', "");
    let negative = plain.starts_with('-');
    let unsigned = plain.strip_prefix(['+', '-']).unwrap_or(&plain);
    let (digits, radix) = if let Some(digits) = unsigned
        .strip_prefix("0x")
        .or_else(|| unsigned.strip_prefix("0X"))
    {
        (digits, 16)
    } else if let Some(digits) = unsigned
        .strip_prefix("0b")
        .or_else(|| unsigned.strip_prefix("0B"))
    {
        (digits, 2)
    } else if let Some(digits) = unsigned
        .strip_prefix("0o")
        .or_else(|| unsigned.strip_prefix("0O"))
    {
        (digits, 8)
    } else if unsigned.starts_with('0') && unsigned.len() > 1 {
        (&unsigned[1..], 8)
    } else {
        (unsigned, 10)
    };
    if !digits.is_empty()
        && digits.chars().all(|character| character.is_digit(radix))
        && let Ok(value) = u64::from_str_radix(digits, radix)
    {
        if negative {
            if value == (1_u64 << 63) {
                return Resolved::Integer(i64::MIN);
            }
            if let Ok(value) = i64::try_from(value) {
                return Resolved::Integer(-value);
            }
        } else if let Ok(value) = i64::try_from(value) {
            return Resolved::Integer(value);
        } else {
            return Resolved::Unsigned(value);
        }
    }
    // Go's resolver accepts decimal/exponent floats after integer resolution,
    // including legacy 08/09. It does not treat bare inf/NaN as YAML numbers.
    if plain
        .bytes()
        .all(|byte| byte.is_ascii_digit() || b"+-.eE".contains(&byte))
        && let Ok(value) = plain.parse::<f64>()
        && value.is_finite()
    {
        return Resolved::Float(value);
    }
    Resolved::String
}

// The four timestamp layouts accepted by yaml.v3 resolve.go. No timestamps are
// materialized as wall-clock values: Config destinations retain their text.
fn valid_timestamp(text: &str) -> bool {
    fn decimal(text: &str, max_digits: usize) -> Option<u32> {
        (!text.is_empty()
            && text.len() <= max_digits
            && text.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| text.parse().ok())
        .flatten()
    }
    let (date, time, separator) = match text.find(['T', 't', ' ']) {
        Some(index) => (
            &text[..index],
            Some(&text[index + 1..]),
            text.as_bytes()[index],
        ),
        None => (text, None, 0),
    };
    let date: Vec<&str> = date.split('-').collect();
    if date.len() != 3 || date[0].len() != 4 {
        return false;
    }
    let (Some(year), Some(month), Some(day)) = (
        decimal(date[0], 4),
        decimal(date[1], 2),
        decimal(date[2], 2),
    ) else {
        return false;
    };
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => return false,
    };
    if day == 0 || day > max_day {
        return false;
    }
    let Some(mut time) = time else { return true };
    if separator != b' ' {
        if let Some(without_zone) = time.strip_suffix('Z') {
            time = without_zone;
        } else if let Some(index) = time.find(['+', '-']) {
            let zone = &time[index + 1..];
            if zone.len() != 5 || zone.as_bytes()[2] != b':' {
                return false;
            }
            let (Some(hours), Some(minutes)) = (decimal(&zone[..2], 2), decimal(&zone[3..], 2))
            else {
                return false;
            };
            if hours > 24 || minutes > 60 {
                return false;
            }
            time = &time[..index];
        } else {
            return false;
        }
    }
    if let Some(index) = time.find(['.', ',']) {
        let fraction = &time[index + 1..];
        if fraction.is_empty() || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
            return false;
        }
        time = &time[..index];
    }
    let time: Vec<&str> = time.split(':').collect();
    if time.len() != 3 {
        return false;
    }
    let (Some(hour), Some(minute), Some(second)) = (
        decimal(time[0], 2),
        decimal(time[1], 2),
        decimal(time[2], 2),
    ) else {
        return false;
    };
    hour < 24 && minute < 60 && second < 60
}
