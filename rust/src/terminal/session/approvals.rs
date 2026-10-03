use super::*;
use crate::approval::{detect, identity};
impl Session {
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
            let (reservation, summary, cwd, input) = {
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
                let phrases = self
                    .options
                    .approval_phrases
                    .get(&s.snapshot.provider)
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                let live = detect::detect_native(&s.snapshot.provider, &s.vt.lines(), phrases)
                    .ok_or(ApprovalActionError::StaleCandidate)?;
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
                let option = live
                    .options
                    .iter()
                    .find(|o| option_input(o) == request.send_text && !request.send_text.is_empty())
                    .ok_or(ApprovalActionError::StaleCandidate)?;
                let negative = negative_label(&option.label);
                let automatic = matches!(request.origin, NativeActionOrigin::Automatic { .. });
                if !matches!(request.origin, NativeActionOrigin::User) {
                    if !negative && !positive_label(&option.label) {
                        return Err(ApprovalActionError::PersistentOption);
                    }
                    if !negative && live.summary.risk == "high" {
                        return Err(ApprovalActionError::HighRisk);
                    }
                    if automatic && (!positive_label(&option.label) || live.summary.risk != "low") {
                        return Err(ApprovalActionError::HighRisk);
                    }
                }
                let input = option_input(option);
                s.approval_reservation = Some((reservation, request.binding.clone()));
                (reservation, live.summary, s.snapshot.cwd.clone(), input)
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
                selected_text: input.clone(),
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
        now: SystemTime,
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
            s.snapshot.state = s.snapshot.activity.display_state().into();
            effects.0.push(CoreEffect::Broadcast(s.update_message()));
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
}
fn label(text: &str) -> String {
    let mut s = text
        .trim()
        .to_lowercase()
        .replace('’', "'")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    for suffix in [
        " (y)",
        " (p)",
        " (n)",
        " (esc)",
        " (escape)",
        " (!)",
        " (?)",
        " (#)",
        " (recommended)",
    ] {
        if let Some(v) = s.strip_suffix(suffix) {
            s = v.trim().into();
            break;
        }
    }
    s
}
pub(super) fn negative_label(text: &str) -> bool {
    let s = label(text);
    matches!(
        s.as_str(),
        "no" | "deny"
            | "deny once"
            | "reject"
            | "cancel"
            | "skip"
            | "abort"
            | "decline"
            | "do not allow"
            | "don't allow"
            | "do not run"
            | "don't run"
            | "拒否"
            | "許可しない"
            | "中止"
    ) || [
        "no ", "deny ", "reject ", "cancel ", "skip ", "abort ", "decline ",
    ]
    .iter()
    .any(|p| s.starts_with(p))
}
pub(super) fn positive_label(text: &str) -> bool {
    let s = label(text);
    if negative_label(&s)
        || [
            "always",
            "don't ask",
            "dont ask",
            "all similar",
            "this session",
            "session",
        ]
        .iter()
        .any(|p| s.contains(p))
    {
        return false;
    }
    matches!(
        s.as_str(),
        "yes"
            | "yes, allow once"
            | "allow"
            | "allow once"
            | "approve"
            | "run"
            | "run command"
            | "run (once)"
            | "continue"
            | "proceed"
            | "yes, proceed"
            | "yes proceed"
            | "許可"
            | "実行"
            | "続行"
    )
}
pub(super) fn option_input(option: &proto::ApprovalOption) -> String {
    if !option.send_text.is_empty() {
        option.send_text.clone()
    } else if option.is_current {
        "\r".into()
    } else if option.num > 0 {
        format!("{}\r", option.num)
    } else {
        String::new()
    }
}
