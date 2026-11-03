//! Opt-in trusted coordinator; never wired to HTTP or production capabilities.
//! A committed start/dispatch claim is never retried, including after shutdown.
use super::iris_execution_launch::{OwnedLaunchPort, prepare_spec};
use crate::{
    hub::{
        iris_execution_grants::{ApprovedGrant, GrantLoader},
        iris_execution_journal::{Acceptance, Binding, DispatchClaim, Journal},
        iris_execution_owner::{OwnerStore, OwnerTicket},
    },
    proto::core::{HttpWaitCancellation, SessionBinding, SpawnWaitOutcome},
};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{SyncSender, sync_channel},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    sync::{Notify, mpsc, watch},
    task::JoinHandle,
    time::{Instant, sleep, timeout},
};

#[derive(Clone, Debug)]
pub struct DispatcherConfig {
    pub queue_capacity: usize,
    pub registration_timeout: Duration,
    pub root_timeout: Duration,
    pub poll_interval: Duration,
    pub stop_timeout: Duration,
}
impl Default for DispatcherConfig {
    fn default() -> Self {
        Self {
            queue_capacity: 8,
            registration_timeout: Duration::from_secs(20),
            root_timeout: Duration::from_secs(10),
            poll_interval: Duration::from_millis(50),
            stop_timeout: Duration::from_secs(5),
        }
    }
}
impl DispatcherConfig {
    pub fn validate(&self) -> Result<(), DispatchError> {
        if !(1..=128).contains(&self.queue_capacity)
            || [
                self.registration_timeout,
                self.root_timeout,
                self.stop_timeout,
            ]
            .iter()
            .any(|v| v.is_zero() || *v > Duration::from_secs(60))
            || self.poll_interval < Duration::from_millis(1)
            || self.poll_interval > Duration::from_secs(1)
            || self.poll_interval > self.root_timeout
        {
            return Err(DispatchError::Configuration);
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DispatchError {
    Configuration,
    Unavailable,
    Conflict,
    Busy,
    GrantRejected,
    AdmissionRejected,
    Persistence,
    WorkerStopped,
}
impl DispatchError {
    pub fn code(self) -> &'static str {
        match self {
            Self::Configuration => "DISPATCH_CONFIGURATION",
            Self::Unavailable => "DISPATCH_UNAVAILABLE",
            Self::Conflict => "DISPATCH_REQUEST_CONFLICT",
            Self::Busy => "DISPATCH_QUEUE_FULL",
            Self::GrantRejected => "DISPATCH_GRANT_REJECTED",
            Self::AdmissionRejected => "DISPATCH_ADMISSION_REJECTED",
            Self::Persistence => "DISPATCH_PERSISTENCE",
            Self::WorkerStopped => "DISPATCH_WORKER_STOPPED",
        }
    }
}
impl std::fmt::Display for DispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}
impl std::error::Error for DispatchError {}
fn now_ms() -> Result<u64, DispatchError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| DispatchError::Unavailable)?
        .as_millis()
        .try_into()
        .map_err(|_| DispatchError::Unavailable)
}
fn now() -> Result<i64, DispatchError> {
    (now_ms()? / 1000)
        .try_into()
        .map_err(|_| DispatchError::Unavailable)
}
fn scope(mut value: Value) -> Value {
    if let Some(fields) = value.as_object_mut() {
        for key in ["attempt_id", "fence", "executor_instance_id"] {
            fields.remove(key);
        }
    }
    value
}
fn fresh_grant(
    journal: &mut Journal,
    root: &Path,
    binding: &Binding,
) -> Result<ApprovedGrant, DispatchError> {
    let mut loader = GrantLoader::open(root).map_err(|_| DispatchError::GrantRejected)?;
    let grants = loader
        .load_approved()
        .map_err(|_| DispatchError::GrantRejected)?;
    let expected = scope(json!(binding));
    let mut selected = None;
    // Import every approved revision before checking the requested revision.
    for grant in grants {
        journal
            .register_grant(grant.grant.clone())
            .map_err(|_| DispatchError::GrantRejected)?;
        if scope(json!(grant.grant.binding)) == expected {
            selected = Some(grant);
        }
    }
    selected.ok_or(DispatchError::GrantRejected)
}
struct CleanupState {
    pending: AtomicUsize,
    failed: AtomicBool,
    changed: Notify,
}
struct CleanupPending(Arc<CleanupState>);
impl Drop for CleanupPending {
    fn drop(&mut self) {
        self.0.pending.fetch_sub(1, Ordering::SeqCst);
        self.0.changed.notify_waiters();
    }
}
struct CleanupLane {
    sender: SyncSender<Binding>,
    state: Arc<CleanupState>,
}
impl CleanupLane {
    fn new(journal: Arc<Mutex<Journal>>, capacity: usize) -> Result<Self, DispatchError> {
        let (sender, receiver) = sync_channel::<Binding>(capacity);
        let state = Arc::new(CleanupState {
            pending: AtomicUsize::new(0),
            failed: AtomicBool::new(false),
            changed: Notify::new(),
        });
        let thread_state = state.clone();
        // Never capture Shared/sender here: closing the last sender must end the
        // lane. This detached OS thread is not a Tokio blocking-pool task, so a
        // busy SQLite writer cannot make Runtime::drop wait for this cleanup.
        std::thread::Builder::new()
            .name("iris-dispatch-cleanup".into())
            .spawn(move || {
                while let Ok(binding) = receiver.recv() {
                    let pending = CleanupPending(thread_state.clone());
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        persist_unknown(&journal, &binding)
                    }));
                    if !matches!(result, Ok(Ok(()))) {
                        thread_state.failed.store(true, Ordering::SeqCst);
                    }
                    drop(pending);
                }
            })
            .map_err(|_| DispatchError::Unavailable)?;
        Ok(Self { sender, state })
    }
    fn enqueue(&self, binding: &Binding) {
        self.state.pending.fetch_add(1, Ordering::SeqCst);
        if self.sender.try_send(binding.clone()).is_err() {
            // Full/closed is a visible cleanup failure, never permission to
            // release a claim or pretend execution was not delivered.
            self.state.failed.store(true, Ordering::SeqCst);
            drop(CleanupPending(self.state.clone()));
        }
    }
    async fn wait(&self, deadline: Instant) -> Result<(), DispatchError> {
        loop {
            let changed = self.state.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.state.pending.load(Ordering::SeqCst) == 0 {
                return if self.state.failed.load(Ordering::SeqCst) {
                    Err(DispatchError::Persistence)
                } else {
                    Ok(())
                };
            }
            tokio::time::timeout_at(deadline, changed)
                .await
                .map_err(|_| DispatchError::Unavailable)?;
        }
    }
}
fn persist_unknown(journal: &Mutex<Journal>, binding: &Binding) -> Result<(), DispatchError> {
    let mut journal = journal.lock().map_err(|_| DispatchError::Persistence)?;
    let record = journal
        .query(binding)
        .map_err(|_| DispatchError::Persistence)?;
    if matches!(
        record["execution_state"].as_str(),
        Some("accepted" | "running" | "stop_requested")
    ) {
        let seq = record["event_seq"]
            .as_u64()
            .and_then(|n| n.checked_add(1))
            .ok_or(DispatchError::Persistence)?;
        journal.observe(binding,json!({"binding":binding,"event_kind":"disconnected","event_seq":seq,"observed_at":now()?})).map_err(|_| DispatchError::Persistence)?;
    }
    Ok(())
}
/// Additional trusted profile admission, checked again when queued work reaches
/// the launch boundary. This is not supplied by the HTTP caller.
pub type LaunchAdmission = Arc<dyn Fn(&Binding) -> Result<(), DispatchError> + Send + Sync>;

