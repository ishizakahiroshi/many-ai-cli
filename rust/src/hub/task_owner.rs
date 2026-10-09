//! Hub-owned HTTP and effect lifetimes, independent of response waiters.
//!
//! Shutdown order is deliberate: stop HTTP admission, drain admitted requests
//! while effects/confirmations still accept work, shut down pending core
//! confirmations, then stop, cancel if needed, and drain effects. Only after both
//! lanes drain may the journal, socket writers and runtime be shut down.
//!
//! A confirmation caller acquires an effect permit BEFORE `AcceptedSpawnDecision::run`.
//! It calls `run(permit.cancellation())` and `permit.start(future)` synchronously in
//! the same poll, with no intervening await. Closing a lane does not revoke its
//! permits. `start` has no fallible enqueue and does not consult admission again.
//! Owner/runtime abandonment is cancellation, never a successful completion.
use super::approval_actions::ApprovalEffectOwner;
use crate::proto::core::{CoreFuture, SessionError, TaskCancellation};
use std::{
    collections::BTreeMap,
    future::Future,
    panic::{AssertUnwindSafe, catch_unwind},
    pin::Pin,
    sync::{Arc, Mutex, MutexGuard, Weak},
    task::{Context, Poll},
};
use tokio::{
    runtime::Handle,
    sync::{Notify, oneshot},
    task::AbortHandle,
};

/// A receiver may be dropped freely; failures remain visible in lane receipts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskFailure {
    Panicked,
    Cancelled,
}
impl From<TaskFailure> for SessionError {
    fn from(failure: TaskFailure) -> Self {
        match failure {
            TaskFailure::Panicked => Self::Transport("Hub-owned task panicked".into()),
            TaskFailure::Cancelled => Self::Cancelled,
        }
    }
}

/// Aggregate receipts use constant space and never retain completed task results.
/// `completed` means the future returned, not that its application result was Ok.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TaskLaneSnapshot {
    pub accepting: bool,
    pub unstarted: usize,
    pub running: usize,
    pub completed: u64,
    pub panicked: u64,
    pub cancelled: u64,
    pub abandoned_permits: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskOwnerSnapshot {
    pub requests: TaskLaneSnapshot,
    pub effects: TaskLaneSnapshot,
}

#[derive(Clone, Copy)]
enum Lane {
    Request,
    Effect,
}
struct TaskRecord {
    cancel: TaskCancellation,
    started: bool,
    abort: Option<AbortHandle>,
    abort_requested: bool,
}
struct LaneState {
    tasks: BTreeMap<u64, TaskRecord>,
    receipt: TaskLaneSnapshot,
}
impl Default for LaneState {
    fn default() -> Self {
        Self {
            tasks: BTreeMap::new(),
            receipt: TaskLaneSnapshot {
                accepting: true,
                ..Default::default()
            },
        }
    }
}
impl LaneState {
    fn snapshot(&self) -> TaskLaneSnapshot {
        let mut receipt = self.receipt;
        receipt.unstarted = self.tasks.values().filter(|task| !task.started).count();
        receipt.running = self.tasks.len() - receipt.unstarted;
        receipt
    }
}
#[derive(Default)]
struct State {
    next_id: u64,
    requests: LaneState,
    effects: LaneState,
}
impl State {
    fn lane(&self, lane: Lane) -> &LaneState {
        match lane {
            Lane::Request => &self.requests,
            Lane::Effect => &self.effects,
        }
    }
    fn lane_mut(&mut self, lane: Lane) -> &mut LaneState {
        match lane {
            Lane::Request => &mut self.requests,
            Lane::Effect => &mut self.effects,
        }
    }
}
#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    changed: Notify,
}
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct Lifetime {
    shared: Arc<Shared>,
    runtime: Handle,
}
impl Drop for Lifetime {
    fn drop(&mut self) {
        // Tasks and permits own Shared, never Lifetime. The last actual owner
        // therefore cannot be kept alive by its own unfinished work.
        {
            let mut state = lock(&self.shared.state);
            state.requests.receipt.accepting = false;
            state.effects.receipt.accepting = false;
        }
        self.shared.cancel(Lane::Request, true);
        self.shared.cancel(Lane::Effect, true);
    }
}

