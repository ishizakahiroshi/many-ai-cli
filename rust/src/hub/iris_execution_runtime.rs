//! Internal, deliberately incomplete runtime bridge for Iris-owned attempts.
//!
//! This module does not spawn, authorize, expose HTTP routes, or turn
//! session-end/Done/socket-close observations into execution or stop evidence.
//! A composition owner may call `bind_attempt` ONLY after its mandatory durable
//! execution journal and grant checks have committed and an actual core-created
//! registration proof has established this exact SessionBinding. A session ID,
//! client_request_id, label, PID, or wrapper-provided metadata is not that proof.
//!
//! These bindings and stop receipts are process-local. On restart they are lost;
//! the composition owner must retain UNKNOWN in its durable journal and must not
//! rebuild authority by scanning labels or silently relaunch a provider. This
//! module does not implement rebinding after reattach: recovery requires a future
//! independently authenticated, journaled binding transition.
//!
//! `stop_bound` issues one exact CancelSession effect, not kill-all. Effect
//! success says neither that a wrapper received the frame nor that its provider,
//! descendants or remote jobs stopped. A cancelled/failed effect may have done
//! partial I/O, so the reserved request is never automatically retried.
//! Optional OwnerStore composition commits the exact owner stop ID before effect
//! admission. It persists intent only, never process cessation or task success.
//!
//! Still missing before real execution can be advertised: owner-written durable
//! provider/PTY receipts bound to attempt/fence/instance and stop_request_id;
//! checked termination and bounded Job/group emptiness observation; recovery of
//! those receipts independently of the Hub socket; and verified task/artifact
//! results (process exit zero and Done text are insufficient). Remote-job stop
//! and effects=none are not established by terminating a local process. The
//! capabilities here therefore explicitly keep `execution_ready` false.

use super::iris_execution_owner::{OwnerStore, OwnerTicket};
use crate::{
    proto::{
        core::{CoreEffectSink, LiveSessionId, SessionBinding, SessionError, StopReason},
        time::Timestamp,
    },
    terminal::session::SessionEngine,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fmt,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const MAX_BINDINGS: usize = 4096;
const MAX_STOP_REQUESTS: usize = 4096;
const STOP_EFFECT_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeError {
    InvalidIdentity,
    UnknownAttempt,
    StaleAttempt,
    OwnershipConflict,
    SessionAlreadyOwned,
    StaleBinding,
    RequestConflict,
    StopAlreadyRequested,
    Capacity,
    StateUnavailable,
}
impl BridgeError {
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidIdentity => "INVALID_IDENTITY",
            Self::UnknownAttempt => "UNKNOWN_ATTEMPT",
            Self::StaleAttempt => "STALE_ATTEMPT",
            Self::OwnershipConflict => "OWNERSHIP_CONFLICT",
            Self::SessionAlreadyOwned => "SESSION_ALREADY_OWNED",
            Self::StaleBinding => "STALE_BINDING",
            Self::RequestConflict => "REQUEST_CONFLICT",
            Self::StopAlreadyRequested => "STOP_ALREADY_REQUESTED",
            Self::Capacity => "BRIDGE_CAPACITY",
            Self::StateUnavailable => "BRIDGE_STATE_UNAVAILABLE",
        }
    }
}
impl fmt::Display for BridgeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}
impl std::error::Error for BridgeError {}

#[derive(Clone, PartialEq, Eq)]
struct Ownership {
    fence: u64,
    instance: String,
    binding: SessionBinding,
}
struct Attempt {
    owner: Ownership,
    binding_receipt: Value,
    stop_request: Option<String>,
    durable_owner: Option<OwnerTicket>,
}
struct StopRequest {
    attempt_id: String,
    owner: Ownership,
    receipt: Value,
    effect_observation: &'static str,
}
#[derive(Default)]
struct State {
    attempts: BTreeMap<String, Attempt>,
    sessions: BTreeMap<LiveSessionId, String>,
    stops: BTreeMap<String, StopRequest>,
}

