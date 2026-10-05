//! Pending confirmation and accepted-task ownership share SessionEngine's
//! admission transaction. The injected executor owns actual preparation/I/O.
use super::*;
use crate::{config, orchestration::child_options};
use tokio::sync::Notify;

/// Already authenticated and validated request with the existing reservation.
/// Production executors retain a Weak<SessionEngine>, never another session map.
pub struct ConfirmedChildRequest {
    /// Original pending options remain available for source change-note evidence.
    pub original_body: ResolvedChildSpawn,
    pub parent: SessionBinding,
    pub requested_provider: String,
    pub body: ResolvedChildSpawn,
    pub admission: AdmissionId,
}
pub struct ConfirmationPresentation {
    pub config: config::Config,
    pub trust_grant_providers: Vec<String>,
}

/// No default implementation: unavailable preparation/board ownership cannot
/// turn into a successful placeholder launch or an incomplete disclosure.
pub trait ConfirmationExecutor: Send + Sync {
    fn presentation(&self) -> Result<ConfirmationPresentation, SessionError>;
    fn spawn<'a>(
        &'a self,
        request: ConfirmedChildRequest,
        cancel: TaskCancellation,
    ) -> CoreFuture<'a, Result<ChildSpawnResult, SessionError>>;
    fn record_refusal<'a>(
        &'a self,
        pending: &'a PendingSpawnConfirmation,
        cancel: TaskCancellation,
    ) -> CoreFuture<'a, Result<(), SessionError>>;
    fn notify_waiter_gone<'a>(
        &'a self,
        pending: &'a PendingSpawnConfirmation,
        result: &'a Result<ChildSpawnResult, SessionError>,
        cancel: TaskCancellation,
    ) -> CoreFuture<'a, Result<(), SessionError>>;
}

#[derive(Default)]
pub(super) struct Completion {
    state: Mutex<CompletionState>,
    changed: Notify,
}
#[derive(Default)]
struct CompletionState {
    outcome: Option<ConfirmationWaitOutcome>,
    handed_off: bool,
    waiter_gone: bool,
}
impl Completion {
    fn finish(&self, outcome: ConfirmationWaitOutcome) {
        let mut state = lock(&self.state);
        if state.outcome.is_none() {
            state.outcome = Some(outcome);
            drop(state);
            self.changed.notify_waiters();
        }
    }
    fn waiter_gone(&self) -> bool {
        lock(&self.state).waiter_gone
    }
    fn drop_waiter(&self) {
        let mut state = lock(&self.state);
        if !state.handed_off && state.outcome.is_none() {
            state.waiter_gone = true;
        }
    }
}
struct Waiter {
    id: SpawnConfirmationId,
    completion: Arc<Completion>,
}
impl Drop for Waiter {
    fn drop(&mut self) {
        self.completion.drop_waiter();
    }
}
impl ConfirmationWaiter for Waiter {
    fn id(&self) -> &SpawnConfirmationId {
        &self.id
    }
    fn wait(
        self: Box<Self>,
        cancel: HttpWaitCancellation,
    ) -> CoreFuture<'static, ConfirmationWaitOutcome> {
        Box::pin(async move {
            loop {
                let changed = self.completion.changed.notified();
                tokio::pin!(changed);
                // Register before reading the result: notify_waiters carries no
                // permit for a waiter created after its completion transition.
                changed.as_mut().enable();
                if let Some(outcome) = lock(&self.completion.state).outcome.clone() {
                    return outcome;
                }
                tokio::select! {
                    _ = changed => {},
                    _ = cancel.token().cancelled() => {
                        return ConfirmationWaitOutcome::WaiterCancelled;
                    }
                }
            }
        })
    }
}