/// Cloneable lifecycle owner; dropping its last clone cancels and aborts jobs.
/// Keep an owner alive until explicit drains finish. Tokio must keep running to
/// finish asynchronous cleanup; Drop alone can only request abort and run RAII.
#[derive(Clone)]
pub struct HubTaskOwner {
    lifetime: Arc<Lifetime>,
}
/// Weak admission handle for task-captured services. Only the external Hub
/// lifecycle owns HubTaskOwner; a running request cannot keep its own owner alive.
#[derive(Clone)]
pub struct HubTaskHandle {
    lifetime: Weak<Lifetime>,
}
impl HubTaskHandle {
    pub fn request_permit(&self) -> Result<OwnedTaskPermit, SessionError> {
        self.lifetime
            .upgrade()
            .ok_or(SessionError::Shutdown)
            .and_then(|lifetime| HubTaskOwner { lifetime }.request_permit())
    }
    pub fn effect_permit(&self) -> Result<OwnedTaskPermit, SessionError> {
        self.lifetime
            .upgrade()
            .ok_or(SessionError::Shutdown)
            .and_then(|lifetime| HubTaskOwner { lifetime }.effect_permit())
    }
}
impl HubTaskOwner {
    pub fn handle(&self) -> HubTaskHandle {
        HubTaskHandle {
            lifetime: Arc::downgrade(&self.lifetime),
        }
    }
    pub fn new(runtime: Handle) -> Self {
        Self {
            lifetime: Arc::new(Lifetime {
                shared: Arc::new(Shared::default()),
                runtime,
            }),
        }
    }

    pub fn request_permit(&self) -> Result<OwnedTaskPermit, SessionError> {
        self.permit(Lane::Request)
    }
    pub fn effect_permit(&self) -> Result<OwnedTaskPermit, SessionError> {
        self.permit(Lane::Effect)
    }
    fn permit(&self, lane: Lane) -> Result<OwnedTaskPermit, SessionError> {
        let mut state = lock(&self.lifetime.shared.state);
        if !state.lane(lane).receipt.accepting {
            return Err(SessionError::Shutdown);
        }
        let id = state
            .next_id
            .checked_add(1)
            .ok_or_else(|| SessionError::InvalidRequest("Hub task identity exhausted".into()))?;
        state.next_id = id;
        let cancel = TaskCancellation::default();
        state.lane_mut(lane).tasks.insert(
            id,
            TaskRecord {
                cancel: cancel.clone(),
                started: false,
                abort: None,
                abort_requested: false,
            },
        );
        Ok(OwnedTaskPermit {
            reservation: Some(Reservation {
                shared: self.lifetime.shared.clone(),
                lane,
                id,
            }),
            runtime: self.lifetime.runtime.clone(),
            cancel,
        })
    }

    /// Stop admission only. Already issued request permits remain valid.
    pub fn stop_requests(&self) {
        lock(&self.lifetime.shared.state).requests.receipt.accepting = false;
    }

    /// The handoff lane cannot close underneath an admitted HTTP operation.
    pub fn stop_effects(&self) -> Result<(), SessionError> {
        let mut state = lock(&self.lifetime.shared.state);
        if state.requests.receipt.accepting || !state.requests.tasks.is_empty() {
            return Err(SessionError::InvalidRequest(
                "stop and drain HTTP requests before stopping Hub effects".into(),
            ));
        }
        state.effects.receipt.accepting = false;
        Ok(())
    }

    /// Signal this lane's task cancellation domain, never an HTTP wait token.
    /// Call after stopping request admission; drain still waits for cleanup.
    pub fn cancel_requests(&self) -> Result<(), SessionError> {
        self.cancel_closed(Lane::Request, false)
    }
    pub fn cancel_effects(&self) -> Result<(), SessionError> {
        self.cancel_closed(Lane::Effect, false)
    }
    /// Last resort for noncooperative work. Aborting does not imply async cleanup
    /// ran; receipts report cancellation, and drain waits for future destruction.
    pub fn abort_requests(&self) -> Result<(), SessionError> {
        self.cancel_closed(Lane::Request, true)
    }
    pub fn abort_effects(&self) -> Result<(), SessionError> {
        self.cancel_closed(Lane::Effect, true)
    }
    fn cancel_closed(&self, lane: Lane, abort: bool) -> Result<(), SessionError> {
        if lock(&self.lifetime.shared.state)
            .lane(lane)
            .receipt
            .accepting
        {
            return Err(SessionError::InvalidRequest(
                "stop Hub task admission before cancelling its lane".into(),
            ));
        }
        self.lifetime.shared.cancel(lane, abort);
        Ok(())
    }

    /// Wait for all requests AND unstarted permits. Stop admission first for a
    /// final drain; an idle accepting lane may accept work immediately afterward.
    pub async fn drain_requests(&self) -> TaskLaneSnapshot {
        self.lifetime.shared.drain(Lane::Request).await
    }
    /// Keep effects accepting throughout request drain. Stop effects afterward
    /// for a final drain, before shutting down storage, sockets, or the runtime.
    pub async fn drain_effects(&self) -> TaskLaneSnapshot {
        self.lifetime.shared.drain(Lane::Effect).await
    }
    pub fn snapshot(&self) -> TaskOwnerSnapshot {
        let state = lock(&self.lifetime.shared.state);
        TaskOwnerSnapshot {
            requests: state.requests.snapshot(),
            effects: state.effects.snapshot(),
        }
    }
    pub fn approval_effects(&self) -> HubApprovalEffectOwner {
        HubApprovalEffectOwner {
            lifetime: Arc::downgrade(&self.lifetime),
            shared: self.lifetime.shared.clone(),
        }
    }
}

