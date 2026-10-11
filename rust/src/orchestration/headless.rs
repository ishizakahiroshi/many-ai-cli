//! Source: internal/headless/{adapter,runner,prompt}.go at 21d0bc7.
//! Process containment and bounded pipe ownership are exclusively shared process APIs.
#[cfg(all(test, windows))]
mod windows_contracts;

use super::headless_formats::{Event, Parser, parser_for};
use crate::{
    config::{HeadlessDef, Resource, RuntimePaths, normalize_headless_prompt_via, private_io},
    process::{
        Cancellation, ExitOutcome, ManagedProcess, OutputStream, ProcessEvent, ProcessOutput,
        ProcessPlan,
    },
};
use std::{
    fs::File,
    io::{self, Write},
    path::{Path, PathBuf},
};
use tokio::sync::broadcast::error::RecvError;

pub const SESSION_ID_PLACEHOLDER: &str = "{{many-ai-cli:session-id}}";
pub const MAX_LINE_BYTES: usize = 1 << 20;
pub fn expand_prompt(prompt: &str, session_id: i64) -> String {
    prompt.replace(SESSION_ID_PLACEHOLDER, &session_id.to_string())
}
pub fn build_argv(def: &HeadlessDef, launch_args: &[String], prompt: &str) -> Vec<String> {
    let mut args = def.args.clone();
    if normalize_headless_prompt_via(&def.prompt_via) == "arg" {
        args.push(prompt.into());
    }
    args.extend_from_slice(launch_args);
    args
}

