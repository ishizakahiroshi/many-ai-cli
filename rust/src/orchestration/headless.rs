//! Source: internal/headless/{adapter,runner,prompt}.go at 21d0bc7.
//! Process containment and bounded pipe ownership are exclusively shared process APIs.
#[cfg(all(test, windows))]
mod windows_contracts;

use super::headless_formats::{Event, Parser, parser_for};
use super::headless_progress::{HeadlessProgress, ProgressParser, ProgressState, StrictLineFramer};
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
/// `run` plus the allowlisted progress projection. Terminal output is the same
/// as `run`; the Claude stream is additionally read by a strict, bounded parser
/// whose snapshots never carry provider bodies. `progress` receives a snapshot
/// when it changes, a heartbeat about once a second, and always a final one.
pub async fn run_with_progress(
    spec: Spec,
    run_id: String,
    cancel: &Cancellation,
    emit: impl FnMut(&[u8]),
    progress: impl FnMut(HeadlessProgress),
) -> io::Result<RunResult> {
    run_with_progress_mode(spec, run_id, false, cancel, emit, progress).await
}
/// `strict` is the API key billing output boundary (Issue #14): the run must be
/// Claude stream-json, no raw log is written, no process output is retained,
/// and no provider byte (stdout record, stderr, broken JSON, tool input or
/// result, error body) reaches the terminal. The terminal shows fixed lines and
/// the same masked, bounded last-utterance excerpt the progress card shows.
/// Process failures map to fixed messages.
pub async fn run_with_progress_mode(
    spec: Spec,
    run_id: String,
    strict_output: bool,
    cancel: &Cancellation,
    mut emit: impl FnMut(&[u8]),
    mut progress: impl FnMut(HeadlessProgress),
) -> io::Result<RunResult> {
    use std::time::{Duration, Instant};
    let parser = parser_for(&spec.format).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("headless: no parser for output format {:?}", spec.format),
        )
    })?;
    // Trusted run IDs only (issued by the Hub), never a provider's own ID.
    HeadlessProgress::new(run_id.clone(), 0)
        .validate()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    let structured = parser == Parser::ClaudeStreamJson;
    let mut spec = spec;
    if strict_output {
        if !structured {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "API key billing requires structured output",
            ));
        }
        spec.raw_logs = None;
        spec.process.output_cap = 0;
    }
    // The strict terminal view: fixed text plus the masked progress excerpt.
    let mut shown_excerpt = String::new();
    let mut strict_view = |snapshot: &HeadlessProgress, emit: &mut dyn FnMut(&[u8])| {
        if strict_output
            && !snapshot.last_assistant.is_empty()
            && snapshot.last_assistant != shown_excerpt
        {
            shown_excerpt.clone_from(&snapshot.last_assistant);
            emit(format!("[assistant] {}\r\n", snapshot.last_assistant).as_bytes());
        }
    };
    if strict_output {
        emit(b"many-ai-cli: API key billing run; provider output is shown only as progress\r\n");
    }
    let mut plan = spec.process;
    plan.stdin = if normalize_headless_prompt_via(&spec.prompt_via) == "stdin" {
        spec.prompt.into_bytes()
    } else {
        vec![]
    };
    let now_ms = || chrono::Utc::now().timestamp_millis();
    // The screen decides "measured" vs "unknown" from this flag, so it follows
    // the same format test as the parser (not the provider name).
    let new_tracker = || {
        let mut tracker = ProgressParser::new(run_id.clone(), now_ms());
        tracker.set_structured(structured);
        tracker
    };
    let mut tracker = new_tracker();
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
    let (mut process, mut receiver) = ManagedProcess::spawn_owned(plan, spec.event_capacity.max(1));
    let mut strict = StrictLineFramer::default();
    let mut stdout = LineFramer::default();
    let mut stderr = LineFramer::default();
    let mut started: Option<Instant> = None;
    let elapsed = |started: Option<Instant>| {
        started.map_or(0, |at| {
            u64::try_from(at.elapsed().as_millis()).unwrap_or(u64::MAX)
        })
    };
    let mut dropped = 0u64;
    let mut canceled = false;
    let mut lost = false;
    let mut ticker = tokio::time::interval(Duration::from_millis(250));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_heartbeat = 0;
    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled(), if !canceled => { process.close(); canceled = true; },
            _ = ticker.tick(), if started.is_some() => {
                let now = elapsed(started);
                if now.saturating_sub(last_heartbeat) >= 1000 {
                    tracker.heartbeat();
                    last_heartbeat = now;
                }
                if let Some(snapshot) = tracker.snapshot(now) {
                    strict_view(&snapshot, &mut emit);
                    progress(snapshot);
                }
            },
            event = receiver.recv() => match event {
                Ok(ProcessEvent::Started { .. }) => {
                    // The clock starts with the actual provider process.
                    started = Some(Instant::now());
                    tracker = new_tracker();
                    if let Some(snapshot) = tracker.snapshot(0) { progress(snapshot); }
                },
                Ok(ProcessEvent::Output { stream, bytes }) => {
                    if structured && stream == OutputStream::Stdout && !lost {
                        strict.push(&bytes, |line| match line {
                            Some(line) => tracker.observe(line),
                            None => tracker.mark_incomplete(),
                        });
                    }
                    // API key billing: no provider byte reaches the terminal.
                    if !strict_output {
                        let (framer, raw) = if stream == OutputStream::Stdout { (&mut stdout, &mut raw_out) } else { (&mut stderr, &mut raw_err) };
                        framer.push(&bytes, |line| deliver(line, stream, parser, raw, &mut raw_errors, &mut emit));
                    }
                }
                Err(RecvError::Closed) => break,
                Err(RecvError::Lagged(count)) => {
                    dropped += count;
                    lost = true;
                    // Lost bytes cannot safely be spliced into structured records.
                    stdout.clear(); stderr.clear(); strict.clear(); process.close();
                    tracker.mark_incomplete();
                    let line = format!("many-ai-cli: headless output lost ({count} chunks); stopping run\r\n");
                    emit(line.as_bytes());
                }
            }
        }
    }
    if structured && !lost {
        strict.finish(|line| match line {
            Some(line) => tracker.observe(line),
            None => tracker.mark_incomplete(),
        });
    }
    // Nothing was pushed into these framers in strict mode.
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
            result = process.wait() => break match result {
                Ok(output) => output,
                Err(error) => {
                    let output = ProcessOutput {
                        stdout: vec![],
                        stderr: vec![],
                        stdout_truncated: false,
                        stderr_truncated: false,
                        pipes_forced_closed: false,
                        outcome: ExitOutcome::Exited { code: Some(-1), signal: None },
                    };
                    progress(tracker.finish(&output, dropped, elapsed(started)));
                    return Err(if strict_output {
                        io::Error::new(error.kind(), "headless provider execution failed")
                    } else {
                        error
                    });
                }
            },
            _ = cancel.cancelled(), if !canceled => { process.close(); canceled = true; }
        }
    };
    let final_progress = tracker.finish(&output, dropped, elapsed(started));
    let failed = final_progress.state != ProgressState::Completed;
    strict_view(&final_progress, &mut emit);
    progress(final_progress);
    let mut result = RunResult::from_output(output, dropped, raw_errors);
    // A Claude result that reports an error is a failed run even on exit 0.
    if failed {
        result.state = "error".into();
    }
    Ok(result)
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
        && let Err(error) = file
            .write_all(&crate::storage::mask_secret_bytes(line))
            .and_then(|_| file.write_all(b"\n"))
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

