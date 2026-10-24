//! Subscription lifecycle adapters over the existing core, effects and worktree owners.
use crate::{
    application::{
        event_observer::{ApplicationEventObserver, EventWarning},
        ordinary_spawn::OrdinarySpawnService,
        subscriptions::probe::SubscriptionProbeHooks,
    },
    hub::{sockets::OrderedEventObserver, task_owner::HubTaskHandle},
    proto::{core::*, time::Timestamp},
    terminal::session::SessionEngine,
};
use std::{
    io,
    sync::{Arc, OnceLock, Weak},
};
pub(super) struct ProbeHooks {
    pub core: Weak<SessionEngine>,
    pub effects: Weak<dyn CoreEffectSink>,
    pub ordinary: Weak<OrdinarySpawnService>,
    pub trial: bool,
}
impl ProbeHooks {
    fn dismiss_observed(
        core: &SessionEngine,
        details: &SessionDetails,
    ) -> Result<CoreEffects, SessionError> {
        // Observation and mutation take separate core locks. A wrapper may
        // reconnect between them, so validate the full binding at mutation.
        core.dismiss_bound(details.binding, Timestamp::now())
    }
}
impl SubscriptionProbeHooks for ProbeHooks {
    fn dismiss<'a>(&'a self, binding: SessionBinding) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            let core = self.core.upgrade().ok_or(SessionError::Shutdown)?;
            let Some(details) = core.details(binding.session) else {
                return Ok(());
            };
            if details.binding != binding {
                return Err(SessionError::StaleBinding);
            }
            let actions = Self::dismiss_observed(&core, &details)?;
            let effects = self.effects.upgrade().ok_or(SessionError::Shutdown)?;
            let delivered = effects.apply(actions).await;
            self.ordinary
                .upgrade()
                .ok_or(SessionError::Shutdown)?
                .dismissed(&details.snapshot)
                .await;
            delivered.map_err(|failure| failure.error)
        })
    }
    fn wrapper_pid(&self, binding: SessionBinding) -> Option<u32> {
        self.core.upgrade()?.wrapper_pid(binding).ok().flatten()
    }
    fn recover_process(&self, _pid: u32) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "PID-only probe recovery is not permitted",
        ))
    }
    fn recover_identified_process(&self, pid: u32, identity: &str) -> io::Result<()> {
        if self.trial {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "trial refuses native process recovery",
            ));
        }
        crate::process::wrapper_startup::recovery_identity::terminate(pid, identity)
    }
}

pub(super) struct LoginObserver {
    inner: Arc<ApplicationEventObserver>,
    hooks: OnceLock<Weak<ProbeHooks>>,
    tasks: HubTaskHandle,
    warning: EventWarning,
}
impl LoginObserver {
    pub fn new(
        inner: Arc<ApplicationEventObserver>,
        tasks: HubTaskHandle,
        warning: EventWarning,
    ) -> Self {
        Self {
            inner,
            hooks: OnceLock::new(),
            tasks,
            warning,
        }
    }
    pub fn bind(&self, hooks: Weak<ProbeHooks>) -> io::Result<()> {
        self.hooks
            .set(hooks)
            .map_err(|_| io::Error::other("subscription login lifecycle already bound"))
    }
}
impl OrderedEventObserver for LoginObserver {
    fn approval_opened<'a>(
        &'a self,
        session: LiveSessionId,
        record: &'a ImmutableApprovalRecord,
    ) -> CoreFuture<'a, Result<bool, SessionError>> {
        self.inner.approval_opened(session, record)
    }
    fn observe<'a>(&'a self, event: &'a CoreEvent) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            self.inner.observe(event).await?;
            if let CoreEvent::Ended { binding, .. } = event {
                let hooks = self
                    .hooks
                    .get()
                    .and_then(Weak::upgrade)
                    .ok_or(SessionError::Shutdown)?;
                let core = hooks.core.upgrade().ok_or(SessionError::Shutdown)?;
                if core.is_subscription_login(*binding)? {
                    let permit = self.tasks.effect_permit()?;
                    let binding = *binding;
                    let warning = self.warning.clone();
                    drop(permit.start(async move {
                        if let Err(error) = hooks.dismiss(binding).await {
                            warning("subscription login dismiss", &error);
                        }
                    }));
                }
            }
            Ok(())
        })
    }
}

#[cfg(test)]
#[path = "subscriptions/tests.rs"]
mod tests;