/// Explicit log opt-in. Keys are relative to RuntimePaths::Logs; absolute prefixes
/// cannot make a trial write outside its selected root.
#[derive(Clone, Debug)]
pub struct RawLogs {
    pub paths: RuntimePaths,
    pub prefix: PathBuf,
}
#[derive(Clone)]
pub struct Spec {
    pub process: ProcessPlan,
    pub prompt: String,
    pub prompt_via: String,
    pub format: String,
    pub raw_logs: Option<RawLogs>,
    pub event_capacity: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunResult {
    pub state: String,
    pub exit_code: i32,
    pub timed_out: bool,
    pub canceled: bool,
    /// A streaming gap is surfaced and cancels the run, never a successful scan.
    pub dropped_chunks: u64,
    /// Raw-log errors do not veto an actual provider exit verdict.
    pub raw_log_errors: Vec<String>,
    pub process_output: ProcessOutput,
}
impl RunResult {
    fn from_output(
        output: ProcessOutput,
        dropped_chunks: u64,
        raw_log_errors: Vec<String>,
    ) -> Self {
        let (exit_code, timed_out, canceled) = match output.outcome {
            ExitOutcome::Exited { code, .. } => (code.unwrap_or(-1), false, false),
            ExitOutcome::TimedOut => (-1, true, false),
            ExitOutcome::Cancelled => (-1, false, true),
        };
        Self {
            state: if exit_code == 0 && !timed_out && !canceled && dropped_chunks == 0 {
                "completed"
            } else {
                "error"
            }
            .into(),
            exit_code,
            timed_out,
            canceled,
            dropped_chunks,
            raw_log_errors,
            process_output: output,
        }
    }
}
/// Readers are started by ManagedProcess before its prompt writer. The event
/// receiver is drained until closed, then the authoritative process is awaited.
/// Callbacks are serialized in this future and receive CRLF-framed terminal text.
pub async fn run(
    spec: Spec,
    cancel: &Cancellation,
    mut emit: impl FnMut(&[u8]),
) -> io::Result<RunResult> {
    let parser = parser_for(&spec.format).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("headless: no parser for output format {:?}", spec.format),
        )
    })?;
    let mut plan = spec.process;
    plan.stdin = if normalize_headless_prompt_via(&spec.prompt_via) == "stdin" {
        spec.prompt.into_bytes()
    } else {
        vec![]
    };
    // Honor cancellation before a process can be admitted.
    if cancel.is_cancelled() {
        return Ok(RunResult::from_output(
            ProcessOutput {
                stdout: vec![],
                stderr: vec![],
                stdout_truncated: false,
                stderr_truncated: false,
                pipes_forced_closed: false,
                outcome: ExitOutcome::Cancelled,
            },
            0,
            vec![],
        ));
    }
    let (mut raw_out, mut raw_err, mut raw_errors) = open_raw_logs(spec.raw_logs.as_ref());
    let (mut process, mut receiver) = ManagedProcess::spawn_owned(plan, spec.event_capacity.max(1));
    let mut stdout = LineFramer::default();
    let mut stderr = LineFramer::default();
    let mut dropped = 0u64;
    let mut canceled = false;
    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled(), if !canceled => { process.close(); canceled = true; },
            event = receiver.recv() => match event {
                Ok(ProcessEvent::Started { .. }) => {},
                Ok(ProcessEvent::Output { stream, bytes }) => {
                    let (framer, raw) = if stream == OutputStream::Stdout { (&mut stdout, &mut raw_out) } else { (&mut stderr, &mut raw_err) };
                    framer.push(&bytes, |line| deliver(line, stream, parser, raw, &mut raw_errors, &mut emit));
                }
                Err(RecvError::Closed) => break,
                Err(RecvError::Lagged(count)) => {
                    dropped += count;
                    // Lost bytes cannot safely be spliced into structured records.
                    stdout.clear(); stderr.clear(); process.close();
                    let line = format!("many-ai-cli: headless output lost ({count} chunks); stopping run\r\n");
                    emit(line.as_bytes());
                }
            }
        }
    }
    stdout.finish(|line| {
        deliver(
            line,
            OutputStream::Stdout,
            parser,
            &mut raw_out,
            &mut raw_errors,
            &mut emit,
        )
    });
    stderr.finish(|line| {
        deliver(
            line,
            OutputStream::Stderr,
            parser,
            &mut raw_err,
            &mut raw_errors,
            &mut emit,
        )
    });
    let output = loop {
        tokio::select! {
            result = process.wait() => break result?,
            _ = cancel.cancelled(), if !canceled => { process.close(); canceled = true; }
        }
    };
    Ok(RunResult::from_output(output, dropped, raw_errors))
}
fn deliver(
    line: &[u8],
    stream: OutputStream,
    parser: Parser,
    raw: &mut Option<File>,
    errors: &mut Vec<String>,
    emit: &mut impl FnMut(&[u8]),
) {
    if let Some(file) = raw
        && let Err(error) = file.write_all(line).and_then(|_| file.write_all(b"\n"))
    {
        errors.push(format!("raw {stream:?}: {error}"));
        *raw = None;
    }
    let events = if stream == OutputStream::Stdout {
        parser.parse(line)
    } else if line.is_empty() {
        vec![]
    } else {
        vec![Event::new("stderr", String::from_utf8_lossy(line))]
    };
    for event in events {
        emit(format!("{}\r\n", event.line()).as_bytes());
    }
}
fn open_raw_logs(raw: Option<&RawLogs>) -> (Option<File>, Option<File>, Vec<String>) {
    let Some(raw) = raw else {
        return (None, None, vec![]);
    };
    let mut errors = vec![];
    let mut open = |suffix: &str| {
        let result = (|| {
            let key = PathBuf::from(format!("{}{suffix}", raw.prefix.to_string_lossy()));
            let path = raw.paths.checked_child(Resource::Logs, &key)?;
            private_io::open_append(&path)
        })();
        match result {
            Ok(file) => Some(file),
            Err(e) => {
                errors.push(format!("raw log {suffix}: {e}"));
                None
            }
        }
    };
    let out = open(".stdout.log");
    let err = open(".stderr.log");
    (out, err, errors)
}
/// Go bufio.Reader's 64KiB ReadLine framing, including CR at a buffer boundary.
/// Every piece is drained. A boundary CR can defer delivery by one reader chunk,
/// so the reference's effective pending bound is MAX_LINE_BYTES + 64KiB - 1.
#[derive(Default)]
pub struct LineFramer {
    pending: Vec<u8>,
    chunk: Vec<u8>,
}
impl LineFramer {
    pub fn push(&mut self, bytes: &[u8], mut on_line: impl FnMut(&[u8])) {
        const READ_BUFFER: usize = 64 * 1024;
        for &byte in bytes {
            if byte == b'\n' {
                if self.chunk.last() == Some(&b'\r') {
                    self.chunk.pop();
                }
                self.pending.extend_from_slice(&self.chunk);
                self.chunk.clear();
                on_line(&self.pending);
                self.pending.clear();
            } else {
                self.chunk.push(byte);
                if self.chunk.len() == READ_BUFFER {
                    // ReadLine unreads a trailing CR on ErrBufferFull; the next
                    // fragment decides whether it is data or the CRLF delimiter.
                    let cr = self.chunk.last() == Some(&b'\r');
                    let end = self.chunk.len() - usize::from(cr);
                    self.pending.extend_from_slice(&self.chunk[..end]);
                    self.chunk.clear();
                    if cr {
                        self.chunk.push(b'\r');
                    }
                    if self.pending.len() >= MAX_LINE_BYTES {
                        on_line(&self.pending);
                        self.pending.clear();
                    }
                }
            }
        }
    }
    pub fn finish(&mut self, mut on_line: impl FnMut(&[u8])) {
        self.pending.extend_from_slice(&self.chunk);
        self.chunk.clear();
        if !self.pending.is_empty() {
            on_line(&self.pending);
            self.pending.clear();
        }
    }
    pub fn clear(&mut self) {
        self.pending.clear();
        self.chunk.clear();
    }
    pub fn pending_bytes(&self) -> usize {
        self.pending.len() + self.chunk.len()
    }
}

