//! Actionable board notices keep their own FIFO remainder. They never enter
//! reconnect input queues, which cannot recheck approval or composer readiness.
use super::*;

pub enum BoardEventWrite {
    Written,
    Blocked,
    Failed {
        remainder: Vec<u8>,
        error: SessionError,
    },
}
impl SessionEngine {
    pub fn board_notice_output_idle(&self, binding: SessionBinding) -> Result<bool, SessionError> {
        let mut state = lock(&self.state);
        let session = state.session(binding)?;
        Ok(session.snapshot.parent_session_id.0 == 0
            && !session.snapshot.orchestration_id.0.is_empty()
            && !session.input.initial_prompt_phase()
            && !session.snapshot.activity.awaiting_user
            && session.snapshot.activity.is_idle())
    }
    pub fn mark_orchestration_child_state(
        &self,
        binding: SessionBinding,
        declared: &str,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError> {
        let mut state = lock(&self.state);
        let session = state.session(binding)?;
        if session.snapshot.state == declared {
            return Ok(CoreEffects::default());
        }
        session.snapshot.state = declared.to_owned();
        let effects = CoreEffects(vec![
            CoreEffect::Broadcast(session.update_message()),
            CoreEffect::Persist(PersistenceEffect::SessionState {
                session: binding.session,
                state: declared.to_owned(),
                last_output_at: session.snapshot.last_output_at.clone(),
            }),
        ]);
        let _ = now;
        Ok(state.route(effects))
    }
    pub fn deliver_board_event<'a>(
        &'a self,
        binding: SessionBinding,
        orchestration: &'a OrchestrationId,
        bytes: Vec<u8>,
        cancel: &'a TaskCancellation,
    ) -> CoreFuture<'a, BoardEventWrite> {
        let ticket = self.input_ticket(binding);
        Box::pin(async move {
            let ticket = match ticket {
                Ok(ticket) => ticket,
                Err(error) => {
                    return BoardEventWrite::Failed {
                        remainder: bytes,
                        error,
                    };
                }
            };
            if !ticket.wait(cancel).await {
                return BoardEventWrite::Failed {
                    remainder: bytes,
                    error: SessionError::Cancelled,
                };
            }
            {
                let mut state = lock(&self.state);
                let session = match state.session(binding) {
                    Ok(session) => session,
                    Err(error) => {
                        return BoardEventWrite::Failed {
                            remainder: bytes,
                            error,
                        };
                    }
                };
                if session.snapshot.parent_session_id.0 != 0
                    || session.snapshot.orchestration_id != *orchestration
                    || !session.connected
                    || session.input.initial_prompt_phase()
                    || session.input.gated(Timestamp::now())
                    || session.input.pending_len() != 0
                    || session.input.resend_len() != 0
                    || session.snapshot.activity.awaiting_user
                    || session.snapshot.activity.awaiting_approval
                    || session.approval.record().is_some()
                    || !session.snapshot.activity.is_idle()
                {
                    return BoardEventWrite::Blocked;
                }
                if session.snapshot.provider == "codex" {
                    let lines = session.vt.lines();
                    let screen: String = lines
                        .concat()
                        .chars()
                        .filter(|c| !c.is_whitespace())
                        .collect();
                    if screen.contains("Sidefrommainthread")
                        || crate::orchestration::initial_prompt::screen_blocker("codex", &lines)
                            .is_some()
                        || !screen.contains("AskCodextodoanything")
                    {
                        return BoardEventWrite::Blocked;
                    }
                }
            }
            // Go sends this one already framed <=4096-byte notice together;
            // unlike user submits it does not settle/retry a second Enter.
            match self
                .write_frame(binding, bytes.clone(), None, &InputAuthority::Internal)
                .await
            {
                Ok(_) => BoardEventWrite::Written,
                Err(error) => BoardEventWrite::Failed {
                    remainder: bytes,
                    error,
                },
            }
        })
    }
}
