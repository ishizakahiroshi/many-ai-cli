//! Source UI-disconnect idle timer. Epoch/check/stop admission stays in core;
//! this owner retains only one timer and its join handle.
use crate::{
    application::event_observer::EventWarning, config::ConfigStore, hub::task_owner::HubTaskHandle,
    process::Cancellation, proto::core::CoreEffectSink, terminal::session::SessionEngine,
};
use std::{sync::Arc, time::Duration};
pub(super) struct IdleGuard {
    cancel: Cancellation,
    join: Option<tokio::task::JoinHandle<()>>,
}
impl IdleGuard {
    pub fn start(
        core: Arc<SessionEngine>,
        config: Arc<ConfigStore>,
        effects: Arc<dyn CoreEffectSink>,
        tasks: HubTaskHandle,
        warning: EventWarning,
    ) -> Self {
        let cancel = Cancellation::default();
        let stop = cancel.clone();
        let mut presence = core.subscribe_ui_presence();
        let join = tokio::spawn(async move {
            loop {
                let ui = *presence.borrow_and_update();
                let minutes = config
                    .snapshot()
                    .map(|snapshot| snapshot.config.hub.idle_timeout_min)
                    .unwrap_or(0);
                let duration = u64::try_from(minutes)
                    .ok()
                    .filter(|minutes| *minutes > 0)
                    .and_then(|minutes| minutes.checked_mul(60))
                    .map(Duration::from_secs);
                let deadline = ui
                    .disconnected_at
                    .zip(duration)
                    .and_then(|(at, duration)| at.checked_add(duration));
                tokio::select! {
                    biased;
                    _ = stop.cancelled() => break,
                    changed = presence.changed() => if changed.is_err() { break; },
                    _ = async { match deadline { Some(at) => tokio::time::sleep_until(at).await, None => std::future::pending().await } } => {
                        let permit = match tasks.effect_permit() { Ok(permit) => permit, Err(error) => { warning("idle timeout ownership", &error); break; } };
                        let actions = core.expire_ui_idle(ui.generation);
                        let effects = effects.clone();
                        let warning = warning.clone();
                        drop(permit.start(async move {
                            if let Err(failure) = effects.apply(actions).await { warning("idle timeout stop effects", &failure.error); }
                        }));
                    }
                }
            }
        });
        Self {
            cancel,
            join: Some(join),
        }
    }
    pub async fn stop_and_join(mut self) {
        self.cancel.cancel();
        if let Some(join) = self.join.take() {
            let _ = join.await;
        }
    }
}
impl Drop for IdleGuard {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(join) = self.join.take() {
            join.abort();
        }
    }
}