/// Resolve the caller's generated prefix before constructing RawLogs, if needed.
pub fn raw_log_path(paths: &RuntimePaths, relative: &Path) -> io::Result<PathBuf> {
    paths.checked_child(Resource::Logs, relative)
}

/// Structured Claude execution uses a bounded allowlist parser for every public
/// output. API billing also disables raw files and process capture, defensively
/// here as well as at the wrapper boundary. The legacy runner remains available
/// for the unchanged Go-oracle contract; production headless uses this entry.
pub async fn run_with_progress(
    mut spec: Spec,
    run_id: String,
    strict_api: bool,
    cancel: &Cancellation,
    mut emit: impl FnMut(&[u8]),
    mut progress: impl FnMut(super::headless_progress::HeadlessProgress),
) -> io::Result<RunResult> {
    use super::headless_progress::{ProgressParser, ProgressState, StrictLineFramer};
    use std::time::{Duration, Instant};
    let format = parser_for(&spec.format).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "headless output format unsupported",
        )
    })?;
    let structured = format == Parser::ClaudeStreamJson;
    if strict_api && !structured {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "API billing requires structured output",
        ));
    }
    if strict_api {
        spec.raw_logs = None;
        spec.process.output_cap = 0;
    }
    spec.process.stdin = if normalize_headless_prompt_via(&spec.prompt_via) == "stdin" {
        spec.prompt.into_bytes()
    } else {
        vec![]
    };
    let now = chrono::Utc::now().timestamp_millis();
    let mut tracker = ProgressParser::new(run_id.clone(), now);
    // Trusted run IDs, never a provider's session/response ID.
    super::headless_progress::HeadlessProgress::new(run_id.clone(), now)
        .validate()
        .map_err(io::Error::other)?;
    if cancel.is_cancelled() {
        let output = ProcessOutput {
            stdout: vec![],
            stderr: vec![],
            stdout_truncated: false,
            stderr_truncated: false,
            pipes_forced_closed: false,
            outcome: ExitOutcome::Cancelled,
        };
        progress(tracker.finish(&output, 0, 0));
        return Ok(RunResult::from_output(output, 0, vec![]));
    }
    let (mut raw_out, mut raw_err, mut raw_errors) = open_raw_logs(spec.raw_logs.as_ref());
    let (mut process, mut receiver) =
        ManagedProcess::spawn_owned(spec.process, spec.event_capacity.max(1));
    let mut strict_stdout = StrictLineFramer::default();
    let mut stdout = LineFramer::default();
    let mut stderr = LineFramer::default();
    let mut started = None;
    let mut dropped = 0u64;
    let mut canceled = false;
    let mut lost = false;
    let mut ticker = tokio::time::interval(Duration::from_millis(250));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_heartbeat = 0;
    let mut last_terminal = None;
    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled(), if !canceled => { process.close(); canceled = true; },
            _ = ticker.tick(), if started.is_some() => {
                let elapsed = started.map(|at: Instant| at.elapsed().as_millis() as u64).unwrap_or(0);
                let heartbeat = elapsed.saturating_sub(last_heartbeat) >= 1000;
                if heartbeat { tracker.heartbeat(); last_heartbeat = elapsed; }
                if let Some(snapshot) = tracker.snapshot(elapsed) {
                    // Heartbeats advance clocks without filling terminal history
                    // with repeated copies of the same completed utterance.
                    let key = (snapshot.state, snapshot.observed_turns, snapshot.result_turns, snapshot.active_tools.clone(), snapshot.cost.clone(), snapshot.last_assistant.clone(), snapshot.data_incomplete);
                    if structured && last_terminal.as_ref() != Some(&key) { emit(snapshot.terminal_line().as_bytes()); last_terminal = Some(key); }
                    progress(snapshot);
                }
            },
            event = receiver.recv() => match event {
                Ok(ProcessEvent::Started { .. }) => {
                    started = Some(Instant::now());
                    tracker = ProgressParser::new(run_id.clone(), chrono::Utc::now().timestamp_millis());
                    if let Some(snapshot) = tracker.snapshot(0) { progress(snapshot); }
                },
                Ok(ProcessEvent::Output { stream, bytes }) if !lost => {
                    if structured {
                        if !strict_api {
                            // Plan-mode raw logging remains an explicit existing
                            // opt-in. It is never a source for public projection.
                            let raw = if stream == OutputStream::Stdout { &mut raw_out } else { &mut raw_err };
                            if let Some(file) = raw && file.write_all(&bytes).is_err() { raw_errors.push("headless raw log write failed".into()); *raw = None; }
                        }
                        if stream == OutputStream::Stdout {
                            strict_stdout.push(&bytes, |line| match line { Some(line) => tracker.observe(line), None => tracker.mark_incomplete() });
                            if tracker.has_incomplete_data() { process.close(); lost = true; strict_stdout.clear(); }
                        }
                        // stderr never becomes public text, including auth errors.
                    } else {
                        let (framer, raw) = if stream == OutputStream::Stdout { (&mut stdout, &mut raw_out) } else { (&mut stderr, &mut raw_err) };
                        framer.push(&bytes, |line| deliver(line, stream, format, raw, &mut raw_errors, &mut emit));
                    }
                },
                Ok(ProcessEvent::Output { .. }) => {},
                Err(RecvError::Closed) => break,
                Err(RecvError::Lagged(count)) => {
                    dropped = dropped.saturating_add(count);
                    lost = true;
                    strict_stdout.clear(); stdout.clear(); stderr.clear();
                    tracker.mark_incomplete(); process.close();
                },
            }
        }
    }
    if !lost {
        if structured {
            strict_stdout.finish(|line| match line {
                Some(line) => tracker.observe(line),
                None => tracker.mark_incomplete(),
            });
        } else {
            stdout.finish(|line| {
                deliver(
                    line,
                    OutputStream::Stdout,
                    format,
                    &mut raw_out,
                    &mut raw_errors,
                    &mut emit,
                )
            });
            stderr.finish(|line| {
                deliver(
                    line,
                    OutputStream::Stderr,
                    format,
                    &mut raw_err,
                    &mut raw_errors,
                    &mut emit,
                )
            });
        }
    }
    let output = loop {
        tokio::select! {
            result = process.wait() => break match result {
                Ok(output) => output,
                Err(error) => {
                    let output = ProcessOutput { stdout: vec![], stderr: vec![], stdout_truncated: false, stderr_truncated: false, pipes_forced_closed: false, outcome: ExitOutcome::Exited { code: Some(-1), signal: None } };
                    let elapsed = started.map(|at| at.elapsed().as_millis() as u64).unwrap_or(0);
                    let final_progress = tracker.finish(&output, dropped, elapsed);
                    if structured { emit(final_progress.terminal_line().as_bytes()); }
                    progress(final_progress);
                    return Err(if strict_api { io::Error::new(error.kind(), "headless provider execution failed") } else { error });
                }
            },
            _ = cancel.cancelled(), if !canceled => { process.close(); canceled = true; }
        }
    };
    let elapsed = started
        .map(|at| at.elapsed().as_millis() as u64)
        .unwrap_or(0);
    let final_progress = tracker.finish(&output, dropped, elapsed);
    let failed = final_progress.state != ProgressState::Completed;
    if structured {
        emit(final_progress.terminal_line().as_bytes());
    }
    progress(final_progress);
    let mut result = RunResult::from_output(output, dropped, raw_errors);
    if failed {
        result.state = "error".into();
    }
    Ok(result)
}

