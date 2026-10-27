//! 一時観測 `rust-spawn-probe`（instrumentation.json に登録）。原因が確定したら撤去する。
//!
//! 何のためか: AI CLI のセッションを起動してから、端末に最初の文字が出るまでが遅い（2026-10-20）。
//! どの段階に時間がかかっているかが分からないので、Hub と wrapper の両方で段階ごとの時刻を 1 行ずつ残す。
//!
//! 守っていること:
//! - debug ビルドでだけ組み込まれる（logging/mod.rs の cfg）。`--release` の成果物には入らない
//! - 残すのは段階名・壊れない時刻（epoch ミリ秒）・自分の pid・対象の wrapper pid・セッション番号と、
//!   spawn の最初の行（Hub 側）の provider 名だけ。パス・引数・環境変数・端末の出力本文は残さない
//! - `MANY_AI_CLI_SPAWN_PROBE=0` で止まる。テストでは書かない
//! - 置き場所は `<logs>/spawn-probe/<日付>_<pid>.jsonl`。各プロセスが最初に書く前に、7 日より古いファイルと、
//!   合計 20MB を超えた古い分を消す。1 ファイルは 4MB で書き止める。書き込みの失敗は捨てて呼び出し元を止めない
//!
//! 突き合わせ方: Hub は wrapper を起こした直後に `subject`（wrapper の pid）つきで残し、wrapper は自分の
//! pid を `subject` にして残す。同じ `subject` の行を `t_ms` で並べると 1 回の起動の段階別の所要時間になる。
//!
//! 登録受付までの内訳（wrapper が登録を送ってから Hub が受け付けるまで）: wrapper は接続完了と登録送信の
//! 直後（`wrapper_connected` / `wrapper_register_sent`）、Hub は登録メッセージの `pid` を `subject` にして
//! コアの登録・ACK 前の準備・登録準備の前後（`hub_core_register_begin` / `hub_core_registered` /
//! `hub_before_ack_begin` / `hub_before_ack_done` / `hub_prepare_begin` / `hub_prepared`）と、準備の中の
//! 指示ルールと使用量フックの前後（`hub_rules_registered_*` / `hub_usage_registered_*`。`subject` なし）を残す。
use serde_json::{Map, Value, json};
use std::{
    ffi::OsStr,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const DIRECTORY_NAME: &str = "spawn-probe";
const DISABLE_ENV: &str = "MANY_AI_CLI_SPAWN_PROBE";
const RETENTION: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const TOTAL_LIMIT: u64 = 20 * 1024 * 1024;
const FILE_LIMIT: u64 = 4 * 1024 * 1024;

static DIRECTORY: OnceLock<Option<PathBuf>> = OnceLock::new();
static SINK: Mutex<Sink> = Mutex::new(Sink::Unopened);

enum Sink {
    Unopened,
    Open { file: File, written: u64 },
    Failed,
}

/// プロセスの起動時に 1 回呼ぶ。2 回目以降は何もしない。ファイルは最初の記録まで作らない。
pub(crate) fn init(log_dir: &Path) {
    if cfg!(test) {
        return;
    }
    let enabled = enabled(std::env::var_os(DISABLE_ENV).as_deref());
    let _ = DIRECTORY.set(enabled.then(|| log_dir.join(DIRECTORY_NAME)));
}

fn enabled(value: Option<&OsStr>) -> bool {
    value.is_none_or(|value| value != "0")
}

/// 段階を 1 行残す。`subject` は対象の wrapper の pid（wrapper 自身は自分の pid）、
/// `session` はセッション番号（決まる前は None）。
pub(crate) fn mark(stage: &'static str, subject: Option<u32>, session: Option<i64>) {
    record(stage, subject, session, None);
}

/// `mark` に provider 名を足した 1 行。spawn の最初の行（Hub 側）だけが使う。
/// provider 名は英数字と `-` `_` `.` の短い名前だけを残し、それ以外は `other` に置き換える。
pub(crate) fn mark_provider(
    stage: &'static str,
    subject: Option<u32>,
    session: Option<i64>,
    provider: &str,
) {
    record(stage, subject, session, Some(provider));
}

fn record(stage: &str, subject: Option<u32>, session: Option<i64>, provider: Option<&str>) {
    let Some(Some(directory)) = DIRECTORY.get() else {
        return;
    };
    let now = SystemTime::now();
    let Some(bytes) = render(now, std::process::id(), stage, subject, session, provider) else {
        return;
    };
    let Ok(mut sink) = SINK.lock() else {
        return;
    };
    if matches!(*sink, Sink::Unopened) {
        *sink = open(directory, now).unwrap_or(Sink::Failed);
    }
    if let Sink::Open { file, written } = &mut *sink
        && *written + bytes.len() as u64 <= FILE_LIMIT
        && file.write_all(&bytes).is_ok()
    {
        *written += bytes.len() as u64;
    }
}

/// 1 行ぶんのバイト列（末尾の改行つき）を作る。残す項目はここに並ぶものだけ。
fn render(
    now: SystemTime,
    pid: u32,
    stage: &str,
    subject: Option<u32>,
    session: Option<i64>,
    provider: Option<&str>,
) -> Option<Vec<u8>> {
    let mut line = Map::new();
    line.insert(
        "t_ms".into(),
        json!(
            now.duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as u64)
        ),
    );
    line.insert("pid".into(), json!(pid));
    line.insert("stage".into(), json!(stage));
    if let Some(subject) = subject {
        line.insert("subject".into(), json!(subject));
    }
    if let Some(session) = session {
        line.insert("session".into(), json!(session));
    }
    if let Some(provider) = provider {
        line.insert("provider".into(), json!(provider_label(provider)));
    }
    let mut bytes = serde_json::to_vec(&Value::Object(line)).ok()?;
    bytes.push(b'\n');
    Some(bytes)
}

