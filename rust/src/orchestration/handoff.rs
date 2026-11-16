//! Typed path-only handoff records. Source: internal/handoff/{handoff,render}.go, 21d0bc7.
//! Do not add transcript bodies, dynamic metadata, raw input, diffs or environment.
use crate::proto::time::Timestamp;
use crate::{
    config::{Resource, RuntimePaths, private_io},
    files::safe_fs::Dir,
    proto::{
        time::format_rfc3339,
        wire::{Field, GoWire, Schema},
    },
    storage::mask_secrets,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    io::{self, BufRead, Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

pub const RECORD_VERSION: i64 = 1;
pub const KIND_SESSION_START: &str = "session_start";
pub const KIND_SESSION_END: &str = "session_end";
pub const KIND_DONE: &str = "done";
pub const KIND_GIT_TURN: &str = "git_turn";
pub const KIND_INTENT: &str = "intent";
pub const KIND_TURN_SUMMARY: &str = "turn_summary";
pub const KIND_TRANSCRIPT: &str = "transcript";
pub const KIND_NOTE: &str = "note";
pub const TEXT_MAX_RUNES: usize = 320;
pub const FILES_MAX_COUNT: usize = 20;
pub const RENDER_MAX_DONE_ENTRIES: usize = 8;
pub const RENDER_MAX_GIT_TURN_ENTRIES: usize = 6;
pub const MAX_RECORD_BYTES: u64 = 1 << 20;
/// Session numbers above this found in handoff file names are ignored when the
/// Hub picks its first session number, so one odd file name cannot exhaust the
/// counter.
pub const MAX_SEEDED_SESSION_ID: i64 = 1 << 31;
static APPEND_LOCK: Mutex<()> = Mutex::new(());
fn null_files<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    Ok(Option::<Vec<String>>::deserialize(d)?.unwrap_or_default())
}
fn is_zero(v: &i64) -> bool {
    *v == 0
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Record {
    pub version: i64,
    pub ts: String,
    pub session_id: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub provider: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub cwd: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub branch: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub model: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub subscription_id: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(deserialize_with = "null_files")]
    pub files: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub commit: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub commit_subject: String,
    #[serde(skip_serializing_if = "is_zero")]
    pub added: i64,
    #[serde(skip_serializing_if = "is_zero")]
    pub removed: i64,
    #[serde(skip_serializing_if = "is_zero")]
    pub files_changed: i64,
    #[serde(skip_serializing_if = "is_zero")]
    pub turn: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub work_doc: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(skip_serializing_if = "is_zero")]
    pub handoff_from: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub transcript: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
}
impl GoWire for Record {
    const GO_TYPE: &'static str = "HandoffRecord";
    const SCHEMAS: &'static [Schema] = &[Schema {
        name: "HandoffRecord",
        fields: &[
            Field {
                name: "version",
                kind: "int",
            },
            Field {
                name: "ts",
                kind: "string",
            },
            Field {
                name: "session_id",
                kind: "int",
            },
            Field {
                name: "provider",
                kind: "string",
            },
            Field {
                name: "cwd",
                kind: "string",
            },
            Field {
                name: "branch",
                kind: "string",
            },
            Field {
                name: "model",
                kind: "string",
            },
            Field {
                name: "subscription_id",
                kind: "string",
            },
            Field {
                name: "kind",
                kind: "string",
            },
            Field {
                name: "files",
                kind: "[]string",
            },
            Field {
                name: "commit",
                kind: "string",
            },
            Field {
                name: "commit_subject",
                kind: "string",
            },
            Field {
                name: "added",
                kind: "int",
            },
            Field {
                name: "removed",
                kind: "int",
            },
            Field {
                name: "files_changed",
                kind: "int",
            },
            Field {
                name: "turn",
                kind: "int",
            },
            Field {
                name: "work_doc",
                kind: "string",
            },
            Field {
                name: "text",
                kind: "string",
            },
            Field {
                name: "handoff_from",
                kind: "int",
            },
            Field {
                name: "transcript",
                kind: "string",
            },
            Field {
                name: "note",
                kind: "string",
            },
        ],
    }];
}
pub fn sanitize(mut record: Record) -> Record {
    record.text = mask_secrets(&record.text);
    if record.text.chars().count() > TEXT_MAX_RUNES {
        record.text = record.text.chars().take(TEXT_MAX_RUNES).collect::<String>() + "…";
    }
    if record.files.len() > FILES_MAX_COUNT {
        let omitted = record.files.len() - FILES_MAX_COUNT;
        record.files.truncate(FILES_MAX_COUNT);
        record.files.push(format!("…他 {omitted} 件"));
    }
    record
}
#[derive(Clone)]
pub struct HandoffStore {
    paths: RuntimePaths,
    /// Last transcript path appended per session. The transcript file changes
    /// constantly while the path rarely does; repeating it only grows the file.
    last_transcript: Arc<Mutex<HashMap<i64, String>>>,
}
impl HandoffStore {
    pub fn new(paths: RuntimePaths) -> Self {
        Self {
            paths,
            last_transcript: Arc::default(),
        }
    }
    pub fn directory(&self) -> PathBuf {
        self.paths.resource(Resource::Handoff)
    }
    fn path(&self, id: i64, suffix: &str) -> io::Result<PathBuf> {
        self.paths
            .checked_child(Resource::Handoff, Path::new(&format!("s{id}{suffix}")))
    }
    pub fn path_for(&self, id: i64) -> io::Result<PathBuf> {
        self.path(id, ".jsonl")
    }
    pub fn rendered_path_for(&self, id: i64) -> io::Result<PathBuf> {
        self.path(id, "_handoff.md")
    }
    pub fn note_path_for(&self, id: i64) -> io::Result<PathBuf> {
        self.path(id, ".note.md")
    }
    pub fn manual_note_path_for(&self, id: i64) -> io::Result<PathBuf> {
        self.path(id, ".manual.note.md")
    }
    /// Append one encoded record to the caller-selected store. The kernel append
    /// handle preserves prior bytes without reading them. Callers must perform
    /// this blocking IO outside their session/Hub state locks.
    pub fn append(&self, session_id: i64, mut record: Record, now: Timestamp) -> io::Result<()> {
        record.version = RECORD_VERSION;
        if record.session_id == 0 {
            record.session_id = session_id;
        }
        if record.ts.trim().is_empty() {
            record.ts = format_rfc3339(now).map_err(io::Error::other)?;
        }
        let transcript = (record.kind == KIND_TRANSCRIPT).then(|| record.transcript.clone());
        let mut line =
            crate::proto::provider::to_go_json(&sanitize(record)).map_err(io::Error::other)?;
        line.push(b'\n');
        let _guard = APPEND_LOCK
            .lock()
            .map_err(|_| io::Error::other("handoff append lock poisoned"))?;
        if let Some(path) = &transcript {
            let mut last = self
                .last_transcript
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if last.get(&session_id) == Some(path) {
                return Ok(());
            }
            last.insert(session_id, path.clone());
        }
        let path = self.path_for(session_id)?;
        private_io::open_append(&path)?.write_all(&line)
    }
    /// Drops transcript records that repeat the previous transcript path, which
    /// older versions appended on every change. Rendering reads only the latest
    /// path, so nothing a reader can see is lost. Returns the bytes reclaimed.
    pub fn compact_transcripts(&self) -> io::Result<u64> {
        let directory = match Dir::open(&self.directory()) {
            Ok(directory) => directory,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e),
        };
        let mut reclaimed = 0u64;
        for name in directory.entries()? {
            if !(name.starts_with('s') && name.ends_with(".jsonl")) {
                continue;
            }
            let _guard = APPEND_LOCK
                .lock()
                .map_err(|_| io::Error::other("handoff append lock poisoned"))?;
            let mut original = Vec::new();
            if directory
                .open_file(&name, false)
                .and_then(|mut file| file.read_to_end(&mut original))
                .is_err()
            {
                continue;
            }
            let compacted = drop_repeated_transcripts(&original);
            if compacted.len() < original.len()
                && directory.replace(&name, &compacted, 0o600).is_ok()
            {
                reclaimed += (original.len() - compacted.len()) as u64;
            }
        }
        Ok(reclaimed)
    }
    pub fn read_session(&self, id: i64) -> io::Result<Vec<Record>> {
        match read_all(&self.path_for(id)?) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(vec![]),
            other => other,
        }
    }
    /// Records of the latest launch only. Files written before session numbers
    /// survived a Hub restart can hold several launches under one number; the
    /// older ones stay on disk untouched but are not this session's history.
    pub fn read_current_session(&self, id: i64) -> io::Result<Vec<Record>> {
        let mut records = self.read_session(id)?;
        let start = current_segment_start(&records);
        records.drain(..start);
        Ok(records)
    }
    /// Whether a record file already exists for this number. The Hub never gives
    /// a new launch a number that already has a file (see
    /// `highest_session_id`), so an existing file means the same launch came back
    /// (wrapper reattach), not a new session.
    pub fn has_records(&self, id: i64) -> bool {
        Dir::open(&self.directory())
            .and_then(|directory| directory.metadata(&format!("s{id}.jsonl")))
            .is_ok_and(|metadata| metadata.is_file())
    }
    /// The highest session number any handoff file is named after; 0 when there
    /// are none. The Hub starts numbering new sessions above it, so a number is
    /// not reused while its records exist (issue #44).
    pub fn highest_session_id(&self) -> i64 {
        let Ok(directory) = Dir::open(&self.directory()) else {
            return 0;
        };
        let Ok(names) = directory.entries() else {
            return 0;
        };
        names
            .iter()
            .filter_map(|name| session_id_from_file_name(name))
            .filter(|id| *id <= MAX_SEEDED_SESSION_ID)
            .max()
            .unwrap_or(0)
    }
    pub fn write_rendered(&self, id: i64) -> io::Result<(PathBuf, String)> {
        let path = self.rendered_path_for(id)?;
        // Hold the same directory for both the source and the replacement. A
        // concurrent root rename cannot redirect either operation elsewhere.
        let directory = Dir::open_or_create_private_components(&self.directory())?;
        let records = match directory.open_file(&format!("s{id}.jsonl"), false) {
            Ok(file) => read_records(file)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => vec![],
            Err(e) => return Err(e),
        };
        let markdown = render_markdown(id, &records);
        directory.replace(&format!("s{id}_handoff.md"), markdown.as_bytes(), 0o600)?;
        Ok((path, markdown))
    }
    pub fn prune_older_than(&self, cutoff: Timestamp) -> io::Result<()> {
        let directory = match Dir::open(&self.directory()) {
            Ok(e) => e,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };
        for name in directory.entries()? {
            if !is_handoff_file_name(&name) {
                continue;
            }
            let Ok(info) = directory.metadata(&name) else {
                continue;
            };
            if info
                .modified()
                .ok()
                .and_then(|time| Timestamp::from_system_time(time).ok())
                .is_some_and(|time| time < cutoff)
            {
                let _ = directory.remove_file(&name);
            }
        }
        Ok(())
    }
    pub fn stat_dir(&self, now: Timestamp) -> io::Result<DirStatus> {
        let directory = match Dir::open(&self.directory()) {
            Ok(e) => e,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(DirStatus::default()),
            Err(e) => return Err(e),
        };
        let mut status = DirStatus::default();
        for name in directory.entries()? {
            if !name.ends_with(".jsonl") {
                continue;
            }
            let Ok(info) = directory.metadata(&name) else {
                continue;
            };
            status.files += 1;
            if let Some(time) = info
                .modified()
                .ok()
                .and_then(|time| Timestamp::from_system_time(time).ok())
            {
                status.oldest_age = status
                    .oldest_age
                    .max(now.duration_since(time).unwrap_or_default());
            }
        }
        status.exists = status.files > 0;
        Ok(status)
    }
}
#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub struct DirStatus {
    pub exists: bool,
    pub files: usize,
    pub oldest_age: Duration,
}
fn drop_repeated_transcripts(bytes: &[u8]) -> Vec<u8> {
    const MARK: &[u8] = b"\"kind\":\"transcript\"";
    let mut out = Vec::with_capacity(bytes.len());
    let mut previous: Option<String> = None;
    for line in bytes.split_inclusive(|b| *b == b'\n') {
        if line.windows(MARK.len()).any(|w| w == MARK)
            && let Ok(value) = serde_json::from_slice::<serde_json::Value>(line)
            && value["kind"] == KIND_TRANSCRIPT
            && let Some(path) = value["transcript"].as_str()
        {
            if previous.as_deref() == Some(path) {
                continue;
            }
            previous = Some(path.to_owned());
        }
        out.extend_from_slice(line);
    }
    out
}
/// `s12.jsonl`, `s12_handoff.md`, `s12.note.md` and `s12.manual.note.md` all
/// name session 12.
pub fn session_id_from_file_name(name: &str) -> Option<i64> {
    let rest = name.strip_prefix('s')?;
    let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 {
        return None;
    }
    let suffix = &rest[digits..];
    if !matches!(
        suffix,
        ".jsonl" | "_handoff.md" | ".note.md" | ".manual.note.md"
    ) {
        return None;
    }
    rest[..digits].parse::<i64>().ok().filter(|id| *id > 0)
}
/// Index of the latest `session_start`; 0 when there is none. One file used to
/// collect several launches when a session number was reused after a Hub
/// restart, so only the records from the last start describe this session.
pub fn current_segment_start(records: &[Record]) -> usize {
    records
        .iter()
        .rposition(|record| record.kind == KIND_SESSION_START)
        .unwrap_or(0)
}
/// The latest non-empty model in the records. A session's start is recorded
/// before the model is known, so the start alone usually has none.
pub fn latest_model(records: &[Record]) -> &str {
    records
        .iter()
        .rev()
        .map(|record| record.model.trim())
        .find(|model| !model.is_empty())
        .unwrap_or("")
}
/// An end that is recorded earlier than the start belongs to another launch or
/// to a broken clock; it is not shown as this session's end.
pub fn end_follows_start(start: &str, end: &str) -> bool {
    match (
        crate::proto::time::parse_rfc3339(start),
        crate::proto::time::parse_rfc3339(end),
    ) {
        (Ok(start), Ok(end)) => end >= start,
        _ => true,
    }
}
pub fn is_handoff_file_name(name: &str) -> bool {
    name.ends_with(".jsonl") || name.ends_with("_handoff.md") || name.ends_with(".note.md")
}
/// Malformed records are skipped. Oversized records fail explicitly, like the
/// reference scanner, without ever allocating the unbounded remainder.
pub fn read_all(path: &Path) -> io::Result<Vec<Record>> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("handoff has no parent"))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::other("handoff has no UTF-8 filename"))?;
    read_records(Dir::open(parent)?.open_file(name, false)?)
}
fn read_records(file: fs::File) -> io::Result<Vec<Record>> {
    let mut reader = io::BufReader::with_capacity(64 * 1024, file);
    let mut records = vec![];
    loop {
        let mut line = vec![];
        let n = reader
            .by_ref()
            .take(MAX_RECORD_BYTES)
            .read_until(b'\n', &mut line)?;
        if n == 0 {
            break;
        }
        if n as u64 == MAX_RECORD_BYTES && !line.ends_with(b"\n") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "handoff JSONL record exceeds 1MiB",
            ));
        }
        if let Ok(record) = crate::proto::decode_wire::<Record>(&line) {
            records.push(record);
        }
    }
    Ok(records)
}

