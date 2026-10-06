//! Temporary cfg(test)-only failure observations for prepare's root resolution.
use crate::process::{ExitOutcome, ProcessOutput};
use std::{
    io,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
pub(super) struct Context {
    pub enabled: bool,
    pub started: Instant,
    pub timeout: Duration,
}
#[derive(Clone, Copy, Debug)]
pub(super) enum IoStage {
    Metadata,
    Policy,
    ProcessIo,
}
#[derive(Clone, Default)]
pub(super) struct Probe {
    // Fixture-local capture makes forwarding testable without global stderr
    // interception, a process-wide hook, or changing the returned error.
    last: Arc<Mutex<Option<String>>>,
}
impl Probe {
    fn emit(&self, context: Context, fields: std::fmt::Arguments<'_>) {
        if !context.enabled {
            return;
        }
        let line = format!(
            "relay_prepare_resolution_diagnostic operation=resolve_root budget_ms={} elapsed_ms={} {fields}\n",
            context.timeout.as_millis(),
            context.started.elapsed().as_millis(),
        );
        if let Ok(mut last) = self.last.lock() {
            *last = Some(line.clone());
        }
        crate::logging::write_diagnostic(&line);
    }
    pub(super) fn boundary(&self, context: Context) {
        self.emit(context, format_args!("stage=Boundary"));
    }
    pub(super) fn io(&self, context: Context, stage: IoStage, error: &io::Error) {
        // Only the typed payload from trial_git's whitelist constructor is
        // printable. Never format an arbitrary io::Error or its inner source.
        if let Some(metadata) = super::super::trial_git::diagnostic_metadata(error) {
            self.emit(context, format_args!("stage={stage:?} {metadata}"));
        } else {
            self.emit(
                context,
                format_args!(
                    "stage={stage:?} kind={:?} os_code={:?}",
                    error.kind(),
                    error.raw_os_error()
                ),
            );
        }
    }
    pub(super) fn output(&self, context: Context, output: &ProcessOutput) {
        let stage =
            if output.stdout_truncated || output.stderr_truncated || output.pipes_forced_closed {
                "Output"
            } else {
                "Exit"
            };
        self.emit(
            context,
            format_args!(
                "stage={stage} outcome={:?} stdout_truncated={} stderr_truncated={} pipes_forced_closed={}",
                output.outcome,
                output.stdout_truncated,
                output.stderr_truncated,
                output.pipes_forced_closed
            ),
        );
    }
    pub(super) fn last(&self) -> Option<String> {
        self.last.lock().ok().and_then(|last| last.clone())
    }
}

#[test]
fn diagnostic_ignores_arbitrary_error_text_and_process_output() {
    let context = Context {
        enabled: true,
        started: Instant::now(),
        timeout: Duration::from_secs(3),
    };
    let probe = Probe::default();
    let secret = "synthetic-private-path-content";
    probe.io(
        context,
        IoStage::Metadata,
        &io::Error::new(io::ErrorKind::PermissionDenied, secret),
    );
    let line = probe.last().unwrap();
    assert!(line.contains("stage=Metadata kind=PermissionDenied os_code=None"));
    assert!(!line.contains(secret));
    for (outcome, stdout_truncated, stderr_truncated, pipes_forced_closed) in [
        (
            ExitOutcome::Exited {
                code: Some(17),
                signal: None,
            },
            false,
            false,
            false,
        ),
        (ExitOutcome::TimedOut, false, false, false),
        (ExitOutcome::Cancelled, false, false, false),
        (
            ExitOutcome::Exited {
                code: Some(0),
                signal: None,
            },
            true,
            false,
            false,
        ),
        (
            ExitOutcome::Exited {
                code: Some(0),
                signal: None,
            },
            false,
            true,
            false,
        ),
        (
            ExitOutcome::Exited {
                code: Some(0),
                signal: None,
            },
            false,
            false,
            true,
        ),
    ] {
        let output = ProcessOutput {
            stdout: secret.as_bytes().to_vec(),
            stderr: secret.as_bytes().to_vec(),
            stdout_truncated,
            stderr_truncated,
            pipes_forced_closed,
            outcome,
        };
        probe.output(context, &output);
        let line = probe.last().unwrap();
        assert!(line.contains(&format!("outcome={:?}", output.outcome)));
        let stage = if stdout_truncated || stderr_truncated || pipes_forced_closed {
            "Output"
        } else {
            "Exit"
        };
        assert!(line.contains(&format!("stage={stage}")));
        assert!(line.contains(&format!("stdout_truncated={stdout_truncated}")));
        assert!(line.contains(&format!("stderr_truncated={stderr_truncated}")));
        assert!(line.contains(&format!("pipes_forced_closed={pipes_forced_closed}")));
        assert!(!line.contains(secret));
    }
}
