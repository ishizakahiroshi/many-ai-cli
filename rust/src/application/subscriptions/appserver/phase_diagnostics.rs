//! Temporary Windows-test-only observations; remove with the ledger entry.
use crate::process::{ExitOutcome, ProcessOutput};
use serde::Serialize;
use std::{
    io::{self, Read},
    path::Path,
    sync::{Arc, Mutex},
    time::Instant,
};

const EVENT_LIMIT: usize = 64;

tokio::task_local! {
    pub(super) static ACTIVE: Trace;
}

#[derive(Clone)]
pub(super) struct Trace {
    started: Instant,
    data: Arc<Mutex<Native>>,
}

#[derive(Default, Serialize)]
struct Native {
    events: Vec<Event>,
    dropped_events: usize,
    cleanup: Option<Cleanup>,
}

#[derive(Serialize)]
struct Event {
    stage: Stage,
    rpc_id: Option<u8>,
    count: usize,
    elapsed_ms: u128,
}

#[derive(Serialize)]
pub(super) enum Stage {
    CallEntered,
    ProcessStarted,
    WriteRequested,
    WriteAcknowledged,
    NotificationRequested,
    NotificationAcknowledged,
    StdoutBytes,
    ResponseLine,
    InvalidJson,
    UnmatchedId,
    MatchingResponseId,
    CleanupStarted,
}

#[derive(Serialize)]
struct Cleanup {
    elapsed_ms: u128,
    wait_error_kind: Option<String>,
    outcome: Option<&'static str>,
    exit_code: Option<i32>,
    forced_pipes: Option<bool>,
    stdout_bytes: Option<usize>,
    stdout_truncated: Option<bool>,
}

impl Trace {
    pub(super) fn new() -> Self {
        Self {
            started: Instant::now(),
            data: Arc::new(Mutex::new(Native::default())),
        }
    }
}

pub(super) fn event(stage: Stage, id: i64, count: usize) {
    let _ = ACTIVE.try_with(|trace| {
        if let Ok(mut data) = trace.data.lock() {
            if data.events.len() < EVENT_LIMIT {
                data.events.push(Event {
                    stage,
                    rpc_id: match id {
                        0..=2 => Some(id as u8),
                        _ => None,
                    },
                    count,
                    elapsed_ms: trace.started.elapsed().as_millis(),
                });
            } else {
                data.dropped_events = data.dropped_events.saturating_add(1);
            }
        }
    });
}

pub(super) fn cleanup(result: &io::Result<ProcessOutput>) {
    let _ = ACTIVE.try_with(|trace| {
        if let Ok(mut data) = trace.data.lock() {
            let mut observed = Cleanup {
                elapsed_ms: trace.started.elapsed().as_millis(),
                wait_error_kind: None,
                outcome: None,
                exit_code: None,
                forced_pipes: None,
                stdout_bytes: None,
                stdout_truncated: None,
            };
            match result {
                Ok(output) => {
                    observed.outcome = Some(match &output.outcome {
                        ExitOutcome::Exited { code, .. } => {
                            observed.exit_code = *code;
                            "exited"
                        }
                        ExitOutcome::Cancelled => "cancelled",
                        ExitOutcome::TimedOut => "timed_out",
                    });
                    observed.forced_pipes = Some(output.pipes_forced_closed);
                    observed.stdout_bytes = Some(output.stdout.len());
                    observed.stdout_truncated = Some(output.stdout_truncated);
                }
                // ManagedProcess does not retain the original raw OS error.
                Err(error) => observed.wait_error_kind = Some(format!("{:?}", error.kind())),
            }
            data.cleanup = Some(observed);
        }
    });
}

fn methods(requests: Option<&str>) -> serde_json::Value {
    let mut parsed = 0usize;
    let mut invalid = 0usize;
    let mut sequence = Vec::new();
    let mut total = 0usize;
    for line in requests.unwrap_or_default().lines() {
        total = total.saturating_add(1);
        if sequence.len() == EVENT_LIMIT {
            continue;
        }
        let method = match serde_json::from_str::<serde_json::Value>(line) {
            Ok(value) => {
                parsed += 1;
                match value.get("method").and_then(serde_json::Value::as_str) {
                    Some("initialize") => "initialize",
                    Some("initialized") => "initialized",
                    Some("account/read") => "account/read",
                    Some("account/rateLimits/read") => "account/rateLimits/read",
                    _ => "unknown",
                }
            }
            Err(_) => {
                invalid += 1;
                "invalid_json"
            }
        };
        sequence.push(method);
    }
    serde_json::json!({"available":requests.is_some(), "lines":total,
        "parsed_prefix":parsed, "invalid_prefix":invalid,
        "sequence":sequence, "clipped":total > EVENT_LIMIT})
}

fn child(path: &Path) -> serde_json::Value {
    let mut text = String::new();
    let read = std::fs::File::open(path).and_then(|file| file.take(8193).read_to_string(&mut text));
    let mut events = Vec::new();
    let mut malformed = false;
    for line in text.lines().take(EVENT_LIMIT) {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if let [stage, iteration, elapsed] = fields.as_slice()
            && let (Ok(stage), Ok(iteration), Ok(elapsed)) = (
                stage.parse::<u8>(),
                iteration.parse::<u64>(),
                elapsed.parse::<u64>(),
            )
            && stage <= 9
        {
            events.push((stage, iteration, elapsed));
            continue;
        }
        malformed = true;
    }
    serde_json::json!({"at_event_limit":events.len() == EVENT_LIMIT,
        "events":events, "malformed":malformed,
        "clipped":text.len() > 8192 || text.lines().count() > EVENT_LIMIT,
        "read_error_kind":read.err().map(|error| format!("{:?}", error.kind()))})
}

pub(super) fn summary(requests: Option<&str>, path: &Path, trace: &Trace) -> String {
    let native = trace
        .data
        .lock()
        .ok()
        .and_then(|data| serde_json::to_value(&*data).ok());
    // Only fixed labels, bounded counts, elapsed times and error classifications.
    // Never serialize a request, response, path, environment entry or error text.
    format!(
        "windows_appserver_phase_diagnostic {}",
        serde_json::json!({"requests":methods(requests), "child":child(path), "native":native})
    )
}
