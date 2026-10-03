//! Source: internal/headless/{adapter,runner,prompt}.go at 21d0bc7.
//! Process containment and bounded pipe ownership are exclusively shared process APIs.
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
