use super::*;
struct Timer {
    id: OrchestrationId,
    child: LiveSessionId,
    parent: SessionBinding,
    role: String,
    path: PathBuf,
    kind: &'static str,
    threshold: i64,
    screen: Vec<String>,
    blocker: String,
}
impl OrchestrationProgram {
    pub async fn session_ended(
        &self,
        session: LiveSessionId,
        state: &str,
    ) -> Result<(), SessionError> {
        let record = {
            let mut owned = lock(&self.state);
            owned.boards.iter_mut().find_map(|(id, board)| {
                let child = board.children.get_mut(&session)?;
                child.done = true;
                Some((
                    id.clone(),
                    board.path.clone(),
                    child.registration.parent.session,
                    child.registration.role.clone(),
                ))
            })
        };
        let Some((id, path, parent, role)) = record else {
            return Ok(());
        };
        let dismissed = state == "dismissed";
        let evidence = if dismissed {
            format!(
                "child dismissed from the dashboard: role={role} session={} (closed by the user; its work may be unfinished)\n",
                session.0
            )
        } else {
            format!(
                "child completed without DONE marker: role={role} session={} state={state} (session_end)\n",
                session.0
            )
        };
        self.boards
            .append(&path, "hub", &evidence, Timestamp::now())
            .map_err(|e| SessionError::Transport(e.to_string()))?;
        let text = if dismissed {
            format!(
                "\n[orchestration] child dismissed role={role} id={} (closed from the dashboard by the user; treat its work as unfinished)\n",
                session.0
            )
        } else {
            format!(
                "\n[orchestration] child complete via session_end role={role} id={} state={state}\n",
                session.0
            )
        };
        self.queue_event(parent, &id, text).await
    }
    fn queue_respawn(
        &self,
        id: &OrchestrationId,
        session: LiveSessionId,
        max: i64,
    ) -> Result<(), SessionError> {
        if max <= 0 {
            return Ok(());
        }
        let request = {
            let mut state = lock(&self.state);
            let Some(board) = state.boards.get_mut(id) else {
                return Ok(());
            };
            let Some(child) = board.children.get_mut(&session) else {
                return Ok(());
            };
            if child.retries >= max || child.registration.restart_spec.provider.is_empty() {
                return Ok(());
            }
            let retries = child.retries + 1;
            child.retries = max;
            (
                child.registration.restart_spec.clone(),
                child.registration.parent,
                child.registration.role.clone(),
                child.registration.preparation.clone(),
                child.registration.initial_prompt.clone(),
                retries,
            )
        };
        let weak = self.this.clone();
        let cancel = self.deps.tasks.effect_permit()?;
        let cancellation = cancel.cancellation();
        drop(cancel.start(async move {
            let Some(owner) = weak.upgrade() else { return };
            let (parent, id, role) = (
                request.1,
                request.3.orchestration.clone(),
                request.2.clone(),
            );
            let result = owner.respawn(request, cancellation).await;
            if let Err(error) = result {
                let text = format!("role={role} id={}: {}", session.0, error_detail(&error));
                if let Err(error) = owner
                    .notify_orchestration_error(
                        parent,
                        "timeout_respawn",
                        &text,
                        TaskCancellation::default(),
                    )
                    .await
                {
                    owner.warning("timeout respawn notification", &error);
                }
            } else {
                let path = lock(&owner.state).boards.get(&id).map(|b| b.path.clone());
                if let Some(path) = path
                    && let Err(error) = owner.boards.append(
                        &path,
                        "hub",
                        &format!(
                            "timeout retry spawned: role={role} old_session={}\n",
                            session.0
                        ),
                        Timestamp::now(),
                    )
                {
                    owner.warning(
                        "timeout retry board record",
                        &SessionError::Transport(error.to_string()),
                    );
                }
            }
        }));
        Ok(())
    }
    async fn respawn(
        &self,
        request: (
            WrappedSpawnSpec,
            SessionBinding,
            String,
            child_launch::ChildPreparation,
            String,
            i64,
        ),
        cancel: TaskCancellation,
    ) -> Result<(), SessionError> {
        let (mut spec, parent, role, preparation, initial, retries) = request;
        let restart = spec.clone();
        let core = self.core()?;
        let cfg = self.config()?.orchestration;
        let now = Timestamp::now();
        let admission = core
            .reserve_children(
                AdmissionRequest {
                    parent: parent.session,
                    slots: 1,
                    origin: VerifiedSpawnOrigin::Autonomous,
                    replace: None,
                },
                AdmissionLimits {
                    max_children_per_parent: cfg.max_children_per_parent,
                    max_total_sessions: cfg.max_total_sessions,
                },
            )
            .map_err(|e| SessionError::Transport(format!("respawn admission: {e:?}")))?;
        struct Release {
            core: Arc<SessionEngine>,
            id: AdmissionId,
        }
        impl Drop for Release {
            fn drop(&mut self) {
                self.core.release_children(&self.id);
            }
        }
        let _release = Release {
            core: core.clone(),
            id: admission.id,
        };
        let via_arg = !crate::config::is_headless_execution_mode(&spec.execution_mode)
            && crate::config::launch_prompt_via_arg(&spec.provider)
            && self.launch_arg_usable(&spec.provider)?;
        let delivery = child_launch::prompt::child_launch_prompt(
            &spec.execution_mode,
            via_arg,
            &initial,
            &preparation.board_path,
            &role,
            &preparation.branch,
        );
        spec.initial_prompt = match &delivery {
            child_launch::prompt::PromptDelivery::AtLaunch(text) => text.clone(),
            _ => String::new(),
        };
        spec.label = format!(
            "orch-{}-{}-retry-{}",
            child_launch::safe_token(&preparation.orchestration.0),
            role,
            now.unix_nanos()
        );
        spec.registration_metadata = SpawnRegistrationMetadata {
            parent: parent.session,
            role: role.clone(),
            auto: true,
            depth: 1,
            orchestration: preparation.orchestration.clone(),
            board_path: preparation.board_path.to_string_lossy().into_owned(),
            worktree_branch: preparation.branch.clone(),
            spawned_at: Some(now),
            prompt_at_launch: matches!(delivery, child_launch::prompt::PromptDelivery::AtLaunch(_)),
            ..Default::default()
        };
        spec.cancellation = cancel.clone();
        let binding = match core
            .spawn_and_wait(
                spec,
                Duration::from_secs(20),
                &HttpWaitCancellation::default(),
            )
            .await
        {
            SpawnWaitOutcome::Registered(binding) => binding,
            _ => {
                return Err(SessionError::Transport(
                    "timeout respawn did not register".into(),
                ));
            }
        };
        core.consume_children(&_release.id, 1);
        let inject = matches!(
            delivery,
            child_launch::prompt::PromptDelivery::AfterRegistration
        )
        .then(|| {
            child_launch::prompt::child_initial_prompt(
                &initial,
                &preparation.board_path,
                &role,
                &preparation.branch,
                &binding.session.0.to_string(),
            )
        });
        self.child_registered(
            RegisteredChild {
                binding,
                parent,
                role,
                preparation: preparation.clone(),
                restart_spec: restart,
                initial_prompt: initial,
                prompt_via_launch_arg: via_arg,
                inject_after_registration: inject,
                spawned_at: now,
            },
            cancel,
        )
        .await?;
        if let Some(child) = lock(&self.state)
            .boards
            .get_mut(&preparation.orchestration)
            .and_then(|board| board.children.get_mut(&binding.session))
        {
            child.retries = retries;
        }
        Ok(())
    }
}
pub(super) async fn poll(owner: &OrchestrationProgram, now: Timestamp) -> Result<(), SessionError> {
    let core = owner.core()?;
    let cfg = owner.config()?.orchestration;
    let ids = {
        let state = lock(&owner.state);
        state
            .boards
            .iter()
            .flat_map(|(id, board)| {
                board
                    .children
                    .keys()
                    .map(|child| (id.clone(), *child))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    };
    let mut timers = Vec::new();
    let mut ends = Vec::new();
    for (id, child_id) in ids {
        let Some(details) = core.details(child_id) else {
            continue;
        };
        let screen = core
            .initial_prompt_observation(details.binding)
            .map(|(_, screen)| screen)
            .unwrap_or_default();
        let outcome = core.initial_prompt_outcome(details.binding).ok().flatten();
        let mut state = lock(&owner.state);
        let Some(board) = state.boards.get_mut(&id) else {
            continue;
        };
        let Some(child) = board.children.get_mut(&child_id) else {
            continue;
        };
        if child.done || child.startup_failed || child.timed_out {
            continue;
        }
        if !details.connected && matches!(details.snapshot.state.as_str(), "completed" | "error") {
            ends.push((child_id, details.snapshot.state.clone()));
            continue;
        }
        if details.snapshot.state == "standby" {
            child.standby_since.get_or_insert(now);
        } else {
            child.standby_since = None;
        }
        let (delivered, failed) = match outcome.as_ref() {
            Some((InitialPromptOutcome::Delivered { .. }, at)) => (Some(*at), false),
            Some((InitialPromptOutcome::Failed { .. }, _)) => (None, true),
            _ => (None, false),
        };
        let waiting = child.registration.prompt_via_launch_arg
            && child.stamp.is_none()
            && delivered.is_none()
            && crate::orchestration::initial_prompt::launch_arg_startup_screen(
                &details.snapshot.provider,
                &screen,
                child.registration.spawned_at,
                details.last_output_at,
                now,
            )
            .is_some();
        let parent = child.registration.parent;
        let role = child.registration.role.clone();
        let path = board.path.clone();
        let timer = |kind, threshold, blocker| Timer {
            id: id.clone(),
            child: child_id,
            parent,
            role: role.clone(),
            path: path.clone(),
            kind,
            threshold,
            screen: screen.clone(),
            blocker,
        };
        let grace = if cfg.child_startup_grace_seconds <= 0 {
            60
        } else {
            cfg.child_startup_grace_seconds
        };
        if cfg.child_startup_fail_enabled()
            && delivered.is_some()
            && !failed
            && child.stamp.is_none()
            && details.approval.record.is_none()
            && child.standby_since.is_some_and(|since| {
                now.duration_since(since)
                    .is_ok_and(|age| age >= Duration::from_secs(grace as u64))
            })
        {
            let event = timer("startup_failed", grace, String::new());
            child.startup_failed = true;
            timers.push(event);
            continue;
        }
        let mut activity = child.registration.spawned_at.max(child.last_board_write);
        if let Some(output) = details.last_output_at {
            activity = activity.max(output);
        }
        if cfg.child_timeout_seconds > 0
            && !waiting
            && now
                .duration_since(activity)
                .is_ok_and(|age| age > Duration::from_secs(cfg.child_timeout_seconds as u64))
        {
            let event = timer("timeout", cfg.child_timeout_seconds, String::new());
            child.timed_out = true;
            timers.push(event);
            continue;
        }
        if cfg.idle_done_threshold_sec > 0 {
            let threshold = Duration::from_secs(cfg.idle_done_threshold_sec as u64);
            let idle = now
                .duration_since(child.last_board_write)
                .is_ok_and(|age| age > threshold)
                && details
                    .last_output_at
                    .is_none_or(|at| now.duration_since(at).is_ok_and(|age| age > threshold));
            if idle && !child.idle_warned {
                let event = timer("idle", cfg.idle_done_threshold_sec, String::new());
                child.idle_warned = true;
                timers.push(event);
            } else if !idle {
                child.idle_warned = false;
            }
        }
        if waiting && !child.startup_wait_notified {
            let blocker = crate::orchestration::initial_prompt::screen_blocker(
                &details.snapshot.provider,
                &screen,
            )
            .unwrap_or("")
            .to_owned();
            let event = timer("startup_wait", 0, blocker);
            child.startup_wait_notified = true;
            timers.push(event);
        }
    }
    for (session, state) in ends {
        owner.session_ended(session, &state).await?;
    }
    for timer in timers {
        match timer.kind {
            "startup_failed" => {
                let tail = timer
                    .screen
                    .iter()
                    .rev()
                    .filter(|line| !line.trim().is_empty() && !line.trim().starts_with(['│', '┃']))
                    .take(5)
                    .cloned()
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<Vec<_>>()
                    .join("\n");
                let tail = child_launch::prompt::sanitize_inject_text(&tail);
                owner.boards.append(&timer.path,"hub",&format!("startup failed: role={} id={} went idle for {}s after its initial prompt was delivered and never wrote its progress file. screen tail:\n{tail}\n",timer.role,timer.child.0,timer.threshold),now).map_err(|e|SessionError::Transport(e.to_string()))?;
                let detail = format!(
                    "role={} id={}: the child went idle for {}s after its initial prompt was delivered and never wrote its progress file. Hub has no other evidence of why it stopped; read the screen tail below before retrying the same role/provider. screen tail: {tail}",
                    timer.role, timer.child.0, timer.threshold
                );
                owner
                    .notify_orchestration_error(
                        timer.parent,
                        "startup_failed",
                        &detail,
                        TaskCancellation::default(),
                    )
                    .await?;
                if let Some(details) = core.details(timer.child) {
                    owner
                        .apply_effects(core.mark_orchestration_child_state(
                            details.binding,
                            "error",
                            now,
                        )?)
                        .await?;
                }
                if cfg.child_startup_kill_enabled() {
                    owner
                        .apply_effects(core.stop(timer.child, StopReason::StartupFailed, now)?)
                        .await?;
                }
            }
            "timeout" => {
                owner
                    .notify_orchestration_error(
                        timer.parent,
                        "timeout",
                        &format!(
                            "role={} id={} threshold={}s",
                            timer.role, timer.child.0, timer.threshold
                        ),
                        TaskCancellation::default(),
                    )
                    .await?;
                if let Some(details) = core.details(timer.child) {
                    owner
                        .apply_effects(core.mark_orchestration_child_state(
                            details.binding,
                            "timeout",
                            now,
                        )?)
                        .await?;
                }
                if cfg.timeout_respawn {
                    owner.queue_respawn(&timer.id, timer.child, cfg.max_timeout_respawns)?;
                }
            }
            "idle" => {
                owner.queue_event(timer.parent.session,&timer.id,format!("\n[orchestration] idle warning role={} id={} no board update and no PTY output for {}s\n",timer.role,timer.child.0,timer.threshold)).await?;
            }
            "startup_wait" => {
                let blocker = if timer.blocker.is_empty() {
                    "a screen Hub does not recognise; the input box has not appeared"
                } else {
                    &timer.blocker
                };
                let folder = matches!(
                    timer.blocker.as_str(),
                    "Doyoutrustthecontentsofthisdirectory"
                        | "Trustthisfolder?Codexcanread"
                        | "Isthisaprojectyoucreatedoroneyoutrust?"
                        | "Yes,Itrustthisfolder"
                );
                let text = if folder {
                    format!(
                        "child waiting on its folder-trust prompt: role={} id={} ({blocker}). The instructions were handed over at launch and start as soon as the user opens the child session and chooses Yes (Codex: Trust and continue); do not send them again",
                        timer.role, timer.child.0
                    )
                } else {
                    format!(
                        "child waiting on a startup screen before taking its instructions: role={} id={} ({blocker}). The instructions were handed over at launch and start as soon as the user opens the child session and answers that screen; do not send them again",
                        timer.role, timer.child.0
                    )
                };
                owner
                    .queue_event(
                        timer.parent.session,
                        &timer.id,
                        format!("\n[orchestration] {text}\n"),
                    )
                    .await?;
            }
            _ => unreachable!(),
        }
    }
    Ok(())
}

fn error_detail(error: &SessionError) -> String {
    let detail = match error {
        SessionError::Transport(detail) | SessionError::InvalidRequest(detail) => detail.as_str(),
        SessionError::ChildLaunch { detail, .. } => detail.as_str(),
        SessionError::Storage(error) => error.detail.as_str(),
        SessionError::Cancelled => "cancelled",
        SessionError::Shutdown => "Hub stopped",
        SessionError::StaleBinding => "session binding changed",
        SessionError::NotFound(_) => "session not found",
        _ => "child retry failed",
    };
    crate::storage::mask_secrets(detail)
}
