//! Bounded typed transcript records for Claude, Codex and Command Code.
//! All output text uses the sessionlog mask; tool identity and record cursors
//! stay parser-owned across polls.
use super::reader::*;
use crate::{
    proto::{
        self,
        wire::{Field, GoWire, Schema},
    },
    storage::mask_secrets,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, VecDeque},
    io,
    path::Path,
    time::{Duration, SystemTime},
};
const TEXT_MAX: usize = 64 * 1024;
const MESSAGE_MAX: usize = 200;
const BATCH_MAX: usize = 8 * 1024 * 1024;
const TOOLS_MAX: usize = 64;
const PENDING_MAX: usize = 256;
const PENDING_BYTES: usize = 4 * 1024 * 1024;
#[derive(Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentChatTool {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub input: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub result: String,
}
#[derive(Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentChatMessage {
    pub role: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub presentation: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub thinking: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<AgentChatTool>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ts: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub message_id: String,
    #[serde(skip)]
    pub source_start: u64,
    #[serde(skip)]
    pub source_end: u64,
}
impl AgentChatMessage {
    fn bytes(&self) -> usize {
        self.role.len()
            + self.kind.len()
            + self.text.len()
            + self.ts.len()
            + self.message_id.len()
            + self.thinking.iter().map(String::len).sum::<usize>()
            + self
                .tools
                .iter()
                .map(|t| t.id.len() + t.name.len() + t.input.len() + t.result.len())
                .sum::<usize>()
    }
    fn bound(&mut self) {
        self.text = limit(&self.text);
        self.thinking.truncate(64);
        for t in &mut self.thinking {
            *t = limit(t);
        }
        self.tools.truncate(TOOLS_MAX);
        for t in &mut self.tools {
            t.name = limit(&t.name);
            t.input = limit(&t.input);
            t.result = limit(&t.result);
        }
        if self.message_id.is_empty()
            && let Some(tool) = self.tools.iter().find(|t| !t.id.is_empty())
        {
            self.message_id = format!("tool:{}", tool.id);
        }
    }
}
#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct CodexCompletion {
    pub turn_id: String,
    pub at: String,
    pub last_agent_message: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderFormat {
    Claude,
    Codex,
    CommandCode,
}
impl ProviderFormat {
    pub fn for_provider(provider: &str) -> Option<Self> {
        match provider {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "command-code" => Some(Self::CommandCode),
            _ => None,
        }
    }
}
#[derive(Clone)]
struct ToolRef {
    message: u64,
    index: usize,
    batch: u64,
}
#[derive(Clone)]
pub struct ParseState {
    pub read_state: ReadState,
    pub last_read: ReadStats,
    messages: BTreeMap<u64, AgentChatMessage>,
    visible: VecDeque<u64>,
    tools: BTreeMap<String, ToolRef>,
    pending_order: VecDeque<String>,
    next_message: u64,
    batch: u64,
    max_messages: usize,
    max_bytes: usize,
    pub parsed_messages: u64,
    completions: Vec<CodexCompletion>,
}
impl Default for ParseState {
    fn default() -> Self {
        Self::new(MESSAGE_MAX, BATCH_MAX)
    }
}
impl ParseState {
    pub fn new(max_messages: usize, max_bytes: usize) -> Self {
        Self {
            read_state: ReadState::default(),
            last_read: ReadStats::default(),
            messages: BTreeMap::new(),
            visible: VecDeque::new(),
            tools: BTreeMap::new(),
            pending_order: VecDeque::new(),
            next_message: 0,
            batch: 0,
            max_messages: if max_messages == 0 {
                MESSAGE_MAX
            } else {
                max_messages
            },
            max_bytes: if max_bytes == 0 { BATCH_MAX } else { max_bytes },
            parsed_messages: 0,
            completions: Vec::new(),
        }
    }
    pub fn begin_batch(&mut self) {
        self.visible.clear();
        self.completions.clear();
        self.batch = self.batch.wrapping_add(1).max(1);
        self.collect_unused();
    }
    fn collect_unused(&mut self) {
        let visible = &self.visible;
        let refs = &self.tools;
        self.messages
            .retain(|id, _| visible.contains(id) || refs.values().any(|r| r.message == *id));
    }
    fn remove_tool(&mut self, id: &str) {
        self.tools.remove(id);
        self.pending_order.retain(|key| key != id);
    }
    fn pending_bytes(&self) -> usize {
        let mut ids = std::collections::HashSet::new();
        self.tools
            .values()
            .filter(|r| ids.insert(r.message))
            .filter_map(|r| self.messages.get(&r.message))
            .map(AgentChatMessage::bytes)
            .sum()
    }
    fn evict_pending(&mut self) -> bool {
        if let Some(id) = self.pending_order.pop_front() {
            self.tools.remove(&id);
            true
        } else {
            false
        }
    }
    fn append(&mut self, mut message: AgentChatMessage, register: bool) {
        message.bound();
        self.next_message += 1;
        let id = self.next_message;
        self.messages.insert(id, message);
        self.append_existing(id, true);
        if register {
            self.register_tools(id);
        }
        self.collect_unused();
    }
    fn append_existing(&mut self, id: u64, count: bool) {
        if count {
            self.parsed_messages += 1;
        }
        let bytes = self
            .messages
            .get(&id)
            .map(AgentChatMessage::bytes)
            .unwrap_or(0);
        while self.visible.len() >= self.max_messages
            || self.batch_bytes().saturating_add(bytes) > self.max_bytes
        {
            let Some(old) = self.visible.pop_front() else {
                break;
            };
            let ids: Vec<_> = self
                .messages
                .get(&old)
                .map(|m| m.tools.iter().map(|t| t.id.clone()).collect())
                .unwrap_or_default();
            for tool in ids {
                self.remove_tool(&tool);
            }
        }
        self.visible.push_back(id);
    }
    fn batch_bytes(&self) -> usize {
        self.visible
            .iter()
            .filter_map(|id| self.messages.get(id))
            .map(AgentChatMessage::bytes)
            .sum()
    }
    fn register_tools(&mut self, id: u64) {
        let Some(message) = self.messages.get(&id) else {
            return;
        };
        let ids: Vec<_> = message
            .tools
            .iter()
            .enumerate()
            .filter(|(_, t)| !t.id.is_empty())
            .map(|(i, t)| (i, t.id.clone()))
            .collect();
        let bytes = message.bytes();
        for (index, tool) in ids {
            self.remove_tool(&tool);
            if bytes > PENDING_BYTES {
                continue;
            }
            let counted = self.tools.values().any(|r| r.message == id);
            while self.tools.len() >= PENDING_MAX
                || (!counted && self.pending_bytes() + bytes > PENDING_BYTES)
            {
                if !self.evict_pending() {
                    break;
                }
            }
            if self.tools.len() >= PENDING_MAX
                || (!counted && self.pending_bytes() + bytes > PENDING_BYTES)
            {
                continue;
            }
            self.tools.insert(
                tool.clone(),
                ToolRef {
                    message: id,
                    index,
                    batch: self.batch,
                },
            );
            self.pending_order.push_back(tool);
        }
    }
    fn attach_result(&mut self, id: &str, result: String) {
        if result.is_empty() {
            return;
        }
        let Some(reference) = self.tools.get(id).cloned() else {
            return;
        };
        let Some(message) = self.messages.get_mut(&reference.message) else {
            return;
        };
        let Some(tool) = message.tools.get_mut(reference.index) else {
            return;
        };
        tool.result = masked(&result);
        while self.pending_bytes() > PENDING_BYTES && !self.tools.is_empty() {
            if !self.evict_pending() {
                break;
            }
        }
        if reference.batch != self.batch {
            if let Some(r) = self.tools.get_mut(id) {
                r.batch = self.batch;
            }
            self.append_existing(reference.message, false);
        }
        self.remove_tool(id);
        self.collect_unused();
    }
    pub fn output_messages(&self) -> Vec<AgentChatMessage> {
        self.visible
            .iter()
            .filter_map(|id| self.messages.get(id).cloned())
            .collect()
    }
    pub fn take_completions(&mut self) -> Vec<CodexCompletion> {
        std::mem::take(&mut self.completions)
    }
    pub fn pending_count(&self) -> usize {
        self.tools.len()
    }
    pub fn retained_pending_bytes(&self) -> usize {
        self.pending_bytes()
    }
    pub fn parse_record(&mut self, format: ProviderFormat, line: &[u8]) {
        if line.len() > MAX_RECORD {
            return;
        }
        match format {
            ProviderFormat::Claude | ProviderFormat::CommandCode => {
                if let Ok(record) = decode_anthropic(line) {
                    self.anthropic(format, record);
                }
            }
            ProviderFormat::Codex => {
                if let Ok(record) = decode_codex_record(line)
                    && let Some(raw) = record.payload.0
                    && let Ok(payload) = decode_codex_payload(&raw)
                {
                    self.codex(&record.kind, &record.timestamp, payload);
                }
            }
        }
    }
    pub fn read_forward(
        &mut self,
        path: &Path,
        format: ProviderFormat,
        budget: &ReadBudget,
    ) -> io::Result<Vec<AgentChatMessage>> {
        self.begin_batch();
        let mut continuation = std::mem::take(&mut self.read_state);
        let offset = continuation.next_offset;
        let result = read_range(path, offset, &mut continuation, budget, |line, _, _| {
            self.parse_record(format, line);
            Ok(())
        });
        self.read_state = continuation;
        self.last_read = result?;
        Ok(self.output_messages())
    }
    /// Prime/tail decode is atomic for the selected bounded page: once decoding
    /// starts, finish every selected record even if the deadline arrives meanwhile.
    pub fn read_tail(
        &mut self,
        path: &Path,
        format: ProviderFormat,
        end: Option<u64>,
        budget: &ReadBudget,
    ) -> io::Result<Vec<AgentChatMessage>> {
        self.begin_batch();
        let (records, mut stats) =
            read_tail_page(path, tail_record_limit(self.max_messages), end, budget)?;
        let mut cursor = stats.offset;
        if !stats.tail_page_ready {
            stats.hit_budget = true;
            stats.offset = stats.snapshot_end;
            self.last_read = stats;
            return Ok(Vec::new());
        }
        if !records.is_empty() && budget.expired() {
            stats.hit_budget = true;
            cursor = stats.snapshot_end;
        } else {
            for record in records.iter().rev() {
                let before = self.next_message;
                self.parse_record(format, &record.line);
                for id in before + 1..=self.next_message {
                    if let Some(message) = self.messages.get_mut(&id) {
                        message.source_start = record.start;
                        message.source_end = record.end;
                    }
                }
                stats.decoded_records += 1;
                cursor = record.start;
                if budget.expired() {
                    stats.hit_budget = true;
                }
            }
        }
        if let Some(first) = self.visible.front().and_then(|id| self.messages.get(id))
            && (!stats.hit_budget || cursor == 0 || first.source_start < cursor)
        {
            cursor = first.source_start;
        }
        stats.decode_committed = stats.tail_page_ready
            && stats.decoded_records == records.len()
            && (!records.is_empty() || !stats.hit_budget);
        stats.offset = cursor;
        self.last_read = stats;
        Ok(self.output_messages())
    }
    pub fn adopt_prime_cursor(&mut self) -> bool {
        if !self.last_read.decode_committed {
            return false;
        }
        self.read_state.reset(self.last_read.safe_offset);
        self.completions.clear();
        true
    }
}
fn limit(text: &str) -> String {
    let text = text.trim();
    if text.len() <= TEXT_MAX {
        return text.into();
    }
    let mut cut = TEXT_MAX;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}…", &text[..cut])
}
fn masked(text: &str) -> String {
    limit(&mask_secrets(text.trim()))
}
fn raw_text(raw: &RawField) -> String {
    raw.0
        .as_ref()
        .map(|v| raw_text_bytes(v))
        .unwrap_or_default()
}
fn raw_text_bytes(raw: &[u8]) -> String {
    if let Ok(text) = proto::decode_wire::<String>(raw) {
        return text.trim().into();
    }
    let Ok(Some(items)) = proto::wire::decode_go_json_array(raw) else {
        return String::new();
    };
    let mut maps = Vec::new();
    for item in items {
        match proto::wire::decode_go_json_object(&item) {
            Ok(map) => maps.push(map.unwrap_or_default()),
            Err(_) => return String::new(),
        }
    }
    maps.iter()
        .map(|map| {
            let text = map
                .get("text")
                .map(|v| raw_text_bytes(v))
                .unwrap_or_default();
            if text.is_empty() {
                map.get("content")
                    .map(|v| raw_text_bytes(v))
                    .unwrap_or_default()
            } else {
                text
            }
        })
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .into()
}
fn summarize_json(raw: &RawField) -> String {
    let Some(raw) = &raw.0 else {
        return String::new();
    };
    if raw.as_slice() == b"null" {
        return String::new();
    }
    let value = decode_go_value(raw);
    let text = raw_text_bytes(raw);
    if !text.is_empty() {
        return masked(&text);
    }
    let bytes = match value {
        Ok(v) => go_value_json(&v).into_bytes(),
        Err(_) => return masked(&String::from_utf8_lossy(raw)),
    };
    let text = if bytes.len() > 1200 {
        let mut s = String::from_utf8_lossy(&bytes[..1200]).into_owned();
        s.push_str("...");
        s
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    };
    masked(&text)
}

