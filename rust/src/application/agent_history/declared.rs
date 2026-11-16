//! Conversation file lookup and reading driven by a provider's `transcript`
//! declaration (see `proto::provider::TranscriptDefinition`). The built-ins
//! whose lookup is exactly expressible use the same code.
use super::discovery::{
    Location, distance, existing_dir, existing_file, same_path, slug_alnum, trim_cwd, uuid,
};
use crate::{
    application::session_observations::subagents::{entries, open_artifact},
    approval::transcript::parser::AgentChatMessage,
    config::RuntimePaths,
    proto::{
        core::TranscriptSessionIdentity,
        provider::{TranscriptDefinition, TranscriptReader},
        time::parse_rfc3339,
    },
};
use serde_json::Value;
use std::{
    io::{self, BufRead, BufReader, Read},
    path::{Path, PathBuf},
    time::Duration,
};

const WINDOW: Duration = Duration::from_secs(600);
const MAX_READ: u64 = 8 * 1024 * 1024;
const TEXT_MAX: usize = 64 * 1024;

/// The `transcript` declaration that applies to `provider`, if any.
pub(super) fn declaration(provider: &str) -> Option<TranscriptDefinition> {
    crate::profile::declared::with_declared(provider, |d| d.transcript.clone()).flatten()
}

fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Resolve a path pattern for this session. `None` when a part it needs
/// (home, a variable, the working directory) is missing or the result is not
/// absolute. Segments are joined one by one, so the result equals a hand
/// written `Path::join` chain.
pub(super) fn expand(
    pattern: &str,
    identity: &TranscriptSessionIdentity,
    env: &dyn Fn(&str) -> Option<String>,
) -> Option<PathBuf> {
    let mut out: Option<PathBuf> = None;
    for segment in pattern.split(['/', '\\']).filter(|s| !s.is_empty()) {
        let mut text = String::new();
        let mut rest = segment;
        while let Some(open) = rest.find('{') {
            text.push_str(&rest[..open]);
            let close = rest[open..].find('}')? + open;
            let token = &rest[open + 1..close];
            let value = if token == "home" {
                non_empty(&identity.home_dir)?
            } else if let Some(name) = token.strip_prefix("env:") {
                non_empty(&env(name)?)?
            } else if token == "cwd_slug:alnum" {
                trim_cwd(&identity.cwd)?;
                slug_alnum(&identity.cwd)
            } else if token == "cwd_slug:path" {
                trim_cwd(&identity.cwd)?;
                crate::files::scope::clean(Path::new(&identity.cwd))
                    .to_string_lossy()
                    .replace(['\\', '/', ':'], "-")
            } else {
                return None;
            };
            text.push_str(&value);
            rest = &rest[close + 1..];
        }
        text.push_str(rest);
        match out.as_mut() {
            None => out = Some(PathBuf::from(text)),
            Some(path) => path.push(text),
        }
    }
    out.filter(|p| p.is_absolute())
}

fn non_empty(value: &str) -> Option<String> {
    let value = super::trim_space(value);
    (!value.is_empty()).then(|| value.to_owned())
}

fn process_env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

fn key_path<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    key.split('.').try_fold(value, |v, part| v.get(part))
}

fn extension(t: &TranscriptDefinition) -> &str {
    if t.extension.is_empty() {
        "json"
    } else {
        &t.extension
    }
}

fn needs_cwd(t: &TranscriptDefinition) -> bool {
    t.find.iter().any(|s| s == "meta" || s == "index")
}

fn session_file(
    paths: &RuntimePaths,
    t: &TranscriptDefinition,
    root: &Path,
    id: &str,
) -> Option<PathBuf> {
    let ok = if t.id_format == "uuid" {
        uuid(id)
    } else {
        safe_id(id)
    };
    if !ok {
        return None;
    }
    let path = root.join(format!("{id}.{}", extension(t)));
    existing_file(paths, &path).then_some(path)
}

/// Pick the candidate nearest to the session start. Two candidates less than a
/// second apart make the answer ambiguous, so none is chosen.
fn pick(candidates: Vec<(PathBuf, Duration)>) -> Option<PathBuf> {
    let mut best: Option<(PathBuf, Duration)> = None;
    let mut second: Option<Duration> = None;
    for (path, delta) in candidates {
        if best.as_ref().is_none_or(|(_, d)| delta < *d) {
            if let Some((_, d)) = &best {
                second = Some(*d);
            }
            best = Some((path, delta));
        } else if second.is_none_or(|d| delta < d) {
            second = Some(delta);
        }
    }
    let (path, best) = best?;
    if second.is_some_and(|s| s.saturating_sub(best) < Duration::from_secs(1)) {
        None
    } else {
        Some(path)
    }
}