impl SpawnConfirmations for SessionEngine {
    fn pending(&self) -> Vec<PendingSpawnConfirmation> {
        let state = lock(&self.state);
        let mut pending: Vec<_> = state
            .confirmations
            .values()
            .map(|entry| {
                let mut pending = entry.pending.clone();
                pending.waiter_gone = entry.completion.waiter_gone();
                pending
            })
            .collect();
        pending.sort_by_key(|pending| pending.requested_at);
        pending
    }
    fn register(
        &self,
        request: ConfirmationRequest,
    ) -> Result<ConfirmationRegistration, AdmissionError> {
        if lock(&self.state).confirmations_stopped {
            return Err(AdmissionError::Unavailable);
        }
        // Live disclosure callbacks must run before State is locked and before
        // reserving capacity. Never publish a dialog with fallback permissions.
        let disclosure = self.confirmation_disclosure().map_err(|error| {
            self.warn("confirmation disclosure unavailable", &error);
            AdmissionError::Unavailable
        })?;
        let mut message = requested_message(
            request.parent,
            &request.body,
            request.requested_at,
            &disclosure,
        );
        let mut state = lock(&self.state);
        if state.confirmations_stopped {
            return Err(AdmissionError::Unavailable);
        }
        if !state.sessions.contains_key(&request.parent) {
            return Err(AdmissionError::ParentNotFound);
        }
        let sequence = state
            .next_confirmation
            .checked_add(1)
            .ok_or(AdmissionError::IdentityExhausted)?;
        let previous = state
            .confirmations
            .values()
            .find(|entry| {
                entry.pending.parent == request.parent
                    && entry.pending.body.request().role == request.body.request().role
            })
            .map(|entry| (entry.pending.id.clone(), entry.pending.admission.clone()));
        let reservation = state.admission.reserve_confirmation(
            request.parent,
            1,
            previous.as_ref().map(|(_, admission)| admission.clone()),
        )?;
        state.next_confirmation = sequence;
        let id = SpawnConfirmationId(format!(
            "sc-{}-{sequence}",
            request.requested_at.unix_nanos()
        ));
        let completion = Arc::new(Completion::default());
        let pending = PendingSpawnConfirmation {
            id: id.clone(),
            parent: request.parent,
            requested_provider: request.requested_provider,
            body: request.body,
            requested_at: request.requested_at,
            admission: reservation.id,
            waiter_gone: false,
        };
        let mut effects = CoreEffects::default();
        if let Some((previous, _)) = previous {
            let old = state
                .confirmations
                .remove(&previous)
                .expect("matched entry");
            old.completion.finish(ConfirmationWaitOutcome::Decided(
                ConfirmationOutcome::Superseded,
            ));
            effects.0.push(CoreEffect::Broadcast(closed_message(
                &old.pending,
                &ConfirmationOutcome::Superseded,
            )));
        }
        message.spawn_confirmation_id.clone_from(&id.0);
        effects.0.push(CoreEffect::Broadcast(message));
        state.confirmations.insert(
            id.clone(),
            ConfirmationEntry {
                pending: pending.clone(),
                completion: completion.clone(),
                decision: None,
            },
        );
        Ok(ConfirmationRegistration {
            pending,
            effects: state.route(effects),
            waiter: Box::new(Waiter { id, completion }),
        })
    }
    fn accept_decision(
        self: Arc<Self>,
        response: SpawnConfirmationResponse,
        proof: VerifiedConfirmationRequest,
        _now: Timestamp,
    ) -> Result<Box<dyn AcceptedSpawnDecision>, SessionError> {
        if response.confirmation_id.0.trim().is_empty() {
            return Err(SessionError::InvalidRequest(
                "confirmation_id is required".into(),
            ));
        }
        let executor = self.confirmation_executor()?;
        let (body, completion) = {
            let state = lock(&self.state);
            if state.confirmations_stopped {
                return Err(SessionError::Shutdown);
            }
            if state.auth_epoch != proof.auth_epoch() {
                return Err(SessionError::AuthenticationExpired);
            }
            let entry = state
                .confirmations
                .get(&response.confirmation_id)
                .ok_or_else(no_longer_pending)?;
            if entry.decision.is_some() {
                return Err(already_decided());
            }
            (entry.pending.body.clone(), entry.completion.clone())
        };
        // Snapshot/validation can call application code. Revalidate the exact
        // entry after returning so replacement or auth rotation cannot be lost.
        executor.presentation()?;
        let body = if response.approved {
            let mut body = body.approve(&response, proof.clone())?;
            validate_launch_options(&mut body)?;
            Some(body)
        } else {
            None
        };
        let mut state = lock(&self.state);
        if state.confirmations_stopped {
            return Err(SessionError::Shutdown);
        }
        if state.auth_epoch != proof.auth_epoch() {
            return Err(SessionError::AuthenticationExpired);
        }
        let entry = state
            .confirmations
            .get(&response.confirmation_id)
            .filter(|entry| Arc::ptr_eq(&entry.completion, &completion))
            .ok_or_else(no_longer_pending)?;
        if entry.decision.is_some() {
            return Err(already_decided());
        }
        let lease = state
            .next_confirmation_decision
            .checked_add(1)
            .ok_or_else(|| {
                SessionError::InvalidRequest("confirmation decision identity exhausted".into())
            })?;
        state.next_confirmation_decision = lease;
        state
            .confirmations
            .get_mut(&response.confirmation_id)
            .expect("validated entry")
            .decision = Some(lease);
        drop(state);
        Ok(Box::new(Decision {
            engine: self,
            id: response.confirmation_id,
            lease,
            proof,
            body,
            executor,
        }))
    }
    fn parent_ended(&self, parent: LiveSessionId) -> CoreEffects {
        let mut state = lock(&self.state);
        let ids: Vec<_> = state
            .confirmations
            .values()
            .filter(|entry| entry.pending.parent == parent)
            .map(|entry| entry.pending.id.clone())
            .collect();
        let mut effects = CoreEffects::default();
        for id in ids {
            let entry = state.confirmations.remove(&id).expect("collected entry");
            state.admission.release_children(&entry.pending.admission);
            entry.completion.finish(ConfirmationWaitOutcome::Decided(
                ConfirmationOutcome::ParentEnded,
            ));
            effects.0.push(CoreEffect::Broadcast(closed_message(
                &entry.pending,
                &ConfirmationOutcome::ParentEnded,
            )));
        }
        state.route(effects)
    }
}

