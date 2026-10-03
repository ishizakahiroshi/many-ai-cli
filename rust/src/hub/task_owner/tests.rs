use super::*;
use crate::proto::core::{AcceptedSpawnDecision, ConfirmationOutcome, SpawnConfirmationId};
use futures_util::{
    poll,
    task::{ArcWake, waker_ref},
};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::runtime::Builder;

fn owner() -> HubTaskOwner {
    HubTaskOwner::new(Handle::current())
}
fn runtime() -> tokio::runtime::Runtime {
    Builder::new_current_thread().build().unwrap()
}
#[derive(Default)]
struct WakeCount(AtomicUsize);
impl ArcWake for WakeCount {
    fn wake_by_ref(arc_self: &Arc<Self>) {
        arc_self.0.fetch_add(1, Ordering::SeqCst);
    }
}
struct DropMark(Arc<AtomicUsize>);
impl Drop for DropMark {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
struct PendingJob {
    dropped: DropMark,
    polled: Arc<AtomicBool>,
}
impl Future for PendingJob {
    type Output = ();
    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
        self.polled.store(true, Ordering::SeqCst);
        Poll::Pending
    }
}
impl Drop for PendingJob {
    fn drop(&mut self) {
        // Keep the marker as an ordinary field: its destructor must also run
        // before the owner publishes its empty-lane wake.
        let _ = &self.dropped;
    }
}
fn pending_job() -> (PendingJob, Arc<AtomicUsize>, Arc<AtomicBool>) {
    let dropped = Arc::new(AtomicUsize::new(0));
    let polled = Arc::new(AtomicBool::new(false));
    (
        PendingJob {
            dropped: DropMark(dropped.clone()),
            polled: polled.clone(),
        },
        dropped,
        polled,
    )
}

#[tokio::test]
async fn lane_close_checks_admission_and_unstarted_requests() {
    let owner = owner();
    assert!(owner.stop_effects().is_err());
    assert!(owner.cancel_requests().is_err());
    assert!(owner.cancel_effects().is_err());
    assert!(owner.abort_effects().is_err());
    let request = owner.request_permit().unwrap();
    owner.stop_requests();
    assert!(matches!(
        owner.request_permit(),
        Err(SessionError::Shutdown)
    ));
    assert!(owner.stop_effects().is_err());
    let effects = owner.effect_permit().unwrap();
    let drain = owner.drain_requests();
    tokio::pin!(drain);
    assert!(poll!(&mut drain).is_pending());
    drop(request);
    let receipt = drain.await;
    assert_eq!(receipt.abandoned_permits, 1);
    assert_eq!(receipt.unstarted, 0);
    owner.stop_effects().unwrap();
    owner.stop_effects().unwrap();
    assert!(matches!(owner.effect_permit(), Err(SessionError::Shutdown)));
    // A permit obtained while HTTP requests were draining remains valid.
    assert_eq!(effects.start(async { 17 }).wait().await, Ok(17));
    assert_eq!(owner.drain_effects().await.completed, 1);
}

