//! Application ordering before CoreEvent bus publication. Git capture uses the
//! real Files owner; handoff and UI publication finish before its capture gate
//! is released. Other events are published by EffectDriver for owned consumers.
//!
//! This is newly implemented integration, not recovered checkpoint source.
//! End capture reserves the Files gate synchronously, then runs on the retained
//! Hub effect-task owner. Confirmed input cannot overtake the reserved end.
use crate::{
    config::{ConfigStore, RuntimePaths},
    files::{FilesService, GitTurnCompleted, GitTurnSnapshot},
    hub::{sockets::OrderedEventObserver, task_owner::HubTaskHandle},
    orchestration::handoff::{HandoffStore, KIND_GIT_TURN, Record},
    process::{self, Cancellation, ExitOutcome, ProcessPlan},
    proto::{
        core::*,
        time::{Timestamp, parse_rfc3339},
    },
    storage::mask_secrets,
    terminal::session::SessionEngine,
};
use std::{
    sync::{Arc, OnceLock, Weak},
    time::Duration,
};

/// Required source completion operations. The real application must implement
/// summary await state/input delivery and any DONE fallback callback. Neither
/// can be represented by a successful default or a test observer in serve.
pub trait GitTurnCompletionCallbacks: Send + Sync {
    fn routine_completed<'a>(
        &'a self,
        binding: SessionBinding,
        summary: &'a crate::proto::DoneSummary,
    ) -> CoreFuture<'a, Result<(), SessionError>>;
    fn handoff_note_written<'a>(
        &'a self,
        binding: SessionBinding,
        path: &'a std::path::Path,
    ) -> CoreFuture<'a, Result<(), SessionError>>;
    fn before_done_publish<'a>(
        &'a self,
        binding: SessionBinding,
        summary: &'a crate::proto::DoneSummary,
    ) -> CoreFuture<'a, Result<(), SessionError>>;
    fn inject_turn_summary<'a>(
        &'a self,
        binding: SessionBinding,
        turn: i64,
    ) -> CoreFuture<'a, Result<(), SessionError>>;
    fn after_git_turn_broadcast<'a>(
        &'a self,
        binding: SessionBinding,
        turn: &'a GitTurnSnapshot,
    ) -> CoreFuture<'a, Result<(), SessionError>>;
}

