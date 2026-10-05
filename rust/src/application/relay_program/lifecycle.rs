use super::*;
use crate::orchestration::initial_prompt::InitialPromptOutcome;
impl RelayProgram {
    pub fn start(self: &Arc<Self>) -> Result<RelayGuard, SessionError> {
        self.core()?;
        if self.started.swap(true, std::sync::atomic::Ordering::AcqRel) {
            return Err(SessionError::InvalidRequest(
                "relay worker already started".into(),
            ));
        }
        let cancel = TaskCancellation::default();
        let token = cancel.clone();
        let weak = Arc::downgrade(self);
        let join = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(2));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {biased;_=token.token().cancelled()=>break,_=interval.tick()=>{let Some(owner)=weak.upgrade()else{break};if let Err(error)=owner.poll(Timestamp::now(),token.clone()).await{owner.warn("relay poll",error)}}}
            }
        });
        Ok(RelayGuard {
            cancel,
            join: Some(join),
        })
    }
    pub async fn restore(&self, now: Timestamp) -> Result<(), RelayError> {
        let (files, warnings) = self
            .store
            .load()
            .map_err(|e| RelayError::new(500, "relay_restore_error", e.to_string()))?;
        for warning in warnings {
            self.warn("relay restore skipped record", warning)
        }
        for file in files {
            self.core()?.revoke_relay_children(
                &OrchestrationId(file.orchestration_id.clone()),
                &file.revoked_child_labels,
                true,
            )?;
            if file.state == "completed" || (file.state == "stopped" && !file.resumable()) {
                continue;
            }
            let id = file.orchestration_id.clone();
            if self.owns(&id) {
                continue;
            }
            let mut run = Run::new(file, now);
            {
                let mut state = lock(&self.state);
                state.sequence += 1;
                run.sequence = state.sequence;
            }
            self.deps
                .orchestration
                .claim_relay_board(&OrchestrationId(id.clone()));
            let headless = run.file.roles.keys().any(|role| run.file.headless(role));
            if !run.file.terminal() {
                let message = if headless {
                    "hub restarted; a headless relay child cannot reattach"
                } else {
                    "hub restarted; waiting for the children to reattach"
                };
                run.file.event("restored", message.into(), now);
                self.board(&run, message, now);
                if !headless {
                    run.awaiting_reconnect = true;
                }
            }
            let handle = Arc::new(AsyncMutex::new(run));
            lock(&self.state).runs.insert(id, handle.clone());
            let mut run = handle.lock().await;
            if !run.file.terminal() && headless {
                self.finish(
                    &mut run,
                    "stopped",
                    "hub_restart",
                    "a headless relay child cannot reattach",
                    now,
                )
                .await;
            } else {
                self.save(&run).await;
            }
        }
        Ok(())
    }
    pub async fn resume_relay(
        &self,
        parent: LiveSessionId,
        id: &str,
        now: Timestamp,
        cancel: TaskCancellation,
    ) -> Result<proto::RelayStatus, RelayError> {
        let handle = self.resolve(parent, id, true)?;
        let core = self.core()?;
        let parent = core.details(parent).ok_or_else(RelayError::parent)?;
        let mut run = handle.lock().await;
        if run.parent_attached && run.file.parent_session_id != parent.binding.session.0 {
            return Err(RelayError::missing());
        }
        if run
            .cleanup_in_progress
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err(CleanupClaim::busy());
        }
        if run.awaiting_reconnect || !run.file.resumable() {
            return Err(RelayError::new(
                409,
                "relay_not_resumable",
                "relay is not in a resumable state (stopped by hub_restart / child_exited / timeout)",
            ));
        }
        if !self.owned_children(&run, &core).is_empty() {
            return Err(RelayError::new(
                409,
                "relay_children_active",
                "close prior relay child sessions before resuming",
            ));
        }
        if run.file.mode == "worktree" {
            if !Path::new(&run.file.worktree_path).is_dir() {
                return Err(RelayError::new(
                    409,
                    "relay_worktree_missing",
                    "relay worktree no longer exists; it cannot be resumed",
                ));
            }
            let parent_cwd = if run.file.parent_cwd.trim().is_empty() {
                &parent.snapshot.cwd
            } else {
                &run.file.parent_cwd
            };
            self.git
                .validate(
                    Path::new(parent_cwd),
                    Path::new(&run.file.worktree_path),
                    &run.file.branch,
                    &cancel,
                )
                .await
                .map_err(|e| {
                    RelayError::new(409, "spawn_error", format!("relay request failed: {e}"))
                })?;
        }
        let labels = ROLES
            .iter()
            .map(|role| run.file.child_label(role).to_owned())
            .filter(|label| !label.is_empty())
            .collect::<Vec<_>>();
        core.revoke_relay_children(
            &OrchestrationId(run.file.orchestration_id.clone()),
            &labels,
            false,
        )
        .map_err(|_| {
            RelayError::new(
                409,
                "relay_children_active",
                "close prior relay child sessions before resuming",
            )
        })?;
        for label in labels {
            if !run.file.revoked_child_labels.contains(&label) {
                run.file.revoked_child_labels.push(label);
            }
        }
        self.store.save(&run.file).map_err(|error| {
            RelayError::new(
                500,
                "relay_persist_error",
                format!("relay resume identity checkpoint failed: {error}"),
            )
        })?;
        let mut reservation = self.reserve(parent.binding.session, 2)?;
        run.admission = reservation.id.clone();
        let old = LiveSessionId(run.file.parent_session_id);
        if !run.parent_attached || old != parent.binding.session {
            run.file.parent_session_id = parent.binding.session.0;
            run.parent_attached = true;
            run.file.parent_started_at = parent.snapshot.started_at.clone();
            run.file.parent_provider = parent.snapshot.provider.clone();
            run.file.parent_cwd = parent.snapshot.cwd.clone();
            self.deps.orchestration.rebind_relay_parent(
                &OrchestrationId(run.file.orchestration_id.clone()),
                old,
                parent.binding.session,
            );
            self.effects(core.mark_conductor(
                parent.binding,
                OrchestrationId(run.file.orchestration_id.clone()),
                run.file.board_path.clone(),
            )?)
            .await?;
            self.board(
                &run,
                &format!(
                    "relay adopted by session={} (was #{})",
                    parent.binding.session.0, old.0
                ),
                now,
            );
        }
        self.deps.orchestration.register_conductor(
            parent.binding,
            &OrchestrationId(run.file.orchestration_id.clone()),
            Path::new(&run.file.board_path),
        )?;
        for role in ROLES {
            run.file.set_child(role, 0, String::new(), 0);
            run.file.set_baseline(role, 0);
        }
        run.reconnected.clear();
        run.nudged.clear();
        run.timers.clear();
        run.file.active_implementer = IMPLEMENTATION.into();
        run.file.round = 0;
        // The in-memory transition is nonterminal so its cancellation guard owns
        // cleanup. Until a child is registered, the durable image stays explicitly
        // resumable: an active zero-child record cannot reconnect after a crash.
        run.file.state = "implementing".into();
        run.file.reason.clear();
        run.file.updated_at = time::format_rfc3339(now).unwrap_or_default();
        let mut recovery = run.file.clone();
        recovery.state = "stopped".into();
        recovery.reason = "hub_restart".into();
        if let Err(error) = self.store.save(&recovery) {
            run.file.state = recovery.state;
            run.file.reason = recovery.reason;
            self.publish_meta(&run);
            return Err(RelayError::new(
                500,
                "relay_persist_error",
                format!("relay resume checkpoint failed: {error}"),
            ));
        }
        self.board(
            &run,
            &format!(
                "relay resume requested by session={} c={}",
                parent.binding.session.0,
                run.file.current_c()
            ),
            now,
        );
        self.publish_meta(&run);
        let prompt = run.file.prompts().resume_prompt();
        if let Err(error) = self
            .spawn(&mut run, IMPLEMENTATION, prompt, None, &cancel, now)
            .await
        {
            self.finish(
                &mut run,
                "stopped",
                failure_reason(&error),
                &error.to_string(),
                now,
            )
            .await;
            return Err(error);
        }
        run.file.event(
            "resumed",
            format!("resumed by session #{}", parent.binding.session.0),
            now,
        );
        self.transition(&mut run, "implementing", "", now).await;
        reservation.armed = false;
        Ok(run.file.status())
    }
    pub async fn cleanup_relay(
        &self,
        parent: LiveSessionId,
        id: &str,
        now: Timestamp,
        cancel: TaskCancellation,
    ) -> Result<proto::RelayStatus, RelayError> {
        if id.trim().is_empty() {
            return Err(RelayError::bad("orchestration_id is required"));
        }
        let handle = self.resolve(parent, id, false)?;
        let core = self.core()?;
        let (path, parent_cwd, ids, children, _cleanup_claim) = {
            let mut run = handle.lock().await;
            if !run.file.terminal() {
                return Err(RelayError::new(
                    409,
                    "relay_active",
                    "active relay worktree cannot be cleaned up",
                ));
            }
            if run.file.mode != "worktree" {
                return Err(RelayError::new(
                    400,
                    "relay_no_worktree",
                    "same-tree relay has no worktree to clean up",
                ));
            }
            if run.file.worktree_path.trim().is_empty() {
                return Err(RelayError::new(
                    409,
                    "relay_worktree_missing",
                    "relay worktree has already been cleaned up",
                ));
            }
            self.git
                .validate_cleanup(
                    Path::new(&run.file.parent_cwd),
                    Path::new(&run.file.worktree_path),
                    &self.cfg()?.orchestration,
                )
                .map_err(RelayError::bad)?;
            let claim = CleanupClaim::acquire(run.cleanup_in_progress.clone())?;
            let labels = ROLES
                .iter()
                .map(|role| run.file.child_label(role).to_owned())
                .filter(|label| !label.is_empty())
                .collect::<Vec<_>>();
            for label in &labels {
                if !run.file.revoked_child_labels.contains(label) {
                    run.file.revoked_child_labels.push(label.clone());
                }
            }
            self.store.save(&run.file).map_err(|error| {
                RelayError::new(
                    500,
                    "relay_persist_error",
                    format!("relay cleanup identity checkpoint failed: {error}"),
                )
            })?;
            let children = core.revoke_relay_children(
                &OrchestrationId(run.file.orchestration_id.clone()),
                &labels,
                true,
            )?;
            (
                run.file.worktree_path.clone(),
                run.file.parent_cwd.clone(),
                run.file.child_ids(),
                children,
                claim,
            )
        };
        for binding in &children {
            match core.dismiss_bound(*binding, now) {
                Ok(effects) => {
                    if let Err(error) = self.effects(effects).await {
                        self.warn("relay child cleanup", error)
                    }
                }
                Err(error) => self.warn("relay child cleanup", error),
            }
        }
        let mut run = handle.lock().await;
        let remaining = self.owned_children(&run, &core);
        if !remaining.is_empty() {
            return Err(RelayError::new(
                409,
                "relay_children_active",
                "relay child sessions are still present; close them before cleaning up the worktree",
            ));
        }
        if !run.file.terminal() || run.file.child_ids() != ids || run.file.parent_cwd != parent_cwd
        {
            return Err(RelayError::new(
                409,
                "relay_active",
                "relay changed while its worktree cleanup was waiting",
            ));
        }
        if run.file.worktree_path != path {
            return Err(RelayError::new(
                409,
                "relay_worktree_missing",
                "relay worktree has already been cleaned up",
            ));
        }
        if !Path::new(&path).is_dir() {
            return Err(RelayError::new(
                409,
                "relay_worktree_missing",
                "relay worktree is no longer present",
            ));
        }
        self.git
            .cleanup(Path::new(&parent_cwd), Path::new(&path), &cancel)
            .await
            .map_err(|e| {
                RelayError::new(
                    500,
                    "relay_cleanup_error",
                    format!("relay worktree cleanup failed: {e}"),
                )
            })?;
        run.file.worktree_path.clear();
        run.file.updated_at = time::format_rfc3339(now).unwrap_or_default();
        self.board(
            &run,
            &format!(
                "relay worktree cleaned path={path} branch={}",
                run.file.branch
            ),
            now,
        );
        self.save(&run).await;
        Ok(run.file.status())
    }
    pub async fn poll(&self, now: Timestamp, cancel: TaskCancellation) -> Result<(), RelayError> {
        let handles = lock(&self.state).runs.values().cloned().collect::<Vec<_>>();
        for handle in handles {
            if cancel.token().is_cancelled() {
                return Err(SessionError::Cancelled.into());
            }
            let mut run = handle.lock().await;
            self.reidentify(&mut run, now).await?;
            if run.awaiting_reconnect && !run.file.terminal() {
                let all = run.parent_attached
                    && ROLES.iter().all(|role| {
                        run.file.child_id(role) == 0 || run.reconnected.contains(*role)
                    });
                if all {
                    run.awaiting_reconnect = false;
                    run.file
                        .event("resumed", "children reattached; continuing".into(), now);
                    self.board(&run, "relay continues after hub restart", now);
                    self.save(&run).await;
                } else if now
                    .duration_since(run.restored_at)
                    .is_ok_and(|age| age > Duration::from_secs(120))
                {
                    run.awaiting_reconnect = false;
                    self.finish(
                        &mut run,
                        "stopped",
                        "hub_restart",
                        "children not reconnected within 2m0s",
                        now,
                    )
                    .await;
                } else {
                    continue;
                }
            }
            if run.file.terminal() {
                continue;
            }
            let core = self.core()?;
            let cfg = self.cfg()?.orchestration;
            // Snapshot IDs because an instruction can replace one headless child.
            for child in run.file.child_ids() {
                if run.file.terminal() {
                    break;
                }
                let Some(role) = run.file.role_of(child) else {
                    continue;
                };
                let previous = run.timers.get(&child).and_then(|timer| timer.stamp);
                match self
                    .store
                    .read_progress_if_changed(&run.file, role, previous)
                {
                    Ok(Some((text, stamp))) => {
                        let timer = run.timers.entry(child).or_default();
                        timer.stamp = Some(stamp);
                        timer.last_write_at = Some(now);
                        self.deps.orchestration.record_relay_progress(
                            &OrchestrationId(run.file.orchestration_id.clone()),
                            LiveSessionId(child),
                            stamp,
                            now,
                        );
                        if run.file.headless(role) {
                            if relay::text::count_done_lines(
                                &text,
                                role,
                                run.file.progress_id(role),
                            )
                            .count
                                > run.file.baseline(role)
                            {
                                run.timers
                                    .entry(child)
                                    .or_default()
                                    .pending_done_at
                                    .get_or_insert(now);
                            }
                        } else {
                            self.advance(&mut run, child, &text, false, now, &cancel)
                                .await;
                        }
                    }
                    Ok(None) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => self.warn("relay progress read", error),
                }
                if run.file.terminal() || run.file.role_of(child).is_none() {
                    continue;
                }
                let Some(details) = core.details(LiveSessionId(child)) else {
                    self.exited(&mut run, child, "dismissed", now, &cancel)
                        .await;
                    continue;
                };
                if !details.connected
                    && matches!(details.snapshot.state.as_str(), "completed" | "error")
                {
                    let timer = run.timers.entry(child).or_default();
                    if !timer.exit_handled {
                        timer.exit_handled = true;
                        self.exited(&mut run, child, &details.snapshot.state, now, &cancel)
                            .await;
                    }
                    continue;
                }
                let startup_wait = self
                    .deps
                    .orchestration
                    .relay_launch_startup_wait(LiveSessionId(child), now)?;
                if let Some(blocker) = startup_wait.as_deref()
                    && !run.timers.entry(child).or_default().startup_wait_notified
                {
                    run.timers.entry(child).or_default().startup_wait_notified = true;
                    let folder = matches!(
                        blocker,
                        "Doyoutrustthecontentsofthisdirectory"
                            | "Trustthisfolder?Codexcanread"
                            | "Isthisaprojectyoucreatedoroneyoutrust?"
                            | "Yes,Itrustthisfolder"
                    );
                    let screen = if blocker.is_empty() {
                        "a screen Hub does not recognise; the input box has not appeared"
                    } else {
                        blocker
                    };
                    let text = if folder {
                        format!(
                            "child waiting on its folder-trust prompt: role={role} id={child} ({screen}). The instructions were handed over at launch and start as soon as the user opens the child session and chooses Yes (Codex: Trust and continue); do not send them again"
                        )
                    } else {
                        format!(
                            "child waiting on a startup screen before taking its instructions: role={role} id={child} ({screen}). The instructions were handed over at launch and start as soon as the user opens the child session and answers that screen; do not send them again"
                        )
                    };
                    self.board(&run, &text, now);
                    if run.parent_attached {
                        self.deps
                            .orchestration
                            .notify_relay_parent(
                                LiveSessionId(run.file.parent_session_id),
                                &OrchestrationId(run.file.orchestration_id.clone()),
                                format!("\n[orchestration] {text}\n"),
                            )
                            .await?;
                    }
                }
                // Deliberate safety correction to Go's early-DONE race: a
                // headless DONE waits for exit authority. The existing configured
                // child timeout bounds that wait; an explicitly disabled timeout
                // remains disabled.
                // Timeout stops the relay and retains the child/worktree.
                if run.file.headless(role)
                    && cfg.child_timeout_seconds > 0
                    && run
                        .timers
                        .get(&child)
                        .and_then(|timer| timer.pending_done_at)
                        .is_some_and(|at| {
                            now.duration_since(at).is_ok_and(|age| {
                                age > Duration::from_secs(cfg.child_timeout_seconds as u64)
                            })
                        })
                {
                    self.finish(
                        &mut run,
                        "stopped",
                        "timeout",
                        &format!(
                            "{role} #{child} reported DONE but did not exit within {}s",
                            cfg.child_timeout_seconds
                        ),
                        now,
                    )
                    .await;
                    continue;
                }
                if child != run.file.awaited() {
                    continue;
                }
                let timer = run.timers.entry(child).or_default();
                if details.snapshot.state == "standby" {
                    timer.standby_since.get_or_insert(now);
                } else {
                    timer.standby_since = None;
                }
                let grace = if cfg.child_startup_grace_seconds <= 0 {
                    60
                } else {
                    cfg.child_startup_grace_seconds
                };
                let delivered = matches!(
                    core.initial_prompt_outcome(details.binding).ok().flatten(),
                    Some((InitialPromptOutcome::Delivered { .. }, _))
                );
                if cfg.child_startup_fail_enabled()
                    && delivered
                    && timer.stamp.is_none()
                    && details.approval.record.is_none()
                    && timer.standby_since.is_some_and(|since| {
                        now.duration_since(since)
                            .is_ok_and(|age| age >= Duration::from_secs(grace as u64))
                    })
                {
                    let screen = core
                        .initial_prompt_observation(details.binding)
                        .map(|(_, screen)| screen)
                        .unwrap_or_default();
                    let tail = screen
                        .iter()
                        .rev()
                        .filter(|line| {
                            !line.trim().is_empty() && !line.trim().starts_with(['│', '┃'])
                        })
                        .take(5)
                        .cloned()
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect::<Vec<_>>()
                        .join("\n");
                    let tail =
                        crate::orchestration::child_launch::prompt::sanitize_inject_text(&tail);
                    self.board(&run,&format!("startup failed: role={role} id={child} went idle for {grace}s after its initial prompt was delivered and never wrote its progress file. screen tail:\n{tail}\n"),now);
                    self.finish(
                        &mut run,
                        "stopped",
                        "startup_failed",
                        &format!("{role} #{child}"),
                        now,
                    )
                    .await;
                    if cfg.child_startup_kill_enabled() {
                        self.effects(core.stop(
                            LiveSessionId(child),
                            StopReason::StartupFailed,
                            now,
                        )?)
                        .await?;
                    }
                    continue;
                }
                let activity = timer
                    .assigned_at
                    .into_iter()
                    .chain(timer.last_write_at)
                    .chain(details.last_output_at)
                    .max()
                    .unwrap_or(now);
                let waiting = self
                    .deps
                    .orchestration
                    .launch_instruction_not_taken(LiveSessionId(child))?;
                if cfg.child_timeout_seconds > 0
                    && startup_wait.is_none()
                    && now.duration_since(activity).is_ok_and(|age| {
                        age > Duration::from_secs(cfg.child_timeout_seconds as u64)
                    })
                {
                    self.finish(
                        &mut run,
                        "stopped",
                        "timeout",
                        &format!("{role} #{child}"),
                        now,
                    )
                    .await;
                    continue;
                }
                if cfg.idle_done_threshold_sec > 0
                    && !run.file.headless(role)
                    && !run.nudged.contains(&child)
                    && now.duration_since(activity).is_ok_and(|age| {
                        age > Duration::from_secs(cfg.idle_done_threshold_sec as u64)
                    })
                {
                    if waiting {
                        if !run.timers.entry(child).or_default().nudge_wait_notified {
                            run.timers.entry(child).or_default().nudge_wait_notified = true;
                            self.board(&run,&format!("reminder not typed: {role} #{child} has not taken its launch instructions yet (still starting or waiting on a startup screen); they start once the user answers that screen"),now);
                        }
                        continue;
                    }
                    let prompt = run.file.prompts().nudge_text(role);
                    run.nudged.insert(child);
                    if let Err(error) = self
                        .deliver(LiveSessionId(child), format!("\n{prompt}\n"), &cancel, now)
                        .await
                    {
                        self.warn("relay reminder delivery", error)
                    }
                    run.file.event("nudge", format!("{role} #{child}"), now);
                    self.save(&run).await;
                }
            }
        }
        Ok(())
    }
    /// Cold-reattach metadata barrier. Never lock an active launch's run while
    /// that launch waits for this same wrapper's registration acknowledgement.
    pub async fn prepare_reattach(
        &self,
        binding: SessionBinding,
        now: Timestamp,
    ) -> Result<(), SessionError> {
        let core = self.core()?;
        let details = core
            .details(binding.session)
            .ok_or(SessionError::NotFound(binding.session))?;
        if details.binding != binding {
            return Err(SessionError::StaleBinding);
        }
        let snapshot = &details.snapshot;
        let handles = {
            let state = lock(&self.state);
            state
                .meta
                .iter()
                .filter(|(_, meta)| {
                    meta.restoring
                        && ((!meta.attached
                            && !meta.parent_identity.0.is_empty()
                            && meta.parent_identity.0 == snapshot.started_at
                            && meta.parent_identity.1 == snapshot.cwd
                            && meta.parent_identity.2 == snapshot.provider)
                            || (!snapshot.launch_label.is_empty()
                                && meta
                                    .child_labels
                                    .iter()
                                    .any(|(_, label)| label == &snapshot.launch_label)))
                })
                .filter_map(|(id, _)| state.runs.get(id).cloned())
                .collect::<Vec<_>>()
        };
        for handle in handles {
            let mut run = handle.lock().await;
            if run.parent_attached && !run.awaiting_reconnect {
                continue;
            }
            self.reidentify_one(&mut run, &details, now)
                .await
                .map_err(|error| SessionError::ChildLaunch {
                    status: error.status,
                    code: error.code,
                    detail: error.detail,
                })?;
        }
        Ok(())
    }
    async fn reidentify(&self, run: &mut Run, now: Timestamp) -> Result<(), RelayError> {
        if run.parent_attached && !run.awaiting_reconnect {
            return Ok(());
        }
        let core = self.core()?;
        for snapshot in core.snapshots() {
            if let Some(details) = core.details(snapshot.id) {
                self.reidentify_one(run, &details, now).await?;
            }
        }
        Ok(())
    }
    async fn reidentify_one(
        &self,
        run: &mut Run,
        details: &SessionDetails,
        now: Timestamp,
    ) -> Result<(), RelayError> {
        if !details.connected {
            return Ok(());
        }
        let core = self.core()?;
        let snapshot = &details.snapshot;
        let mut changed = false;
        if !run.parent_attached
            && !run.file.parent_started_at.is_empty()
            && snapshot.started_at == run.file.parent_started_at
            && snapshot.cwd == run.file.parent_cwd
            && snapshot.provider == run.file.parent_provider
        {
            let old = LiveSessionId(run.file.parent_session_id);
            run.file.parent_session_id = snapshot.id.0;
            run.parent_attached = true;
            self.effects(core.apply_relay_binding(
                details.binding,
                LiveSessionId(0),
                OrchestrationId(run.file.orchestration_id.clone()),
                run.file.board_path.clone(),
                String::new(),
                now,
            )?)
            .await?;
            self.deps.orchestration.rebind_relay_parent(
                &OrchestrationId(run.file.orchestration_id.clone()),
                old,
                snapshot.id,
            );
            self.deps.orchestration.register_conductor(
                details.binding,
                &OrchestrationId(run.file.orchestration_id.clone()),
                Path::new(&run.file.board_path),
            )?;
            // Children that arrived first were deliberately bound to parent 0,
            // not a recycled numeric identity. Attach them only now.
            for role in ROLES {
                if !run.reconnected.contains(role) {
                    continue;
                }
                if let Some(child) = core.details(LiveSessionId(run.file.child_id(role)))
                    && child.snapshot.launch_label == run.file.child_label(role)
                    && child.snapshot.orchestration_id.0 == run.file.orchestration_id
                {
                    self.effects(core.apply_relay_binding(
                        child.binding,
                        snapshot.id,
                        OrchestrationId(run.file.orchestration_id.clone()),
                        run.file.board_path.clone(),
                        role.into(),
                        now,
                    )?)
                    .await?;
                }
            }
            self.board(
                run,
                &format!(
                    "conductor reattached session={} (was #{})",
                    snapshot.id.0, old.0
                ),
                now,
            );
            changed = true;
        }
        for role in ROLES {
            if snapshot.launch_label.trim().is_empty()
                || run.reconnected.contains(role)
                || run.file.child_id(role) == 0
                || run.file.child_label(role) != snapshot.launch_label
            {
                continue;
            }
            let old = run.file.child_id(role);
            let progress = run.file.progress_id(role);
            let label = run.file.child_label(role).to_owned();
            run.file.set_child(role, snapshot.id.0, label, progress);
            run.reconnected.insert(role.into());
            run.timers.insert(
                snapshot.id.0,
                relay::ChildTimer {
                    assigned_at: Some(now),
                    last_write_at: Some(now),
                    ..Default::default()
                },
            );
            let parent = LiveSessionId(if run.parent_attached {
                run.file.parent_session_id
            } else {
                0
            });
            self.effects(core.apply_relay_binding(
                details.binding,
                parent,
                OrchestrationId(run.file.orchestration_id.clone()),
                run.file.board_path.clone(),
                role.into(),
                now,
            )?)
            .await?;
            self.board(run,&format!("child reattached role={role} session={} (was #{old}, progress file child-{progress}.md)",snapshot.id.0),now);
            changed = true;
        }
        if changed {
            self.save(run).await;
        }
        Ok(())
    }
}