#[cfg(test)]
mod progress_contracts {
    use super::*;
    use crate::orchestration::headless_progress::CostKind;
    use std::{collections::BTreeMap, time::Duration};

    // Synthetic Claude stream records. No provider, credential or user
    // configuration is touched by these child processes.
    const RECORDS: &[&str] = &[
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
    fn fixture(mode: &str, dir: &Path) -> Spec {
        let body = match mode {
            // The final record has no LF.
            "safe" | "bad_exit" | "split_records" | "text" => RECORDS.join("\n"),
            "api_error" => [
                r#"{"type":"assistant","is_api_error_message":true,"error":"authentication_failed","message":{"id":"error","content":[{"type":"text","text":"API Error: PRIVATE_ERROR_SENTINEL"}]}}"#,
                r#"{"type":"result","subtype":"error_during_execution","is_error":true,"total_cost_usd":0.01}"#,
            ]
            .join("\n"),
            "malformed" => "PRIVATE_BAD_JSON_SENTINEL\n".into(),
            "sleep" => String::new(),
            "burst" => "{\"type\":\"system\"}\n".repeat(100_000),
            _ => panic!("unknown synthetic mode"),
        };
        let file = format!("synthetic-progress-{mode}.jsonl");
        std::fs::write(dir.join(&file), body).unwrap();
        let exit = if mode == "bad_exit" { 7 } else { 0 };
        #[cfg(unix)]
        let (executable, args) = (
            "/bin/sh",
            vec![
                "-c".into(),
                if mode == "sleep" {
                    "sleep 2".into()
                } else if mode == "split_records" {
                    format!(
                        "while IFS= read -r line || [ -n \"$line\" ]; do printf '%s\\n' \"$line\"; sleep 0.08; done < {file}"
                    )
                    .into()
                } else {
                    format!("printf '%s\\n' 'PRIVATE_STDERR_SENTINEL' >&2; cat {file}; exit {exit}")
                        .into()
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
                    format!(
                        "Get-Content {file} | ForEach-Object {{ [Console]::Out.WriteLine($_); [Console]::Out.Flush(); Start-Sleep -Milliseconds 80 }}"
                    )
                    .into()
                } else {
                    format!(
                        "[Console]::Error.WriteLine('PRIVATE_STDERR_SENTINEL'); $b=[IO.File]::ReadAllBytes((Join-Path (Get-Location) '{file}')); [Console]::OpenStandardOutput().Write($b,0,$b.Length); exit {exit}"
                    )
                    .into()
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
                timeout: Duration::from_secs(10),
                output_cap: 4096,
                pipe_drain_timeout: Duration::from_millis(100),
            },
            prompt: String::new(),
            prompt_via: "stdin".into(),
            format: if mode == "text" {
                "text"
            } else {
                "claude-stream-json"
            }
            .into(),
            raw_logs: None,
            event_capacity: 1024,
        }
    }
    async fn run_fixture(
        spec: Spec,
        cancel: &Cancellation,
    ) -> (RunResult, Vec<HeadlessProgress>, String) {
        let mut terminal = vec![];
        let mut snapshots = vec![];
        let result = run_with_progress(
            spec,
            "1:2".into(),
            cancel,
            |b| terminal.extend_from_slice(b),
            |p| snapshots.push(p),
        )
        .await
        .unwrap();
        (result, snapshots, String::from_utf8(terminal).unwrap())
    }
    fn assert_wire_clean(snapshots: &[HeadlessProgress]) {
        let wire = serde_json::to_string(snapshots).unwrap();
        for forbidden in ["SENTINEL", "SYNTHETIC_ONLY", "sk-ant-"] {
            assert!(!wire.contains(forbidden), "provider body entered progress");
        }
        for snapshot in snapshots {
            snapshot.validate().unwrap();
        }
        assert!(snapshots.windows(2).all(|s| s[0].sequence < s[1].sequence));
        assert!(
            snapshots
                .windows(2)
                .all(|s| s[0].elapsed_ms <= s[1].elapsed_ms)
        );
    }

    #[tokio::test]
    async fn progress_is_allowlisted_and_terminal_projection_is_unchanged() {
        let root = tempfile::tempdir().unwrap();
        let (result, snapshots, terminal) =
            run_fixture(fixture("safe", root.path()), &Cancellation::default()).await;
        assert_eq!(result.state, "completed");
        assert_wire_clean(&snapshots);
        let last = snapshots.last().unwrap();
        assert_eq!(last.last_assistant, "Ready ***");
        assert_eq!(last.cost.amount_usd, Some(0.123));
        assert_eq!(last.cost.kind, CostKind::Result);
        assert_eq!(last.result_turns, Some(2));
        assert!(last.active_tools.is_empty());
        assert_eq!(last.state, ProgressState::Completed);
        assert!(!last.data_incomplete);
        assert!(snapshots.iter().all(|p| p.structured));
        // The terminal keeps showing the existing formatted projection.
        assert!(terminal.contains("[tool] Read"));
        assert!(terminal.contains("[result]"));
        assert!(!terminal.contains("[progress]"));
    }
    /// Issue #14: an API key billing run shows no provider byte in the terminal.
    #[tokio::test]
    async fn api_key_billing_terminal_shows_only_fixed_lines_and_the_masked_excerpt() {
        let root = tempfile::tempdir().unwrap();
        for mode in ["safe", "api_error", "malformed", "bad_exit"] {
            let mut terminal = vec![];
            let mut snapshots = vec![];
            let mut spec = fixture(mode, root.path());
            spec.raw_logs = Some(RawLogs {
                paths: RuntimePaths::production(root.path()).unwrap(),
                prefix: PathBuf::from("headless").join("synthetic-api"),
            });
            run_with_progress_mode(
                spec,
                "1:2".into(),
                true,
                &Cancellation::default(),
                |b| terminal.extend_from_slice(b),
                |p| snapshots.push(p),
            )
            .await
            .unwrap();
            let terminal = String::from_utf8(terminal).unwrap();
            for forbidden in [
                "SENTINEL",
                "SYNTHETIC_ONLY",
                "sk-ant-",
                "[tool]",
                "\"type\"",
            ] {
                assert!(!terminal.contains(forbidden), "{mode}: {forbidden}");
            }
            assert!(terminal.starts_with("many-ai-cli: API key billing run"));
            assert_wire_clean(&snapshots);
            // No raw provider log is written, even when one was configured.
            fn any_log(dir: &Path) -> bool {
                std::fs::read_dir(dir)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .any(|entry| {
                        let path = entry.path();
                        path.to_string_lossy().contains("synthetic-api")
                            || (path.is_dir() && any_log(&path))
                    })
            }
            assert!(!any_log(root.path()), "{mode}: raw log written");
            if mode == "safe" {
                assert!(terminal.contains("[assistant] Ready ***"));
            }
        }
        // A text-format provider cannot run with API key billing.
        let error = run_with_progress_mode(
            fixture("text", root.path()),
            "1:2".into(),
            true,
            &Cancellation::default(),
            |_| {},
            |_| {},
        )
        .await
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }
    #[tokio::test]
    async fn text_format_reports_lifecycle_without_inventing_structured_values() {
        let root = tempfile::tempdir().unwrap();
        let (result, snapshots, terminal) =
            run_fixture(fixture("text", root.path()), &Cancellation::default()).await;
        assert_eq!(result.state, "completed");
        assert_wire_clean(&snapshots);
        let last = snapshots.last().unwrap();
        assert_eq!(last.state, ProgressState::Completed);
        // The screen shows "unknown" for these, decided by the same format test.
        assert!(snapshots.iter().all(|p| !p.structured));
        assert_eq!(last.observed_turns, 0);
        assert!(last.result_turns.is_none());
        assert!(last.last_assistant.is_empty());
        assert_eq!(last.cost.kind, CostKind::Unavailable);
        assert!(last.final_cost_missing);
        assert!(terminal.contains("\"type\":\"result\""));
    }
    #[tokio::test]
    async fn unreadable_record_is_incomplete_but_nonzero_exit_is_failed() {
        let root = tempfile::tempdir().unwrap();
        let (result, snapshots, _) =
            run_fixture(fixture("malformed", root.path()), &Cancellation::default()).await;
        assert_wire_clean(&snapshots);
        let last = snapshots.last().unwrap();
        assert_eq!(result.state, "completed");
        assert_eq!(last.state, ProgressState::Completed);
        assert!(last.data_incomplete);
        assert!(last.final_cost_missing);

        let (result, snapshots, _) =
            run_fixture(fixture("bad_exit", root.path()), &Cancellation::default()).await;
        assert_wire_clean(&snapshots);
        let last = snapshots.last().unwrap();
        assert_eq!(result.state, "error");
        assert_eq!(last.state, ProgressState::Failed);
        assert_eq!(last.cost.kind, CostKind::Result);
    }
    #[tokio::test]
    async fn silent_run_has_monotonic_heartbeat_and_cancellation_freezes_final() {
        let root = tempfile::tempdir().unwrap();
        let cancel = Cancellation::default();
        let child = cancel.clone();
        // Cancel only after a real running heartbeat, rather than racing a
        // timer started before the owned process has emitted Started.
        let mut snapshots = vec![];
        run_with_progress(
            fixture("sleep", root.path()),
            "1:2".into(),
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
        assert_wire_clean(&snapshots);
        assert!(snapshots.last().unwrap().ended_at_ms.is_some());
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn lag_marks_incomplete_and_stops_owned_provider() {
        let root = tempfile::tempdir().unwrap();
        let mut spec = fixture("burst", root.path());
        spec.event_capacity = 1;
        let mut snapshots = vec![];
        let mut terminal = vec![];
        let result = run_with_progress(
            spec,
            "1:2".into(),
            &Cancellation::default(),
            |b| terminal.extend_from_slice(b),
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
        // The notice is one CRLF-terminated terminal line, as `run` writes it.
        let terminal = String::from_utf8_lossy(&terminal);
        let notice = terminal
            .find("many-ai-cli: headless output lost")
            .expect("lag notice on the terminal");
        let line_end = terminal[notice..].find('\n').unwrap() + notice;
        assert!(terminal[notice..=line_end].ends_with("; stopping run\r\n"));
        assert!(!terminal[line_end + 1..].starts_with("        "));
    }
    #[tokio::test]
    async fn separated_records_and_error_bodies_never_reach_progress() {
        let root = tempfile::tempdir().unwrap();
        let (result, snapshots, _) = run_fixture(
            fixture("split_records", root.path()),
            &Cancellation::default(),
        )
        .await;
        assert_wire_clean(&snapshots);
        assert_eq!(result.state, "completed");
        // While the response is still arriving, no fragment is published.
        assert!(snapshots.iter().any(|p| p.state == ProgressState::Running
            && p.elapsed_ms > 200
            && p.last_assistant.is_empty()));
        assert_eq!(snapshots.last().unwrap().last_assistant, "Ready ***");
        assert!(!snapshots.last().unwrap().data_incomplete);

        let (result, snapshots, _) =
            run_fixture(fixture("api_error", root.path()), &Cancellation::default()).await;
        assert_wire_clean(&snapshots);
        // A provider error result is a failed run even though the CLI exited 0.
        assert_eq!(result.state, "error");
        assert!(snapshots.iter().all(|p| p.last_assistant.is_empty()));
        assert_eq!(snapshots.last().unwrap().state, ProgressState::Failed);
    }
    #[tokio::test]
    async fn untrusted_run_id_is_refused_before_any_process_starts() {
        let root = tempfile::tempdir().unwrap();
        let error = run_with_progress(
            fixture("safe", root.path()),
            "not a run id".into(),
            &Cancellation::default(),
            |_| {},
            |_| panic!("no progress for a refused run"),
        )
        .await
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }
}