impl Shared {
    fn cancel(&self, lane: Lane, abort: bool) {
        let (cancellations, aborts) = {
            let mut state = lock(&self.state);
            let lane = state.lane_mut(lane);
            let mut cancellations = Vec::with_capacity(lane.tasks.len());
            let mut aborts = Vec::new();
            for task in lane.tasks.values_mut() {
                cancellations.push(task.cancel.clone());
                if abort {
                    task.abort_requested = true;
                    aborts.extend(task.abort.clone());
                }
            }
            (cancellations, aborts)
        };
        // Waking or aborting tasks must not execute arbitrary code under State.
        for cancel in cancellations {
            cancel.cancel();
        }
        for abort in aborts {
            abort.abort();
        }
    }
    async fn drain(&self, lane: Lane) -> TaskLaneSnapshot {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            // notify_waiters does not store a permit: register BEFORE observing
            // tasks, including when several independent drains wait together.
            changed.as_mut().enable();
            {
                let state = lock(&self.state);
                if state.lane(lane).tasks.is_empty() {
                    return state.lane(lane).snapshot();
                }
            }
            changed.await;
        }
    }
}

struct Reservation {
    shared: Arc<Shared>,
    lane: Lane,
    id: u64,
}
impl Reservation {
    fn finish(self, result: Finish) {
        {
            let mut state = lock(&self.shared.state);
            let lane = state.lane_mut(self.lane);
            if lane.tasks.remove(&self.id).is_none() {
                return;
            }
            let count = match result {
                Finish::Completed => &mut lane.receipt.completed,
                Finish::Panicked => &mut lane.receipt.panicked,
                Finish::Cancelled => &mut lane.receipt.cancelled,
                Finish::AbandonedPermit => &mut lane.receipt.abandoned_permits,
            };
            *count = count.saturating_add(1);
        }
        self.shared.changed.notify_waiters();
    }
}
enum Finish {
    Completed,
    Panicked,
    Cancelled,
    AbandonedPermit,
}

/// An owned reservation is counted immediately and survives admission closure.
/// Drop releases an unused reservation. Obtain one before irreversible handoff.
#[must_use = "dropping the permit releases its unstarted reservation"]
pub struct OwnedTaskPermit {
    reservation: Option<Reservation>,
    runtime: Handle,
    cancel: TaskCancellation,
}
impl OwnedTaskPermit {
    pub fn cancellation(&self) -> TaskCancellation {
        self.cancel.clone()
    }
    /// Synchronous, infallible transfer; no channel enqueue or admission recheck.
    /// Even an unpolled task has its guard before entering Tokio. Runtime shutdown
    /// drops that guard and future and reports cancellation instead of success.
    pub fn start<F, T>(mut self, future: F) -> TaskWaiter<T>
    where
        F: Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        let reservation = self.reservation.take().expect("unconsumed task permit");
        let shared = reservation.shared.clone();
        let lane = reservation.lane;
        let id = reservation.id;
        let already_aborted = {
            let mut state = lock(&shared.state);
            let task = state
                .lane_mut(lane)
                .tasks
                .get_mut(&id)
                .expect("owned permit");
            task.started = true;
            task.abort_requested
        };
        let (sender, receiver) = oneshot::channel();
        let task = OwnedTask {
            cancel: self.cancel.clone(),
            future: Some(Box::pin(future)),
            reservation: Some(reservation),
            sender: Some(sender),
        };
        if already_aborted {
            // An abandoned owner cannot begin new work merely because its
            // previously issued permit was retained. Destruction is tracked.
            drop(task);
            return TaskWaiter { receiver };
        }
        // A Handle is captured at owner construction: this works without an
        // ambient runtime, and spawn accepts ownership even during shutdown.
        let spawned = self.runtime.spawn(task);
        let abort = spawned.abort_handle();
        let should_abort = {
            let mut state = lock(&shared.state);
            if let Some(task) = state.lane_mut(lane).tasks.get_mut(&id) {
                task.abort = Some(abort.clone());
                task.abort_requested
            } else {
                // The task may already have completed on another worker.
                false
            }
        };
        if should_abort {
            abort.abort();
        }
        // The live record owns its abort handle and the future owns its guard.
        // JoinHandle detachment does not detach the task from lifecycle tracking.
        drop(spawned);
        TaskWaiter { receiver }
    }
}
impl Drop for OwnedTaskPermit {
    fn drop(&mut self) {
        if let Some(reservation) = self.reservation.take() {
            reservation.finish(Finish::AbandonedPermit);
        }
    }
}