pub struct SessionBridge {
    core: Arc<SessionEngine>,
    effects: Arc<dyn CoreEffectSink>,
    state: Mutex<State>,
    owner_store: Option<Arc<Mutex<OwnerStore>>>,
}
fn identity(value: &str) -> Result<(), BridgeError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Err(BridgeError::InvalidIdentity);
    }
    Ok(())
}
fn attempt_identity(attempt_id: &str, fence: u64, instance: &str) -> Result<(), BridgeError> {
    identity(attempt_id)?;
    identity(instance)?;
    if fence == 0 || fence > i64::MAX as u64 {
        return Err(BridgeError::InvalidIdentity);
    }
    Ok(())
}
fn observed_at() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn core_error(error: SessionError) -> BridgeError {
    match error {
        SessionError::NotFound(_) | SessionError::StaleBinding => BridgeError::StaleBinding,
        _ => BridgeError::StateUnavailable,
    }
}
fn matches_owner(owner: &Ownership, fence: u64, instance: &str) -> Result<(), BridgeError> {
    if owner.fence != fence || owner.instance != instance {
        Err(BridgeError::StaleAttempt)
    } else {
        Ok(())
    }
}
fn validate_durable_owner(
    record: &Value,
    ticket: &OwnerTicket,
    binding: SessionBinding,
) -> Result<(), BridgeError> {
    if record["ticket"] != json!(ticket)
        || record["claimed_session"] != binding.session.0
        || !record["root_pid"]
            .as_u64()
            .is_some_and(|pid| pid > 0 && pid <= u32::MAX as u64)
        || record["root_started_at_ms"].as_u64().is_none()
        || record["local_tree_ceased"] == true
    {
        return Err(BridgeError::OwnershipConflict);
    }
    Ok(())
}
fn persist_owner_stop(
    store: &Mutex<OwnerStore>,
    ticket: &OwnerTicket,
    binding: SessionBinding,
    stop_id: &str,
) -> Result<Value, BridgeError> {
    let mut store = store.lock().map_err(|_| BridgeError::StateUnavailable)?;
    let record = store
        .query(&ticket.binding)
        .map_err(|_| BridgeError::StateUnavailable)?;
    validate_durable_owner(&record, ticket, binding)?;
    if record["stop_request_id"].is_string() {
        return Err(BridgeError::StopAlreadyRequested);
    }
    let seq = record["sequence"]
        .as_u64()
        .and_then(|n| n.checked_add(1))
        .ok_or(BridgeError::StateUnavailable)?;
    let at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as u64;
    store
        .request_stop(ticket, seq, stop_id, at)
        .map_err(|_| BridgeError::StateUnavailable)
}
fn base_receipt(attempt_id: &str, owner: &Ownership) -> Value {
    json!({
        "attempt_id":attempt_id,
        "fence":owner.fence,
        "executor_instance_id":owner.instance,
        "session_id":owner.binding.session.0,
        "session_incarnation":owner.binding.incarnation.0,
        "wrapper_connection_id":owner.binding.wrapper.0,
        "durable":false,
        "owned_execution_ceased":false,
        "result_verified":false,
        "outcome":"unknown"
    })
}
impl SessionBridge {
    fn check_core_owner(
        &self,
        ticket: &OwnerTicket,
        binding: SessionBinding,
    ) -> Result<(), BridgeError> {
        let details = self
            .core
            .details(binding.session)
            .ok_or(BridgeError::StaleBinding)?;
        if details.binding != binding {
            return Err(BridgeError::StaleBinding);
        }
        let profile = if details.snapshot.subscription_profile_id.is_empty() {
            "default"
        } else {
            details.snapshot.subscription_profile_id.as_str()
        };
        if details.snapshot.provider != ticket.binding.provider
            || profile != ticket.binding.profile_id
        {
            return Err(BridgeError::OwnershipConflict);
        }
        Ok(())
    }
    pub fn new(core: Arc<SessionEngine>, effects: Arc<dyn CoreEffectSink>) -> Self {
        Self {
            core,
            effects,
            state: Mutex::new(State::default()),
            owner_store: None,
        }
    }

    /// Optional trusted owner-controller lane. Existing `new` keeps its original
    /// process-local behavior. This does not make production execution ready.
    pub fn new_with_owner_store(
        core: Arc<SessionEngine>,
        effects: Arc<dyn CoreEffectSink>,
        owner_store: Arc<Mutex<OwnerStore>>,
    ) -> Self {
        Self {
            core,
            effects,
            state: Mutex::new(State::default()),
            owner_store: Some(owner_store),
        }
    }

    /// Only the trusted composition may call this after journal/grant admission
    /// and an actual core registration proof, never from caller JSON. Provider
    /// root PID and wrapper PID are different identities; do not equate them.
    pub fn bind_owner_attempt(
        &self,
        ticket: &OwnerTicket,
        binding: SessionBinding,
    ) -> Result<Value, BridgeError> {
        self.bind_internal(
            &ticket.binding.attempt_id,
            ticket.binding.fence,
            &ticket.binding.executor_instance_id,
            binding,
            Some(ticket.clone()),
        )
    }

    /// This building block's capabilities cannot enable production dispatch.
    /// A future composition must intersect journal, runtime and result evidence
    /// capabilities rather than substituting a constant successful response.
    pub fn capabilities(&self) -> Value {
        json!({
            "contract_version":"iris-session-bridge/1",
            "execution_ready":false,
            "spawn_available":false,
            "durable_state":false,
            "durable_start_receipt":false,
            "query_by_attempt":true,
            "query_survives_restart":false,
            "exact_session_stop_request":true,
            "fenced_stop_proof":false,
            "result_sha256":false,
            "verified_task_result":false,
            "automatic_handoff":false
        })
    }

