//! Production registration, transcript and completion integration. All session
//! admission is delegated to the sole SessionEngine; only bounded file-parser
//! continuations live in this worker owner.
use crate::{
    application::event_observer::{EventWarning, GitTurnCompletionCallbacks},
    approval::transcript::{
        parser::{ParseState, ProviderFormat},
        reader::ReadBudget,
    },
    config::{ConfigStore, RuntimePaths},
    files::{FilesService, GitTurnSnapshot},
    hub::task_owner::HubTaskHandle,
    orchestration::{
        child_launch::board::BoardStore,
        handoff::{
            HandoffStore, KIND_DONE, KIND_SESSION_END, KIND_SESSION_START, KIND_TRANSCRIPT,
            KIND_TURN_SUMMARY, Record,
        },
        initial_prompt::{
            InitialPromptCallbacks, InitialPromptDriver, InitialPromptOutcome, InitialPromptRequest,
        },
    },
    proto::{
        self,
        core::*,
        time::{Timestamp, format_rfc3339, parse_rfc3339},
    },
    terminal::session::SessionEngine,
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock, Weak},
    time::Duration,
};

mod handoff_hooks;
mod observations;
#[cfg(test)]
mod tests;
mod transcript_path;
type ActiveRoutineProbe = dyn Fn(&str, Timestamp) -> bool + Send + Sync;
type RoutineRecorder = dyn Fn(&proto::DoneSummary) -> Result<(), SessionError> + Send + Sync;
type ConductorRenderer = dyn Fn(&OrchestrationId) -> Result<String, SessionError> + Send + Sync;
type WorkflowCompletion =
    dyn Fn(SessionBinding, &proto::WorkflowProgress) -> Result<(), SessionError> + Send + Sync;
fn storage_error(detail: &str) -> SessionError {
    SessionError::Storage(StorageError {
        kind: StorageErrorKind::Write,
        detail: detail.into(),
    })
}

