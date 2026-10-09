//! Source bounded metadata-only journal scanner; result bodies are never retained.
use std::{
    collections::BTreeSet,
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::Path,
    time::{Duration, Instant, SystemTime},
};
#[derive(Clone, Copy, Default, PartialEq)]
enum Mode {
    #[default]
    Root,
    Key,
    Colon,
    Value,
    String,
    Primitive,
    Composite,
    Comma,
    Complete,
}
#[derive(Clone, Default)]
pub struct Parser {
    mode: Mode,
    invalid: bool,
    is_key: bool,
    escape: bool,
    depth: i64,
    composite_string: bool,
    composite_escape: bool,
    key: String,
    capture: String,
    quoted: Vec<u8>,
    kind: String,
    agent: String,
}
impl Parser {
    fn begin(&mut self, is_key: bool) {
        self.mode = Mode::String;
        self.is_key = is_key;
        self.escape = false;
        self.capture.clear();
        self.quoted.clear();
        if !is_key && matches!(self.key.as_str(), "type" | "agentId") {
            self.capture = self.key.clone();
        }
    }
    fn byte(&mut self, b: u8) {
        if (self.is_key || !self.capture.is_empty()) && self.quoted.len() < 256 {
            self.quoted.push(b)
        }
    }
    fn invalidate(&mut self) {
        self.invalid = true;
        self.quoted.clear()
    }
    fn finish(&mut self) {
        let mut encoded = vec![b'"'];
        encoded.extend(&self.quoted);
        encoded.push(b'"');
        let Ok(value) = crate::proto::decode_http_json::<String>(&encoded) else {
            self.invalidate();
            return;
        };
        if self.is_key {
            self.key = value;
            self.mode = Mode::Colon
        } else {
            match self.capture.as_str() {
                "type" => self.kind = value,
                "agentId" => self.agent = value,
                _ => {}
            }
            self.mode = Mode::Comma
        }
        self.capture.clear();
        self.quoted.clear();
    }
    pub fn feed(&mut self, b: u8) {
        if self.invalid || self.mode == Mode::Complete {
            return;
        }
        if self.mode == Mode::String {
            if self.escape {
                self.byte(b);
                self.escape = false
            } else if b == b'\\' {
                self.byte(b);
                self.escape = true
            } else if b == b'"' {
                self.finish()
            } else {
                self.byte(b)
            }
            return;
        }
        if self.mode == Mode::Composite {
            if self.composite_string {
                if self.composite_escape {
                    self.composite_escape = false
                } else if b == b'\\' {
                    self.composite_escape = true
                } else if b == b'"' {
                    self.composite_string = false
                }
                return;
            }
            match b {
                b'"' => self.composite_string = true,
                b'{' | b'[' => self.depth += 1,
                b'}' | b']' => {
                    self.depth -= 1;
                    if self.depth <= 0 {
                        self.depth = 0;
                        self.mode = Mode::Comma
                    }
                }
                _ => {}
            }
            return;
        }
        match self.mode {
            Mode::Root => match b {
                b' ' | b'\t' | b'\r' => {}
                b'{' => self.mode = Mode::Key,
                _ => self.invalidate(),
            },
            Mode::Key => match b {
                b' ' | b'\t' | b'\r' | b'\n' => {}
                b'"' => self.begin(true),
                b'}' => self.mode = Mode::Complete,
                _ => self.invalidate(),
            },
            Mode::Colon => match b {
                b' ' | b'\t' | b'\r' => {}
                b':' => self.mode = Mode::Value,
                _ => self.invalidate(),
            },
            Mode::Value => match b {
                b' ' | b'\t' | b'\r' => {}
                b'"' => self.begin(false),
                b'{' | b'[' => {
                    self.mode = Mode::Composite;
                    self.depth = 1;
                    self.composite_string = false;
                    self.composite_escape = false
                }
                b't' | b'f' | b'n' | b'-' | b'0'..=b'9' => self.mode = Mode::Primitive,
                _ => self.invalidate(),
            },
            Mode::Primitive => match b {
                b',' => self.mode = Mode::Key,
                b'}' => self.mode = Mode::Complete,
                b' ' | b'\t' | b'\r' => self.mode = Mode::Comma,
                _ => {}
            },
            Mode::Comma => match b {
                b' ' | b'\t' | b'\r' => {}
                b',' => self.mode = Mode::Key,
                b'}' => self.mode = Mode::Complete,
                _ => self.invalidate(),
            },
            _ => {}
        }
    }
    pub fn event(&self) -> Option<(&str, &str)> {
        (!self.invalid
            && self.mode == Mode::Complete
            && !self.kind.is_empty()
            && !self.agent.is_empty())
        .then_some((&self.kind, &self.agent))
    }
}
#[derive(Clone, Default)]
pub struct FileState {
    pub offset: u64,
    pub started: BTreeSet<String>,
    pub results: BTreeSet<String>,
    pub frozen: bool,
    pub modified: Option<SystemTime>,
    pub last_event: Option<SystemTime>,
    pub line_bytes: u64,
    pub parser: Parser,
}
#[derive(Default)]
pub struct ReadStats {
    pub bytes: usize,
    pub records: usize,
    pub hit_budget: bool,
}
pub struct Budget {
    pub bytes: usize,
    pub records: usize,
    pub deadline: Instant,
}
impl Default for Budget {
    fn default() -> Self {
        Self {
            bytes: 4 * 1024 * 1024,
            records: 256,
            deadline: Instant::now() + Duration::from_millis(100),
        }
    }
}
pub fn tail(path: &Path, state: FileState, budget: &Budget) -> io::Result<(FileState, ReadStats)> {
    tail_opened(File::open(path)?, state, budget)
}
pub fn tail_opened(
    mut file: File,
    mut state: FileState,
    budget: &Budget,
) -> io::Result<(FileState, ReadStats)> {
    let metadata = file.metadata()?;
    state.modified = Some(metadata.modified()?);
    let mut stats = ReadStats::default();
    if state.frozen {
        return Ok((state, stats));
    }
    if metadata.len() < state.offset {
        state.frozen = true;
        return Ok((state, stats));
    }
    if metadata.len() == state.offset {
        return Ok((state, stats));
    }
    file.seek(SeekFrom::Start(state.offset))?;
    let mut buffer = [0u8; 64 * 1024];
    let max_bytes = if budget.bytes == 0 {
        4 * 1024 * 1024
    } else {
        budget.bytes
    };
    let max_records = if budget.records == 0 {
        256
    } else {
        budget.records
    };
    'reading: while stats.records < max_records && stats.bytes < max_bytes {
        if Instant::now() >= budget.deadline {
            stats.hit_budget = true;
            break;
        }
        let count = file.read(&mut buffer[..(max_bytes - stats.bytes).min(64 * 1024)])?;
        stats.bytes += count;
        for &b in &buffer[..count] {
            if Instant::now() >= budget.deadline {
                stats.hit_budget = true;
                break 'reading;
            }
            state.offset += 1;
            state.line_bytes += 1;
            if b != b'\n' {
                state.parser.feed(b);
                continue;
            }
            if let Some((kind, agent)) = state.parser.event() {
                let changed = match kind {
                    "started" => state.started.insert(agent.into()),
                    "result" => state.results.insert(agent.into()),
                    _ => false,
                };
                if changed {
                    state.last_event = state.modified
                }
            }
            state.line_bytes = 0;
            state.parser = Parser::default();
            stats.records += 1;
            if stats.records >= max_records {
                stats.hit_budget = true;
                break 'reading;
            }
        }
        if stats.bytes >= max_bytes || Instant::now() >= budget.deadline {
            stats.hit_budget = true;
            break;
        }
        if count == 0 {
            break;
        }
    }
    Ok((state, stats))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn partial_record_retains_only_metadata_and_truncate_freezes_counts() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("journal.jsonl");
        std::fs::write(&path,b"{\"type\":\"started\",\"agentId\":\"a\"}\n{\"type\":\"result\",\"agentId\":\"a\",\"result\":\"").unwrap();
        let (state, _) = tail(&path, FileState::default(), &Budget::default()).unwrap();
        assert_eq!(state.started.len(), 1);
        assert!(state.results.is_empty());
        assert!(state.parser.quoted.is_empty());
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        for _ in 0..1024 {
            file.write_all(&[b'x'; 1024]).unwrap()
        }
        file.write_all(b"\"}\n").unwrap();
        drop(file);
        let mut state = state;
        for _ in 0..50 {
            state = tail(&path, state, &Budget::default()).unwrap().0;
            assert!(state.parser.quoted.len() <= 256);
            if !state.results.is_empty() {
                break;
            }
        }
        assert_eq!(state.results.len(), 1);
        assert_eq!(state.offset, std::fs::metadata(&path).unwrap().len());
        assert!(state.parser.quoted.is_empty());
        std::fs::write(&path, b"{\"type\":\"started\",\"agentId\":\"new\"}\n").unwrap();
        let (state, _) = tail(&path, state, &Budget::default()).unwrap();
        assert!(state.frozen);
        assert_eq!(state.started.len(), 1);
        assert_eq!(state.results.len(), 1);
    }
    #[test]
    fn budget_offset_stops_at_counted_record_and_replays_no_completed_record() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("journal.jsonl");
        std::fs::write(
            &path,
            b"{\"type\":\"started\",\"agentId\":\"a\"}\n{\"type\":\"result\",\"agentId\":\"a\"}\n",
        )
        .unwrap();
        let budget = Budget {
            records: 1,
            ..Default::default()
        };
        let (state, stats) = tail(&path, FileState::default(), &budget).unwrap();
        assert!(stats.hit_budget);
        assert_eq!(stats.records, 1);
        assert_eq!(state.started.len(), 1);
        assert_eq!(state.results.len(), 0);
        let (state, stats) = tail(&path, state, &Budget::default()).unwrap();
        assert_eq!(stats.records, 1);
        assert_eq!(state.started.len(), 1);
        assert_eq!(state.results.len(), 1);
    }
}
