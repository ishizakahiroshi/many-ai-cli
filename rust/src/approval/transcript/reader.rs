//! Source: agent_chat_parse.go readAgentChatRange/readAgentChatTailPageWithBudget.
use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
pub const MAX_RECORD: usize = 8 * 1024 * 1024;
pub const READ_BUFFER: usize = 64 * 1024;
pub const READ_BYTES: usize = 4 * 1024 * 1024;
pub const READ_RECORDS: usize = 256;
pub const PAGE_BYTES: usize = 16 * 1024 * 1024;
pub const PAGE_RECORDS: usize = 512;
pub const READ_TIME: Duration = Duration::from_millis(100);
#[derive(Clone)]
pub struct ReadBudget {
    pub max_bytes: usize,
    pub max_records: usize,
    pub deadline: Instant,
    pub clock: Arc<dyn Fn() -> Instant + Send + Sync>,
}
impl ReadBudget {
    pub fn live() -> Self {
        Self {
            max_bytes: READ_BYTES,
            max_records: READ_RECORDS,
            deadline: Instant::now() + READ_TIME,
            clock: Arc::new(Instant::now),
        }
    }
    pub fn page() -> Self {
        Self {
            max_bytes: PAGE_BYTES,
            max_records: PAGE_RECORDS,
            ..Self::live()
        }
    }
    pub fn expired(&self) -> bool {
        (self.clock)() >= self.deadline
    }
}
#[derive(Clone, Debug, Default)]
pub struct ReadState {
    pub next_offset: u64,
    pub safe_offset: u64,
    line_bytes: usize,
    record: Vec<u8>,
    oversized: bool,
}
impl ReadState {
    pub fn reset(&mut self, offset: u64) {
        *self = Self {
            next_offset: offset,
            safe_offset: offset,
            ..Self::default()
        };
    }
    pub fn retained_bytes(&self) -> usize {
        self.record.len()
    }
    pub fn has_partial_record(&self) -> bool {
        self.line_bytes != 0
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReadStats {
    pub offset: u64,
    pub safe_offset: u64,
    pub snapshot_end: u64,
    pub bytes_read: usize,
    pub records: usize,
    pub decoded_records: usize,
    pub hit_budget: bool,
    pub complete: bool,
    pub tail_page_ready: bool,
    pub decode_committed: bool,
}
fn fragment<F>(
    state: &mut ReadState,
    bytes: &[u8],
    complete: bool,
    stats: &mut ReadStats,
    handle: &mut F,
) -> io::Result<()>
where
    F: FnMut(&[u8], u64, u64) -> io::Result<()>,
{
    if bytes.is_empty() {
        return Ok(());
    }
    state.line_bytes = state.line_bytes.saturating_add(bytes.len());
    state.next_offset = state.next_offset.saturating_add(bytes.len() as u64);
    if !state.oversized && state.line_bytes <= MAX_RECORD {
        state.record.extend_from_slice(bytes);
    } else {
        state.oversized = true;
        state.record.clear();
    }
    stats.offset = state.next_offset;
    if !complete {
        return Ok(());
    }
    stats.safe_offset = state.next_offset;
    stats.records += 1;
    if !state.oversized && !state.record.is_empty() {
        handle(
            &state.record[..state.record.len() - 1],
            state.next_offset - state.line_bytes as u64,
            state.next_offset,
        )?;
        stats.decoded_records += 1;
    }
    state.line_bytes = 0;
    state.record.clear();
    state.oversized = false;
    state.safe_offset = state.next_offset;
    Ok(())
}
pub fn read_range<F>(
    path: &Path,
    offset: u64,
    state: &mut ReadState,
    budget: &ReadBudget,
    mut handle: F,
) -> io::Result<ReadStats>
where
    F: FnMut(&[u8], u64, u64) -> io::Result<()>,
{
    if state.next_offset != offset {
        state.reset(offset);
    }
    let mut stats = ReadStats {
        offset: state.next_offset,
        safe_offset: state.safe_offset,
        ..Default::default()
    };
    let metadata = path.metadata()?;
    if metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "transcript path is a directory",
        ));
    }
    if metadata.len() < offset {
        stats.complete = true;
        return Ok(stats);
    }
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(offset))?;
    let max_bytes = if budget.max_bytes == 0 {
        READ_BYTES
    } else {
        budget.max_bytes
    };
    let max_records = if budget.max_records == 0 {
        READ_RECORDS
    } else {
        budget.max_records
    };
    let mut buffer = vec![0; READ_BUFFER];
    while stats.bytes_read < max_bytes && stats.records < max_records {
        if budget.expired() {
            stats.hit_budget = true;
            break;
        }
        let count = buffer.len().min(max_bytes - stats.bytes_read);
        let n = file.read(&mut buffer[..count])?;
        stats.bytes_read += n;
        let mut pos = 0;
        while pos < n {
            if budget.expired() {
                stats.hit_budget = true;
                break;
            }
            let end = buffer[pos..n]
                .iter()
                .position(|b| *b == b'\n')
                .map(|i| pos + i + 1);
            match end {
                Some(end) => {
                    fragment(state, &buffer[pos..end], true, &mut stats, &mut handle)?;
                    pos = end;
                }
                None => {
                    fragment(state, &buffer[pos..n], false, &mut stats, &mut handle)?;
                    break;
                }
            }
            if stats.records >= max_records {
                stats.hit_budget = true;
                break;
            }
        }
        if stats.records >= max_records || stats.bytes_read >= max_bytes || budget.expired() {
            stats.hit_budget = true;
            break;
        }
        if n == 0 {
            stats.complete = state.line_bytes == 0;
            break;
        }
    }
    if stats.bytes_read >= max_bytes || stats.records >= max_records || budget.expired() {
        stats.hit_budget = true;
    }
    stats.offset = state.next_offset;
    stats.safe_offset = state.safe_offset;
    if state.line_bytes == 0 && stats.offset >= metadata.len() {
        stats.complete = true;
    }
    Ok(stats)
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TailRecord {
    pub start: u64,
    pub end: u64,
    pub line: Vec<u8>,
}
/// Returns selected records in newest-first order. Parser commits them oldest
/// first. SafeOffset is newline-aligned and must not be replaced by file size.
pub fn read_tail_page(
    path: &Path,
    max_records: usize,
    end_offset: Option<u64>,
    budget: &ReadBudget,
) -> io::Result<(Vec<TailRecord>, ReadStats)> {
    let max_records = if max_records == 0 {
        PAGE_RECORDS
    } else {
        max_records.min(PAGE_RECORDS)
    };
    let max_bytes = if budget.max_bytes == 0 {
        PAGE_BYTES
    } else {
        budget.max_bytes
    };
    let budget_records = if budget.max_records == 0 {
        PAGE_RECORDS
    } else {
        budget.max_records
    };
    let metadata = path.metadata()?;
    if metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "transcript path is a directory",
        ));
    }
    let end = end_offset.unwrap_or(metadata.len()).min(metadata.len());
    let size = end.min(max_bytes as u64) as usize;
    let start = end - size as u64;
    let mut stats = ReadStats {
        snapshot_end: end,
        hit_budget: end > max_bytes as u64,
        ..Default::default()
    };
    let mut window = vec![0; size];
    let mut file = File::open(path)?;
    let mut loaded_start = size;
    while loaded_start > 0 {
        if budget.expired() {
            stats.hit_budget = true;
            break;
        }
        let chunk_start = loaded_start.saturating_sub(READ_BUFFER);
        file.seek(SeekFrom::Start(start + chunk_start as u64))?;
        let chunk = &mut window[chunk_start..loaded_start];
        let mut read = 0;
        while read < chunk.len() {
            let n = file.read(&mut chunk[read..])?;
            stats.bytes_read += n;
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "transcript changed during tail snapshot",
                ));
            }
            read += n;
        }
        loaded_start = chunk_start;
    }
    if loaded_start == size {
        stats.offset = end;
        stats.safe_offset = end;
        stats.tail_page_ready = true;
        stats.complete = true;
        return Ok((Vec::new(), stats));
    }
    if loaded_start > 0 {
        stats.hit_budget = true;
        stats.offset = end;
        return Ok((Vec::new(), stats));
    }
    let mut complete_end = size;
    if window.last() != Some(&b'\n') {
        if let Some(last) = window.iter().rposition(|b| *b == b'\n') {
            complete_end = last + 1;
        } else {
            stats.offset = end;
            if budget.expired() {
                stats.hit_budget = true;
                return Ok((Vec::new(), stats));
            }
            stats.safe_offset = if start > 0 { end } else { 0 };
            stats.tail_page_ready = true;
            return Ok((Vec::new(), stats));
        }
    }
    let mut records = Vec::new();
    let mut pos = complete_end;
    let mut ready = true;
    while pos > 0 && records.len() < max_records {
        if budget.expired() {
            stats.hit_budget = true;
            stats.offset = start + pos as u64;
            ready = false;
            break;
        }
        let line_start = window[..pos - 1]
            .iter()
            .rposition(|b| *b == b'\n')
            .map(|i| i + 1)
            .unwrap_or(0);
        if line_start == 0 && start > 0 {
            break;
        }
        records.push(TailRecord {
            start: start + line_start as u64,
            end: start + pos as u64,
            line: window[line_start..pos - 1].to_vec(),
        });
        pos = line_start;
    }
    if !stats.hit_budget {
        stats.offset = records.last().map(|r| r.start).unwrap_or(end);
    }
    stats.safe_offset = start + complete_end as u64;
    stats.records = records.len();
    stats.complete = complete_end == size;
    stats.tail_page_ready = ready;
    stats.hit_budget |= records.len() >= max_records.min(budget_records);
    Ok((records, stats))
}
pub fn tail_record_limit(max_messages: usize) -> usize {
    (if max_messages == 0 { 200 } else { max_messages })
        .saturating_mul(2)
        .saturating_add(16)
        .min(PAGE_RECORDS)
}
