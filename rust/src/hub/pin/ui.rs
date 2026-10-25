//! Per-connection PIN authority; core membership remains owned by SessionCore.
use super::*;
use crate::proto::{core::*, time::Timestamp};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[cfg(test)]
#[path = "ui_tests.rs"]
mod tests;

pub(super) fn policy_key(secret: &str, pin_hash: &str) -> [u8; 32] {
    let mut hash = Sha256::new();
    for value in [secret, pin_hash] {
        hash.update((value.len() as u64).to_be_bytes());
        hash.update(value.as_bytes());
    }
    hash.finalize().into()
}
struct PinAuthorization {
    active: AtomicBool,
    expires: Option<Timestamp>,
    deadline: Option<tokio::time::Instant>,
}
impl UiAuthorization for PinAuthorization {
    fn is_valid(&self) -> bool {
        self.active.load(Ordering::Acquire)
            && self
                .expires
                .is_none_or(|expires| Timestamp::now() < expires)
            && self
                .deadline
                .is_none_or(|deadline| tokio::time::Instant::now() < deadline)
    }
}
pub(super) struct RemoteUi {
    binding: UiBinding,
    policy: [u8; 32],
    nonce: Option<String>,
    authorization: Arc<PinAuthorization>,
}
impl RemoteUi {
    fn valid(&self, state: &State, policy: [u8; 32], now: i64) -> bool {
        self.policy == policy
            && self.authorization.is_valid()
            && self.nonce.as_ref().is_none_or(|nonce| {
                state.sessions.get(nonce).is_some_and(|session| {
                    session.policy == policy
                        && session.expiry > now
                        && self
                            .authorization
                            .expires
                            .is_some_and(|expires| expires.unix_seconds() == session.expiry)
                })
            })
    }
}
/// Dropping a connection retires only its own authorization, never its wrapper.
pub(in crate::hub) struct RemotePinUi<'a> {
    owner: &'a AuthService,
    binding: UiBinding,
    authorization: Arc<PinAuthorization>,
}
impl RemotePinUi<'_> {
    pub(in crate::hub) fn is_valid(&self) -> bool {
        self.authorization.is_valid()
    }
    pub(in crate::hub) fn authorization(&self) -> Arc<dyn UiAuthorization> {
        self.authorization.clone()
    }
    pub(in crate::hub) async fn expiry(&self) {
        match self.authorization.deadline {
            Some(deadline) => {
                tokio::time::sleep_until(deadline).await;
                self.authorization.active.store(false, Ordering::Release);
            }
            None => std::future::pending().await,
        }
    }
}
impl Drop for RemotePinUi<'_> {
    fn drop(&mut self) {
        self.authorization.active.store(false, Ordering::Release);
        let mut state = self
            .owner
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state
            .remote_uis
            .get(&self.binding.connection)
            .is_some_and(|ui| ui.binding == self.binding)
        {
            state.remote_uis.remove(&self.binding.connection);
        }
    }
}
impl AuthService {
    pub(in crate::hub) fn bind_remote_ui(
        &self,
        request: &Request,
        store: &ConfigStore,
        binding: UiBinding,
        now: Timestamp,
    ) -> Result<RemotePinUi<'_>, SessionError> {
        let config = store.snapshot().map_err(|_| SessionError::Shutdown)?.config;
        let policy = policy_key(&config.auth_cookie_secret, &config.remote_pin_hash);
        let session = if config.remote_pin_hash.trim().is_empty() {
            None
        } else {
            Some(
                self.pin_session(request, &config, now.unix_seconds())
                    .ok_or(SessionError::AuthenticationExpired)?,
            )
        };
        let expires = session
            .as_ref()
            .map(|(_, expiry)| Timestamp::from_unix(*expiry, 0).expect("normalized seconds"));
        let authorization = Arc::new(PinAuthorization {
            active: AtomicBool::new(true),
            expires,
            deadline: expires.map(|expiry| {
                tokio::time::Instant::now() + expiry.duration_since(now).unwrap_or_default()
            }),
        });
        let entry = RemoteUi {
            binding,
            policy,
            nonce: session.map(|(nonce, _)| nonce),
            authorization: authorization.clone(),
        };
        let mut state = self.state.lock().map_err(|_| SessionError::Shutdown)?;
        let current = store.snapshot().map_err(|_| SessionError::Shutdown)?.config;
        if !entry.valid(
            &state,
            policy_key(&current.auth_cookie_secret, &current.remote_pin_hash),
            now.unix_seconds(),
        ) {
            return Err(SessionError::AuthenticationExpired);
        }
        if state.remote_uis.contains_key(&binding.connection) {
            return Err(SessionError::InvalidRequest(
                "remote UI already bound".into(),
            ));
        }
        state.remote_uis.insert(binding.connection, entry);
        Ok(RemotePinUi {
            owner: self,
            binding,
            authorization,
        })
    }
    pub(in crate::hub) fn revoke_invalid_uis(
        &self,
        store: &ConfigStore,
        core: &dyn SessionCore,
        now: i64,
    ) -> CoreEffects {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let policy = store.snapshot().ok().map(|snapshot| {
            policy_key(
                &snapshot.config.auth_cookie_secret,
                &snapshot.config.remote_pin_hash,
            )
        });
        let invalid = state
            .remote_uis
            .iter()
            .filter_map(|(id, entry)| {
                (self.state.is_poisoned()
                    || policy.is_none_or(|policy| !entry.valid(&state, policy, now)))
                .then_some(*id)
            })
            .collect::<Vec<_>>();
        let mut bindings = Vec::new();
        for id in invalid {
            let entry = state.remote_uis.remove(&id).expect("selected remote UI");
            // Close authority before waiting for the core or a socket writer.
            entry.authorization.active.store(false, Ordering::Release);
            bindings.push(entry.binding);
        }
        drop(state);
        let mut effects = CoreEffects::default();
        for binding in bindings {
            effects.0.extend(core.detach_ui(binding).0);
        }
        effects
    }
}
