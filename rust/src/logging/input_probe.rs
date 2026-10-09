//! 一時観測 `rust-input-probe`（instrumentation.json に登録）。原因が確定したら撤去する。
//!
//! 何のためか: Rust 版で Codex へ画像だけを送ると、`@パス ` の後に送った Enter で送信されず、
//! 後から送信されたターンもすぐ中断される（2026-10-09）。Codex 本体のログ（logs_2.sqlite）には
//! 入力欄が貼り付けを受け取った時刻・送信・中断（op: Interrupt）の時刻が残るが、
//! こちらが PTY へいつ何を書いたかの記録が無く、Enter が改行として吸われたのか、
//! 中断のキーを誰が送ったのかを確かめられない。そこで Hub が送った入力と、
//! ラッパーが PTY へ実際に書いた時刻を残す。
//!
//! 守っていること:
//! - debug ビルドでだけ組み込まれる（logging/mod.rs の cfg）。`--release` の成果物には入らない
//! - 本文とパスは残さない。印字文字は `text(文字数)` に、`@絶対パス` は `atpath(文字数)` に
//!   畳み、Enter・Esc・Ctrl+C・ペーストの区切りなどの制御文字だけを名前で残す
//!   （v0.5.x で打鍵を無マスクのまま保存し、監査で撤去した前例があるため）
//! - `MANY_AI_CLI_INPUT_PROBE=0` で止まる。テストでは書かない
//! - 置き場所は `<logs>/input-probe/<日付>_<pid>.jsonl`。各プロセスが最初に書く前に、
//!   7 日より古いファイルと、合計 20MB を超えた古い分を消す。1 ファイルは 4MB で書き止める。
//!   フォルダごと消しても動作に影響しない
use crate::proto::core::InputAuthority;
use serde_json::{Map, Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const DIRECTORY_NAME: &str = "input-probe";
const DISABLE_ENV: &str = "MANY_AI_CLI_INPUT_PROBE";
const RETENTION: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const TOTAL_LIMIT: u64 = 20 * 1024 * 1024;
const FILE_LIMIT: u64 = 4 * 1024 * 1024;
const MAX_TOKENS: usize = 64;

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
    let enabled = std::env::var_os(DISABLE_ENV).is_none_or(|value| value != "0");
    let _ = DIRECTORY.set(enabled.then(|| log_dir.join(DIRECTORY_NAME)));
}

/// Hub がラッパーへ `pty_input` を送る直前。送り元の種類で、画面からの入力か Hub 内部かを分ける。
pub(crate) fn hub_input(session: i64, seq: i64, authority: &InputAuthority, bytes: &[u8]) {
    let source = match authority {
        InputAuthority::Ui(_) => "ui",
        InputAuthority::Internal => "internal",
        InputAuthority::InitialPrompt => "initial_prompt",
        InputAuthority::NativeApproval(_) => "native_approval",
    };
    record(json!({
        "role": "hub",
        "event": "send",
        "session": session,
        "seq": seq,
        "source": source,
        "bytes": bytes.len(),
        "shape": shape(bytes),
    }));
}

/// ラッパーが Hub から入力を受け取った時点。直後の `write` がこの入力を書いた記録になる。
pub(crate) fn wrapper_receive(session: i64, seq: i64, attach: bool, bytes: &[u8]) {
    record(json!({
        "role": "wrapper",
        "event": "receive",
        "session": session,
        "seq": seq,
        "attach": attach,
        "bytes": bytes.len(),
        "shape": shape(bytes),
    }));
}

/// ラッパーが PTY へ 1 回書き終えた時点。Enter を遅らせて別に書く場合は 2 行に分かれる。
pub(crate) fn wrapper_write(provider: &str, bytes: &[u8]) {
    record(json!({
        "role": "wrapper",
        "event": "write",
        "provider": provider,
        "bytes": bytes.len(),
        "shape": shape(bytes),
    }));
}

fn record(value: Value) {
    let Some(Some(directory)) = DIRECTORY.get() else {
        return;
    };
    let Value::Object(mut fields) = value else {
        return;
    };
    let now = SystemTime::now();
    let ts = crate::proto::time::Timestamp::from_system_time(now)
        .and_then(crate::proto::time::format_rfc3339)
        .unwrap_or_default();
    let mut line = Map::new();
    line.insert(
        "t_ms".into(),
        json!(
            now.duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as u64)
        ),
    );
    line.insert("ts".into(), json!(ts));
    line.insert("pid".into(), json!(std::process::id()));
    line.append(&mut fields);
    let Ok(mut bytes) = serde_json::to_vec(&Value::Object(line)) else {
        return;
    };
    bytes.push(b'\n');
    // 観測が本来の入力処理を止めてはいけないので、ロックの失敗も書き込みの失敗も捨てる。
    let Ok(mut sink) = SINK.lock() else {
        return;
    };
    if matches!(*sink, Sink::Unopened) {
        *sink = open(directory, &ts).unwrap_or(Sink::Failed);
    }
    if let Sink::Open { file, written } = &mut *sink
        && *written + bytes.len() as u64 <= FILE_LIMIT
        && file.write_all(&bytes).is_ok()
    {
        *written += bytes.len() as u64;
    }
}

