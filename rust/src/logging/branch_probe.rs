//! 一時観測 `rust-branch-probe`（instrumentation.json に登録）。原因が確定したら撤去する。
//!
//! 何のためか: Rust 版のサイドバーで、git リポジトリのセッションが「(no git)」のまま残ることがある
//! （2026-10-16・docs/local/bugfix_session-branch-no-git-on-start_2026-10-16.md）。
//! ブランチ取得は失敗の理由を区別せず空の文字列にしてしまうため、起動時の取得（250ms の制限で 1 回）が
//! 時間切れで空になったのか、git の起動失敗や 0 以外の終了なのか、2 秒ごとの再取得がなぜ直せなかったのかを
//! 後から確かめられない。そこで 1 回の取得ごとに、結果の種類・失敗の理由・かかった時間を 1 行残す。
//!
//! 守っていること:
//! - debug ビルドでだけ組み込まれる（logging/mod.rs の cfg）。`--release` の成果物には入らない
//! - 生のパス・ブランチ名・git の出力本文・環境変数の値は残さない。作業フォルダは
//!   パスから作る短いハッシュ（`cwd_id`）で、register と refresh で同じ関数を使い突き合わせる。
//!   結果は `branch` / `detached` / `empty` の種類だけ
//! - `MANY_AI_CLI_BRANCH_PROBE=0` で止まる。テストでは書かない
//! - 置き場所は `<logs>/branch-probe/<日付>_<pid>.jsonl`（input-probe の隣）。各プロセスが最初に書く前に、
//!   7 日より古いファイルと、合計 20MB を超えた古い分を消す。1 ファイルは 4MB で書き止める。
//!   フォルダごと消しても動作に影響しない。書き込みの失敗は捨てて、呼び出し元を止めない
//!
//! 取り出し方: `GitBranchSource::branch` の返り値と trait は変えず、`files::branch` が内部で詳しい結果
//! （`BranchLookupDetail`）を作って `note_lookup` へ渡す。呼び出し側は `scope` で包んだ範囲の
//! タスクローカルな入れ物からそれを受け取り、phase・セッション番号などと合わせて 1 行にする。
use crate::files::branch::{BranchLookupDetail, LookupStep, StepOutcome};
use serde_json::{Map, Value, json};
use std::{
    cell::RefCell,
    ffi::OsStr,
    fs::{self, File, OpenOptions},
    future::Future,
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const DIRECTORY_NAME: &str = "branch-probe";
const DISABLE_ENV: &str = "MANY_AI_CLI_BRANCH_PROBE";
const RETENTION: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const TOTAL_LIMIT: u64 = 20 * 1024 * 1024;
const FILE_LIMIT: u64 = 4 * 1024 * 1024;
/// 再取得の行を残す時間の下限。取得の上限 250ms に近づいたものと、枠待ちが長いものを拾う。
const SLOW: Duration = Duration::from_millis(200);

static DIRECTORY: OnceLock<Option<PathBuf>> = OnceLock::new();
static SINK: Mutex<Sink> = Mutex::new(Sink::Unopened);

tokio::task_local! {
    static COLLECTED: RefCell<Collected>;
}

enum Sink {
    Unopened,
    Open { file: File, written: u64 },
    Failed,
}

/// `scope` の中で集めたもの。取得の詳しい結果と、再取得を各セッションへ当てた結果。
#[derive(Default)]
pub(crate) struct Collected {
    lookup: Option<BranchLookupDetail>,
    /// `(セッション番号, apply_branch_observation が true を返したか)`。
    applied: Vec<(i64, bool)>,
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

fn active() -> bool {
    matches!(DIRECTORY.get(), Some(Some(_)))
}

/// `fut` の中で呼ばれた `note_lookup` / `note_applied` を集めて、結果と一緒に返す。
/// `fut` は同じタスクの中で await されること（別タスクへ spawn した先の記録は集まらない）。
pub(crate) async fn scope<F: Future>(fut: F) -> (F::Output, Collected) {
    COLLECTED
        .scope(RefCell::new(Collected::default()), async {
            let output = fut.await;
            (output, COLLECTED.with(|slot| slot.take()))
        })
        .await
}

/// `files::branch` がブランチ取得 1 回ごとに呼ぶ。`scope` の外では何もしない。
pub(crate) fn note_lookup(detail: &BranchLookupDetail) {
    if !active() {
        return;
    }
    let _ = COLLECTED.try_with(|slot| slot.borrow_mut().lookup = Some(detail.clone()));
}

/// 再取得の結果を 1 セッションへ当てたとき。`changed` が true なら画面へ送る更新が作られた。
pub(crate) fn note_applied(session: i64, changed: bool) {
    if !active() {
        return;
    }
    let _ = COLLECTED.try_with(|slot| slot.borrow_mut().applied.push((session, changed)));
}

/// 起動時（`register`）と再接続時（`reattach`）の取得の直後。
/// 起動時はセッション番号がまだ無い。再接続時は要求された番号。
pub(crate) fn record_register(
    entry: &'static str,
    session: Option<i64>,
    cwd: &str,
    branch: &str,
    collected: &Collected,
    total: Duration,
) {
    if !active() {
        return;
    }
    let sessions: Vec<i64> = session.into_iter().collect();
    record(lookup_line(
        "register",
        Some(entry),
        cwd,
        &sessions,
        branch,
        collected,
        None,
        total,
    ));
}

/// 起動時の取得のあと、セッション番号が決まった時点。同じ作業フォルダのハッシュで取得の行と結び付ける。
pub(crate) fn record_assigned(session: i64, cwd: &str, branch: &str) {
    if !active() {
        return;
    }
    record(json!({
        "event": "assigned",
        "cwd": cwd_id(cwd),
        "sessions": [session],
        "result": result_kind(branch),
    }));
}

/// 2 秒ごとの再取得 1 回分。`slot_wait` は 4 つの枠の空きを待った時間、`total` は枠を得てから公開までの時間。
/// 何も起きなかった回（取れた・速い・画面へ送る更新なし）は残さない。全部残すと作業フォルダ 8 個で
/// 1 秒に約 4 行になり、1 ファイル 4MB の上限に 1 時間ほどで届いて、その後の再発を記録できなくなるため。
pub(crate) fn record_refresh(
    cwd: &str,
    ids: &[i64],
    branch: &str,
    collected: &Collected,
    slot_wait: Duration,
    total: Duration,
) {
    if !active() || !refresh_worth_recording(branch, collected, slot_wait) {
        return;
    }
    record(lookup_line(
        "refresh",
        None,
        cwd,
        ids,
        branch,
        collected,
        Some(slot_wait),
        total,
    ));
}

/// 再取得の行を残すか。空だった・どちらかの手順が失敗した・詳細が届かなかった・画面へ送る更新を作った・
/// 取得か枠待ちが `SLOW` 以上かかった、のどれかに当たる回だけ残す。
fn refresh_worth_recording(branch: &str, collected: &Collected, slot_wait: Duration) -> bool {
    if branch.is_empty() || slot_wait >= SLOW || collected.applied.iter().any(|(_, sent)| *sent) {
        return true;
    }
    let Some(detail) = &collected.lookup else {
        return true;
    };
    detail.elapsed >= SLOW
        || [&detail.first, &detail.second]
            .into_iter()
            .flatten()
            .any(|step| step.outcome != StepOutcome::Ok)
}

/// 作業フォルダを、生のパスを残さずに突き合わせるための短いハッシュ。
/// 空白を除いたパスの FNV-1a 64 ビットの先頭 10 桁。register と refresh は同じ関数を使う。
pub(crate) fn cwd_id(cwd: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in cwd.trim().bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")[..10].to_owned()
}

fn result_kind(branch: &str) -> &'static str {
    if branch.is_empty() {
        "empty"
    } else if branch.starts_with("detached:") {
        "detached"
    } else {
        "branch"
    }
}

fn millis(duration: Duration) -> u64 {
    duration.as_millis() as u64
}

fn step_value(step: &LookupStep) -> Value {
    let mut fields = Map::new();
    fields.insert("ms".into(), json!(millis(step.elapsed)));
    match &step.outcome {
        StepOutcome::Ok => {
            fields.insert("outcome".into(), json!("ok"));
        }
        StepOutcome::MissingExecutable => {
            fields.insert("outcome".into(), json!("missing_executable"));
        }
        StepOutcome::Start => {
            fields.insert("outcome".into(), json!("start"));
        }
        StepOutcome::Expired => {
            fields.insert("outcome".into(), json!("expired"));
        }
        StepOutcome::Output {
            exit_code,
            signal,
            timed_out,
            cancelled,
            stdout_truncated,
            stderr_truncated,
            pipes_forced_closed,
        } => {
            fields.insert("outcome".into(), json!("output"));
            fields.insert("exit".into(), json!(exit_code));
            fields.insert("signal".into(), json!(signal));
            fields.insert("timed_out".into(), json!(timed_out));
            fields.insert("cancelled".into(), json!(cancelled));
            fields.insert("stdout_truncated".into(), json!(stdout_truncated));
            fields.insert("stderr_truncated".into(), json!(stderr_truncated));
            fields.insert("pipes_forced_closed".into(), json!(pipes_forced_closed));
        }
    }
    Value::Object(fields)
}

/// 空になった理由。結果が空でないときは `null`。
fn reason(detail: &BranchLookupDetail, branch: &str) -> Value {
    if !branch.is_empty() {
        return Value::Null;
    }
    let reason = if detail.blank_cwd {
        "blank_cwd"
    } else if detail
        .first
        .as_ref()
        .is_some_and(|step| step.outcome != StepOutcome::Ok)
    {
        "first_failed"
    } else {
        match &detail.second {
            Some(step) if step.outcome != StepOutcome::Ok => "second_failed",
            Some(_) => "second_empty",
            None => "first_empty",
        }
    };
    json!(reason)
}

#[allow(clippy::too_many_arguments)]
fn lookup_line(
    phase: &'static str,
    entry: Option<&'static str>,
    cwd: &str,
    sessions: &[i64],
    branch: &str,
    collected: &Collected,
    slot_wait: Option<Duration>,
    total: Duration,
) -> Value {
    let mut line = Map::new();
    line.insert("event".into(), json!("lookup"));
    line.insert("phase".into(), json!(phase));
    if let Some(entry) = entry {
        line.insert("entry".into(), json!(entry));
    }
    line.insert("cwd".into(), json!(cwd_id(cwd)));
    line.insert("sessions".into(), json!(sessions));
    line.insert("result".into(), json!(result_kind(branch)));
    match &collected.lookup {
        Some(detail) => {
            line.insert("reason".into(), reason(detail, branch));
            line.insert(
                "first".into(),
                detail.first.as_ref().map_or(Value::Null, step_value),
            );
            line.insert(
                "second".into(),
                detail.second.as_ref().map_or(Value::Null, step_value),
            );
            line.insert("lookup_ms".into(), json!(millis(detail.elapsed)));
        }
        None => {
            // 取得の詳細が届かなかった（この経路では branch() が呼ばれなかった）。
            line.insert("reason".into(), json!("no_detail"));
            line.insert("first".into(), Value::Null);
            line.insert("second".into(), Value::Null);
            line.insert("lookup_ms".into(), Value::Null);
        }
    }
    line.insert("total_ms".into(), json!(millis(total)));
    if let Some(wait) = slot_wait {
        line.insert("slot_wait_ms".into(), json!(millis(wait)));
        let applied: Vec<Value> = collected
            .applied
            .iter()
            .map(|(session, sent)| json!({ "session": session, "sent": sent }))
            .collect();
        line.insert("applied".into(), Value::Array(applied));
    }
    Value::Object(line)
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
    // 観測が本来のブランチ取得を止めてはいけないので、ロックの失敗も書き込みの失敗も捨てる。
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

#[cfg(test)]
mod tests {
    use super::*;

    fn step(outcome: StepOutcome, ms: u64) -> LookupStep {
        LookupStep {
            outcome,
            elapsed: Duration::from_millis(ms),
        }
    }

    fn output(code: Option<i32>, timed_out: bool) -> StepOutcome {
        StepOutcome::Output {
            exit_code: code,
            signal: None,
            timed_out,
            cancelled: false,
            stdout_truncated: false,
            stderr_truncated: false,
            pipes_forced_closed: false,
        }
    }

    fn detail(
        first: Option<LookupStep>,
        second: Option<LookupStep>,
        ms: u64,
    ) -> BranchLookupDetail {
        BranchLookupDetail {
            blank_cwd: false,
            first,
            second,
            elapsed: Duration::from_millis(ms),
        }
    }

    #[test]
    fn cwd_id_is_short_stable_and_hides_the_path() {
        // FNV-1a 64 ビットの既知の値（"abc"）。
        assert_eq!(cwd_id("abc"), "e71fa21905");
        let path = "/synthetic/secret-repo";
        let id = cwd_id(path);
        assert_eq!(id.len(), 10);
        assert!(id.bytes().all(|b| b.is_ascii_hexdigit()));
        assert!(!id.contains("secret"));
        // register は空白付きのまま、refresh は trim 済みで呼ぶ。同じ値になること。
        assert_eq!(cwd_id(&format!("  {path}\n")), id);
        assert_ne!(cwd_id("/synthetic/other-repo"), id);
    }

    #[test]
    fn probe_is_on_by_default_and_stops_with_zero() {
        assert!(enabled(None));
        assert!(enabled(Some(OsStr::new("1"))));
        assert!(enabled(Some(OsStr::new(""))));
        assert!(!enabled(Some(OsStr::new("0"))));
    }

    #[test]
    fn register_line_names_the_expired_first_step_without_the_path() {
        let collected = Collected {
            lookup: Some(detail(Some(step(StepOutcome::Expired, 251)), None, 251)),
            applied: vec![],
        };
        let cwd = "D:/synthetic/secret-repo";
        let line = lookup_line(
            "register",
            Some("register"),
            cwd,
            &[],
            "",
            &collected,
            None,
            Duration::from_millis(260),
        );
        assert_eq!(line["event"], "lookup");
        assert_eq!(line["phase"], "register");
        assert_eq!(line["entry"], "register");
        assert_eq!(line["cwd"], cwd_id(cwd));
        assert_eq!(line["sessions"], json!([]));
        assert_eq!(line["result"], "empty");
        assert_eq!(line["reason"], "first_failed");
        assert_eq!(line["first"], json!({"ms": 251, "outcome": "expired"}));
        assert_eq!(line["second"], Value::Null);
        assert_eq!(line["lookup_ms"], 251);
        assert_eq!(line["total_ms"], 260);
        assert!(line.get("slot_wait_ms").is_none());
        assert!(!line.to_string().contains("secret"));
    }

    #[test]
    fn refresh_line_carries_wait_and_what_was_sent_per_session() {
        let collected = Collected {
            lookup: Some(detail(
                Some(step(StepOutcome::Ok, 40)),
                Some(step(StepOutcome::Ok, 30)),
                71,
            )),
            applied: vec![(7, true), (9, false)],
        };
        let line = lookup_line(
            "refresh",
            None,
            "/synthetic/repo",
            &[7, 9],
            "detached:abc1234",
            &collected,
            Some(Duration::from_millis(1500)),
            Duration::from_millis(120),
        );
        assert_eq!(line["phase"], "refresh");
        assert!(line.get("entry").is_none());
        assert_eq!(line["sessions"], json!([7, 9]));
        assert_eq!(line["result"], "detached");
        assert_eq!(line["reason"], Value::Null);
        assert_eq!(line["slot_wait_ms"], 1500);
        assert_eq!(
            line["applied"],
            json!([{"session": 7, "sent": true}, {"session": 9, "sent": false}])
        );
        assert!(!line.to_string().contains("abc1234"));
    }

    #[test]
    fn reasons_tell_the_two_steps_and_output_details_apart() {
        let failed = detail(Some(step(output(Some(128), false), 12)), None, 12);
        assert_eq!(reason(&failed, ""), json!("first_failed"));
        assert_eq!(reason(&failed, "main"), Value::Null);
        let second_failed = detail(
            Some(step(StepOutcome::Ok, 10)),
            Some(step(output(None, true), 240)),
            250,
        );
        assert_eq!(reason(&second_failed, ""), json!("second_failed"));
        let second_empty = detail(
            Some(step(StepOutcome::Ok, 10)),
            Some(step(StepOutcome::Ok, 5)),
            15,
        );
        assert_eq!(reason(&second_empty, ""), json!("second_empty"));
        let first_empty = detail(Some(step(StepOutcome::Ok, 10)), None, 10);
        assert_eq!(reason(&first_empty, ""), json!("first_empty"));
        let blank = BranchLookupDetail {
            blank_cwd: true,
            ..BranchLookupDetail::default()
        };
        assert_eq!(reason(&blank, ""), json!("blank_cwd"));

        let value = step_value(&step(
            StepOutcome::Output {
                exit_code: None,
                signal: None,
                timed_out: true,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: true,
                pipes_forced_closed: true,
            },
            250,
        ));
        assert_eq!(
            value,
            json!({
                "ms": 250, "outcome": "output", "exit": null, "signal": null,
                "timed_out": true, "cancelled": false, "stdout_truncated": false,
                "stderr_truncated": true, "pipes_forced_closed": true,
            })
        );
        assert_eq!(step_value(&step(output(Some(128), false), 3))["exit"], 128);
    }

    #[test]
    fn quiet_refreshes_are_skipped_and_anything_notable_is_kept() {
        let quiet = || Collected {
            lookup: Some(detail(Some(step(StepOutcome::Ok, 40)), None, 40)),
            applied: vec![(7, false)],
        };
        let fast = Duration::from_millis(5);
        assert!(!refresh_worth_recording("main", &quiet(), fast));
        // 空・枠待ちが長い・画面へ送った・取得が遅い・手順の失敗・詳細なし は残す。
        assert!(refresh_worth_recording("", &quiet(), fast));
        assert!(refresh_worth_recording("main", &quiet(), SLOW));
        let mut sent = quiet();
        sent.applied = vec![(7, false), (9, true)];
        assert!(refresh_worth_recording("main", &sent, fast));
        let mut slow = quiet();
        slow.lookup = Some(detail(Some(step(StepOutcome::Ok, 210)), None, 210));
        assert!(refresh_worth_recording("main", &slow, fast));
        let mut second_failed = quiet();
        second_failed.lookup = Some(detail(
            Some(step(StepOutcome::Ok, 10)),
            Some(step(output(None, true), 30)),
            40,
        ));
        assert!(refresh_worth_recording(
            "detached:abc",
            &second_failed,
            fast
        ));
        assert!(refresh_worth_recording("main", &Collected::default(), fast));
    }

    #[test]
    fn missing_detail_is_reported_instead_of_guessed() {
        let line = lookup_line(
            "register",
            Some("reattach"),
            "x",
            &[5],
            "main",
            &Collected::default(),
            None,
            Duration::ZERO,
        );
        assert_eq!(line["reason"], "no_detail");
        assert_eq!(line["result"], "branch");
        assert_eq!(line["sessions"], json!([5]));
    }

    #[tokio::test]
    async fn notes_are_collected_only_inside_scope() {
        // scope の外では何も起きない（panic もしない）。
        note_applied(1, true);
        // テストでは init が何もしないので active() は false。入れ物への記録は
        // 本番と同じ経路で確かめるため、collected へ直接書く関数を通す。
        let ((), collected) = scope(async {
            COLLECTED.with(|slot| {
                let mut slot = slot.borrow_mut();
                slot.applied.push((3, true));
                slot.lookup = Some(BranchLookupDetail::default());
            });
        })
        .await;
        assert_eq!(collected.applied, [(3, true)]);
        assert!(collected.lookup.is_some());
        let ((), empty) = scope(async {}).await;
        assert!(empty.applied.is_empty() && empty.lookup.is_none());
    }

    #[test]
    fn tests_never_write_a_file() {
        let dir = tempfile::tempdir().unwrap();
        init(dir.path());
        assert!(!active());
        record_assigned(1, "/synthetic/repo", "main");
        record_register(
            "register",
            None,
            "/synthetic/repo",
            "main",
            &Collected::default(),
            Duration::ZERO,
        );
        record_refresh(
            "/synthetic/repo",
            &[1],
            "main",
            &Collected::default(),
            Duration::ZERO,
            Duration::ZERO,
        );
        assert!(!dir.path().join(DIRECTORY_NAME).exists());
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
        purge(dir.path(), modified + Duration::from_secs(8 * 24 * 60 * 60));
        assert!(!old.exists());
        assert!(!fresh.exists());
        assert!(other.exists());

        fs::write(&fresh, vec![b'x'; 16]).unwrap();
        purge(dir.path(), SystemTime::now());
        assert!(fresh.exists());
    }

    #[test]
    fn open_writes_one_json_object_per_line_under_the_probe_directory() {
        let dir = tempfile::tempdir().unwrap();
        let directory = dir.path().join(DIRECTORY_NAME);
        let Some(Sink::Open { mut file, .. }) = open(&directory, "2026-10-16T14:00:00Z") else {
            panic!("probe file must open");
        };
        let line = lookup_line(
            "register",
            Some("register"),
            "/synthetic/repo",
            &[],
            "main",
            &Collected::default(),
            None,
            Duration::ZERO,
        );
        let mut bytes = serde_json::to_vec(&line).unwrap();
        bytes.push(b'\n');
        file.write_all(&bytes).unwrap();
        let name = format!("20261016_{}.jsonl", std::process::id());
        let text = fs::read_to_string(directory.join(name)).unwrap();
        assert_eq!(text.lines().count(), 1);
        let parsed: Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(parsed["event"], "lookup");
    }
}
