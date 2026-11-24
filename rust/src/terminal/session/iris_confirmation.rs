//! One-shot replies to an explicit binary work question. The caller establishes
//! the owner's decision. The session owns the binding and the exact input bytes.
use super::*;
use crate::approval::marker;
use regex::Regex;
use std::sync::LazyLock;
use tokio::sync::oneshot;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IrisAnswerOutcome {
    Applied,
    Expired,
    Unknown,
}

static BINARY_MAPPING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[（(][ ]*[YＹ][ ]*[:：][ ]*1[ ]*[/／][ ]*[NＮ][ ]*[:：][ ]*0[ ]*[）)]")
        .expect("binary marker grammar")
});

static NUMBERED_CHOICE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:^|\s)(?:[0-9]{1,2}[.)、]\s+|[QＱ][0-9]{1,2}[:：\s])").expect("numbered prompt")
});

/// A single question followed by the literal Yes=1, No=0 mapping. The ordinary
/// UI accepts additional marker forms; their answer must be chosen locally.
pub fn iris_confirmation_question(record: &ApprovalRecordData) -> Option<String> {
    if record.origin != "marker" || record.kind != "marker" || !record.options.is_empty() {
        return None;
    }
    let mut lines = record.block.trim().lines().map(str::trim);
    if lines.next()? != marker::OPEN {
        return None;
    }
    let line = lines.next()?;
    if lines.next()? != marker::CLOSE || lines.next().is_some() {
        return None;
    }
    let mut mappings = BINARY_MAPPING.find_iter(line);
    let suffix = mappings.next()?;
    if suffix.end() != line.len() || mappings.next().is_some() {
        return None;
    }
    let question = line[..suffix.start()].trim();
    if !question.ends_with(['?', '？'])
        || question.matches(['?', '？']).count() != 1
        || NUMBERED_CHOICE.is_match(question)
        || question.contains(marker::OPEN)
        || question.contains(marker::CLOSE)
        || question.chars().count() < 2
        || question.chars().any(|c| {
            c.is_control() || matches!(c, '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{206f}')
        })
    {
        return None;
    }
    Some(question.to_owned())
}

impl Session {
    /// Keep the ordinary dashboard's historical record, but retire its remote
    /// action when its source changes, including if the same text later reappears.
    pub(super) fn retire_iris_confirmation(&mut self) {
        if let Some(record) = self.approval.record() {
            self.iris_invalidated = Some(record.data().candidate.clone());
        }
    }

    pub(super) fn iris_vt_marker_visible(&self, block: &str) -> bool {
        // extract_vt includes scrollback for the local history UI. Only use
        // text still on the current screen to authorize a delayed answer.
        let screen = self.vt.lines().join("\n");
        marker::extract(&screen).is_some_and(|m| m.block == block)
    }

    fn matches_iris_confirmation(
        &self,
        identity: &CandidateIdentity,
        expected_block: &str,
        detected_at: Timestamp,
    ) -> bool {
        let matches_identity = |candidate: &CandidateIdentity| candidate.same_candidate(identity);
        self.connected
            && self.native_detection_eligible(Timestamp::now())
            && !self.subscription_login
            && !crate::profile::declared::is_acp(&self.snapshot.provider)
            && self.snapshot.execution_mode != "headless"
            && self.ended.is_none()
            && !self.native_prompt_visible
            && !self.awaiting_submit_enter
            && !self.input.gated(Timestamp::now())
            && self.input.pending_len() == 0
            && self.input.resend_len() == 0
            && self.input.inflight_len() == 0
            && !self.iris_invalidated.as_ref().is_some_and(matches_identity)
            && !self.approval.consumed().is_some_and(matches_identity)
            && self.approval.record().is_some_and(|record| {
                let data = record.data();
                identity.source_epoch.0 != 0
                    && matches_identity(&data.candidate)
                    && data.detected_at == detected_at
                    && data.block == expected_block
                    && match data.source.as_str() {
                        "go_vt" => self.iris_vt_marker_visible(expected_block),
                        "transcript" => {
                            !self.marker_source.codex_thread_ambiguous
                                && self.marker_source.miss_streak == 0
                        }
                        _ => false,
                    }
                    && iris_confirmation_question(data).is_some()
            })
    }
}