/// Pure rendering; only recorded path strings are displayed, never opened.
pub fn render_markdown(mut session_id: i64, records: &[Record]) -> String {
    use std::fmt::Write;
    let records = &records[current_segment_start(records)..];
    let model = latest_model(records);
    let mut start = None;
    let mut end = None;
    let (mut dones, mut git_turns, mut intents) = (vec![], vec![], vec![]);
    let (mut work_doc, mut transcript, mut note) = ("", "", "");
    for record in records {
        match record.kind.as_str() {
            "session_start" => start = Some(record),
            "session_end" => {
                end = Some(record).filter(|end| {
                    start.is_none_or(|start: &Record| end_follows_start(&start.ts, &end.ts))
                })
            }
            "done" => dones.push(record),
            "git_turn" => git_turns.push(record),
            "intent" => intents.push(record),
            _ => {}
        }
        if !record.work_doc.trim().is_empty() {
            work_doc = &record.work_doc;
        }
        if !record.transcript.trim().is_empty() {
            transcript = &record.transcript;
        }
        if !record.note.trim().is_empty() {
            note = &record.note;
        }
    }
    if session_id == 0
        && let Some(start) = start
    {
        session_id = start.session_id;
    }
    let mut out = format!("# 引き継ぎ: セッション #{session_id}\n\n## セッションの素性\n");
    if let Some(start) = start {
        for (label, value) in [
            ("provider", start.provider.as_str()),
            ("cwd", &start.cwd),
            ("branch", &start.branch),
            ("model", model),
            ("subscription", &start.subscription_id),
            ("開始", &start.ts),
        ] {
            writeln!(out, "- {label}: {}", or_no_record(value)).unwrap();
        }
        if start.handoff_from != 0 {
            writeln!(out, "- 引き継ぎ元セッション: #{}", start.handoff_from).unwrap();
        }
        if let Some(end) = end {
            writeln!(
                out,
                "- 終了: {} ({})",
                or_no_record(&end.ts),
                or_no_record(&end.text)
            )
            .unwrap();
        } else {
            out.push_str("- 終了: (未終了 — 看板の最新記録時点)\n");
        }
    } else {
        out.push_str("記録なし（session_start が看板に無い）\n");
    }
    out.push('\n');
    if !note.is_empty() {
        writeln!(out, "## 引き継ぎメモ\n- {note}").unwrap();
        out.push_str("前任が止まる前に書いた引き継ぎメモ。**本書より先にこれを読む**（次の一手・未検証の前提・開いている論点が、この md より詳しく書かれている）。\n\n");
    }
    if !transcript.is_empty() {
        writeln!(out, "## 前任の会話ログ\n- {transcript}").unwrap();
        out.push_str("このファイルは前任の会話ログ（JSONL）。まず末尾から、最後のユーザーの指示と最後の assistant の発言（作業の要約・次のステップがあればそれ）を読み、次に本書の「次の一手」と突き合わせてから作業に入る。ファイルが大きいときは末尾 200 行と `grep` で足りる。\n\n");
    }
    out.push_str("## 作業中の md\n");
    if !work_doc.is_empty() {
        writeln!(out, "- {work_doc}").unwrap();
    } else {
        out.push_str("記録なし（この経路からはまだ分からない。開いていた plan/bugfix があれば手で伝えること）\n");
    }
    out.push_str("\n## 直近の完了\n");
    recent(&mut out, &dones, RENDER_MAX_DONE_ENTRIES, |r| {
        format!(
            "{}: {}",
            or_no_record(&r.ts),
            if r.text.trim().is_empty() {
                "(本文なし)"
            } else {
                &r.text
            }
        )
    });
    out.push_str("\n## 直近の変更\n");
    recent(
        &mut out,
        &git_turns,
        RENDER_MAX_GIT_TURN_ENTRIES,
        git_turn_line,
    );
    out.push_str("\n## 次の一手 / 未検証の前提\n");
    if let Some(intent) = intents.last() {
        writeln!(out, "- {}", intent.text).unwrap();
    } else {
        out.push_str("記録なし。何が変わったかは分かるが、なぜそうしたかは分からない（最後の完了報告から止まるまでの1ターン分は構造的に欠けている）。\n");
    }
    out.push_str("\n## 後継への指示\n1. まずこの md を読み、上の「作業中の md」に記録があればそのファイルも読む。\n2. 「直近の変更」に挙げたコミット以降から作業を再開する（この md にはコードの中身・diff・PTY 出力・環境変数は一切含まれていない）。\n3. 「次の一手 / 未検証の前提」に書かれていないことは分からない前提で動く。憶測で断定しない。\n");
    if let Some(start) = start
        && start.handoff_from != 0
    {
        writeln!(out, "4. このセッション自身も別セッション（#{}）からの引き継ぎである。さらに遡って読みたい場合は看板ディレクトリの s{}.jsonl を確認する。", start.handoff_from, start.handoff_from).unwrap();
    }
    out
}
fn or_no_record(value: &str) -> &str {
    if value.trim().is_empty() {
        "(記録なし)"
    } else {
        value
    }
}
fn recent(out: &mut String, records: &[&Record], max: usize, format: impl Fn(&Record) -> String) {
    use std::fmt::Write;
    if records.is_empty() {
        out.push_str("記録なし\n");
        return;
    }
    for record in records.iter().rev().take(max) {
        writeln!(out, "- {}", format(record)).unwrap();
    }
    if records.len() > max {
        writeln!(out, "- …他 {} 件（古いものから省略）", records.len() - max).unwrap();
    }
}
fn git_turn_line(record: &Record) -> String {
    let mut parts = vec![];
    if !record.commit.is_empty() {
        let end = record.commit.len().min(12);
        let subject = if record.commit_subject.is_empty() {
            "(件名なし)"
        } else {
            &record.commit_subject
        };
        parts.push(format!(
            "commit {} {}",
            crate::proto::wire::go_utf8_lossy(&record.commit.as_bytes()[..end]),
            go_quote(subject)
        ));
    }
    if record.turn > 0 {
        parts.push(format!("turn {}", record.turn));
    }
    if !record.files.is_empty() {
        parts.push(format!("files: {}", record.files.join(", ")));
    }
    if parts.is_empty() {
        or_no_record(&record.ts).into()
    } else {
        parts.join(" / ")
    }
}
fn go_quote(text: &str) -> String {
    crate::proto::go_quote::quote(text)
}
#[cfg(test)]
mod transcript_dedupe_tests {
    use super::*;

