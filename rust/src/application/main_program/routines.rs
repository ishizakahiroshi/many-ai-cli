//! Real routine launch preparation, completion persistence and scheduler owner.
use crate::{
    application::{
        event_observer::{EventWarning, GitTurnCompletionCallbacks},
        orchestration_program::OrchestrationProgram,
        session_workers::SessionWorkers,
    },
    config::{self, RuntimePaths},
    files::GitTurnSnapshot,
    orchestration::child_launch::ChildLaunchServices,
    proto::{self, core::*},
    routine::{
        model::Run,
        runner::{RoutineLaunchPreparation, RoutineRunner},
        store::Error,
    },
};
use std::sync::{Arc, OnceLock, Weak};
pub struct RoutinePreparation {
    pub paths: RuntimePaths,
    pub orchestration: Arc<OrchestrationProgram>,
    pub warning: EventWarning,
}
impl RoutineLaunchPreparation for RoutinePreparation {
    fn prepare<'a>(
        &'a self,
        run: &'a Run,
        prompt: String,
        cancellation: TaskCancellation,
    ) -> CoreFuture<'a, Result<WrappedSpawnSpec, Error>> {
        Box::pin(async move {
            crate::profile::subscriptions::check_path(&self.paths, std::path::Path::new(&run.cwd))
                .map_err(|_| Error::Invalid("routine cwd outside trial".into()))?;
            if cancellation.token().is_cancelled() {
                return Err(Error::LaunchUnavailable);
            }
            let launch = config::launch_prompt_via_arg(&run.provider)
                && self
                    .orchestration
                    .launch_arg_usable(&run.provider)
                    .map_err(|_| Error::LaunchUnavailable)?;
            let metadata = SpawnRegistrationMetadata {
                spawned_at: Some(crate::proto::time::Timestamp::now()),
                prompt_at_launch: launch,
                initial_prompt: if launch {
                    String::new()
                } else {
                    prompt.clone()
                },
                ..Default::default()
            };
            Ok(WrappedSpawnSpec {
                registration_metadata: metadata,
                spawn_attempt: None,
                registration_proof: None,
                provider: run.provider.clone(),
                cwd: run.cwd.clone().into(),
                model: run.model.clone(),
                model_selection: String::new(),
                risk_confirmed: false,
                label: run.session_label.clone(),
                permission_mode: String::new(),
                sandbox: String::new(),
                ask_for_approval: String::new(),
                route: String::new(),
                utf8_session: false,
                effort: String::new(),
                execution_mode: String::new(),
                permission_preset: String::new(),
                initial_prompt: if launch { prompt } else { String::new() },
                subscription_profile_id: String::new(),
                subscription_login: false,
                usage_probe: false,
                grants: InternalSpawnGrants::default(),
                cancellation,
            })
        })
    }
    fn finish(&self, _run: &Run, outcome: &SpawnWaitOutcome) {
        // Metadata lives exclusively in the core's proof-bound spawn transaction,
        // whose terminal failure cleanup has already run before this callback.
        // This adapter owns no label map, prompt file or native process to clean.
        if !matches!(outcome, SpawnWaitOutcome::Registered(_)) {
            (self.warning)(
                "routine launch failed",
                &SessionError::InvalidRequest("AI session did not register".into()),
            );
        }
    }
}
pub struct RoutineCallbacks {
    workers: Arc<SessionWorkers>,
    runner: OnceLock<Weak<RoutineRunner>>,
    warning: EventWarning,
}
impl RoutineCallbacks {
    pub fn new(workers: Arc<SessionWorkers>, warning: EventWarning) -> Arc<Self> {
        Arc::new(Self {
            workers,
            runner: OnceLock::new(),
            warning,
        })
    }
    pub fn bind(&self, runner: &Arc<RoutineRunner>) -> Result<(), SessionError> {
        self.runner.set(Arc::downgrade(runner)).map_err(|_| {
            SessionError::InvalidRequest("routine completion owner already bound".into())
        })
    }
}
impl GitTurnCompletionCallbacks for RoutineCallbacks {
    fn routine_completed<'a>(
        &'a self,
        _binding: SessionBinding,
        summary: &'a proto::DoneSummary,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            let runner = self
                .runner
                .get()
                .and_then(Weak::upgrade)
                .ok_or(SessionError::Shutdown)?;
            if runner.record_done(summary).is_err() {
                (self.warning)(
                    "routine DONE persistence",
                    &SessionError::InvalidRequest("routine result save unavailable".into()),
                );
            }
            Ok(())
        })
    }
    fn handoff_note_written<'a>(
        &'a self,
        binding: SessionBinding,
        path: &'a std::path::Path,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        self.workers.handoff_note_written(binding, path)
    }
    fn before_done_publish<'a>(
        &'a self,
        binding: SessionBinding,
        summary: &'a proto::DoneSummary,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            // Save the complete result before handoff/UI/notification. A disk failure
            // remains retryable inside the real RoutineStore and does not veto DONE.
            self.routine_completed(binding, summary).await?;
            self.workers.before_done_publish(binding, summary).await
        })
    }
    fn inject_turn_summary<'a>(
        &'a self,
        binding: SessionBinding,
        turn: i64,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        self.workers.inject_turn_summary(binding, turn)
    }
    fn after_git_turn_broadcast<'a>(
        &'a self,
        binding: SessionBinding,
        turn: &'a GitTurnSnapshot,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        self.workers.after_git_turn_broadcast(binding, turn)
    }
}
pub struct RoutineGuard {
    cancellation: TaskCancellation,
    join: Option<tokio::task::JoinHandle<()>>,
}
impl RoutineGuard {
    pub fn start(
        runner: Arc<RoutineRunner>,
        cancellation: TaskCancellation,
        service_stop: crate::process::Cancellation,
    ) -> Self {
        let cancelled = cancellation.clone();
        Self {
            cancellation,
            join: Some(tokio::spawn(async move {
                let run = runner.run();
                tokio::pin!(run);
                tokio::select! { _=service_stop.cancelled()=>{cancelled.cancel();run.await;},_= &mut run=>{} }
            })),
        }
    }
    pub async fn stop_and_join(mut self) {
        self.cancellation.cancel();
        if let Some(join) = self.join.take() {
            let _ = join.await;
        }
    }
}
impl Drop for RoutineGuard {
    fn drop(&mut self) {
        self.cancellation.cancel();
        if let Some(join) = self.join.take() {
            join.abort();
        }
    }
}
