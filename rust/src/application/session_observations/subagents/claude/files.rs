use super::*;
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Stamp {
    pub size: u64,
    pub modified: Option<Timestamp>,
}
impl Stamp {
    pub fn read(paths: &RuntimePaths, path: &Path) -> io::Result<Self> {
        let m = super::super::artifact_metadata(paths, path)?;
        Ok(Self {
            size: m.len(),
            modified: m
                .modified()
                .ok()
                .and_then(|v| Timestamp::from_system_time(v).ok()),
        })
    }
    pub fn millis(self) -> i64 {
        self.modified.map(millis).unwrap_or(0)
    }
}
pub(super) fn head_cap(b: &ReadBudget) -> usize {
    if b.head_bytes == 0 {
        64 * 1024
    } else {
        b.head_bytes.min(64 * 1024) as usize
    }
}
pub(super) fn tail_cap(b: &ReadBudget) -> usize {
    if b.tail_bytes == 0 {
        128 * 1024
    } else {
        b.tail_bytes.min(128 * 1024) as usize
    }
}
pub(super) fn millis(v: Timestamp) -> i64 {
    v.unix_seconds()
        .saturating_mul(1000)
        .saturating_add(i64::from(v.subsec_nanos() / 1_000_000))
}
pub(super) fn list(paths: &RuntimePaths, path: &Path) -> io::Result<Vec<super::super::Entry>> {
    super::super::entries(paths, path)
}
pub(super) fn limited(paths: &RuntimePaths, path: &Path, cap: usize) -> io::Result<Vec<u8>> {
    super::super::read_head(paths, path, cap as u64)
}
pub(super) fn read_meta<T: GoWire>(
    paths: &RuntimePaths,
    path: &Path,
    stats: &mut ReadStats,
) -> Option<T> {
    let mut file = super::super::open_artifact(paths, path).ok()?;
    let mut bytes = vec![];
    file.read_to_end(&mut bytes).ok()?;
    stats.metadata_bytes += bytes.len();
    stats.files_read += 1;
    wire::decode(&bytes).ok()
}
pub(super) fn decode_default<T: GoWire>(bytes: &[u8]) -> Result<T, serde_json::Error> {
    wire::decode(bytes)
}
fn tail(
    paths: &RuntimePaths,
    path: &Path,
    cap: usize,
    records_cap: usize,
) -> io::Result<(Vec<Vec<u8>>, usize)> {
    super::super::check_path(paths, path)?;
    let started = Instant::now();
    let deadline = Duration::from_millis(100);
    let mut file = super::super::open_artifact(paths, path)?;
    let size = file.metadata()?.len();
    let start = size.saturating_sub(cap as u64);
    let len = usize::try_from(size - start).map_err(io::Error::other)?;
    let mut bytes = vec![0; len];
    let mut loaded = len;
    let mut n = 0;
    while loaded > 0 {
        if started.elapsed() >= deadline {
            return Ok((vec![], n));
        }
        let begin = loaded.saturating_sub(64 * 1024);
        file.seek(SeekFrom::Start(start + begin as u64))?;
        file.read_exact(&mut bytes[begin..loaded])?;
        n += loaded - begin;
        loaded = begin;
    }
    let Some(last) = bytes.iter().rposition(|b| *b == b'\n') else {
        return Ok((vec![], n));
    };
    let mut end = last + 1;
    let mut records = vec![];
    while end > 0 && records.len() < records_cap {
        if started.elapsed() >= deadline {
            break;
        }
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
pub(super) fn read_tail(
    paths: &RuntimePaths,
    path: &Path,
    cap: usize,
    parent: bool,
    stats: &mut ReadStats,
) -> Vec<Vec<u8>> {
    match tail(paths, path, cap, if parent { PARENT_RECORDS } else { 512 }) {
        Ok((rows, n)) => {
            stats.files_read += 1;
            if parent {
                stats.parent_bytes += n
            } else {
                stats.tail_bytes += n
            }
            rows
        }
        Err(_) => vec![],
    }
}
fn go_space(c: char) -> bool {
    matches!(c,'\t'..='\r'|' '| '\u{85}'|'\u{a0}'|'\u{1680}'|'\u{2000}'..='\u{200a}'|'\u{2028}'|'\u{2029}'|'\u{202f}'|'\u{205f}'|'\u{3000}')
}
pub(super) fn clamp_summary(raw: &str) -> String {
    let s = raw
        .split(go_space)
        .filter(|v| !v.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if s.chars().count() <= 100 {
        s
    } else {
        s.chars().take(99).collect::<String>() + "…"
    }
}
pub(super) fn finish_tree(
    mut nodes: BTreeMap<String, SubagentNode>,
    since: i64,
    max: usize,
    now: i64,
) -> Option<SubagentTree> {
    super::drop_orphans(&mut nodes);
    nodes.retain(|_, n| n.state == "running" || n.started_at >= since);
    super::drop_orphans(&mut nodes);
    let mut omitted = 0;
    while nodes.len() > max {
        let Some(victim) = super::pick_drop_victim(&nodes) else {
            break;
        };
        let count = nodes.len();
        nodes.remove(&victim);
        super::drop_orphans(&mut nodes);
        omitted += count - nodes.len();
    }
    let mut nodes: Vec<_> = nodes.into_values().collect();
    nodes.sort_by(|a, b| (a.started_at, &a.id).cmp(&(b.started_at, &b.id)));
    if nodes.is_empty() {
        return None;
    }
    Some(SubagentTree {
        provider: "subagent:claude-v1".into(),
        nodes,
        omitted: omitted as i64,
        updated_at: now,
    })
}
