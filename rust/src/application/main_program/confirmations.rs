//! One-time construction binding for the actual source child executor.
use crate::{proto::core::*, terminal::session::confirmations::*};
use std::sync::{Arc, OnceLock};
#[derive(Default)]
pub struct BoundConfirmationExecutor(OnceLock<Arc<dyn ConfirmationExecutor>>);
impl BoundConfirmationExecutor {
    pub fn bind(&self, executor: Arc<dyn ConfirmationExecutor>) -> Result<(), SessionError> {
        self.0
            .set(executor)
            .map_err(|_| SessionError::InvalidRequest("child executor already bound".into()))
    }
    fn owner(&self) -> Result<&dyn ConfirmationExecutor, SessionError> {
        self.0.get().map(Arc::as_ref).ok_or(SessionError::Shutdown)
    }
}
impl ConfirmationExecutor for BoundConfirmationExecutor {
    fn presentation(&self) -> Result<ConfirmationPresentation, SessionError> {
        self.owner()?.presentation()
    }
    fn spawn<'a>(
        &'a self,
        request: ConfirmedChildRequest,
        cancel: TaskCancellation,
    ) -> CoreFuture<'a, Result<ChildSpawnResult, SessionError>> {
        Box::pin(async move { self.owner()?.spawn(request, cancel).await })
    }
    fn record_refusal<'a>(
        &'a self,
        pending: &'a PendingSpawnConfirmation,
        cancel: TaskCancellation,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move { self.owner()?.record_refusal(pending, cancel).await })
    }
    fn notify_waiter_gone<'a>(
        &'a self,
        pending: &'a PendingSpawnConfirmation,
        result: &'a Result<ChildSpawnResult, SessionError>,
        cancel: TaskCancellation,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            self.owner()?
                .notify_waiter_gone(pending, result, cancel)
                .await
        })
    }
}
