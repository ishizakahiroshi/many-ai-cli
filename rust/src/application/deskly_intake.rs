//! Opt-in Deskly stage-4 PC-relay intake for a read-only PR session.
//!
//! Not composed into `serve` yet: nothing here runs unless a caller builds an
//! `IntakeCoordinator`. Ownership stays where it already is:
//! - Deskly owns request, policy, revision and the one-time claim/handoff
//!   authority (existing `relay_claim` / `relay_handoff` / `validate_issued`).
//! - `SessionEngine` (through `OrdinarySpawnService`) owns session admission;
//!   launch requires the owner's server-verified confirmation request.
//! - `journal` only remembers what this Hub already did, so an unknown reply
//!   or a restart holds the tuple instead of acting twice.
//!
//! Discovery is advisory. The approved repository / PR / immutable HEAD and
//! read-only scope come from a separate `TargetSource`; Deskly has no
//! contract for that binding yet, so the production source refuses.
pub mod github;
pub mod journal;
pub mod launch;
pub mod mcp;
#[cfg(test)]
mod tests;
pub mod wire;

use crate::proto::core::VerifiedConfirmationRequest;
use github::{PrHeadReader, RepoName};
use journal::{Journal, Record, State};
use launch::{LaunchError, ReadOnlyLaunch, ReadOnlyLauncher, ReadOnlyProvider};
use mcp::{DesklyRelay, Step};
use std::{collections::BTreeMap, path::PathBuf};
use wire::{Candidate, HandoffApplied};