/// provider 名として残してよい形（英数字と `-` `_` `.`、64 文字まで）だけを通す。
/// パスや引数が紛れ込んでも、そのまま残さない。
fn provider_label(provider: &str) -> &str {
    let valid = !provider.is_empty()
        && provider.len() <= 64
        && provider
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    if valid { provider } else { "other" }
}

fn open(directory: &Path, now: SystemTime) -> Option<Sink> {
    fs::create_dir_all(directory).ok()?;
    purge(directory, now);
    let day = crate::proto::time::Timestamp::from_system_time(now)
        .and_then(crate::proto::time::format_rfc3339)
        .unwrap_or_default();
    let day: String = day
        .chars()
        .take(10)
        .filter(|c| c.is_ascii_digit())
        .collect();
    let name = format!(
        "{}_{}.jsonl",
        if day.is_empty() { "unknown" } else { &day },
        std::process::id()
    );
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(directory.join(name)).ok()?;
    let written = file.metadata().map_or(0, |m| m.len());
    Some(Sink::Open { file, written })
}

/// 置き場所の中の `.jsonl` だけを見る。シンボリックリンクと他の拡張子には触らない。
fn purge(directory: &Path, now: SystemTime) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut kept = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "jsonl") {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let modified = meta.modified().unwrap_or(now);
        if now
            .duration_since(modified)
            .is_ok_and(|age| age > RETENTION)
        {
            let _ = fs::remove_file(&path);
            continue;
        }
        kept.push((modified, meta.len(), path));
    }
    kept.sort_by_key(|(modified, ..)| *modified);
    let mut total: u64 = kept.iter().map(|(_, len, _)| len).sum();
    for (_, len, path) in kept {
        if total <= TOTAL_LIMIT {
            break;
        }
        if fs::remove_file(&path).is_ok() {
            total -= len;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 登録受付までの内訳として足した段階名と、その呼び出し点があるファイルの中身。
    const REGISTRATION_STAGES: &[(&str, &str)] = &[
        ("wrapper_connected", include_str!("../wrapper/transport.rs")),
        (
            "wrapper_register_sent",
            include_str!("../wrapper/transport.rs"),
        ),
        (
            "hub_core_register_begin",
            include_str!("../hub/websocket.rs"),
        ),
        ("hub_core_registered", include_str!("../hub/websocket.rs")),
        ("hub_before_ack_begin", include_str!("../hub/websocket.rs")),
        ("hub_before_ack_done", include_str!("../hub/websocket.rs")),
        ("hub_prepare_begin", include_str!("../hub/websocket.rs")),
        ("hub_prepared", include_str!("../hub/websocket.rs")),
        (
            "hub_rules_registered_begin",
            include_str!("../application/main_program/serve.rs"),
        ),
        (
            "hub_rules_registered",
            include_str!("../application/main_program/serve.rs"),
        ),
        (
            "hub_usage_registered_begin",
            include_str!("../application/main_program/serve.rs"),
        ),
        (
            "hub_usage_registered",
            include_str!("../application/main_program/serve.rs"),
        ),
    ];

    fn parse(bytes: &[u8]) -> Map<String, Value> {
        assert_eq!(bytes.last(), Some(&b'\n'));
        match serde_json::from_slice(&bytes[..bytes.len() - 1]).expect("one JSON line") {
            Value::Object(map) => map,
            other => panic!("probe line is not an object: {other}"),
        }
    }

    #[test]
    fn only_zero_disables_the_probe() {
        assert!(enabled(None));
        assert!(enabled(Some(OsStr::new("1"))));
        assert!(!enabled(Some(OsStr::new("0"))));
    }

    #[test]
    fn every_registration_stage_has_a_call_site() {
        for (stage, source) in REGISTRATION_STAGES {
            assert!(
                source.contains(&format!("\"{stage}\"")),
                "no spawn_probe call site for {stage}"
            );
        }
        let spawn = include_str!("../application/wrapped_spawn.rs");
        assert!(spawn.contains("spawn_probe::mark_provider("));
        assert!(spawn.contains("\"hub_native_spawn_begin\""));
    }

    #[test]
    fn registration_stages_are_written_with_only_the_allowed_fields() {
        let directory = tempfile::tempdir().expect("temporary probe directory");
        let now = SystemTime::now();
        let Some(Sink::Open { mut file, .. }) = open(directory.path(), now) else {
            panic!("probe file did not open");
        };
        for (index, (stage, _)) in REGISTRATION_STAGES.iter().enumerate() {
            let bytes = render(now, 10, stage, Some(4242), Some(index as i64 + 1), None)
                .expect("rendered probe line");
            file.write_all(&bytes).expect("probe line written");
        }
        drop(file);
        let files: Vec<_> = fs::read_dir(directory.path())
            .expect("probe directory")
            .flatten()
            .collect();
        assert_eq!(files.len(), 1);
        let text = fs::read_to_string(files[0].path()).expect("probe file");
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines.len(), REGISTRATION_STAGES.len());
        for (line, (stage, _)) in lines.iter().zip(REGISTRATION_STAGES) {
            let map = parse(format!("{line}\n").as_bytes());
            assert_eq!(map["stage"], *stage);
            assert_eq!(map["subject"], 4242);
            assert!(map["t_ms"].as_u64().is_some_and(|t| t > 0));
            let mut keys: Vec<_> = map.keys().map(String::as_str).collect();
            keys.sort_unstable();
            assert_eq!(keys, ["pid", "session", "stage", "subject", "t_ms"]);
        }
    }

    #[test]
    fn provider_is_recorded_only_as_a_plain_name() {
        let now = SystemTime::now();
        let line = |provider| {
            parse(
                &render(now, 1, "hub_native_spawn_begin", None, None, Some(provider))
                    .expect("rendered probe line"),
            )
        };
        assert_eq!(line("claude")["provider"], "claude");
        assert_eq!(line("cursor-agent")["provider"], "cursor-agent");
        assert_eq!(line("my_tool.v2")["provider"], "my_tool.v2");
        assert_eq!(line("/opt/synthetic/tool")["provider"], "other");
        assert_eq!(line(r"synthetic\tool.exe")["provider"], "other");
        assert_eq!(line("claude --model x")["provider"], "other");
        assert_eq!(line("")["provider"], "other");
        assert_eq!(line(&"a".repeat(65))["provider"], "other");
        let without = parse(
            &render(now, 1, "hub_native_spawn_begin", None, None, None)
                .expect("rendered probe line"),
        );
        assert!(!without.contains_key("provider"));
    }
}
