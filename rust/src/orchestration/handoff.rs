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
    fs,
    io::{self, BufRead, Read, Write},
    path::{Path, PathBuf},
    sync::Mutex,
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
}
impl HandoffStore {
    pub fn new(paths: RuntimePaths) -> Self {
        Self { paths }
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
        let mut line =
            crate::proto::provider::to_go_json(&sanitize(record)).map_err(io::Error::other)?;
        line.push(b'\n');
        let _guard = APPEND_LOCK
            .lock()
            .map_err(|_| io::Error::other("handoff append lock poisoned"))?;
        let path = self.path_for(session_id)?;
        private_io::open_append(&path)?.write_all(&line)
    }
    pub fn read_session(&self, id: i64) -> io::Result<Vec<Record>> {
        match read_all(&self.path_for(id)?) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(vec![]),
            other => other,
        }
    }
    pub fn write_rendered(&self, id: i64) -> io::Result<(PathBuf, String)> {
        let path = self.rendered_path_for(id)?;
        // Hold the same directory for both the source and the replacement. A
        // concurrent root rename cannot redirect either operation elsewhere.
        let directory = Dir::open_or_create_private(&self.directory())?;
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
    let mut start = None;
    let mut end = None;
    let (mut dones, mut git_turns, mut intents) = (vec![], vec![], vec![]);
    let (mut work_doc, mut transcript, mut note) = ("", "", "");
    for record in records {
        match record.kind.as_str() {
            "session_start" => start = Some(record),
            "session_end" => end = Some(record),
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
            ("provider", &start.provider),
            ("cwd", &start.cwd),
            ("branch", &start.branch),
            ("model", &start.model),
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
    use std::fmt::Write;
    static NONPRINT: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let nonprint = NONPRINT.get_or_init(|| {
        regex::Regex::new(r"[^\p{L}\p{M}\p{N}\p{P}\p{S} ]").expect("Go printable class")
    });
    let mut quoted = String::from("\"");
    for c in text.chars() {
        match c {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            '\u{7}' => quoted.push_str("\\a"),
            '\u{8}' => quoted.push_str("\\b"),
            '\u{b}' => quoted.push_str("\\v"),
            '\u{c}' => quoted.push_str("\\f"),
            c if c < ' ' || c == '\u{7f}' => {
                write!(quoted, "\\x{:02x}", c as u32).unwrap();
            }
            c if nonprint.is_match(c.encode_utf8(&mut [0u8; 4])) => {
                if c as u32 <= 0xffff {
                    write!(quoted, "\\u{:04x}", c as u32).unwrap();
                } else {
                    write!(quoted, "\\U{:08x}", c as u32).unwrap();
                }
            }
            c => quoted.push(c),
        }
    }
    quoted.push('"');
    quoted
}