    fn transcript(path: &str) -> Record {
        Record {
            kind: KIND_TRANSCRIPT.into(),
            transcript: path.into(),
            ..Record::default()
        }
    }
    fn store() -> (tempfile::TempDir, tempfile::TempDir, HandoffStore) {
        let root = tempfile::tempdir().unwrap();
        let installed = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::trial(root.path(), 49120, installed.path()).unwrap();
        (root, installed, HandoffStore::new(paths))
    }

    #[test]
    fn an_unchanged_transcript_path_is_not_appended_again() {
        let (_root, _installed, store) = store();
        let now = Timestamp::now();
        for path in ["a.jsonl", "a.jsonl", "b.jsonl", "b.jsonl", "a.jsonl"] {
            store.append(1, transcript(path), now).unwrap();
        }
        let kept: Vec<_> = store
            .read_session(1)
            .unwrap()
            .into_iter()
            .map(|r| r.transcript)
            .collect();
        assert_eq!(kept, ["a.jsonl", "b.jsonl", "a.jsonl"]);
    }

    #[test]
    fn compaction_keeps_the_latest_path_and_every_other_record() {
        let (_root, _installed, store) = store();
        let path = store.path_for(2).unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut lines = String::new();
        for body in [
            r#"{"version":1,"kind":"session_start","session_id":2}"#,
            r#"{"version":1,"kind":"transcript","transcript":"a.jsonl"}"#,
            r#"{"version":1,"kind":"transcript","transcript":"a.jsonl"}"#,
            r#"{"version":1,"kind":"done","text":"finished"}"#,
            r#"{"version":1,"kind":"transcript","transcript":"a.jsonl"}"#,
            r#"{"version":1,"kind":"transcript","transcript":"b.jsonl"}"#,
            r#"{"version":1,"kind":"transcript","transcript":"b.jsonl"}"#,
        ] {
            lines.push_str(body);
            lines.push('\n');
        }
        std::fs::write(&path, &lines).unwrap();
        assert!(store.compact_transcripts().unwrap() > 0);
        let records = store.read_session(2).unwrap();
        let kinds: Vec<_> = records.iter().map(|r| r.kind.as_str()).collect();
        assert_eq!(kinds, ["session_start", "transcript", "done", "transcript"]);
        assert_eq!(records.last().unwrap().transcript, "b.jsonl");
        assert_eq!(store.compact_transcripts().unwrap(), 0);
    }
}
#[cfg(test)]
mod quote_consumer_tests {
    use super::*;