    /// Trusted composition only: mandatory journal+grant+proof gate is described
    /// at module level. This method records no permission and launches nothing.
    pub fn bind_attempt(
        &self,
        attempt_id: &str,
        fence: u64,
        executor_instance_id: &str,
        binding: SessionBinding,
    ) -> Result<Value, BridgeError> {
        self.bind_internal(attempt_id, fence, executor_instance_id, binding, None)
    }
    fn bind_internal(
        &self,
        attempt_id: &str,
        fence: u64,
        executor_instance_id: &str,
        binding: SessionBinding,
        ticket: Option<OwnerTicket>,
    ) -> Result<Value, BridgeError> {
        attempt_identity(attempt_id, fence, executor_instance_id)?;
        let proposed = Ownership {
            fence,
            instance: executor_instance_id.into(),
            binding,
        };
        let mut state = self
            .state
            .lock()
            .map_err(|_| BridgeError::StateUnavailable)?;
        if let Some(ticket) = ticket.as_ref() {
            if !self.core.is_current(binding) {
                return Err(BridgeError::StaleBinding);
            }
            self.check_core_owner(ticket, binding)?;
            let store = self
                .owner_store
                .as_ref()
                .ok_or(BridgeError::StateUnavailable)?
                .lock()
                .map_err(|_| BridgeError::StateUnavailable)?;
            let record = store
                .query(&ticket.binding)
                .map_err(|_| BridgeError::OwnershipConflict)?;
            validate_durable_owner(&record, ticket, binding)?;
        }
        if let Some(existing) = state.attempts.get_mut(attempt_id) {
            if existing.owner == proposed {
                if let Some(ticket) = ticket {
                    if existing
                        .durable_owner
                        .as_ref()
                        .is_some_and(|old| old != &ticket)
                    {
                        return Err(BridgeError::OwnershipConflict);
                    }
                    existing.durable_owner = Some(ticket);
                }
                // An idempotent replay retains its original ownership, even if
                // the session has since gone. Query will report that uncertainty.
                return Ok(existing.binding_receipt.clone());
            }
            return Err(BridgeError::OwnershipConflict);
        }
        if state.sessions.contains_key(&binding.session) {
            return Err(BridgeError::SessionAlreadyOwned);
        }
        if !self.core.is_current(binding) {
            return Err(BridgeError::StaleBinding);
        }
        if state.attempts.len() >= MAX_BINDINGS {
            return Err(BridgeError::Capacity);
        }
        let mut receipt = base_receipt(attempt_id, &proposed);
        receipt["operation"] = "bind_registered_session".into();
        receipt["binding_recorded_at"] = observed_at().into();
        receipt["execution_state"] = "registered_observation_only".into();
        state.sessions.insert(binding.session, attempt_id.into());
        state.attempts.insert(
            attempt_id.into(),
            Attempt {
                owner: proposed,
                binding_receipt: receipt.clone(),
                stop_request: None,
                durable_owner: ticket,
            },
        );
        Ok(receipt)
    }

    pub fn query(
        &self,
        attempt_id: &str,
        fence: u64,
        executor_instance_id: &str,
    ) -> Result<Value, BridgeError> {
        attempt_identity(attempt_id, fence, executor_instance_id)?;
        let (owner, stop) = {
            let state = self
                .state
                .lock()
                .map_err(|_| BridgeError::StateUnavailable)?;
            let attempt = state
                .attempts
                .get(attempt_id)
                .ok_or(BridgeError::UnknownAttempt)?;
            matches_owner(&attempt.owner, fence, executor_instance_id)?;
            let stop = attempt
                .stop_request
                .as_ref()
                .and_then(|request| state.stops.get(request))
                .map(|entry| (entry.receipt.clone(), entry.effect_observation));
            (attempt.owner.clone(), stop)
        };
        let mut response = base_receipt(attempt_id, &owner);
        response["operation"] = "query".into();
        response["observed_at"] = observed_at().into();
        response["execution_state"] = "unknown".into();
        response["observation"] = match self.core.details(owner.binding.session) {
            None => json!({"kind":"session_missing","proof":false}),
            Some(details) if details.binding != owner.binding => {
                json!({"kind":"binding_replaced","proof":false})
            }
            Some(details) if !details.connected => {
                json!({"kind":"wrapper_disconnected","proof":false})
            }
            Some(details) if ["completed", "error"].contains(&details.snapshot.state.as_str()) => {
                json!({"kind":"wrapper_reported_terminal","proof":false})
            }
            Some(_) => {
                response["execution_state"] = "registered_observation_only".into();
                json!({"kind":"registered_connection","proof":false})
            }
        };
        if let Some((receipt, effect_observation)) = stop {
            response["execution_state"] = "stop_unconfirmed".into();
            response["stop_receipt"] = receipt;
            response["stop_effect_observation"] = effect_observation.into();
        }
        Ok(response)
    }

