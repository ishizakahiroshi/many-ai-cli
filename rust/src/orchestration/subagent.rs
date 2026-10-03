//! Source: internal/hub/subagent_{tree,source_claude,source_codex,source_grok}.go.
//! Provider-owned paths are supplied by the resolver. No home lookup, PTY parsing,
//! transcript content field or dynamically typed continuation crosses this API.
use crate::proto::time::{Timestamp, UNIX_EPOCH};
mod claude;
mod codex;
mod grok;
use crate::proto::{SubagentNode, SubagentTree};
use serde::{Deserialize, Deserializer};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::Duration,
};

pub use claude::ClaudeState;
pub use codex::CodexState;
pub use grok::GrokState;
pub const POLL_INTERVAL: Duration = Duration::from_secs(3);
pub const RESOLVE_RETRY_INTERVAL: Duration = Duration::from_secs(30);
const PARENT_BYTES: usize = 2 * 1024 * 1024;
const PARENT_RECORDS: usize = 500;
const METADATA_BYTES: usize = 4 * 1024 * 1024;
const FRESH_MS: i64 = 600_000;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Adapter {
    Claude,
    Codex,
    Grok,
}
impl Adapter {
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "subagent:claude-v1" => Some(Self::Claude),
            "subagent:codex-v1" => Some(Self::Codex),
            "subagent:grok-v1" => Some(Self::Grok),
            _ => None,
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Self::Claude => "subagent:claude-v1",
            Self::Codex => "subagent:codex-v1",
            Self::Grok => "subagent:grok-v1",
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct ReadBudget {
    pub head_bytes: usize,
    pub tail_bytes: usize,
    pub max_nodes: usize,
}
impl Default for ReadBudget {
    fn default() -> Self {
        Self {
            head_bytes: 64 * 1024,
            tail_bytes: 128 * 1024,
            max_nodes: 50,
        }
    }
}
impl ReadBudget {
    fn normalized(self) -> Self {
        let d = Self::default();
        Self {
            head_bytes: if self.head_bytes == 0 {
                d.head_bytes
            } else {
                self.head_bytes.min(d.head_bytes)
            },
            tail_bytes: if self.tail_bytes == 0 {
                d.tail_bytes
            } else {
                self.tail_bytes.min(d.tail_bytes)
            },
            max_nodes: self.max_nodes.min(d.max_nodes),
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReadStats {
    pub head_bytes: usize,
    pub tail_bytes: usize,
    pub parent_bytes: usize,
    pub metadata_bytes: usize,
    pub files_read: usize,
}
#[derive(Clone)]
pub enum Continuation {
    Claude(ClaudeState),
    Codex(CodexState),
    Grok(GrokState),
}
impl Continuation {
    fn for_adapter(adapter: Adapter) -> Self {
        match adapter {
            Adapter::Claude => Self::Claude(ClaudeState::default()),
            Adapter::Codex => Self::Codex(CodexState::default()),
            Adapter::Grok => Self::Grok(GrokState::default()),
        }
    }
}
pub struct ReadBatch {
    /// None is unavailable/no change, never erase. A completed empty scan is an
    /// explicit Some(empty tree), preserving the reference attempted-empty clear.
    pub tree: Option<SubagentTree>,
    pub continuation: Continuation,
    pub stats: ReadStats,
}
pub fn read_tree(
    adapter: Adapter,
    parent: &Path,
    since: Timestamp,
    prior: Option<Continuation>,
    budget: ReadBudget,
    now: Timestamp,
) -> io::Result<ReadBatch> {
    let budget = budget.normalized();
    let mut stats = ReadStats::default();
    let continuation = prior.unwrap_or_else(|| Continuation::for_adapter(adapter));
    let (tree, continuation) = match (adapter, continuation) {
        (Adapter::Claude, Continuation::Claude(state)) => {
            let (tree, next) = claude::read(
                parent,
                millis(since),
                state,
                budget,
                millis(now),
                &mut stats,
            )?;
            (tree, Continuation::Claude(next))
        }
        (Adapter::Codex, Continuation::Codex(state)) => {
            let (tree, next) = codex::read(parent, millis(since), state, budget, now, &mut stats)?;
            (tree, Continuation::Codex(next))
        }
        (Adapter::Grok, Continuation::Grok(state)) => {
            let (tree, next) = grok::read(
                parent,
                millis(since),
                state,
                budget,
                millis(now),
                &mut stats,
            )?;
            (tree, Continuation::Grok(next))
        }
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "subagent continuation belongs to another adapter",
            ));
        }
    };
    Ok(ReadBatch {
        tree,
        continuation,
        stats,
    })
}
/// Stateful broadcast/turn boundary helper. Callers guard asynchronous application
/// with generation; the session incarnation remains the session owner's check.
#[derive(Clone, Default)]
pub struct PollState {
    pub continuation: Option<Continuation>,
    pub tree: Option<SubagentTree>,
    pub turn_started_at: Option<Timestamp>,
    pub running_count: usize,
    pub generation: u64,
    signature: String,
}
impl PollState {
    pub fn confirmed_user_turn(&mut self, now: Timestamp) {
        if self.running_count == 0 {
            self.turn_started_at = Some(now);
        }
    }
    pub fn reattach(&mut self, started_at: &str, now: Timestamp) {
        if self.turn_started_at.is_none() {
            self.turn_started_at =
                Some(crate::proto::time::parse_rfc3339(started_at).unwrap_or(now));
        }
        self.signature.clear();
        self.generation = self.generation.wrapping_add(1);
    }
    pub fn stop(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }
    pub fn apply(&mut self, expected_generation: u64, batch: ReadBatch) -> Option<SubagentTree> {
        if self.generation != expected_generation {
            return None;
        }
        self.continuation = Some(batch.continuation);
        let tree = batch.tree?;
        self.running_count = tree.nodes.iter().filter(|n| n.state == "running").count();
        // An initial empty read does not need a redundant clear broadcast.
        if tree.nodes.is_empty() && self.tree.is_none() {
            return None;
        }
        let signature = tree_signature(&tree);
        self.tree = if tree.nodes.is_empty() {
            None
        } else {
            Some(tree.clone())
        };
        if signature == self.signature {
            return None;
        }
        self.signature = signature;
        Some(tree)
    }
}
pub fn tree_signature(tree: &SubagentTree) -> String {
    use std::fmt::Write;
    let mut value = format!("{}|{}", tree.provider, tree.omitted);
    for n in &tree.nodes {
        write!(
            value,
            "|N:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}",
            n.id,
            n.parent_id,
            n.depth,
            n.label,
            n.agent_type,
            n.model,
            n.state,
            n.started_at,
            n.last_activity_at,
            n.finished_at,
            n.tool_calls,
            n.last_tool_name,
            n.last_tool_summary
        )
        .unwrap();
    }
    let digest = Sha256::digest(value.as_bytes());
    let mut signature = String::with_capacity(digest.len() * 2);
    for byte in digest.iter() {
        write!(signature, "{byte:02x}").unwrap();
    }
    signature
}
fn null_default<'de, D: Deserializer<'de>, T: Deserialize<'de> + Default>(
    d: D,
) -> Result<T, D::Error> {
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}
fn millis(time: Timestamp) -> i64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis().min(i64::MAX as u128) as i64,
        Err(e) => {
            -(e.duration()
                .as_nanos()
                .div_ceil(1_000_000)
                .min(i64::MAX as u128) as i64)
        }
    }
}
fn parse_timestamp(value: &str) -> i64 {
    crate::proto::time::parse_rfc3339(value)
        .map(millis)
        .unwrap_or(0)
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Stamp {
    size: u64,
    modified: Option<Timestamp>,
}
impl Stamp {
    fn read(path: &Path) -> io::Result<Self> {
        let m = fs::metadata(path)?;
        if !m.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "subagent record is not a file",
            ));
        }
        Ok(Self {
            size: m.len(),
            modified: m
                .modified()
                .ok()
                .and_then(|at| Timestamp::from_system_time(at).ok()),
        })
    }
    fn millis(self) -> i64 {
        self.modified.map(millis).unwrap_or(0)
    }
}
fn list(path: &Path) -> io::Result<Vec<fs::DirEntry>> {
    let mut entries = fs::read_dir(path)?.collect::<io::Result<Vec<_>>>()?;
    entries.sort_by_key(|e| e.file_name());
    Ok(entries)
}
fn limited(path: &Path, cap: usize) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?.take(cap as u64).read_to_end(&mut bytes)?;
    Ok(bytes)
}
/// Newest first, whole newline-terminated records only. Windows beginning in a
/// record and incomplete final records are excluded, just like agent_chat_parse.go.
fn tail(path: &Path, cap: usize, record_cap: usize) -> io::Result<(Vec<Vec<u8>>, usize)> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    let start = size.saturating_sub(cap as u64);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = vec![];
    let expected = usize::try_from(size - start).map_err(io::Error::other)?;
    file.take(expected as u64).read_to_end(&mut bytes)?;
    let n = bytes.len();
    if n != expected {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "subagent snapshot shortened while reading",
        ));
    }
    let Some(last) = bytes.iter().rposition(|b| *b == b'\n') else {
        return Ok((vec![], n));
    };
    let mut end = last + 1;
    let mut records = vec![];
    while end > 0 && records.len() < record_cap {
        let begin = bytes[..end - 1]
            .iter()
            .rposition(|b| *b == b'\n')
            .map_or(0, |i| i + 1);
        if begin == 0 && start > 0 {
            break;
        }
        records.push(bytes[begin..end - 1].to_vec());
        end = begin;
    }
    Ok((records, n))
}
fn read_tail(path: &Path, cap: usize, parent: bool, stats: &mut ReadStats) -> Vec<Vec<u8>> {
    match tail(path, cap, if parent { PARENT_RECORDS } else { 512 }) {
        Ok((records, n)) => {
            stats.files_read += 1;
            if parent {
                stats.parent_bytes += n;
            } else {
                stats.tail_bytes += n;
            }
            records
        }
        Err(_) => vec![],
    }
}
fn read_meta<T: serde::de::DeserializeOwned + Default>(
    path: &Path,
    stats: &mut ReadStats,
) -> Option<T> {
    if Stamp::read(path).ok()?.size > METADATA_BYTES as u64 {
        return None;
    }
    let bytes = limited(path, METADATA_BYTES).ok()?;
    stats.metadata_bytes += bytes.len();
    stats.files_read += 1;
    decode_default(&bytes).ok()
}
fn decode_default<T: serde::de::DeserializeOwned + Default>(
    bytes: &[u8],
) -> Result<T, serde_json::Error> {
    serde_json::from_slice::<Option<T>>(bytes).map(Option::unwrap_or_default)
}
fn clamp_summary(raw: &str) -> String {
    let text = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= 100 {
        text
    } else {
        text.chars().take(99).collect::<String>() + "…"
    }
}
fn drop_orphans(nodes: &mut BTreeMap<String, SubagentNode>) {
    loop {
        let remove: Vec<_> = nodes
            .iter()
            .filter(|(_, n)| !n.parent_id.is_empty() && !nodes.contains_key(&n.parent_id))
            .map(|(id, _)| id.clone())
            .collect();
        if remove.is_empty() {
            break;
        }
        for id in remove {
            nodes.remove(&id);
        }
    }
}
fn finish_tree(
    adapter: Adapter,
    mut nodes: BTreeMap<String, SubagentNode>,
    since: i64,
    max: usize,
    now: i64,
) -> SubagentTree {
    drop_orphans(&mut nodes);
    nodes.retain(|_, n| n.state == "running" || n.started_at >= since);
    drop_orphans(&mut nodes);
    let mut omitted = 0;
    while nodes.len() > max {
        let victim = nodes
            .values()
            .filter(|n| matches!(n.state.as_str(), "done" | "failed"))
            .min_by_key(|n| (n.finished_at, &n.id))
            .or_else(|| nodes.values().min_by_key(|n| (n.started_at, &n.id)))
            .map(|n| n.id.clone());
        let Some(victim) = victim else {
            break;
        };
        let old = nodes.len();
        nodes.remove(&victim);
        drop_orphans(&mut nodes);
        omitted += old - nodes.len();
    }
    let mut nodes: Vec<_> = nodes.into_values().collect();
    nodes.sort_by(|a, b| (a.started_at, &a.id).cmp(&(b.started_at, &b.id)));
    let omitted = if nodes.is_empty() { 0 } else { omitted as i64 };
    SubagentTree {
        provider: adapter.key().into(),
        nodes,
        omitted,
        updated_at: now,
    }
}
/// A provider-owned identifier is a single path component, never a traversal.
fn component(value: &str) -> bool {
    !value.is_empty() && value != "." && value != ".." && !value.contains(['/', '\\', '\0', ':'])
}