impl SessionEngine {
    /// The Hub first stops accepting operations, then calls this hook and
    /// cancels/drains committed tasks through its separate task owner. Pending
    /// waiters stop and reservations release without a fabricated wire reason.
    pub fn shutdown_confirmations(&self) {
        let mut state = lock(&self.state);
        state.confirmations_stopped = true;
        for (_, entry) in std::mem::take(&mut state.confirmations) {
            state.admission.release_children(&entry.pending.admission);
            entry.completion.finish(ConfirmationWaitOutcome::HubStopped);
        }
    }
    fn confirmation_executor(&self) -> Result<Arc<dyn ConfirmationExecutor>, SessionError> {
        self.options
            .confirmation_executor
            .clone()
            .ok_or(SessionError::Shutdown)
    }
    pub(super) fn confirmation_disclosure(&self) -> Result<ConfirmationPresentation, SessionError> {
        self.confirmation_executor()?.presentation()
    }
}
fn validate_launch_options(body: &mut ResolvedChildSpawn) -> Result<(), SessionError> {
    let body = body.request_mut();
    body.effort = body.effort.trim().to_owned();
    body.execution_mode = config::normalize_execution_mode(&body.execution_mode);
    body.permission_preset = config::normalize_permission_preset(&body.permission_preset);
    config::validate_effort(&body.provider, &body.effort)
        .and_then(|()| config::validate_execution_mode(&body.execution_mode))
        .and_then(|()| config::validate_permission_preset(&body.permission_preset))
        .map_err(|error| SessionError::InvalidRequest(error.to_string()))
}
fn requested_message(
    parent: LiveSessionId,
    resolved: &ResolvedChildSpawn,
    requested_at: Timestamp,
    disclosure: &ConfirmationPresentation,
) -> proto::Message {
    let body = resolved.request();
    proto::Message {
        r#type: "spawn_confirmation_requested".into(),
        session_id: parent.0,
        role: body.role.clone(),
        provider: body.provider.clone(),
        model: body.model.clone(),
        effort: body.effort.clone(),
        execution_mode: body.execution_mode.clone(),
        permission_preset: body.permission_preset.clone(),
        remember_permission: disclosure
            .config
            .user_prefs
            .spawn
            .role_permission
            .get(&body.role)
            .is_some_and(|value| !value.is_empty()),
        cwd: body.cwd.clone(),
        initial_prompt: body.initial_prompt.clone(),
        // Go's UnixMilli uses floor division for pre-epoch timestamps.
        spawn_requested_at_ms: requested_at.unix_nanos().div_euclid(1_000_000) as i64,
        spawn_child_approval: child_options::preview_tiers(resolved, &disclosure.config)
            .into_iter()
            .map(|(provider, tiers)| (provider, Some(tiers)))
            .collect(),
        trust_grant_providers: disclosure.trust_grant_providers.clone(),
        ..Default::default()
    }
}
impl State {
    pub(super) fn confirmation_frames(
        &self,
        disclosure: &ConfirmationPresentation,
    ) -> Vec<proto::Message> {
        let mut pending: Vec<_> = self
            .confirmations
            .values()
            .map(|entry| &entry.pending)
            .collect();
        pending.sort_by_key(|pending| pending.requested_at);
        pending
            .into_iter()
            .map(|pending| {
                let mut message = requested_message(
                    pending.parent,
                    &pending.body,
                    pending.requested_at,
                    disclosure,
                );
                message.spawn_confirmation_id.clone_from(&pending.id.0);
                message
            })
            .collect()
    }
}
fn closed_message(
    pending: &PendingSpawnConfirmation,
    outcome: &ConfirmationOutcome,
) -> proto::Message {
    let (reason, child, text) = match outcome {
        ConfirmationOutcome::Approved(child) => ("approved", child.id.0, String::new()),
        ConfirmationOutcome::Refused => ("refused", 0, String::new()),
        ConfirmationOutcome::Superseded => ("superseded", 0, String::new()),
        ConfirmationOutcome::ParentEnded => ("parent_gone", 0, String::new()),
        ConfirmationOutcome::SpawnFailed(error) => (
            "spawn_failed",
            0,
            match error {
                SessionError::ChildLaunch { detail, .. }
                | SessionError::Transport(detail)
                | SessionError::InvalidRequest(detail) => detail.clone(),
                _ => format!("{error:?}"),
            },
        ),
    };
    proto::Message {
        r#type: "spawn_confirmation_closed".into(),
        spawn_confirmation_id: pending.id.0.clone(),
        session_id: pending.parent.0,
        reason: reason.into(),
        spawn_child_session_id: child,
        text,
        ..Default::default()
    }
}
fn no_longer_pending() -> SessionError {
    SessionError::ConfirmationMissing
}
fn already_decided() -> SessionError {
    SessionError::ConfirmationDecided
}