/// Default discovery interval. The scheduler itself is not composed yet.
pub const DEFAULT_POLL_SECONDS: u64 = 300;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    ReadOnly,
}
impl Scope {
    /// The capability set must be exactly `["read"]`; any extra or unknown
    /// capability is a refusal, not a downgrade.
    pub fn from_capabilities(capabilities: &[&str]) -> Option<Self> {
        (capabilities == ["read"]).then_some(Self::ReadOnly)
    }
}
/// An authenticated, approved binding of one Deskly request to one PR HEAD.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApprovedTarget {
    pub request_id: String,
    pub expected_revision: u64,
    pub authorization_epoch: u64,
    pub repo: RepoName,
    pub pull_request: u64,
    pub head_sha: String,
    pub scope: Scope,
    pub expires_at_ms: i64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TargetResolution {
    Bound(ApprovedTarget),
    Unavailable(&'static str),
}
#[allow(async_fn_in_trait)]
pub trait TargetSource {
    async fn resolve(&self, candidate: &Candidate) -> TargetResolution;
}
/// Production source until Deskly defines an authenticated PR/HEAD/scope
/// binding for stage-4 requests. Never infers one from discovery or wire text.
pub struct NoTargetContract;
impl TargetSource for NoTargetContract {
    async fn resolve(&self, _: &Candidate) -> TargetResolution {
        TargetResolution::Unavailable("deskly_target_binding_contract_missing")
    }
}

/// Owner-local policy. Repositories absent here are never read.
pub struct LocalPolicy {
    pub checkouts: BTreeMap<RepoName, PathBuf>,
    pub provider: ReadOnlyProvider,
}

/// What the owner approves: the exact tuple shown, bound to the Hub-verified
/// confirmation request (constructed only by the server after auth/PIN).
pub struct OwnerApproval<'a> {
    pub request_id: &'a str,
    pub generation: u64,
    pub head_sha: &'a str,
    pub verified: &'a VerifiedConfirmationRequest,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PollReport {
    pub pending_owner: Vec<Record>,
    pub already_known: usize,
    pub unbound: usize,
    pub rejected: Vec<(String, &'static str)>,
    pub discovery_error: Option<String>,
    pub truncated: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApproveOutcome {
    Launched {
        client_request_id: String,
    },
    /// Deskly or the local check refused before anything was consumed.
    Refused(String),
    /// May have been applied somewhere; held for manual reconciliation.
    Held(String),
    /// Nothing changed (wrong tuple, stale Hub authentication, not pending).
    NotApplied(&'static str),
}

pub struct IntakeCoordinator<D, T, H> {
    pub deskly: D,
    pub targets: T,
    pub heads: H,
    pub journal: Journal,
    pub policy: LocalPolicy,
    pub clock: fn() -> i64,
}
impl<D: DesklyRelay, T: TargetSource, H: PrHeadReader> IntakeCoordinator<D, T, H> {
    /// One discovery read. Records new candidates as owner-pending only after
    /// the binding, local policy and live PR HEAD agree. Sends nothing.
    pub async fn poll_once(&self) -> PollReport {
        let mut report = PollReport::default();
        let discovery = match self.deskly.discover().await {
            Ok(discovery) => discovery,
            Err(error) => {
                report.discovery_error = Some(error);
                return report;
            }
        };
        // No cursor exists; a partial snapshot is not acted on.
        if discovery.truncated {
            report.truncated = true;
            return report;
        }
        for candidate in discovery.work {
            match self.journal.get(&candidate.request_id, 1) {
                Ok(Some(_)) => {
                    report.already_known += 1;
                    continue;
                }
                Ok(None) => {}
                Err(_) => {
                    report.rejected.push((candidate.request_id, "journal"));
                    continue;
                }
            }
            let target = match self.targets.resolve(&candidate).await {
                TargetResolution::Bound(target) => target,
                TargetResolution::Unavailable(_) => {
                    report.unbound += 1;
                    continue;
                }
            };
            if let Err(reason) = self.check_target(&candidate, &target).await {
                report.rejected.push((candidate.request_id, reason));
                continue;
            }
            let Ok(attempt_id) = journal::new_attempt_id() else {
                report.rejected.push((candidate.request_id, "attempt id"));
                continue;
            };
            let record = Record {
                request_id: candidate.request_id.clone(),
                generation: 1,
                attempt_id,
                state: State::OwnerPending,
                reason: String::new(),
                revision: candidate.expected_revision,
                authorization_epoch: candidate.authorization_epoch,
                session_id: candidate.session_id.clone(),
                repo: target.repo.as_str().to_owned(),
                pull_request: target.pull_request,
                head_sha: target.head_sha.clone(),
                client_request_id: String::new(),
            };
            match self.journal.reserve(&record, (self.clock)()) {
                Ok(true) => report.pending_owner.push(record),
                Ok(false) => report.already_known += 1,
                Err(_) => report.rejected.push((candidate.request_id, "journal")),
            }
        }
        report
    }

    async fn check_target(
        &self,
        candidate: &Candidate,
        target: &ApprovedTarget,
    ) -> Result<(), &'static str> {
        if target.request_id != candidate.request_id
            || target.expected_revision != candidate.expected_revision
            || target.authorization_epoch != candidate.authorization_epoch
        {
            return Err("binding does not match discovery");
        }
        if target.scope != Scope::ReadOnly {
            return Err("scope");
        }
        if target.expires_at_ms <= (self.clock)() {
            return Err("binding expired");
        }
        if !wire::valid_sha1(&target.head_sha) || target.pull_request == 0 {
            return Err("binding shape");
        }
        if !self.policy.checkouts.contains_key(&target.repo) {
            return Err("repository not allowed locally");
        }
        let head = self
            .heads
            .head(&target.repo, target.pull_request)
            .await
            .map_err(|_| "pr head unavailable")?;
        if !head.open || head.base_repo != target.repo.as_str() {
            return Err("pr not open in repository");
        }
        if head.sha != target.head_sha {
            return Err("pr head moved");
        }
        Ok(())
    }

    pub fn decline(&self, request_id: &str, generation: u64) -> bool {
        self.journal
            .transition(
                request_id,
                generation,
                State::OwnerPending,
                State::OwnerDeclined,
                "owner_declined",
                (self.clock)(),
            )
            .unwrap_or(false)
    }

    fn mark(&self, record: &Record, from: State, to: State, reason: &str) -> bool {
        self.journal
            .transition(
                &record.request_id,
                record.generation,
                from,
                to,
                reason,
                (self.clock)(),
            )
            .unwrap_or(false)
    }
    fn held(&self, record: &Record, from: State, reason: &str) -> ApproveOutcome {
        self.mark(record, from, State::Held, reason);
        ApproveOutcome::Held(reason.to_owned())
    }
    fn refused(&self, record: &Record, from: State, reason: &str) -> ApproveOutcome {
        self.mark(record, from, State::Refused, reason);
        ApproveOutcome::Refused(reason.to_owned())
    }

    /// Owner said yes. Re-checks everything at the boundary, then runs the
    /// existing Deskly one-time authority and the Hub launch. Each mutating
    /// request is preceded by a durable intent row; any unknown result holds.
    pub async fn approve<L: ReadOnlyLauncher>(
        &self,
        approval: OwnerApproval<'_>,
        launcher: &L,
    ) -> ApproveOutcome {
        let Ok(Some(record)) = self.journal.get(approval.request_id, approval.generation) else {
            return ApproveOutcome::NotApplied("unknown request");
        };
        if record.state != State::OwnerPending {
            return ApproveOutcome::NotApplied("not pending");
        }
        if record.head_sha != approval.head_sha {
            return ApproveOutcome::NotApplied("approved tuple differs");
        }
        if approval.verified.auth_epoch() != launcher.auth_epoch() {
            return ApproveOutcome::NotApplied("hub authentication expired");
        }
        let Ok(repo) = RepoName::parse(&record.repo) else {
            return ApproveOutcome::NotApplied("journal repo");
        };
        let Some(cwd) = self.policy.checkouts.get(&repo).cloned() else {
            return ApproveOutcome::NotApplied("repository not allowed locally");
        };
        let candidate = Candidate {
            request_id: record.request_id.clone(),
            expected_revision: record.revision,
            authorization_epoch: record.authorization_epoch,
            session_id: record.session_id.clone(),
        };
        // Re-resolve and re-read HEAD: the PR may have moved while waiting.
        let fresh = match self.targets.resolve(&candidate).await {
            TargetResolution::Bound(target) => target,
            TargetResolution::Unavailable(reason) => {
                return self.refused(&record, State::OwnerPending, reason);
            }
        };
        if fresh.repo != repo
            || fresh.pull_request != record.pull_request
            || fresh.head_sha != record.head_sha
        {
            return self.refused(&record, State::OwnerPending, "binding changed");
        }
        if let Err(reason) = self.check_target(&candidate, &fresh).await {
            return self.refused(&record, State::OwnerPending, reason);
        }
        if !self.mark(&record, State::OwnerPending, State::Approved, "") {
            return ApproveOutcome::NotApplied("not pending");
        }

        // Claim: preview is read-only on Deskly; apply consumes authority.
        let preview = match self
            .deskly
            .claim_preview(&record.request_id, record.revision, &record.session_id)
            .await
        {
            Step::Done(preview) => preview,
            Step::Refused(code) => return self.refused(&record, State::Approved, &code),
            Step::Unknown(reason) => return self.held(&record, State::Approved, reason),
        };
        if !self.mark(&record, State::Approved, State::ClaimSent, "") {
            return self.held(&record, State::Approved, "journal");
        }
        let claimed = match self
            .deskly
            .claim_apply(
                &record.request_id,
                record.revision,
                &record.session_id,
                &preview.token,
            )
            .await
        {
            Step::Done(receipt) => receipt,
            Step::Refused(code) => return self.refused(&record, State::ClaimSent, &code),
            Step::Unknown(reason) => {
                return self.held(
                    &record,
                    State::ClaimSent,
                    &format!("claim_unknown:{reason}"),
                );
            }
        };
        if !self.mark(&record, State::ClaimSent, State::Claimed, "") {
            return self.held(&record, State::ClaimSent, "journal");
        }

        let handoff_preview = match self
            .deskly
            .handoff_preview(&record.request_id, claimed.revision, &record.attempt_id)
            .await
        {
            Step::Done(preview) => preview,
            // Claimed but not handed off: authority state is Deskly's to resolve.
            Step::Refused(code) => {
                return self.held(&record, State::Claimed, &format!("handoff_preview:{code}"));
            }
            Step::Unknown(reason) => return self.held(&record, State::Claimed, reason),
        };
        let session_ok = handoff_preview
            .confirmation
            .as_ref()
            .is_some_and(|c| c.session_id == record.session_id);
        if !session_ok {
            return self.held(&record, State::Claimed, "handoff session mismatch");
        }
        if !self.mark(&record, State::Claimed, State::HandoffSent, "") {
            return self.held(&record, State::Claimed, "journal");
        }
        let issued = match self
            .deskly
            .handoff_apply(
                &record.request_id,
                claimed.revision,
                &record.attempt_id,
                &handoff_preview.token,
            )
            .await
        {
            Step::Done(HandoffApplied::Issued(issued)) => issued,
            Step::Done(HandoffApplied::Spent(_)) => {
                return self.held(&record, State::HandoffSent, "handoff_spent");
            }
            Step::Refused(code) => {
                return self.held(
                    &record,
                    State::HandoffSent,
                    &format!("handoff_refused:{code}"),
                );
            }
            Step::Unknown(reason) => {
                return self.held(
                    &record,
                    State::HandoffSent,
                    &format!("handoff_unknown:{reason}"),
                );
            }
        };
        if issued.session_id != record.session_id
            || issued.authorization_epoch != record.authorization_epoch
            || issued.expires_at_ms <= (self.clock)()
        {
            return self.held(&record, State::HandoffSent, "issued tuple mismatch");
        }
        if !self.mark(&record, State::HandoffSent, State::Issued, "") {
            return self.held(&record, State::HandoffSent, "journal");
        }
        match self
            .deskly
            .validate_issued(issued.receipt.revision, &issued)
            .await
        {
            Step::Done(valid_until) if valid_until > (self.clock)() => {}
            Step::Done(_) => return self.held(&record, State::Issued, "issued validation expired"),
            Step::Refused(code) => {
                return self.held(&record, State::Issued, &format!("validate_refused:{code}"));
            }
            Step::Unknown(reason) => return self.held(&record, State::Issued, reason),
        }

        let client_request_id = format!("deskly-{}", record.attempt_id);
        let request = ReadOnlyLaunch {
            client_request_id: client_request_id.clone(),
            provider: self.policy.provider,
            cwd,
            repo,
            pull_request: record.pull_request,
            head_sha: record.head_sha.clone(),
        };
        if !self.mark(&record, State::Issued, State::LaunchSent, "") {
            return self.held(&record, State::Issued, "journal");
        }
        match launcher.launch(&request, approval.verified).await {
            Ok(()) => {
                self.journal
                    .launched(
                        &record.request_id,
                        record.generation,
                        &client_request_id,
                        (self.clock)(),
                    )
                    .ok();
                ApproveOutcome::Launched { client_request_id }
            }
            Err(LaunchError::AuthenticationExpired) => {
                self.held(&record, State::LaunchSent, "launch_auth_expired")
            }
            Err(LaunchError::Unknown(code)) => self.held(
                &record,
                State::LaunchSent,
                &format!("launch_unknown:{code}"),
            ),
        }
    }
}
