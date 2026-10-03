//! Detached wrapper launch and startup-only ownership.
//!
//! Source: Go `internal/hub/spawn_handler.go` (`spawnWrappedSession`,
//! `startWrapProcess`) at 21d0bc7935a2c4696fb89ccff2e324157a528c2d.
//! This map is bounded pending-attempt ownership, never another session/label
//! registry. Core alone issues/consumes proof and owns registered sessions.
//! An HTTP observer can disappear without terminating an accepted wrapper.
mod launch;
pub use launch::{ResolvedSpawnPolicy, SpawnLaunchPolicy, WrapperSpawnOptions};

use crate::{
    process::wrapper_startup::{
        ProvisionalWrapper, ReapReceipt, SpawnRegistrationReceipt, WrapperStartup,
    },
    proto::core::{
        CoreFuture, HttpWaitCancellation, SessionBinding, SessionError, SpawnAttemptId,
        SpawnWaitOutcome, WrappedSessionSpawner, WrappedSpawnSpec,
    },
};
use std::{
    collections::BTreeMap,
    io,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};
use tokio::sync::watch;

pub type SpawnFailureCallback =
    Arc<dyn Fn(SpawnAttemptId, &str) -> Result<(), SessionError> + Send + Sync>;
pub type SpawnWarningCallback = Arc<dyn Fn(&'static str) + Send + Sync>;

// Destructor paths must not inherit a panic from application callbacks. Forget
// an exceptional panic payload because its own destructor may panic as well.
fn contained<R>(operation: impl FnOnce() -> R) -> Option<R> {
    match catch_unwind(AssertUnwindSafe(operation)) {
        Ok(result) => Some(result),
        Err(payload) => {
            std::mem::forget(payload);
            None
        }
    }
}
fn warn_safely(warn: &SpawnWarningCallback, message: &'static str) {
    contained(|| warn(message));
}

fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    value
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
struct StartupBundle {
    owner: WrapperStartup,
    prompt: Option<launch::PromptFile>,
    record_model: Option<String>,
}
enum Phase {
    Installing,
    Starting(StartupBundle),
    HandedOff,
    Terminal,
}
struct EntryState {
    phase: Phase,
    reap: Option<ReapReceipt>,
}
struct Entry {
    attempt: SpawnAttemptId,
    provider: String,
    state: Mutex<EntryState>,
    changed: watch::Sender<u64>,
    outcome: watch::Sender<Option<SpawnWaitOutcome>>,
}
impl Entry {
    fn notify(&self) {
        self.changed
            .send_modify(|version| *version = version.wrapping_add(1));
    }
}
struct Registry {
    closed: bool,
    entries: BTreeMap<SpawnAttemptId, Arc<Entry>>,
}
struct Inner {
    registry: Mutex<Registry>,
    options: WrapperSpawnOptions,
    policy: Arc<dyn SpawnLaunchPolicy>,
    failed: SpawnFailureCallback,
    warn: SpawnWarningCallback,
    active: watch::Sender<usize>,
    cleanup_failures: Mutex<usize>,
}
impl Inner {
    fn remove(&self, entry: &Entry) {
        lock(&self.registry).entries.remove(&entry.attempt);
    }
    /// All caller callbacks run after releasing ownership/registry locks.
    fn fail(&self, entry: &Entry, outcome: SpawnWaitOutcome) {
        {
            let mut state = lock(&entry.state);
            if matches!(state.phase, Phase::HandedOff | Phase::Terminal) {
                return;
            }
            let phase = std::mem::replace(&mut state.phase, Phase::Terminal);
            if let Phase::Starting(bundle) = phase {
                state.reap = Some(bundle.owner.abort());
                drop(bundle.prompt);
            }
        }
        self.remove(entry);
        // Publish completion before advisory callbacks: their failure must not
        // strand an HTTP or registration observer after ownership is closed.
        entry.outcome.send_replace(Some(outcome));
        entry.notify();
        match contained(|| (self.failed)(entry.attempt, &entry.provider).is_err()) {
            Some(false) => {}
            Some(true) => warn_safely(&self.warn, "failed to release wrapper startup admission"),
            None => warn_safely(&self.warn, "wrapper startup failure callback panicked"),
        }
    }
    fn close(&self) {
        let entries = {
            let mut registry = lock(&self.registry);
            registry.closed = true;
            registry.entries.values().cloned().collect::<Vec<_>>()
        };
        for entry in entries {
            self.fail(&entry, SpawnWaitOutcome::HubStopped);
        }
    }
}

/// Construct before SessionEngine with callbacks which upgrade its Weak pointer.
/// The launch policy must resolve profiles/routes (including auto selection) once;
/// no fallback account or guessed route is supplied by this adapter.
pub struct ProcessWrappedSpawner {
    inner: Arc<Inner>,
}
impl ProcessWrappedSpawner {
    pub fn new(
        options: WrapperSpawnOptions,
        policy: Arc<dyn SpawnLaunchPolicy>,
        failed: SpawnFailureCallback,
        warn: SpawnWarningCallback,
    ) -> io::Result<Self> {
        options.validate()?;
        let (active, _) = watch::channel(0);
        Ok(Self {
            inner: Arc::new(Inner {
                registry: Mutex::new(Registry {
                    closed: false,
                    entries: BTreeMap::new(),
                }),
                options,
                policy,
                failed,
                warn,
                active,
                cleanup_failures: Mutex::new(0),
            }),
        })
    }
    pub fn pending_count(&self) -> usize {
        lock(&self.inner.registry).entries.len()
    }
    /// Immediately refuse new attempts and close only startup ownership. Accepted
    /// wrappers are detached before ACK and therefore survive Hub restart.
    pub fn close(&self) {
        self.inner.close();
    }
    pub fn shutdown(&self) {
        self.close();
    }
    /// Call after close. Lifecycle jobs wait a bounded duration for each owned
    /// child's passive reap receipt; an incomplete reap is reported, never hidden.
    pub async fn drain(&self) -> StartupDrainReport {
        let mut active = self.inner.active.subscribe();
        loop {
            if *active.borrow_and_update() == 0 {
                break;
            }
            if active.changed().await.is_err() {
                break;
            }
        }
        StartupDrainReport {
            unreaped_startups: *lock(&self.inner.cleanup_failures),
        }
    }
    /// The WS caller must await this before writing its first registered ACK.
    /// It can only pass a core-created receipt; label or public PID lookup cannot
    /// transfer a process. Fast registration waits for installation exactly once.
    pub async fn prepare_registration(
        &self,
        receipt: SpawnRegistrationReceipt,
    ) -> io::Result<PendingRegistrationAck> {
        let entry = lock(&self.inner.registry)
            .entries
            .get(&receipt.attempt())
            .cloned()
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "wrapper startup attempt is not pending",
                )
            })?;
        let mut changed = entry.changed.subscribe();
        let mut receipt = Some(receipt);
        loop {
            changed.borrow_and_update();
            let transfer = {
                let mut state = lock(&entry.state);
                match &state.phase {
                    Phase::Installing => None,
                    Phase::Starting(bundle) => {
                        if bundle.owner.pid()
                            != receipt
                                .as_ref()
                                .expect("receipt retained until transfer")
                                .pid()
                        {
                            return Err(io::Error::new(
                                io::ErrorKind::PermissionDenied,
                                "registration PID does not own this wrapper startup",
                            ));
                        }
                        let Phase::Starting(bundle) =
                            std::mem::replace(&mut state.phase, Phase::HandedOff)
                        else {
                            unreachable!()
                        };
                        // Disarm under this entry's ownership lock. Neither
                        // deadline nor close can observe a half-finished transfer.
                        let receipt = receipt.take().expect("one registration transfer");
                        let binding = receipt.binding();
                        let reap = bundle.owner.receipt();
                        match bundle.owner.begin_registration(receipt) {
                            Ok(owner) => Some(Ok(PendingRegistrationAck {
                                owner: Some(owner),
                                prompt: bundle.prompt,
                                entry: entry.clone(),
                                warn: self.inner.warn.clone(),
                                binding,
                                policy: self.inner.policy.clone(),
                                record_model: bundle.record_model,
                            })),
                            Err(error) => {
                                drop(bundle.prompt);
                                state.phase = Phase::Terminal;
                                state.reap = Some(reap);
                                Some(Err(error))
                            }
                        }
                    }
                    Phase::HandedOff | Phase::Terminal => {
                        return Err(io::Error::new(
                            io::ErrorKind::BrokenPipe,
                            "wrapper startup is no longer pending",
                        ));
                    }
                }
            };
            if let Some(transfer) = transfer {
                self.inner.remove(&entry);
                if transfer.is_err() {
                    // Core consumed this attempt when sealing the receipt.
                    // Transport owns cleanup of the registered binding.
                    entry.outcome.send_replace(Some(SpawnWaitOutcome::Failed(
                        "wrapper exited before registration transfer".into(),
                    )));
                }
                entry.notify();
                return transfer;
            }
            changed
                .changed()
                .await
                .map_err(|_| io::Error::other("wrapper installation channel closed"))?;
        }
    }
}
impl Drop for ProcessWrappedSpawner {
    fn drop(&mut self) {
        self.close();
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartupDrainReport {
    /// Reap waits which failed/timed out or were abandoned before observation.
    /// Native passive reapers continue independently after runtime shutdown.
    pub unreaped_startups: usize,
}

/// A successful registration is Hub acceptance, not proof the wrapper received
/// ACK or executed its provider. Drop/uncertain delivery detach; only a known
/// pre-ACK rejection may use reject_before_ack. No registered process registry
/// or shutdown termination right is retained here.
pub struct PendingRegistrationAck {
    owner: Option<ProvisionalWrapper>,
    prompt: Option<launch::PromptFile>,
    entry: Arc<Entry>,
    warn: SpawnWarningCallback,
    binding: SessionBinding,
    policy: Arc<dyn SpawnLaunchPolicy>,
    record_model: Option<String>,
}
impl PendingRegistrationAck {
    pub fn binding(&self) -> SessionBinding {
        self.binding
    }
    pub fn acknowledged(mut self) -> ReapReceipt {
        self.accept()
    }
    pub fn delivery_uncertain(mut self) -> ReapReceipt {
        self.accept()
    }
    fn accept(&mut self) -> ReapReceipt {
        if let Some(prompt) = self.prompt.take() {
            prompt.release_to_wrapper();
        }
        let receipt = self
            .owner
            .take()
            .expect("one registration completion")
            .acknowledged();
        // ACK acceptance is already established and detached. Publish it even
        // when optional model persistence or warning code panics during Drop.
        self.entry
            .outcome
            .send_replace(Some(SpawnWaitOutcome::Registered(self.binding)));
        if let Some(model) = self.record_model.take() {
            match contained(|| {
                self.policy
                    .registered_model(&self.entry.provider, &model)
                    .is_err()
            }) {
                Some(false) => {}
                Some(true) => warn_safely(
                    &self.warn,
                    "accepted wrapper model preference could not be saved",
                ),
                None => warn_safely(&self.warn, "accepted wrapper model callback panicked"),
            }
        }
        receipt
    }
    pub fn reject_before_ack(mut self) -> ReapReceipt {
        let receipt = self
            .owner
            .take()
            .expect("one registration completion")
            .abort();
        self.prompt.take();
        // The proof was already consumed into a core binding; WS disconnect
        // policy handles that session rather than releasing a pending lease.
        self.entry
            .outcome
            .send_replace(Some(SpawnWaitOutcome::Failed(
                "wrapper registration rejected before ACK".into(),
            )));
        receipt
    }
}
impl Drop for PendingRegistrationAck {
    fn drop(&mut self) {
        if self.owner.is_some() {
            self.accept();
        }
    }
}

/// Created before Tokio receives the future, so runtime abandonment before the
/// first poll has the same ownership cleanup as unwind during launch or wait.
struct StartupJob {
    inner: Arc<Inner>,
    entry: Arc<Entry>,
    // Retain the passive receipt while awaiting it. If that await is abandoned,
    // the native reaper still owns its thread/child; the drain reports that its
    // completion was not observed instead of claiming a successful reap.
    reap: Option<ReapReceipt>,
}
impl Drop for StartupJob {
    fn drop(&mut self) {
        self.inner.fail(
            &self.entry,
            SpawnWaitOutcome::Failed("wrapper lifecycle was abandoned".into()),
        );
        let reap = self
            .reap
            .take()
            .or_else(|| lock(&self.entry.state).reap.take());
        if reap.is_some() {
            *lock(&self.inner.cleanup_failures) += 1;
            warn_safely(
                &self.inner.warn,
                "wrapper lifecycle ended before passive reap was observed",
            );
        }
        // Required synchronous abort/removal/wake/callback containment precedes
        // active-count release. A drain cannot race ahead of those operations.
        self.inner.active.send_modify(|count| *count -= 1);
    }
}
async fn lifecycle(
    inner: Arc<Inner>,
    entry: Arc<Entry>,
    spec: WrappedSpawnSpec,
    startup_wait: Duration,
    mut job: StartupJob,
) {
    let launched = launch::prepare(
        &inner.options,
        inner.policy.as_ref(),
        &spec,
        inner.warn.as_ref(),
    )
    .and_then(|prepared| {
        if !matches!(lock(&entry.state).phase, Phase::Installing) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "wrapper startup closed",
            ));
        }
        let owner = WrapperStartup::spawn(
            entry.attempt,
            &prepared.process,
            prepared.stdout,
            prepared.stderr,
        )?;
        Ok(StartupBundle {
            owner,
            prompt: prepared.prompt,
            record_model: prepared.record_model,
        })
    });
    let receipt = match launched {
        Ok(bundle) => {
            let receipt = bundle.owner.receipt();
            let mut state = lock(&entry.state);
            if matches!(state.phase, Phase::Installing) {
                state.phase = Phase::Starting(bundle);
            } else {
                state.reap = Some(bundle.owner.abort());
                drop(bundle.prompt);
            }
            drop(state);
            entry.notify();
            Some(receipt)
        }
        Err(_) => {
            inner.fail(
                &entry,
                SpawnWaitOutcome::Failed("wrapper process launch failed".into()),
            );
            None
        }
    };
    if let Some(receipt) = receipt {
        let mut changed = entry.changed.subscribe();
        let deadline = tokio::time::sleep(startup_wait);
        tokio::pin!(deadline);
        loop {
            changed.borrow_and_update();
            if matches!(lock(&entry.state).phase, Phase::HandedOff | Phase::Terminal) {
                break;
            }
            tokio::select! {
                _ = receipt.wait() => {
                    inner.fail(&entry, SpawnWaitOutcome::Failed("wrapper exited before registration".into())); break;
                },
                _ = &mut deadline => { inner.fail(&entry, SpawnWaitOutcome::TimedOut); break; },
                _ = spec.cancellation.token().cancelled() => {
                    inner.fail(&entry, SpawnWaitOutcome::Failed("wrapper startup task cancelled".into())); break;
                },
                _ = changed.changed() => {},
            }
        }
    }
    job.reap = lock(&entry.state).reap.take();
    if let Some(reap) = &job.reap
        && !matches!(
            tokio::time::timeout(inner.options.reap_timeout, reap.wait()).await,
            Ok(Ok(_))
        )
    {
        *lock(&inner.cleanup_failures) += 1;
        warn_safely(
            &inner.warn,
            "wrapper startup cleanup did not finish within its reap deadline",
        );
    }
    job.reap = None;
}
impl WrappedSessionSpawner for ProcessWrappedSpawner {
    fn spawn_and_wait<'a>(
        &'a self,
        spec: WrappedSpawnSpec,
        wait: Duration,
        waiter: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome> {
        Box::pin(async move {
            let Some(attempt) = spec.spawn_attempt else {
                return SpawnWaitOutcome::Failed("core spawn attempt is required".into());
            };
            if spec.registration_proof.is_none() {
                return SpawnWaitOutcome::Failed("core registration proof is required".into());
            }
            if spec.cancellation.token().is_cancelled() {
                return SpawnWaitOutcome::Failed("wrapper startup task cancelled".into());
            }
            let wait = if wait.is_zero() {
                Duration::from_secs(1)
            } else {
                wait
            };
            if tokio::time::Instant::now().checked_add(wait).is_none() {
                return SpawnWaitOutcome::Failed("invalid wrapper registration deadline".into());
            }
            let (outcome, mut result) = watch::channel(None);
            let (changed, _) = watch::channel(0);
            let entry = Arc::new(Entry {
                attempt,
                provider: spec.provider.clone(),
                state: Mutex::new(EntryState {
                    phase: Phase::Installing,
                    reap: None,
                }),
                changed,
                outcome,
            });
            {
                let mut registry = lock(&self.inner.registry);
                if registry.closed {
                    return SpawnWaitOutcome::HubStopped;
                }
                if registry.entries.contains_key(&attempt) {
                    return SpawnWaitOutcome::Failed("wrapper attempt already pending".into());
                }
                if registry.entries.len() >= self.inner.options.pending_capacity {
                    return SpawnWaitOutcome::Failed("wrapper startup capacity reached".into());
                }
                registry.entries.insert(attempt, entry.clone());
                self.inner.active.send_modify(|count| *count += 1);
            }
            // Spawn before the first await: dropping the HTTP future cannot own
            // or cancel the process lifecycle once the attempt was reserved.
            tokio::spawn(lifecycle(
                self.inner.clone(),
                entry.clone(),
                spec,
                wait,
                StartupJob {
                    inner: self.inner.clone(),
                    entry,
                    reap: None,
                },
            ));
            let deadline = tokio::time::sleep(wait);
            tokio::pin!(deadline);
            loop {
                if let Some(outcome) = result.borrow_and_update().clone() {
                    return outcome;
                }
                tokio::select! {
                    biased;
                    value = result.changed() => if value.is_err() { return SpawnWaitOutcome::Failed("wrapper lifecycle ended".into()); },
                    _ = waiter.token().cancelled() => return SpawnWaitOutcome::WaiterCancelled,
                    _ = &mut deadline => return SpawnWaitOutcome::TimedOut,
                }
            }
        })
    }
}

#[cfg(test)]
mod tests;