#[cfg(test)]
mod progress_contracts {
    use super::*;
    use crate::orchestration::headless_progress::{CostKind, ProgressState};
    use std::{collections::BTreeMap, time::Duration};

    fn fixture(mode: &str, dir: &Path) -> Spec {
        let records = [
            r#"{"type":"system","subtype":"init","auth":"PRIVATE_INIT_SENTINEL"}"#,
            r#"{"type":"system","subtype":"permission_denied","message":"PRIVATE_SYSTEM_SENTINEL","tool_name":"Bash","tool_use_id":"tool_zero"}"#,
            r#"{"type":"user","message":{"role":"user","content":"PRIVATE_USER_SENTINEL"}}"#,
            r#"{"type":"stream_event","event":{"type":"message_start","message":{"id":"msg_one","model":"claude-sonnet-4-5-20250929","usage":{"input_tokens":1000}}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"PRIVATE_PARTIAL_SENTINEL"}}}"#,
            r#"{"type":"assistant","uuid":"text-zero","message":{"id":"msg_one","content":[{"type":"text","text":"Ready sk-ant-"}]}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_stop","index":0}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}}"#,
            r#"{"type":"assistant","uuid":"text-one","message":{"id":"msg_one","content":[{"type":"text","text":"SYNTHETIC_ONLY_NOT_A_REAL_KEY_1234567890"}]}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_stop","index":1}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"tool_1","name":"Read","input":{"command":"PRIVATE_ARGUMENT_SENTINEL"}}}}"#,
            r#"{"type":"assistant","uuid":"tool-two","message":{"id":"msg_one","content":[{"type":"tool_use","id":"tool_1","name":"Read","input":{"command":"PRIVATE_ARGUMENT_SENTINEL"}}]}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_stop","index":2}}"#,
            r#"{"type":"stream_event","event":{"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":30}}}"#,
            r#"{"type":"stream_event","event":{"type":"message_stop"}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tool_1","content":"PRIVATE_RESULT_SENTINEL"}]}}"#,
            r#"{"type":"result","subtype":"success","num_turns":2,"total_cost_usd":0.123,"result":"PRIVATE_RESULT_TEXT_SENTINEL"}"#,
        ];
        // A fixed synthetic file avoids libtest's stdout preamble. No provider,
        // credential or user configuration is touched by these child processes.
        let body = match mode {
            "safe" | "bad_exit" | "split_records" => records.join("\n"), // final record has no LF
            "api_error" => [
                r#"{"type":"assistant","is_api_error_message":true,"error":"authentication_failed","message":{"id":"error","content":[{"type":"text","text":"API Error: PRIVATE_ERROR_SENTINEL"}]}}"#,
                r#"{"type":"result","subtype":"error_during_execution","is_error":true,"total_cost_usd":0.01}"#,
            ].join("\n"),
            "malformed" => "PRIVATE_BAD_JSON_SENTINEL\n".into(),
            "sleep" => String::new(),
            "burst" => "{\"type\":\"system\"}\n".repeat(100_000),
            _ => panic!("unknown synthetic mode"),
        };
        std::fs::write(dir.join("synthetic-progress.jsonl"), body).unwrap();
        let exit = if mode == "bad_exit" { 7 } else { 0 };
        #[cfg(unix)]
        let (executable, args) = (
            "/bin/sh",
            vec![
                "-c".into(),
                if mode == "sleep" {
                    "sleep 2".into()
                } else if mode == "split_records" {
                    "while IFS= read -r line || [ -n \"$line\" ]; do printf '%s\\n' \"$line\"; sleep 0.08; done < synthetic-progress.jsonl".into()
                } else {
                    format!("printf '%s\\n' 'PRIVATE_STDERR_SENTINEL' >&2; cat synthetic-progress.jsonl; exit {exit}").into()
                },
            ],
        );
        #[cfg(windows)]
        let (executable, args) = (
            "pwsh",
            vec![
                "-NoProfile".into(),
                "-Command".into(),
                if mode == "sleep" {
                    "Start-Sleep -Seconds 2".into()
                } else if mode == "split_records" {
                    "Get-Content synthetic-progress.jsonl | ForEach-Object { [Console]::Out.WriteLine($_); [Console]::Out.Flush(); Start-Sleep -Milliseconds 80 }".into()
                } else {
                    format!("[Console]::Error.WriteLine('PRIVATE_STDERR_SENTINEL'); $b=[IO.File]::ReadAllBytes((Join-Path (Get-Location) 'synthetic-progress.jsonl')); [Console]::OpenStandardOutput().Write($b,0,$b.Length); exit {exit}").into()
                },
            ],
        );
        Spec {
            process: ProcessPlan {
                executable: executable.into(),
                args,
                cwd: dir.into(),
                env: BTreeMap::new(),
                stdin: vec![],
                timeout: Duration::from_secs(5),
                output_cap: 4096,
                pipe_drain_timeout: Duration::from_millis(100),
            },
            prompt: String::new(),
            prompt_via: "stdin".into(),
            format: "claude-stream-json".into(),
            raw_logs: None,
            event_capacity: 1024,
        }
    }
    #[tokio::test]
    async fn api_projection_drops_raw_logs_capture_and_sentinel_bodies() {
        let root = tempfile::tempdir().unwrap();
        let installed = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::trial(root.path(), 49331, installed.path()).unwrap();
        let mut spec = fixture("safe", root.path());
        spec.raw_logs = Some(RawLogs {
            paths: paths.clone(),
            prefix: "should-not-exist".into(),
        });
        let mut terminal = vec![];
        let mut snapshots = vec![];
        let result = run_with_progress(
            spec,
            "1:2".into(),
            true,
            &Cancellation::default(),
            |b| terminal.extend_from_slice(b),
            |p| snapshots.push(p),
        )
        .await
        .unwrap();
        assert_eq!(result.state, "completed");
        assert!(result.process_output.stdout.is_empty());
        assert!(result.process_output.stderr.is_empty());
        assert!(!paths.resource(Resource::Logs).exists());
        let wire = serde_json::to_string(&snapshots).unwrap();
        let terminal = String::from_utf8(terminal).unwrap();
        assert!(!wire.contains("SENTINEL"));
        assert!(!terminal.contains("SENTINEL"));
        assert!(!wire.contains("SYNTHETIC_ONLY"));
        assert!(!terminal.contains("SYNTHETIC_ONLY"));
        let last = snapshots.last().unwrap();
        assert_eq!(last.last_assistant, "Ready ***");
        assert_eq!(last.cost.amount_usd, Some(0.123));
        assert_eq!(last.result_turns, Some(2));
        assert!(last.active_tools.is_empty());
        assert_eq!(last.state, ProgressState::Completed);
        last.validate().unwrap();
        assert!(snapshots.windows(2).all(|s| s[0].sequence < s[1].sequence));
    }
    #[tokio::test]
    async fn malformed_record_and_nonzero_exit_cannot_be_success() {
        let root = tempfile::tempdir().unwrap();
        for mode in ["malformed", "bad_exit"] {
            let mut snapshots = vec![];
            let mut terminal = vec![];
            let result = run_with_progress(
                fixture(mode, root.path()),
                "1:2".into(),
                true,
                &Cancellation::default(),
                |b| terminal.extend_from_slice(b),
                |p| snapshots.push(p),
            )
            .await
            .unwrap();
            assert_eq!(result.state, "error");
            assert_eq!(snapshots.last().unwrap().state, ProgressState::Failed);
            assert!(!String::from_utf8(terminal).unwrap().contains("SENTINEL"));
            if mode == "bad_exit" {
                assert_eq!(snapshots.last().unwrap().cost.kind, CostKind::Result);
            } else {
                assert!(snapshots.last().unwrap().data_incomplete);
            }
        }
    }
    #[tokio::test]
    async fn silent_run_has_monotonic_heartbeat_and_cancellation_freezes_final() {
        let root = tempfile::tempdir().unwrap();
        let cancel = Cancellation::default();
        let child = cancel.clone();
        // Cancel only after a real running heartbeat, rather than racing a
        // timer started before the owned process has emitted Started. The
        // fixture's process timeout still bounds a missing-heartbeat failure.
        let mut snapshots = vec![];
        run_with_progress(
            fixture("sleep", root.path()),
            "1:2".into(),
            true,
            &cancel,
            |_| {},
            |p| {
                if p.state == ProgressState::Running && p.elapsed_ms >= 900 {
                    child.cancel();
                }
                snapshots.push(p);
            },
        )
        .await
        .unwrap();
        assert!(
            snapshots
                .iter()
                .any(|p| p.state == ProgressState::Running && p.elapsed_ms >= 900)
        );
        assert_eq!(snapshots.last().unwrap().state, ProgressState::Canceled);
        assert!(
            snapshots
                .windows(2)
                .all(|s| s[0].elapsed_ms <= s[1].elapsed_ms)
        );
        assert!(snapshots.last().unwrap().ended_at_ms.is_some());
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn lag_marks_incomplete_and_stops_owned_provider() {
        let root = tempfile::tempdir().unwrap();
        let mut spec = fixture("burst", root.path());
        spec.event_capacity = 1;
        let mut snapshots = vec![];
        let result = run_with_progress(
            spec,
            "1:2".into(),
            true,
            &Cancellation::default(),
            |_| {},
            |p| {
                if snapshots.is_empty() {
                    std::thread::sleep(Duration::from_millis(150));
                }
                snapshots.push(p);
            },
        )
        .await
        .unwrap();
        assert!(result.dropped_chunks > 0);
        assert_eq!(result.state, "error");
        let last = snapshots.last().unwrap();
        assert_eq!(last.state, ProgressState::Failed);
        assert!(last.data_incomplete);
    }
    #[tokio::test]
    async fn separated_record_process_never_emits_unmasked_fragments_or_error_bodies() {
        let root = tempfile::tempdir().unwrap();
        for mode in ["split_records", "api_error"] {
            let mut snapshots = vec![];
            let mut terminal = vec![];
            let result = run_with_progress(
                fixture(mode, root.path()),
                "1:2".into(),
                true,
                &Cancellation::default(),
                |b| terminal.extend_from_slice(b),
                |p| snapshots.push(p),
            )
            .await
            .unwrap();
            let serialized = serde_json::to_string(&snapshots).unwrap();
            let terminal = String::from_utf8(terminal).unwrap();
            for forbidden in ["sk-ant-", "SYNTHETIC_ONLY", "PRIVATE_"] {
                assert!(!serialized.contains(forbidden));
                assert!(!terminal.contains(forbidden));
            }
            if mode == "split_records" {
                assert_eq!(result.state, "completed");
                assert!(snapshots.iter().any(|p| p.state == ProgressState::Running
                    && p.elapsed_ms > 200
                    && p.last_assistant.is_empty()));
                assert_eq!(snapshots.last().unwrap().last_assistant, "Ready ***");
                assert!(!snapshots.last().unwrap().data_incomplete);
            } else {
                assert_eq!(result.state, "error");
                assert!(snapshots.iter().all(|p| p.last_assistant.is_empty()));
                assert_eq!(snapshots.last().unwrap().state, ProgressState::Failed);
            }
        }
    }
}
