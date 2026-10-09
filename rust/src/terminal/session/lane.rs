//! FIFO tickets are reserved synchronously, before a returned future is polled.
//! Dropping a waiting operation marks only its own ticket complete. Successors
//! still wait for every earlier live ticket, including when polling is reversed.
use super::lock;
use crate::proto::core::TaskCancellation;
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
};
#[derive(Default)]
pub(super) struct InputLane {
    state: Mutex<State>,
    changed: tokio::sync::Notify,
}
#[derive(Default)]
struct State {
    issued: u64,
    serving: u64,
    finished: BTreeSet<u64>,
}
pub(super) struct Ticket {
    lane: Arc<InputLane>,
    number: u64,
}
impl InputLane {
    pub fn reserve(self: &Arc<Self>) -> Ticket {
        let mut state = lock(&self.state);
        let number = state.issued;
        state.issued = state
            .issued
            .checked_add(1)
            .expect("input ticket identity exhausted");
        Ticket {
            lane: self.clone(),
            number,
        }
    }
}
impl Ticket {
    pub async fn wait_ordered(&self) {
        loop {
            let changed = self.lane.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if lock(&self.lane.state).serving == self.number {
                return;
            }
            changed.await;
        }
    }
    pub async fn wait(&self, cancel: &TaskCancellation) -> bool {
        loop {
            let changed = self.lane.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if lock(&self.lane.state).serving == self.number {
                return !cancel.token().is_cancelled();
            }
            tokio::select! {biased;_=cancel.token().cancelled()=>return false,_=changed=>{}}
        }
    }
}
impl Drop for Ticket {
    fn drop(&mut self) {
        let mut state = lock(&self.lane.state);
        state.finished.insert(self.number);
        loop {
            let next = state.serving;
            if !state.finished.remove(&next) {
                break;
            }
            state.serving = state
                .serving
                .checked_add(1)
                .expect("input ticket identity exhausted");
        }
        drop(state);
        self.lane.changed.notify_waiters();
    }
}

impl crate::proto::core::PersistenceOrder for Ticket {
    fn wait(&self) -> crate::proto::core::CoreFuture<'_, ()> {
        Box::pin(self.wait_ordered())
    }
}
