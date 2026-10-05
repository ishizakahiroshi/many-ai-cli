use super::*;
use crate::approval::{detect, identity, marker};
impl SessionEngine {
    pub fn observe_output(
        &self,
        binding: SessionBinding,
        chunk: OutputChunk,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError> {
        let stamp = timestamp(now)?;
        let mut state = lock(&self.state);
        let s = state.session(binding)?;
        if !s.connected {
            return Err(SessionError::StaleBinding);
        }
        let mut effects = CoreEffects::default();
        let mut marker_warning = None;
        let model_clean = identity::strip_ansi(&proto::wire::go_utf8_lossy(&chunk.bytes));
        let model_change = crate::application::output_model::change(
            &s.snapshot.provider,
            &chunk.bytes,
            &model_clean,
        );
        if self.journal.enabled() {
            let masked = crate::storage::mask_secret_bytes(&chunk.bytes);
            effects.0.push(history(binding.session,now,"pty_output",object(serde_json::json!({"data_b64":STANDARD.encode(&masked),"text":identity::strip_ansi(&proto::wire::go_utf8_lossy(&masked))})))?);
        }
        s.replay.append(&chunk.bytes);
        s.pty_bytes_seen = s.pty_bytes_seen.saturating_add(chunk.bytes.len() as i64);
        s.vt.write(&chunk.bytes);
        let cross_message =
            if s.snapshot.provider == "claude" && !s.snapshot.orchestration_id.0.is_empty() {
                let candidate =
                    crate::application::session_observations::cross_message::detect(&s.vt.lines());
                let signature = candidate
                    .as_ref()
                    .map(|v| v.signature.as_str())
                    .unwrap_or("");
                if signature != s.cross_message_screen_signature {
                    s.cross_message_screen_signature = signature.into();
                    candidate
                } else {
                    None
                }
            } else {
                None
            };
        let initial_model = if !s.initial_model_scan_done
            && s.snapshot.model.is_empty()
            && crate::application::output_model::initial_provider(&s.snapshot.provider)
        {
            s.initial_model_scan_bytes =
                s.initial_model_scan_bytes.saturating_add(chunk.bytes.len());
            if s.initial_model_scan_bytes > crate::application::output_model::INITIAL_SCAN_MAX_BYTES
            {
                s.initial_model_scan_done = true;
                None
            } else {
                Some(crate::application::output_model::banner(
                    &s.snapshot.provider,
                    &s.snapshot.cwd,
                    &s.vt.lines(),
                ))
            }
        } else {
            None
        };
        s.output_generation = s.output_generation.wrapping_add(1);
        s.last_output = Some(now);
        s.snapshot.last_output_at = stamp;
        if !s.usage_probe {
            let clean = identity::strip_ansi(&proto::wire::go_utf8_lossy(&chunk.bytes));
            effects.0.extend(s.scan_completion(&clean, now)?.0);
            effects
                .0
                .push(CoreEffect::Notify(CoreEvent::OutputObserved {
                    binding,
                    clean,
                    at: now,
                }));
        }
        let eligible = !s.usage_probe
            && !s.custom_provider
            && s.snapshot.provider != "shell"
            && !terminal(&s.snapshot.state)
            && s.resize_debounce.is_none_or(|until| now > until);
        if eligible
            && !s.marker_source.is_transcript(&s.snapshot.provider)
            && let Some(marker) = marker::extract_vt(&s.vt)
        {
            let (marker_effects, warning) = s.observe_marker(Some(marker), "go_vt", now)?;
            effects.0.extend(marker_effects.0);
            marker_warning = warning;
        }
        effects.0.push(CoreEffect::Broadcast(proto::Message {
            r#type: "pty_data".into(),
            session_id: binding.session.0,
            data: chunk.bytes,
            ..Default::default()
        }));
        if eligible {
            let lines = s.vt.tail_lines(90);
            let tail = lines.join("\n");
            if s.native_tail != tail {
                s.native_tail = tail;
                let phrases = self.approval_phrases_for(&s.snapshot.provider);
                if let Some(native) = detect::detect_native(&s.snapshot.provider, &lines, &phrases)
                {
                    let candidate = identity::candidate(
                        &s.snapshot.provider,
                        &native.kind,
                        &native.question,
                        &native.context,
                        &native.options,
                        s.approval.epoch(),
                    );
                    effects.0.extend(
                        s.approval
                            .observe(
                                ApprovalRecordData {
                                    candidate,
                                    sig: native.sig,
                                    origin: "native".into(),
                                    source: "go_vt".into(),
                                    kind: native.kind,
                                    block: String::new(),
                                    question: native.question,
                                    context: native.context,
                                    options: native.options,
                                    summary: native.summary,
                                    detected_at: now,
                                },
                                None,
                                now,
                            )
                            .0,
                    );
                } else if s
                    .approval
                    .record()
                    .is_some_and(|r| r.data().origin == "native")
                {
                    s.approval.note_native_seen(false);
                    if s.approval.native_clear_misses() >= 3 {
                        effects
                            .0
                            .extend(s.approval.close(ApprovalCloseReason::Vanished, "", now).0);
                    }
                }
            }
        }
        if !terminal(&s.snapshot.state) {
            let before = s.snapshot.activity.clone();
            let old = s.snapshot.state.clone();
            s.refresh_awaiting();
            s.snapshot.activity.output_idle = false;
            s.snapshot.activity.workflow_active = !s.snapshot.activity.awaiting_user;
            s.snapshot.activity.normalize();
            s.snapshot.state = s.snapshot.activity.display_state().into();
            if s.snapshot.activity != before || s.snapshot.state != old {
                effects.0.push(CoreEffect::Broadcast(s.activity_update()));
                effects.0.push(CoreEffect::Notify(CoreEvent::SessionState {
                    binding,
                    activity: s.snapshot.activity.clone(),
                    display_state: s.snapshot.state.clone(),
                }));
            }
            effects
                .0
                .push(CoreEffect::Persist(PersistenceEffect::SessionState {
                    session: binding.session,
                    state: s.snapshot.state.clone(),
                    last_output_at: s.snapshot.last_output_at.clone(),
                }));
        }
        let model_generation = s.output_generation;
        let model_revision = s.model_revision;
        let model_provider = s.snapshot.provider.clone();
        if let Some(candidate) = cross_message {
            effects.0.extend(
                state
                    .record_cross_message(binding, &candidate.sender, &candidate.line, now)?
                    .0,
            );
        }
        let mut effects = state.route(effects);
        drop(state);
        // Config publication may take the core lock while ConfigStore is held.
        // Route lookup therefore runs outside this lock, followed by exact
        // binding/generation admission so a later output cannot be overwritten.
        let mut candidates = Vec::new();
        if let Some(detected) = model_change {
            let route = (self.options.model_route)(&model_provider, &detected.model);
            candidates.push((detected, false, route));
        }
        if let Some(detected) = initial_model
            && !detected.model.is_empty()
        {
            let route = (self.options.model_route)(&model_provider, &detected.model);
            candidates.push((detected, true, route));
        }
        if !candidates.is_empty() {
            let mut state = lock(&self.state);
            let mut model_effects = CoreEffects::default();
            if let Ok(session) = state.session(binding)
                && session.connected
                && session.output_generation == model_generation
                && session.model_revision == model_revision
            {
                for (detected, initial, route) in candidates {
                    session.apply_detected_model(detected, initial, &route, &mut model_effects);
                }
            }
            effects.0.extend(state.route(model_effects).0);
        }
        if let Some(warning) = marker_warning {
            self.warn("approval marker suppressed: corrupt block", &warning);
        }
        Ok(effects)
    }
    /// The transcript worker supplies its detected marker through the same
    /// Session-owned admission path as VT. Binding validation precedes state or
    /// effects; the caller applies returned effects after this lock is released.
    pub fn observe_transcript_marker(
        &self,
        binding: SessionBinding,
        marker: Option<marker::Marker>,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError> {
        timestamp(now)?;
        let mut state = lock(&self.state);
        let session = state.session(binding)?;
        let (effects, warning) = session.observe_marker(marker, "transcript", now)?;
        let effects = state.route(effects);
        drop(state);
        if let Some(warning) = warning {
            self.warn("approval marker suppressed: corrupt block", &warning);
        }
        Ok(effects)
    }
    pub fn apply_observation(
        &self,
        binding: SessionBinding,
        observation: SessionObservation,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError> {
        self.apply_observation_guarded(binding, None, observation, now)
    }
    pub fn apply_observation_for_turn(
        &self,
        binding: SessionBinding,
        expected_turn: u64,
        observation: SessionObservation,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError> {
        self.apply_observation_guarded(binding, Some(expected_turn), observation, now)
    }
    fn apply_observation_guarded(
        &self,
        binding: SessionBinding,
        expected_turn: Option<u64>,
        observation: SessionObservation,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError> {
        timestamp(now)?;
        let mut state = lock(&self.state);
        let s = state.session(binding)?;
        if expected_turn.is_some_and(|turn| {
            turn != s.completion.confirmed_turn || !s.connected || s.ended.is_some()
        }) {
            return Err(SessionError::StaleBinding);
        }
        if let SessionObservation::CrossSessionMessage(message) = &observation {
            let candidate = crate::application::session_observations::cross_message::detect(
                std::slice::from_ref(&message.text),
            );
            let effects = match candidate {
                Some(v) => state.record_cross_message(binding, &v.sender, &v.line, now)?,
                None => CoreEffects::default(),
            };
            return Ok(state.route(effects));
        }
        let id = binding.session;
        let mut effects = CoreEffects::default();
        match observation {
            SessionObservation::Messages { first, last } => {
                s.snapshot.first_message = first.clone();
                s.snapshot.last_message = last.clone();
                effects
                    .0
                    .push(CoreEffect::Persist(PersistenceEffect::SessionMessages {
                        session: id,
                        first: first.clone(),
                        last: last.clone(),
                    }));
                effects.0.push(CoreEffect::Notify(CoreEvent::UserMessage {
                    binding,
                    first,
                    last,
                }));
                effects.0.push(CoreEffect::Broadcast(s.update_message()));
            }
            SessionObservation::Branch { branch, git_root } => {
                s.snapshot.branch = branch;
                s.git_root = git_root;
                effects.0.push(CoreEffect::Broadcast(s.update_message()));
            }
            SessionObservation::Model { model, effort } => {
                if (!model.is_empty() && s.snapshot.model != model)
                    || (!effort.is_empty() && s.snapshot.effort != effort)
                {
                    s.model_revision = s.model_revision.wrapping_add(1);
                }
                if !model.is_empty() {
                    s.snapshot.model = model;
                }
                if !effort.is_empty() {
                    s.snapshot.effort = effort;
                }
                effects.0.push(CoreEffect::Broadcast(s.update_message()));
            }
            SessionObservation::Transcript {
                path,
                agent_session_id,
                safe_offset,
                grew_at,
            } => {
                if safe_offset < 0 {
                    return Err(SessionError::InvalidRequest(
                        "negative transcript offset".into(),
                    ));
                }
                s.transcript.native_log_path = path.to_string_lossy().into_owned();
                s.transcript.agent_session_id = agent_session_id;
                s.transcript_offset = safe_offset;
                s.marker_source.resolved(path.clone());
                s.snapshot.transcript_grew_at = grew_at.clone();
                effects
                    .0
                    .push(CoreEffect::Notify(CoreEvent::TranscriptChanged {
                        binding,
                        path,
                        safe_offset,
                        grew_at,
                    }));
            }
            SessionObservation::Workflow(progress) => {
                s.workflow = Some(progress.clone());
                effects.0.push(CoreEffect::Broadcast(proto::Message {
                    r#type: "workflow_progress".into(),
                    session_id: id.0,
                    workflow_progress: Some(progress.clone()),
                    ..Default::default()
                }));
                effects
                    .0
                    .push(CoreEffect::Notify(CoreEvent::WorkflowChanged {
                        binding,
                        progress,
                    }));
            }
            SessionObservation::Subagents(tree) => {
                s.subagents = (!tree.nodes.is_empty()).then(|| tree.clone());
                effects.0.push(CoreEffect::Broadcast(proto::Message {
                    r#type: "subagent_tree".into(),
                    session_id: id.0,
                    subagent_tree: Some(tree.clone()),
                    ..Default::default()
                }));
                effects
                    .0
                    .push(CoreEffect::Notify(CoreEvent::SubagentsChanged {
                        binding,
                        tree,
                    }));
            }
            SessionObservation::Done(summary) => {
                effects
                    .0
                    .extend(s.publish_completion(summary, true, now)?.0);
            }
            SessionObservation::CrossSessionMessage(_) => {
                unreachable!("handled before receiver borrow")
            }
            SessionObservation::Relays(relays) => {
                s.snapshot.relays = relays.into_iter().map(Some).collect();
                effects.0.push(CoreEffect::Broadcast(s.update_message()));
            }
            SessionObservation::BoardNotifyPending(pending) => {
                s.snapshot.board_notify_pending = pending;
                effects.0.push(CoreEffect::Broadcast(s.update_message()));
            }
        }
        Ok(state.route(effects))
    }
    pub fn reset_history(
        &self,
        id: LiveSessionId,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError> {
        self.reset_history_authorized(None, id, now)
    }
    pub fn reset_history_from_ui(
        &self,
        ui: UiBinding,
        id: LiveSessionId,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError> {
        self.reset_history_authorized(Some(ui), id, now)
    }
    fn reset_history_authorized(
        &self,
        ui: Option<UiBinding>,
        id: LiveSessionId,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError> {
        timestamp(now)?;
        let mut state = lock(&self.state);
        if ui.is_some_and(|ui| !state.authorized(ui)) {
            return Err(SessionError::AuthenticationExpired);
        }
        let ids: Vec<_> = if id.0 > 0 {
            if !state.sessions.contains_key(&id) {
                return Err(SessionError::NotFound(id));
            }
            vec![id]
        } else {
            state.sessions.keys().copied().collect()
        };
        let mut effects = CoreEffects::default();
        let mut updates = Vec::new();
        for id in ids {
            let s = state.sessions.get_mut(&id).expect("selected session");
            s.replay.reset();
            s.vt.reset();
            s.snapshot.first_message.clear();
            s.snapshot.last_message.clear();
            s.native_tail.clear();
            s.approval_reservation = None;
            s.marker_suppression.reset_history();
            effects.0.extend(s.approval.reset_history(now).0);
            if let Some(update) = s.refresh_approval_activity() {
                effects.0.push(CoreEffect::Broadcast(update));
            }
            effects
                .0
                .push(CoreEffect::Persist(PersistenceEffect::ClearSessionHistory(
                    id,
                )));
            effects.0.push(history(
                id,
                now,
                "session_history_reset",
                JsonObject::new(),
            )?);
            updates.push(s.update_message());
            effects
                .0
                .push(CoreEffect::Notify(CoreEvent::HistoryReset(id)));
        }
        effects.0.push(CoreEffect::Broadcast(proto::Message {
            r#type: "session_history_reset".into(),
            session_id: id.0,
            ..Default::default()
        }));
        effects
            .0
            .extend(updates.into_iter().map(CoreEffect::Broadcast));
        Ok(state.route(effects))
    }
    /// Atomic source PATCH semantics, including the inherited invalid-color
    /// partial-memory update. No callback, persistence or UI publication under State.
    pub fn patch_card_meta(
        &self,
        id: LiveSessionId,
        patch: SessionCardMetaPatch,
    ) -> Result<SessionCardMetaUpdate, SessionError> {
        if patch.label.is_none()
            && patch.pinned.is_none()
            && patch.color.is_none()
            && patch.note.is_none()
        {
            return Err(SessionError::InvalidRequest(
                "at least one meta field is required".into(),
            ));
        }
        let normalize = |text: &str, cap: usize| -> String {
            text.chars()
                .filter(|c| !c.is_control())
                .collect::<String>()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(cap)
                .collect()
        };
        let mut state = lock(&self.state);
        let session = state
            .sessions
            .get_mut(&id)
            .ok_or(SessionError::NotFound(id))?;
        if let Some(label) = patch.label {
            session.snapshot.label = normalize(&label, 120);
        }
        if let Some(pinned) = patch.pinned {
            session.snapshot.pinned = pinned;
        }
        if let Some(color) = patch.color {
            let color = color.trim().to_lowercase();
            if !matches!(
                color.as_str(),
                "" | "blue" | "green" | "orange" | "red" | "purple"
            ) {
                return Err(SessionError::InvalidRequest("invalid session color".into()));
            }
            session.snapshot.color = color;
        }
        if let Some(note) = patch.note {
            session.snapshot.note = normalize(&note, 160);
        }
        let meta = session.card_meta();
        let notification = session.card_update();
        let effects = state.route(CoreEffects(vec![CoreEffect::Persist(
            PersistenceEffect::CardMeta {
                session: id,
                meta: meta.clone(),
            },
        )]));
        Ok(SessionCardMetaUpdate {
            meta,
            effects,
            notification,
        })
    }
    pub fn update_card_meta(
        &self,
        id: LiveSessionId,
        meta: SessionCardMeta,
    ) -> Result<CoreEffects, SessionError> {
        let mut state = lock(&self.state);
        let s = state
            .sessions
            .get_mut(&id)
            .ok_or(SessionError::NotFound(id))?;
        s.snapshot.label = meta.label.clone();
        s.snapshot.pinned = meta.pinned;
        s.snapshot.color = meta.color.clone();
        s.snapshot.note = meta.note.clone();
        s.snapshot.auto_title = meta.auto_title.clone();
        let effects = CoreEffects(vec![
            CoreEffect::Persist(PersistenceEffect::CardMeta { session: id, meta }),
            CoreEffect::Broadcast(s.card_update()),
        ]);
        Ok(state.route(effects))
    }
    pub fn evaluate_idle(&self, now: Timestamp) -> CoreEffects {
        let mut questions = self.open_idle_text_questions(now);
        let mut state = lock(&self.state);
        let mut effects = CoreEffects::default();
        for s in state.sessions.values_mut() {
            if terminal(&s.snapshot.state) {
                continue;
            }
            let before = s.snapshot.activity.clone();
            let old = s.snapshot.state.clone();
            s.snapshot.activity.output_idle = s.last_output.is_none_or(|at| {
                now.duration_since(at).unwrap_or_default() >= self.options.idle_after
            });
            s.refresh_awaiting();
            s.snapshot.activity.workflow_active =
                !s.snapshot.activity.output_idle && !s.snapshot.activity.awaiting_user;
            s.snapshot.activity.normalize();
            s.snapshot.state = s.snapshot.activity.display_state().into();
            if old == "running" && s.snapshot.state == "standby" {
                match s.schedule_fallback(now) {
                    Ok(end) => effects.0.extend(end.0),
                    Err(error) => self.warn("completion fallback scheduling failed", &error),
                }
            }
            if old != s.snapshot.state || before != s.snapshot.activity {
                effects.0.push(CoreEffect::Broadcast(s.activity_update()));
                effects.0.push(CoreEffect::Notify(CoreEvent::SessionState {
                    binding: s.binding,
                    activity: s.snapshot.activity.clone(),
                    display_state: s.snapshot.state.clone(),
                }));
            }
        }
        questions.0.extend(state.route(effects).0);
        questions
    }
}
impl State {
    fn record_cross_message(
        &mut self,
        binding: SessionBinding,
        sender: &str,
        line: &str,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError> {
        let receiver = self.session(binding)?;
        if receiver.snapshot.orchestration_id.0.is_empty() {
            return Ok(CoreEffects::default());
        }
        let receiver_id = receiver.snapshot.id;
        let receiver_role = receiver.snapshot.role.clone();
        let conductor_id = if receiver.snapshot.parent_session_id.0 != 0 {
            receiver.snapshot.parent_session_id
        } else {
            receiver_id
        };
        let Some(conductor) = self.sessions.get_mut(&conductor_id) else {
            return Ok(CoreEffects::default());
        };
        if conductor.snapshot.orchestration_id.0.is_empty() {
            return Ok(CoreEffects::default());
        }
        let sender = mask_secrets(sender);
        let text = mask_secrets(line);
        if conductor
            .snapshot
            .cross_session_messages
            .last()
            .is_some_and(|last| {
                last.receiver_session_id == receiver_id.0
                    && last.sender == sender
                    && last.text == text
            })
        {
            return Ok(CoreEffects::default());
        }
        let message = proto::CrossSessionMessage {
            at: proto::time::format_rfc3339(now).map_err(|_| {
                SessionError::InvalidRequest("timestamp outside RFC3339 range".into())
            })?,
            receiver_session_id: receiver_id.0,
            receiver_role,
            sender,
            text,
        };
        let messages = &mut conductor.snapshot.cross_session_messages;
        messages.push(message.clone());
        if messages.len() > 50 {
            messages.drain(..messages.len() - 50);
        }
        Ok(CoreEffects(vec![
            CoreEffect::Broadcast(conductor.update_message()),
            CoreEffect::Notify(CoreEvent::CrossSessionMessage {
                binding: conductor.binding,
                message,
            }),
        ]))
    }
}
impl Session {
    pub(super) fn observe_marker(
        &mut self,
        marker: Option<marker::Marker>,
        source: &str,
        now: Timestamp,
    ) -> Result<(CoreEffects, Option<SessionError>), SessionError> {
        let mut effects = CoreEffects::default();
        match self.marker_suppression.evaluate(marker.as_ref(), now) {
            MarkerSuppressionDecision::Empty => Ok((effects, None)),
            MarkerSuppressionDecision::Valid => {
                let marker = marker.expect("valid admission requires a marker");
                let candidate = identity::marker_candidate(
                    &self.snapshot.provider,
                    &marker.block,
                    self.approval.epoch(),
                );
                effects.0.extend(
                    self.approval
                        .observe(
                            ApprovalRecordData {
                                candidate,
                                sig: marker.sig,
                                origin: "marker".into(),
                                source: source.into(),
                                kind: "marker".into(),
                                block: marker.block,
                                question: String::new(),
                                context: String::new(),
                                options: Vec::new(),
                                summary: Default::default(),
                                detected_at: now,
                            },
                            None,
                            now,
                        )
                        .0,
                );
                Ok((effects, None))
            }
            MarkerSuppressionDecision::Suppressed {
                reason,
                new_signature,
                notify,
                lines,
            } => {
                let marker = marker.expect("suppressed admission requires a marker");
                let warning = new_signature.then(|| {
                    let short_sig: String = marker.sig.chars().take(10).collect();
                    SessionError::InvalidRequest(format!(
                        "session_id={} provider={} reason={reason} sig={short_sig} lines={lines}",
                        self.binding.session.0, self.snapshot.provider
                    ))
                });
                if notify {
                    effects.0.push(CoreEffect::Broadcast(proto::Message {
                        r#type: "approval_marker_suppressed".into(),
                        session_id: self.binding.session.0,
                        provider: self.snapshot.provider.clone(),
                        approval_sig: marker.sig,
                        approval_source: source.into(),
                        reason: reason.into(),
                        detected_at: proto::time::format_rfc3339(now).map_err(|_| {
                            SessionError::InvalidRequest("invalid suppression timestamp".into())
                        })?,
                        ..Default::default()
                    }));
                }
                Ok((effects, warning))
            }
        }
    }
    pub(super) fn refresh_approval_activity(&mut self) -> Option<proto::Message> {
        let before = self.snapshot.activity.clone();
        let before_state = self.snapshot.state.clone();
        self.refresh_awaiting();
        self.snapshot.activity.normalize();
        if terminal(&self.snapshot.state) {
            return None;
        }
        self.snapshot.state = self.snapshot.activity.display_state().into();
        (before != self.snapshot.activity || before_state != self.snapshot.state)
            .then(|| self.update_message())
    }
    pub(super) fn refresh_awaiting(&mut self) {
        let awaiting = self.approval.record().is_some();
        self.snapshot.activity.awaiting_approval = awaiting;
        self.snapshot.activity.awaiting_user = awaiting;
    }
}

impl Session {
    fn apply_detected_model(
        &mut self,
        detected: crate::application::output_model::DetectedModel,
        only_if_empty: bool,
        route: &str,
        effects: &mut CoreEffects,
    ) {
        if only_if_empty && !self.snapshot.model.is_empty() {
            return;
        }
        let mut changed = false;
        if self.snapshot.model != detected.model {
            self.snapshot.route = route.to_owned();
            self.snapshot.model = detected.model;
            changed = true;
        }
        if !detected.effort.is_empty() && self.snapshot.effort != detected.effort {
            self.snapshot.effort = detected.effort;
            changed = true;
        }
        if changed {
            self.model_revision = self.model_revision.wrapping_add(1);
            self.initial_model_scan_done = true;
            effects.0.push(CoreEffect::Broadcast(self.update_message()));
        }
    }
}
#[cfg(test)]
#[path = "observer_tests.rs"]
mod observer_tests;
#[cfg(test)]
#[path = "output_model_tests.rs"]
mod output_model_tests;
