use super::{sockets::SocketRegistry, task_owner::HubTaskHandle};
use crate::{process::Cancellation, proto::core::SessionError};
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, Ordering},
};
/// Callers of `/api/shutdown` name themselves with this header so the Hub log
/// can say why it stopped. Only the fixed sources below reach the log.
pub const STOP_SOURCE_HEADER: &str = "x-many-ai-cli-stop-source";
const STOP_SOURCES: &[&str] = &["stop_command", "web_ui", "stale_binary_restart", "launcher"];
/// Unknown or missing header values never become log text.
pub fn stop_source(header: &str) -> &'static str {
    STOP_SOURCES
        .iter()
        .copied()
        .find(|source| *source == header.trim())
        .unwrap_or("api_unspecified")
}
/// Detail for the stopping/stopped lines: the first recorded reason, otherwise
/// a listener failure, otherwise `unknown` (a stop no path should produce).
pub fn stop_reason(recorded: Option<&str>, server_error: Option<&dyn std::fmt::Display>) -> String {
    match (recorded, server_error) {
        (Some(reason), _) => reason.to_owned(),
        (None, Some(error)) => format!("server_error error={error}"),
        (None, None) => "unknown".to_owned(),
    }
}
/// Shared intentional-stop owner. The API and signal path announce once,
/// before socket cancellation, and every delay remains in the effect lane.
pub struct ShutdownOwner {
    cancel: Cancellation,
    sockets: Arc<SocketRegistry>,
    tasks: HubTaskHandle,
    announced: AtomicBool,
    reason: OnceLock<&'static str>,
}
impl ShutdownOwner {
    pub fn new(cancel: Cancellation, sockets: Arc<SocketRegistry>, tasks: HubTaskHandle) -> Self {
        Self {
            cancel,
            sockets,
            tasks,
            announced: false.into(),
            reason: OnceLock::new(),
        }
    }
    /// The first reason wins; a Ctrl+C during an API stop keeps the API source.
    pub fn record_reason(&self, reason: &'static str) {
        let _ = self.reason.set(reason);
    }
    pub fn reason(&self) -> Option<&'static str> {
        self.reason.get().copied()
    }
    pub async fn announce(&self, reason: &str) -> Result<(), SessionError> {
        if !self.announced.swap(true, Ordering::SeqCst) {
            self.sockets.notify_hub_shutdown(reason).await?;
        }
        Ok(())
    }
    pub async fn request(&self, source: &'static str) -> Result<(), SessionError> {
        let permit = self.tasks.effect_permit()?;
        self.record_reason(source);
        // Wrapper delivery is advisory in fixed Go. A stalled/disconnected
        // wrapper must not veto the already-authorized Hub stop.
        let _notice = self.announce("ui_shutdown").await;
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
        owner.request("stop_command").await.unwrap();
        assert_eq!(owner.reason(), Some("stop_command"));
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
    struct UnusedWriter;
    impl crate::hub::sockets::FrameWriter for UnusedWriter {
        fn write<'a>(
            &'a mut self,
            _: crate::hub::sockets::WireFrame,
        ) -> crate::proto::core::CoreFuture<'a, Result<(), SessionError>> {
            Box::pin(async {
                Err(SessionError::Transport(
                    "synthetic unavailable wrapper".into(),
                ))
            })
        }
    }
    #[tokio::test(start_paused = true)]
    async fn advisory_wrapper_failure_cannot_veto_an_authorized_shutdown() {
        use crate::proto::core::{
            LiveSessionId, SessionBinding, SessionIncarnation, WrapperConnectionId,
        };
        let tasks = super::super::task_owner::HubTaskOwner::new(tokio::runtime::Handle::current());
        let cancel = Cancellation::default();
        let sockets = Arc::new(SocketRegistry::default());
        sockets
            .insert_wrapper(
                SessionBinding {
                    session: LiveSessionId(1),
                    incarnation: SessionIncarnation(1),
                    wrapper: WrapperConnectionId(1),
                },
                Box::new(UnusedWriter),
            )
            .unwrap();
        // A stale/unacknowledged writer is a real registry error, not a mock
        // successful notification. The intentional stop remains independent.
        assert!(sockets.notify_hub_shutdown("probe").await.is_err());
        let owner = ShutdownOwner::new(cancel.clone(), sockets, tasks.handle());
        owner.request("web_ui").await.unwrap();
        assert!(!cancel.is_cancelled());
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_millis(100)).await;
        tokio::task::yield_now().await;
        assert!(cancel.is_cancelled());
        tasks.stop_requests();
        tasks.drain_requests().await;
        tasks.stop_effects().unwrap();
        assert_eq!(tasks.drain_effects().await.completed, 1);
    }
    #[test]
    fn stop_source_accepts_only_the_named_callers() {
        for source in STOP_SOURCES {
            assert_eq!(stop_source(source), *source);
        }
        assert_eq!(stop_source(" stop_command "), "stop_command");
        let long = "x".repeat(4096);
        for header in [
            "",
            "Stop_Command",
            "signal",
            "unknown",
            "web_ui; rm -rf /",
            &long,
        ] {
            assert_eq!(stop_source(header), "api_unspecified");
        }
    }
    #[tokio::test]
    async fn first_stop_reason_is_kept() {
        let tasks = super::super::task_owner::HubTaskOwner::new(tokio::runtime::Handle::current());
        let owner = ShutdownOwner::new(
            Cancellation::default(),
            Arc::new(SocketRegistry::default()),
            tasks.handle(),
        );
        assert_eq!(owner.reason(), None);
        owner.request("stale_binary_restart").await.unwrap();
        owner.record_reason("signal");
        assert_eq!(owner.reason(), Some("stale_binary_restart"));
        tasks.stop_requests();
        tasks.drain_requests().await;
        tasks.stop_effects().unwrap();
        tasks.drain_effects().await;
    }
    #[tokio::test]
    async fn refused_stop_does_not_record_a_reason() {
        let tasks = super::super::task_owner::HubTaskOwner::new(tokio::runtime::Handle::current());
        let owner = ShutdownOwner::new(
            Cancellation::default(),
            Arc::new(SocketRegistry::default()),
            tasks.handle(),
        );
        tasks.stop_requests();
        tasks.drain_requests().await;
        tasks.stop_effects().unwrap();
        assert!(owner.request("stop_command").await.is_err());
        assert_eq!(owner.reason(), None);
    }
    #[test]
    fn stop_reason_prefers_recorded_then_server_error() {
        let error = std::io::Error::other("synthetic listener failure");
        assert_eq!(stop_reason(Some("signal"), Some(&error)), "signal");
        assert_eq!(
            stop_reason(None, Some(&error)),
            "server_error error=synthetic listener failure"
        );
        assert_eq!(stop_reason(None, None), "unknown");
    }
}