    #[test]
    fn git_subject_rendering_uses_pinned_unicode_assignments() {
        let record = Record {
            commit: "0123456789abcdef".into(),
            commit_subject: "\u{2ebf0}".into(),
            ..Record::default()
        };
        assert_eq!(
            git_turn_line(&record),
            "commit 0123456789ab \"\\U0002ebf0\""
        );
    }
}
#[cfg(test)]
mod session_reuse_tests {
    //! Issue #44: one `s{id}.jsonl` used to collect several launches when the
    //! Hub reused a session number after a restart.
    use super::*;

    fn store() -> (tempfile::TempDir, tempfile::TempDir, HandoffStore) {
        let root = tempfile::tempdir().unwrap();
        let installed = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::trial(root.path(), 49121, installed.path()).unwrap();
        (root, installed, HandoffStore::new(paths))
    }
    /// A file as older versions left it: two launches under number 6, the
    /// first ended later than the second started, the model only on the ends.
    const MIXED: &[&str] = &[
        r#"{"version":1,"ts":"2026-10-01T09:00:00Z","session_id":6,"kind":"session_start","provider":"codex","cwd":"/old/project"}"#,
        r#"{"version":1,"ts":"2026-10-01T10:00:00Z","session_id":6,"kind":"done","text":"OLD-DONE-SENTINEL"}"#,
        r#"{"version":1,"ts":"2026-10-01T10:05:00Z","session_id":6,"kind":"git_turn","commit":"aaaaaaaaaaaa","commit_subject":"OLD-COMMIT-SENTINEL"}"#,
        r#"{"version":1,"ts":"2026-10-01T10:06:00Z","session_id":6,"kind":"intent","text":"OLD-INTENT-SENTINEL"}"#,
        r#"{"version":1,"ts":"2026-10-01T10:07:00Z","session_id":6,"kind":"transcript","transcript":"/old/transcript.jsonl"}"#,
        r#"{"version":1,"ts":"2026-10-30T12:00:00Z","session_id":6,"kind":"session_end","model":"old-model","text":"exit"}"#,
        "not json at all",
        r#"{"version":1,"ts":"2026-11-02T08:00:00Z","session_id":6,"kind":"session_start","provider":"claude","cwd":"/new/project"}"#,
        r#"{"version":1,"ts":"2026-11-02T08:30:00Z","session_id":6,"kind":"done","model":"new-model","text":"NEW-DONE"}"#,
    ];
    fn write_mixed(store: &HandoffStore) {
        let path = store.path_for(6).unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, MIXED.join("\n") + "\n").unwrap();
    }

    #[test]
    fn a_reused_number_renders_only_the_latest_launch() {
        let (_root, _installed, store) = store();
        write_mixed(&store);
        let before = std::fs::read(store.path_for(6).unwrap()).unwrap();
        let (_, markdown) = store.write_rendered(6).unwrap();
        for old in [
            "OLD-DONE-SENTINEL",
            "OLD-COMMIT-SENTINEL",
            "OLD-INTENT-SENTINEL",
            "/old/transcript.jsonl",
            "/old/project",
            "old-model",
            "2026-10-30T12:00:00Z",
        ] {
            assert!(!markdown.contains(old), "{old} leaked into:\n{markdown}");
        }
        assert!(markdown.contains("- provider: claude"));
        assert!(markdown.contains("- cwd: /new/project"));
        assert!(markdown.contains("- model: new-model"), "{markdown}");
        assert!(markdown.contains("NEW-DONE"));
        assert!(markdown.contains("- 終了: (未終了"), "{markdown}");
        // Reading never rewrites or drops the older launches.
        assert_eq!(std::fs::read(store.path_for(6).unwrap()).unwrap(), before);
        assert_eq!(store.read_session(6).unwrap().len(), MIXED.len() - 1);
        let current = store.read_current_session(6).unwrap();
        assert_eq!(current.len(), 2);
        assert_eq!(current[0].provider, "claude");
    }

    #[test]
    fn an_end_recorded_before_the_start_is_not_shown() {
        let records = [
            Record {
                kind: KIND_SESSION_START.into(),
                ts: "2026-11-02T08:00:00Z".into(),
                ..Record::default()
            },
            Record {
                kind: KIND_SESSION_END.into(),
                ts: "2026-11-01T08:00:00Z".into(),
                ..Record::default()
            },
        ];
        let markdown = render_markdown(3, &records);
        assert!(!markdown.contains("2026-11-01T08:00:00Z"), "{markdown}");
        assert!(markdown.contains("- 終了: (未終了"));
        let ended = [
            records[0].clone(),
            Record {
                ts: "2026-11-02T09:00:00Z".into(),
                ..records[1].clone()
            },
        ];
        assert!(render_markdown(3, &ended).contains("- 終了: 2026-11-02T09:00:00Z"));
    }

    #[test]
    fn records_without_a_start_still_render() {
        let records = [Record {
            kind: KIND_DONE.into(),
            text: "only-done".into(),
            model: "m1".into(),
            ..Record::default()
        }];
        let markdown = render_markdown(4, &records);
        assert!(markdown.contains("only-done"));
        assert!(markdown.contains("記録なし（session_start が看板に無い）"));
    }

    #[test]
    fn the_highest_recorded_number_seeds_new_sessions() {
        let (_root, _installed, store) = store();
        assert_eq!(store.highest_session_id(), 0);
        assert!(!store.has_records(6));
        write_mixed(&store);
        let dir = store.directory();
        for name in [
            "s9_handoff.md",
            "s12.manual.note.md",
            "s3.note.md",
            "s99999999999.jsonl",
            "sx.jsonl",
            "s.jsonl",
            "s40.txt",
            "notes.jsonl",
        ] {
            std::fs::write(dir.join(name), "").unwrap();
        }
        assert!(store.has_records(6));
        assert!(!store.has_records(12));
        assert_eq!(store.highest_session_id(), 12);
        assert_eq!(session_id_from_file_name("s7.jsonl"), Some(7));
        assert_eq!(session_id_from_file_name("s0.jsonl"), None);
        assert_eq!(session_id_from_file_name("s7.jsonl.bak"), None);
    }
}