fn first_line(paths: &RuntimePaths, path: &Path) -> Option<Vec<u8>> {
    let mut reader = BufReader::with_capacity(64 * 1024, open_artifact(paths, path).ok()?);
    let mut line = vec![];
    let n = reader
        .by_ref()
        .take(4 * 1024 * 1024 + 1)
        .read_until(b'\n', &mut line)
        .ok()?;
    (n > 0 && line.len() <= 4 * 1024 * 1024).then_some(line)
}

fn by_meta(
    paths: &RuntimePaths,
    t: &TranscriptDefinition,
    root: &Path,
    identity: &TranscriptSessionIdentity,
) -> Option<PathBuf> {
    let meta = t.meta.as_ref()?;
    let started = parse_rfc3339(&identity.started_at).ok()?;
    let suffix = format!(".{}", extension(t));
    let mut found = vec![];
    for entry in entries(paths, root).unwrap_or_default() {
        if entry.is_dir || !entry.name.ends_with(&suffix) {
            continue;
        }
        let path = root.join(&entry.name);
        let Some(value) =
            first_line(paths, &path).and_then(|line| serde_json::from_slice::<Value>(&line).ok())
        else {
            continue;
        };
        if !meta.kind_key.is_empty()
            && key_path(&value, &meta.kind_key).and_then(Value::as_str)
                != Some(meta.kind_value.as_str())
        {
            continue;
        }
        let (Some(cwd), Some(stamp)) = (
            key_path(&value, &meta.cwd_key).and_then(Value::as_str),
            key_path(&value, &meta.time_key).and_then(Value::as_str),
        ) else {
            continue;
        };
        if !same_path(cwd, &identity.cwd) {
            continue;
        }
        let Some(delta) = parse_rfc3339(stamp).ok().and_then(|v| distance(v, started)) else {
            continue;
        };
        if delta <= WINDOW {
            found.push((path, delta));
        }
    }
    pick(found)
}

fn by_index(
    paths: &RuntimePaths,
    t: &TranscriptDefinition,
    root: &Path,
    identity: &TranscriptSessionIdentity,
    env: &dyn Fn(&str) -> Option<String>,
) -> Option<PathBuf> {
    let index = t.index.as_ref()?;
    let started = parse_rfc3339(&identity.started_at).ok()?;
    let db = expand(&index.file, identity, env)?;
    if paths.is_trial() {
        paths.relative_to_selected_root(&db).ok()?;
    }
    if !db.is_file() {
        return None;
    }
    let ms = index.time_unit == "ms";
    let at = if ms {
        i128::from(started.unix_seconds()) * 1000
    } else {
        i128::from(started.unix_seconds())
    } as i64;
    let window = WINDOW.as_secs() as i64 * if ms { 1000 } else { 1 };
    let connection =
        rusqlite::Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .ok()?;
    connection.busy_timeout(Duration::from_millis(200)).ok()?;
    connection.execute_batch("PRAGMA query_only=ON").ok()?;
    // Names were validated as plain identifiers when the definition was accepted.
    let sql = format!(
        "SELECT \"{}\", \"{}\", \"{}\" FROM \"{}\" WHERE \"{}\" BETWEEN ?1 AND ?2",
        index.id_column, index.cwd_column, index.time_column, index.table, index.time_column
    );
    let mut statement = connection.prepare(&sql).ok()?;
    let rows = statement
        .query_map([at - window, at + window], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .ok()?;
    let mut candidates: Vec<(String, Duration)> = vec![];
    for (id, cwd, time) in rows.flatten() {
        if !same_path(&cwd, &identity.cwd) {
            continue;
        }
        let diff = (time - at).unsigned_abs();
        candidates.push((
            id,
            if ms {
                Duration::from_millis(diff)
            } else {
                Duration::from_secs(diff)
            },
        ));
    }
    // The tie rule applies to the sessions, whether or not their files exist.
    let ids = candidates
        .iter()
        .map(|(id, d)| (PathBuf::from(id), *d))
        .collect();
    let id = pick(ids)?;
    session_file(paths, t, root, &id.to_string_lossy())
}

pub(super) fn find_with(
    paths: &RuntimePaths,
    identity: &TranscriptSessionIdentity,
    t: &TranscriptDefinition,
    env: &dyn Fn(&str) -> Option<String>,
) -> Option<PathBuf> {
    if identity.cwd.is_empty() || t.root.is_empty() {
        return None;
    }
    let root = expand(&t.root, identity, env)?;
    for step in &t.find {
        let found = match step.as_str() {
            "session_id" => session_file(paths, t, &root, &identity.agent_session_id),
            "meta" => by_meta(paths, t, &root, identity),
            "index" => by_index(paths, t, &root, identity, env),
            _ => None,
        };
        if found.is_some() {
            return found;
        }
    }
    None
}

/// The conversation file of this session, or `None`.
pub(super) fn find(
    paths: &RuntimePaths,
    identity: &TranscriptSessionIdentity,
    t: &TranscriptDefinition,
) -> Option<PathBuf> {
    find_with(paths, identity, t, &process_env)
}

pub(super) fn location_with(
    paths: &RuntimePaths,
    identity: &TranscriptSessionIdentity,
    t: &TranscriptDefinition,
    env: &dyn Fn(&str) -> Option<String>,
) -> Location {
    let text =
        |value: &str, fallback: &str| if value.is_empty() { fallback } else { value }.to_owned();
    let label = text(&t.label, "Transcript");
    let unavailable = text(&t.unavailable_reason, "Transcript location is unavailable");
    let not_found = text(&t.not_found_reason, "Transcript not found yet");
    if !t.file.is_empty() {
        let Some(path) = expand(&t.file, identity, env) else {
            return Location::missing_owned(unavailable);
        };
        return if existing_file(paths, &path) {
            Location::found(path, &label)
        } else {
            Location::missing_owned(not_found)
        };
    }
    let Some(root) = expand(&t.root, identity, env) else {
        return Location::missing_owned(unavailable);
    };
    if identity.cwd.is_empty() && needs_cwd(t) {
        return Location::missing_owned(unavailable);
    }
    if let Some(path) = find_with(paths, identity, t, env) {
        return Location::found(path, &label);
    }
    if t.dir_fallback && existing_dir(paths, &root) {
        return Location::found(root, &text(&t.dir_label, &label));
    }
    Location::missing_owned(not_found)
}

pub(super) fn location(
    paths: &RuntimePaths,
    identity: &TranscriptSessionIdentity,
    t: &TranscriptDefinition,
) -> Location {
    location_with(paths, identity, t, &process_env)
}

fn cap(text: &str) -> String {
    let text = text.trim();
    if text.len() <= TEXT_MAX {
        return text.to_owned();
    }
    let mut cut = TEXT_MAX;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}…", &text[..cut])
}