// Explicit raw null differs from absent for arguments-vs-input fallback.
#[derive(Clone, Default)]
struct RawField(Option<Vec<u8>>);
#[derive(Default, Deserialize)]
#[serde(default)]
struct AnthropicMessage {
    role: String,
    #[serde(skip)]
    content: RawField,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct AnthropicRecord {
    #[serde(rename = "type")]
    kind: String,
    timestamp: String,
    #[serde(rename = "isSidechain")]
    sidechain: bool,
    message: AnthropicMessage,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Block {
    #[serde(rename = "type")]
    kind: String,
    text: String,
    thinking: String,
    id: String,
    name: String,
    #[serde(skip)]
    input: RawField,
    tool_use_id: String,
    #[serde(skip)]
    content: RawField,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct CodexRecord {
    #[serde(rename = "type")]
    kind: String,
    timestamp: String,
    #[serde(skip)]
    payload: RawField,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct CodexMetadata {
    content_item_kinds: Option<Vec<String>>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct CodexPayload {
    #[serde(rename = "type")]
    kind: String,
    phase: String,
    channel: String,
    role: String,
    text: String,
    message: String,
    name: String,
    call_id: String,
    #[serde(skip)]
    arguments: RawField,
    #[serde(skip)]
    input: RawField,
    #[serde(skip)]
    output: RawField,
    #[serde(skip)]
    content: RawField,
    #[serde(skip)]
    summary: RawField,
    turn_id: String,
    completed_at: f64,
    #[serde(skip)]
    last_agent_message: RawField,
    internal_chat_message_metadata_passthrough: Option<CodexMetadata>,
}
macro_rules! schema {($name:literal,$($field:literal=>$kind:literal),*$(,)?)=>{Schema{name:$name,fields:&[$(Field{name:$field,kind:$kind}),*]}};}
const SCHEMAS: &[Schema] = &[
    schema!("TranscriptAnthropicMessage","role"=>"string"),
    schema!("TranscriptAnthropicRecord","type"=>"string","timestamp"=>"string","isSidechain"=>"bool","message"=>"TranscriptAnthropicMessage"),
    schema!("TranscriptBlock","type"=>"string","text"=>"string","thinking"=>"string","id"=>"string","name"=>"string","tool_use_id"=>"string"),
    schema!("TranscriptCodexRecord","type"=>"string","timestamp"=>"string"),
    schema!("TranscriptCodexMetadata","content_item_kinds"=>"[]string"),
    schema!("TranscriptCodexPayload","type"=>"string","phase"=>"string","channel"=>"string","role"=>"string","text"=>"string","message"=>"string","name"=>"string","call_id"=>"string","turn_id"=>"string","completed_at"=>"float64","internal_chat_message_metadata_passthrough"=>"*TranscriptCodexMetadata"),
];
macro_rules! wire {
    ($ty:ty,$name:literal) => {
        impl GoWire for $ty {
            const GO_TYPE: &'static str = $name;
            const SCHEMAS: &'static [Schema] = SCHEMAS;
        }
    };
}
wire!(AnthropicRecord, "TranscriptAnthropicRecord");
wire!(Block, "TranscriptBlock");
wire!(CodexRecord, "TranscriptCodexRecord");
wire!(CodexPayload, "TranscriptCodexPayload");
fn blocks(raw: &RawField) -> Option<Vec<Block>> {
    let raw = raw.0.as_ref()?;
    if let Ok(items) = proto::wire::decode_go_json_array(raw) {
        let mut result = Vec::new();
        for bytes in items.unwrap_or_default() {
            result.push(decode_block(&bytes).ok()?);
        }
        return Some(result);
    }
    let text: String = proto::decode_wire::<TextValue>(raw).ok()?.0;
    (!text.trim().is_empty()).then(|| {
        vec![Block {
            kind: "text".into(),
            text,
            ..Default::default()
        }]
    })
}
#[derive(Deserialize)]
#[serde(transparent)]
struct TextValue(String);
impl GoWire for TextValue {
    const GO_TYPE: &'static str = "string";
}
fn raw_field(members: &[(String, Vec<u8>)], key: &str) -> RawField {
    RawField(proto::wire::last_go_raw_field(members, key).map(<[u8]>::to_vec))
}
fn decode_anthropic(bytes: &[u8]) -> Result<AnthropicRecord, serde_json::Error> {
    let mut result = proto::decode_wire::<AnthropicRecord>(bytes)?;
    // Go merges duplicate nested structs, so each non-null message object can
    // overwrite its content while an absent content retains the earlier one.
    for member in proto::wire::decode_go_json_members(bytes)?.unwrap_or_default() {
        if let Some(message) =
            proto::wire::last_go_raw_field(std::slice::from_ref(&member), "message")
            && let Some(fields) = proto::wire::decode_go_json_members(message)?
            && let Some(content) = proto::wire::last_go_raw_field(&fields, "content")
        {
            result.message.content = RawField(Some(content.to_vec()));
        }
    }
    Ok(result)
}
fn decode_block(bytes: &[u8]) -> Result<Block, serde_json::Error> {
    let mut result = proto::decode_wire::<Block>(bytes)?;
    let fields = proto::wire::decode_go_json_members(bytes)?.unwrap_or_default();
    result.input = raw_field(&fields, "input");
    result.content = raw_field(&fields, "content");
    Ok(result)
}
fn decode_codex_record(bytes: &[u8]) -> Result<CodexRecord, serde_json::Error> {
    let mut result = proto::decode_wire::<CodexRecord>(bytes)?;
    let fields = proto::wire::decode_go_json_members(bytes)?.unwrap_or_default();
    result.payload = raw_field(&fields, "payload");
    Ok(result)
}
fn decode_codex_payload(bytes: &[u8]) -> Result<CodexPayload, serde_json::Error> {
    let mut result = proto::decode_wire::<CodexPayload>(bytes)?;
    let fields = proto::wire::decode_go_json_members(bytes)?.unwrap_or_default();
    result.arguments = raw_field(&fields, "arguments");
    result.input = raw_field(&fields, "input");
    result.output = raw_field(&fields, "output");
    result.content = raw_field(&fields, "content");
    result.summary = raw_field(&fields, "summary");
    result.last_agent_message = raw_field(&fields, "last_agent_message");
    Ok(result)
}

impl ParseState {
    fn anthropic(&mut self, format: ProviderFormat, record: AnthropicRecord) {
        if format == ProviderFormat::Claude && !matches!(record.kind.as_str(), "user" | "assistant")
            || format == ProviderFormat::CommandCode && record.kind != "message"
        {
            return;
        }
        let Some(blocks) = blocks(&record.message.content) else {
            return;
        };
        let role = if record.message.role.is_empty() && format == ProviderFormat::Claude {
            &record.kind
        } else {
            &record.message.role
        };
        match role.as_str() {
            "user" => {
                let mut text = Vec::new();
                for block in blocks {
                    match block.kind.as_str() {
                        "text" if !block.text.trim().is_empty() => text.push(block.text),
                        "tool_result" => {
                            let result = raw_text(&block.content);
                            let result = if result.is_empty() {
                                block.text
                            } else {
                                result
                            };
                            self.attach_result(&block.tool_use_id, result);
                        }
                        _ => {}
                    }
                }
                let text = text.join("\n").trim().to_owned();
                if text.is_empty()
                    || text.contains("<command-name>")
                    || text.contains("<system-reminder>")
                {
                    return;
                }
                self.append(
                    AgentChatMessage {
                        role: "user".into(),
                        kind: "text".into(),
                        text: masked(&text),
                        ts: record.timestamp,
                        ..Default::default()
                    },
                    false,
                );
            }
            "assistant" => {
                let (mut text, mut thinking, mut tools) = (Vec::new(), Vec::new(), Vec::new());
                for block in blocks {
                    match block.kind.as_str() {
                        "text" if !block.text.trim().is_empty() => text.push(block.text),
                        "thinking" if !block.thinking.trim().is_empty() => {
                            thinking.push(masked(&block.thinking))
                        }
                        "tool_use" => tools.push(AgentChatTool {
                            id: block.id,
                            name: masked(&block.name),
                            input: summarize_json(&block.input),
                            result: String::new(),
                        }),
                        _ => {}
                    }
                }
                let mut text = text.join("\n").trim().to_owned();
                let sidechain = format == ProviderFormat::Claude && record.sidechain;
                if sidechain {
                    if !text.is_empty() {
                        thinking.push(masked(&text));
                    }
                    text.clear();
                }
                if text.is_empty() && thinking.is_empty() && tools.is_empty() {
                    return;
                }
                let kind = if sidechain {
                    "sidechain"
                } else if text.is_empty() && !tools.is_empty() {
                    "tool"
                } else if text.is_empty() {
                    "thinking"
                } else {
                    "text"
                };
                self.append(
                    AgentChatMessage {
                        role: "assistant".into(),
                        kind: kind.into(),
                        text: masked(&text),
                        thinking,
                        tools,
                        ts: record.timestamp,
                        ..Default::default()
                    },
                    true,
                );
            }
            _ => {}
        }
    }
    fn codex(&mut self, record_type: &str, ts: &str, payload: CodexPayload) {
        match record_type {
            "response_item" => match payload.kind.as_str() {
                "message" => {
                    let role = &payload.role;
                    if role != "user" && role != "assistant" {
                        return;
                    }
                    let mut content = payload.content.clone();
                    if role == "user"
                        && let Some(metadata) = &payload.internal_chat_message_metadata_passthrough
                        && let Some(kinds) = metadata
                            .content_item_kinds
                            .as_ref()
                            .filter(|v| !v.is_empty())
                    {
                        content = filter_codex_user_content(&content, kinds);
                    }
                    let (text, thinking, tools) = parse_codex_content(&content);
                    if role == "user" && text.is_empty()
                        || text.is_empty() && thinking.is_empty() && tools.is_empty()
                    {
                        return;
                    }
                    let kind = if role == "assistant" && text.is_empty() && !tools.is_empty() {
                        "tool"
                    } else if role == "assistant" && text.is_empty() {
                        "thinking"
                    } else {
                        "text"
                    };
                    self.append(
                        AgentChatMessage {
                            role: role.clone(),
                            kind: kind.into(),
                            presentation: presentation(&payload, role).into(),
                            text: masked(&text),
                            thinking,
                            tools,
                            ts: ts.into(),
                            ..Default::default()
                        },
                        true,
                    );
                }
                "reasoning" => {
                    let thinking = codex_summary(&payload.summary);
                    if !thinking.is_empty() {
                        self.append(
                            AgentChatMessage {
                                role: "assistant".into(),
                                kind: "thinking".into(),
                                thinking,
                                ts: ts.into(),
                                ..Default::default()
                            },
                            true,
                        );
                    }
                }
                "function_call" | "custom_tool_call" => {
                    let input = if payload.arguments.0.is_some() {
                        &payload.arguments
                    } else {
                        &payload.input
                    };
                    if payload.name.is_empty() && input.0.is_none() {
                        return;
                    }
                    self.append(
                        AgentChatMessage {
                            role: "assistant".into(),
                            kind: "tool".into(),
                            tools: vec![AgentChatTool {
                                id: payload.call_id,
                                name: masked(&payload.name),
                                input: summarize_json(input),
                                result: String::new(),
                            }],
                            ts: ts.into(),
                            ..Default::default()
                        },
                        true,
                    );
                }
                "function_call_output" | "custom_tool_call_output" => {
                    let result = raw_text(&payload.output);
                    let result = if result.is_empty() {
                        raw_text(&payload.content)
                    } else {
                        result
                    };
                    self.attach_result(&payload.call_id, result);
                }
                _ => {}
            },
            "event_msg" => match payload.kind.as_str() {
                "user_message" | "agent_message" | "agent_reasoning" => {
                    let text = if payload.message.is_empty() {
                        &payload.text
                    } else {
                        &payload.message
                    };
                    if text.trim().is_empty() {
                        return;
                    }
                    let role = if payload.kind == "user_message" {
                        "user"
                    } else {
                        "assistant"
                    };
                    let thinking = payload.kind == "agent_reasoning";
                    self.append(
                        AgentChatMessage {
                            role: role.into(),
                            kind: if thinking { "thinking" } else { "text" }.into(),
                            presentation: if !thinking {
                                presentation(&payload, role).into()
                            } else {
                                String::new()
                            },
                            text: if thinking {
                                String::new()
                            } else {
                                masked(text)
                            },
                            thinking: if thinking {
                                vec![masked(text)]
                            } else {
                                Vec::new()
                            },
                            ts: ts.into(),
                            ..Default::default()
                        },
                        true,
                    );
                }
                "task_complete" => {
                    let mut at = ts.trim().to_owned();
                    if at.is_empty() && payload.completed_at > 0.0 {
                        let seconds = if payload.completed_at >= 1e12 {
                            payload.completed_at / 1000.0
                        } else {
                            payload.completed_at
                        };
                        if seconds.is_finite() && seconds <= u64::MAX as f64 {
                            let time = Duration::try_from_secs_f64(seconds)
                                .ok()
                                .and_then(|duration| SystemTime::UNIX_EPOCH.checked_add(duration));
                            if let Some(time) = time {
                                at = proto::time::format_with_offset(time, 0, false)
                                    .unwrap_or_default();
                            }
                        }
                    }
                    let turn_id = payload.turn_id.trim().to_owned();
                    if at.is_empty() && turn_id.is_empty() {
                        return;
                    }
                    let last = payload
                        .last_agent_message
                        .0
                        .as_ref()
                        .and_then(|v| proto::decode_wire::<String>(v).ok());
                    self.completions.push(CodexCompletion {
                        turn_id,
                        at,
                        last_agent_message: last.as_deref().map(masked).unwrap_or_default(),
                    });
                    if self.completions.len() > 16 {
                        self.completions.remove(0);
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
}
fn presentation(payload: &CodexPayload, role: &str) -> &'static str {
    if role != "assistant" {
        ""
    } else if payload.channel == "analysis" || payload.phase == "analysis" {
        "hidden"
    } else if payload.phase == "commentary" || payload.channel == "commentary" {
        "progress"
    } else if payload.phase == "final_answer" || payload.channel == "final" {
        "answer"
    } else {
        "unclassified"
    }
}
fn filter_codex_user_content(raw: &RawField, kinds: &[String]) -> RawField {
    let Some(bytes) = &raw.0 else {
        return RawField::default();
    };
    if let Ok(items) = proto::wire::decode_go_json_array(bytes) {
        let items = items.unwrap_or_default();
        if items.len() != kinds.len() {
            return RawField::default();
        }
        let filtered: Vec<_> = items
            .into_iter()
            .zip(kinds)
            .filter(|(_, k)| k.starts_with("user."))
            .map(|(raw, _)| raw)
            .collect();
        if filtered.is_empty() {
            return RawField::default();
        }
        let mut result = vec![b'['];
        for (i, item) in filtered.iter().enumerate() {
            if i > 0 {
                result.push(b',');
            }
            result.extend(item);
        }
        result.push(b']');
        RawField(Some(result))
    } else if kinds.len() == 1 && kinds[0].starts_with("user.") {
        raw.clone()
    } else {
        RawField::default()
    }
}
fn raw_maps(raw: &RawField) -> Option<Vec<BTreeMap<String, Vec<u8>>>> {
    let bytes = raw.0.as_ref()?;
    let items = proto::wire::decode_go_json_array(bytes)
        .ok()?
        .unwrap_or_default();
    let mut maps = Vec::new();
    for item in items {
        maps.push(
            proto::wire::decode_go_json_object(&item)
                .ok()?
                .unwrap_or_default(),
        );
    }
    Some(maps)
}
fn map_text(map: &BTreeMap<String, Vec<u8>>, key: &str) -> String {
    map.get(key).map(|v| raw_text_bytes(v)).unwrap_or_default()
}
fn map_string(map: &BTreeMap<String, Vec<u8>>, key: &str) -> String {
    map.get(key)
        .and_then(|v| proto::decode_wire::<String>(v).ok())
        .unwrap_or_default()
}
fn codex_summary(raw: &RawField) -> Vec<String> {
    if let Some(maps) = raw_maps(raw) {
        maps.iter()
            .map(|map| {
                let text = map_text(map, "text");
                if text.is_empty() {
                    map_text(map, "summary_text")
                } else {
                    text
                }
            })
            .filter(|s| !s.is_empty())
            .map(|s| masked(&s))
            .collect()
    } else {
        let text = raw_text(raw);
        if text.is_empty() {
            Vec::new()
        } else {
            vec![masked(&text)]
        }
    }
}
fn parse_codex_content(raw: &RawField) -> (String, Vec<String>, Vec<AgentChatTool>) {
    let Some(maps) = raw_maps(raw) else {
        return (raw_text(raw), Vec::new(), Vec::new());
    };
    let (mut texts, mut thinking, mut tools) = (Vec::new(), Vec::new(), Vec::new());
    for map in maps {
        match map_string(&map, "type").as_str() {
            "output_text" | "input_text" | "text" => {
                let text = map_text(&map, "text");
                let text = if text.is_empty() {
                    map_text(&map, "content")
                } else {
                    text
                };
                if !text.is_empty() {
                    texts.push(text);
                }
            }
            "summary_text" | "reasoning" | "reasoning_summary" => {
                let text = map_text(&map, "text");
                if !text.is_empty() {
                    thinking.push(masked(&text));
                }
            }
            "function_call" | "custom_tool_call" => {
                let input = RawField(map.get("arguments").or_else(|| map.get("input")).cloned());
                tools.push(AgentChatTool {
                    id: map_string(&map, "call_id"),
                    name: masked(&map_string(&map, "name")),
                    input: summarize_json(&input),
                    result: String::new(),
                });
            }
            _ => {}
        }
    }
    (texts.join("\n").trim().into(), thinking, tools)
}

// json.Unmarshal into interface{} uses float64 for every number; summaries
// therefore need different number handling than RawMessage or integer DTOs.
fn decode_go_value(bytes: &[u8]) -> Result<Value, serde_json::Error> {
    let first = bytes
        .iter()
        .copied()
        .find(|b| !matches!(b, b' ' | b'\n' | b'\r' | b'\t'));
    match first {
        Some(b'n') => {
            let parsed: Option<String> = serde_json::from_slice(bytes)?;
            if parsed.is_none() {
                Ok(Value::Null)
            } else {
                unreachable!()
            }
        }
        Some(b'"') => proto::decode_wire::<String>(bytes).map(Value::String),
        Some(b't' | b'f') => proto::decode_wire::<bool>(bytes).map(Value::Bool),
        Some(b'[') => {
            let mut out = Vec::new();
            for item in proto::wire::decode_go_json_array(bytes)?.unwrap_or_default() {
                out.push(decode_go_value(&item)?);
            }
            Ok(Value::Array(out))
        }
        Some(b'{') => {
            let mut out = serde_json::Map::new();
            for (key, raw) in proto::wire::decode_go_json_object(bytes)?.unwrap_or_default() {
                out.insert(key, decode_go_value(&raw)?);
            }
            Ok(Value::Object(out))
        }
        _ => proto::decode_wire::<f64>(bytes)
            .map(|v| Value::Number(serde_json::Number::from_f64(v).expect("finite JSON float"))),
    }
}
fn go_value_json(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::String(s) => String::from_utf8(proto::provider::to_go_json(s).expect("JSON string"))
            .expect("UTF8 JSON string"),
        Value::Number(n) => {
            let value = n.as_f64().expect("JSON number");
            if value != 0.0 && (value.abs() < 1e-6 || value.abs() >= 1e21) {
                let text = format!("{value:e}");
                let (mantissa, exponent) = text.split_once('e').expect("scientific float");
                let exponent = exponent.parse::<i32>().expect("float exponent");
                format!("{mantissa}e{exponent:+}")
            } else {
                value.to_string()
            }
        }
        Value::Array(items) => format!(
            "[{}]",
            items
                .iter()
                .map(go_value_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        Value::Object(map) => format!(
            "{{{}}}",
            map.iter()
                .map(|(k, v)| format!(
                    "{}:{}",
                    go_value_json(&Value::String(k.clone())),
                    go_value_json(v)
                ))
                .collect::<Vec<_>>()
                .join(",")
        ),
    }
}
