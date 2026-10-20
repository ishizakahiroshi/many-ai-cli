//! 一時観測 `rust-spawn-probe`（instrumentation.json に登録）。原因が確定したら撤去する。
//!
//! 何のためか: AI CLI のセッションを起動してから、端末に最初の文字が出るまでが遅い（2026-10-20）。
//! どの段階に時間がかかっているかが分からないので、Hub と wrapper の両方で段階ごとの時刻を 1 行ずつ残す。
//!
//! 守っていること:
//! - debug ビルドでだけ組み込まれる（logging/mod.rs の cfg）。`--release` の成果物には入らない
//! - 残すのは段階名・壊れない時刻（epoch ミリ秒）・自分の pid・対象の wrapper pid・セッション番号だけ。
//!   パス・引数・環境変数・端末の出力本文は残さない
//! - `MANY_AI_CLI_SPAWN_PROBE=0` で止まる。テストでは書かない
//! - 置き場所は `<logs>/spawn-probe/<日付>_<pid>.jsonl`。各プロセスが最初に書く前に、7 日より古いファイルと、
//!   合計 20MB を超えた古い分を消す。1 ファイルは 4MB で書き止める。書き込みの失敗は捨てて呼び出し元を止めない
//!
//! 突き合わせ方: Hub は wrapper を起こした直後に `subject`（wrapper の pid）つきで残し、wrapper は自分の
//! pid を `subject` にして残す。同じ `subject` の行を `t_ms` で並べると 1 回の起動の段階別の所要時間になる。
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
    let Some(Some(directory)) = DIRECTORY.get() else {
        return;
    };
    let now = SystemTime::now();
    let mut line = Map::new();
    line.insert(
        "t_ms".into(),
        json!(
            now.duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as u64)
        ),
    );
    line.insert("pid".into(), json!(std::process::id()));
    line.insert("stage".into(), json!(stage));
    if let Some(subject) = subject {
        line.insert("subject".into(), json!(subject));
    }
    if let Some(session) = session {
        line.insert("session".into(), json!(session));
    }
    let Ok(mut bytes) = serde_json::to_vec(&Value::Object(line)) else {
        return;
    };
    bytes.push(b'\n');
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

    #[test]
    fn only_zero_disables_the_probe() {
        assert!(enabled(None));
        assert!(enabled(Some(OsStr::new("1"))));
        assert!(!enabled(Some(OsStr::new("0"))));
    }
}