struct Shared {
    journal: Arc<Mutex<Journal>>,
    owners: Arc<Mutex<OwnerStore>>,
    runtime_root: PathBuf,
    port: Arc<dyn OwnedLaunchPort>,
    config: DispatcherConfig,
    cleanup: CleanupLane,
    admission: Option<LaunchAdmission>,
}
impl Shared {
    fn unknown(&self, binding: &Binding) {
        self.cleanup.enqueue(binding);
    }
    fn ticket(&self, binding: &Binding) -> Result<OwnerTicket, DispatchError> {
        let record = self
            .owners
            .lock()
            .map_err(|_| DispatchError::Persistence)?
            .query(binding)
            .map_err(|_| DispatchError::Persistence)?;
        let ticket: OwnerTicket = serde_json::from_value(record["ticket"].clone())
            .map_err(|_| DispatchError::Persistence)?;
        if ticket.binding != *binding {
            return Err(DispatchError::Conflict);
        }
        Ok(ticket)
    }
}
struct Job {
    claim: DispatchClaim,
    ticket: OwnerTicket,
    shared: Arc<Shared>,
    complete: bool,
}
// Includes queued jobs dropped when a worker panics: the receiver dropping must
// not silently leave accepted jobs looking available for redispatch.
impl Drop for Job {
    fn drop(&mut self) {
        if !self.complete {
            self.shared.unknown(&self.ticket.binding);
        }
    }
}