#[tokio::test]
async fn response_drop_does_not_drop_request_or_its_late_effects() {
    let owner = owner();
    let effect_owner = owner.approval_effects();
    let (entered, started) = oneshot::channel();
    let (resume, released) = oneshot::channel();
    let effects_ran = Arc::new(AtomicUsize::new(0));
    let ran = effects_ran.clone();
    let waiter = owner.request_permit().unwrap().start(async move {
        entered.send(()).unwrap();
        released.await.unwrap();
        effect_owner
            .start(Box::pin(async move {
                ran.fetch_add(1, Ordering::SeqCst);
            }))
            .await
            .unwrap();
        23
    });
    started.await.unwrap();
    drop(waiter);
    owner.stop_requests();
    assert!(owner.stop_effects().is_err());
    let drain = owner.drain_requests();
    tokio::pin!(drain);
    assert!(poll!(&mut drain).is_pending());
    assert_eq!(effects_ran.load(Ordering::SeqCst), 0);
    resume.send(()).unwrap();
    assert_eq!(drain.await.completed, 1);
    owner.stop_effects().unwrap();
    assert_eq!(owner.drain_effects().await.completed, 1);
    assert_eq!(effects_ran.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn dropping_effect_waiter_before_first_poll_does_not_cancel_job() {
    let owner = owner();
    let effects = owner.approval_effects();
    let ran = Arc::new(AtomicUsize::new(0));
    let in_job = ran.clone();
    let waiter = effects.start(Box::pin(async move {
        in_job.fetch_add(1, Ordering::SeqCst);
    }));
    drop(waiter);
    owner.stop_requests();
    owner.stop_effects().unwrap();
    effects.drain().await;
    assert_eq!(ran.load(Ordering::SeqCst), 1);
    assert_eq!(owner.snapshot().effects.completed, 1);
}

struct Decision {
    id: SpawnConfirmationId,
    ran: Arc<AtomicUsize>,
    dropped: Arc<AtomicUsize>,
}
impl AcceptedSpawnDecision for Decision {
    fn id(&self) -> &SpawnConfirmationId {
        &self.id
    }
    fn run(
        self: Box<Self>,
        cancel: TaskCancellation,
    ) -> Result<CoreFuture<'static, ConfirmationOutcome>, SessionError> {
        assert!(!cancel.token().is_cancelled());
        self.ran.fetch_add(1, Ordering::SeqCst);
        let guard = DropMark(self.dropped.clone());
        Ok(Box::pin(async move {
            let _guard = guard;
            ConfirmationOutcome::Refused
        }))
    }
}

#[tokio::test]
async fn decision_handoff_uses_preacquired_permit_after_lane_closes() {
    let owner = owner();
    let ran = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicUsize::new(0));
    let decision: Box<dyn AcceptedSpawnDecision> = Box::new(Decision {
        id: SpawnConfirmationId("synthetic-confirmation".into()),
        ran: ran.clone(),
        dropped: dropped.clone(),
    });
    let permit = owner.effect_permit().unwrap();
    owner.stop_requests();
    owner.stop_effects().unwrap();
    // The actual core contract's commit and transfer contain no await/fallible
    // enqueue. A lane closure cannot invalidate this already-owned permit.
    let future = decision.run(permit.cancellation()).unwrap();
    let waiter = permit.start(future);
    assert_eq!(ran.load(Ordering::SeqCst), 1);
    assert_eq!(dropped.load(Ordering::SeqCst), 0);
    drop(waiter);
    assert_eq!(owner.drain_effects().await.completed, 1);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn several_drains_wake_when_unstarted_permit_is_dropped() {
    let owner = owner();
    let permit = owner.effect_permit().unwrap();
    owner.stop_requests();
    owner.stop_effects().unwrap();
    let one = owner.drain_effects();
    let two = owner.drain_effects();
    tokio::pin!(one, two);
    let wake_one = Arc::new(WakeCount::default());
    let wake_two = Arc::new(WakeCount::default());
    let waker_one = waker_ref(&wake_one);
    let waker_two = waker_ref(&wake_two);
    let mut cx_one = Context::from_waker(&waker_one);
    let mut cx_two = Context::from_waker(&waker_two);
    assert!(one.as_mut().poll(&mut cx_one).is_pending());
    assert!(two.as_mut().poll(&mut cx_two).is_pending());
    drop(permit);
    // Do not hide a missed notify by manually repolling an unwoken future.
    assert_eq!(wake_one.0.load(Ordering::SeqCst), 1);
    assert_eq!(wake_two.0.load(Ordering::SeqCst), 1);
    assert!(one.as_mut().poll(&mut cx_one).is_ready());
    assert!(two.as_mut().poll(&mut cx_two).is_ready());
    assert_eq!(owner.snapshot().effects.abandoned_permits, 1);
    // Completion before a drain's first poll also cannot be lost.
    assert_eq!(owner.drain_effects().await.abandoned_permits, 1);
}

#[tokio::test]
async fn cancelled_drain_can_be_replaced_without_losing_completion() {
    let owner = owner();
    let permit = owner.effect_permit().unwrap();
    owner.stop_requests();
    owner.stop_effects().unwrap();
    {
        let drain = owner.drain_effects();
        tokio::pin!(drain);
        assert!(poll!(&mut drain).is_pending());
    }
    drop(permit);
    assert_eq!(owner.drain_effects().await.abandoned_permits, 1);
}

#[tokio::test]
async fn completed_result_survives_late_receiver_poll_and_other_clone_drop() {
    let owner = owner();
    let other = owner.clone();
    let waiter = owner.effect_permit().unwrap().start(async { 41 });
    drop(other);
    assert_eq!(owner.drain_effects().await.completed, 1);
    assert_eq!(waiter.wait().await, Ok(41));
    assert!(owner.effect_permit().is_ok());
}

#[tokio::test]
async fn panic_is_reported_even_after_completion_waiter_is_gone() {
    let owner = owner();
    let dropped = Arc::new(AtomicUsize::new(0));
    let guard = DropMark(dropped.clone());
    let waiter = owner.effect_permit().unwrap().start(async move {
        let _guard = guard;
        panic!("synthetic owned-task panic");
    });
    drop(waiter);
    owner.stop_requests();
    owner.stop_effects().unwrap();
    let receipt = owner.drain_effects().await;
    assert_eq!(receipt.panicked, 1);
    assert_eq!(receipt.completed, 0);
    assert_eq!(receipt.running, 0);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn panic_after_suspension_returns_typed_failure() {
    let owner = owner();
    let (resume, released) = oneshot::channel();
    let waiter = owner.effect_permit().unwrap().start(async move {
        released.await.unwrap();
        panic!("synthetic resumed-task panic");
    });
    resume.send(()).unwrap();
    assert_eq!(waiter.wait().await, Err(TaskFailure::Panicked));
    assert_eq!(owner.drain_effects().await.panicked, 1);
}

#[tokio::test]
async fn permit_unwind_releases_reservation_without_starting_a_job() {
    let owner = owner();
    let result = catch_unwind(AssertUnwindSafe(|| {
        let _permit = owner.effect_permit().unwrap();
        panic!("synthetic pre-handoff failure");
    }));
    assert!(result.is_err());
    assert_eq!(owner.drain_effects().await.abandoned_permits, 1);
    assert_eq!(owner.snapshot().effects.running, 0);
}

#[tokio::test]
async fn cooperative_task_cancellation_waits_for_async_cleanup() {
    let owner = owner();
    let permit = owner.effect_permit().unwrap();
    let cancellation = permit.cancellation();
    let (saw_cancel, cancelled) = oneshot::channel();
    let (finish_cleanup, cleanup) = oneshot::channel();
    let waiter = permit.start(async move {
        cancellation.token().cancelled().await;
        saw_cancel.send(()).unwrap();
        cleanup.await.unwrap();
        59
    });
    owner.stop_requests();
    owner.stop_effects().unwrap();
    owner.cancel_effects().unwrap();
    cancelled.await.unwrap();
    let drain = owner.drain_effects();
    tokio::pin!(drain);
    assert!(poll!(&mut drain).is_pending());
    finish_cleanup.send(()).unwrap();
    assert_eq!(waiter.wait().await, Ok(59));
    assert_eq!(drain.await.completed, 1);
}

#[tokio::test]
async fn cancellation_reaches_unstarted_permit_and_does_not_cancel_other_lane() {
    let owner = owner();
    let request = owner.request_permit().unwrap();
    let effect = owner.effect_permit().unwrap();
    let request_cancel = request.cancellation();
    let effect_cancel = effect.cancellation();
    owner.stop_requests();
    owner.cancel_requests().unwrap();
    assert!(request_cancel.token().is_cancelled());
    assert!(!effect_cancel.token().is_cancelled());
    drop(request);
    owner.stop_effects().unwrap();
    owner.cancel_effects().unwrap();
    assert!(effect_cancel.token().is_cancelled());
    let waiter = effect.start(async move { effect_cancel.token().is_cancelled() });
    assert_eq!(waiter.wait().await, Ok(true));
}

#[tokio::test]
async fn forced_abort_drops_job_before_drain_returns() {
    let owner = owner();
    let (job, dropped, polled) = pending_job();
    let waiter = owner.effect_permit().unwrap().start(job);
    tokio::task::yield_now().await;
    assert!(polled.load(Ordering::SeqCst));
    owner.stop_requests();
    owner.stop_effects().unwrap();
    owner.abort_effects().unwrap();
    assert_eq!(owner.drain_effects().await.cancelled, 1);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert_eq!(waiter.wait().await, Err(TaskFailure::Cancelled));
}

#[test]
fn last_owner_drop_aborts_never_polled_task_but_adapter_does_not_keep_it_alive() {
    let runtime = runtime();
    let owner = HubTaskOwner::new(runtime.handle().clone());
    let effects = owner.approval_effects();
    let shared = owner.lifetime.shared.clone();
    let (job, dropped, polled) = pending_job();
    let waiter = effects.start(Box::pin(job));
    drop(owner);
    assert_eq!(runtime.block_on(waiter), Err(SessionError::Cancelled));
    assert!(!polled.load(Ordering::SeqCst));
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert_eq!(runtime.block_on(shared.drain(Lane::Effect)).cancelled, 1);
    assert_eq!(
        runtime.block_on(effects.start(Box::pin(async {}))),
        Err(SessionError::Shutdown)
    );
}

#[test]
fn runtime_abandonment_drops_unpolled_future_and_publishes_cancellation() {
    let runtime = runtime();
    let owner = HubTaskOwner::new(runtime.handle().clone());
    let (job, dropped, polled) = pending_job();
    let permit = owner.effect_permit().unwrap();
    let cancellation = permit.cancellation();
    let waiter = permit.start(job);
    drop(runtime);
    assert!(cancellation.token().is_cancelled());
    assert!(!polled.load(Ordering::SeqCst));
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    let receipt = owner.snapshot().effects;
    assert_eq!(receipt.running, 0);
    assert_eq!(receipt.cancelled, 1);
    let replacement = self::runtime();
    assert_eq!(
        replacement.block_on(waiter.wait()),
        Err(TaskFailure::Cancelled)
    );
    assert_eq!(replacement.block_on(owner.drain_effects()).cancelled, 1);
}

#[test]
fn start_after_runtime_shutdown_settles_instead_of_leaking_a_permit() {
    let runtime = runtime();
    let owner = HubTaskOwner::new(runtime.handle().clone());
    let permit = owner.effect_permit().unwrap();
    drop(runtime);
    let (job, dropped, polled) = pending_job();
    let waiter = permit.start(job);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert!(!polled.load(Ordering::SeqCst));
    assert_eq!(owner.snapshot().effects.cancelled, 1);
    let replacement = self::runtime();
    assert_eq!(
        replacement.block_on(waiter.wait()),
        Err(TaskFailure::Cancelled)
    );
}

#[test]
fn permit_keeps_bookkeeping_but_not_the_owner_alive() {
    let runtime = runtime();
    let owner = HubTaskOwner::new(runtime.handle().clone());
    let permit = owner.effect_permit().unwrap();
    let cancel = permit.cancellation();
    let shared = owner.lifetime.shared.clone();
    drop(owner);
    assert!(cancel.token().is_cancelled());
    let (job, dropped, polled) = pending_job();
    let waiter = permit.start(job);
    assert_eq!(runtime.block_on(waiter.wait()), Err(TaskFailure::Cancelled));
    assert!(!polled.load(Ordering::SeqCst));
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert_eq!(runtime.block_on(shared.drain(Lane::Effect)).cancelled, 1);
}

struct PanicOnDrop;
impl Future for PanicOnDrop {
    type Output = ();
    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
        Poll::Pending
    }
}
impl Drop for PanicOnDrop {
    fn drop(&mut self) {
        panic!("synthetic future destructor panic");
    }
}
#[tokio::test]
async fn destructor_panic_is_reported_without_stranding_drain() {
    let owner = owner();
    let waiter = owner.effect_permit().unwrap().start(PanicOnDrop);
    owner.stop_requests();
    owner.stop_effects().unwrap();
    owner.abort_effects().unwrap();
    assert_eq!(waiter.wait().await, Err(TaskFailure::Panicked));
    assert_eq!(owner.drain_effects().await.panicked, 1);
}
