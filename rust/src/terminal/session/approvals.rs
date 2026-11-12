use super::*;
use crate::approval::{
    detect, identity, selection,
    shell_authority::{self, CommandAuthority},
};
impl Session {
    /// Applies the last transcript read to a freshly detected native prompt
    /// (audit U23). Pure: the transcript was read outside the lock.
    pub(super) fn authorize_native(
        &self,
        native: detect::NativeApproval,
    ) -> (detect::NativeApproval, CommandAuthority) {
        let (summary, authority) = shell_authority::authorize(
            native.summary.clone(),
            &native.question,
            &native.context,
            self.pending_shell.as_deref(),
        );
        (detect::NativeApproval { summary, ..native }, authority)
    }
    pub(super) fn approval_action_matches(&self, binding: &ApprovalActionBinding) -> bool {
        self.connected
            && self.binding == binding.session
            && self.approval.record().is_some_and(|record| {
                let r = record.data();
                r.origin == "native"
                    && r.sig == binding.sig
                    && r.candidate.key == binding.candidate_key
                    && r.candidate.source_epoch == binding.source_epoch
            })
    }
}
struct ActionGuard<'a> {
    engine: &'a SessionEngine,
    binding: ApprovalActionBinding,
    reservation: ApprovalReservationId,
    armed: bool,
}
impl Drop for ActionGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            let mut state = lock(&self.engine.state);
            if let Some(session) = state.sessions.get_mut(&self.binding.session.session)
                && session.approval_reservation.as_ref()
                    == Some(&(self.reservation, self.binding.clone()))
            {
                session.approval_reservation = None;
            }
        }
    }
}
impl ApprovalActions for SessionEngine {
    fn pending_native_approval_actions(&self) -> Vec<PendingNativeApprovalAction> {
        let state = lock(&self.state);
        state
            .sessions
            .values()
            .filter_map(|session| {
                let record = session.approval.record()?.data();
                if record.origin != "native" || record.options.is_empty() {
                    return None;
                }
                let phrases = self.approval_phrases_for(&session.snapshot.provider);
                let (live, _) = session.authorize_native(detect::detect_native(
                    &session.snapshot.provider,
                    &session.vt.lines(),
                    &phrases,
                )?);
                if live.sig != record.sig {
                    return None;
                }
                let candidate = identity::candidate(
                    &session.snapshot.provider,
                    &live.kind,
                    &live.question,
                    &live.context,
                    &live.options,
                    session.approval.epoch(),
                );
                // Match counts in the Go batch endpoint include a pending record
                // even when its wrapper is absent. Delivery still validates the
                // retained exact binding after FIFO acquisition.
                Some(PendingNativeApprovalAction {
                    binding: ApprovalActionBinding {
                        session: session.binding,
                        candidate_key: candidate.key,
                        source_epoch: candidate.source_epoch,
                        sig: live.sig,
                    },
                    provider: session.snapshot.provider.clone(),
                    cwd: session.snapshot.cwd.clone(),
                    summary: live.summary,
                    connected: session.connected,
                })
            })
            .collect()
    }
    fn prepare_and_send<'a>(
        &'a self,
        request: NativeActionRequest,
        cancel: &'a TaskCancellation,
    ) -> CoreFuture<'a, Result<ReservedApprovalAction, ApprovalActionError>> {
        let ticket = self.input_ticket(request.binding.session);
        Box::pin(async move {
            let ticket = ticket.map_err(|_| ApprovalActionError::StaleWrapper)?;
            if !ticket.wait(cancel).await {
                return Err(ApprovalActionError::Cancelled);
            }
            let (reservation, summary, cwd, input, selected_text) = {
                let mut state = lock(&self.state);
                let reservation = ApprovalReservationId(
                    state
                        .next_reservation
                        .checked_add(1)
                        .ok_or(ApprovalActionError::Reserved)?,
                );
                state.next_reservation = reservation.0;
                let s = state
                    .sessions
                    .get_mut(&request.binding.session.session)
                    .ok_or(ApprovalActionError::NotFound)?;
                if s.binding != request.binding.session || !s.connected {
                    return Err(ApprovalActionError::StaleWrapper);
                }
                if s.approval_reservation.is_some() {
                    return Err(ApprovalActionError::Reserved);
                }
                if !s.approval_action_matches(&request.binding) {
                    return Err(ApprovalActionError::StaleCandidate);
                }
                // A record can outlive a repaint. Re-detect under the input lane
                // immediately before reserving; never send against stale UI text.
                let phrases = self.approval_phrases_for(&s.snapshot.provider);
                let (live, authority) = s.authorize_native(
                    detect::detect_native(&s.snapshot.provider, &s.vt.lines(), &phrases)
                        .ok_or(ApprovalActionError::StaleCandidate)?,
                );
                let candidate = identity::candidate(
                    &s.snapshot.provider,
                    &live.kind,
                    &live.question,
                    &live.context,
                    &live.options,
                    s.approval.epoch(),
                );
                if live.sig != request.binding.sig
                    || candidate.key != request.binding.candidate_key
                    || candidate.source_epoch != request.binding.source_epoch
                {
                    return Err(ApprovalActionError::StaleCandidate);
                }
                let (input, selected_text) = match &request.selection {
                    NativeActionSelection::Exact {
                        selected_text,
                        send_text,
                    } => {
                        // Only an explicit human action may supply keystrokes.
                        // One-shot callers carry intent, never a snapshot's Enter.
                        if !matches!(request.origin, NativeActionOrigin::User) {
                            return Err(ApprovalActionError::PersistentOption);
                        }
                        if send_text.is_empty()
                            || !live
                                .options
                                .iter()
                                .any(|option| selection::raw_option_input(option) == *send_text)
                        {
                            return Err(ApprovalActionError::StaleCandidate);
                        }
                        (send_text.clone(), selected_text.clone())
                    }
                    NativeActionSelection::ApproveOnce => {
                        if live.summary.risk == "high" {
                            return Err(ApprovalActionError::HighRisk);
                        }
                        // Audit U23: rules, batches and one-tap links never approve
                        // a heading prompt whose whole command is not confirmed.
                        if authority == CommandAuthority::Unverified
                            && !matches!(request.origin, NativeActionOrigin::User)
                        {
                            return Err(ApprovalActionError::UnverifiedCommand);
                        }
                        // Audit N08: a rule or a batch acts only on a command the
                        // transcript confirmed, whatever risk the summary carries.
                        if matches!(
                            request.origin,
                            NativeActionOrigin::Batch | NativeActionOrigin::Automatic { .. }
                        ) && (live.summary.risk != "low"
                            || authority != CommandAuthority::Verified)
                        {
                            return Err(ApprovalActionError::StaleCandidate);
                        }
                        let input = selection::approve_once_input(&live.options);
                        (input.clone(), input)
                    }
                    NativeActionSelection::RejectOnce => {
                        let input = selection::reject_once_input(&live.options);
                        (input.clone(), input)
                    }
                };
                if input.is_empty() {
                    return Err(ApprovalActionError::PersistentOption);
                }
                s.approval_reservation = Some((reservation, request.binding.clone()));
                (
                    reservation,
                    live.summary,
                    s.snapshot.cwd.clone(),
                    input,
                    selected_text,
                )
            };
            let mut guard = ActionGuard {
                engine: self,
                binding: request.binding.clone(),
                reservation,
                armed: true,
            };
            let action = ReservedApprovalAction {
                reservation,
                binding: request.binding.clone(),
                selected_text,
                origin: request.origin.clone(),
            };
            if let NativeActionOrigin::Automatic { rule_id } = &request.origin
                && !self
                    .options
                    .approval_policy
                    .as_ref()
                    .is_some_and(|p| p(rule_id, &cwd, &summary))
            {
                self.release(action);
                return Err(ApprovalActionError::PolicyChanged);
            }
            let result = self
                .send_combined(
                    request.binding.session,
                    input.into_bytes(),
                    &InputAuthority::NativeApproval(request.binding),
                    cancel,
                )
                .await;
            if !result.remainder.is_empty() || result.error.is_some() {
                self.release(action);
                return Err(match result.error {
                    Some(SessionError::Cancelled) => ApprovalActionError::Cancelled,
                    Some(SessionError::StaleBinding) => ApprovalActionError::StaleWrapper,
                    Some(error) => ApprovalActionError::Transport(format!("{error:?}")),
                    None => ApprovalActionError::Transport(
                        "approval input was not completely written".into(),
                    ),
                });
            }
            guard.armed = false;
            Ok(action)
        })
    }
    fn commit(
        &self,
        action: ReservedApprovalAction,
        now: Timestamp,
    ) -> Result<CoreEffects, ApprovalActionError> {
        let mut state = lock(&self.state);
        let s = state
            .sessions
            .get_mut(&action.binding.session.session)
            .ok_or(ApprovalActionError::NotFound)?;
        if s.approval_reservation.as_ref() != Some(&(action.reservation, action.binding.clone())) {
            return Err(ApprovalActionError::Reserved);
        }
        if !s.approval_action_matches(&action.binding) {
            s.approval_reservation = None;
            return Err(ApprovalActionError::StaleCandidate);
        }
        let mut effects = s
            .approval
            .consume(&action.binding, &action.selected_text, now)?;
        s.approval_reservation = None;
        s.refresh_awaiting();
        if !terminal(&s.snapshot.state) {
            let previous = s.snapshot.state.clone();
            s.snapshot.state = s.snapshot.activity.display_state().into();
            let entered = s.restart_stall_clock(&previous, now);
            effects.0.push(CoreEffect::Broadcast(
                s.with_stall_clock(s.update_message(), entered),
            ));
        }
        if matches!(action.origin, NativeActionOrigin::Automatic { .. }) {
            // Priming broadcasts were queued before the ordered application
            // observer could make its automatic decision. Remove only this
            // candidate's unflushed open; retain all earlier ledger closures
            // and every unrelated session/event in the priming queue.
            for ui in state.uis.values_mut() {
                ui.queued.retain(|frame| {
                    keep_queued_after_automatic_approval(
                        frame,
                        action.binding.session.session.0,
                        &action.binding.candidate_key,
                        action.binding.source_epoch.0,
                    )
                });
            }
        }
        Ok(state.route(effects))
    }
    fn release(&self, action: ReservedApprovalAction) {
        let mut state = lock(&self.state);
        if let Some(s) = state.sessions.get_mut(&action.binding.session.session)
            && s.approval_reservation.as_ref() == Some(&(action.reservation, action.binding))
        {
            s.approval_reservation = None;
        }
    }
    fn refresh_pending_shell<'a>(&'a self, session: LiveSessionId) -> CoreFuture<'a, ()> {
        Box::pin(self.reload_pending_shell(session))
    }
}

