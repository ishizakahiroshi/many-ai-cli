//! Source automatic approval runtime, sharing the persisted policy with batch HTTP.
use super::event_observer::EventWarning;
use crate::{
    approval::policy::{Decision, PolicyStore},
    config::{ConfigStore, RuntimePaths},
    hub::task_owner::HubTaskHandle,
    proto::{
        self,
        core::*,
        time::{Timestamp, format_go_rfc3339_layout},
    },
    storage::mask_secrets,
    terminal::session::SessionEngine,
};
use serde::Serialize;
use std::{
    collections::VecDeque,
    io,
    sync::{Arc, Mutex, OnceLock, Weak},
};

#[derive(Clone, Serialize)]
pub struct Candidate {
    pub at: String,
    pub session_id: i64,
    pub provider: String,
    pub cwd: String,
    pub summary: proto::ApprovalSummary,
    pub decision: Decision,
}
#[cfg(test)]
mod tests;
#[derive(Serialize)]
pub struct AuditRecord {
    pub timestamp: String,
    pub session_id: i64,
    pub provider: String,
    pub rule_id: String,
    pub command: String,
    pub risk: String,
}
/// A real rotating writer is required; absence must never discard the audit.
pub type AuditWriter = Arc<dyn Fn(&AuditRecord) -> io::Result<()> + Send + Sync>;
struct Owners {
    core: Weak<SessionEngine>,
    effects: Weak<dyn CoreEffectSink>,
}
pub struct ApprovalRules {
    pub policy: Arc<PolicyStore>,
    history: Mutex<VecDeque<Candidate>>,
    owners: OnceLock<Owners>,
    tasks: HubTaskHandle,
    warning: EventWarning,
    audit: AuditWriter,
}
impl ApprovalRules {
    pub fn new(
        config: Arc<ConfigStore>,
        paths: RuntimePaths,
        tasks: HubTaskHandle,
        warning: EventWarning,
        audit: AuditWriter,
    ) -> Arc<Self> {
        let (policy, error) = PolicyStore::open(
            paths,
            Arc::new(move || {
                config
                    .snapshot()
                    .is_ok_and(|s| s.config.user_prefs.approval.auto_approval_enabled)
            }),
        );
        if error.is_some() {
            warning(
                "automatic approval policy load",
                &SessionError::InvalidRequest(
                    "policy file unavailable; automatic authorization disabled".into(),
                ),
            );
        }
        Arc::new(Self {
            policy,
            history: Mutex::new(VecDeque::new()),
            owners: OnceLock::new(),
            tasks,
            warning,
            audit,
        })
    }
    pub fn bind(
        &self,
        core: Weak<SessionEngine>,
        effects: Weak<dyn CoreEffectSink>,
    ) -> Result<(), SessionError> {
        self.owners.set(Owners { core, effects }).map_err(|_| {
            SessionError::InvalidRequest("automatic approval owners already bound".into())
        })
    }
    pub fn history(&self) -> Vec<Candidate> {
        self.history
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .cloned()
            .collect()
    }
    pub fn try_apply<'a>(
        &'a self,
        session: LiveSessionId,
        record: &'a ImmutableApprovalRecord,
    ) -> CoreFuture<'a, Result<bool, SessionError>> {
        Box::pin(async move {
            let data = record.data();
            if data.origin != "native" || data.options.is_empty() {
                return Ok(false);
            }
            let owners = self.owners.get().ok_or(SessionError::Shutdown)?;
            let core = owners.core.upgrade().ok_or(SessionError::Shutdown)?;
            let effects = owners.effects.upgrade().ok_or(SessionError::Shutdown)?;
            let Some(details) = core.details(session) else {
                return Ok(false);
            };
            let decision = self.policy.evaluate_policy(
                &data.summary.command,
                &details.snapshot.cwd,
                &data.summary.risk,
            );
            {
                let mut history = self.history.lock().unwrap_or_else(|p| p.into_inner());
                history.push_back(Candidate {
                    at: format_go_rfc3339_layout(
                        Timestamp::now(),
                        chrono::Local::now().offset().local_minus_utc(),
                        true,
                    )
                    .map_err(|_| SessionError::InvalidRequest("invalid audit time".into()))?,
                    session_id: session.0,
                    provider: details.snapshot.provider.clone(),
                    cwd: details.snapshot.cwd.clone(),
                    summary: data.summary.clone(),
                    decision: decision.clone(),
                });
                while history.len() > 100 {
                    history.pop_front();
                }
            }
            if !decision.allowed || !self.policy.enabled() {
                return Ok(false);
            }
            // Reserve retained effect ownership before any irreversible send.
            let audit_timestamp = format_go_rfc3339_layout(
                Timestamp::now(),
                chrono::Local::now().offset().local_minus_utc(),
                false,
            )
            .map_err(|_| SessionError::InvalidRequest("invalid audit time".into()))?;
            let permit = self.tasks.effect_permit()?;
            let action = match core
                .prepare_and_send(
                    NativeActionRequest {
                        binding: ApprovalActionBinding {
                            session: details.binding,
                            candidate_key: data.candidate.key.clone(),
                            source_epoch: data.candidate.source_epoch,
                            sig: data.sig.clone(),
                        },
                        selection: NativeActionSelection::ApproveOnce,
                        origin: NativeActionOrigin::Automatic {
                            rule_id: decision.rule_id.clone(),
                        },
                    },
                    &permit.cancellation(),
                )
                .await
            {
                Ok(action) => action,
                Err(_) => {
                    (self.warning)(
                        "automatic approval skipped",
                        &SessionError::InvalidRequest(
                            "candidate, input or live policy changed".into(),
                        ),
                    );
                    return Ok(false);
                }
            };
            let audit = self.audit.clone();
            let warning = self.warning.clone();
            let audit_record = AuditRecord {
                timestamp: audit_timestamp,
                session_id: session.0,
                provider: details.snapshot.provider.clone(),
                rule_id: decision.rule_id.clone(),
                command: mask_secrets(&data.summary.command),
                risk: data.summary.risk.clone(),
            };
            let notification = proto::Message {
                r#type: "auto_approval_applied".into(),
                session_id: session.0,
                provider: details.snapshot.provider,
                approval_sig: data.sig.clone(),
                approval_summary: Some(data.summary.clone()),
                text: decision.rule_id,
                ..Default::default()
            };
            // Commit is synchronous; neither HTTP cancellation nor observer drop
            // can leave accepted input without owned closure effects.
            let release = ReservedApprovalAction {
                reservation: action.reservation,
                binding: action.binding.clone(),
                selected_text: action.selected_text.clone(),
                origin: action.origin.clone(),
            };
            let actions = match core.commit(action, Timestamp::now()) {
                Ok(actions) => actions,
                Err(_) => {
                    core.release(release);
                    (self.warning)(
                        "automatic approval skipped",
                        &SessionError::InvalidRequest("approval changed after send".into()),
                    );
                    return Ok(false);
                }
            };
            let waiter = permit.start(async move {
                if let Err(failure) = effects.apply(actions).await {
                    warning("automatic approval closure", &failure.error);
                }
                if audit(&audit_record).is_err() {
                    warning(
                        "automatic approval audit write",
                        &SessionError::InvalidRequest("audit write unavailable".into()),
                    );
                }
                if let Err(failure) = effects.apply(core.broadcast_ui(notification)).await {
                    warning("automatic approval broadcast", &failure.error);
                }
            });
            // The enclosing detection batch still owns earlier persistence
            // tickets. Waiting here would prevent that batch from dropping its
            // suppressed detection ticket and deadlock the committed closure.
            // Hub shutdown drains the retained task, independently of this
            // observer's completion or cancellation.
            drop(waiter);
            Ok(true)
        })
    }
}