fn message(item: &Value, index: usize, r: &TranscriptReader) -> Option<AgentChatMessage> {
    let role = key_path(item, &r.role_key)?.as_str()?;
    let role = r.roles.get(role)?;
    let text = key_path(item, &r.text_key)?.as_str()?;
    let text = cap(&crate::storage::mask_secrets(text.trim()));
    if text.is_empty() {
        return None;
    }
    let ts = if r.time_key.is_empty() {
        String::new()
    } else {
        key_path(item, &r.time_key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    Some(AgentChatMessage {
        role: role.clone(),
        kind: "text".into(),
        text,
        ts,
        message_id: format!("m{index}"),
        ..Default::default()
    })
}

/// All readable messages of the file, oldest first.
pub(super) fn read_messages(
    paths: &RuntimePaths,
    path: &Path,
    r: &TranscriptReader,
) -> io::Result<Vec<AgentChatMessage>> {
    let mut bytes = vec![];
    open_artifact(paths, path)?
        .take(MAX_READ + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_READ {
        return Err(io::Error::other("transcript too large"));
    }
    let mut out = vec![];
    if r.format == "jsonl" {
        for (i, line) in bytes.split(|b| *b == b'\n').enumerate() {
            if let Ok(item) = serde_json::from_slice::<Value>(line)
                && let Some(m) = message(&item, i, r)
            {
                out.push(m);
            }
        }
    } else {
        let doc: Value = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        let items = key_path(&doc, &r.messages)
            .and_then(Value::as_array)
            .ok_or_else(|| io::Error::other("message array missing"))?;
        for (i, item) in items.iter().enumerate() {
            if let Some(m) = message(item, i, r) {
                out.push(m);
            }
        }
    }
    Ok(out)
}

/// One page: up to `limit` messages that end before index `cursor` (`None` =
/// the newest). Returns the messages and the index of the first one.
pub(super) fn page(
    messages: Vec<AgentChatMessage>,
    limit: usize,
    cursor: Option<usize>,
) -> (Vec<AgentChatMessage>, usize, usize) {
    let total = messages.len();
    let end = cursor.map_or(total, |c| c.min(total));
    let start = end.saturating_sub(limit);
    (messages[start..end].to_vec(), start, total)
}