fn keep_queued_after_automatic_approval(
    frame: &UiFrame,
    session: i64,
    candidate: &str,
    epoch: u64,
) -> bool {
    match frame {
        UiFrame::Message(message) => {
            !(message.session_id == session
                && message
                    .approval_state
                    .as_ref()
                    .and_then(|state| state.open.as_ref())
                    .is_some_and(|open| {
                        open.candidate_key == candidate && open.source_epoch == epoch
                    }))
        }
        UiFrame::GitTurn(_) | UiFrame::SpawnCorrelation(_) => true,
    }
}
impl SessionEngine {
    /// Re-reads the unanswered shell calls of a Claude session's transcript
    /// (audit U23). The read runs through `EngineOptions::pending_shell` with the
    /// state lock released; only the result is stored.
    async fn reload_pending_shell(&self, session: LiveSessionId) {
        let Some(lookup) = self.options.pending_shell.clone() else {
            return;
        };
        let (binding, identity) = {
            let state = lock(&self.state);
            match state.sessions.get(&session) {
                Some(s) if s.snapshot.provider == "claude" => (s.binding, s.transcript.clone()),
                _ => return,
            }
        };
        let pending = lookup(identity).await;
        let mut state = lock(&self.state);
        if let Some(s) = state.sessions.get_mut(&session)
            && s.binding == binding
        {
            s.pending_shell = pending.map(Arc::from);
        }
    }
    /// Authority of a recorded prompt against the last transcript read. Pass the
    /// terminal-text summary (`detect::native_summary`), not a stored one.
    pub fn authorize_approval(
        &self,
        session: LiveSessionId,
        question: &str,
        context: &str,
    ) -> (proto::ApprovalSummary, CommandAuthority) {
        let pending = lock(&self.state)
            .sessions
            .get(&session)
            .and_then(|s| s.pending_shell.clone());
        shell_authority::authorize(
            detect::native_summary(question, context),
            question,
            context,
            pending.as_deref(),
        )
    }
}
#[cfg(test)]
mod spawn_correlation_tests {
    use super::*;
    #[test]
    fn spawn_correlation_survives_automatic_approval_priming_cleanup() {
        let mut queued = vec![
            UiFrame::SpawnCorrelation(SpawnCorrelationNotification {
                session_id: 7,
                client_request_id: "request-1".into(),
                ..Default::default()
            }),
            UiFrame::Message(Box::new(proto::Message {
                r#type: "session_update".into(),
                session_id: 7,
                ..Default::default()
            })),
        ];
        queued.retain(|frame| keep_queued_after_automatic_approval(frame, 7, "candidate", 1));
        assert_eq!(queued.len(), 2);
        assert!(matches!(queued[0], UiFrame::SpawnCorrelation(_)));
    }
}
