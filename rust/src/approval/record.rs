//! Immutable single-record lifecycle; ordered effects are applied after releasing
//! the session lock. Old ledger closure always precedes replacement insertion.
use crate::proto::{self, core::*};
use std::time::SystemTime;

#[derive(Clone)]
pub struct ApprovalState {
    session: LiveSessionId,
    provider: String,
    version: ApprovalStateVersion,
    epoch: ApprovalSourceEpoch,
    record: Option<ImmutableApprovalRecord>,
    consumed: Option<CandidateIdentity>,
    epoch_pending: bool,
    native_clear_misses: usize,
}
impl ApprovalState {
    pub fn new(session: LiveSessionId, provider: String) -> Self {
        Self {
            session,
            provider,
            version: ApprovalStateVersion(0),
            epoch: ApprovalSourceEpoch(1),
            record: None,
            consumed: None,
            epoch_pending: false,
            native_clear_misses: 0,
        }
    }
    pub fn record(&self) -> Option<&ImmutableApprovalRecord> {
        self.record.as_ref()
    }
    pub fn epoch(&self) -> ApprovalSourceEpoch {
        self.epoch
    }
    pub fn version(&self) -> ApprovalStateVersion {
        self.version
    }
    pub fn consumed(&self) -> Option<&CandidateIdentity> {
        self.consumed.as_ref()
    }
    pub fn snapshot(&self) -> ApprovalSessionSnapshot {
        ApprovalSessionSnapshot {
            session: self.session,
            version: self.version,
            record: self.record.clone(),
        }
    }
    pub fn note_native_seen(&mut self, seen: bool) {
        if seen {
            self.native_clear_misses = 0;
        } else {
            self.native_clear_misses = self.native_clear_misses.saturating_add(1);
        }
    }
    pub fn native_clear_misses(&self) -> usize {
        self.native_clear_misses
    }
    pub fn blocks_vt_marker(&self) -> bool {
        self.record
            .as_ref()
            .is_some_and(|r| r.data().origin == "native")
            && self.native_clear_misses == 0
    }
    fn advance_epoch(&mut self) {
        self.epoch = ApprovalSourceEpoch(self.epoch.0.wrapping_add(1).max(1));
        self.epoch_pending = false;
    }
    fn carry(&mut self, latest_vt: Option<&CandidateIdentity>) {
        if let (Some(consumed), Some(latest)) = (&mut self.consumed, latest_vt)
            && !consumed.key.is_empty()
            && consumed.key == latest.key
        {
            if !latest.shape.is_empty() {
                consumed.shape = latest.shape.clone();
            }
            consumed.source_epoch = self.epoch;
            self.epoch_pending = true;
        }
    }
    /// A live boundary only. Reflow and replay do not call this operation.
    pub fn user_turn_boundary(&mut self, latest_vt: Option<&CandidateIdentity>) {
        if self.record.is_some() {
            return;
        }
        self.advance_epoch();
        self.carry(latest_vt);
    }
    /// Transcript user records are fresh observations: do not carry old VT text.
    pub fn transcript_user_boundary(&mut self) {
        if self.record.is_none() {
            self.advance_epoch();
        }
    }
    pub fn observe(
        &mut self,
        mut data: ApprovalRecordData,
        latest_vt: Option<&CandidateIdentity>,
        now: SystemTime,
    ) -> CoreEffects {
        if data.source == "go_vt" && data.origin == "marker" && self.blocks_vt_marker() {
            return CoreEffects::default();
        }
        if self.epoch_pending {
            if self
                .consumed
                .as_ref()
                .is_some_and(|c| c.key == data.candidate.key && c.source_epoch == self.epoch)
            {
                return CoreEffects::default();
            }
            self.advance_epoch();
            self.carry(latest_vt);
        }
        data.candidate.source_epoch = self.epoch;
        if self
            .consumed
            .as_ref()
            .is_some_and(|c| c.same_candidate(&data.candidate))
            || self
                .record
                .as_ref()
                .is_some_and(|r| r.data().candidate.same_candidate(&data.candidate))
        {
            return CoreEffects::default();
        }
        let mut effects = self.close(ApprovalCloseReason::Superseded, "", now);
        self.version.0 = self.version.0.wrapping_add(1);
        self.native_clear_misses = 0;
        let detected = ApprovalDetected {
            live_session_id: self.session,
            sig: data.sig.clone(),
            source: data.source.clone(),
            kind: data.kind.clone(),
            provider: self.provider.clone(),
            question: data.question.clone(),
            context: data.context.clone(),
            block: data.block.clone(),
            candidate_key: data.candidate.key.clone(),
            source_epoch: data.candidate.source_epoch,
            options: data.options.clone(),
            detected_at: Some(data.detected_at),
        };
        let record = ImmutableApprovalRecord::new(data);
        effects
            .0
            .push(CoreEffect::Persist(PersistenceEffect::ApprovalDetected(
                detected,
            )));
        effects.0.push(CoreEffect::Broadcast(proto::Message {
            r#type: "approval_state".into(),
            session_id: self.session.0,
            provider: self.provider.clone(),
            approval_state: Some(proto::ApprovalState {
                version: self.version.0,
                open: Some(wire_record(&record)),
                close: None,
            }),
            ..Default::default()
        }));
        self.record = Some(record);
        effects
    }
    pub fn mark_consumed(&mut self, identity: CandidateIdentity) {
        self.epoch_pending = identity.source_epoch == self.epoch;
        self.consumed = Some(identity);
    }
    pub fn consume(
        &mut self,
        binding: &ApprovalActionBinding,
        selected: &str,
        now: SystemTime,
    ) -> Result<CoreEffects, ApprovalActionError> {
        let Some(record) = &self.record else {
            return Err(ApprovalActionError::NotFound);
        };
        if binding.session.session != self.session
            || binding.candidate_key != record.data().candidate.key
            || binding.source_epoch != record.data().candidate.source_epoch
            || binding.sig != record.data().sig
        {
            return Err(ApprovalActionError::StaleCandidate);
        }
        self.mark_consumed(record.data().candidate.clone());
        Ok(self.close(ApprovalCloseReason::Answered, selected, now))
    }
    pub fn submitted_turn(&mut self, text: &str, now: SystemTime) -> CoreEffects {
        if let Some(record) = &self.record
            && record.data().origin == "marker"
        {
            self.mark_consumed(record.data().candidate.clone());
            return self.close(ApprovalCloseReason::AnsweredTerminal, text, now);
        }
        CoreEffects::default()
    }
    pub fn close(
        &mut self,
        reason: ApprovalCloseReason,
        answer: &str,
        now: SystemTime,
    ) -> CoreEffects {
        let Some(record) = self.record.take() else {
            return CoreEffects::default();
        };
        let record = record.data();
        self.version.0 = self.version.0.wrapping_add(1);
        self.native_clear_misses = 0;
        let mut effects = CoreEffects::default();
        if reason != ApprovalCloseReason::HistoryReset && !record.sig.is_empty() {
            effects
                .0
                .push(CoreEffect::Persist(PersistenceEffect::ApprovalConsumed {
                    session: self.session,
                    sig: record.sig.clone(),
                    selected_text: truncate_answer(answer),
                    resolved_at: now,
                }));
        }
        let reason = match reason {
            ApprovalCloseReason::Answered => "answered",
            ApprovalCloseReason::AnsweredTerminal => "answered_terminal",
            ApprovalCloseReason::Superseded => "superseded",
            ApprovalCloseReason::Vanished => "vanished",
            ApprovalCloseReason::SessionEnd => "session_end",
            ApprovalCloseReason::HistoryReset => "history_reset",
        };
        effects.0.push(CoreEffect::Broadcast(proto::Message {
            r#type: "approval_state".into(),
            session_id: self.session.0,
            provider: self.provider.clone(),
            approval_state: Some(proto::ApprovalState {
                version: self.version.0,
                open: None,
                close: Some(proto::ApprovalRecordClose {
                    candidate_key: record.candidate.key.clone(),
                    source_epoch: record.candidate.source_epoch.0,
                    sig: record.sig.clone(),
                    origin: record.origin.clone(),
                    reason: reason.into(),
                }),
            }),
            ..Default::default()
        }));
        effects
    }
    /// The only persisted answered-question recovery gate. Only the latest VT
    /// question of the matching kinds may restore the same candidate identity.
    pub fn restore_answered_vt_question(
        &mut self,
        identity: &CandidateIdentity,
        latest: Option<&ApprovalRow>,
    ) -> bool {
        if self.blocks_vt_marker()
            || identity.key.is_empty()
            || self
                .consumed
                .as_ref()
                .is_some_and(|c| c.key == identity.key)
            || self
                .record
                .as_ref()
                .is_some_and(|r| r.data().candidate.key == identity.key)
        {
            return false;
        }
        let Some(row) = latest else {
            return false;
        };
        if row.candidate_key != identity.key
            || row.source != "go_vt"
            || row.state != "resolved"
            || row.selected_text.trim().is_empty()
            || !matches!(
                row.kind.as_str(),
                "marker" | "plain_yes_no" | "sequential_choice" | "hub_choice"
            )
        {
            return false;
        }
        // A different just-consumed candidate still owns the sole slot. The
        // ledger can suppress this question without evicting that candidate.
        if !self.epoch_pending {
            let mut restored = identity.clone();
            restored.source_epoch = self.epoch;
            self.mark_consumed(restored);
        }
        true
    }
    pub fn wire_snapshot(&self) -> proto::ApprovalSessionState {
        proto::ApprovalSessionState {
            session_id: self.session.0,
            version: self.version.0,
            record: self.record.as_ref().map(wire_record),
        }
    }
}
pub fn wire_record(record: &ImmutableApprovalRecord) -> proto::ApprovalRecord {
    let r = record.data();
    proto::ApprovalRecord {
        candidate_key: r.candidate.key.clone(),
        candidate_shape: r.candidate.shape.clone(),
        source_epoch: r.candidate.source_epoch.0,
        sig: r.sig.clone(),
        origin: r.origin.clone(),
        source: r.source.clone(),
        kind: r.kind.clone(),
        block: r.block.clone(),
        question: r.question.clone(),
        context: r.context.clone(),
        options: r.options.clone(),
        summary: (r.origin == "native").then(|| r.summary.clone()),
        detected_at: crate::proto::time::format_rfc3339(r.detected_at).unwrap_or_default(),
    }
}
pub fn truncate_answer(answer: &str) -> String {
    answer.trim().chars().take(200).collect()
}
pub fn confirmed_turn_text(raw: &str) -> Option<String> {
    if !raw.ends_with('\r') && !raw.ends_with("\x1b[201~") {
        return None;
    }
    let text = raw
        .trim_end_matches(['\r', '\n'])
        .replace("\x1b[200~", "")
        .replace("\x1b[201~", "");
    (!text.is_empty()).then_some(text)
}