    /// Caller must first commit the exact stop request and current grant to its
    /// durable journal. The returned immutable receipt never claims delivery,
    /// completed execution, no effects, or confirmed termination.
    pub async fn stop_bound(
        &self,
        attempt_id: &str,
        fence: u64,
        executor_instance_id: &str,
        stop_request_id: &str,
    ) -> Result<Value, BridgeError> {
        attempt_identity(attempt_id, fence, executor_instance_id)?;
        identity(stop_request_id)?;
        let (effects, receipt) = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| BridgeError::StateUnavailable)?;
            if let Some(previous) = state.stops.get(stop_request_id) {
                if previous.attempt_id != attempt_id
                    || previous.owner.fence != fence
                    || previous.owner.instance != executor_instance_id
                {
                    return Err(BridgeError::RequestConflict);
                }
                return Ok(previous.receipt.clone());
            }
            let attempt = state
                .attempts
                .get(attempt_id)
                .ok_or(BridgeError::UnknownAttempt)?;
            matches_owner(&attempt.owner, fence, executor_instance_id)?;
            if attempt.stop_request.is_some() {
                return Err(BridgeError::StopAlreadyRequested);
            }
            let owner = attempt.owner.clone();
            if !self.core.is_current(owner.binding) {
                return Err(BridgeError::StaleBinding);
            }
            let effects = self
                .core
                .stop_bound(owner.binding, StopReason::User, Timestamp::now())
                .map_err(core_error)?;
            if state.stops.len() >= MAX_STOP_REQUESTS {
                return Err(BridgeError::Capacity);
            }
            // The exact durable owner stop identity is committed synchronously
            // before any effect is polled. The process-local reservation below
            // also retains a failed gate: repair does not authorize blind retry.
            let durable_gate = if let Some(store) = self.owner_store.as_ref() {
                Some(match attempt.durable_owner.as_ref() {
                    Some(ticket) => self.check_core_owner(ticket, owner.binding).and_then(|_| {
                        persist_owner_stop(store, ticket, owner.binding, stop_request_id)
                    }),
                    None => Err(BridgeError::OwnershipConflict),
                })
            } else {
                None
            };
            let allowed = durable_gate.as_ref().is_none_or(|result| result.is_ok());
            let mut receipt = base_receipt(attempt_id, &owner);
            receipt["operation"] = "request_session_stop".into();
            receipt["stop_request_id"] = stop_request_id.into();
            receipt["recorded_at"] = observed_at().into();
            receipt["execution_state"] = "stop_unconfirmed".into();
            receipt["reason"] = "owner_written_stop_receipt_unavailable".into();
            if let Some(result) = &durable_gate {
                receipt["owner_stop_request_durable"] = json!(result.is_ok());
                receipt["reason"] = json!(if result.is_ok() {
                    "durable_owner_stop_intent_only"
                } else {
                    "durable_owner_gate_failed_no_effect"
                });
                if let Ok(owner_receipt) = result {
                    receipt["owner_stop_receipt"] = owner_receipt.clone();
                }
            }
            state.stops.insert(
                stop_request_id.into(),
                StopRequest {
                    attempt_id: attempt_id.into(),
                    owner,
                    receipt: receipt.clone(),
                    effect_observation: if allowed {
                        "reserved_delivery_unknown"
                    } else {
                        "owner_gate_failed_no_effect"
                    },
                },
            );
            state
                .attempts
                .get_mut(attempt_id)
                .expect("validated attempt retained under lock")
                .stop_request = Some(stop_request_id.into());
            (if allowed { Some(effects) } else { None }, receipt)
        };
        let Some(effects) = effects else {
            return Ok(receipt);
        };
        // No bridge/core mutex crosses async I/O. Reservation is already visible
        // if this future is cancelled while an effect may be partially applied.
        let effect_observation =
            match tokio::time::timeout(STOP_EFFECT_TIMEOUT, self.effects.apply(effects)).await {
                Ok(Ok(())) => "effect_batch_returned_without_stop_proof",
                Ok(Err(_)) => "effect_batch_failed_delivery_unknown",
                Err(_) => "effect_batch_timed_out_delivery_unknown",
            };
        if let Ok(mut state) = self.state.lock()
            && let Some(entry) = state.stops.get_mut(stop_request_id)
        {
            entry.effect_observation = effect_observation;
        }
        Ok(receipt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::RuntimePaths,
        proto::{self, core::*},
        terminal::{
            events::CoreEventBus,
            journal::{JournalOptions, SessionJournal},
            session::EngineOptions,
        },
    };
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::sync::Notify;

    struct NoNative;
    impl WrapperTransport for NoNative {
        fn send<'a>(
            &'a self,
            _: SessionBinding,
            _: proto::Message,
        ) -> CoreFuture<'a, Result<(), SessionError>> {
            Box::pin(async { panic!("synthetic bridge tests must not reach a wrapper") })
        }
    }
    impl WrappedSessionSpawner for NoNative {
        fn spawn_and_wait<'a>(
            &'a self,
            _: WrappedSpawnSpec,
            _: Duration,
            _: &'a HttpWaitCancellation,
        ) -> CoreFuture<'a, SpawnWaitOutcome> {
            Box::pin(async { panic!("synthetic bridge tests must not spawn a provider") })
        }
    }
    #[derive(Default)]
    struct Effects {
        cancelled: Mutex<Vec<SessionBinding>>,
        fail: AtomicBool,
        pause: AtomicBool,
        entered: Notify,
        release: Notify,
        owner_expectation: Mutex<
            Option<(
                std::path::PathBuf,
                crate::hub::iris_execution_journal::Binding,
                String,
            )>,
        >,
        owner_gate_seen: AtomicBool,
    }
    impl CoreEffectSink for Effects {
        fn apply<'a>(
            &'a self,
            effects: CoreEffects,
        ) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
            Box::pin(async move {
                if let Some((path, binding, stop_id)) =
                    self.owner_expectation.lock().unwrap().as_ref()
                {
                    let store = OwnerStore::open(path)
                        .expect("owner store must be readable before effects");
                    let record = store
                        .query(binding)
                        .expect("full binding must already be durable");
                    assert_eq!(record["stop_request_id"], stop_id.as_str());
                    assert_eq!(record["sequence"], 1);
                    self.owner_gate_seen.store(true, Ordering::SeqCst);
                }
                for effect in effects.0 {
                    let CoreEffect::CancelSession { binding, reason } = effect else {
                        panic!("bridge dispatched an unrelated effect")
                    };
                    assert_eq!(reason, StopReason::User);
                    self.cancelled.lock().unwrap().push(binding);
                }
                self.entered.notify_one();
                if self.pause.load(Ordering::SeqCst) {
                    self.release.notified().await;
                }
                if self.fail.load(Ordering::SeqCst) {
                    return Err(CoreEffectFailure {
                        index: 0,
                        error: SessionError::Transport("must-not-copy-private-detail".into()),
                    });
                }
                Ok(())
            })
        }
    }
    struct Fixture {
        core: Arc<SessionEngine>,
        bridge: Arc<SessionBridge>,
        effects: Arc<Effects>,
        root: tempfile::TempDir,
    }
    fn fixture() -> Fixture {
        let root = tempfile::tempdir().unwrap();
        let runtime = root.path().join("runtime");
        std::fs::create_dir(&runtime).unwrap();
        let paths = RuntimePaths::trial(&runtime, 49467, &root.path().join("installed")).unwrap();
        let effects = Arc::new(Effects::default());
        let core = Arc::new(SessionEngine::new(
            EngineOptions::default(),
            Arc::new(SessionJournal::new(paths, None, JournalOptions::default())),
            Arc::new(NoNative),
            effects.clone(),
            Arc::new(NoNative),
            CoreEventBus::new(64).unwrap(),
        ));
        let bridge = Arc::new(SessionBridge::new(core.clone(), effects.clone()));
        Fixture {
            core,
            bridge,
            effects,
            root,
        }
    }
    async fn register(fixture: &Fixture, connection: u64) -> SessionBinding {
        fixture
            .core
            .register(
                RegisterRequest {
                    message: proto::Message {
                        provider: "synthetic-provider".into(),
                        cwd: fixture.root.path().to_string_lossy().into_owned(),
                        pid: 3000 + connection as i64,
                        usage_probe: true,
                        cols: 120,
                        rows: 30,
                        ..Default::default()
                    },
                    spawn_proof: None,
                },
                WrapperConnectionId(connection),
                Timestamp::now(),
            )
            .await
            .unwrap()
            .binding
    }
    async fn replace(fixture: &Fixture, binding: SessionBinding) -> SessionBinding {
        fixture
            .core
            .reattach(
                ReattachRequest {
                    message: proto::Message {
                        session_id: binding.session.0,
                        provider: "synthetic-provider".into(),
                        cwd: fixture.root.path().to_string_lossy().into_owned(),
                        pid: 3000 + binding.wrapper.0 as i64,
                        started_at: fixture.core.snapshot(binding.session).unwrap().started_at,
                        usage_probe: true,
                        cols: 120,
                        rows: 30,
                        ..Default::default()
                    },
                    restored_metadata: None,
                },
                WrapperConnectionId(99),
                Timestamp::now(),
            )
            .await
            .unwrap()
            .binding
    }
    #[tokio::test]
    async fn immutable_binding_cannot_be_reassigned_or_aliased() {
        let f = fixture();
        let one = register(&f, 1).await;
        let two = register(&f, 2).await;
        let receipt = f
            .bridge
            .bind_attempt("attempt-a", 1, "instance-a", one)
            .unwrap();
        assert_eq!(
            receipt,
            f.bridge
                .bind_attempt("attempt-a", 1, "instance-a", one)
                .unwrap()
        );
        assert_eq!(
            f.bridge
                .bind_attempt("attempt-a", 2, "instance-a", one)
                .unwrap_err(),
            BridgeError::OwnershipConflict
        );
        assert_eq!(
            f.bridge
                .bind_attempt("attempt-a", 1, "instance-a", two)
                .unwrap_err(),
            BridgeError::OwnershipConflict
        );
        assert_eq!(
            f.bridge
                .bind_attempt("attempt-b", 1, "instance-a", one)
                .unwrap_err(),
            BridgeError::SessionAlreadyOwned
        );
        assert!(
            f.bridge
                .bind_attempt("attempt-b", 1, "instance-a", two)
                .is_ok()
        );
        assert!(f.effects.cancelled.lock().unwrap().is_empty());
    }
    #[tokio::test]
    async fn exact_stop_is_issued_once_and_never_proves_exit() {
        let f = fixture();
        let one = register(&f, 1).await;
        let other = register(&f, 2).await;
        f.bridge
            .bind_attempt("attempt-a", 1, "instance-a", one)
            .unwrap();
        let receipt = f
            .bridge
            .stop_bound("attempt-a", 1, "instance-a", "stop-a")
            .await
            .unwrap();
        assert_eq!(receipt["owned_execution_ceased"], false);
        assert_eq!(receipt["execution_state"], "stop_unconfirmed");
        assert_eq!(
            receipt,
            f.bridge
                .stop_bound("attempt-a", 1, "instance-a", "stop-a")
                .await
                .unwrap()
        );
        assert_eq!(*f.effects.cancelled.lock().unwrap(), vec![one]);
        assert!(f.core.is_current(other));
        assert_eq!(
            f.bridge
                .stop_bound("attempt-a", 1, "instance-a", "stop-b")
                .await
                .unwrap_err(),
            BridgeError::StopAlreadyRequested
        );
        let observed = f.bridge.query("attempt-a", 1, "instance-a").unwrap();
        assert_eq!(
            observed["stop_effect_observation"],
            "effect_batch_returned_without_stop_proof"
        );
        assert_eq!(observed["outcome"], "unknown");
    }
    #[tokio::test]
    async fn stale_request_or_attempt_never_dispatches_and_request_reuse_conflicts() {
        let f = fixture();
        let one = register(&f, 1).await;
        let two = register(&f, 2).await;
        f.bridge
            .bind_attempt("attempt-a", 1, "instance-a", one)
            .unwrap();
        f.bridge
            .bind_attempt("attempt-b", 1, "instance-a", two)
            .unwrap();
        assert_eq!(
            f.bridge.query("attempt-a", 2, "instance-a").unwrap_err(),
            BridgeError::StaleAttempt
        );
        assert_eq!(
            f.bridge
                .stop_bound("attempt-a", 1, "old-instance", "stop-a")
                .await
                .unwrap_err(),
            BridgeError::StaleAttempt
        );
        assert_eq!(
            f.bridge
                .stop_bound("missing", 1, "instance-a", "stop-a")
                .await
                .unwrap_err(),
            BridgeError::UnknownAttempt
        );
        assert!(f.effects.cancelled.lock().unwrap().is_empty());
        f.bridge
            .stop_bound("attempt-a", 1, "instance-a", "stop-a")
            .await
            .unwrap();
        assert_eq!(
            f.bridge
                .stop_bound("attempt-b", 1, "instance-a", "stop-a")
                .await
                .unwrap_err(),
            BridgeError::RequestConflict
        );
        assert_eq!(*f.effects.cancelled.lock().unwrap(), vec![one]);
    }
    #[tokio::test]
    async fn replaced_wrapper_is_unknown_and_stale_stop_cannot_target_replacement() {
        let f = fixture();
        let old = register(&f, 1).await;
        f.bridge
            .bind_attempt("attempt-a", 1, "instance-a", old)
            .unwrap();
        let current = replace(&f, old).await;
        assert_ne!(old, current);
        let query = f.bridge.query("attempt-a", 1, "instance-a").unwrap();
        assert_eq!(query["execution_state"], "unknown");
        assert_eq!(query["observation"]["kind"], "binding_replaced");
        assert_eq!(
            f.bridge
                .stop_bound("attempt-a", 1, "instance-a", "stop-a")
                .await
                .unwrap_err(),
            BridgeError::StaleBinding
        );
        assert_eq!(
            f.bridge
                .bind_attempt("attempt-a", 1, "instance-a", current)
                .unwrap_err(),
            BridgeError::OwnershipConflict
        );
        assert!(f.effects.cancelled.lock().unwrap().is_empty());
        assert!(f.core.is_current(current));
    }
    #[tokio::test]
    async fn terminal_disconnect_and_missing_are_observations_not_proof() {
        let f = fixture();
        let binding = register(&f, 1).await;
        f.bridge
            .bind_attempt("attempt-a", 1, "instance-a", binding)
            .unwrap();
        f.core
            .observe_end(
                binding,
                SessionEnd {
                    declared_state: "completed".into(),
                    exit_code: 0,
                    reason: "synthetic".into(),
                },
                Timestamp::now(),
            )
            .unwrap();
        let query = f.bridge.query("attempt-a", 1, "instance-a").unwrap();
        assert_eq!(query["execution_state"], "unknown");
        assert_eq!(query["observation"]["kind"], "wrapper_reported_terminal");
        assert_eq!(query["result_verified"], false);
        f.core.disconnected(binding, Timestamp::now()).unwrap();
        assert_eq!(
            f.bridge.query("attempt-a", 1, "instance-a").unwrap()["observation"]["kind"],
            "wrapper_disconnected"
        );
        f.core.dismiss(binding.session, Timestamp::now()).unwrap();
        let missing = f.bridge.query("attempt-a", 1, "instance-a").unwrap();
        assert_eq!(missing["observation"]["kind"], "session_missing");
        assert_eq!(missing["owned_execution_ceased"], false);
    }
    #[tokio::test]
    async fn partially_failed_effect_keeps_same_receipt_without_secret_error_copy() {
        let f = fixture();
        let binding = register(&f, 1).await;
        f.bridge
            .bind_attempt("attempt-a", 1, "instance-a", binding)
            .unwrap();
        f.effects.fail.store(true, Ordering::SeqCst);
        let receipt = f
            .bridge
            .stop_bound("attempt-a", 1, "instance-a", "stop-a")
            .await
            .unwrap();
        let query = f.bridge.query("attempt-a", 1, "instance-a").unwrap();
        assert_eq!(
            query["stop_effect_observation"],
            "effect_batch_failed_delivery_unknown"
        );
        assert!(!query.to_string().contains("must-not-copy-private-detail"));
        assert_eq!(
            receipt,
            f.bridge
                .stop_bound("attempt-a", 1, "instance-a", "stop-a")
                .await
                .unwrap()
        );
        assert_eq!(f.effects.cancelled.lock().unwrap().len(), 1);
    }
    #[tokio::test]
    async fn concurrent_duplicate_and_cancelled_waiter_do_not_repeat_stop_effects() {
        let f = fixture();
        let binding = register(&f, 1).await;
        f.bridge
            .bind_attempt("attempt-a", 1, "instance-a", binding)
            .unwrap();
        f.effects.pause.store(true, Ordering::SeqCst);
        let bridge = f.bridge.clone();
        let first = tokio::spawn(async move {
            bridge
                .stop_bound("attempt-a", 1, "instance-a", "stop-a")
                .await
        });
        f.effects.entered.notified().await;
        let duplicate = f
            .bridge
            .stop_bound("attempt-a", 1, "instance-a", "stop-a")
            .await
            .unwrap();
        first.abort();
        let _ = first.await;
        assert_eq!(
            duplicate,
            f.bridge
                .stop_bound("attempt-a", 1, "instance-a", "stop-a")
                .await
                .unwrap()
        );
        assert_eq!(f.effects.cancelled.lock().unwrap().len(), 1);
        assert_eq!(
            f.bridge.query("attempt-a", 1, "instance-a").unwrap()["stop_effect_observation"],
            "reserved_delivery_unknown"
        );
    }
    #[tokio::test(start_paused = true)]
    async fn effect_timeout_is_uncertain_and_cannot_be_retried_as_a_fresh_stop() {
        let f = fixture();
        let binding = register(&f, 1).await;
        f.bridge
            .bind_attempt("attempt-a", 1, "instance-a", binding)
            .unwrap();
        f.effects.pause.store(true, Ordering::SeqCst);
        let receipt = f
            .bridge
            .stop_bound("attempt-a", 1, "instance-a", "stop-a")
            .await
            .unwrap();
        let query = f.bridge.query("attempt-a", 1, "instance-a").unwrap();
        assert_eq!(
            query["stop_effect_observation"],
            "effect_batch_timed_out_delivery_unknown"
        );
        assert_eq!(query["owned_execution_ceased"], false);
        assert_eq!(
            receipt,
            f.bridge
                .stop_bound("attempt-a", 1, "instance-a", "stop-a")
                .await
                .unwrap()
        );
        assert_eq!(f.effects.cancelled.lock().unwrap().len(), 1);
    }
    #[tokio::test]
    async fn fresh_bridge_after_restart_does_not_rediscover_or_invent_ownership() {
        let f = fixture();
        let binding = register(&f, 1).await;
        f.bridge
            .bind_attempt("attempt-a", 1, "instance-a", binding)
            .unwrap();
        let fresh = SessionBridge::new(f.core.clone(), f.effects.clone());
        assert_eq!(
            fresh.query("attempt-a", 1, "instance-a").unwrap_err(),
            BridgeError::UnknownAttempt
        );
        assert_eq!(
            fresh
                .stop_bound("attempt-a", 1, "instance-a", "stop-a")
                .await
                .unwrap_err(),
            BridgeError::UnknownAttempt
        );
        let capabilities = fresh.capabilities();
        for field in [
            "execution_ready",
            "spawn_available",
            "durable_state",
            "durable_start_receipt",
            "fenced_stop_proof",
            "result_sha256",
            "automatic_handoff",
        ] {
            assert_eq!(capabilities[field], false);
        }
        assert!(f.effects.cancelled.lock().unwrap().is_empty());
    }
    fn durable_ticket(
        f: &Fixture,
        binding: SessionBinding,
    ) -> (Arc<Mutex<OwnerStore>>, OwnerTicket, std::path::PathBuf) {
        let path = f.root.path().join("iris-owner.sqlite3");
        let mut store = OwnerStore::open(&path).unwrap();
        let b = crate::hub::iris_execution_journal::Binding {
            principal_id: "iris".into(),
            workspace_id: "workspace".into(),
            plan_id: "plan".into(),
            plan_revision: 1,
            task_id: "task".into(),
            instruction_revision: 1,
            instruction_hash: "a".repeat(64),
            approval_id: "approval".into(),
            approval_revision: 1,
            scope_hash: "b".repeat(64),
            execution_host_id: "host".into(),
            executor_instance_id: "instance-a".into(),
            attempt_id: "attempt-a".into(),
            fence: 1,
            provider: "synthetic-provider".into(),
            profile_id: "default".into(),
        };
        let t = store.begin_owner(&b, "owner-a", 1, 100).unwrap();
        store.claim_spawn(&t, binding.session.0, 100).unwrap();
        store.bind_root(&t, 9001, 100).unwrap();
        (Arc::new(Mutex::new(store)), t, path)
    }
    #[tokio::test]
    async fn durable_stop_is_committed_before_effect_and_duplicate_does_not_resequence() {
        let f = fixture();
        let binding = register(&f, 1).await;
        let (store, ticket, path) = durable_ticket(&f, binding);
        *f.effects.owner_expectation.lock().unwrap() =
            Some((path, ticket.binding.clone(), "stop-a".into()));
        let bridge =
            SessionBridge::new_with_owner_store(f.core.clone(), f.effects.clone(), store.clone());
        bridge.bind_owner_attempt(&ticket, binding).unwrap();
        let receipt = bridge
            .stop_bound("attempt-a", 1, "instance-a", "stop-a")
            .await
            .unwrap();
        assert_eq!(receipt["owner_stop_request_durable"], true);
        assert_eq!(receipt["owned_execution_ceased"], false);
        assert!(f.effects.owner_gate_seen.load(Ordering::SeqCst));
        assert_eq!(
            bridge
                .stop_bound("attempt-a", 1, "instance-a", "stop-a")
                .await
                .unwrap(),
            receipt
        );
        assert_eq!(
            bridge
                .stop_bound("attempt-a", 1, "instance-a", "stop-b")
                .await
                .unwrap_err(),
            BridgeError::StopAlreadyRequested
        );
        assert_eq!(
            store.lock().unwrap().query(&ticket.binding).unwrap()["sequence"],
            1
        );
        assert_eq!(f.effects.cancelled.lock().unwrap().len(), 1);
        assert_eq!(bridge.capabilities()["execution_ready"], false);
    }
    #[tokio::test]
    async fn failed_owner_write_reserves_uncertainty_and_applies_zero_effects() {
        let f = fixture();
        let binding = register(&f, 1).await;
        let (store, ticket, path) = durable_ticket(&f, binding);
        let bridge =
            SessionBridge::new_with_owner_store(f.core.clone(), f.effects.clone(), store.clone());
        bridge.bind_owner_attempt(&ticket, binding).unwrap();
        let fault = rusqlite::Connection::open(path).unwrap();
        fault.execute_batch("CREATE TRIGGER fail_owner_stop BEFORE INSERT ON receipts BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
        let receipt = bridge
            .stop_bound("attempt-a", 1, "instance-a", "stop-a")
            .await
            .unwrap();
        assert_eq!(receipt["owner_stop_request_durable"], false);
        assert_eq!(receipt["execution_state"], "stop_unconfirmed");
        assert!(f.effects.cancelled.lock().unwrap().is_empty());
        assert_eq!(
            store.lock().unwrap().query(&ticket.binding).unwrap()["sequence"],
            0
        );
        fault.execute_batch("DROP TRIGGER fail_owner_stop").unwrap();
        assert_eq!(
            bridge
                .stop_bound("attempt-a", 1, "instance-a", "stop-a")
                .await
                .unwrap(),
            receipt
        );
        assert_eq!(
            bridge
                .stop_bound("attempt-a", 1, "instance-a", "stop-b")
                .await
                .unwrap_err(),
            BridgeError::StopAlreadyRequested
        );
        assert!(f.effects.cancelled.lock().unwrap().is_empty());
    }
    #[tokio::test]
    async fn stale_owner_fence_scope_and_missing_ticket_never_admit_an_effect() {
        let f = fixture();
        let binding = register(&f, 1).await;
        let (store, ticket, _) = durable_ticket(&f, binding);
        let bridge =
            SessionBridge::new_with_owner_store(f.core.clone(), f.effects.clone(), store.clone());
        let mut stale = ticket.clone();
        stale.binding.fence = 2;
        assert!(bridge.bind_owner_attempt(&stale, binding).is_err());
        let mut wrong = ticket.clone();
        wrong.binding.profile_id = "wrong-profile".into();
        assert_eq!(
            bridge.bind_owner_attempt(&wrong, binding).unwrap_err(),
            BridgeError::OwnershipConflict
        );
        bridge.bind_owner_attempt(&ticket, binding).unwrap();
        assert_eq!(
            bridge
                .stop_bound("attempt-a", 2, "instance-a", "stop-a")
                .await
                .unwrap_err(),
            BridgeError::StaleAttempt
        );
        assert!(f.effects.cancelled.lock().unwrap().is_empty());
        assert!(store.lock().unwrap().query(&ticket.binding).unwrap()["stop_request_id"].is_null());
        let legacy = SessionBridge::new_with_owner_store(f.core.clone(), f.effects.clone(), store);
        legacy
            .bind_attempt("attempt-a", 1, "instance-a", binding)
            .unwrap();
        let receipt = legacy
            .stop_bound("attempt-a", 1, "instance-a", "stop-no-ticket")
            .await
            .unwrap();
        assert_eq!(receipt["owner_stop_request_durable"], false);
        assert!(f.effects.cancelled.lock().unwrap().is_empty());
    }
}
