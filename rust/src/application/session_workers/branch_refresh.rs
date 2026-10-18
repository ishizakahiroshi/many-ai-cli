//! Per-cwd refresh ownership mirrors Go's pending-ID queue and four worker
//! slots. Queued work is Hub-owned, never tied to a WebSocket request future.
use super::*;
use crate::files::{
    branch::GitBranchSource,
    change_watch::{ChangeWatch, Lease},
};
use std::sync::Mutex;
use tokio::sync::Semaphore;

pub(super) struct Owner {
    source: Arc<dyn GitBranchSource>,
    worker: Weak<SessionWorkers>,
    tasks: HubTaskHandle,
    slots: Arc<Semaphore>,
    pending: Mutex<BTreeMap<String, Vec<LiveSessionId>>>,
    /// Windows: counts are taken only after the repository changed
    /// (plan_git-invocation-index-lock C7). `None` counts every time.
    watch: Option<Arc<ChangeWatch>>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Owner {
    pub fn new(
        source: Arc<dyn GitBranchSource>,
        worker: Weak<SessionWorkers>,
        tasks: HubTaskHandle,
        watch: Option<Arc<ChangeWatch>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            source,
            worker,
            tasks,
            slots: Arc::new(Semaphore::new(4)),
            pending: Mutex::new(BTreeMap::new()),
            watch,
        })
    }

    /// Sort the change notifications (asking Git which paths it ignores),
    /// and make the sessions of repositories that changed due now.
    async fn watch_changes(&self, core: &SessionEngine) {
        let Some(watch) = &self.watch else {
            return;
        };
        for pending in watch.take() {
            let asked = pending.asked();
            let ignored = self.source.ignored(pending.root(), &asked).await;
            watch.learn(pending, ignored);
        }
        let roots = watch.due();
        if roots.is_empty() {
            return;
        }
        let ids: Vec<LiveSessionId> = core
            .branch_refresh_cwds()
            .into_iter()
            .filter(|(_, cwd)| watch.root_of(cwd).is_some_and(|root| roots.contains(&root)))
            .map(|(id, _)| id)
            .collect();
        core.restart_branch_refresh_for(&ids);
    }

    async fn changes(&self, cwd: &str) -> (i64, i64, i64) {
        let Some(watch) = &self.watch else {
            return self.source.changes(cwd).await;
        };
        match watch.lease(cwd) {
            Lease::Reuse(counts) => counts,
            Lease::Count(ticket) => {
                let (counts, answered) = self.source.changes_counted(cwd).await;
                if let Some(ticket) = ticket
                    && answered
                {
                    watch.store(ticket, counts);
                }
                counts
            }
        }
    }

    pub async fn run_timer(self: Arc<Self>, cancel: HubShutdownCancellation) {
        let mut tick = tokio::time::interval(Duration::from_millis(200));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut watched = false;
        loop {
            tokio::select! { biased;
                _ = cancel.token().cancelled() => break,
                _ = tick.tick() => {
                    let Some(worker) = self.worker.upgrade() else { break; };
                    let Ok(core) = worker.core() else { break; };
                    // A watch holds its folders (they cannot be renamed), so
                    // only repositories of live sessions stay watched.
                    if let Some(watch) = &self.watch {
                        let live = core
                            .branch_refresh_cwds()
                            .iter()
                            .filter_map(|(_, cwd)| watch.root_of(cwd))
                            .collect();
                        watch.keep(&live);
                    }
                    // The counts are only shown on a screen. With no browser
                    // attached no Git starts; one that attaches gets every
                    // session counted again (plan_git-invocation-index-lock C6).
                    if !core.ui_attached() {
                        watched = false;
                        continue;
                    }
                    if !watched {
                        watched = true;
                        core.restart_branch_refresh();
                    }
                    self.watch_changes(&core).await;
                    if let Err(error) = self.queue(core.branch_refresh_requests(Timestamp::now())) {
                        (worker.warning)("branch refresh admission", &error);
                    }
                }
            }
        }
    }

    fn queue(self: &Arc<Self>, requests: Vec<(LiveSessionId, String)>) -> Result<(), SessionError> {
        let mut grouped: BTreeMap<String, Vec<LiveSessionId>> = BTreeMap::new();
        for (id, cwd) in requests {
            let cwd = cwd.trim();
            if !cwd.is_empty() {
                grouped.entry(cwd.into()).or_default().push(id);
            }
        }
        for (cwd, ids) in grouped {
            let mut state = lock(&self.pending);
            if let Some(next) = state.get_mut(&cwd) {
                for id in ids {
                    if !next.contains(&id) {
                        next.push(id);
                    }
                }
                continue;
            }
            let permit = self.tasks.effect_permit()?;
            let cancel = permit.cancellation();
            state.insert(cwd.clone(), Vec::new());
            drop(state);
            let owner = self.clone();
            // The guard exists before task admission transfers, so an unpolled
            // cancelled task cannot strand a cwd in the in-flight map.
            let flight = Flight {
                owner: owner.clone(),
                cwd: cwd.clone(),
                armed: true,
            };
            drop(permit.start(async move {
                owner.run(cwd, ids, cancel, flight).await;
            }));
        }
        Ok(())
    }

    async fn run(
        self: Arc<Self>,
        cwd: String,
        mut ids: Vec<LiveSessionId>,
        cancel: TaskCancellation,
        mut flight: Flight,
    ) {
        loop {
            let waiting_since = std::time::Instant::now();
            let slot = tokio::select! { biased;
                _ = cancel.token().cancelled() => return,
                slot = self.slots.clone().acquire_owned() => match slot { Ok(slot) => slot, Err(_) => return },
            };
            let slot_wait = waiting_since.elapsed();
            let refresh = async {
                let Some(worker) = self.worker.upgrade() else {
                    return;
                };
                let Ok(core) = worker.core() else {
                    return;
                };
                // 一時観測 rust-branch-probe: 記録するだけで、取得・適用の中身は変えない。
                let started = std::time::Instant::now();
                let ((result, probe_branch), probe) = crate::logging::branch_probe::scope(async {
                    let branch = self.source.branch(&cwd).await;
                    let probe_branch = branch.clone();
                    let changes = self.changes(&cwd).await;
                    // Go checks the current sessions after branch/stats finish.
                    let project_id = if core.branch_project_needed(&cwd, &ids) {
                        self.source.project(&cwd).await
                    } else {
                        None
                    };
                    let git_root = project_id
                        .as_ref()
                        .filter(|root| !root.is_empty())
                        .map(PathBuf::from);
                    let observation = SessionObservation::Branch {
                        branch,
                        git_root,
                        changes,
                        project_id,
                    };
                    let result = match core.apply_branch_refresh(&cwd, &ids, observation) {
                        Ok(effects) => worker.apply(effects).await,
                        Err(error) => Err(error),
                    };
                    (result, probe_branch)
                })
                .await;
                if let Err(error) = result {
                    (worker.warning)("branch refresh publication", &error);
                }
                crate::logging::branch_probe::record_refresh(
                    &cwd,
                    &ids.iter().map(|id| id.0).collect::<Vec<_>>(),
                    &probe_branch,
                    &probe,
                    slot_wait,
                    started.elapsed(),
                );
            };
            tokio::select! { biased;
                _ = cancel.token().cancelled() => return,
                _ = refresh => {},
            }
            // Hold the slot through publication, then release before admitting
            // this cwd's pending pass, like Go's deferred semaphore release.
            drop(slot);
            let mut state = lock(&self.pending);
            let Some(next) = state.get_mut(&cwd) else {
                return;
            };
            if next.is_empty() {
                state.remove(&cwd);
                flight.armed = false;
                return;
            }
            ids = std::mem::take(next);
        }
    }
}

struct Flight {
    owner: Arc<Owner>,
    cwd: String,
    armed: bool,
}
impl Drop for Flight {
    fn drop(&mut self) {
        if self.armed {
            lock(&self.owner.pending).remove(&self.cwd);
        }
    }
}

#[cfg(test)]
mod tests;