#[must_use = "wait for the result or drop the waiter without cancelling the task"]
pub struct TaskWaiter<T> {
    receiver: oneshot::Receiver<Result<T, TaskFailure>>,
}
impl<T> TaskWaiter<T> {
    pub async fn wait(self) -> Result<T, TaskFailure> {
        self.receiver.await.unwrap_or(Err(TaskFailure::Cancelled))
    }
}

/// Own the job explicitly so its destructor (admission and cleanup guards) runs
/// BEFORE removing the live record. Merely capturing both in an async block does
/// not establish this ordering when Tokio drops a never-polled task.
struct OwnedTask<F, T> {
    cancel: TaskCancellation,
    future: Option<Pin<Box<F>>>,
    reservation: Option<Reservation>,
    sender: Option<oneshot::Sender<Result<T, TaskFailure>>>,
}
impl<F, T> Unpin for OwnedTask<F, T> {}
impl<F, T> OwnedTask<F, T> {
    fn drop_future(&mut self) -> bool {
        catch_unwind(AssertUnwindSafe(|| drop(self.future.take()))).is_err()
    }
    fn finish(&mut self, result: Result<T, TaskFailure>) {
        let mut finish = match &result {
            Ok(_) => Finish::Completed,
            Err(TaskFailure::Panicked) => Finish::Panicked,
            Err(TaskFailure::Cancelled) => Finish::Cancelled,
        };
        // A dropped receiver can make send return the owned result; drop that
        // result before drain sees completion, including a panicking destructor.
        if catch_unwind(AssertUnwindSafe(|| {
            if let Some(sender) = self.sender.take() {
                let _ = sender.send(result);
            }
        }))
        .is_err()
        {
            finish = Finish::Panicked;
        }
        if let Some(reservation) = self.reservation.take() {
            reservation.finish(finish);
        }
    }
}
impl<F: Future<Output = T>, T> Future for OwnedTask<F, T> {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        let polled = catch_unwind(AssertUnwindSafe(|| {
            this.future.as_mut().expect("live task").as_mut().poll(cx)
        }));
        match polled {
            Ok(Poll::Pending) => Poll::Pending,
            Ok(Poll::Ready(value)) => {
                if this.drop_future() {
                    let _ = catch_unwind(AssertUnwindSafe(|| drop(value)));
                    this.finish(Err(TaskFailure::Panicked));
                } else {
                    this.finish(Ok(value));
                }
                Poll::Ready(())
            }
            Err(_) => {
                this.cancel.cancel();
                this.drop_future();
                this.finish(Err(TaskFailure::Panicked));
                Poll::Ready(())
            }
        }
    }
}
impl<F, T> Drop for OwnedTask<F, T> {
    fn drop(&mut self) {
        if self.reservation.is_some() {
            // Also signal cancellation on runtime abandonment, before dropping
            // the job and its synchronous cleanup/admission guards.
            self.cancel.cancel();
        }
        let panicked = self.drop_future();
        if self.reservation.is_some() {
            self.finish(Err(if panicked {
                TaskFailure::Panicked
            } else {
                TaskFailure::Cancelled
            }));
        }
    }
}

/// Concrete adapter for approval closure effects. Construct it from the SAME
/// owner used by HTTP/confirmation admission. The adapter does not keep the
/// lifecycle owner alive, even when captured by one of its tasks. Its drain
/// waits for effect idleness
/// and never stops admission; the outer lifecycle must stop/drain requests and
/// call `stop_effects` first when performing final shutdown.
#[derive(Clone)]
pub struct HubApprovalEffectOwner {
    lifetime: Weak<Lifetime>,
    shared: Arc<Shared>,
}
impl ApprovalEffectOwner for HubApprovalEffectOwner {
    fn start(&self, job: CoreFuture<'static, ()>) -> CoreFuture<'static, Result<(), SessionError>> {
        let permit = self
            .lifetime
            .upgrade()
            .ok_or(SessionError::Shutdown)
            .and_then(|lifetime| HubTaskOwner { lifetime }.effect_permit());
        match permit {
            Ok(permit) => {
                let waiter = permit.start(job);
                Box::pin(async move { waiter.wait().await.map_err(Into::into) })
            }
            Err(error) => {
                drop(job);
                Box::pin(async move { Err(error) })
            }
        }
    }
    fn drain(&self) -> CoreFuture<'_, ()> {
        Box::pin(async move {
            self.shared.drain(Lane::Effect).await;
        })
    }
}

#[cfg(test)]
mod tests;