pub struct OwnedDispatcher {
    shared: Arc<Shared>,
    queue: mpsc::Sender<Job>,
    stopping: watch::Sender<bool>,
    worker: Option<JoinHandle<()>>,
}
impl OwnedDispatcher {
    pub fn new(
        journal: Arc<Mutex<Journal>>,
        owners: Arc<Mutex<OwnerStore>>,
        runtime_root: PathBuf,
        port: Arc<dyn OwnedLaunchPort>,
        config: DispatcherConfig,
    ) -> Result<Self, DispatchError> {
        Self::new_with_admission(journal, owners, runtime_root, port, config, None)
    }

    pub fn new_with_admission(
        journal: Arc<Mutex<Journal>>,
        owners: Arc<Mutex<OwnerStore>>,
        runtime_root: PathBuf,
        port: Arc<dyn OwnedLaunchPort>,
        config: DispatcherConfig,
        admission: Option<LaunchAdmission>,
    ) -> Result<Self, DispatchError> {
        config.validate()?;
        let runtime_root = runtime_root
            .canonicalize()
            .map_err(|_| DispatchError::Configuration)?;
        if !runtime_root.is_dir() {
            return Err(DispatchError::Configuration);
        }
        let handle =
            tokio::runtime::Handle::try_current().map_err(|_| DispatchError::Unavailable)?;
        if !journal
            .lock()
            .map_err(|_| DispatchError::Persistence)?
            .supports_owned_dispatch()
            .map_err(|_| DispatchError::Persistence)?
        {
            return Err(DispatchError::Unavailable);
        }
        let cleanup = CleanupLane::new(journal.clone(), config.queue_capacity + 1)?;
        let shared = Arc::new(Shared {
            journal,
            owners,
            runtime_root,
            port,
            config,
            cleanup,
            admission,
        });
        let (queue, receiver) = mpsc::channel(shared.config.queue_capacity);
        let (stopping, stopped) = watch::channel(false);
        let worker = Some(handle.spawn(worker(shared.clone(), receiver, stopped)));
        Ok(Self {
            shared,
            queue,
            stopping,
            worker,
        })
    }
    pub fn submit(&self, request_id: &str, binding: &Binding) -> Result<Value, DispatchError> {
        let mut journal = self
            .shared
            .journal
            .lock()
            .map_err(|_| DispatchError::Persistence)?;
        // Replay precedes queue reservation, grant refresh and shutdown checks.
        match journal.query_request(&binding.principal_id, &binding.workspace_id, request_id) {
            Ok(view) => {
                let receipt = &view["request_receipt"];
                if receipt["binding"] != json!(binding) || receipt["operation"] != "start" {
                    return Err(DispatchError::Conflict);
                }
                return Ok(receipt.clone());
            }
            Err(error) if error.code == "NOT_FOUND" => {}
            Err(_) => return Err(DispatchError::Persistence),
        }
        if *self.stopping.borrow() {
            return Err(DispatchError::WorkerStopped);
        }
        let reservation = self.queue.try_reserve().map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => DispatchError::Busy,
            mpsc::error::TrySendError::Closed(_) => DispatchError::WorkerStopped,
        })?;
        let _grant = fresh_grant(&mut journal, &self.shared.runtime_root, binding)?;
        let at = now()?;
        let receipt = match journal.start(request_id, binding, at).map_err(|error| {
            if error.code == "REQUEST_CONFLICT" {
                DispatchError::Conflict
            } else {
                DispatchError::AdmissionRejected
            }
        })? {
            Acceptance::Replay(receipt) => return Ok(receipt),
            Acceptance::New(receipt) => receipt,
        };
        let claim = match journal.claim_dispatch(binding, at) {
            Ok(claim) => claim,
            Err(_) => {
                drop(journal);
                self.shared.unknown(binding);
                return Err(DispatchError::AdmissionRejected);
            }
        };
        drop(journal);
        let ticket_result = self
            .shared
            .owners
            .lock()
            .map_err(|_| DispatchError::Persistence)
            .and_then(|mut owners| {
                owners
                    .begin_owner(binding, claim.claim_id(), claim.epoch(), now_ms()?)
                    .map_err(|_| DispatchError::Persistence)
            });
        let ticket = match ticket_result {
            Ok(ticket) => ticket,
            Err(error) => {
                self.shared.unknown(binding);
                return Err(error);
            }
        };
        // A concurrent shutdown can win after durable acceptance. Keep receipt;
        // never enqueue it later, remint its claim or call this an unstarted job.
        if *self.stopping.borrow() || self.queue.is_closed() {
            self.shared.unknown(binding);
            return Err(DispatchError::WorkerStopped);
        }
        reservation.send(Job {
            claim,
            ticket,
            shared: self.shared.clone(),
            complete: false,
        });
        Ok(receipt)
    }
    pub async fn request_stop(
        &self,
        request_id: &str,
        binding: &Binding,
    ) -> Result<Value, DispatchError> {
        let receipt = {
            let mut journal = self
                .shared
                .journal
                .lock()
                .map_err(|_| DispatchError::Persistence)?;
            match journal.stop(request_id, binding, now()?).map_err(|error| {
                if error.code == "REQUEST_CONFLICT" {
                    DispatchError::Conflict
                } else {
                    DispatchError::AdmissionRejected
                }
            })? {
                Acceptance::New(value) => value,
                // A receipt replay is inspection, never renewed effect authority.
                // A pre-root intent is continued only by its existing worker.
                Acceptance::Replay(value) => return Ok(value),
            }
        };
        if let Ok(ticket) = self.shared.ticket(binding) {
            // An unbound root may reject this attempt; worker later uses this
            // same persisted ID. The launch port owns effect idempotency.
            let _ = timeout(
                self.shared.config.stop_timeout,
                self.shared.port.stop_owner(&ticket, request_id),
            )
            .await;
        }
        Ok(receipt)
    }
    pub fn query(&self, binding: &Binding) -> Result<Value, DispatchError> {
        let mut record = {
            let journal = self
                .shared
                .journal
                .lock()
                .map_err(|_| DispatchError::Persistence)?;
            let mut record = journal.query(binding).map_err(|error| {
                if error.code == "NOT_FOUND" {
                    DispatchError::Unavailable
                } else {
                    DispatchError::Persistence
                }
            })?;
            record["dispatch"] = match journal.query_dispatch(binding) {
                Ok(value) => value,
                Err(error) if error.code == "NOT_FOUND" => {
                    json!({"available":false,"reason":"not_found"})
                }
                Err(error) if error.code == "DISPATCH_SCHEMA_REQUIRED" => {
                    json!({"available":false,"reason":"legacy_schema"})
                }
                Err(_) => return Err(DispatchError::Persistence),
            };
            record
        };
        record["owner"] = match self
            .shared
            .owners
            .lock()
            .map_err(|_| DispatchError::Persistence)?
            .query(binding)
        {
            Ok(value) => value,
            Err(error) if error.code == "NOT_FOUND" => {
                json!({"available":false,"reason":"not_found"})
            }
            Err(_) => return Err(DispatchError::Persistence),
        };
        Ok(record)
    }
    pub async fn shutdown(&mut self) -> Result<(), DispatchError> {
        self.stopping.send_replace(true);
        let deadline = Instant::now() + self.shared.config.stop_timeout;
        let mut worker_failed = false;
        if let Some(mut worker) = self.worker.take() {
            // One constructor-bounded total budget, including abort cleanup.
            // Aborting is only a fallback; Job guards retain unknown and never
            // restore a consumed claim or assert that Core delivery was undone.
            match timeout(self.shared.config.stop_timeout / 2, &mut worker).await {
                Ok(result) => worker_failed = result.is_err(),
                Err(_) => {
                    worker.abort();
                    let _ = tokio::time::timeout_at(deadline, worker).await;
                    worker_failed = true;
                }
            }
        }
        self.shared.cleanup.wait(deadline).await?;
        if worker_failed {
            Err(DispatchError::Unavailable)
        } else {
            Ok(())
        }
    }
}
impl Drop for OwnedDispatcher {
    fn drop(&mut self) {
        self.stopping.send_replace(true);
    }
}

