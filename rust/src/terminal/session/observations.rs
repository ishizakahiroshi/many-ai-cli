use super::*;
use crate::approval::{detect, identity, marker};
impl SessionEngine {
    pub fn observe_output(
        &self,
        binding: SessionBinding,
        chunk: OutputChunk,
        now: SystemTime,
    ) -> Result<CoreEffects, SessionError> {
        let stamp = timestamp(now)?;
        let mut state = lock(&self.state);
        let s = state.session(binding)?;
        if !s.connected {
            return Err(SessionError::StaleBinding);
        }
        let mut effects = CoreEffects::default();
        if self.journal.enabled() {
            let masked = crate::storage::mask_secret_bytes(&chunk.bytes);
            effects.0.push(history(binding.session,now,"pty_output",object(serde_json::json!({"data_b64":STANDARD.encode(&masked),"text":identity::strip_ansi(&proto::wire::go_utf8_lossy(&masked))})))?);
        }
        s.replay.append(&chunk.bytes);
        s.pty_bytes_seen = s.pty_bytes_seen.saturating_add(chunk.bytes.len() as i64);
        s.vt.write(&chunk.bytes);
        s.output_generation = s.output_generation.wrapping_add(1);
        s.last_output = Some(now);
        s.snapshot.last_output_at = stamp;
        let eligible = !s.usage_probe
            && !s.custom_provider
            && s.snapshot.provider != "shell"
            && !terminal(&s.snapshot.state)
            && s.resize_debounce.is_none_or(|until| now > until);
        if eligible
            && !s.marker_source.is_transcript(&s.snapshot.provider)
            && let Some(marker) = marker::extract_vt(&s.vt)
            && marker::classify(&marker.block).is_empty()
        {
            let candidate =
                identity::marker_candidate(&s.snapshot.provider, &marker.block, s.approval.epoch());
            effects.0.extend(
                s.approval
                    .observe(
                        ApprovalRecordData {
                            candidate,
                            sig: marker.sig,
                            origin: "marker".into(),
                            source: "go_vt".into(),
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
                let phrases = self
                    .options
                    .approval_phrases
                    .get(&s.snapshot.provider)
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                if let Some(native) = detect::detect_native(&s.snapshot.provider, &lines, phrases) {
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
        Ok(state.route(effects))
    }
    pub fn apply_observation(
        &self,
        binding: SessionBinding,
        observation: SessionObservation,
        now: SystemTime,
    ) -> Result<CoreEffects, SessionError> {
        timestamp(now)?;
        let mut state = lock(&self.state);
        let s = state.session(binding)?;
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
                s.subagents = Some(tree.clone());
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
                s.done = Some(summary.clone());
                effects.0.push(CoreEffect::Broadcast(proto::Message {
                    r#type: "done_summary".into(),
                    session_id: id.0,
                    done_summary: Some(summary.clone()),
                    ..Default::default()
                }));
                effects.0.push(CoreEffect::Notify(CoreEvent::Completed {
                    binding,
                    summary,
                    fallback: false,
                }));
            }
            SessionObservation::CrossSessionMessage(message) => {
                s.snapshot.cross_session_messages.push(message.clone());
                effects.0.push(CoreEffect::Broadcast(s.update_message()));
                effects
                    .0
                    .push(CoreEffect::Notify(CoreEvent::CrossSessionMessage {
                        binding,
                        message,
                    }));
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
        now: SystemTime,
    ) -> Result<CoreEffects, SessionError> {
        self.reset_history_authorized(None, id, now)
    }
    pub fn reset_history_from_ui(
        &self,
        ui: UiBinding,
        id: LiveSessionId,
        now: SystemTime,
    ) -> Result<CoreEffects, SessionError> {
        self.reset_history_authorized(Some(ui), id, now)
    }
    fn reset_history_authorized(
        &self,
        ui: Option<UiBinding>,
        id: LiveSessionId,
        now: SystemTime,
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
    pub fn evaluate_idle(&self, now: SystemTime) -> CoreEffects {
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
            if old != s.snapshot.state || before != s.snapshot.activity {
                effects.0.push(CoreEffect::Broadcast(s.activity_update()));
                effects.0.push(CoreEffect::Notify(CoreEvent::SessionState {
                    binding: s.binding,
                    activity: s.snapshot.activity.clone(),
                    display_state: s.snapshot.state.clone(),
                }));
            }
        }
        state.route(effects)
    }
}
impl Session {
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
