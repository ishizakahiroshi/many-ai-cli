use super::{sockets::SocketRegistry, task_owner::HubTaskHandle};
use crate::{process::Cancellation, proto::core::SessionError};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
/// Shared intentional-stop owner. The API and signal path announce once,
/// before socket cancellation, and every delay remains in the effect lane.
pub struct ShutdownOwner {
    cancel: Cancellation,
    sockets: Arc<SocketRegistry>,
    tasks: HubTaskHandle,
    announced: AtomicBool,
}
impl ShutdownOwner {
    pub fn new(cancel: Cancellation, sockets: Arc<SocketRegistry>, tasks: HubTaskHandle) -> Self {
        Self {
            cancel,
            sockets,
            tasks,
            announced: false.into(),
        }
    }
    pub async fn announce(&self, reason: &str) -> Result<(), SessionError> {
        if !self.announced.swap(true, Ordering::SeqCst) {
            self.sockets.notify_hub_shutdown(reason).await?;
        }
        Ok(())
    }
    pub async fn request(&self) -> Result<(), SessionError> {
        let permit = self.tasks.effect_permit()?;
        self.announce("ui_shutdown").await?;
        let cancel = self.cancel.clone();
        drop(permit.start(async move {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            cancel.cancel();
        }));
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test(start_paused = true)]
    async fn intentional_shutdown_retains_delay_and_closes_only_after_response_window() {
        let tasks = super::super::task_owner::HubTaskOwner::new(tokio::runtime::Handle::current());
        let cancel = Cancellation::default();
        let owner = ShutdownOwner::new(
            cancel.clone(),
            Arc::new(SocketRegistry::default()),
            tasks.handle(),
        );
        owner.request().await.unwrap();
        assert!(!cancel.is_cancelled());
        tasks.stop_requests();
        assert!(tasks.request_permit().is_err());
        // The retained timer starts even if its response waiter was dropped.
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_millis(99)).await;
        assert!(!cancel.is_cancelled());
        tokio::time::advance(std::time::Duration::from_millis(1)).await;
        tokio::task::yield_now().await;
        assert!(cancel.is_cancelled());
        assert!(tasks.drain_requests().await.running == 0);
        tasks.stop_effects().unwrap();
        assert_eq!(tasks.drain_effects().await.completed, 1);
    }
}