async fn worker(
    shared: Arc<Shared>,
    mut receiver: mpsc::Receiver<Job>,
    mut stopped: watch::Receiver<bool>,
) {
    loop {
        // process() may already have consumed the watch change. Checking the
        // current value prevents waiting forever for a second shutdown change.
        if *stopped.borrow() {
            break;
        }
        let mut job = tokio::select! { biased; _ = stopped.changed() => break, job = receiver.recv() => match job { Some(job) => job, None => break } };
        if process(&shared, &job, &mut stopped).await.is_ok() {
            job.complete = true;
        }
    }
    receiver.close();
    while let Some(job) = receiver.recv().await {
        drop(job);
    }
}
async fn process(
    shared: &Shared,
    job: &Job,
    stopped: &mut watch::Receiver<bool>,
) -> Result<(), DispatchError> {
    if *stopped.borrow() {
        return Err(DispatchError::WorkerStopped);
    }
    let binding = &job.ticket.binding;
    let spec = {
        let mut journal = shared
            .journal
            .lock()
            .map_err(|_| DispatchError::Persistence)?;
        let grant = fresh_grant(&mut journal, &shared.runtime_root, binding)?;
        journal
            .authorize_dispatch(binding, now()?)
            .map_err(|_| DispatchError::AdmissionRejected)?;
        let claim = journal
            .query_dispatch(binding)
            .map_err(|_| DispatchError::Persistence)?;
        if claim["claim_id"] != job.claim.claim_id() || job.claim.binding() != binding {
            return Err(DispatchError::Conflict);
        }
        prepare_spec(&grant, &job.ticket).map_err(|_| DispatchError::GrantRejected)?
    };
    if let Some(admit) = &shared.admission {
        let admit = admit.clone();
        let admitted_binding = binding.clone();
        // Profile admission reads operator files and a SQLite-backed pin. A
        // contended verifier must not block Tokio's stop/deadline lane. A late
        // blocking result is read-only and can never launch a timed-out job.
        let check = tokio::task::spawn_blocking(move || admit(&admitted_binding));
        tokio::select! {
            biased;
            _ = stopped.changed() => return Err(DispatchError::WorkerStopped),
            result = timeout(shared.config.registration_timeout, check) => {
                result.map_err(|_| DispatchError::Unavailable)?
                    .map_err(|_| DispatchError::Unavailable)??;
            }
        }
    }
    let cancel = HttpWaitCancellation::default();
    let outcome = tokio::select! {
        biased;
        _ = stopped.changed() => { cancel.cancel(); return Err(DispatchError::WorkerStopped); }
        outcome = timeout(shared.config.registration_timeout, shared.port.launch_registered(spec, shared.config.registration_timeout, &cancel)) => outcome,
    };
    let session = match outcome {
        Ok(SpawnWaitOutcome::Registered(session)) => session,
        _ => {
            cancel.cancel();
            return Err(DispatchError::Unavailable);
        }
    };
    let deadline = Instant::now() + shared.config.root_timeout;
    loop {
        if *stopped.borrow() {
            return Err(DispatchError::WorkerStopped);
        }
        if !shared.port.is_current(session) {
            return Err(DispatchError::Unavailable);
        }
        let owner = shared
            .owners
            .lock()
            .map_err(|_| DispatchError::Persistence)?
            .query(binding)
            .map_err(|_| DispatchError::Persistence)?;
        if owner["ticket"] != json!(job.ticket) || owner["local_tree_ceased"] == true {
            return Err(DispatchError::Unavailable);
        }
        if owner["claimed_session"]
            .as_i64()
            .is_some_and(|id| id != session.session.0)
        {
            return Err(DispatchError::Conflict);
        }
        if root_bound(&owner, session) {
            shared
                .port
                .bind_owner(&job.ticket, session)
                .map_err(|_| DispatchError::Unavailable)?;
            let pending_stop = {
                let mut journal = shared
                    .journal
                    .lock()
                    .map_err(|_| DispatchError::Persistence)?;
                let record = journal
                    .query(binding)
                    .map_err(|_| DispatchError::Persistence)?;
                if let Some(stop_id) = record["stop_request_id"].as_str() {
                    Some(stop_id.to_string())
                } else if record["execution_state"] == "accepted" {
                    if !shared.port.is_current(session) {
                        return Err(DispatchError::Unavailable);
                    }
                    let seq = record["event_seq"]
                        .as_u64()
                        .and_then(|n| n.checked_add(1))
                        .ok_or(DispatchError::Persistence)?;
                    journal.observe(binding,json!({"binding":binding,"event_kind":"running","event_seq":seq,"observed_at":now()?})).map_err(|_| DispatchError::Persistence)?;
                    None
                } else {
                    return Err(DispatchError::Unavailable);
                }
            };
            if let Some(stop_id) = pending_stop {
                let _ = timeout(
                    shared.config.stop_timeout,
                    shared.port.stop_owner(&job.ticket, &stop_id),
                )
                .await;
            }
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(DispatchError::Unavailable);
        }
        tokio::select! { biased; _ = stopped.changed() => return Err(DispatchError::WorkerStopped), _ = sleep(shared.config.poll_interval) => {} }
    }
}
fn root_bound(record: &Value, session: SessionBinding) -> bool {
    record["claimed_session"] == session.session.0
        && record["root_pid"]
            .as_u64()
            .is_some_and(|pid| pid > 0 && pid <= u32::MAX as u64)
        && record["root_started_at_ms"].as_u64().is_some()
}
