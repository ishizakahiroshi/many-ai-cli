//! Single ordered core event bus. Construct before the transport effect sink;
//! publish only while applying CoreEffect::Notify, never when producing effects.
use crate::proto::core::*;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
#[derive(Clone)]
pub struct CoreEventBus {
    inner: Arc<Inner>,
}
struct Inner {
    sequence: Mutex<u64>,
    sender: broadcast::Sender<VersionedCoreEvent>,
}
impl CoreEventBus {
    pub fn new(capacity: usize) -> Result<Self, SessionError> {
        if capacity == 0 {
            return Err(SessionError::InvalidRequest(
                "core event capacity must be positive".into(),
            ));
        }
        let (sender, _) = broadcast::channel(capacity);
        Ok(Self {
            inner: Arc::new(Inner {
                sequence: Mutex::new(0),
                sender,
            }),
        })
    }
    pub fn subscribe(&self) -> Box<dyn CoreEventSubscription> {
        Box::new(Subscription {
            receiver: self.inner.sender.subscribe(),
        })
    }
}
impl CoreEventPublisher for CoreEventBus {
    fn publish(&self, event: CoreEvent) -> Result<EventSequence, SessionError> {
        let mut sequence = self
            .inner
            .sequence
            .lock()
            .map_err(|_| SessionError::Shutdown)?;
        *sequence = sequence
            .checked_add(1)
            .ok_or_else(|| SessionError::InvalidRequest("core event sequence exhausted".into()))?;
        let sequence = EventSequence(*sequence);
        // No subscribers is ordinary during startup/teardown, not a false error.
        // Keep the sequence lock until send so concurrent publishers stay ordered.
        let _ = self
            .inner
            .sender
            .send(VersionedCoreEvent { sequence, event });
        Ok(sequence)
    }
}
struct Subscription {
    receiver: broadcast::Receiver<VersionedCoreEvent>,
}
impl CoreEventSubscription for Subscription {
    fn next<'a>(
        &'a mut self,
        cancel: &'a HubShutdownCancellation,
    ) -> CoreFuture<'a, CoreEventPoll> {
        Box::pin(async move {
            tokio::select! {biased;_=cancel.token().cancelled()=>CoreEventPoll::Cancelled,result=self.receiver.recv()=>match result{Ok(event)=>CoreEventPoll::Event(Box::new(event)),Err(broadcast::error::RecvError::Lagged(missed))=>CoreEventPoll::Lagged{missed},Err(broadcast::error::RecvError::Closed)=>CoreEventPoll::Closed}}
        })
    }
}
