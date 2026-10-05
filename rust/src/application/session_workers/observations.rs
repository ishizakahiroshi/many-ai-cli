//! Retained parser continuations keyed by the core's immutable incarnation.
//! Output queues work; bounded parsing runs outside the canonical session lock.
use super::*;
use crate::application::session_observations::{
    journal_state::{PollContext, Watcher},
    task_detail, workflow,
    workflow_state::{self, VtState},
};
mod native;

struct Continuation {
    binding: SessionBinding,
    vt: VtState,
    journal: Watcher,
    detail: TaskDetail,
    native: native::Native,
}
impl Continuation {
    fn new(binding: SessionBinding) -> Self {
        Self {
            binding,
            vt: VtState::default(),
            journal: Watcher::default(),
            detail: TaskDetail::default(),
            native: native::Native::default(),
        }
    }
    fn rebind(&mut self, binding: SessionBinding) {
        if self.binding.session != binding.session
            || self.binding.incarnation != binding.incarnation
        {
            *self = Self::new(binding);
        } else {
            self.binding = binding;
        }
    }
}
#[derive(Default)]
struct TaskDetail {
    unavailable: bool,
    attempts: usize,
    run: Option<PathBuf>,
    directory: Option<PathBuf>,
    task: Option<String>,
    due: Option<Timestamp>,
    file: task_detail::FileState,
    progress: Option<proto::WorkflowProgress>,
}
impl TaskDetail {
    fn stop(&mut self) {
        let unavailable = self.unavailable;
        *self = Self {
            unavailable,
            ..Default::default()
        };
    }
    fn start(&mut self, run: PathBuf, directory: PathBuf, now: Timestamp) {
        if self.unavailable {
            return;
        }
        self.stop();
        self.run = Some(run);
        self.directory = Some(directory);
        self.due = Some(now);
    }
    fn poll(
        &mut self,
        worker: &SessionWorkers,
        cwd: &str,
        journal_settled: bool,
        now: Timestamp,
    ) -> Result<bool, SessionError> {
        if self.due.is_none_or(|due| due > now) {
            return Ok(false);
        }
        self.due = None;
        let (Some(run), Some(directory)) = (&self.run, &self.directory) else {
            return Ok(false);
        };
        if self.task.is_none() {
            self.attempts += 1;
            let mut transcript = directory.as_os_str().to_os_string();
            transcript.push(".jsonl");
            self.task = task_detail::resolve(
                &worker.paths,
                Path::new(&transcript),
                &run.to_string_lossy(),
            );
            if self.task.is_none() {
                if self.attempts >= 5 {
                    self.unavailable = true;
                } else {
                    self.due = now.checked_add(Duration::from_secs(1));
                }
                return Ok(false);
            }
        }
        let temporary = worker.observation_temporary.get().ok_or_else(|| {
            SessionError::InvalidRequest("observation temporary root is not bound".into())
        })?;
        let Some(session) = directory.file_name() else {
            return Ok(false);
        };
        let output = temporary
            .join("claude")
            .join(project_name(cwd))
            .join(session)
            .join("tasks")
            .join(format!("{}.output", self.task.as_deref().unwrap()));
        if let Ok((file, _)) = task_detail::poll(&worker.paths, &output, self.file.clone()) {
            self.file = file;
            if self.file.loaded
                && let Some(progress) = task_detail::build(&self.file.entries)
            {
                self.progress = Some(progress);
            }
        }
        if !journal_settled {
            self.due = now.checked_add(Duration::from_secs(4));
        }
        Ok(true)
    }
}
fn project_name(cwd: &str) -> String {
    let mut clean = PathBuf::new();
    for component in Path::new(cwd).components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if clean.file_name().is_some_and(|name| name != "..") {
                    clean.pop();
                } else if !clean.has_root() {
                    clean.push("..");
                }
            }
            other => clean.push(other.as_os_str()),
        }
    }
    if clean.as_os_str().is_empty() {
        clean.push(".");
    }
    clean.to_string_lossy().replace(['\\', '/', ':'], "-")
}
fn project(identity: &TranscriptSessionIdentity) -> Option<PathBuf> {
    if identity.cwd.trim().is_empty() {
        return None;
    }
    let root = if !identity.claude_dir.trim().is_empty() {
        PathBuf::from(identity.claude_dir.trim())
    } else if !identity.home_dir.trim().is_empty() {
        Path::new(identity.home_dir.trim()).join(".claude")
    } else {
        return None;
    };
    Some(root.join("projects").join(project_name(&identity.cwd)))
}
#[derive(Default)]
pub(super) struct Collector {
    states: BTreeMap<LiveSessionId, Continuation>,
}
impl Collector {
    pub(super) fn event(&mut self, worker: &SessionWorkers, event: &CoreEvent) {
        if let CoreEvent::Ended { binding, .. } = event {
            if let Some(state) = self.states.get_mut(&binding.session)
                && state.binding == *binding
            {
                let now = Timestamp::now();
                let _ = state.vt.finalize(now);
                state.journal.finalize();
                state.detail.stop();
                state.native.stop();
                if let Some(progress) = workflow_state::compose(
                    state.vt.progress.as_ref(),
                    state.journal.progress.as_ref(),
                    state.journal.progress.as_ref().map_or(0, |p| p.total),
                ) {
                    let _ = state.vt.publish(progress, now);
                }
            }
            return;
        }
        if let CoreEvent::GitTurnCapture {
            binding,
            ended_at: None,
            ..
        }
        | CoreEvent::Reattached(binding) = event
        {
            let state = self
                .states
                .entry(binding.session)
                .or_insert_with(|| Continuation::new(*binding));
            state.rebind(*binding);
            let now = Timestamp::now();
            if matches!(event, CoreEvent::Reattached(_)) {
                state.native.reattach(now);
            } else {
                state.native.start(now);
            }
            return;
        }
        let CoreEvent::OutputObserved { binding, at, .. } = event else {
            return;
        };
        let Ok(core) = worker.core() else {
            return;
        };
        let Ok(snapshot) = core.session_observation_snapshot(*binding) else {
            return;
        };
        if snapshot.details.snapshot.provider != "claude" {
            return;
        }
        let state = self
            .states
            .entry(binding.session)
            .or_insert_with(|| Continuation::new(*binding));
        state.rebind(*binding);
        state.vt.queue(*at, snapshot.resize_debounce);
    }
    pub(super) async fn poll(
        &mut self,
        worker: &SessionWorkers,
        now: Timestamp,
    ) -> Result<(), SessionError> {
        let core = worker.core()?;
        let config = worker.config()?;
        let ids = core.registered_session_ids();
        self.states.retain(|id, state| {
            ids.contains(id)
                && core
                    .details(*id)
                    .is_some_and(|details| details.binding.incarnation == state.binding.incarnation)
        });
        for state in self.states.values_mut() {
            let Some(details) = core.details(state.binding.session) else {
                continue;
            };
            if !details.connected {
                continue;
            }
            state.rebind(details.binding);
            let scan_due = state.vt.due.is_some_and(|due| due <= now);
            let journal_due = state.journal.due.is_some_and(|due| due <= now);
            let detail_due = state.detail.due.is_some_and(|due| due <= now);
            let native_due = state.native.due.is_some_and(|due| due <= now);
            if !scan_due && !journal_due && !detail_due && !native_due {
                continue;
            }
            let snapshot = match core.session_observation_snapshot(state.binding) {
                Ok(snapshot) => snapshot,
                Err(SessionError::StaleBinding) => continue,
                Err(error) => return Err(error),
            };
            if let Some(tree) = state.native.poll(
                worker,
                &snapshot,
                config.workflow.subagent_tree_enabled,
                now,
            )? {
                match core.apply_observation_for_turn(
                    state.binding,
                    snapshot.confirmed_turn,
                    SessionObservation::Subagents(tree),
                    now,
                ) {
                    Ok(effects) => worker.apply(effects).await?,
                    Err(SessionError::StaleBinding) => state.native.invalidate_publication(),
                    Err(error) => return Err(error),
                }
            }
            if snapshot.details.snapshot.provider != "claude" {
                continue;
            }
            // A resize can be admitted after the output event queued the scan.
            if scan_due && snapshot.resize_debounce.is_some_and(|until| until > now) {
                state.vt.due = snapshot.resize_debounce;
                if !journal_due && !detail_due {
                    continue;
                }
            } else if scan_due {
                state.vt.scan(
                    workflow::parse(&snapshot.tail),
                    snapshot.details.snapshot.activity.output_idle,
                    now,
                );
                if config.workflow.journal_enabled && state.vt.has_signal {
                    let previous = state.journal.progress.as_ref().is_some_and(|p| p.settled);
                    state.journal.start(
                        now,
                        &state.vt.signature,
                        snapshot.details.snapshot.activity.output_idle,
                    );
                    if previous && state.journal.progress.is_none() {
                        state.detail.stop();
                    }
                }
            }
            if !config.workflow.journal_enabled {
                state.journal.disable();
            } else if let Some(project) = project(&snapshot.details.transcript) {
                state
                    .journal
                    .poll(
                        &worker.paths,
                        &project,
                        PollContext {
                            vt: state.vt.progress.as_ref(),
                            has_signal: state.vt.has_signal,
                            signature: &state.vt.signature,
                            idle: snapshot.details.snapshot.activity.output_idle,
                            now,
                        },
                    )
                    .map_err(|_| storage_error("workflow journal observation failed"))?;
                if let Some(run) = state.journal.new_run.take()
                    && config.workflow.task_detail_enabled
                    && let Some(directory) = state.journal.directory.clone()
                {
                    state.detail.start(run, directory, now);
                }
            } else {
                // Source releases the first-association settle guard even when
                // there is no project directory to discover.
                state.journal.pending = false;
                state.journal.due = if state.vt.progress.as_ref().is_some_and(|p| p.settled) {
                    None
                } else {
                    now.checked_add(Duration::from_secs(1))
                };
            }
            if !config.workflow.task_detail_enabled {
                state.detail.stop();
            } else {
                state.detail.poll(
                    worker,
                    &snapshot.details.snapshot.cwd,
                    state.journal.progress.as_ref().is_some_and(|p| p.settled),
                    now,
                )?;
            }
            let started = state.journal.progress.as_ref().map_or(0, |p| p.total);
            let Some(mut progress) = workflow_state::compose(
                state.vt.progress.as_ref(),
                state.journal.progress.as_ref(),
                started,
            ) else {
                continue;
            };
            if state.journal.pending && progress.settled_by == "vt" {
                progress.settled = false;
                progress.settled_by.clear();
            }
            if let Some(detail) = &state.detail.progress {
                task_detail::overlay(&mut progress, detail);
            }
            if let Some(publication) = state.vt.publish(progress, now) {
                let completed = publication.completion;
                let progress = publication.progress;
                match core.apply_observation_for_turn(
                    state.binding,
                    snapshot.confirmed_turn,
                    SessionObservation::Workflow(progress.clone()),
                    now,
                ) {
                    Ok(effects) => {
                        worker.apply(effects).await?;
                        if completed {
                            worker.workflow_completion.get().ok_or_else(|| {
                                SessionError::InvalidRequest(
                                    "workflow completion callback is not bound".into(),
                                )
                            })?(state.binding, &progress)?;
                        }
                    }
                    Err(SessionError::StaleBinding) => {}
                    Err(error) => return Err(error),
                }
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reattach_retains_parser_continuation_but_new_incarnation_discards_it() {
        let binding = SessionBinding {
            session: LiveSessionId(1),
            incarnation: SessionIncarnation(2),
            wrapper: WrapperConnectionId(3),
        };
        let mut state = Continuation::new(binding);
        state.detail.unavailable = true;
        state.vt.signature = "retained".into();
        state.rebind(SessionBinding {
            wrapper: WrapperConnectionId(4),
            ..binding
        });
        assert!(state.detail.unavailable);
        assert_eq!(state.vt.signature, "retained");
        assert_eq!(state.binding.wrapper, WrapperConnectionId(4));
        state.rebind(SessionBinding {
            incarnation: SessionIncarnation(5),
            wrapper: WrapperConnectionId(6),
            ..binding
        });
        assert!(!state.detail.unavailable);
        assert!(state.vt.signature.is_empty());
    }
}