fn open(directory: &Path, ts: &str) -> Option<Sink> {
    fs::create_dir_all(directory).ok()?;
    purge(directory, SystemTime::now());
    let day: String = ts.chars().take(10).filter(|c| c.is_ascii_digit()).collect();
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

/// 入力を、本文を含まない記号の並びにする。
fn shape(bytes: &[u8]) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut omitted = 0usize;
    let mut push = |token: String| {
        if tokens.len() < MAX_TOKENS {
            tokens.push(token);
        } else {
            omitted += 1;
        }
    };
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b >= 0x20 && b != 0x7f {
            let start = i;
            while i < bytes.len() && bytes[i] >= 0x20 && bytes[i] != 0x7f {
                i += 1;
            }
            let run = &bytes[start..i];
            let chars = run.iter().filter(|b| **b & 0xc0 != 0x80).count();
            let kind = if run.first() == Some(&b'@') && absolute_path_shape(&run[1..]) {
                "atpath"
            } else {
                "text"
            };
            push(format!("{kind}({chars})"));
            continue;
        }
        if b == 0x1b {
            let rest = &bytes[i + 1..];
            if rest.starts_with(b"[200~") {
                push("PASTE_START".into());
                i += 6;
                continue;
            }
            if rest.starts_with(b"[201~") {
                push("PASTE_END".into());
                i += 6;
                continue;
            }
            if rest.first() == Some(&b'[') {
                // CSI: 引数（0x20〜0x3F）と終端（0x40〜0x7E）。矢印・マウス・フォーカス報告など。
                let mut j = 1;
                while j < rest.len() && (0x20..=0x3f).contains(&rest[j]) {
                    j += 1;
                }
                if j < rest.len() && (0x40..=0x7e).contains(&rest[j]) {
                    let body = String::from_utf8_lossy(&rest[1..=j]);
                    push(format!("CSI({body})"));
                    i += 1 + j + 1;
                    continue;
                }
                push("ESC[unterminated".into());
                i = bytes.len();
                continue;
            }
            if rest.first() == Some(&b'O') && rest.len() >= 2 && (0x40..=0x7e).contains(&rest[1]) {
                push(format!("SS3({})", rest[1] as char));
                i += 3;
                continue;
            }
            push("ESC".into());
            i += 1;
            continue;
        }
        push(match b {
            b'\r' => "CR".into(),
            b'\n' => "LF".into(),
            b'\t' => "TAB".into(),
            0x08 => "BS".into(),
            0x7f => "DEL".into(),
            _ => format!("^{}", (b + 0x40) as char),
        });
        i += 1;
    }
    if omitted > 0 {
        tokens.push(format!("+{omitted}"));
    }
    tokens
}

fn absolute_path_shape(bytes: &[u8]) -> bool {
    bytes.first() == Some(&b'/')
        || (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'/' | b'\\'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_keeps_control_keys_and_drops_text_and_paths() {
        let secret = "@F:/build/secret-name/image.png ";
        let mut input = secret.as_bytes().to_vec();
        input.extend_from_slice(b"\r");
        let tokens = shape(&input);
        assert_eq!(tokens, ["atpath(32)", "CR"]);
        assert!(!tokens.concat().contains("secret"));

        let tokens = shape("\x1b[200~日本語の本文\x1b[201~\r\x1b\x03\x15".as_bytes());
        assert_eq!(
            tokens,
            [
                "PASTE_START",
                "text(6)",
                "PASTE_END",
                "CR",
                "ESC",
                "^C",
                "^U"
            ]
        );
    }

    #[test]
    fn shape_names_escape_sequences_without_their_neighbours() {
        assert_eq!(
            shape(b"\x1b[<64;10;5M\x1b[A\x1bOP\x1b[I"),
            ["CSI(<64;10;5M)", "CSI(A)", "SS3(P)", "CSI(I)"]
        );
        assert_eq!(shape(b"\x1b[12"), ["ESC[unterminated"]);
        assert_eq!(shape(b"@mention text"), ["text(13)"]);
    }

    #[test]
    fn shape_caps_the_token_count() {
        let tokens = shape(&[b'\r'; MAX_TOKENS + 5]);
        assert_eq!(tokens.len(), MAX_TOKENS + 1);
        assert_eq!(tokens.last().map(String::as_str), Some("+5"));
    }

    #[test]
    fn purge_removes_only_expired_or_oldest_probe_files() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("20261001_1.jsonl");
        let fresh = dir.path().join("20261009_2.jsonl");
        let other = dir.path().join("keep.txt");
        for path in [&old, &fresh, &other] {
            fs::write(path, b"{}\n").unwrap();
        }
        let modified = fs::metadata(&fresh).unwrap().modified().unwrap();
        // 8 日後として見れば、すべての .jsonl が保存期間を過ぎている。
        purge(dir.path(), modified + Duration::from_secs(8 * 24 * 60 * 60));
        assert!(!old.exists());
        assert!(!fresh.exists());
        assert!(other.exists());

        fs::write(&fresh, vec![b'x'; 16]).unwrap();
        purge(dir.path(), SystemTime::now());
        assert!(fresh.exists());
    }
}