impl SessionEngine {
    pub fn iris_confirmation_is_current(
        &self,
        binding: SessionBinding,
        identity: &CandidateIdentity,
        expected_block: &str,
        detected_at: Timestamp,
    ) -> bool {
        lock(&self.state).session(binding).is_ok_and(|session| {
            session.matches_iris_confirmation(identity, expected_block, detected_at)
        })
    }

    pub async fn apply_iris_confirmation(
        &self,
        binding: SessionBinding,
        identity: &CandidateIdentity,
        expected_block: &str,
        detected_at: Timestamp,
        cancel: &TaskCancellation,
    ) -> IrisAnswerOutcome {
        let Ok(ticket) = self.input_ticket(binding) else {
            return IrisAnswerOutcome::Expired;
        };
        // One busy wrapper must not hold up other sessions' confirmations.
        // Use the same deadline for lane admission, socket dispatch and ACK.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        if !tokio::time::timeout_at(deadline, ticket.wait(cancel))
            .await
            .unwrap_or(false)
        {
            return IrisAnswerOutcome::Expired;
        }
        let frame = {
            let mut state = lock(&self.state);
            let Ok(s) = state.session(binding) else {
                return IrisAnswerOutcome::Expired;
            };
            if !s.matches_iris_confirmation(identity, expected_block, detected_at) {
                return IrisAnswerOutcome::Expired;
            }
            // Same single frame as the ordinary UI for an explicit Yes (1).
            // Allocate a unique sequence, but never queue consent for replay.
            let frame = s.input.reserve(binding.wrapper, b"1\r".to_vec());
            s.input.release(binding.wrapper, frame.seq);
            frame
        };
        let (acknowledge, received) = oneshot::channel();
        let mut attempted = false;
        let seq = frame.seq;
        let message = proto::Message {
            r#type: "pty_input".into(),
            session_id: binding.session.0,
            input_seq: seq.0,
            data: frame.bytes,
            ..Default::default()
        };
        let send = self.transport.send_guarded(
            binding,
            message,
            Box::new(|dispatch| {
                let mut state = lock(&self.state);
                let s = state.session(binding)?;
                if cancel.token().is_cancelled()
                    || !s.matches_iris_confirmation(identity, expected_block, detected_at)
                {
                    return Err(SessionError::StaleBinding);
                }
                // A repaint suppressed during resize can leave the cached flag
                // behind the visible screen. Inspect it at the actual send.
                let phrases = self.approval_phrases_for(&s.snapshot.provider);
                if crate::approval::detect::detect_native(
                    &s.snapshot.provider,
                    &s.vt.tail_lines(90),
                    &phrases,
                )
                .is_some()
                {
                    return Err(SessionError::StaleBinding);
                }
                attempted = true;
                s.approval.mark_consumed(identity.clone());
                s.iris_ack = Some((seq, acknowledge));
                dispatch()
            }),
        );
        tokio::select! {
            biased;
            _ = cancel.token().cancelled() => {}
            _ = tokio::time::timeout_at(deadline, send) => {}
        }
        if !attempted {
            return IrisAnswerOutcome::Expired;
        }
        // A socket flush alone cannot establish that the wrapper wrote input.
        // An ACK belongs to the original connection and the exact frame seq.
        let outcome = tokio::select! {
            biased;
            ack = received => if ack.is_ok() { IrisAnswerOutcome::Applied } else { IrisAnswerOutcome::Unknown },
            _ = cancel.token().cancelled() => IrisAnswerOutcome::Unknown,
            _ = tokio::time::sleep_until(deadline) => IrisAnswerOutcome::Unknown,
        };
        let effects = {
            let mut state = lock(&self.state);
            let Ok(s) = state.session(binding) else {
                return outcome;
            };
            s.iris_ack = None;
            if !s.approval.record().is_some_and(|record| {
                let data = record.data();
                data.candidate.same_candidate(identity)
                    && data.detected_at == detected_at
                    && data.block == expected_block
            }) {
                return outcome;
            }
            let now = Timestamp::now();
            let mut effects = if outcome == IrisAnswerOutcome::Applied {
                s.approval.close(ApprovalCloseReason::Answered, "1", now)
            } else {
                s.approval.close(ApprovalCloseReason::Vanished, "", now)
            };
            if let Some(update) = s.refresh_approval_activity(now) {
                effects.0.push(CoreEffect::Broadcast(update));
            }
            state.route(effects)
        };
        if let Err(error) = self.apply_effects(effects).await {
            self.warn("Iris confirmation closure", &error);
        }
        outcome
    }
}
