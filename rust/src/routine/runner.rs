//! Routine tasks reuse the actual core admission/spawn boundary. The caller must
//! supply registry/pending-prompt preparation; there is no default launcher.
use super::{
    model::Run,
    store::{Admission, Error, Observation, RoutineStore},
    validation,
};
use crate::proto::time::Timestamp;
use crate::proto::{DoneSummary, core::*};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

/// Production implementation belongs to the shared launch/orchestration wiring.
/// It selects argument versus post-registration prompt delivery and records the
/// latter under the immutable run label before returning the shared spawn spec.
/// `finish` removes pending launch metadata on failure, matching the Go caller.
/// Neither method may allocate a second session/admission owner or fake success.
pub trait RoutineLaunchPreparation: Send + Sync {
    fn prepare<'a>(
        &'a self,
        run: &'a Run,
        prompt: String,
        cancellation: TaskCancellation,
    ) -> CoreFuture<'a, Result<WrappedSpawnSpec, Error>>;
    fn finish(&self, run: &Run, outcome: &SpawnWaitOutcome);
}
pub type Warning = Arc<dyn Fn(&'static str, &Error) + Send + Sync>;
pub struct RoutineRunner {
    store: Arc<RoutineStore>,
    core: Arc<dyn SessionCore>,
    preparation: Option<Arc<dyn RoutineLaunchPreparation>>,
    home: PathBuf,
    instance: String,
    cancellation: TaskCancellation,
    now: Arc<dyn Fn() -> Timestamp + Send + Sync>,
    warning: Warning,
    tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
}
impl RoutineRunner {
    pub fn new(
        store: Arc<RoutineStore>,
        core: Arc<dyn SessionCore>,
        home: PathBuf,
        instance: String,
        cancellation: TaskCancellation,
        warning: Warning,
    ) -> Self {
        Self {
            store,
            core,
            preparation: None,
            home,
            instance,
            cancellation,
            now: Arc::new(Timestamp::now),
            warning,
            tasks: Mutex::new(vec![]),
        }
    }
    pub fn with_preparation(mut self, preparation: Arc<dyn RoutineLaunchPreparation>) -> Self {
        self.preparation = Some(preparation);
        self
    }
    pub fn start(
        self: &Arc<Self>,
        id: &str,
        request: &str,
        trigger: &str,
        now: Timestamp,
    ) -> Result<Admission, Error> {
        let preparation = self.preparation.clone().ok_or(Error::LaunchUnavailable)?;
        let runtime =
            tokio::runtime::Handle::try_current().map_err(|_| Error::LaunchUnavailable)?;
        let admitted = self.store.admit(id, request, trigger, now)?;
        if !admitted.existing {
            let runner = self.clone();
            let run = admitted.run.clone();
            let task = runtime.spawn(async move {
                runner.launch(run, preparation).await;
            });
            let mut tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
            tasks.retain(|t| !t.is_finished());
            tasks.push(task);
        }
        Ok(admitted)
    }
    async fn launch(&self, run: Run, preparation: Arc<dyn RoutineLaunchPreparation>) {
        let mut definition = super::model::Definition {
            name: run.routine_name.clone(),
            cwd: run.cwd.clone(),
            provider: run.provider.clone(),
            model: run.model.clone(),
            prompt: run.prompt.clone(),
            schedule: super::schedule::Schedule {
                kind: "manual".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        let outcome = if validation::validate(&mut definition, &self.home, (self.now)()).is_err() {
            SpawnWaitOutcome::Failed("routine definition is no longer valid".into())
        } else {
            match preparation
                .prepare(&run, prompt(&run.prompt), self.cancellation.clone())
                .await
            {
                Ok(mut spec)
                    if spec.provider == run.provider
                        && spec.cwd == std::path::Path::new(&run.cwd)
                        && spec.model == run.model
                        && spec.label == run.session_label
                        && spec.spawn_attempt.is_none()
                        && spec.registration_proof.is_none() =>
                {
                    // Core owns admission/proof. This waiter is service-owned,
                    // independent of whichever HTTP request admitted the run.
                    let waiter = HttpWaitCancellation::default();
                    spec.cancellation = self.cancellation.clone();
                    self.core
                        .spawn_and_wait(spec, Duration::from_secs(30), &waiter)
                        .await
                }
                Ok(_) => SpawnWaitOutcome::Failed(
                    "routine launch preparation changed its immutable binding".into(),
                ),
                Err(_) => SpawnWaitOutcome::Failed("routine launch preparation failed".into()),
            }
        };
        preparation.finish(&run, &outcome);
        let session = match outcome {
            SpawnWaitOutcome::Registered(binding) => Ok(binding.session),
            _ => Err(()),
        };
        if let Err(error) = self
            .store
            .launched(&run.id, session, &self.instance, (self.now)())
        {
            (self.warning)("save routine launch state", &error);
        }
    }
    pub fn tick(self: &Arc<Self>, now: Timestamp, startup: bool) -> Result<(), Error> {
        for definition in self.store.definitions()? {
            if !definition.enabled
                || definition.schedule.kind == "manual"
                || definition.next_run_at.is_empty()
            {
                continue;
            }
            let Ok(due) = crate::proto::time::parse_rfc3339(&definition.next_run_at) else {
                continue;
            };
            let Ok(late) = now.duration_since(due) else {
                continue;
            };
            if startup || late > Duration::from_secs(60) {
                if let Err(error) = self.store.skip_schedule(
                    &definition,
                    now,
                    "The scheduled time passed while the Hub was unavailable.",
                ) {
                    (self.warning)("save skipped routine schedule", &error);
                }
                continue;
            }
            match self.start(
                &definition.id,
                &format!("schedule:{}", definition.next_run_at),
                "schedule",
                now,
            ) {
                Ok(admission) if admission.existing => {
                    if let Err(error) = self.store.skip_schedule(
                        &definition,
                        now,
                        "The previous run was still active at the scheduled time.",
                    ) {
                        (self.warning)("save skipped active routine", &error);
                    }
                }
                Ok(_) | Err(Error::Missing | Error::ScheduleChanged) => {}
                Err(error) => (self.warning)("start routine schedule", &error),
            }
        }
        Ok(())
    }
    pub fn refresh(&self, now: Timestamp) -> Result<(), Error> {
        let observations = self
            .core
            .snapshots()
            .into_iter()
            .filter_map(|snapshot| {
                let details = self.core.details(snapshot.id)?;
                Some(Observation {
                    id: details.snapshot.id,
                    db_id: details.db_id,
                    launch_label: details.snapshot.launch_label,
                    state: details.snapshot.state,
                    waiting: details.approval.record.is_some()
                        || details.snapshot.activity.awaiting_user,
                    last_output: details.last_output_at,
                })
            })
            .collect::<Vec<_>>();
        self.store.refresh(&observations, &self.instance, now)
    }
    pub fn record_done(&self, summary: &DoneSummary) -> Result<(), Error> {
        let Some(details) = self.core.details(LiveSessionId(summary.session_id)) else {
            return Ok(());
        };
        self.store.record_done(
            &details.snapshot.launch_label,
            details.db_id,
            summary,
            &self.instance,
        )
    }
    pub fn active_session(&self, id: LiveSessionId, at: Timestamp) -> bool {
        self.core
            .snapshot(id)
            .is_some_and(|snapshot| self.store.active_session(&snapshot.launch_label, at))
    }
    pub fn run_url(&self, id: LiveSessionId) -> Option<String> {
        self.core
            .snapshot(id)
            .and_then(|snapshot| self.store.run_url(&snapshot.launch_label))
    }
    pub async fn run(self: Arc<Self>) {
        if !self.store.ready() {
            return;
        }
        if let Err(error) = self.tick((self.now)(), true) {
            (self.warning)("initial routine schedule", &error);
        }
        let mut timer = tokio::time::interval_at(
            tokio::time::Instant::now() + Duration::from_secs(2),
            Duration::from_secs(2),
        );
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _=self.cancellation.token().cancelled()=>break,
                _=timer.tick()=>{
                    let now=(self.now)();
                    if let Err(error)=self.refresh(now) {(self.warning)("refresh routine runs",&error);}
                    if let Err(error)=self.tick(now,false) {(self.warning)("tick routine schedules",&error);}
                }
            }
        }
    }
    /// Join admitted jobs at shutdown while their shared task cancellation is
    /// already signaled; an HTTP caller never calls this or cancels these jobs.
    pub async fn join_launches(&self) {
        let tasks = std::mem::take(&mut *self.tasks.lock().unwrap_or_else(|e| e.into_inner()));
        for task in tasks {
            let _ = task.await;
        }
    }
}
pub fn prompt(text: &str) -> String {
    format!(
        "{text}\n\nWhen this routine finishes or needs user action, end your response with [MANY-AI-CLI-DONE] a short factual result, including any unfinished work or unperformed checks [/MANY-AI-CLI-DONE]. Do not claim success from merely reaching the end of the response."
    )
}
#[cfg(test)]
#[path = "runner_tests.rs"]
mod tests;
