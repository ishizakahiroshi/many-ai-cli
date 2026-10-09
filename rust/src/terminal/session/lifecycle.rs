use super::*;
impl Session {
    fn spawn_correlation(&self, hub_instance: &str) -> Option<SpawnCorrelationNotification> {
        if self.snapshot.client_request_id.is_empty() || self.usage_probe {
            return None;
        }
        Some(SpawnCorrelationNotification {
            hub_instance: hub_instance.into(),
            session_id: self.binding.session.0,
            started_at: self.snapshot.started_at.clone(),
            client_request_id: self.snapshot.client_request_id.clone(),
            ..Default::default()
        })
    }

    /// Go finalizeWorkflowOnSessionEnd publishes the timeout frame while
    /// deliberately consuming completion without sending a completion push.
    fn finalize_workflow(&mut self, effects: &mut CoreEffects) {
        let Some(progress) = self.workflow.as_mut().filter(|progress| {
            !progress.settled
                || progress.running != 0
                || progress.pending != 0
                || progress.waiting_dynamic != 0
        }) else {
            return;
        };
        progress.running = 0;
        progress.pending = 0;
        progress.waiting_dynamic = 0;
        if !progress.settled {
            progress.settled = true;
            progress.settled_by = "timeout".into();
        }
        effects.0.push(CoreEffect::Broadcast(proto::Message {
            r#type: "workflow_progress".into(),
            session_id: self.binding.session.0,
            workflow_progress: Some(progress.clone()),
            ..Default::default()
        }));
    }
}
impl SessionEngine {
    pub async fn register(
        &self,
        request: RegisterRequest,
        connection: WrapperConnectionId,
        now: Timestamp,
    ) -> Result<Registration, SessionError> {
        // Like Go registration, resolve the initial branch before publishing the
        // card, without holding the session-state or admission locks.
        let branch = (self.options.branch_lookup)(request.message.cwd.clone()).await;
        let _registration = self.registration.lock().await;
        let persistence_order = { lock(&self.state).persistence_lane.reserve() };
        persistence_order.wait_ordered().await;
        let m = request.message;
        // Native startup ownership matches the actual owned PID; ordinary
        // manual registrations retain Go's signed wire PID compatibility.
        let startup_pid = if request.spawn_proof.is_some() {
            Some(
                u32::try_from(m.pid)
                    .ok()
                    .filter(|pid| *pid != 0)
                    .ok_or_else(|| {
                        SessionError::InvalidRequest("invalid startup process identity".into())
                    })?,
            )
        } else {
            None
        };
        let started = timestamp(now)?;
        let (binding, size, lease, metadata) = {
            let mut state = lock(&self.state);
            // Allocation failure must neither consume a one-use proof nor leave
            // a newly acquired provider-spawn lease behind.
            let next_id = state
                .next_id
                .checked_add(1)
                .ok_or_else(|| SessionError::InvalidRequest("session identity exhausted".into()))?;
            let next_incarnation = state.next_incarnation.checked_add(1).ok_or_else(|| {
                SessionError::InvalidRequest("session incarnation exhausted".into())
            })?;
            let pending = if let Some(proof) = request.spawn_proof {
                let hash = crate::approval::identity::digest(&proof);
                let Some(lease) = state.spawn_proofs.get(&hash) else {
                    return Err(SessionError::InvalidRequest(
                        "invalid registration proof".into(),
                    ));
                };
                if lease.lease.provider != m.provider {
                    return Err(SessionError::InvalidRequest(
                        "invalid registration proof".into(),
                    ));
                }
                state
                    .spawn_proofs
                    .remove(&hash)
                    .expect("validated one-use proof")
            } else {
                PendingSpawn {
                    lease: state
                        .admission
                        .begin_provider_spawn(&m.provider)
                        .map_err(|_| SessionError::ProviderUpdating(m.provider.clone()))?,
                    metadata: SpawnRegistrationMetadata::default(),
                }
            };
            state.next_id = next_id;
            state.next_incarnation = next_incarnation;
            let size = if usable(state.last_ui_size) {
                state.last_ui_size
            } else if usable(TerminalSize {
                cols: m.cols,
                rows: m.rows,
            }) {
                TerminalSize {
                    cols: m.cols,
                    rows: m.rows,
                }
            } else {
                TerminalSize {
                    cols: 200,
                    rows: 50,
                }
            };
            (
                SessionBinding {
                    session: LiveSessionId(state.next_id),
                    incarnation: SessionIncarnation(state.next_incarnation),
                    wrapper: connection,
                },
                size,
                pending.lease,
                pending.metadata,
            )
        };
        let mut session =
            match self.initialize(binding, &m, &branch, size, &started, now, false, &metadata) {
                Ok(session) => session,
                Err(error) => {
                    lock(&self.state).admission.end_provider_spawn(&lease);
                    return Err(error);
                }
            };
        let snapshot = session.snapshot.clone();
        let registered = proto::Message {
            r#type: "registered".into(),
            session_id: binding.session.0,
            cols: size.cols,
            rows: size.rows,
            started_at: started,
            log_path: snapshot.log_path.clone(),
            jsonl_path: snapshot.jsonl_path.clone(),
            token_statusbar: self.options.token_statusbar || m.usage_probe,
            orchestration_id: metadata.orchestration.0.clone(),
            auto: metadata.auto,
            board_path: metadata.board_path.clone(),
            ..Default::default()
        };
        let mut effects = CoreEffects::default();
        let mut announce = session.update_message();
        announce.shell = m.shell.clone();
        announce.log_path = snapshot.log_path.clone();
        announce.jsonl_path = snapshot.jsonl_path.clone();
        effects.0.push(CoreEffect::Broadcast(announce));
        if let Some(event) = session.spawn_correlation(&self.options.hub_instance) {
            effects.0.push(CoreEffect::BroadcastSpawnCorrelation(event));
        }
        if !session.usage_probe {
            effects.0.push(history(binding.session,now,"session_start",object(serde_json::json!({"provider":m.provider,"cwd":m.cwd,"branch":session.snapshot.branch,"label":m.label,"model":m.model,"shell":m.shell,"pid":m.pid,"parent_session_id":metadata.parent.0,"role":metadata.role,"auto":metadata.auto,"orchestration_id":metadata.orchestration.0,"board_path":metadata.board_path,"subscription_profile_id":session.snapshot.subscription_profile_id})))?);
        }
        effects
            .0
            .push(CoreEffect::Notify(CoreEvent::Registered(binding)));
        session.replay_epoch = ReplayEpoch(1);
        let mut state = lock(&self.state);
        if let Err(error) = state.admission.register_spawn(
            &lease,
            binding.session,
            session.snapshot.parent_session_id,
        ) {
            state.admission.end_provider_spawn(&lease);
            drop(state);
            self.journal.close_writer(binding.session);
            return Err(SessionError::InvalidRequest(format!(
                "registration reservation failed: {error:?}"
            )));
        }
        state.sessions.insert(binding.session, session);
        let effects = state.route(effects);
        Ok(Registration {
            startup_receipt: startup_pid.map(|pid| {
                crate::process::wrapper_startup::SpawnRegistrationReceipt::proof_bound(
                    lease.id, binding, pid,
                )
            }),
            startup_metadata: metadata,
            binding,
            snapshot,
            registered,
            after_registered: effects,
        })
    }
    /// Optional journal/SQLite failures are observable warnings, not launch
    /// failures. This explicit initialization boundary performs all registration
    /// I/O outside State, before a wrapper ACK or any session publication.
    #[allow(clippy::too_many_arguments)]
    fn initialize(
        &self,
        binding: SessionBinding,
        m: &proto::Message,
        branch: &str,
        size: TerminalSize,
        started_text: &str,
        started: Timestamp,
        append: bool,
        metadata: &SpawnRegistrationMetadata,
    ) -> Result<Session, SessionError> {
        let paths =
            self.journal
                .paths_for_timestamp(binding.session, &m.provider, &m.cwd, started_text)?;
        // Fixed Go resolves the wrapper's actual profile ID and current display
        // name before initial or reattach publication. Missing profiles retain
        // their validated ID; invalid IDs never reach storage or the UI.
        let subscription_id = crate::config::normalize_subscription_id(&m.subscription_id);
        let (subscription_id, subscription_name) =
            if crate::config::validate_subscription_id(&subscription_id).is_ok() {
                let name = (self.options.subscription_name)(&m.provider, &subscription_id);
                (subscription_id, name)
            } else {
                (String::new(), String::new())
            };
        let start = SessionStart {
            live_session_id: binding.session,
            provider: m.provider.clone(),
            display: m.display_name.clone(),
            cwd: m.cwd.clone(),
            // Go attachStore writes this before optional history delivery, even
            // when session logging is disabled (the default).
            branch: branch.to_owned(),
            label: m.label.clone(),
            model: m.model.clone(),
            route: m.route.trim().into(),
            shell: m.shell.clone(),
            state: if append { "running" } else { "standby" }.into(),
            started_at: started_text.into(),
            log_path: paths.raw.to_string_lossy().into_owned(),
            jsonl_path: paths.jsonl.to_string_lossy().into_owned(),
            subscription_id: subscription_id.clone(),
            parent_session_id: metadata.parent,
            role: metadata.role.clone(),
            auto: metadata.auto,
            depth: metadata.depth,
            orchestration_id: metadata.orchestration.clone(),
            board_path: metadata.board_path.clone(),
            worktree_branch: metadata.worktree_branch.clone(),
        };
        let (paths, db_id, card) = self.journal.attach_session(
            binding,
            start,
            SessionCardMeta {
                label: m.label.clone(),
                ..Default::default()
            },
            started_text,
            append,
            !m.usage_probe,
        )?;
        let snapshot = SessionSnapshot {
            id: binding.session,
            provider: m.provider.clone(),
            provider_revision: m.provider_revision.clone(),
            display: m.display_name.clone(),
            cwd: m.cwd.clone(),
            branch: branch.to_owned(),
            label: card.label,
            launch_label: m.label.clone(),
            pinned: card.pinned,
            color: card.color,
            note: card.note,
            auto_title: card.auto_title,
            model: m.model.clone(),
            effort: m.effort.clone(),
            execution_mode: m.execution_mode.clone(),
            permission_mode: m.permission_mode.clone(),
            route: m.route.trim().into(),
            shell: m.shell.clone(),
            activity: proto::SessionActivity {
                output_idle: true,
                ..Default::default()
            },
            state: if append { "running" } else { "standby" }.into(),
            started_at: started_text.into(),
            client_request_id: metadata.client_request_id.clone(),
            subscription_profile_id: subscription_id,
            subscription_profile_name: subscription_name,
            log_path: paths.raw.to_string_lossy().into_owned(),
            jsonl_path: paths.jsonl.to_string_lossy().into_owned(),
            parent_session_id: metadata.parent,
            handoff_from: metadata.handoff_from,
            role: metadata.role.clone(),
            auto: metadata.auto,
            depth: metadata.depth,
            orchestration_id: metadata.orchestration.clone(),
            board_path: metadata.board_path.clone(),
            worktree_branch: metadata.worktree_branch.clone(),
            normal_worktree: metadata.normal_worktree.clone(),
            worktree_cleanup: metadata.worktree_cleanup.clone(),
            ..Default::default()
        };
        let mut input = InputState::default();
        if metadata.needs_initial_gate() {
            input.set_initial_gate(started);
        }
        Ok(Session {
            binding,
            snapshot,
            connected: true,
            pid: m.pid,
            db_id,
            git_root: None,
            branch_refresh: branch::RefreshState::default(),
            transcript: TranscriptSessionIdentity {
                provider: m.provider.clone(),
                cwd: m.cwd.clone(),
                started_at: started_text.into(),
                home_dir: m.home_dir.clone(),
                codex_home: m.codex_home.clone(),
                claude_dir: m.claude_dir.clone(),
                grok_home: m.grok_home.clone(),
                agent_session_id: m.agent_session_id.clone(),
                ..Default::default()
            },
            transcript_offset: 0,
            approval: ApprovalState::new(binding.session, m.provider.clone()),
            marker_source: TranscriptSource::default(),
            marker_suppression: MarkerSuppressionState::default(),
            approval_reservation: None,
            workflow: None,
            subagents: None,
            observer_turn_started_at: started,
            cross_message_screen_signature: String::new(),
            done: None,
            completion: Default::default(),
            replay: ReplayBuffer::default(),
            replay_epoch: ReplayEpoch(1),
            pty_bytes_seen: 0,
            vt: VtBuffer::new(size.cols.max(0) as usize, size.rows.max(0) as usize),
            size,
            resize_debounce: None,
            controlling_ui: None,
            input,
            input_lane: Arc::new(lane::InputLane::default()),
            awaiting_submit_enter: false,
            last_output: None,
            output_generation: 0,
            usage_probe: m.usage_probe,
            subscription_login: m.subscription_login,
            custom_provider: self.options.custom_providers.contains(&m.provider),
            native_tail: String::new(),
            usage: None,
            model_revision: 0,
            initial_model_scan_bytes: 0,
            initial_model_scan_done: false,
            ended: None,
        })
    }
    pub async fn reattach(
        &self,
        request: ReattachRequest,
        connection: WrapperConnectionId,
        now: Timestamp,
    ) -> Result<Reattachment, SessionError> {
        // Reject already-revoked immutable labels before waiting for registration
        // or ordered persistence. Terminal/skipped relays may supply no metadata.
        if lock(&self.state)
            .revoked_relay_children
            .contains(&request.message.label)
        {
            return Err(SessionError::InvalidRequest("session dismissed".into()));
        }
        let branch = (self.options.branch_lookup)(request.message.cwd.clone()).await;
        let _registration = self.registration.lock().await;
        let persistence_order = { lock(&self.state).persistence_lane.reserve() };
        persistence_order.wait_ordered().await;
        let m = request.message;
        let requested = LiveSessionId(m.session_id);
        if m.session_id <= 0 {
            return Err(SessionError::InvalidRequest("invalid session_id".into()));
        }
        if lock(&self.state).dismissed.contains(&requested) {
            return Err(SessionError::InvalidRequest("session dismissed".into()));
        }
        // Go's base64 decoder ignores CR/LF only.
        let encoded: Vec<u8> = m
            .replay_b64
            .bytes()
            .filter(|b| !matches!(b, b'\r' | b'\n'))
            .collect();
        let mut replay = STANDARD
            .decode(encoded)
            .map_err(|_| SessionError::InvalidRequest("invalid replay_b64".into()))?;
        if replay.len() > replay::REPLAY_LIMIT {
            replay.drain(..replay.len() - replay::REPLAY_LIMIT);
        }
        // Validate the Hub clock before acquiring a lease or rebinding the
        // journal. A valid wrapper timestamp does not validate this later clock
        // used by cold replay and the reattach history event.
        let reattached_at = timestamp(now)?;
        let (started, started_text) = match proto::time::parse_rfc3339(&m.started_at) {
            Ok(t) => (t, m.started_at.clone()),
            Err(_) => (now, reattached_at.clone()),
        };
        let (binding, lease) = {
            let mut state = lock(&self.state);
            if state.dismissed.contains(&requested) {
                return Err(SessionError::InvalidRequest("session dismissed".into()));
            }
            let collision = state.sessions.get(&requested).is_some_and(|s| {
                s.connected
                    && !(m.pid > 0
                        && s.pid == m.pid
                        && s.snapshot.provider == m.provider
                        && s.snapshot.cwd == m.cwd)
            });
            let id = if collision {
                state
                    .next_id
                    .checked_add(1)
                    .map(LiveSessionId)
                    .ok_or_else(|| {
                        SessionError::InvalidRequest("session identity exhausted".into())
                    })?
            } else {
                requested
            };
            state.next_id = state.next_id.max(id.0);
            let incarnation = if let Some(s) = state.sessions.get(&id) {
                s.binding.incarnation
            } else {
                state.next_incarnation =
                    state.next_incarnation.checked_add(1).ok_or_else(|| {
                        SessionError::InvalidRequest("session incarnation exhausted".into())
                    })?;
                SessionIncarnation(state.next_incarnation)
            };
            let lease = state
                .admission
                .begin_provider_spawn(&m.provider)
                .map_err(|_| SessionError::ProviderUpdating(m.provider.clone()))?;
            (
                SessionBinding {
                    session: id,
                    incarnation,
                    wrapper: connection,
                },
                lease,
            )
        };
        let size = TerminalSize {
            cols: m.cols,
            rows: m.rows,
        };
        // Preserve server-owned warm metadata before the storage upsert. Cold
        // metadata is supplied only by the relay identity resolver, not JSON.
        let metadata = {
            let state = lock(&self.state);
            state
                .sessions
                .get(&binding.session)
                .map(|old| SpawnRegistrationMetadata {
                    client_request_id: old.snapshot.client_request_id.clone(),
                    parent: old.snapshot.parent_session_id,
                    role: old.snapshot.role.clone(),
                    auto: old.snapshot.auto,
                    depth: old.snapshot.depth,
                    orchestration: old.snapshot.orchestration_id.clone(),
                    board_path: old.snapshot.board_path.clone(),
                    worktree_branch: old.snapshot.worktree_branch.clone(),
                    normal_worktree: old.snapshot.normal_worktree.clone(),
                    worktree_cleanup: old.snapshot.worktree_cleanup.clone(),
                    ..Default::default()
                })
                .or(request.restored_metadata)
                .unwrap_or_default()
        };
        {
            let mut state = lock(&self.state);
            if state.revoked_relay_children.contains(&m.label) {
                state.admission.end_provider_spawn(&lease);
                return Err(SessionError::InvalidRequest("session dismissed".into()));
            }
        }
        let initialized = self.initialize(
            binding,
            &m,
            &branch,
            size,
            &started_text,
            started,
            true,
            &metadata,
        );
        let mut session = match initialized {
            Ok(s) => s,
            Err(e) => {
                lock(&self.state).admission.end_provider_spawn(&lease);
                return Err(e);
            }
        };
        let mut state = lock(&self.state);
        if state.dismissed.contains(&requested) || state.revoked_relay_children.contains(&m.label) {
            state.admission.end_provider_spawn(&lease);
            drop(state);
            self.journal.close_writer(binding.session);
            return Err(SessionError::InvalidRequest("session dismissed".into()));
        }
        // The private pending lease blocks provider updates and cannot be
        // consumed elsewhere. Still handle admission errors before taking the
        // retained session; everything after this point is infallible.
        let parent = state
            .sessions
            .get(&binding.session)
            .map_or(session.snapshot.parent_session_id, |old| {
                old.snapshot.parent_session_id
            });
        if let Err(error) = state
            .admission
            .register_spawn(&lease, binding.session, parent)
        {
            state.admission.end_provider_spawn(&lease);
            return Err(SessionError::InvalidRequest(format!(
                "reattach reservation failed: {error:?}"
            )));
        }
        let mut effects = CoreEffects::default();
        let mut gap = replay.clone();
        if let Some(mut old) = state.sessions.remove(&binding.session) {
            gap = reattach_gap(&replay, m.pty_bytes, old.pty_bytes_seen).to_vec();
            old.input.disconnected(old.binding.wrapper, false);
            if old.connected {
                effects.0.push(CoreEffect::CloseWrapper {
                    binding: old.binding,
                });
            }
            old.replay.append(&gap);
            session.replay = old.replay;
            session.pty_bytes_seen = if m.pty_bytes > 0 {
                m.pty_bytes
            } else {
                old.pty_bytes_seen.saturating_add(gap.len() as i64)
            };
            session.replay_epoch = replay::next_replay_epoch(old.replay_epoch);
            if old.size == size {
                session.vt = old.vt;
                session.vt.write(&gap);
            } else {
                if old.vt.alt_screen() {
                    session.vt.write(replay::ALT_SCREEN_ENTER);
                }
                session.vt.write(&replay);
            }
            session.approval = old.approval;
            session.marker_source = old.marker_source;
            session.marker_suppression = old.marker_suppression;
            session.input = old.input;
            session.input_lane = old.input_lane;
            session.awaiting_submit_enter = old.awaiting_submit_enter;
            session.transcript.native_log_path = old.transcript.native_log_path;
            session.transcript_offset = old.transcript_offset;
            session.git_root = old.git_root;
            session.branch_refresh = old.branch_refresh;
            session.workflow = old.workflow;
            session.subagents = old.subagents;
            session.observer_turn_started_at = old.observer_turn_started_at;
            session.cross_message_screen_signature = old.cross_message_screen_signature;
            session.done = old.done;
            session.completion = old.completion;
            session.last_output = old.last_output;
            preserve_warm_conversation(&mut session.snapshot, &old.snapshot);
            session.snapshot.effort = old.snapshot.effort;
            session.snapshot.execution_mode = old.snapshot.execution_mode;
            session.snapshot.permission_mode = old.snapshot.permission_mode;
            if !old.snapshot.provider_revision.is_empty() {
                session.snapshot.provider_revision = old.snapshot.provider_revision;
            }
        } else {
            session.replay.append(&replay);
            session.pty_bytes_seen = replay.len() as i64;
            session.vt.write(&replay);
            session.snapshot.activity.output_idle = replay.is_empty();
            session.snapshot.activity.workflow_active = !replay.is_empty();
            if !replay.is_empty() {
                session.last_output = Some(now);
                session.snapshot.last_output_at = reattached_at.clone();
            }
        }
        // Go reattach restarts the two-second timer while retaining warm
        // project/stats latches, including a same-ID replacement's saved state.
        session.branch_refresh.checked_at = Some(now);
        session
            .input
            .observe_high_watermark(InputSeq(m.input_seq_high_watermark));
        let snapshot = session.snapshot.clone();
        let reattached = proto::Message {
            r#type: "reattach_ack".into(),
            session_id: binding.session.0,
            ..Default::default()
        };
        let mut announce = session.update_message();
        announce.shell = m.shell.clone();
        announce.log_path = snapshot.log_path.clone();
        announce.jsonl_path = snapshot.jsonl_path.clone();
        effects.0.push(CoreEffect::Broadcast(announce));
        if let Some(event) = session.spawn_correlation(&self.options.hub_instance) {
            effects.0.push(CoreEffect::BroadcastSpawnCorrelation(event));
        }
        if !gap.is_empty() {
            effects.0.push(CoreEffect::Broadcast(proto::Message {
                r#type: "pty_data".into(),
                session_id: binding.session.0,
                data: gap,
                replay: true,
                replay_epoch: session.replay_epoch.0,
                approval_source_epoch: session.approval.epoch().0,
                ..Default::default()
            }));
        }
        effects.0.push(CoreEffect::Broadcast(session.replay_done()));
        if !m.usage_probe {
            effects.0.push(history_at(binding.session,reattached_at,"session_reattach",object(serde_json::json!({"old_session_id":requested.0,"provider":m.provider,"cwd":m.cwd,"branch":session.snapshot.branch,"label":m.label,"model":m.model,"shell":m.shell,"pid":m.pid,"renumbered":binding.session!=requested}))));
        }
        effects
            .0
            .push(CoreEffect::Notify(CoreEvent::Registered(binding)));
        state.sessions.insert(binding.session, session);
        effects
            .0
            .push(CoreEffect::Notify(CoreEvent::Reattached(binding)));
        Ok(Reattachment {
            binding,
            snapshot,
            reattached,
            after_reattached: state.route(effects),
        })
    }
    pub fn observe_end(
        &self,
        binding: SessionBinding,
        end: SessionEnd,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError> {
        let mut state = lock(&self.state);
        let s = state.session(binding)?;
        if s.ended.is_some() {
            return Ok(CoreEffects::default());
        }
        let code = end.exit_code;
        let mut body = object(serde_json::json!({"state":end.declared_state,"exit_code":code}));
        if !end.reason.is_empty() {
            body.insert("reason".into(), end.reason.clone().into());
        }
        let mut effects = CoreEffects(vec![history(binding.session, now, "session_end", body)?]);
        if matches!(end.declared_state.as_str(), "completed" | "error") {
            s.snapshot.state = end.declared_state.clone();
            if !end.reason.is_empty() {
                s.snapshot.end_reason = end.reason.clone();
            }
        }
        s.ended = Some(end.clone());
        s.finalize_workflow(&mut effects);
        effects
            .0
            .push(CoreEffect::Notify(CoreEvent::Ended { binding, end }));
        Ok(state.route(effects))
    }
    pub fn disconnected(
        &self,
        binding: SessionBinding,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError> {
        let mut state = lock(&self.state);
        let Some(s) = state.sessions.get_mut(&binding.session) else {
            return Ok(CoreEffects::default());
        };
        if s.binding != binding {
            if s.binding.incarnation == binding.incarnation {
                s.input.disconnected(binding.wrapper, false);
            }
            return Ok(CoreEffects::default());
        }
        if !s.connected {
            return Ok(CoreEffects::default());
        }
        s.connected = false;
        s.usage = None;
        s.input.disconnected(binding.wrapper, false);
        if !s.input.ack_capable() {
            s.input.take_pending();
        }
        if !terminal(&s.snapshot.state) {
            s.snapshot.state = "disconnected".into();
        }
        let mut effects = s.approval.close(ApprovalCloseReason::SessionEnd, "", now);
        s.finalize_workflow(&mut effects);
        s.approval_reservation = None;
        effects
            .0
            .push(CoreEffect::Persist(PersistenceEffect::EndSession {
                binding,
                state: s.snapshot.state.clone(),
                reason: s.snapshot.end_reason.clone(),
                ended_at: now,
            }));
        effects.0.push(CoreEffect::Broadcast(proto::Message {
            r#type: "session_end".into(),
            session_id: binding.session.0,
            state: s.snapshot.state.clone(),
            reason: s.snapshot.end_reason.clone(),
            ..Default::default()
        }));
        if s.ended.is_none() {
            effects.0.push(CoreEffect::Notify(CoreEvent::Ended {
                binding,
                end: SessionEnd {
                    declared_state: s.snapshot.state.clone(),
                    exit_code: 0,
                    reason: s.snapshot.end_reason.clone(),
                },
            }));
        }
        Ok(state.route(effects))
    }
    /// Source orchestration.go markConductor changes the current session card
    /// and broadcasts only; it does not persist a second session record. Parent
    /// lifetime is session+incarnation, so a warm wrapper reconnect is permitted.
    pub fn mark_conductor(
        &self,
        binding: SessionBinding,
        orchestration: OrchestrationId,
        board_path: String,
    ) -> Result<CoreEffects, SessionError> {
        self.mark_conductor_with_previous(binding, orchestration, board_path)
            .map(|(effects, _)| effects)
    }
    /// Capture the replaced card metadata under the same lock as the write.
    /// Startup compensation must use this receipt, rather than an earlier snapshot.
    pub fn mark_conductor_with_previous(
        &self,
        binding: SessionBinding,
        orchestration: OrchestrationId,
        board_path: String,
    ) -> Result<(CoreEffects, (OrchestrationId, String)), SessionError> {
        let mut state = lock(&self.state);
        let session = state
            .sessions
            .get_mut(&binding.session)
            .ok_or(SessionError::NotFound(binding.session))?;
        if session.binding.incarnation != binding.incarnation {
            return Err(SessionError::StaleBinding);
        }
        let previous = (
            session.snapshot.orchestration_id.clone(),
            session.snapshot.board_path.clone(),
        );
        if session.snapshot.orchestration_id == orchestration
            && session.snapshot.board_path == board_path
        {
            return Ok((CoreEffects::default(), previous));
        }
        session.snapshot.orchestration_id = orchestration;
        session.snapshot.board_path = board_path;
        let message = session.update_message();
        Ok((
            state.route(CoreEffects(vec![CoreEffect::Broadcast(message)])),
            previous,
        ))
    }
    /// A failed startup may restore its parent card only while the exact
    /// session incarnation and relay identity still match. A warm reconnect
    /// retains that owner; a newer incarnation or relay is left untouched.
    pub fn restore_conductor_if_matches(
        &self,
        binding: SessionBinding,
        expected_id: &OrchestrationId,
        expected_path: &str,
        previous_id: OrchestrationId,
        previous_path: String,
    ) -> Result<CoreEffects, SessionError> {
        let mut state = lock(&self.state);
        let Some(session) = state.sessions.get_mut(&binding.session) else {
            return Ok(CoreEffects::default());
        };
        if session.binding.incarnation != binding.incarnation
            || session.snapshot.orchestration_id != *expected_id
            || session.snapshot.board_path != expected_path
            || (session.snapshot.orchestration_id == previous_id
                && session.snapshot.board_path == previous_path)
        {
            return Ok(CoreEffects::default());
        }
        session.snapshot.orchestration_id = previous_id;
        session.snapshot.board_path = previous_path;
        let message = session.update_message();
        Ok(state.route(CoreEffects(vec![CoreEffect::Broadcast(message)])))
    }
    pub fn dismiss(&self, id: LiveSessionId, now: Timestamp) -> Result<CoreEffects, SessionError> {
        self.dismiss_authorized(None, None, id, now)
    }
    pub(crate) fn dismiss_bound(
        &self,
        binding: SessionBinding,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError> {
        self.dismiss_authorized(None, Some(binding), binding.session, now)
    }
    pub fn dismiss_from_ui(
        &self,
        ui: UiBinding,
        id: LiveSessionId,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError> {
        self.dismiss_authorized(Some(ui), None, id, now)
    }
    fn dismiss_authorized(
        &self,
        ui: Option<UiBinding>,
        expected: Option<SessionBinding>,
        id: LiveSessionId,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError> {
        let mut state = lock(&self.state);
        if ui.is_some_and(|ui| !state.authorized(ui)) {
            return Err(SessionError::AuthenticationExpired);
        }
        if expected.is_some_and(|binding| {
            state
                .sessions
                .get(&id)
                .is_none_or(|session| session.binding != binding)
        }) {
            return Err(SessionError::StaleBinding);
        }
        if state.confirmations.values().any(|c| c.pending.parent == id) {
            return Ok(
                state.route(CoreEffects(vec![CoreEffect::Broadcast(proto::Message {
                    r#type: "session_dismiss_refused".into(),
                    session_id: id.0,
                    ..Default::default()
                })])),
            );
        }
        let mut effects = CoreEffects::default();
        if let Some(mut s) = state.sessions.remove(&id) {
            if !s.connected {
                state.dismissed.insert(id);
            }
            effects
                .0
                .extend(s.approval.close(ApprovalCloseReason::SessionEnd, "", now).0);
            effects
                .0
                .push(CoreEffect::Persist(PersistenceEffect::EndSession {
                    binding: s.binding,
                    state: "dismissed".into(),
                    reason: String::new(),
                    ended_at: now,
                }));
            if s.connected {
                effects.0.push(CoreEffect::CancelSession {
                    binding: s.binding,
                    reason: StopReason::Dismissed,
                });
            }
        } else {
            state.dismissed.insert(id);
        }
        state.admission.dismiss_session(id);
        state.admission.release_parent(id);
        effects.0.push(CoreEffect::Broadcast(proto::Message {
            r#type: "session_removed".into(),
            session_id: id.0,
            ..Default::default()
        }));
        effects.0.push(CoreEffect::Notify(CoreEvent::Dismissed(id)));
        Ok(state.route(effects))
    }
    pub fn stop(
        &self,
        id: LiveSessionId,
        reason: StopReason,
        _now: Timestamp,
    ) -> Result<CoreEffects, SessionError> {
        let state = lock(&self.state);
        let s = state.sessions.get(&id).ok_or(SessionError::NotFound(id))?;
        Ok(CoreEffects(vec![CoreEffect::CancelSession {
            binding: s.binding,
            reason,
        }]))
    }
}
fn usable(size: TerminalSize) -> bool {
    size.cols >= 80 && size.rows >= 20
}
fn reattach_gap(replay: &[u8], total: i64, seen: i64) -> &[u8] {
    if replay.is_empty() || total <= 0 {
        return &[];
    }
    let missing = total.saturating_sub(seen);
    if missing <= 0 {
        &[]
    } else if missing >= replay.len() as i64 {
        replay
    } else {
        &replay[replay.len() - missing as usize..]
    }
}

/// Go reattach_state.go:252–283 keeps conversation metadata and orthogonal
/// activity across a socket replacement. The constructor's running connection
/// state and freshly resolved wire metadata are deliberately left alone.
fn preserve_warm_conversation(dst: &mut SessionSnapshot, old: &SessionSnapshot) {
    dst.client_request_id.clone_from(&old.client_request_id);
    dst.parent_session_id = old.parent_session_id;
    dst.handoff_from = old.handoff_from;
    dst.project_id.clone_from(&old.project_id);
    dst.role.clone_from(&old.role);
    dst.auto = old.auto;
    dst.depth = old.depth;
    dst.orchestration_id.clone_from(&old.orchestration_id);
    dst.board_path.clone_from(&old.board_path);
    dst.worktree_branch.clone_from(&old.worktree_branch);
    dst.normal_worktree.clone_from(&old.normal_worktree);
    dst.worktree_cleanup.clone_from(&old.worktree_cleanup);
    dst.board_notify_pending = old.board_notify_pending;
    dst.cross_session_messages
        .clone_from(&old.cross_session_messages);
    dst.activity.clone_from(&old.activity);
    dst.last_output_at.clone_from(&old.last_output_at);
    dst.transcript_grew_at.clone_from(&old.transcript_grew_at);
    if !old.started_at.is_empty() {
        dst.started_at.clone_from(&old.started_at);
    }
    dst.first_message.clone_from(&old.first_message);
    dst.last_message.clone_from(&old.last_message);
    dst.end_reason.clone_from(&old.end_reason);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warm_conversation_preserves_project_and_board_but_not_connection_metadata() {
        let old = SessionSnapshot {
            project_id: "existing-project".into(),
            board_notify_pending: true,
            state: "completed".into(),
            provider: "previous-provider".into(),
            label: "previous-card-label".into(),
            started_at: String::new(),
            ..Default::default()
        };
        let mut current = SessionSnapshot {
            state: "running".into(),
            provider: "wire-provider".into(),
            label: "restored-card-label".into(),
            started_at: "2026-01-02T03:04:05Z".into(),
            ..Default::default()
        };
        preserve_warm_conversation(&mut current, &old);
        assert_eq!(current.project_id, "existing-project");
        assert!(current.board_notify_pending);
        assert_eq!(current.state, "running");
        assert_eq!(current.provider, "wire-provider");
        assert_eq!(current.label, "restored-card-label");
        assert_eq!(current.started_at, "2026-01-02T03:04:05Z");
    }
}
