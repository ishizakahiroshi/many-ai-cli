use super::*;
impl SessionEngine {
    pub fn broadcast_ui(&self, message: proto::Message) -> CoreEffects {
        lock(&self.state).route(CoreEffects(vec![CoreEffect::Broadcast(message)]))
    }
    pub fn broadcast_git_turn(&self, event: GitTurnNotification) -> CoreEffects {
        lock(&self.state).route(CoreEffects(vec![CoreEffect::BroadcastGitTurn(event)]))
    }
    pub fn attach_ui(
        &self,
        ui: UiBinding,
        active: Option<LiveSessionId>,
        initial_size: Option<TerminalSize>,
    ) -> Result<UiPriming, SessionError> {
        let mut state = lock(&self.state);
        if state.auth_epoch != ui.auth_epoch {
            return Err(SessionError::AuthenticationExpired);
        }
        if state.uis.contains_key(&ui.connection) {
            return Err(SessionError::InvalidRequest(
                "UI connection is already attached".into(),
            ));
        }
        if let Some(size) = initial_size.filter(|s| s.cols > 0 && s.rows > 0) {
            state.last_ui_size = size;
        }
        state.uis.insert(
            ui.connection,
            UiState {
                binding: ui,
                active,
                sizes: BTreeMap::new(),
                priming: true,
                queued: Vec::new(),
                work: Arc::new(authorization::UiWorkState::default()),
            },
        );
        let mut frames = Vec::new();
        let mut sessions = Vec::new();
        for s in state.sessions.values_mut().filter(|s| !s.usage_probe) {
            sessions.push(s.snapshot.clone());
            if !s.replay.is_empty() {
                s.replay_epoch = replay::next_replay_epoch(s.replay_epoch);
                frames.push(proto::Message {
                    r#type: "pty_data".into(),
                    session_id: s.binding.session.0,
                    data: s.replay.ui_snapshot(
                        active == Some(s.binding.session),
                        s.vt.alt_screen(),
                        USER_TURN_MARKER,
                    ),
                    replay: true,
                    replay_epoch: s.replay_epoch.0,
                    approval_source_epoch: s.approval.epoch().0,
                    ..Default::default()
                });
                frames.push(s.replay_done());
            }
        }
        for s in state.sessions.values().filter(|s| !s.usage_probe) {
            if let Some(workflow) = &s.workflow {
                frames.push(proto::Message {
                    r#type: "workflow_progress".into(),
                    session_id: s.binding.session.0,
                    workflow_progress: Some(workflow.clone()),
                    ..Default::default()
                });
            }
        }
        for s in state.sessions.values().filter(|s| !s.usage_probe) {
            if let Some(tree) = &s.subagents {
                frames.push(proto::Message {
                    r#type: "subagent_tree".into(),
                    session_id: s.binding.session.0,
                    subagent_tree: Some(tree.clone()),
                    ..Default::default()
                });
            }
        }
        frames.push(proto::Message {
            r#type: "approval_snapshot".into(),
            approval_snapshot: state
                .sessions
                .values()
                .filter(|s| !s.usage_probe)
                .map(|s| s.approval.wire_snapshot())
                .collect(),
            ..Default::default()
        });
        Ok(UiPriming {
            hub_instance: self.options.hub_instance.clone(),
            sessions,
            ordered_frames: frames,
        })
    }
    pub fn finish_ui_priming(&self, ui: UiBinding) -> Result<CoreEffects, SessionError> {
        let mut state = lock(&self.state);
        if !state.authorized(ui) {
            return Err(SessionError::AuthenticationExpired);
        }
        let u = state.uis.get_mut(&ui.connection).expect("authorized UI");
        if u.queued.is_empty() {
            u.priming = false;
            return Ok(CoreEffects::default());
        }
        // Keep priming until the caller applies this batch and an empty drain
        // atomically marks live. Concurrent traffic queues behind this batch.
        Ok(CoreEffects(
            std::mem::take(&mut u.queued)
                .into_iter()
                .map(|frame| match frame {
                    UiFrame::Message(message) => CoreEffect::SendUi {
                        binding: ui,
                        message: *message,
                    },
                    UiFrame::GitTurn(event) => CoreEffect::SendUiGitTurn {
                        binding: ui,
                        event,
                        best_effort: false,
                    },
                })
                .collect(),
        ))
    }
    pub fn detach_ui(&self, ui: UiBinding) -> CoreEffects {
        let mut state = lock(&self.state);
        if !state
            .uis
            .get(&ui.connection)
            .is_some_and(|u| u.binding == ui)
        {
            return CoreEffects::default();
        }
        let retired = state.uis.remove(&ui.connection).expect("validated UI");
        state
            .retired_uis
            .insert((ui.connection, ui.auth_epoch), retired.work);
        for s in state.sessions.values_mut() {
            if s.controlling_ui == Some(ui.connection) {
                s.controlling_ui = None;
            }
        }
        CoreEffects(vec![CoreEffect::DrainUi(ui), CoreEffect::CloseUi(ui)])
    }
    pub fn invalidate_all_ui(&self) -> CoreEffects {
        let mut state = lock(&self.state);
        state.auth_epoch.0 = state.auth_epoch.0.wrapping_add(1);
        let uis = std::mem::take(&mut state.uis);
        for s in state.sessions.values_mut() {
            s.controlling_ui = None;
        }
        let mut effects = CoreEffects::default();
        for retired in uis.into_values() {
            let ui = retired.binding;
            state
                .retired_uis
                .insert((ui.connection, ui.auth_epoch), retired.work);
            effects.0.push(CoreEffect::DrainUi(ui));
            effects.0.push(CoreEffect::CloseUi(ui));
        }
        effects
    }
    pub fn claim_ui_session(
        &self,
        ui: UiBinding,
        id: LiveSessionId,
        size: Option<TerminalSize>,
        now: SystemTime,
    ) -> Result<CoreEffects, SessionError> {
        let mut state = lock(&self.state);
        if !state.authorized(ui) {
            return Err(SessionError::AuthenticationExpired);
        }
        if !state.sessions.contains_key(&id) {
            return Err(SessionError::NotFound(id));
        }
        let u = state.uis.get_mut(&ui.connection).expect("authorized UI");
        u.active = Some(id);
        if let Some(size) = size.filter(|s| s.cols > 0 && s.rows > 0) {
            u.sizes.insert(id, size);
        }
        let size = u.sizes.get(&id).copied();
        state
            .sessions
            .get_mut(&id)
            .expect("existing session")
            .controlling_ui = Some(ui.connection);
        if let Some(size) = size {
            state.last_ui_size = size;
            let effects = resize_session(
                state.sessions.get_mut(&id).expect("existing session"),
                size,
                now,
            )?
            .1;
            Ok(state.route(effects))
        } else {
            Ok(CoreEffects::default())
        }
    }
    pub fn resize(
        &self,
        ui: UiBinding,
        id: LiveSessionId,
        size: TerminalSize,
        now: SystemTime,
    ) -> (ResizeOutcome, CoreEffects) {
        if size.cols <= 0 || size.rows <= 0 {
            return (ResizeOutcome::InvalidSize, CoreEffects::default());
        }
        let mut state = lock(&self.state);
        if !state.authorized(ui) {
            return (ResizeOutcome::StaleUi, CoreEffects::default());
        }
        state
            .uis
            .get_mut(&ui.connection)
            .expect("authorized UI")
            .sizes
            .insert(id, size);
        let Some(s) = state.sessions.get_mut(&id) else {
            return (ResizeOutcome::MissingSession, CoreEffects::default());
        };
        if let Some(owner) = s.controlling_ui {
            if owner != ui.connection {
                return (ResizeOutcome::NotController, CoreEffects::default());
            }
        } else {
            s.controlling_ui = Some(ui.connection);
        }
        let result = resize_session(s, size, now);
        state.last_ui_size = size;
        match result {
            Ok((outcome, effects)) => (outcome, state.route(effects)),
            Err(error) => {
                drop(state);
                self.warn("resize history timestamp", &error);
                (ResizeOutcome::InvalidSize, CoreEffects::default())
            }
        }
    }
    pub fn resync_approval(
        &self,
        ui: UiBinding,
        id: LiveSessionId,
        _now: SystemTime,
    ) -> Result<CoreEffects, SessionError> {
        let mut state = lock(&self.state);
        if !state.authorized(ui) {
            return Err(SessionError::AuthenticationExpired);
        }
        let s = state.sessions.get(&id).ok_or(SessionError::NotFound(id))?;
        if s.usage_probe {
            return Ok(CoreEffects::default());
        }
        let message = proto::Message {
            r#type: "approval_state".into(),
            session_id: id.0,
            provider: s.snapshot.provider.clone(),
            approval_state: Some(proto::ApprovalState {
                version: s.approval.version().0,
                open: s
                    .approval
                    .record()
                    .map(crate::approval::record::wire_record),
                close: None,
            }),
            ..Default::default()
        };
        let target = state.uis.get_mut(&ui.connection).expect("authorized UI");
        if target.priming {
            target.queued.push(UiFrame::Message(Box::new(message)));
            Ok(CoreEffects::default())
        } else {
            Ok(CoreEffects(vec![CoreEffect::SendUi {
                binding: ui,
                message,
            }]))
        }
    }

