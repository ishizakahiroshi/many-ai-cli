use super::plan::UpdatePlan;
use crate::{
    process::{ExitOutcome, ProcessOutput, SpawnOptions, run_capped_with_options},
    proto::core::{ProviderAdmissionError, ProviderUpdateAdmission, ProviderUpdateLease},
};
use std::{io, sync::Arc};

/// Holds the same core admission owner used by spawn. Cancellation, IO failure,
/// and task cancellation all release the exact lease through Drop.
struct UpdateLease {
    owner: Arc<dyn ProviderUpdateAdmission>,
    lease: Option<ProviderUpdateLease>,
}
impl Drop for UpdateLease {
    fn drop(&mut self) {
        if let Some(lease) = self.lease.take() {
            self.owner.end_provider_update(lease);
        }
    }
}
#[derive(Debug)]
pub enum UpdateError {
    Admission(ProviderAdmissionError),
    Io(io::Error),
}
pub struct UpdateExecution {
    pub argv: Vec<String>,
    pub log: Vec<u8>,
    pub output: ProcessOutput,
}
pub async fn execute(
    plan: UpdatePlan,
    admission: Arc<dyn ProviderUpdateAdmission>,
) -> Result<UpdateExecution, UpdateError> {
    let lease = admission
        .begin_provider_update(&plan.command.provider)
        .map_err(UpdateError::Admission)?;
    let _lease = UpdateLease {
        owner: admission,
        lease: Some(lease),
    };
    let argv = plan.argv();
    let mut log = plan.log_header().into_bytes();
    let output = run_capped_with_options(
        &plan.command.process,
        &plan.command.cancellation,
        SpawnOptions {
            combined_output: true,
            no_window: true,
            ..Default::default()
        },
    )
    .await
    .map_err(UpdateError::Io)?;
    // Process capture is individually bounded; keep the Go combined log budget.
    let cap = 1024 * 1024;
    let header_len = log.len();
    log.extend_from_slice(&output.stdout[..output.stdout.len().min(cap)]);
    let remaining = cap.saturating_sub(log.len() - header_len);
    log.extend_from_slice(&output.stderr[..output.stderr.len().min(remaining)]);
    Ok(UpdateExecution { argv, log, output })
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Updated,
    Latest,
    Unknown,
    Failed,
    FileInUse,
    LoginRequired,
}
impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Updated => "updated",
            Self::Latest => "latest",
            Self::Unknown => "unknown",
            Self::Failed => "failed",
            Self::FileInUse => "file_in_use",
            Self::LoginRequired => "login_required",
        }
    }
}
fn login_required(output: &str) -> bool {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(||regex::Regex::new(r"(?i)(\[unauthenticated\]|\bunauthenticated\b|\bnot\s+authenticated\b|\bauthentication\s+(?:required|failed)\b|\blog[- ]?in\s+(?:required|needed)\b|\bsign[- ]?in\s+(?:required|needed)\b)").expect("frozen update outcome pattern")).is_match(output)
}
pub fn classify(
    output: &str,
    exit: &ExitOutcome,
    start_failed: bool,
    before: Option<&str>,
    after: Option<&str>,
) -> Outcome {
    if start_failed || matches!(exit, ExitOutcome::TimedOut | ExitOutcome::Cancelled) {
        return Outcome::Failed;
    }
    if login_required(output) {
        return Outcome::LoginRequired;
    }
    if !matches!(
        exit,
        ExitOutcome::Exited {
            code: Some(0),
            signal: None
        }
    ) {
        if [
            "EBUSY",
            "EPERM",
            "ETXTBSY",
            "Text file busy",
            "resource busy",
            "being used by another process",
            "Access is denied",
            "アクセスが拒否",
            "別のプロセスが使用中",
        ]
        .iter()
        .any(|m| output.contains(m))
        {
            return Outcome::FileInUse;
        }
        return Outcome::Failed;
    }
    match (before, after) {
        (Some(a), Some(b)) if a.trim() == b.trim() => Outcome::Latest,
        (Some(_), Some(_)) => Outcome::Updated,
        _ => Outcome::Unknown,
    }
}
