//! Ordered callbacks share the actual instruction owner without a Core cycle.
use crate::{
    application::instruction_rules::InstructionRules,
    hub::{sockets::OrderedEventObserver, task_owner::HubTaskHandle},
    process::Cancellation,
    proto::core::*,
};
use std::sync::{Arc, OnceLock, Weak};
#[cfg(test)]
mod tests;
pub struct InstructionObserver {
    next: Arc<dyn OrderedEventObserver>,
    rules: OnceLock<Weak<InstructionRules>>,
    tasks: HubTaskHandle,
    cancel: Cancellation,
}
impl InstructionObserver {
    pub fn new(
        next: Arc<dyn OrderedEventObserver>,
        tasks: HubTaskHandle,
        cancel: Cancellation,
    ) -> Arc<Self> {
        Arc::new(Self {
            next,
            rules: OnceLock::new(),
            tasks,
            cancel,
        })
    }
    pub fn bind(&self, rules: Weak<InstructionRules>) -> Result<(), SessionError> {
        self.rules
            .set(rules)
            .map_err(|_| SessionError::InvalidRequest("instruction owner already bound".into()))
    }
    pub fn settings_published(&self) -> Result<(), SessionError> {
        let rules = self
            .rules
            .get()
            .and_then(Weak::upgrade)
            .ok_or(SessionError::Shutdown)?;
        let cancel = self.cancel.clone();
        drop(self.tasks.effect_permit()?.start(async move {
            rules.registered(&cancel).await;
        }));
        Ok(())
    }
}
impl OrderedEventObserver for InstructionObserver {
    fn observe<'a>(&'a self, event: &'a CoreEvent) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            // Instruction lifetime follows the core event even when a downstream
            // publication fails. Preserve that failure after housekeeping runs.
            let result = self.next.observe(event).await;
            if let Some(rules) = self.rules.get().and_then(Weak::upgrade) {
                match event {
                    // Register/reattach preparation belongs to the WebSocket
                    // caller's pre/post-ACK barrier, never this publication pass.
                    CoreEvent::Ended { .. } | CoreEvent::Dismissed(_) => {
                        rules.ended(&self.cancel).await
                    }
                    _ => {}
                }
            }
            result
        })
    }
    fn approval_opened<'a>(
        &'a self,
        session: LiveSessionId,
        record: &'a ImmutableApprovalRecord,
    ) -> CoreFuture<'a, Result<bool, SessionError>> {
        self.next.approval_opened(session, record)
    }
}
