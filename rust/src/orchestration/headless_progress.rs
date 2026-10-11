//! Rust-only, allowlisted headless progress. Never serialize provider records.
//! A result is accounting evidence, not process-exit evidence.
use crate::{
    process::{ExitOutcome, ProcessOutput},
    storage::mask_secrets,
};
use serde::{Deserialize, Serialize, de::IgnoredAny};
use serde_json::value::RawValue;
use std::collections::{BTreeMap, BTreeSet};

pub const PROGRESS_VERSION: u32 = 1;
pub const MAX_PROGRESS_BYTES: usize = 16 * 1024;
pub const MAX_RECORD_BYTES: usize = 1024 * 1024;
pub const MAX_ACTIVE_TOOLS: usize = 16;
const MAX_RESPONSES: usize = 4096;
// One main response owns text at a time; never MAX_RESPONSES copies of raw text.
const MAX_RESPONSE_TEXT_BYTES: usize = 64 * 1024;
const MAX_RESPONSE_BLOCKS: usize = 256;
const MAX_TOKENS: u64 = 1_000_000_000;
const MAX_TURNS: u64 = 1_000_000;
const MAX_COST: f64 = 1_000_000.0;
const MAX_ELAPSED_MS: u64 = 366 * 24 * 60 * 60 * 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProgressState {
    Running,
    Finishing,
    Completed,
    Failed,
    Canceled,
    Disconnected,
}
impl ProgressState {
    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Running | Self::Finishing)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CostKind {
    Estimate,
    Result,
    Unavailable,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CostCoverage {
    ObservedMain,
    Partial,
    CliResult,
    Unavailable,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgressCost {
    pub amount_usd: Option<f64>,
    pub currency: String,
    pub kind: CostKind,
    pub coverage: CostCoverage,
}
impl Default for ProgressCost {
    fn default() -> Self {
        Self {
            amount_usd: None,
            currency: "USD".into(),
            kind: CostKind::Unavailable,
            coverage: CostCoverage::Unavailable,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveTool {
    pub id: String,
    pub name: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeadlessProgress {
    pub version: u32,
    pub run_id: String,
    pub sequence: u64,
    /// Filled only by the Hub from confirmed spawn metadata. Never trusted from a wrapper.
    #[serde(default)]
    pub billing_mode: String,
    pub observed_turns: u64,
    pub result_turns: Option<u64>,
    pub active_tools: Vec<ActiveTool>,
    pub started_at_ms: i64,
    pub elapsed_ms: u64,
    pub ended_at_ms: Option<i64>,
    pub cost: ProgressCost,
    /// Masked before truncation, control-stripped, then HTML escaped.
    pub last_assistant: String,
    pub state: ProgressState,
    pub data_incomplete: bool,
    pub final_cost_missing: bool,
}
impl HeadlessProgress {
    pub fn new(run_id: String, started_at_ms: i64) -> Self {
        Self {
            version: PROGRESS_VERSION,
            run_id,
            sequence: 0,
            billing_mode: String::new(),
            observed_turns: 0,
            result_turns: None,
            active_tools: vec![],
            started_at_ms,
            elapsed_ms: 0,
            ended_at_ms: None,
            cost: ProgressCost::default(),
            last_assistant: String::new(),
            state: ProgressState::Running,
            data_incomplete: false,
            final_cost_missing: false,
        }
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.version != PROGRESS_VERSION
            || !valid_id(&self.run_id)
            || self.sequence > 9_007_199_254_740_991
            || self.observed_turns > MAX_TURNS
            || self.result_turns.is_some_and(|v| v > MAX_TURNS)
            || self.started_at_ms < 0
            || self.started_at_ms > 9_007_199_254_740_991
            || self.elapsed_ms > MAX_ELAPSED_MS
            || self
                .ended_at_ms
                .is_some_and(|v| v < self.started_at_ms || v > 9_007_199_254_740_991)
            || self.active_tools.len() > MAX_ACTIVE_TOOLS
            || self.last_assistant.len() > 3072
            || !safe_wire_text(&self.last_assistant)
            || self.cost.currency != "USD"
            || !matches!(self.billing_mode.as_str(), "" | "plan-login" | "api-key")
            || self.cost.amount_usd.is_some_and(|n| !valid_cost(n))
        {
            return Err("invalid headless progress");
        }
        let mut ids = BTreeSet::new();
        for tool in &self.active_tools {
            if !valid_id(&tool.id)
                || !ids.insert(&tool.id)
                || tool.name.is_empty()
                || tool.name.len() > 384
                || !safe_wire_text(&tool.name)
            {
                return Err("invalid headless tool");
            }
        }
        if self.state.is_terminal() && (!self.active_tools.is_empty() || self.ended_at_ms.is_none())
        {
            return Err("invalid headless lifecycle");
        }
        if matches!(self.cost.kind, CostKind::Unavailable) != self.cost.amount_usd.is_none()
            || (self.cost.kind == CostKind::Result && self.cost.coverage != CostCoverage::CliResult)
            || (self.cost.kind == CostKind::Estimate
                && !matches!(
                    self.cost.coverage,
                    CostCoverage::ObservedMain | CostCoverage::Partial
                ))
            || (self.cost.kind == CostKind::Unavailable
                && self.cost.coverage != CostCoverage::Unavailable)
        {
            return Err("invalid headless cost");
        }
        Ok(())
    }
    pub fn terminal_line(&self) -> String {
        let tool = self
            .active_tools
            .last()
            .map(|t| t.name.as_str())
            .unwrap_or("none");
        let turns = self.result_turns.unwrap_or(self.observed_turns);
        let cost = self
            .cost
            .amount_usd
            .map(|v| {
                format!(
                    "{v:.6} USD ({})",
                    match self.cost.kind {
                        CostKind::Result => "CLI result",
                        _ => "observed estimate",
                    }
                )
            })
            .unwrap_or_else(|| "unavailable".into());
        let state = match self.state {
            ProgressState::Running => "running",
            ProgressState::Finishing => "finishing",
            ProgressState::Completed => "completed",
            ProgressState::Failed => "failed",
            ProgressState::Canceled => "canceled",
            ProgressState::Disconnected => "disconnected",
        };
        format!(
            "[progress] {state} turns={turns} tool={tool} active={} elapsed={}ms cost={cost}{}\r\n",
            self.active_tools.len(),
            self.elapsed_ms,
            if self.last_assistant.is_empty() {
                String::new()
            } else {
                format!(" assistant={}", self.last_assistant)
            }
        )
    }
}
pub fn decode_progress(bytes: &[u8]) -> Result<HeadlessProgress, &'static str> {
    if bytes.len() > MAX_PROGRESS_BYTES {
        return Err("headless progress too large");
    }
    let progress: HeadlessProgress =
        serde_json::from_slice(bytes).map_err(|_| "invalid headless progress")?;
    progress.validate()?;
    Ok(progress)
}
fn valid_cost(n: f64) -> bool {
    n.is_finite() && (0.0..=MAX_COST).contains(&n)
}
fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_:-.".contains(&b))
        && mask_secrets(value) == value
}
fn safe_wire_text(value: &str) -> bool {
    let plain = value
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&gt;", ">")
        .replace("&lt;", "<")
        .replace("&amp;", "&");
    safe_excerpt(&plain, usize::MAX) == value
}
/// Never truncate before the full-text mask: a secret can straddle the limit.
pub fn safe_excerpt(text: &str, limit: usize) -> String {
    let masked = mask_secrets(text);
    let mut clean: String = masked
        .chars()
        .filter_map(|c| match c {
            '\n' | '\r' | '\t' => Some(' '),
            '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' => None,
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect();
    // Removing a control byte can join two pieces into a recognizable secret.
    // Re-mask that complete normalized value before applying any length limit.
    clean = mask_secrets(&clean);
    if clean.len() > limit {
        let mut end = limit;
        while !clean.is_char_boundary(end) {
            end -= 1;
        }
        clean.truncate(end);
    }
    clean
        .trim()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

// Provider objects intentionally have no Debug/Serialize. Inspect the envelope
// before decoding a relevant body: a valid system message can be a string and
// an assistant API error body must never enter the text accumulator.
#[derive(Deserialize)]
struct RecordHeader {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    parent_tool_use_id: Option<String>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct AssistantRecord<'a> {
    #[serde(borrow)]
    message: Option<&'a RawValue>,
    uuid: Option<String>,
    error: Option<IgnoredAny>,
    is_api_error_message: bool,
    is_error: bool,
    api_error: Option<IgnoredAny>,
    aborted: bool,
    is_meta: bool,
    is_virtual: bool,
    historical: bool,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct UserRecord<'a> {
    #[serde(borrow)]
    message: Option<UserMessage<'a>>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct UserMessage<'a> {
    #[serde(borrow)]
    content: Option<&'a RawValue>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct StreamRecord<'a> {
    #[serde(borrow)]
    event: Option<&'a RawValue>,
    api_message_id: Option<String>,
    abandoned_blocks: Option<IgnoredAny>,
    text_runs_joined: bool,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct ResultRecord {
    subtype: String,
    is_error: bool,
    total_cost_usd: Option<f64>,
    num_turns: Option<u64>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct ProviderMessage {
    id: String,
    model: String,
    content: Vec<Block>,
    usage: Option<Usage>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Block {
    #[serde(rename = "type")]
    kind: String,
    id: String,
    name: String,
    text: String,
    tool_use_id: String,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct StreamEvent {
    message: Option<ProviderMessage>,
    usage: Option<Usage>,
    content_block: Option<Block>,
    index: Option<u64>,
    delta: Option<MessageDelta>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct MessageDelta {
    stop_reason: Option<String>,
}
// There is deliberately no Debug/Serialize on either private text holder.
struct BufferedBlock {
    is_text: bool,
    closed: bool,
    text: Option<String>,
    record_id: Option<String>,
}
struct PendingResponse {
    id: String,
    blocks: BTreeMap<u64, BufferedBlock>,
    current_block: Option<u64>,
    text_bytes: usize,
    terminal_reason: bool,
    suppressed: bool,
}
impl PendingResponse {
    fn new(id: String, suppressed: bool) -> Self {
        Self {
            id,
            blocks: BTreeMap::new(),
            current_block: None,
            text_bytes: 0,
            terminal_reason: false,
            suppressed,
        }
    }
    fn suppress(&mut self) {
        self.suppressed = true;
        // Drop the allocation as well as its content on every rejection path.
        self.blocks.clear();
        self.current_block = None;
        self.text_bytes = 0;
    }
    fn start_block(&mut self, index: u64, block: &Block) {
        if self.suppressed {
            return;
        }
        if self.blocks.len() >= MAX_RESPONSE_BLOCKS
            || index >= MAX_RESPONSE_BLOCKS as u64
            || index != self.blocks.len() as u64
            || self.blocks.contains_key(&index)
            || self
                .current_block
                .and_then(|i| self.blocks.get(&i))
                .is_some_and(|b| !b.closed)
        {
            self.suppress();
            return;
        }
        self.blocks.insert(
            index,
            BufferedBlock {
                is_text: block.kind == "text",
                closed: false,
                text: None,
                record_id: None,
            },
        );
        self.current_block = Some(index);
    }
    fn stop_block(&mut self, index: u64) {
        if self.suppressed {
            return;
        }
        if let Some(block) = self.blocks.get_mut(&index) {
            block.closed = true;
        } else {
            self.suppress();
        }
    }
    fn assistant(&mut self, uuid: Option<&str>, blocks: &[Block]) {
        if self.suppressed || !blocks.iter().any(|b| b.kind == "text") {
            return;
        }
        // The verified CLI emits one assistant record per content_block_stop.
        // Joined/non-streaming/plugin shapes have no proven index mapping here.
        let Some(uuid) = uuid.filter(|id| valid_id(id)) else {
            self.suppress();
            return;
        };
        if blocks.len() != 1 || blocks[0].kind != "text" {
            self.suppress();
            return;
        }
        let text = &blocks[0].text;
        if let Some(previous) = self
            .blocks
            .values()
            .find(|b| b.record_id.as_deref() == Some(uuid))
        {
            // A delayed identical UUID is a replay, even after another block
            // started. A changed body under the same UUID is not completion proof.
            if previous.text.as_ref() != Some(text) {
                self.suppress();
            }
            return;
        }
        let Some(index) = self.current_block else {
            self.suppress();
            return;
        };
        let Some(block) = self.blocks.get_mut(&index) else {
            self.suppress();
            return;
        };
        if !block.is_text
            || block.record_id.is_some()
            || text.len() > MAX_RESPONSE_TEXT_BYTES.saturating_sub(self.text_bytes)
        {
            self.suppress();
            return;
        }
        self.text_bytes += text.len();
        block.text = Some(text.clone());
        block.record_id = Some(uuid.into());
    }
    // Claude Code 2.1.295 emits assistant records before each block-stop event;
    // only a terminal message_delta plus closed/accounted blocks and message_stop
    // proves the whole response. Its synthesized interrupted stops alone do not.
    // The official SDK schema also documents abandoned_blocks/text_runs_joined.
    fn finish(self) -> Option<String> {
        if self.suppressed
            || !self.terminal_reason
            || self.blocks.is_empty()
            || self
                .blocks
                .values()
                .any(|b| !b.closed || b.is_text && b.text.is_none())
        {
            return None;
        }
        let mut complete = String::with_capacity(self.text_bytes);
        for block in self.blocks.into_values() {
            if let Some(text) = block.text {
                complete.push_str(&text);
            }
        }
        (!complete.trim().is_empty()).then(|| safe_excerpt(&complete, 512))
    }
}
#[derive(Clone, Default, Deserialize)]
#[serde(default)]
struct Usage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
    cache_creation: Option<CacheCreation>,
    service_tier: Option<String>,
    inference_geo: Option<String>,
    speed: Option<String>,
    server_tool_use: Option<ServerToolUse>,
}
#[derive(Clone, Default, Deserialize)]
struct ServerToolUse {}
#[derive(Clone, Default, Deserialize)]
#[serde(default)]
struct CacheCreation {
    ephemeral_5m_input_tokens: Option<u64>,
    ephemeral_1h_input_tokens: Option<u64>,
}
#[derive(Default)]
struct Response {
    model: String,
    usage: Usage,
    observed: bool,
    partial: bool,
    text_finished: bool,
}
impl Usage {
    fn merge(&mut self, newer: Usage, accept_output: bool) {
        // These are cumulative observations for one response. Replace the
        // previous value (never add); a delayed duplicate cannot roll it back.
        if newer.input_tokens.is_some() {
            self.input_tokens = self.input_tokens.max(newer.input_tokens);
        }
        if accept_output && newer.output_tokens.is_some() {
            self.output_tokens = self.output_tokens.max(newer.output_tokens);
        }
        if newer.cache_read_input_tokens.is_some() {
            self.cache_read_input_tokens = self
                .cache_read_input_tokens
                .max(newer.cache_read_input_tokens);
        }
        if newer.cache_creation_input_tokens.is_some() {
            self.cache_creation_input_tokens = self
                .cache_creation_input_tokens
                .max(newer.cache_creation_input_tokens);
        }
        if newer.cache_creation.is_some() {
            self.cache_creation = newer.cache_creation;
        }
        if newer.service_tier.is_some() {
            self.service_tier = newer.service_tier.map(|v| {
                if v == "standard" {
                    v
                } else {
                    "unsupported".into()
                }
            });
        }
        if newer.inference_geo.is_some() {
            self.inference_geo = newer.inference_geo.map(|v| {
                if v == "global" {
                    v
                } else {
                    "unsupported".into()
                }
            });
        }
        if newer.speed.is_some() {
            self.speed = newer.speed.map(|v| {
                if v == "standard" {
                    v
                } else {
                    "unsupported".into()
                }
            });
        }
        if newer.server_tool_use.is_some() {
            self.server_tool_use = newer.server_tool_use;
        }
    }
}
/// Exact serving IDs only, verified 2026-10-10. USD per million tokens.
/// https://platform.claude.com/docs/en/about-claude/pricing
/// https://platform.claude.com/docs/en/models/sonnet-4-5/overview
/// https://platform.claude.com/docs/en/models/sonnet-4-6/overview
/// https://platform.claude.com/docs/en/models/opus-4-6/overview
const PRICES: &[(&str, [f64; 5])] = &[
    ("claude-sonnet-4-5-20250929", [3.0, 15.0, 0.30, 3.75, 6.0]),
    ("claude-sonnet-4-6", [3.0, 15.0, 0.30, 3.75, 6.0]),
    ("claude-opus-4-6", [5.0, 25.0, 0.50, 6.25, 10.0]),
];
fn response_estimate(response: &Response) -> (Option<f64>, bool) {
    let Some((_, price)) = PRICES.iter().find(|(id, _)| *id == response.model) else {
        return (None, true);
    };
    let u = &response.usage;
    // Do not guess unsupported pricing modifiers. Even known base rates cannot
    // turn regional/fast/priority/long-context usage into a complete estimate.
    if u.service_tier.as_deref().is_some_and(|v| v != "standard")
        || u.inference_geo.as_deref().is_some_and(|v| v != "global")
        || u.speed.as_deref().is_some_and(|v| v != "standard")
    {
        return (None, true);
    }
    let total_input = u
        .input_tokens
        .unwrap_or(0)
        .saturating_add(u.cache_read_input_tokens.unwrap_or(0))
        .saturating_add(u.cache_creation_input_tokens.unwrap_or(0));
    if total_input > 200_000 {
        return (None, true);
    }
    let mut partial = response.partial
        || u.input_tokens.is_none()
        || u.output_tokens.is_none()
        || u.server_tool_use.is_some();
    let mut amount = 0.0;
    let mut observed = false;
    for (tokens, rate) in [
        (u.input_tokens, price[0]),
        (u.output_tokens, price[1]),
        (u.cache_read_input_tokens, price[2]),
    ] {
        if let Some(tokens) = tokens {
            if tokens <= MAX_TOKENS {
                amount += tokens as f64 * rate / 1_000_000.0;
                observed = true;
            } else {
                partial = true;
            }
        }
    }
    match (&u.cache_creation, u.cache_creation_input_tokens) {
        (Some(cache), total) => {
            let five = cache.ephemeral_5m_input_tokens;
            let hour = cache.ephemeral_1h_input_tokens;
            if let (Some(five), Some(hour)) = (five, hour) {
                if five <= MAX_TOKENS
                    && hour <= MAX_TOKENS
                    && total.is_none_or(|v| v == five + hour)
                {
                    amount += (five as f64 * price[3] + hour as f64 * price[4]) / 1_000_000.0;
                    observed = true;
                } else {
                    partial = true;
                }
            } else {
                partial = true;
            }
        }
        (None, Some(0) | None) => {}
        (None, Some(_)) => partial = true,
    }
    (observed.then_some(amount), partial)
}

pub struct ProgressParser {
    progress: HeadlessProgress,
    responses: BTreeMap<String, Response>,
    completed_tools: BTreeSet<String>,
    current_id: Option<String>,
    pending_text: Option<PendingResponse>,
    result_seen: bool,
    result_failed: bool,
    result_cost: Option<f64>,
    estimate: ProgressCost,
    dirty: bool,
}
impl ProgressParser {
    pub fn new(run_id: String, started_at_ms: i64) -> Self {
        Self {
            progress: HeadlessProgress::new(run_id, started_at_ms),
            responses: BTreeMap::new(),
            completed_tools: BTreeSet::new(),
            current_id: None,
            pending_text: None,
            result_seen: false,
            result_failed: false,
            result_cost: None,
            estimate: ProgressCost::default(),
            dirty: true,
        }
    }
    pub fn heartbeat(&mut self) {
        self.dirty = true;
    }
    pub fn mark_incomplete(&mut self) {
        self.suppress_pending_text();
        self.progress.data_incomplete = true;
        self.dirty = true;
    }
    pub fn has_incomplete_data(&self) -> bool {
        self.progress.data_incomplete
    }
    pub fn observe(&mut self, line: &[u8]) {
        if line.iter().all(u8::is_ascii_whitespace) {
            return;
        }
        if line.len() > MAX_RECORD_BYTES {
            self.mark_incomplete();
            return;
        }
        let result = self.observe_record(line);
        if result.is_err() {
            self.mark_incomplete();
        }
        self.refresh_cost();
    }
    fn observe_record(&mut self, line: &[u8]) -> Result<(), ()> {
        let header: RecordHeader = serde_json::from_slice(line).map_err(|_| ())?;
        if header.parent_tool_use_id.is_some() {
            return Ok(());
        }
        match header.kind.as_str() {
            "assistant" => {
                let record: AssistantRecord<'_> = serde_json::from_slice(line).map_err(|_| ())?;
                if record.error.is_some()
                    || record.is_api_error_message
                    || record.is_error
                    || record.api_error.is_some()
                {
                    self.reject_assistant_error();
                    return Ok(());
                }
                if record.aborted || record.is_meta || record.is_virtual || record.historical {
                    self.suppress_pending_text();
                    return Ok(());
                }
                let message =
                    serde_json::from_str::<ProviderMessage>(record.message.ok_or(())?.get())
                        .map_err(|_| ())?;
                self.assistant(message, record.uuid.as_deref());
            }
            "user" => {
                let record: UserRecord<'_> = serde_json::from_slice(line).map_err(|_| ())?;
                if let Some(content) = record.message.and_then(|m| m.content) {
                    // A user message may carry ordinary prompt text instead of
                    // tool_result blocks. Neither form supplies assistant text.
                    match content.get().as_bytes().first() {
                        Some(b'[') => {
                            let blocks: Vec<Block> =
                                serde_json::from_str(content.get()).map_err(|_| ())?;
                            for block in blocks {
                                if block.kind == "tool_result" {
                                    self.finish_tool(&block.tool_use_id);
                                }
                            }
                        }
                        Some(b'"') | Some(b'n') => {}
                        _ => return Err(()),
                    }
                }
            }
            "stream_event" => {
                let record: StreamRecord<'_> = serde_json::from_slice(line).map_err(|_| ())?;
                let raw = record.event.ok_or(())?;
                let event_header: RecordHeader = serde_json::from_str(raw.get()).map_err(|_| ())?;
                if event_header.kind == "error" {
                    self.reject_assistant_error();
                    return Ok(());
                }
                // Partial text and unknown events are never decoded as a
                // complete message, and never become free text in a snapshot.
                if matches!(
                    event_header.kind.as_str(),
                    "message_start"
                        | "message_delta"
                        | "message_stop"
                        | "content_block_start"
                        | "content_block_stop"
                ) {
                    let event: StreamEvent = serde_json::from_str(raw.get()).map_err(|_| ())?;
                    self.stream(
                        &event_header.kind,
                        event,
                        record.api_message_id.as_deref(),
                        record.abandoned_blocks.is_some(),
                        record.text_runs_joined,
                    )?;
                }
            }
            "result" => {
                let record: ResultRecord = serde_json::from_slice(line).map_err(|_| ())?;
                // A run result does not retroactively complete an unfinished
                // model response. Only the verified streaming boundary does.
                self.pending_text = None;
                self.current_id = None;
                self.result_seen = true;
                self.result_failed =
                    record.is_error || (!record.subtype.is_empty() && record.subtype != "success");
                self.progress.result_turns = record.num_turns.filter(|v| *v <= MAX_TURNS);
                if record.num_turns.is_some() && self.progress.result_turns.is_none() {
                    self.mark_incomplete();
                }
                self.result_cost = record.total_cost_usd.filter(|v| valid_cost(*v));
                self.progress.state = ProgressState::Finishing;
                self.progress.active_tools.clear();
                self.dirty = true;
            }
            "error" => self.reject_assistant_error(),
            // Known system metadata and unknown future envelope types can have
            // entirely different message bodies. Valid JSON is safely ignored.
            _ => {}
        }
        Ok(())
    }
    fn suppress_pending_text(&mut self) {
        if let Some(pending) = &mut self.pending_text {
            pending.suppress();
        }
    }
    fn reject_assistant_error(&mut self) {
        self.suppress_pending_text();
        self.result_failed = true;
        self.dirty = true;
    }
    fn response(&mut self, id: &str) -> Option<&mut Response> {
        if !valid_id(id)
            || (!self.responses.contains_key(id) && self.responses.len() >= MAX_RESPONSES)
        {
            self.mark_incomplete();
            return None;
        }
        Some(self.responses.entry(id.to_owned()).or_default())
    }
    fn assistant(&mut self, message: ProviderMessage, uuid: Option<&str>) {
        let Some(response) = self.response(&message.id) else {
            return;
        };
        response.observed = true;
        if !message.model.is_empty() {
            response.model = if message.model.len() <= 128 {
                message.model
            } else {
                String::new()
            };
        }
        // assistant output_tokens may be placeholders: only stream deltas supply it.
        if let Some(usage) = message.usage {
            response.usage.merge(usage, false);
        }
        self.progress.observed_turns =
            self.responses.values().filter(|r| r.observed).count() as u64;
        if let Some(pending) = &mut self.pending_text {
            if pending.id == message.id {
                pending.assistant(uuid, &message.content);
            } else if message.content.iter().any(|b| b.kind == "text") {
                pending.suppress();
            }
        }
        // With no matching message_start there is no proof that this record is
        // the whole response. Keep the previous safe utterance, never a fragment.
        for block in message.content {
            if block.kind == "tool_use" {
                self.start_tool(block);
            }
        }
        self.dirty = true;
    }
    fn stream(
        &mut self,
        kind: &str,
        event: StreamEvent,
        api_message_id: Option<&str>,
        abandoned: bool,
        text_runs_joined: bool,
    ) -> Result<(), ()> {
        if kind != "message_start"
            && api_message_id.is_some_and(|id| self.current_id.as_deref() != Some(id))
        {
            self.suppress_pending_text();
            return Ok(());
        }
        match kind {
            "message_start" => {
                let message = event.message.ok_or(())?;
                if api_message_id.is_some_and(|id| id != message.id) {
                    return Err(());
                }
                if let Some(previous) = self.pending_text.take()
                    && let Some(response) = self.responses.get_mut(&previous.id)
                {
                    response.text_finished = true;
                }
                let Some(response) = self.response(&message.id) else {
                    self.current_id = None;
                    return Ok(());
                };
                response.model = if message.model.len() <= 128 {
                    message.model
                } else {
                    String::new()
                };
                if let Some(usage) = message.usage {
                    response.usage.merge(usage, false);
                }
                response.observed = true;
                let suppressed =
                    response.text_finished || text_runs_joined || !message.content.is_empty();
                self.pending_text = Some(PendingResponse::new(message.id.clone(), suppressed));
                self.progress.observed_turns =
                    self.responses.values().filter(|r| r.observed).count() as u64;
                self.current_id = Some(message.id);
                self.dirty = true;
            }
            "message_delta" => {
                if let Some(pending) = &mut self.pending_text {
                    pending.terminal_reason =
                        event
                            .delta
                            .and_then(|d| d.stop_reason)
                            .is_some_and(|reason| {
                                matches!(reason.as_str(), "end_turn" | "tool_use" | "stop_sequence")
                            });
                }
                if let (Some(id), Some(usage)) = (self.current_id.clone(), event.usage)
                    && let Some(response) = self.response(&id)
                {
                    response.usage.merge(usage, true);
                    self.dirty = true;
                }
            }
            "message_stop" => {
                if let Some(mut pending) = self.pending_text.take() {
                    if abandoned {
                        pending.suppress();
                    }
                    if let Some(response) = self.responses.get_mut(&pending.id) {
                        response.text_finished = true;
                    }
                    if let Some(text) = pending.finish() {
                        self.progress.last_assistant = text;
                        self.dirty = true;
                    }
                }
                self.current_id = None;
            }
            "content_block_start" => {
                let block = event.content_block.ok_or(())?;
                let index = event.index.ok_or(())?;
                if let Some(pending) = &mut self.pending_text {
                    pending.start_block(index, &block);
                }
                if block.kind == "tool_use" {
                    self.start_tool(block);
                }
            }
            "content_block_stop" => {
                let index = event.index.ok_or(())?;
                if let Some(pending) = &mut self.pending_text {
                    pending.stop_block(index);
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn start_tool(&mut self, block: Block) {
        if !valid_id(&block.id) || block.name.is_empty() {
            self.mark_incomplete();
            return;
        }
        if self.completed_tools.contains(&block.id)
            || self.progress.active_tools.iter().any(|t| t.id == block.id)
        {
            return;
        }
        if self.progress.active_tools.len() >= MAX_ACTIVE_TOOLS {
            self.mark_incomplete();
            return;
        }
        let name = safe_excerpt(&block.name, 64);
        if name.is_empty() {
            return;
        }
        self.progress
            .active_tools
            .push(ActiveTool { id: block.id, name });
        self.dirty = true;
    }
    fn finish_tool(&mut self, id: &str) {
        if !valid_id(id) {
            self.mark_incomplete();
            return;
        }
        if self.completed_tools.len() >= MAX_RESPONSES && !self.completed_tools.contains(id) {
            self.mark_incomplete();
            return;
        }
        self.completed_tools.insert(id.into());
        self.progress.active_tools.retain(|t| t.id != id);
        self.dirty = true;
    }
    fn refresh_cost(&mut self) {
        let mut amount = 0.0;
        let mut known = false;
        let mut partial = self.progress.data_incomplete;
        for response in self.responses.values() {
            let (cost, incomplete) = response_estimate(response);
            partial |= incomplete;
            if let Some(cost) = cost {
                known = true;
                amount += cost;
            }
        }
        if known && valid_cost(amount) {
            self.estimate = ProgressCost {
                amount_usd: Some(amount),
                currency: "USD".into(),
                kind: CostKind::Estimate,
                coverage: if partial {
                    CostCoverage::Partial
                } else {
                    CostCoverage::ObservedMain
                },
            };
        } else if self.estimate.amount_usd.is_some() {
            // Retain the last known observation instead of inventing zero, but
            // never keep a complete-coverage label after pricing becomes unknown.
            self.estimate.coverage = CostCoverage::Partial;
        }
        self.progress.cost = match self.result_cost {
            Some(cost) if cost > 0.0 || self.estimate.amount_usd.unwrap_or(0.0) == 0.0 => {
                ProgressCost {
                    amount_usd: Some(cost),
                    currency: "USD".into(),
                    kind: CostKind::Result,
                    coverage: CostCoverage::CliResult,
                }
            }
            _ => self.estimate.clone(),
        };
    }
    pub fn snapshot(&mut self, elapsed_ms: u64) -> Option<HeadlessProgress> {
        if !self.dirty {
            return None;
        }
        self.progress.elapsed_ms = elapsed_ms.min(MAX_ELAPSED_MS).max(self.progress.elapsed_ms);
        self.progress.sequence = self.progress.sequence.saturating_add(1);
        self.dirty = false;
        Some(self.progress.clone())
    }
    pub fn finish(
        &mut self,
        output: &ProcessOutput,
        dropped: u64,
        elapsed_ms: u64,
    ) -> HeadlessProgress {
        self.pending_text = None;
        self.current_id = None;
        self.progress.elapsed_ms = elapsed_ms.min(MAX_ELAPSED_MS).max(self.progress.elapsed_ms);
        self.progress.ended_at_ms = Some(
            self.progress
                .started_at_ms
                .saturating_add(self.progress.elapsed_ms as i64),
        );
        self.progress.active_tools.clear();
        self.progress.data_incomplete |= dropped > 0 || output.pipes_forced_closed;
        let successful = matches!(output.outcome, ExitOutcome::Exited { code: Some(0), .. })
            && !self.result_failed
            && !self.progress.data_incomplete;
        self.progress.state = if self.progress.data_incomplete {
            ProgressState::Failed
        } else if matches!(output.outcome, ExitOutcome::Cancelled) {
            ProgressState::Canceled
        } else if successful {
            ProgressState::Completed
        } else {
            ProgressState::Failed
        };
        // An unnatural zero on a crash must not erase a positive estimate.
        if !successful && self.result_cost == Some(0.0) {
            self.result_cost = None;
        }
        self.refresh_cost();
        self.progress.final_cost_missing =
            !self.result_seen || self.progress.cost.kind != CostKind::Result;
        self.dirty = true;
        self.snapshot(elapsed_ms).expect("final snapshot is dirty")
    }
}

/// A strict record is never split into independent JSON fragments. Oversized
/// records report loss immediately at the cap, then drain to LF without ever
/// exposing a fragment as an independent record.
#[derive(Default)]
pub struct StrictLineFramer {
    pending: Vec<u8>,
    discarding: bool,
}
impl StrictLineFramer {
    pub fn push(&mut self, bytes: &[u8], mut line: impl FnMut(Option<&[u8]>)) {
        for &byte in bytes {
            if byte == b'\n' {
                if !self.discarding {
                    if self.pending.last() == Some(&b'\r') {
                        self.pending.pop();
                    }
                    line(Some(&self.pending));
                }
                self.pending.clear();
                self.discarding = false;
            } else if !self.discarding {
                if self.pending.len() == MAX_RECORD_BYTES {
                    self.pending.clear();
                    self.discarding = true;
                    line(None);
                } else {
                    self.pending.push(byte);
                }
            }
        }
    }
    pub fn finish(&mut self, mut line: impl FnMut(Option<&[u8]>)) {
        if !self.discarding && !self.pending.is_empty() {
            line(Some(&self.pending));
        }
        self.clear();
    }
    pub fn clear(&mut self) {
        self.pending.clear();
        self.discarding = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn parser() -> ProgressParser {
        ProgressParser::new("7:3".into(), 1_000)
    }
    fn observe(p: &mut ProgressParser, v: serde_json::Value) {
        p.observe(&serde_json::to_vec(&v).unwrap());
    }
    fn start(p: &mut ProgressParser, id: &str, usage: serde_json::Value) {
        observe(
            p,
            json!({"type":"stream_event","parent_tool_use_id":null,"event":{"type":"message_start","message":{"id":id,"model":"claude-sonnet-4-5-20250929","usage":usage}}}),
        );
    }
    fn text_block_records(id: &str, index: u64, uuid: &str, text: &str) -> Vec<serde_json::Value> {
        vec![
            json!({"type":"stream_event","event":{"type":"content_block_start","index":index,"content_block":{"type":"text","text":""}}}),
            json!({"type":"assistant","uuid":uuid,"message":{"id":id,"content":[{"type":"text","text":text}]}}),
            json!({"type":"stream_event","event":{"type":"content_block_stop","index":index}}),
        ]
    }
    fn text_block(p: &mut ProgressParser, id: &str, index: u64, uuid: &str, text: &str) {
        for record in text_block_records(id, index, uuid, text) {
            observe(p, record);
        }
    }
    fn complete(p: &mut ProgressParser) {
        observe(
            p,
            json!({"type":"stream_event","event":{"type":"message_delta","delta":{"stop_reason":"end_turn"}}}),
        );
        observe(
            p,
            json!({"type":"stream_event","event":{"type":"message_stop"}}),
        );
    }
    fn prior_safe(p: &mut ProgressParser) {
        start(p, "prior", json!({"input_tokens":1}));
        text_block(p, "prior", 0, "prior-uuid", "Earlier safe response.");
        complete(p);
        assert_eq!(
            p.snapshot(0).unwrap().last_assistant,
            "Earlier safe response."
        );
    }
    fn assert_no_public_fragment(p: &mut ProgressParser, last: &str, needles: &[&str]) {
        p.heartbeat();
        let snapshot = p.snapshot(1).unwrap();
        assert_eq!(snapshot.last_assistant, last);
        snapshot.validate().unwrap();
        let wire = serde_json::to_string(&snapshot).unwrap();
        let terminal = snapshot.terminal_line();
        for needle in needles {
            assert!(
                !wire.contains(needle),
                "fragment entered a progress snapshot"
            );
            assert!(
                !terminal.contains(needle),
                "fragment entered a terminal projection"
            );
        }
    }
    fn output(code: i32) -> ProcessOutput {
        ProcessOutput {
            stdout: vec![],
            stderr: vec![],
            stdout_truncated: false,
            stderr_truncated: false,
            pipes_forced_closed: false,
            outcome: ExitOutcome::Exited {
                code: Some(code),
                signal: None,
            },
        }
    }
    #[test]
    fn cumulative_usage_is_replaced_and_assistant_placeholder_is_ignored() {
        let mut p = parser();
        start(
            &mut p,
            "msg_1",
            json!({"input_tokens":1000,"output_tokens":1}),
        );
        for count in [10, 20, 20, 10] {
            observe(
                &mut p,
                json!({"type":"stream_event","event":{"type":"message_delta","usage":{"output_tokens":count}}}),
            );
        }
        observe(
            &mut p,
            json!({"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}}),
        );
        for _ in 0..2 {
            observe(
                &mut p,
                json!({"type":"assistant","uuid":"one-block","message":{"id":"msg_1","model":"claude-sonnet-4-5-20250929","usage":{"input_tokens":1000,"output_tokens":9999},"content":[{"type":"text","text":"Done."}]}}),
            );
        }
        observe(
            &mut p,
            json!({"type":"stream_event","event":{"type":"content_block_stop","index":0}}),
        );
        complete(&mut p);
        let snapshot = p.snapshot(99).unwrap();
        assert_eq!(snapshot.observed_turns, 1);
        assert_eq!(snapshot.cost.kind, CostKind::Estimate);
        assert!((snapshot.cost.amount_usd.unwrap() - 0.0033).abs() < 1e-10);
        assert_eq!(snapshot.cost.coverage, CostCoverage::ObservedMain);
        assert_eq!(snapshot.last_assistant, "Done.");
    }
    #[test]
    fn multiple_tools_dedupe_finish_and_clear_on_exit() {
        let mut p = parser();
        for _ in 0..2 {
            observe(
                &mut p,
                json!({"type":"assistant","message":{"id":"msg1","content":[{"type":"tool_use","id":"tool1","name":"Read","input":{"command":"PRIVATE_TOOL_ARGS_SENTINEL"}},{"type":"tool_use","id":"tool2","name":"Bash"}]}}),
            );
        }
        assert_eq!(p.snapshot(0).unwrap().active_tools.len(), 2);
        observe(
            &mut p,
            json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tool1","content":"PRIVATE_TOOL_RESULT_SENTINEL"}]}}),
        );
        assert_eq!(p.snapshot(1).unwrap().active_tools[0].id, "tool2");
        observe(
            &mut p,
            json!({"type":"assistant","message":{"id":"msg1","content":[{"type":"tool_use","id":"tool1","name":"Read"}]}}),
        );
        assert_eq!(p.snapshot(2).unwrap().active_tools.len(), 1);
        assert!(p.finish(&output(0), 0, 3).active_tools.is_empty());
    }
    #[test]
    fn result_cost_and_turns_replace_observations_but_do_not_prove_success() {
        let mut p = parser();
        start(&mut p, "one", json!({"input_tokens":100}));
        for _ in 0..2 {
            observe(
                &mut p,
                json!({"type":"result","subtype":"success","num_turns":8,"total_cost_usd":0.45}),
            );
        }
        let before_exit = p.snapshot(100).unwrap();
        assert_eq!(before_exit.state, ProgressState::Finishing);
        assert_eq!(before_exit.result_turns, Some(8));
        assert_eq!(before_exit.cost.amount_usd, Some(0.45));
        let final_snapshot = p.finish(&output(7), 0, 150);
        assert_eq!(final_snapshot.state, ProgressState::Failed);
        assert_eq!(final_snapshot.cost.kind, CostKind::Result);
        assert_eq!(final_snapshot.cost.amount_usd, Some(0.45));
        assert!(!final_snapshot.final_cost_missing);
        assert_eq!(final_snapshot.ended_at_ms, Some(1150));
    }
    #[test]
    fn partial_unknown_models_cache_and_modifiers_are_not_guessed() {
        let mut p = parser();
        start(
            &mut p,
            "known",
            json!({"input_tokens":1000,"cache_creation_input_tokens":100}),
        );
        observe(
            &mut p,
            json!({"type":"stream_event","event":{"type":"message_delta","usage":{"output_tokens":10}}}),
        );
        let s = p.snapshot(0).unwrap();
        assert_eq!(s.cost.coverage, CostCoverage::Partial);
        assert!((s.cost.amount_usd.unwrap() - 0.00315).abs() < 1e-10);
        start(&mut p, "fast", json!({"input_tokens":1000,"speed":"fast"}));
        observe(
            &mut p,
            json!({"type":"stream_event","event":{"type":"message_start","message":{"id":"unknown","model":"sonnet","usage":{"input_tokens":900000}}}}),
        );
        let s = p.snapshot(1).unwrap();
        assert_eq!(s.cost.coverage, CostCoverage::Partial);
        assert!((s.cost.amount_usd.unwrap() - 0.00315).abs() < 1e-10);
        let mut unknown = parser();
        observe(
            &mut unknown,
            json!({"type":"stream_event","event":{"type":"message_start","message":{"id":"unknown","model":"claude-sonnet-4-5","usage":{"input_tokens":1}}}}),
        );
        assert_eq!(
            unknown.snapshot(0).unwrap().cost.kind,
            CostKind::Unavailable
        );
    }
    #[test]
    fn cache_durations_have_distinct_verified_rates() {
        let mut p = parser();
        start(
            &mut p,
            "msg",
            json!({"input_tokens":1000,"cache_read_input_tokens":1000,"cache_creation_input_tokens":2000,"cache_creation":{"ephemeral_5m_input_tokens":1000,"ephemeral_1h_input_tokens":1000}}),
        );
        observe(
            &mut p,
            json!({"type":"stream_event","event":{"type":"message_delta","usage":{"output_tokens":1000}}}),
        );
        let s = p.snapshot(0).unwrap();
        assert_eq!(s.cost.coverage, CostCoverage::ObservedMain);
        assert!((s.cost.amount_usd.unwrap() - 0.02805).abs() < 1e-10);
    }
    #[test]
    fn missing_invalid_or_crash_zero_final_does_not_erase_estimate() {
        for result in [
            json!({"type":"result","total_cost_usd":0}),
            json!({"type":"result","total_cost_usd":-1}),
            json!({"type":"result"}),
        ] {
            let mut p = parser();
            start(&mut p, "msg", json!({"input_tokens":1000}));
            observe(&mut p, result);
            let s = p.finish(&output(7), 0, 10);
            assert_eq!(s.cost.kind, CostKind::Estimate);
            assert!(s.cost.amount_usd.unwrap() > 0.0);
            assert!(s.final_cost_missing);
        }
    }
    #[test]
    fn subagents_and_partial_text_never_supply_main_text_or_turns() {
        let mut p = parser();
        observe(
            &mut p,
            json!({"type":"assistant","parent_tool_use_id":"tool1","message":{"id":"child","content":[{"type":"text","text":"PRIVATE_CHILD_SENTINEL"}]}}),
        );
        observe(
            &mut p,
            json!({"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"PRIVATE_DELTA_SENTINEL"}}}),
        );
        let s = p.snapshot(0).unwrap();
        assert_eq!(s.observed_turns, 0);
        assert!(s.last_assistant.is_empty());
        assert!(!serde_json::to_string(&s).unwrap().contains("SENTINEL"));
    }
    #[test]
    fn full_text_is_masked_before_truncation_including_split_text_blocks() {
        let sentinel = "sk-ant-SYNTHETIC_ONLY_NOT_A_REAL_KEY_1234567890";
        let mut p = parser();
        start(&mut p, "one", json!({"input_tokens":1}));
        text_block(
            &mut p,
            "one",
            0,
            "block-zero",
            &format!("{}sk-ant-", "a".repeat(500)),
        );
        assert_no_public_fragment(&mut p, "", &["sk-ant-", "SYNTHETIC_ONLY"]);
        text_block(
            &mut p,
            "one",
            1,
            "block-one",
            "SYNTHETIC_ONLY_NOT_A_REAL_KEY_1234567890 <img src=x onerror=alert(1)>\u{001b}[31m",
        );
        assert_no_public_fragment(&mut p, "", &["sk-ant-", "SYNTHETIC_ONLY"]);
        complete(&mut p);
        let s = p.snapshot(0).unwrap();
        let encoded = serde_json::to_string(&s).unwrap();
        assert!(s.last_assistant.contains("***"));
        assert!(!encoded.contains(sentinel));
        assert!(!encoded.contains("SYNTHETIC_ONLY"));
        assert!(!s.last_assistant.contains('<'));
        assert!(!s.last_assistant.contains('\u{001b}'));
        s.validate().unwrap();
        let escaped = safe_excerpt("<img> \"API_KEY=syntheticsecret\"", 512);
        assert!(safe_wire_text(&escaped));
        assert_eq!(
            safe_excerpt(
                "sk-ant-SYNTHETIC\u{001b}_ONLY_NOT_A_REAL_KEY_1234567890",
                512
            ),
            "***"
        );
    }
    #[test]
    fn chunked_json_is_bounded_and_never_publishes_bad_records_or_raw_errors() {
        let mut records = vec![
            json!({"type":"stream_event","event":{"type":"message_start","message":{"id":"one","content":[]}}}),
        ];
        records.extend(text_block_records(
            "one",
            0,
            "one-block",
            "API_KEY=synthetic_secret_key_1234567890",
        ));
        records.extend([
            json!({"type":"stream_event","event":{"type":"message_delta","delta":{"stop_reason":"end_turn"}}}),
            json!({"type":"stream_event","event":{"type":"message_stop"}}),
        ]);
        let source = records
            .iter()
            .map(|v| v.to_string() + "\n")
            .collect::<String>();
        for size in [1, 2, 13, 4096] {
            let mut p = parser();
            let mut f = StrictLineFramer::default();
            for chunk in source.as_bytes().chunks(size) {
                f.push(chunk, |line| p.observe(line.unwrap()));
            }
            f.finish(|line| p.observe(line.unwrap()));
            let s = p.snapshot(0).unwrap();
            assert_eq!(s.last_assistant, "API_KEY=***");
        }
        let mut p = parser();
        p.observe(b"PRIVATE_INVALID_JSON_SENTINEL");
        observe(
            &mut p,
            json!({"type":"result","is_error":true,"error":{"message":"PRIVATE_ERROR_SENTINEL"},"result":"PRIVATE_RESULT_SENTINEL"}),
        );
        let s = p.finish(&output(0), 0, 10);
        assert_eq!(s.state, ProgressState::Failed);
        assert!(s.data_incomplete);
        assert!(!serde_json::to_string(&s).unwrap().contains("SENTINEL"));
        let mut f = StrictLineFramer::default();
        let mut lost = 0;
        f.push(&vec![b'x'; MAX_RECORD_BYTES + 10], |v| {
            assert!(v.is_none());
            lost += 1;
        });
        assert!(f.pending.len() <= MAX_RECORD_BYTES);
        f.finish(|v| {
            assert!(v.is_none());
            lost += 1;
        });
        assert_eq!(lost, 1);
    }
    #[test]
    fn final_unterminated_record_is_flushed_and_lag_cannot_be_success() {
        let mut p = parser();
        let mut f = StrictLineFramer::default();
        f.push(b"{\"type\":\"result\",\"total_cost_usd\":0.2}", |_| {
            panic!("no LF")
        });
        f.finish(|v| p.observe(v.unwrap()));
        let s = p.finish(&output(0), 1, 90);
        assert_eq!(s.state, ProgressState::Failed);
        assert!(s.data_incomplete);
        assert_eq!(s.cost.amount_usd, Some(0.2));
    }
    #[test]
    fn budget_result_with_false_error_flag_and_zero_exit_is_still_failed() {
        let mut p = parser();
        observe(
            &mut p,
            json!({"type":"result","subtype":"error_max_budget_usd","is_error":false,"total_cost_usd":0.2}),
        );
        let s = p.finish(&output(0), 0, 1);
        assert_eq!(s.state, ProgressState::Failed);
        assert_eq!(s.cost.amount_usd, Some(0.2));
        assert_eq!(s.cost.kind, CostKind::Result);
    }
    #[test]
    fn newly_unknown_modifier_preserves_only_a_partial_prior_observation() {
        let mut p = parser();
        start(&mut p, "one", json!({"input_tokens":1000}));
        observe(
            &mut p,
            json!({"type":"stream_event","event":{"type":"message_delta","usage":{"output_tokens":100}}}),
        );
        let before = p.snapshot(1).unwrap();
        assert_eq!(before.cost.coverage, CostCoverage::ObservedMain);
        observe(
            &mut p,
            json!({"type":"stream_event","event":{"type":"message_delta","usage":{"speed":"fast","output_tokens":200}}}),
        );
        let after = p.snapshot(2).unwrap();
        assert_eq!(after.cost.amount_usd, before.cost.amount_usd);
        assert_eq!(after.cost.coverage, CostCoverage::Partial);
    }
    #[test]
    fn cancellation_and_forced_pipe_close_are_not_success() {
        let mut p = parser();
        let mut o = output(0);
        o.outcome = ExitOutcome::Cancelled;
        assert_eq!(p.finish(&o, 0, 1).state, ProgressState::Canceled);
        let mut p = parser();
        let mut o = output(0);
        o.pipes_forced_closed = true;
        assert_eq!(p.finish(&o, 0, 1).state, ProgressState::Failed);
    }
    #[test]
    fn public_wire_rejects_extra_fields_ranges_and_unmasked_content() {
        let mut p = parser();
        let s = p.snapshot(0).unwrap();
        let bytes = serde_json::to_vec(&s).unwrap();
        assert_eq!(decode_progress(&bytes).unwrap(), s);
        let mut extra = serde_json::to_value(&s).unwrap();
        extra["raw_error"] = json!("PRIVATE_SENTINEL");
        assert!(decode_progress(&serde_json::to_vec(&extra).unwrap()).is_err());
        let mut s = s;
        s.last_assistant = "sk-ant-SYNTHETIC_ONLY_NOT_A_REAL_KEY_1234567890".into();
        assert!(s.validate().is_err());
        s.last_assistant.clear();
        s.elapsed_ms = u64::MAX;
        assert!(s.validate().is_err());
        assert!(decode_progress(&vec![b' '; MAX_PROGRESS_BYTES + 1]).is_err());
    }
    #[test]
    fn assistant_error_flags_never_decode_or_publish_error_bodies() {
        for flag in [
            json!({"error":"authentication_failed"}),
            json!({"is_api_error_message":true}),
            json!({"is_error":true}),
            json!({"api_error":"provider_credentials"}),
            json!({"error":{"message":"PRIVATE_NESTED_ERROR_SENTINEL"}}),
        ] {
            let mut p = parser();
            prior_safe(&mut p);
            let mut record = json!({"type":"assistant","uuid":"error-uuid","message":{"id":"error-id","content":[{"type":"text","text":"API Error: PRIVATE_ERROR_SENTINEL sk-ant-SYNTHETIC_ONLY_NOT_A_REAL_KEY_1234567890"}]}});
            for (key, value) in flag.as_object().unwrap() {
                record[key] = value.clone();
            }
            observe(&mut p, record.clone());
            assert_no_public_fragment(
                &mut p,
                "Earlier safe response.",
                &["PRIVATE_", "sk-ant-", "SYNTHETIC_ONLY"],
            );
            // Error flags are inspected before even an incompatible body shape.
            record["message"] = json!("PRIVATE_ERROR_BODY_SENTINEL");
            observe(&mut p, record);
            assert!(!p.has_incomplete_data());
            assert_no_public_fragment(
                &mut p,
                "Earlier safe response.",
                &["PRIVATE_", "sk-ant-", "SYNTHETIC_ONLY"],
            );
            assert_eq!(p.finish(&output(0), 0, 2).state, ProgressState::Failed);
        }
    }
    #[test]
    fn separate_assistant_records_mask_once_after_verified_response_completion() {
        let mut p = parser();
        prior_safe(&mut p);
        start(&mut p, "same_response_id", json!({"input_tokens":1}));
        let first = text_block_records("same_response_id", 0, "uuid-zero", "sk-ant-");
        for record in &first {
            observe(&mut p, record.clone());
            assert_no_public_fragment(
                &mut p,
                "Earlier safe response.",
                &["sk-ant-", "SYNTHETIC_ONLY"],
            );
        }
        let second = text_block_records(
            "same_response_id",
            1,
            "uuid-one",
            "SYNTHETIC_ONLY_NOT_A_REAL_KEY_1234567890",
        );
        observe(&mut p, second[0].clone());
        // A delayed duplicate from the previous block cannot be appended to this one.
        observe(&mut p, first[1].clone());
        for record in &second[1..] {
            observe(&mut p, record.clone());
            assert_no_public_fragment(
                &mut p,
                "Earlier safe response.",
                &["sk-ant-", "SYNTHETIC_ONLY"],
            );
        }
        observe(
            &mut p,
            json!({"type":"stream_event","event":{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":10}}}),
        );
        assert_no_public_fragment(
            &mut p,
            "Earlier safe response.",
            &["sk-ant-", "SYNTHETIC_ONLY"],
        );
        observe(
            &mut p,
            json!({"type":"stream_event","event":{"type":"message_stop"}}),
        );
        assert_no_public_fragment(&mut p, "***", &["sk-ant-", "SYNTHETIC_ONLY"]);
        assert!(p.pending_text.is_none());
        // Later records and a replayed stream with this ID cannot publish another piece.
        observe(&mut p, second[1].clone());
        start(&mut p, "same_response_id", json!({"input_tokens":1}));
        for record in second {
            observe(&mut p, record);
        }
        complete(&mut p);
        assert_no_public_fragment(&mut p, "***", &["sk-ant-", "SYNTHETIC_ONLY"]);
        assert_eq!(p.progress.observed_turns, 2); // prior + one deduplicated response
    }
    #[test]
    fn missing_start_or_unverified_message_stop_never_promotes_partial_text() {
        for mode in [
            "no_start",
            "no_reason",
            "max_tokens",
            "abandoned",
            "missing_block_record",
            "missing_block_stop",
            "joined",
            "missing_uuid",
            "conflicting_duplicate",
        ] {
            let mut p = parser();
            prior_safe(&mut p);
            if mode != "no_start" {
                observe(
                    &mut p,
                    json!({"type":"stream_event","text_runs_joined":mode=="joined","event":{"type":"message_start","message":{"id":"one","content":[]}}}),
                );
            }
            let records = text_block_records("one", 0, "one-uuid", "PRIVATE_PARTIAL_SENTINEL");
            observe(&mut p, records[0].clone());
            if mode != "missing_block_record" {
                let mut record = records[1].clone();
                if mode == "missing_uuid" {
                    record.as_object_mut().unwrap().remove("uuid");
                }
                observe(&mut p, record.clone());
                if mode == "conflicting_duplicate" {
                    record["message"]["content"][0]["text"] = json!("PRIVATE_CHANGED_SENTINEL");
                    observe(&mut p, record);
                }
            }
            if mode != "missing_block_stop" {
                observe(&mut p, records[2].clone());
            }
            if mode != "no_reason" {
                observe(
                    &mut p,
                    json!({"type":"stream_event","event":{"type":"message_delta","delta":{"stop_reason":if mode=="max_tokens" {"max_tokens"} else {"end_turn"}}}}),
                );
            }
            let mut stop = json!({"type":"stream_event","event":{"type":"message_stop"}});
            if mode == "abandoned" {
                stop["abandoned_blocks"] = json!({"api_message_id":"one","from_block_index":1});
            }
            observe(&mut p, stop);
            assert_no_public_fragment(&mut p, "Earlier safe response.", &["PRIVATE_"]);
            observe(
                &mut p,
                json!({"type":"result","subtype":"success","total_cost_usd":0.1}),
            );
            assert_no_public_fragment(&mut p, "Earlier safe response.", &["PRIVATE_"]);
        }
    }
    #[test]
    fn error_or_abort_between_blocks_discards_text_before_late_completion() {
        for marker in [
            json!({"type":"error","message":"PRIVATE_ERROR_SENTINEL"}),
            json!({"type":"stream_event","event":{"type":"error","error":{"message":"PRIVATE_STREAM_ERROR_SENTINEL"}}}),
            json!({"type":"assistant","error":"authentication_failed","message":{"content":[{"type":"text","text":"PRIVATE_ERROR_SENTINEL"}]}}),
            json!({"type":"assistant","is_api_error_message":true,"message":{"content":[{"type":"text","text":"PRIVATE_ERROR_SENTINEL"}]}}),
            json!({"type":"assistant","aborted":true,"message":{"content":[{"type":"text","text":"PRIVATE_ABORT_SENTINEL"}]}}),
        ] {
            let mut p = parser();
            prior_safe(&mut p);
            start(&mut p, "one", json!({"input_tokens":1}));
            text_block(&mut p, "one", 0, "first-uuid", "sk-ant-");
            observe(&mut p, marker);
            let pending = p.pending_text.as_ref().unwrap();
            assert!(pending.suppressed);
            assert_eq!(pending.text_bytes, 0);
            assert!(pending.blocks.is_empty());
            text_block(
                &mut p,
                "one",
                1,
                "second-uuid",
                "SYNTHETIC_ONLY_NOT_A_REAL_KEY_1234567890",
            );
            complete(&mut p);
            assert_no_public_fragment(
                &mut p,
                "Earlier safe response.",
                &["PRIVATE_", "sk-ant-", "SYNTHETIC_ONLY"],
            );
            observe(
                &mut p,
                json!({"type":"result","subtype":"success","total_cost_usd":0.1}),
            );
            assert_no_public_fragment(
                &mut p,
                "Earlier safe response.",
                &["PRIVATE_", "sk-ant-", "SYNTHETIC_ONLY"],
            );
        }
    }
    #[test]
    fn private_response_buffer_is_globally_bounded_and_cleared_on_every_end() {
        let mut p = parser();
        prior_safe(&mut p);
        start(&mut p, "large", json!({"input_tokens":1}));
        text_block(
            &mut p,
            "large",
            0,
            "large-zero",
            &"x".repeat(MAX_RESPONSE_TEXT_BYTES),
        );
        assert_eq!(
            p.pending_text.as_ref().unwrap().text_bytes,
            MAX_RESPONSE_TEXT_BYTES
        );
        text_block(
            &mut p,
            "large",
            1,
            "large-one",
            "PRIVATE_OVER_LIMIT_SENTINEL",
        );
        let pending = p.pending_text.as_ref().unwrap();
        assert!(pending.suppressed);
        assert_eq!(pending.text_bytes, 0);
        assert!(pending.blocks.is_empty());
        complete(&mut p);
        assert!(p.pending_text.is_none());
        assert_no_public_fragment(&mut p, "Earlier safe response.", &["PRIVATE_OVER_LIMIT"]);
        for i in 0..10 {
            let id = format!("unfinished-{i}");
            start(&mut p, &id, json!({"input_tokens":1}));
            text_block(&mut p, &id, 0, "one-uuid", "PRIVATE_UNFINISHED_SENTINEL");
            assert_eq!(p.pending_text.as_ref().unwrap().text_bytes, 27);
        }
        p.finish(&output(0), 0, 10);
        assert!(p.pending_text.is_none());
        assert_no_public_fragment(&mut p, "Earlier safe response.", &["PRIVATE_UNFINISHED"]);
    }
    #[test]
    fn valid_system_metadata_is_ignored_but_malformed_known_records_fail_closed() {
        let mut p = parser();
        for record in [
            json!({"type":"system","subtype":"permission_denied","message":"Synthetic permission was denied","tool_name":"Bash","tool_use_id":"tool_one"}),
            json!({"type":"system","subtype":"future_subtype","message":{"content":"PRIVATE_SYSTEM_SENTINEL"},"usage":"not assistant usage"}),
            json!({"type":"future_type","message":"PRIVATE_FUTURE_SENTINEL"}),
            json!({"type":"user","message":{"role":"user","content":"PRIVATE_USER_SENTINEL"}}),
        ] {
            observe(&mut p, record);
            assert!(!p.has_incomplete_data());
            assert_no_public_fragment(&mut p, "", &["PRIVATE_", "permission was denied"]);
        }
        observe(
            &mut p,
            json!({"type":"result","subtype":"success","total_cost_usd":0.1}),
        );
        assert_eq!(p.finish(&output(0), 0, 1).state, ProgressState::Completed);
        for record in [
            br#"{"type":"assistant","message":"PRIVATE_BAD_BODY_SENTINEL"}"#.as_slice(),
            br#"{"type":"assistant","message":{"id":"one","content":"not blocks"}}"#.as_slice(),
            br#"{"type":"system","message":"truncated"#.as_slice(),
            br#"{"type":"stream_event","event":{"type":"content_block_start","index":0}}"#
                .as_slice(),
        ] {
            let mut p = parser();
            p.observe(record);
            assert!(p.has_incomplete_data());
            assert_no_public_fragment(&mut p, "", &["PRIVATE_", "truncated"]);
        }
    }
}
