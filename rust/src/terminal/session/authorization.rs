//! Accepted-work guard mirrors uiConn.authMu. Membership is removed atomically
//! before revocation, then the transport awaits accepted operations outside the
//! session lock before closing the socket or acknowledging revocation.
use super::*;
#[derive(Default)]
pub(super) struct UiWorkState {
    count: Mutex<usize>,
    changed: tokio::sync::Notify,
}
struct WorkGuard {
    state: Arc<UiWorkState>,
}
impl AcceptedUiWork for WorkGuard {}
impl Drop for WorkGuard {
    fn drop(&mut self) {
        let mut count = lock(&self.state.count);
        *count -= 1;
        let drained = *count == 0;
        drop(count);
        if drained {
            self.state.changed.notify_waiters();
        }
    }
}
impl SessionEngine {
    pub fn authorize_ui_work(
        &self,
        ui: UiBinding,
    ) -> Result<Box<dyn AcceptedUiWork>, SessionError> {
        let state = lock(&self.state);
        if !state.authorized(ui) {
            return Err(SessionError::AuthenticationExpired);
        }
        let work = state
            .uis
            .get(&ui.connection)
            .expect("authorized UI")
            .work
            .clone();
        {
            let mut count = lock(&work.count);
            *count = count.checked_add(1).ok_or(SessionError::Shutdown)?;
        }
        Ok(Box::new(WorkGuard { state: work }))
    }
    pub fn drain_ui_work<'a>(&'a self, ui: UiBinding) -> CoreFuture<'a, ()> {
        Box::pin(async move {
            let work = {
                let state = lock(&self.state);
                state
                    .retired_uis
                    .get(&(ui.connection, ui.auth_epoch))
                    .cloned()
                    .or_else(|| {
                        state
                            .uis
                            .get(&ui.connection)
                            .filter(|u| u.binding == ui)
                            .map(|u| u.work.clone())
                    })
            };
            let Some(work) = work else {
                return;
            };
            loop {
                let changed = work.changed.notified();
                tokio::pin!(changed);
                changed.as_mut().enable();
                if *lock(&work.count) == 0 {
                    break;
                }
                changed.await;
            }
            let mut state = lock(&self.state);
            if state
                .retired_uis
                .get(&(ui.connection, ui.auth_epoch))
                .is_some_and(|current| Arc::ptr_eq(current, &work))
            {
                state.retired_uis.remove(&(ui.connection, ui.auth_epoch));
            }
        })
    }
}
