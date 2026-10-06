//! Temporary test-only, fixture-owned failure metadata. No captured text is retained.
use super::FilesService;
use crate::{
    hub::http::Response,
    process::{ExitOutcome, ProcessOutput},
};
use std::{io, time::Duration};

const LIMIT: usize = 8;

#[derive(Clone, Copy, Debug)]
pub(crate) enum Operation {
    Resolve,
    HeadTree,
    ReadHead,
    ReadEmpty,
    StageWorktree,
    WriteTree,
    Other,
}
impl Operation {
    fn from_args(args: &[&str]) -> Self {
        match args {
            ["rev-parse", "--show-toplevel"] => Self::Resolve,
            ["rev-parse", "--verify", "HEAD^{tree}"] => Self::HeadTree,
            ["read-tree", "HEAD"] => Self::ReadHead,
            ["read-tree", "--empty"] => Self::ReadEmpty,
            ["add", "-A", "--", "."] => Self::StageWorktree,
            ["write-tree"] => Self::WriteTree,
            _ => Self::Other,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Phase {
    OpenRuntime,
    OpenTemporaryParent,
    RandomTemporaryName,
    OpenTemporaryIndex,
    IndexSpelling,
    RemoveTemporaryIndex,
}

#[derive(Debug)]
enum Code {
    SnapshotFailed,
    GitCommandFailed,
    SnapshotCleanupFailed,
    NotFound,
    Conflict,
    NotGitRepo,
    BadSession,
    NoCwd,
    StaleBinding,
    Other,
}

// These fields are deliberately read only by the bounded Debug assertion sink.
#[allow(dead_code)]
#[derive(Debug)]
enum Failure {
    BudgetExhausted {
        operation: Operation,
        elapsed_ms: u128,
    },
    ProcessIo {
        operation: Operation,
        kind: io::ErrorKind,
        os_code: Option<i32>,
        budget_ms: u128,
        elapsed_ms: u128,
    },
    Process {
        operation: Operation,
        outcome: ExitOutcome,
        stdout_bytes: usize,
        stderr_bytes: usize,
        stdout_truncated: bool,
        stderr_truncated: bool,
        pipes_forced_closed: bool,
        budget_ms: u128,
        elapsed_ms: u128,
    },
    Filesystem {
        phase: Phase,
        kind: io::ErrorKind,
        os_code: Option<i32>,
    },
    InvalidTreeId,
}

#[derive(Debug, Default)]
pub(crate) struct Snapshot {
    failures: Vec<Failure>,
    dropped: usize,
    response: Option<(u16, Code)>,
}

#[derive(Default)]
pub(crate) struct Recorder {
    active: bool,
    snapshot: Snapshot,
}
impl Recorder {
    pub(crate) fn begin(&mut self) {
        self.snapshot = Snapshot::default();
        self.active = true;
    }
    pub(crate) fn finish(&mut self) -> Snapshot {
        self.active = false;
        std::mem::take(&mut self.snapshot)
    }
    fn record(&mut self, failure: Failure) {
        if !self.active {
            return;
        }
        if self.snapshot.failures.len() < LIMIT {
            self.snapshot.failures.push(failure);
        } else {
            self.snapshot.dropped = self.snapshot.dropped.saturating_add(1);
        }
    }
    fn response(&mut self, response: &Response) {
        if !self.active {
            return;
        }
        let value = (response.body.len() <= 4096)
            .then(|| serde_json::from_slice::<serde_json::Value>(&response.body).ok())
            .flatten();
        let code = match value.as_ref().and_then(|v| v["error"].as_str()) {
            Some("snapshot_failed") => Code::SnapshotFailed,
            Some("git_command_failed") => Code::GitCommandFailed,
            Some("snapshot_cleanup_failed") => Code::SnapshotCleanupFailed,
            Some("not_found") => Code::NotFound,
            Some("conflict") => Code::Conflict,
            Some("not_git_repo") => Code::NotGitRepo,
            Some("bad_session") => Code::BadSession,
            Some("no_cwd") => Code::NoCwd,
            Some("stale_binding") => Code::StaleBinding,
            _ => Code::Other,
        };
        self.snapshot.response = Some((response.status, code));
    }
}

fn record(service: &FilesService, failure: Failure) {
    service
        .snapshot_diagnostic
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .record(failure);
}

pub(crate) fn budget(service: &FilesService, args: &[&str], elapsed: Duration) {
    record(
        service,
        Failure::BudgetExhausted {
            operation: Operation::from_args(args),
            elapsed_ms: elapsed.as_millis(),
        },
    );
}
pub(crate) fn process_io(
    service: &FilesService,
    args: &[&str],
    error: &io::Error,
    budget: Duration,
    elapsed: Duration,
) {
    record(
        service,
        Failure::ProcessIo {
            operation: Operation::from_args(args),
            kind: error.kind(),
            os_code: error.raw_os_error(),
            budget_ms: budget.as_millis(),
            elapsed_ms: elapsed.as_millis(),
        },
    );
}
pub(crate) fn process(
    service: &FilesService,
    args: &[&str],
    output: &ProcessOutput,
    budget: Duration,
    elapsed: Duration,
) {
    record(
        service,
        Failure::Process {
            operation: Operation::from_args(args),
            outcome: output.outcome.clone(),
            stdout_bytes: output.stdout.len(),
            stderr_bytes: output.stderr.len(),
            stdout_truncated: output.stdout_truncated,
            stderr_truncated: output.stderr_truncated,
            pipes_forced_closed: output.pipes_forced_closed,
            budget_ms: budget.as_millis(),
            elapsed_ms: elapsed.as_millis(),
        },
    );
}
pub(crate) fn filesystem(service: &FilesService, phase: Phase, error: &io::Error) {
    record(
        service,
        Failure::Filesystem {
            phase,
            kind: error.kind(),
            os_code: error.raw_os_error(),
        },
    );
}
pub(crate) fn invalid_tree(service: &FilesService) {
    record(service, Failure::InvalidTreeId);
}
pub(crate) fn response(service: &FilesService, response: &Response) {
    service
        .snapshot_diagnostic
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .response(response);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorder_is_inactive_until_scoped_and_bounds_fixed_failure_fields() {
        let mut recorder = Recorder::default();
        recorder.record(Failure::InvalidTreeId);
        assert!(recorder.finish().failures.is_empty());
        recorder.begin();
        for _ in 0..LIMIT + 3 {
            recorder.record(Failure::InvalidTreeId);
        }
        let snapshot = recorder.finish();
        assert_eq!(snapshot.failures.len(), LIMIT);
        assert_eq!(snapshot.dropped, 3);
        recorder.record(Failure::InvalidTreeId);
        assert!(recorder.finish().failures.is_empty());
    }

    #[test]
    fn response_and_argument_identity_never_copy_unknown_text() {
        let mut recorder = Recorder::default();
        recorder.begin();
        recorder.response(&Response::error(
            500,
            "snapshot_cleanup_failed",
            "private-token /private/path C:\\private\\path",
        ));
        let snapshot = recorder.finish();
        assert!(matches!(
            snapshot.response,
            Some((500, Code::SnapshotCleanupFailed))
        ));
        let rendered = format!("{snapshot:?}");
        assert!(!rendered.contains("private"));
        assert!(matches!(
            Operation::from_args(&["private-token"]),
            Operation::Other
        ));
        recorder.begin();
        recorder.response(&Response::error(500, "private-token", "private-detail"));
        let snapshot = recorder.finish();
        assert!(matches!(snapshot.response, Some((500, Code::Other))));
        assert!(!format!("{snapshot:?}").contains("private"));
    }

    #[test]
    fn failed_process_and_cleanup_keep_typed_identity_without_captured_bytes() {
        let root = tempfile::tempdir().unwrap();
        let runtime = root.path().join("runtime");
        std::fs::create_dir(&runtime).unwrap();
        let paths =
            crate::config::RuntimePaths::trial(&runtime, 49439, &root.path().join("installed"))
                .unwrap();
        let service = FilesService::new(root.path().into(), paths);
        service.snapshot_diagnostic.lock().unwrap().begin();
        let output = ProcessOutput {
            stdout: b"private-stdout /private/path".to_vec(),
            stderr: b"private-stderr C:\\private\\path".to_vec(),
            stdout_truncated: true,
            stderr_truncated: false,
            pipes_forced_closed: true,
            outcome: ExitOutcome::TimedOut,
        };
        process(
            &service,
            &["write-tree"],
            &output,
            Duration::from_millis(700),
            Duration::from_millis(950),
        );
        let original = io::Error::from_raw_os_error(5);
        filesystem(&service, Phase::RemoveTemporaryIndex, &original);
        assert_eq!(original.raw_os_error(), Some(5));
        let private = io::Error::new(
            io::ErrorKind::PermissionDenied,
            "private-token /private/path",
        );
        process_io(
            &service,
            &["private-argument"],
            &private,
            Duration::from_millis(600),
            Duration::from_millis(12),
        );
        response(
            &service,
            &Response::error(500, "snapshot_cleanup_failed", "private-detail"),
        );
        let snapshot = service.snapshot_diagnostic.lock().unwrap().finish();
        assert_eq!(snapshot.failures.len(), 3);
        assert!(matches!(
            snapshot.failures[0],
            Failure::Process {
                operation: Operation::WriteTree,
                outcome: ExitOutcome::TimedOut,
                stdout_truncated: true,
                pipes_forced_closed: true,
                budget_ms: 700,
                elapsed_ms: 950,
                ..
            }
        ));
        assert!(matches!(
            snapshot.failures[1],
            Failure::Filesystem {
                phase: Phase::RemoveTemporaryIndex,
                os_code: Some(5),
                ..
            }
        ));
        assert!(matches!(
            snapshot.failures[2],
            Failure::ProcessIo {
                operation: Operation::Other,
                kind: io::ErrorKind::PermissionDenied,
                os_code: None,
                ..
            }
        ));
        assert!(matches!(
            snapshot.response,
            Some((500, Code::SnapshotCleanupFailed))
        ));
        assert!(!format!("{snapshot:?}").contains("private"));
        assert_eq!(private.kind(), io::ErrorKind::PermissionDenied);
    }
}