struct Decision {
    engine: Arc<SessionEngine>,
    id: SpawnConfirmationId,
    lease: u64,
    proof: VerifiedConfirmationRequest,
    body: Option<ResolvedChildSpawn>,
    executor: Arc<dyn ConfirmationExecutor>,
}
impl Drop for Decision {
    fn drop(&mut self) {
        let mut state = lock(&self.engine.state);
        if let Some(entry) = state.confirmations.get_mut(&self.id)
            && entry.decision == Some(self.lease)
        {
            entry.decision = None;
        }
    }
}
impl AcceptedSpawnDecision for Decision {
    fn id(&self) -> &SpawnConfirmationId {
        &self.id
    }
    fn run(
        mut self: Box<Self>,
        cancel: TaskCancellation,
    ) -> Result<CoreFuture<'static, ConfirmationOutcome>, SessionError> {
        // This transaction executes at construction, not on first future poll.
        let mut state = lock(&self.engine.state);
        if state.confirmations_stopped {
            return Err(SessionError::Shutdown);
        }
        if state.auth_epoch != self.proof.auth_epoch() {
            return Err(SessionError::AuthenticationExpired);
        }
        let entry = state
            .confirmations
            .get(&self.id)
            .ok_or_else(no_longer_pending)?;
        if entry.decision != Some(self.lease) {
            return Err(already_decided());
        }
        // Go confirmations belong to the retained parent ID, not its old socket
        // or incarnation. Resolve the current wrapper only at actual handoff.
        let parent = state
            .sessions
            .get(&entry.pending.parent)
            .ok_or(SessionError::NotFound(entry.pending.parent))?
            .binding;
        let mut entry = state
            .confirmations
            .remove(&self.id)
            .expect("validated entry");
        {
            let mut completion = lock(&entry.completion.state);
            completion.handed_off = true;
            entry.pending.waiter_gone = completion.waiter_gone;
        }
        if self.body.is_none() {
            state.admission.release_children(&entry.pending.admission);
        }
        drop(state);
        let task = CommittedDecision {
            engine: self.engine.clone(),
            executor: self.executor.clone(),
            pending: entry.pending,
            completion: entry.completion,
            parent,
            body: self.body.take(),
            complete: false,
        };
        // Construct the guard above the async block so unpolled drop settles it.
        Ok(Box::pin(async move { task.execute(cancel).await }))
    }
}
struct CommittedDecision {
    engine: Arc<SessionEngine>,
    executor: Arc<dyn ConfirmationExecutor>,
    pending: PendingSpawnConfirmation,
    completion: Arc<Completion>,
    parent: SessionBinding,
    body: Option<ResolvedChildSpawn>,
    complete: bool,
}
impl Drop for CommittedDecision {
    fn drop(&mut self) {
        lock(&self.engine.state)
            .admission
            .release_children(&self.pending.admission);
        if !self.complete {
            self.completion.finish(ConfirmationWaitOutcome::Decided(
                ConfirmationOutcome::SpawnFailed(SessionError::Transport(
                    "accepted confirmation task was dropped".into(),
                )),
            ));
            self.engine.warn(
                "confirmation task cleanup incomplete",
                &SessionError::Cancelled,
            );
        }
    }
}
impl CommittedDecision {
    async fn execute(mut self, cancel: TaskCancellation) -> ConfirmationOutcome {
        let mut spawn_result = None;
        let outcome = if cancel.token().is_cancelled() {
            ConfirmationOutcome::SpawnFailed(SessionError::Transport(
                "confirmation task cancelled".into(),
            ))
        } else if let Some(body) = self.body.take() {
            let result = self
                .executor
                .spawn(
                    ConfirmedChildRequest {
                        original_body: self.pending.body.clone(),
                        parent: self.parent,
                        requested_provider: self.pending.requested_provider.clone(),
                        body,
                        admission: self.pending.admission.clone(),
                    },
                    cancel.clone(),
                )
                .await;
            let outcome = match &result {
                Ok(child) => ConfirmationOutcome::Approved(child.clone()),
                Err(error) => ConfirmationOutcome::SpawnFailed(error.clone()),
            };
            spawn_result = Some(result);
            outcome
        } else {
            // Source refusal board writes are best effort, but attempted once
            // by the real owner before publishing refusal to the waiter.
            if let Err(error) = self
                .executor
                .record_refusal(&self.pending, cancel.clone())
                .await
            {
                self.engine.warn("record confirmation refusal", &error);
            }
            ConfirmationOutcome::Refused
        };
        self.completion
            .finish(ConfirmationWaitOutcome::Decided(outcome.clone()));
        let effects = self
            .engine
            .broadcast_ui(closed_message(&self.pending, &outcome));
        if let Err(error) = self.engine.apply_effects(effects).await {
            self.engine.warn("publish confirmation close", &error);
        }
        if self.pending.waiter_gone
            && let Some(result) = spawn_result
            && let Err(error) = self
                .executor
                .notify_waiter_gone(&self.pending, &result, cancel)
                .await
        {
            self.engine.warn("notify confirmation waiter gone", &error);
        }
        self.complete = true;
        outcome
    }
}

#[cfg(test)]
mod tests;
