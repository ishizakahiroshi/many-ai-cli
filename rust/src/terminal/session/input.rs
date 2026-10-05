use super::super::input::split_bracketed_paste_submit;
use super::*;
use crate::approval::{identity, marker, record::confirmed_turn_text};
pub(super) struct SendResult {
    pub sequences: Vec<InputSeq>,
    pub remainder: Vec<u8>,
    pub submit: SubmitReceipt,
    pub error: Option<SessionError>,
}
struct FrameReservationGuard<'a> {
    engine: &'a SessionEngine,
    reservation: Option<InputReservation>,
}
impl Drop for FrameReservationGuard<'_> {
    fn drop(&mut self) {
        if let Some(reservation) = self.reservation.take() {
            self.engine.release_frame(reservation);
        }
    }
}
impl SessionEngine {
    pub(super) fn input_ticket(
        &self,
        binding: SessionBinding,
    ) -> Result<lane::Ticket, SessionError> {
        let mut state = lock(&self.state);
        let s = state.session(binding)?;
        Ok(s.input_lane.reserve())
    }
    fn input_valid(
        &self,
        binding: SessionBinding,
        authority: &InputAuthority,
    ) -> Result<(), SessionError> {
        let mut state = lock(&self.state);
        if let InputAuthority::Ui(ui) = authority
            && !state.authorized(*ui)
        {
            return Err(SessionError::AuthenticationExpired);
        }
        let s = state.session(binding)?;
        if !s.connected {
            return Err(SessionError::Transport("wrapper not connected".into()));
        }
        if let InputAuthority::NativeApproval(action) = authority
            && !s.approval_action_matches(action)
        {
            return Err(SessionError::StaleBinding);
        }
        Ok(())
    }
    pub(super) async fn write_frame(
        &self,
        binding: SessionBinding,
        bytes: Vec<u8>,
        sequence: Option<InputSeq>,
        authority: &InputAuthority,
    ) -> Result<InputSeq, SessionError> {
        self.input_valid(binding, authority)?;
        let (reservation, frame) = if let Some(seq) = sequence {
            let mut state = lock(&self.state);
            let s = state.session(binding)?;
            let frame = InputFrame { seq, bytes };
            if !s.input.readmit(binding.wrapper, &frame) {
                return Err(SessionError::InvalidRequest(
                    "invalid replayed input sequence".into(),
                ));
            }
            (
                InputReservation {
                    binding,
                    sequence: seq,
                },
                frame,
            )
        } else {
            self.reserve_frame(binding, bytes)?
        };
        let mut guard = FrameReservationGuard {
            engine: self,
            reservation: Some(reservation),
        };
        let result = self
            .transport
            .send(
                binding,
                proto::Message {
                    r#type: "pty_input".into(),
                    session_id: binding.session.0,
                    data: frame.bytes,
                    input_seq: frame.seq.0,
                    ..Default::default()
                },
            )
            .await;
        result?;
        guard.reservation = None;
        Ok(frame.seq)
    }
    async fn settle(
        &self,
        binding: SessionBinding,
        authority: &InputAuthority,
        cancel: &TaskCancellation,
    ) -> Result<(), SessionError> {
        let provider = {
            let mut state = lock(&self.state);
            state.session(binding)?.snapshot.provider.clone()
        };
        let t = self.options.submit_timing;
        let minimum = if matches!(provider.as_str(), "codex" | "opencode") {
            t.slow_minimum
        } else {
            t.minimum
        };
        let start = tokio::time::Instant::now();
        loop {
            self.input_valid(binding, authority)?;
            if cancel.token().is_cancelled() {
                return Err(SessionError::Cancelled);
            }
            let elapsed = start.elapsed();
            if elapsed >= t.maximum {
                return Ok(());
            }
            let quiet = {
                let mut state = lock(&self.state);
                state.session(binding)?.last_output.is_none_or(|at| {
                    Timestamp::now().duration_since(at).unwrap_or_default() >= t.idle_settle
                })
            };
            if elapsed >= minimum && quiet {
                return Ok(());
            }
            tokio::select! {biased;_=cancel.token().cancelled()=>return Err(SessionError::Cancelled),_=tokio::time::sleep(t.poll.max(Duration::from_millis(1)))=>{}}
        }
    }
    async fn output_after(
        &self,
        binding: SessionBinding,
        generation: u64,
        authority: &InputAuthority,
        cancel: &TaskCancellation,
    ) -> Result<bool, SessionError> {
        let until = tokio::time::Instant::now() + self.options.submit_timing.confirm_window;
        loop {
            self.input_valid(binding, authority)?;
            if cancel.token().is_cancelled() {
                return Err(SessionError::Cancelled);
            }
            if lock(&self.state)
                .sessions
                .get(&binding.session)
                .is_some_and(|s| s.output_generation != generation)
            {
                return Ok(true);
            }
            if tokio::time::Instant::now() >= until {
                return Ok(false);
            }
            tokio::select! {biased;_=cancel.token().cancelled()=>return Err(SessionError::Cancelled),_=tokio::time::sleep(self.options.submit_timing.poll.max(Duration::from_millis(1)))=>{}}
        }
    }
    fn restore_awaiting_submit_enter(&self, binding: SessionBinding) {
        let mut state = lock(&self.state);
        if let Some(session) = state
            .sessions
            .get_mut(&binding.session)
            .filter(|session| session.binding.incarnation == binding.incarnation)
        {
            session.awaiting_submit_enter = true;
        }
    }
    pub(super) async fn send_combined(
        &self,
        binding: SessionBinding,
        bytes: Vec<u8>,
        authority: &InputAuthority,
        cancel: &TaskCancellation,
    ) -> SendResult {
        let mut result = SendResult {
            sequences: Vec::new(),
            remainder: bytes.clone(),
            submit: SubmitReceipt::NotRequested,
            error: None,
        };
        let (first, delayed) = split_bracketed_paste_submit(&bytes);
        let awaiting = {
            let mut state = lock(&self.state);
            match state.session(binding) {
                Ok(s) => std::mem::take(&mut s.awaiting_submit_enter),
                Err(e) => {
                    result.error = Some(e);
                    return result;
                }
            }
        };
        let split_enter = awaiting && delayed.is_empty() && first == b"\r";
        if split_enter && let Err(e) = self.settle(binding, authority, cancel).await {
            self.restore_awaiting_submit_enter(binding);
            result.error = Some(e);
            return result;
        }
        if cancel.token().is_cancelled() {
            if split_enter {
                self.restore_awaiting_submit_enter(binding);
            }
            result.error = Some(SessionError::Cancelled);
            return result;
        }
        match self
            .write_frame(binding, first.to_vec(), None, authority)
            .await
        {
            Ok(seq) => result.sequences.push(seq),
            Err(e) => {
                if split_enter {
                    self.restore_awaiting_submit_enter(binding);
                }
                result.error = Some(e);
                return result;
            }
        }
        result.remainder = delayed.to_vec();
        if delayed.is_empty() && !split_enter {
            if first.ends_with(b"\x1b[201~") {
                let mut state = lock(&self.state);
                if let Ok(s) = state.session(binding) {
                    s.awaiting_submit_enter = true;
                }
                result.submit = SubmitReceipt::PendingEnter;
            }
            return result;
        }
        if !delayed.is_empty() {
            if let Err(e) = self.settle(binding, authority, cancel).await {
                result.error = Some(e);
                return result;
            }
            match self
                .write_frame(binding, delayed.to_vec(), None, authority)
                .await
            {
                Ok(seq) => result.sequences.push(seq),
                Err(e) => {
                    result.error = Some(e);
                    return result;
                }
            }
        }
        result.remainder.clear();
        result.submit = SubmitReceipt::EnterWritten;
        // Only a complete lack of newer output permits one extra Enter. Never
        // resend the pasted body and never redirect to a replacement wrapper.
        let generation = lock(&self.state)
            .sessions
            .get(&binding.session)
            .map(|s| s.output_generation)
            .unwrap_or(0);
        match self
            .output_after(binding, generation, authority, cancel)
            .await
        {
            Ok(true) => {
                result.submit = SubmitReceipt::OutputObserved;
                return result;
            }
            Err(e) => {
                result.error = Some(e);
                return result;
            }
            Ok(false) => {}
        }
        match self
            .write_frame(binding, b"\r".to_vec(), None, authority)
            .await
        {
            Ok(seq) => result.sequences.push(seq),
            Err(e) => {
                result.error = Some(e);
                return result;
            }
        }
        let generation = lock(&self.state)
            .sessions
            .get(&binding.session)
            .map(|s| s.output_generation)
            .unwrap_or(0);
        result.submit = match self
            .output_after(binding, generation, authority, cancel)
            .await
        {
            Ok(true) => SubmitReceipt::OutputObserved,
            Ok(false) => SubmitReceipt::UnconfirmedAfterRetry,
            Err(e) => {
                result.error = Some(e);
                SubmitReceipt::EnterWritten
            }
        };
        result
    }
    fn submitted_effects(
        &self,
        binding: SessionBinding,
        request: &InputRequest,
        now: Timestamp,
    ) -> Result<(CoreEffects, CoreEffects), SessionError> {
        let mut state = lock(&self.state);
        let s = state.session(binding)?;
        let mut effects = CoreEffects::default();
        let mut post = CoreEffects::default();
        let mut summary_update = None;
        let mut title_meta = None;
        let raw = proto::wire::go_utf8_lossy(&request.bytes);
        if let Some(text) = confirmed_turn_text(&raw) {
            if matches!(request.authority, InputAuthority::Ui(_)) {
                s.completion.discard_text_question();
                if text == "/clear" {
                    s.snapshot.first_message.clear();
                    s.snapshot.last_message.clear();
                } else {
                    s.completion.user_turn();
                    if s.subagents
                        .as_ref()
                        .is_none_or(|tree| !tree.nodes.iter().any(|n| n.state == "running"))
                    {
                        s.observer_turn_started_at = now;
                    }
                    let latest = marker::extract_vt(&s.vt).map(|m| {
                        identity::marker_candidate(
                            &s.snapshot.provider,
                            &m.block,
                            s.approval.epoch(),
                        )
                    });
                    s.approval.user_turn_boundary(latest.as_ref());
                    let masked = mask_secrets(&text);
                    if s.snapshot.first_message.is_empty() {
                        s.snapshot.first_message = masked.clone();
                        if s.snapshot.auto_title.is_empty() {
                            s.snapshot.auto_title = auto_title(&masked);
                            title_meta = Some(s.card_meta());
                        }
                    }
                    if !text.bytes().all(|b| b.is_ascii_digit()) {
                        s.snapshot.last_message = masked;
                    }
                    s.replay.append(USER_TURN_MARKER);
                    effects
                        .0
                        .push(CoreEffect::Notify(CoreEvent::GitTurnCapture {
                            binding,
                            started_at: timestamp(now)?,
                            ended_at: None,
                        }));
                    effects.0.push(CoreEffect::Broadcast(proto::Message {
                        r#type: "pty_data".into(),
                        session_id: binding.session.0,
                        data: USER_TURN_MARKER.into(),
                        ..Default::default()
                    }));
                    effects.0.push(CoreEffect::Broadcast(proto::Message {
                        r#type: "user_turn_started".into(),
                        session_id: binding.session.0,
                        approval_source_epoch: s.approval.epoch().0,
                        ..Default::default()
                    }));
                }
                summary_update = Some(if text == "/clear" {
                    s.short_update()
                } else {
                    s.summary_update(true)
                });
            }
            if !matches!(request.authority, InputAuthority::NativeApproval(_)) {
                let closure = s.approval.submitted_turn(&text, now);
                if !closure.0.is_empty() {
                    effects.0.extend(closure.0);
                    if let Some(update) = s.refresh_approval_activity() {
                        effects.0.push(CoreEffect::Broadcast(update));
                    }
                }
            }
        }
        if matches!(request.authority, InputAuthority::Ui(_)) {
            post.0.push(history(
                binding.session,
                now,
                "user_input",
                object(serde_json::json!({"text":mask_secrets(&raw)})),
            )?);
        }
        if let Some(update) = summary_update {
            post.0.push(CoreEffect::Broadcast(update));
        }
        if let Some(meta) = title_meta {
            post.0
                .push(CoreEffect::Persist(PersistenceEffect::CardMetaBestEffort {
                    session: binding.session,
                    meta,
                }));
        }
        Ok((state.route(effects), post))
    }
    async fn finish_submitted_effects(&self, binding: SessionBinding, effects: CoreEffects) {
        let effects = {
            let mut state = lock(&self.state);
            if state
                .sessions
                .get(&binding.session)
                .is_none_or(|s| s.binding.incarnation != binding.incarnation)
            {
                return;
            }
            state.route(effects)
        };
        if let Err(error) = self.apply_effects(effects).await {
            self.warn("post-input effects", &error);
        }
    }
    /// Read the canonical input gate for this exact wrapper incarnation.
    /// Observing an expired gate does not clear or otherwise mutate it.
    pub fn initial_gate_pending(
        &self,
        binding: SessionBinding,
        now: Timestamp,
    ) -> Result<bool, SessionError> {
        timestamp(now)?;
        let state = lock(&self.state);
        let session = state
            .sessions
            .get(&binding.session)
            .ok_or(SessionError::NotFound(binding.session))?;
        if session.binding != binding || !session.connected {
            return Err(SessionError::StaleBinding);
        }
        Ok(session.input.gated(now))
    }
    pub fn set_initial_gate(
        &self,
        binding: SessionBinding,
        now: Timestamp,
    ) -> Result<(), SessionError> {
        lock(&self.state)
            .session(binding)?
            .input
            .set_initial_gate(now);
        Ok(())
    }
}
impl InputQueue for SessionEngine {
    fn submit<'a>(
        &'a self,
        binding: SessionBinding,
        request: InputRequest,
        now: Timestamp,
        cancel: &'a TaskCancellation,
    ) -> CoreFuture<'a, InputReceipt> {
        // Reserve at invocation, not first poll (the transport may spawn futures).
        let enqueued_at = tokio::time::Instant::now();
        let ticket = self.input_ticket(binding);
        Box::pin(async move {
            let mut receipt = InputReceipt {
                binding,
                disposition: InputDisposition::StaleBinding,
                submit: SubmitReceipt::NotRequested,
            };
            let ticket = match ticket {
                Ok(t) => t,
                Err(SessionError::NotFound(_)) => {
                    receipt.disposition = InputDisposition::MissingSession;
                    return receipt;
                }
                Err(_) => return receipt,
            };
            if !ticket.wait(cancel).await {
                receipt.disposition = InputDisposition::Failed {
                    unsent_remainder: request.bytes,
                    detail: "cancelled".into(),
                };
                return receipt;
            }
            let now = now.checked_add(enqueued_at.elapsed()).unwrap_or(now);
            // Go holds an authorization read guard over accepted handleInput,
            // including persistence/Git capture and deferred Enter. Revocation
            // removes membership immediately but drains this guard before ACK.
            let _accepted_ui = if let InputAuthority::Ui(ui) = request.authority {
                match self.authorize_ui_work(ui) {
                    Ok(guard) => Some(guard),
                    Err(_) => {
                        receipt.disposition = InputDisposition::AuthenticationExpired;
                        return receipt;
                    }
                }
            } else {
                None
            };
            let delivery_authority = if matches!(request.authority, InputAuthority::Ui(_)) {
                InputAuthority::Internal
            } else {
                request.authority.clone()
            };
            let (effects, post_effects) = match self.submitted_effects(binding, &request, now) {
                Ok(e) => e,
                Err(SessionError::AuthenticationExpired) => {
                    receipt.disposition = InputDisposition::AuthenticationExpired;
                    return receipt;
                }
                Err(_) => return receipt,
            };
            if let Err(error) = self.apply_effects(effects).await {
                receipt.disposition = InputDisposition::Failed {
                    unsent_remainder: request.bytes,
                    detail: format!("input effects failed: {error:?}"),
                };
                return receipt;
            }
            let deferred = {
                let mut state = lock(&self.state);
                let s = match state.session(binding) {
                    Ok(s) => s,
                    Err(_) => return receipt,
                };
                let bypass = matches!(request.authority, InputAuthority::InitialPrompt);
                if (!bypass && (s.input.gated(now) || s.input.pending_len() > 0)) || !s.connected {
                    let reason = if s.input.initial_prompt_phase() {
                        DeferredReason::InitialPrompt
                    } else {
                        DeferredReason::Wrapper
                    };
                    let dropped_oldest = s.input.enqueue(request.bytes.clone());
                    Some((reason, dropped_oldest))
                } else {
                    None
                }
            };
            if let Some((reason, dropped_oldest)) = deferred {
                receipt.disposition = InputDisposition::Deferred {
                    reason,
                    dropped_oldest,
                };
                let effects = self.broadcast_ui(deferred_message(binding.session, reason));
                if let Err(e) = self.apply_effects(effects).await {
                    self.warn("deferred input notice", &e);
                }
                self.finish_submitted_effects(binding, post_effects).await;
                return receipt;
            }
            let result = self
                .send_combined(binding, request.bytes.clone(), &delivery_authority, cancel)
                .await;
            receipt.submit = result.submit;
            if !result.remainder.is_empty() {
                // Revoked/cancelled work is never silently resurrected on reconnect.
                if !matches!(
                    result.error,
                    Some(SessionError::AuthenticationExpired | SessionError::Cancelled)
                ) {
                    let mut state = lock(&self.state);
                    if let Some(s) = state
                        .sessions
                        .get_mut(&binding.session)
                        .filter(|s| s.binding.incarnation == binding.incarnation)
                    {
                        if matches!(request.authority, InputAuthority::InitialPrompt) {
                            s.input.requeue_front([result.remainder.clone()]);
                        } else {
                            s.input.enqueue(result.remainder.clone());
                        }
                    }
                }
                receipt.disposition = InputDisposition::Failed {
                    unsent_remainder: result.remainder,
                    detail: format!("{:?}", result.error),
                };
                let effects =
                    self.broadcast_ui(deferred_message(binding.session, DeferredReason::Wrapper));
                if let Err(e) = self.apply_effects(effects).await {
                    self.warn("deferred input notice", &e);
                }
            } else {
                receipt.disposition = InputDisposition::TransportWritten {
                    sequences: result.sequences,
                };
            }
            self.finish_submitted_effects(binding, post_effects).await;
            receipt
        })
    }
    fn reserve_frame(
        &self,
        binding: SessionBinding,
        bytes: Vec<u8>,
    ) -> Result<(InputReservation, InputFrame), SessionError> {
        let mut state = lock(&self.state);
        let s = state.session(binding)?;
        if !s.connected {
            return Err(SessionError::StaleBinding);
        }
        let frame = s.input.reserve(binding.wrapper, bytes);
        Ok((
            InputReservation {
                binding,
                sequence: frame.seq,
            },
            frame,
        ))
    }
    fn release_frame(&self, reservation: InputReservation) {
        let mut state = lock(&self.state);
        if let Some(s) = state
            .sessions
            .get_mut(&reservation.binding.session)
            .filter(|s| s.binding.incarnation == reservation.binding.incarnation)
        {
            s.input
                .release(reservation.binding.wrapper, reservation.sequence);
        }
    }
    fn acknowledge(&self, binding: SessionBinding, seq: InputSeq) -> AckDisposition {
        let mut state = lock(&self.state);
        let Some(s) = state
            .sessions
            .get_mut(&binding.session)
            .filter(|s| s.binding.incarnation == binding.incarnation)
        else {
            return AckDisposition::MissingSession;
        };
        s.input.acknowledge(binding.wrapper, seq)
    }
    fn transport_failed(&self, binding: SessionBinding, now: Timestamp) -> CoreEffects {
        self.disconnected(binding, now).unwrap_or_default()
    }
    fn flush<'a>(
        &'a self,
        binding: SessionBinding,
        cancel: &'a TaskCancellation,
    ) -> CoreFuture<'a, CoreEffects> {
        let ticket = self.input_ticket(binding);
        Box::pin(async move {
            let Ok(ticket) = ticket else {
                return CoreEffects::default();
            };
            if !ticket.wait(cancel).await {
                return CoreEffects::default();
            }
            let (pending, resend) = {
                let mut state = lock(&self.state);
                let Ok(s) = state.session(binding) else {
                    return CoreEffects::default();
                };
                if !s.connected || s.input.gated(Timestamp::now()) {
                    return CoreEffects::default();
                }
                (s.input.take_pending(), s.input.take_resend())
            };
            for (i, frame) in resend.iter().enumerate() {
                if cancel.token().is_cancelled()
                    || self
                        .write_frame(
                            binding,
                            frame.bytes.clone(),
                            Some(frame.seq),
                            &InputAuthority::Internal,
                        )
                        .await
                        .is_err()
                {
                    let mut state = lock(&self.state);
                    if let Some(s) = state
                        .sessions
                        .get_mut(&binding.session)
                        .filter(|s| s.binding.incarnation == binding.incarnation)
                    {
                        s.input.requeue_resend(resend[i..].iter().cloned());
                        s.input.requeue_front(pending);
                    }
                    return CoreEffects::default();
                }
            }
            for (i, bytes) in pending.iter().enumerate() {
                let result = self
                    .send_combined(binding, bytes.clone(), &InputAuthority::Internal, cancel)
                    .await;
                if !result.remainder.is_empty() {
                    let mut state = lock(&self.state);
                    if let Some(s) = state
                        .sessions
                        .get_mut(&binding.session)
                        .filter(|s| s.binding.incarnation == binding.incarnation)
                    {
                        s.input.requeue_front(
                            std::iter::once(result.remainder)
                                .chain(pending[i + 1..].iter().cloned()),
                        );
                    }
                    break;
                }
            }
            CoreEffects::default()
        })
    }
    fn clear_initial_gate(&self, id: LiveSessionId) -> CoreEffects {
        let mut state = lock(&self.state);
        if let Some(s) = state.sessions.get_mut(&id) {
            s.input.clear_initial_gate();
        }
        CoreEffects::default()
    }
}
fn deferred_message(id: LiveSessionId, reason: DeferredReason) -> proto::Message {
    proto::Message {
        r#type: "input_deferred".into(),
        session_id: id.0,
        reason: match reason {
            DeferredReason::InitialPrompt => "initial_prompt",
            DeferredReason::Wrapper => "wrapper",
        }
        .into(),
        ..Default::default()
    }
}
fn auto_title(text: &str) -> String {
    use regex::Regex;
    use std::sync::LazyLock;
    static PATTERNS: LazyLock<[Regex; 5]> = LazyLock::new(|| {
        [
            r"^```(?:[A-Za-z0-9_+#.-]+)?(?-u:\s)*",
            r"^[A-Za-z]:[\\/][^\t\n\f\r ]+",
            r"^/(?:[^/\t\n\f\r ]+/)+[^/\t\n\f\r ]*",
            r"(?i)^https?://[^\t\n\f\r ]+",
            r"^@[^\t\n\f\r ]+",
        ]
        .map(|s| Regex::new(s).expect("baseline auto title regex"))
    });
    let normalized = text
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut candidate = normalized.clone();
    loop {
        candidate = candidate.trim().into();
        let Some((i, m)) = PATTERNS
            .iter()
            .enumerate()
            .find_map(|(i, r)| r.find(&candidate).map(|m| (i, m)))
        else {
            break;
        };
        let matched = m.as_str().to_owned();
        let rest = candidate[m.end()..].trim();
        candidate = if matches!(i, 1 | 2) && !rest.is_empty() {
            format!(
                "{} {}",
                matched.rsplit(['/', '\\']).next().unwrap_or_default(),
                rest
            )
        } else {
            rest.into()
        };
    }
    candidate = candidate.trim_end_matches("```").trim().into();
    if candidate.is_empty() {
        candidate = normalized;
    }
    candidate.chars().take(40).collect()
}