pub type EventWarning = Arc<dyn Fn(&str, &SessionError) + Send + Sync>;
struct BoundOwners {
    core: Weak<SessionEngine>,
    effects: Weak<dyn CoreEffectSink>,
}
#[derive(Clone)]
pub struct ApplicationEventObserver {
    config: Arc<ConfigStore>,
    files: Arc<FilesService>,
    handoff: Arc<HandoffStore>,
    callbacks: Arc<dyn GitTurnCompletionCallbacks>,
    warning: EventWarning,
    owners: Arc<OnceLock<BoundOwners>>,
    usage_hooks: Arc<OnceLock<Weak<crate::application::usage_hooks::UsageHooks>>>,
    tasks: HubTaskHandle,
    notifications: Option<Arc<crate::notify::Manager>>,
    push: Option<Arc<crate::application::push::PushManager>>,
    one_tap: Option<Arc<crate::approval::token::OneTapManager>>,
    approval_rules: Option<Arc<crate::application::approval_rules::ApprovalRules>>,
    orchestration:
        Arc<OnceLock<Weak<crate::application::orchestration_program::OrchestrationProgram>>>,
}
impl ApplicationEventObserver {
    pub fn new(
        config: Arc<ConfigStore>,
        paths: RuntimePaths,
        files: Arc<FilesService>,
        callbacks: Arc<dyn GitTurnCompletionCallbacks>,
        warning: EventWarning,
        tasks: HubTaskHandle,
    ) -> Self {
        Self {
            config,
            files,
            handoff: Arc::new(HandoffStore::new(paths)),
            callbacks,
            warning,
            owners: Arc::new(OnceLock::new()),
            usage_hooks: Arc::new(OnceLock::new()),
            tasks,
            notifications: None,
            push: None,
            one_tap: None,
            approval_rules: None,
            orchestration: Arc::new(OnceLock::new()),
        }
    }
    pub fn with_push(mut self, push: Option<Arc<crate::application::push::PushManager>>) -> Self {
        self.push = push;
        self
    }
    pub fn bind_usage_hooks(
        &self,
        hooks: &Arc<crate::application::usage_hooks::UsageHooks>,
    ) -> Result<(), SessionError> {
        self.usage_hooks
            .set(Arc::downgrade(hooks))
            .map_err(|_| SessionError::InvalidRequest("usage hooks already bound".into()))
    }
    pub fn with_notifications(mut self, notifications: Arc<crate::notify::Manager>) -> Self {
        self.notifications = Some(notifications);
        self
    }
    pub fn with_one_tap(mut self, manager: Arc<crate::approval::token::OneTapManager>) -> Self {
        self.one_tap = Some(manager);
        self
    }
    pub fn with_approval_rules(
        mut self,
        rules: Arc<crate::application::approval_rules::ApprovalRules>,
    ) -> Self {
        self.approval_rules = Some(rules);
        self
    }
    pub fn with_orchestration(
        mut self,
        owner: Arc<crate::application::orchestration_program::OrchestrationProgram>,
    ) -> Self {
        self.orchestration = Arc::new(OnceLock::from(Arc::downgrade(&owner)));
        self
    }
    pub fn bind_orchestration(
        &self,
        owner: &Arc<crate::application::orchestration_program::OrchestrationProgram>,
    ) -> Result<(), SessionError> {
        self.orchestration
            .set(Arc::downgrade(owner))
            .map_err(|_| SessionError::InvalidRequest("orchestration owner already bound".into()))
    }
    fn approval_notification(
        &self,
        session: LiveSessionId,
        record: &ImmutableApprovalRecord,
    ) -> Result<(), SessionError> {
        let core = self
            .owners
            .get()
            .and_then(|owners| owners.core.upgrade())
            .ok_or(SessionError::Shutdown)?;
        if let Some(push) = &self.push
            && let Err(error) =
                push.notify_approval(&core, session, record, self.one_tap.as_deref(), "")
        {
            (self.warning)("push approval notification admission", &error);
        }
        let Some(manager) = &self.notifications else {
            return Ok(());
        };
        let Some(details) = core.details(session) else {
            return Ok(());
        };
        let record = record.data();
        if !details
            .approval
            .record
            .as_ref()
            .is_some_and(|current| current.data().candidate.same_candidate(&record.candidate))
        {
            return Ok(());
        }
        let config = self
            .config
            .snapshot()
            .map_err(|_| {
                SessionError::InvalidRequest("notification configuration unavailable".into())
            })?
            .config;
        manager.update_config(config.notify.clone());
        let snapshot = details.snapshot;
        let name = [
            snapshot.display.trim(),
            snapshot.provider.trim(),
            "many-ai-cli",
        ]
        .into_iter()
        .find(|value| !value.is_empty())
        .unwrap();
        let title = if snapshot.label.is_empty() {
            format!("{name} #{}", session.0)
        } else {
            format!("{name} #{} [{}]", session.0, snapshot.label)
        };
        let text = [
            &record.question,
            &record.context,
            &snapshot.last_message,
            &snapshot.first_message,
            &snapshot.cwd,
            "Approval is waiting.",
        ]
        .into_iter()
        .find(|value| !value.is_empty())
        .unwrap();
        let body = mask_secrets(text)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let summary = crate::approval::summary::summarize(&record.question, &record.context);
        let external = config
            .hub
            .allowed_hosts
            .first()
            .filter(|host| config.hub.allowed_hosts.len() == 1 && !host.trim().is_empty())
            .map(|host| format!("https://{}", host.trim()));
        let notification_id = if record.sig.is_empty() {
            format!("session-{}-{body}", session.0)
        } else {
            format!("{}#{}", record.sig, record.candidate.source_epoch.0)
        };
        let mut payload = crate::notify::ApprovalPayload {
            id: notification_id,
            session_id: session.0,
            title,
            body,
            risk: summary.risk.clone(),
            open_url: external
                .as_ref()
                .map(|base| format!("{base}/?session_id={}", session.0))
                .unwrap_or_default(),
            ..Default::default()
        };
        if record.origin == "native"
            && !record.options.is_empty()
            && !record.sig.is_empty()
            && let (Some(tokens), Some(base)) = (&self.one_tap, &external)
        {
            use crate::approval::token::OneTapAction;
            for (action, target) in [
                (OneTapAction::Reject, &mut payload.reject_url),
                (OneTapAction::Approve, &mut payload.approve_url),
            ] {
                if action == OneTapAction::Approve && summary.risk == "high" {
                    continue;
                }
                if let Ok(token) = tokens.issue(
                    session,
                    &record.sig,
                    &record.sig,
                    record.candidate.source_epoch,
                    action,
                    Timestamp::now(),
                ) {
                    *target = format!("{base}/api/approval-action/{token}");
                }
            }
        }
        if let Err(error) = manager.send_approval(payload) {
            (self.warning)("approval notification admission", &error);
        }
        Ok(())
    }
    /// Bind the actual owners once after constructing the sink/core cycle. Weak
    /// references prevent the observer from keeping the Hub alive after drain.
    pub fn bind(
        &self,
        core: Weak<SessionEngine>,
        effects: Weak<dyn CoreEffectSink>,
    ) -> Result<(), SessionError> {
        self.owners.set(BoundOwners { core, effects }).map_err(|_| {
            SessionError::InvalidRequest("event observer owners are already bound".into())
        })
    }
    async fn git_turn(
        &self,
        binding: SessionBinding,
        started_at: &str,
        ended_at: Option<&str>,
    ) -> Result<(), SessionError> {
        let owners = self.owners.get().ok_or(SessionError::Shutdown)?;
        let core = owners.core.upgrade().ok_or(SessionError::Shutdown)?;
        let effects = owners.effects.upgrade().ok_or(SessionError::Shutdown)?;
        if let Some(ended_at) = ended_at {
            let permit = self.tasks.effect_permit()?;
            if !core.is_current(binding) {
                return Ok(());
            }
            let Some(reservation) = self.files.reserve_git_turn_end(binding) else {
                return Ok(());
            };
            let observer = self.clone();
            let ended_at = ended_at.to_owned();
            // Dropping the response waiter cannot abandon capture/completion.
            drop(permit.start(async move {
                match observer
                    .files
                    .observe_reserved_git_turn_end(core.as_ref(), reservation, &ended_at)
                    .await
                {
                    Ok(Some(completed)) => {
                        observer
                            .complete(&core, effects.as_ref(), binding, &completed)
                            .await
                    }
                    Ok(None) => {}
                    Err(response) => (observer.warning)(
                        "git turn end snapshot skipped",
                        &SessionError::InvalidRequest(format!(
                            "snapshot unavailable: status={}",
                            response.status
                        )),
                    ),
                }
            }));
            return Ok(());
        }
        let completed = match self
            .files
            .observe_git_turn(core.as_ref(), binding, started_at, ended_at)
            .await
        {
            Ok(completed) => completed,
            Err(response) => {
                // Go snapshot failures are diagnostic and never veto provider
                // input. No response body, Git stderr or work text is logged.
                (self.warning)(
                    "git turn snapshot skipped",
                    &SessionError::InvalidRequest(format!(
                        "snapshot unavailable: status={}",
                        response.status
                    )),
                );
                return Ok(());
            }
        };
        let Some(completed) = completed else {
            return Ok(());
        };
        // Keep the real completion object (and gate) through every operation.
        self.complete(&core, effects.as_ref(), binding, &completed)
            .await;
        Ok(())
    }
    async fn complete(
        &self,
        core: &SessionEngine,
        effects: &dyn CoreEffectSink,
        binding: SessionBinding,
        completed: &GitTurnCompleted,
    ) {
        if let Some(_detail) = &completed.diff_error {
            (self.warning)(
                "git turn summary unavailable",
                &SessionError::InvalidRequest(
                    "summary failed; captured tree retained with zero counts".into(),
                ),
            );
        }
        let cfg = match self.config.snapshot() {
            Ok(snapshot) => Some(snapshot.config),
            Err(_) => {
                (self.warning)("git turn handoff configuration", &SessionError::Shutdown);
                None
            }
        };
        if let Some(config) = &cfg {
            if config.handoff.enabled_or_default() {
                self.record_handoff(core, binding, completed).await;
            }
            if config.handoff.enabled_or_default()
                && config.handoff.turn_summary_enabled()
                && let Err(error) = self
                    .callbacks
                    .inject_turn_summary(binding, completed.snapshot.turn)
                    .await
            {
                (self.warning)("git turn summary injection", &error);
            }
        }
        let turn = &completed.snapshot;
        let notification = GitTurnNotification {
            r#type: GitTurnType::GitTurn,
            session_id: binding.session.0,
            turn: turn.turn,
            started_at: turn.started_at.clone(),
            ended_at: turn.ended_at.clone(),
            files_changed: turn.files,
            added: turn.added,
            removed: turn.removed,
        };
        if let Err(failure) = effects.apply(core.broadcast_git_turn(notification)).await {
            (self.warning)("git turn broadcast", &failure.error);
        }
        if let Err(error) = self.callbacks.after_git_turn_broadcast(binding, turn).await {
            (self.warning)("git turn completion callback", &error);
        }
    }
    async fn record_handoff(
        &self,
        core: &SessionEngine,
        binding: SessionBinding,
        completed: &GitTurnCompleted,
    ) {
        let details = core.details(binding.session);
        if details
            .as_ref()
            .is_none_or(|session| session.binding.incarnation != binding.incarnation)
        {
            return;
        }
        let snapshot = details.unwrap().snapshot;
        let (commit, subject) = self.latest_commit(completed).await;
        let turn = &completed.snapshot;
        let files = completed.diff["files"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|file| file["path"].as_str().map(str::to_owned))
            .collect();
        let record = Record {
            kind: KIND_GIT_TURN.into(),
            provider: snapshot.provider,
            cwd: snapshot.cwd,
            branch: snapshot.branch,
            model: snapshot.model,
            subscription_id: snapshot.subscription_profile_id,
            turn: turn.turn,
            files_changed: turn.files,
            added: turn.added,
            removed: turn.removed,
            files,
            commit,
            commit_subject: subject,
            ..Default::default()
        };
        let now = parse_rfc3339(&turn.ended_at).unwrap_or_else(|_| Timestamp::now());
        // Short append I/O occurs outside Session locks. Keeping it synchronous
        // in the owned completion avoids abandoning a blocking writer on abort.
        if self.handoff.append(binding.session.0, record, now).is_err() {
            (self.warning)(
                "handoff append failed",
                &SessionError::InvalidRequest(
                    "git turn handoff record could not be written".into(),
                ),
            );
        }
    }
    async fn latest_commit(&self, completed: &GitTurnCompleted) -> (String, String) {
        let plan = ProcessPlan {
            executable: self.files.git_executable.clone(),
            args: ["log", "-1", "--pretty=format:%H%x09%s"]
                .into_iter()
                .map(Into::into)
                .collect(),
            cwd: completed.git_root.clone(),
            env: self.files.git_environment.clone(),
            stdin: Vec::new(),
            timeout: Duration::from_secs(5),
            output_cap: 1024 * 1024,
            pipe_drain_timeout: Duration::from_secs(2),
        };
        let Ok(out) = process::run_capped(&plan, &Cancellation::default()).await else {
            return Default::default();
        };
        if !matches!(out.outcome, ExitOutcome::Exited { code: Some(0), .. }) {
            return Default::default();
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let mut fields = text.trim().splitn(2, '\t');
        (
            fields.next().unwrap_or("").into(),
            mask_secrets(fields.next().unwrap_or("")),
        )
    }
}
impl OrderedEventObserver for ApplicationEventObserver {
    fn approval_opened<'a>(
        &'a self,
        session: LiveSessionId,
        record: &'a ImmutableApprovalRecord,
    ) -> CoreFuture<'a, Result<bool, SessionError>> {
        Box::pin(async move {
            if let Some(rules) = &self.approval_rules
                && rules.try_apply(session, record).await?
            {
                return Ok(true);
            }
            Ok(false)
        })
    }
    fn observe<'a>(&'a self, event: &'a CoreEvent) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            match event {
                CoreEvent::Ended { binding, end } => {
                    if let Some(hooks) = self.usage_hooks.get().and_then(Weak::upgrade) {
                        hooks.ended("codex");
                    }
                    if let Some(owner) = self.orchestration.get().and_then(Weak::upgrade) {
                        owner
                            .session_ended(binding.session, &end.declared_state)
                            .await?;
                    }
                    Ok(())
                }
                CoreEvent::Dismissed(session) => {
                    if let Some(hooks) = self.usage_hooks.get().and_then(Weak::upgrade) {
                        hooks.ended("codex");
                    }
                    if let Some(owner) = self.orchestration.get().and_then(Weak::upgrade) {
                        owner.session_ended(*session, "dismissed").await?;
                    }
                    Ok(())
                }
                CoreEvent::ApprovalPublished { session, record } => {
                    self.approval_notification(*session, record)
                }
                CoreEvent::CompletionRecord { binding, summary } => {
                    self.callbacks
                        .before_done_publish(*binding, summary)
                        .await?;
                    Ok(())
                }
                CoreEvent::HandoffNoteWritten { binding, path } => {
                    self.callbacks.handoff_note_written(*binding, path).await
                }
                CoreEvent::RoutineCompleted { binding, summary } => {
                    self.callbacks.routine_completed(*binding, summary).await
                }
                CoreEvent::Completed { summary, .. } => {
                    if !summary.fallback
                        && let Some(manager) = &self.notifications
                    {
                        let snapshot = self.config.snapshot().map_err(|_| {
                            SessionError::InvalidRequest(
                                "notification configuration unavailable".into(),
                            )
                        })?;
                        let config = snapshot.config;
                        let enabled = config
                            .user_prefs
                            .done_summary_notify
                            .enabled
                            .unwrap_or_else(|| {
                                config
                                    .notify
                                    .backends
                                    .as_ref()
                                    .is_some_and(|backends| !backends.is_empty())
                            });
                        if !enabled {
                            return Ok(());
                        }
                        manager.update_config(config.notify);
                        if let Err(error) = manager.send_done(crate::notify::DonePayload {
                            session_id: summary.session_id,
                            title: summary.title.clone(),
                            summary: summary.text.clone(),
                            kind: summary.kind.clone(),
                            ..Default::default()
                        }) {
                            (self.warning)("done notification admission", &error);
                        }
                    }
                    Ok(())
                }
                CoreEvent::GitTurnCapture {
                    binding,
                    started_at,
                    ended_at,
                } => {
                    self.git_turn(*binding, started_at, ended_at.as_deref())
                        .await
                }
                // These events have no pre-publication Git dependency. Their
                // lifecycle/transcript/routine consumers belong to the bus.
                _ => Ok(()),
            }
        })
    }
}

#[cfg(test)]
mod tests;