pub trait RegistrationHook: Send + Sync {
    fn registered(
        &self,
        binding: SessionBinding,
        metadata: &SpawnRegistrationMetadata,
    ) -> Result<(), SessionError>;
}
struct Owners {
    core: Weak<SessionEngine>,
    effects: Weak<dyn CoreEffectSink>,
}
pub struct SessionWorkers {
    config: Arc<ConfigStore>,
    paths: RuntimePaths,
    files: Arc<FilesService>,
    boards: Arc<BoardStore>,
    handoff: HandoffStore,
    tasks: HubTaskHandle,
    warning: EventWarning,
    owners: OnceLock<Owners>,
    started: std::sync::atomic::AtomicBool,
    this: Weak<SessionWorkers>,
    conductor_renderer: OnceLock<Arc<ConductorRenderer>>,
    active_routine_probe: OnceLock<Arc<ActiveRoutineProbe>>,
    routine_recorder: OnceLock<Arc<RoutineRecorder>>,
    observation_registry: OnceLock<super::spawn_policy::RegistrySnapshot>,
    observation_temporary: OnceLock<PathBuf>,
    workflow_completion: OnceLock<Arc<WorkflowCompletion>>,
}
pub struct SessionWorkerGuard {
    cancel: HubShutdownCancellation,
    join: Option<tokio::task::JoinHandle<()>>,
}
impl Drop for SessionWorkerGuard {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(join) = self.join.take() {
            join.abort();
        }
    }
}
impl SessionWorkerGuard {
    /// Stop the polling producer before draining the effect lane it feeds.
    pub async fn stop_and_join(mut self) {
        self.cancel.cancel();
        if let Some(mut join) = self.join.take()
            && tokio::time::timeout(Duration::from_secs(2), &mut join)
                .await
                .is_err()
        {
            join.abort();
            let _ = join.await;
        }
    }
}
impl SessionWorkers {
    pub fn new(
        config: Arc<ConfigStore>,
        paths: RuntimePaths,
        files: Arc<FilesService>,
        tasks: HubTaskHandle,
        warning: EventWarning,
    ) -> Arc<Self> {
        Arc::new_cyclic(|this| Self {
            config,
            boards: Arc::new(BoardStore::new(&paths)),
            handoff: HandoffStore::new(paths.clone()),
            paths,
            files,
            tasks,
            warning,
            owners: OnceLock::new(),
            started: false.into(),
            this: this.clone(),
            conductor_renderer: OnceLock::new(),
            active_routine_probe: OnceLock::new(),
            routine_recorder: OnceLock::new(),
            observation_registry: OnceLock::new(),
            observation_temporary: OnceLock::new(),
            workflow_completion: OnceLock::new(),
        })
    }
    pub fn set_observation_registry(
        &self,
        registry: super::spawn_policy::RegistrySnapshot,
    ) -> Result<(), SessionError> {
        self.observation_registry
            .set(registry)
            .map_err(|_| SessionError::InvalidRequest("observation registry already bound".into()))
    }
    pub fn set_observation_temporary(&self, native_temporary: &Path) -> Result<(), SessionError> {
        let temporary = self.paths.usage_hook_temporary_dir(native_temporary);
        self.observation_temporary.set(temporary).map_err(|_| {
            SessionError::InvalidRequest("observation temporary root already bound".into())
        })
    }
    pub fn set_workflow_completion(
        &self,
        callback: Arc<WorkflowCompletion>,
    ) -> Result<(), SessionError> {
        self.workflow_completion.set(callback).map_err(|_| {
            SessionError::InvalidRequest("workflow completion callback already bound".into())
        })
    }
    pub fn set_conductor_renderer(
        &self,
        renderer: Arc<ConductorRenderer>,
    ) -> Result<(), SessionError> {
        self.conductor_renderer
            .set(renderer)
            .map_err(|_| SessionError::InvalidRequest("conductor renderer already bound".into()))
    }
    pub fn set_active_routine_probe(
        &self,
        probe: Arc<ActiveRoutineProbe>,
    ) -> Result<(), SessionError> {
        self.active_routine_probe
            .set(probe)
            .map_err(|_| SessionError::InvalidRequest("routine probe already bound".into()))
    }
    pub fn set_routine_recorder(&self, recorder: Arc<RoutineRecorder>) -> Result<(), SessionError> {
        self.routine_recorder
            .set(recorder)
            .map_err(|_| SessionError::InvalidRequest("routine recorder already bound".into()))
    }
    pub(crate) fn enqueue_initial_request(
        &self,
        request: InitialPromptRequest,
    ) -> Result<(), SessionError> {
        let driver = Arc::new(InitialPromptDriver::new(
            Arc::downgrade(&self.core()?),
            self.this.upgrade().ok_or(SessionError::Shutdown)?,
        ));
        drop(driver.start(self.tasks.effect_permit()?, request));
        Ok(())
    }
    pub(crate) fn conductor_prompt(id: &str) -> String {
        transcript_path::conductor_prompt(id)
    }
    pub fn enqueue_child_prompt(
        &self,
        child: &crate::orchestration::child_launch::RegisteredChild,
    ) -> Result<(), SessionError> {
        if let Some(request) = InitialPromptRequest::for_child(child) {
            let driver = Arc::new(InitialPromptDriver::new(
                Arc::downgrade(&self.core()?),
                self.this.upgrade().ok_or(SessionError::Shutdown)?,
            ));
            drop(driver.start(self.tasks.effect_permit()?, request));
        }
        Ok(())
    }
    pub fn bind(
        &self,
        core: Weak<SessionEngine>,
        effects: Weak<dyn CoreEffectSink>,
    ) -> Result<(), SessionError> {
        self.owners
            .set(Owners { core, effects })
            .map_err(|_| SessionError::InvalidRequest("session worker owners already bound".into()))
    }
    fn core(&self) -> Result<Arc<SessionEngine>, SessionError> {
        self.owners
            .get()
            .and_then(|owners| owners.core.upgrade())
            .ok_or(SessionError::Shutdown)
    }
    async fn apply(&self, effects: CoreEffects) -> Result<(), SessionError> {
        let sink = self
            .owners
            .get()
            .and_then(|owners| owners.effects.upgrade())
            .ok_or(SessionError::Shutdown)?;
        sink.apply(effects).await.map_err(|failure| failure.error)
    }
    fn config(&self) -> Result<crate::config::Config, SessionError> {
        self.config
            .snapshot()
            .map(|s| s.config)
            .map_err(|_| storage_error("configuration snapshot unavailable"))
    }
    fn append_handoff(
        &self,
        binding: SessionBinding,
        mut record: Record,
    ) -> Result<(), SessionError> {
        if !self.config()?.handoff.enabled_or_default() {
            return Ok(());
        }
        let core = self.core()?;
        let Some(details) = core.details(binding.session) else {
            return Err(SessionError::StaleBinding);
        };
        if details.binding.incarnation != binding.incarnation {
            return Err(SessionError::StaleBinding);
        }
        record.provider = details.snapshot.provider;
        record.cwd = details.snapshot.cwd;
        record.branch = details.snapshot.branch;
        record.model = details.snapshot.model;
        record.subscription_id = details.snapshot.subscription_profile_id;
        self.handoff
            .append(binding.session.0, record, Timestamp::now())
            .map_err(|_| storage_error("handoff record append failed"))
    }
    pub fn start(self: &Arc<Self>) -> Result<SessionWorkerGuard, SessionError> {
        let core = self.core()?;
        if self.started.swap(true, std::sync::atomic::Ordering::AcqRel) {
            return Err(SessionError::InvalidRequest(
                "session workers already started".into(),
            ));
        }
        // Subscribe synchronously before returning; listening cannot race the
        // first registration. Polling starts immediately, matching Go timers.
        let events = core.subscribe();
        let cancel = HubShutdownCancellation::default();
        let task_cancel = cancel.clone();
        let weak = Arc::downgrade(self);
        let join = tokio::spawn(async move {
            run(weak, events, task_cancel).await;
        });
        Ok(SessionWorkerGuard {
            cancel,
            join: Some(join),
        })
    }
    async fn event(&self, event: CoreEvent) -> Result<(), SessionError> {
        match event {
            CoreEvent::Registered(binding) => self.append_handoff(
                binding,
                Record {
                    kind: KIND_SESSION_START.into(),
                    ..Default::default()
                },
            ),
            CoreEvent::TurnSummary {
                binding,
                turn,
                text,
            } => {
                if self.config()?.handoff.turn_summary_enabled() {
                    self.append_handoff(
                        binding,
                        Record {
                            kind: KIND_TURN_SUMMARY.into(),
                            turn,
                            text,
                            ..Default::default()
                        },
                    )
                } else {
                    Ok(())
                }
            }
            CoreEvent::TranscriptChanged { binding, path, .. } => self.append_handoff(
                binding,
                Record {
                    kind: KIND_TRANSCRIPT.into(),
                    transcript: path.to_string_lossy().into_owned(),
                    ..Default::default()
                },
            ),
            CoreEvent::Ended { binding, .. } => self.append_handoff(
                binding,
                Record {
                    kind: KIND_SESSION_END.into(),
                    ..Default::default()
                },
            ),
            CoreEvent::OutputObserved { binding, clean, at } => {
                let core = self.core()?;
                for message in self
                    .files
                    .observe_commit_output(core.as_ref(), binding, &clean, at)
                {
                    self.apply(core.broadcast_ui(message)).await?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
    /// Called by the ordered observer before publishing a completion frame.
    pub fn record_completed(
        &self,
        binding: SessionBinding,
        summary: &proto::DoneSummary,
    ) -> Result<(), SessionError> {
        let text = if summary.kind.trim().is_empty() {
            summary.text.clone()
        } else {
            format!("[{}] {}", summary.kind.trim(), summary.text)
        };
        self.append_handoff(
            binding,
            Record {
                kind: KIND_DONE.into(),
                text,
                ..Default::default()
            },
        )?;
        use std::sync::LazyLock;
        static NEXT: LazyLock<regex::Regex> = LazyLock::new(|| {
            regex::Regex::new(r"(?:^|(?-u:\s))次:(?-u:\s)*(.+?)(?:(?-u:\s)+未検証:|$)").unwrap()
        });
        static UNVERIFIED: LazyLock<regex::Regex> =
            LazyLock::new(|| regex::Regex::new(r"(?:^|(?-u:\s))未検証:(?-u:\s)*(.+)$").unwrap());
        let next = NEXT
            .captures(&summary.text)
            .map(|matched| matched[1].trim().to_owned())
            .unwrap_or_default();
        let unverified = UNVERIFIED
            .captures(&summary.text)
            .map(|matched| matched[1].trim().to_owned())
            .unwrap_or_default();
        if !next.is_empty() || !unverified.is_empty() {
            let text = match (next.is_empty(), unverified.is_empty()) {
                (false, false) => format!("次: {next} / 未検証: {unverified}"),
                (false, true) => format!("次: {next}"),
                _ => format!("未検証: {unverified}"),
            };
            self.append_handoff(
                binding,
                Record {
                    kind: crate::orchestration::handoff::KIND_INTENT.into(),
                    text,
                    ..Default::default()
                },
            )?;
        }
        Ok(())
    }
    async fn poll(
        &self,
        cursors: &mut BTreeMap<LiveSessionId, Cursor>,
    ) -> Result<(), SessionError> {
        let core = self.core()?;
        self.apply(core.evaluate_idle(Timestamp::now())).await?;
        let ids = core.active_ids();
        cursors.retain(|id, _| ids.contains(id));
        for id in ids {
            let Some(details) = core.details(id) else {
                continue;
            };
            if !details.connected || core.is_usage_probe(details.binding)? {
                continue;
            }
            let Some(format) = ProviderFormat::for_provider(&details.snapshot.provider) else {
                continue;
            };
            if let Some(cursor) = cursors.get(&id)
                && cursor.primed
                && details.snapshot.provider == "codex"
                && let Some((identity, peers)) =
                    core.codex_thread_follow_view(details.binding, Timestamp::now())?
            {
                let (next, ambiguous) = transcript_path::thread_switch(
                    &self.paths,
                    &identity,
                    &cursor.path,
                    &peers,
                    Timestamp::now(),
                );
                core.apply_codex_thread_follow(details.binding, &cursor.path, next, ambiguous)?;
            }
            let Some(details) = core.details(id) else {
                continue;
            };
            let Some(path) = transcript_path::resolve(&self.paths, &details.transcript) else {
                if core.observe_transcript_miss(details.binding)? {
                    (self.warning)(
                        "approval marker source fell back to VT",
                        &SessionError::Transport(
                            "provider transcript unresolved for three polls".into(),
                        ),
                    );
                }
                continue;
            };
            // The selected path is a discovery hint. Reads authorize by a held
            // selected-root directory capability, never a check-then-reopen path.
            let file = match crate::application::session_observations::subagents::open_artifact(
                &self.paths,
                &path,
            ) {
                Ok(file) => file,
                Err(_) => {
                    if core.observe_transcript_miss(details.binding)? {
                        (self.warning)(
                            "approval marker source fell back to VT",
                            &storage_error("provider transcript is unreadable"),
                        );
                    }
                    continue;
                }
            };
            let cursor = cursors
                .entry(id)
                .or_insert_with(|| Cursor::new(details.binding, path.clone()));
            if cursor.binding != details.binding || cursor.path != path {
                *cursor = Cursor::new(details.binding, path.clone());
            }
            let prime = !cursor.primed;
            let messages = if prime {
                cursor
                    .parse
                    .read_tail_file(file, format, None, &ReadBudget::page())
            } else {
                cursor
                    .parse
                    .read_forward_file(file, format, &ReadBudget::live())
            };
            let messages = match messages {
                Ok(messages) => messages,
                Err(_) => {
                    if core.observe_transcript_miss(details.binding)? {
                        (self.warning)(
                            "approval marker source fell back to VT",
                            &storage_error("provider transcript is unreadable"),
                        );
                    }
                    continue;
                }
            };
            if prime && !cursor.parse.adopt_prime_cursor() {
                continue;
            }
            cursor.primed = true;
            if !core.is_current(details.binding) {
                continue;
            }
            let now = Timestamp::now();
            let safe = cursor.parse.read_state.safe_offset;
            if cursor.published_path.as_ref() != Some(&path) || safe != cursor.published_offset {
                let effects = core.apply_observation(
                    details.binding,
                    SessionObservation::Transcript {
                        path: path.clone(),
                        agent_session_id: details.transcript.agent_session_id.clone(),
                        safe_offset: i64::try_from(safe).unwrap_or(i64::MAX),
                        grew_at: format_rfc3339(now).map_err(|_| {
                            SessionError::InvalidRequest(
                                "invalid transcript growth timestamp".into(),
                            )
                        })?,
                    },
                    now,
                )?;
                self.apply(effects).await?;
                cursor.published_path = Some(path.clone());
                cursor.published_offset = safe;
            }
            // Authoritative approval admission precedes UI delivery. A prime
            // reconstructs existing view, never a new Codex completion signal.
            self.apply(core.observe_transcript_batch(
                details.binding,
                path.clone(),
                &messages,
                prime,
                now,
            )?)
            .await?;
            for message in messages {
                let wire = serde_json::to_vec(&message)
                    .map_err(|_| storage_error("transcript message encoding failed"))?;
                let decoded = crate::proto::wire::decode::<proto::AgentChatMessage>(&wire)
                    .map_err(|_| storage_error("transcript message wire conversion failed"))?;
                self.apply(core.broadcast_ui(proto::Message {
                    r#type: "agent_chat".into(),
                    session_id: id.0,
                    provider: details.snapshot.provider.clone(),
                    messages: vec![decoded],
                    ..Default::default()
                }))
                .await?;
            }
            if !prime {
                for completion in cursor.parse.take_completions() {
                    if let Ok(at) = parse_rfc3339(&completion.at) {
                        // A confirmed Git work predicate must be established by
                        // completion capture; provider transcript end alone
                        // never turns ordinary conversation into a DONE card.
                        let summary = proto::DoneSummary {
                            session_id: id.0,
                            provider: "codex".into(),
                            text: completion.last_agent_message,
                            kind: "unknown".into(),
                            at: completion.at,
                            ..Default::default()
                        };
                        let label = details.snapshot.launch_label.clone();
                        let active = self
                            .active_routine_probe
                            .get()
                            .is_some_and(|probe| probe(&label, at));
                        self.apply(core.observe_codex_completion_with_routine(
                            details.binding,
                            summary,
                            at,
                            &label,
                            active,
                        )?)
                        .await?;
                    }
                }
            }
        }
        Ok(())
    }
}
struct Cursor {
    binding: SessionBinding,
    path: PathBuf,
    parse: ParseState,
    primed: bool,
    published_path: Option<PathBuf>,
    published_offset: u64,
}
impl Cursor {
    fn new(binding: SessionBinding, path: PathBuf) -> Self {
        Self {
            binding,
            path,
            parse: ParseState::default(),
            primed: false,
            published_path: None,
            published_offset: 0,
        }
    }
}
async fn run(
    weak: Weak<SessionWorkers>,
    mut events: Box<dyn CoreEventSubscription>,
    cancel: HubShutdownCancellation,
) {
    let mut tick = tokio::time::interval(Duration::from_millis(200));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut cursors = BTreeMap::new();
    let mut observations = observations::Collector::default();
    let mut transcript_tick = 0;
    loop {
        tokio::select! { biased;
            _ = cancel.token().cancelled() => break,
            event = events.next(&cancel) => {
                let Some(worker) = weak.upgrade() else { break; };
                match event {
                    CoreEventPoll::Event(event) => {
                        observations.event(&worker, &event.event);
                        if let Err(error) = worker.event(event.event).await { (worker.warning)("session worker event", &error); }
                    },
                    CoreEventPoll::Lagged {..} => { (worker.warning)("session worker event lag", &SessionError::Transport("completion evidence was lost; do not replay completed events".into())); cursors.clear(); },
                    CoreEventPoll::Cancelled | CoreEventPoll::Closed => break,
                }
            },
            _ = tick.tick() => {
                let Some(worker) = weak.upgrade() else { break; };
                transcript_tick += 1;
                let result = if transcript_tick == 5 { transcript_tick = 0; worker.poll(&mut cursors).await } else {
                    match worker.core() {Ok(core) => worker.apply(core.evaluate_idle(Timestamp::now())).await, Err(error) => Err(error)}
                };
                if let Err(error) = result { (worker.warning)("session worker poll", &error); }
                if let Err(error) = observations.poll(&worker, Timestamp::now()).await {
                    (worker.warning)("session observations", &error);
                }
            }
        }
    }
}
impl RegistrationHook for SessionWorkers {
    fn registered(
        &self,
        binding: SessionBinding,
        metadata: &SpawnRegistrationMetadata,
    ) -> Result<(), SessionError> {
        self.core()?.note_registration_prompt(binding, metadata)?;
        let request =
            InitialPromptRequest::for_registration(binding, metadata, |orchestration| match self
                .conductor_renderer
                .get()
            {
                Some(renderer) => renderer(orchestration),
                None => Ok(transcript_path::conductor_prompt(&orchestration.0)),
            })?;
        if let Some(request) = request {
            let driver = Arc::new(InitialPromptDriver::new(
                Arc::downgrade(&self.core()?),
                self.this.upgrade().ok_or(SessionError::Shutdown)?,
            ));
            drop(driver.start(self.tasks.effect_permit()?, request));
        }
        Ok(())
    }
}
impl InitialPromptCallbacks for SessionWorkers {
    fn record_outcome(
        &self,
        binding: SessionBinding,
        outcome: &InitialPromptOutcome,
        at: Timestamp,
    ) -> Result<(), SessionError> {
        self.core()?
            .record_initial_prompt_outcome(binding, outcome, at)
    }
    fn append_board_failure<'a>(
        &'a self,
        path: &'a Path,
        text: String,
        cancel: &'a TaskCancellation,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            if cancel.token().is_cancelled() {
                return Err(SessionError::Cancelled);
            }
            if self.paths.is_trial() {
                transcript_path::check_trial_path(&self.paths, path)?;
            }
            self.boards
                .append(path, "hub", &text, Timestamp::now())
                .map_err(|_| storage_error("initial prompt failure board append failed"))
        })
    }
    fn notify_parent_failure<'a>(
        &'a self,
        parent: SessionBinding,
        limit: &'static str,
        detail: String,
        cancel: &'a TaskCancellation,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            let core = self.core()?;
            let receipt = core.submit(parent, InputRequest {bytes: format!("\x1b[200~[MANY-AI-CLI-ORCHESTRATION-ERROR] {limit}: {detail}\x1b[201~\r").into_bytes(), authority: InputAuthority::Internal}, Timestamp::now(), cancel).await;
            match receipt.disposition {
                InputDisposition::TransportWritten { .. } | InputDisposition::Deferred { .. } => {
                    Ok(())
                }
                _ => Err(SessionError::Transport(
                    "parent startup failure notice could not be delivered".into(),
                )),
            }
        })
    }
    fn apply_effects<'a>(
        &'a self,
        effects: CoreEffects,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(self.apply(effects))
    }
    fn warning(&self, operation: &'static str, error: &SessionError) {
        (self.warning)(operation, error);
    }
}
impl GitTurnCompletionCallbacks for SessionWorkers {
    fn routine_completed<'a>(
        &'a self,
        _binding: SessionBinding,
        summary: &'a proto::DoneSummary,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move { self.routine_recorder.get().ok_or(SessionError::Shutdown)?(summary) })
    }
    fn handoff_note_written<'a>(
        &'a self,
        binding: SessionBinding,
        path: &'a Path,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move { self.finish_handoff_note(binding, path).await })
    }
    fn before_done_publish<'a>(
        &'a self,
        binding: SessionBinding,
        summary: &'a proto::DoneSummary,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move { self.record_completed(binding, summary) })
    }
    fn inject_turn_summary<'a>(
        &'a self,
        binding: SessionBinding,
        turn: i64,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            let cfg = self.config()?;
            if !cfg.handoff.enabled_or_default() || !cfg.handoff.turn_summary_enabled() {
                return Ok(());
            }
            let core = self.core()?;
            if !core.begin_turn_summary(binding, turn, Timestamp::now())? {
                return Ok(());
            }
            let prompt = transcript_path::turn_summary_prompt(
                cfg.user_prefs.display.lang.is_empty()
                    || cfg.user_prefs.display.lang.eq_ignore_ascii_case("ja"),
            );
            let receipt = core
                .submit(
                    binding,
                    InputRequest {
                        bytes: format!("\x1b[200~{prompt}\x1b[201~\r").into_bytes(),
                        authority: InputAuthority::Internal,
                    },
                    Timestamp::now(),
                    &TaskCancellation::default(),
                )
                .await;
            match receipt.disposition {
                InputDisposition::TransportWritten { .. } | InputDisposition::Deferred { .. } => {
                    Ok(())
                }
                _ => Err(SessionError::Transport(
                    "turn summary prompt delivery failed".into(),
                )),
            }
        })
    }
    fn after_git_turn_broadcast<'a>(
        &'a self,
        binding: SessionBinding,
        turn: &'a GitTurnSnapshot,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            let core = self.core()?;
            self.apply(core.completion_after_git(binding, turn.files, &turn.ended_at)?)
                .await
        })
    }
}