    pub fn consume_approval(
        &self,
        ui: UiBinding,
        message: proto::Message,
        now: SystemTime,
    ) -> Result<CoreEffects, SessionError> {
        let mut state = lock(&self.state);
        if !state.authorized(ui) {
            return Err(SessionError::AuthenticationExpired);
        }
        let s = state
            .sessions
            .get_mut(&LiveSessionId(message.session_id))
            .ok_or(SessionError::NotFound(LiveSessionId(message.session_id)))?;
        if message.approval_sig.is_empty() && message.approval_candidate_key.is_empty() {
            return Ok(CoreEffects::default());
        }
        let record = s.approval.record().cloned();
        let key = if !message.approval_candidate_key.is_empty() {
            message.approval_candidate_key.clone()
        } else if let Some(record) = &record {
            if record.data().sig == message.approval_sig {
                record.data().candidate.key.clone()
            } else {
                message.approval_sig.clone()
            }
        } else {
            message.approval_sig.clone()
        };
        let current = s.approval.epoch();
        let reported = ApprovalSourceEpoch(message.approval_source_epoch);
        if reported.0 != 0 && reported != current {
            let previous = reported.0.checked_add(1) == Some(current.0);
            let active_same = record.as_ref().is_some_and(|r| {
                r.data().candidate.key == key && r.data().candidate.source_epoch == reported
            });
            if !previous || (record.is_some() && !active_same) {
                return Ok(CoreEffects::default());
            }
        }
        let epoch = if reported.0 == 0 { current } else { reported };
        let shape = record
            .as_ref()
            .filter(|r| r.data().candidate.key == key)
            .map(|r| r.data().candidate.shape.clone())
            .unwrap_or_default();
        s.approval.mark_consumed(CandidateIdentity {
            key: key.clone(),
            shape,
            source_epoch: epoch,
        });
        let matches = record.as_ref().is_some_and(|record| {
            let r = record.data();
            let same = r.candidate.key == key && r.candidate.source_epoch == epoch;
            (r.origin == "native" && (r.sig == message.approval_sig || same))
                || (r.origin == "marker" && same)
        });
        let mut effects = if matches {
            s.approval
                .close(ApprovalCloseReason::Answered, &message.sent_text, now)
        } else if !message.approval_sig.is_empty() {
            CoreEffects(vec![CoreEffect::Persist(
                PersistenceEffect::ApprovalConsumed {
                    session: s.binding.session,
                    sig: message.approval_sig,
                    selected_text: message.sent_text,
                    resolved_at: now,
                },
            )])
        } else {
            CoreEffects::default()
        };
        if matches {
            s.refresh_awaiting();
            if !terminal(&s.snapshot.state) {
                s.snapshot.state = s.snapshot.activity.display_state().into();
                effects.0.push(CoreEffect::Broadcast(s.update_message()));
            }
        }
        Ok(state.route(effects))
    }
}
fn resize_session(
    s: &mut Session,
    size: TerminalSize,
    now: SystemTime,
) -> Result<(ResizeOutcome, CoreEffects), SessionError> {
    if s.size == size {
        return Ok((ResizeOutcome::Duplicate, CoreEffects::default()));
    }
    // Validate before changing state so an unsupported clock leaves no partial resize.
    let history = history(
        s.binding.session,
        now,
        "pty_resize",
        object(serde_json::json!({"cols":size.cols,"rows":size.rows})),
    )?;
    s.size = size;
    s.vt.resize(size.cols as usize, size.rows as usize);
    s.resize_debounce = now.checked_add(Duration::from_millis(200));
    let message = proto::Message {
        r#type: "pty_resize".into(),
        session_id: s.binding.session.0,
        cols: size.cols,
        rows: size.rows,
        ..Default::default()
    };
    let mut effects = CoreEffects::default();
    if s.connected {
        effects.0.push(CoreEffect::SendWrapperBestEffort {
            binding: s.binding,
            message: message.clone(),
        });
    }
    effects.0.push(CoreEffect::Broadcast(message));
    effects.0.push(history);
    Ok((ResizeOutcome::Applied, effects))
}
