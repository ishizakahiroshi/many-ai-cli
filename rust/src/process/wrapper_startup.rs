//! Startup-only ownership for a detached wrapper, distinct from provider PTY
//! ownership. The fixed Go oracle uses setsid / NEW_PROCESS_GROUP, null stdin,
//! private append-only spawn output, and a separate direct-child waiter.
//!
//! No label lookup can transfer this owner. The Hub's one-use registration-proof
//! caller must match the exact proof and disarm startup ownership before sending
//! its ACK. Provisional drop is delivery-uncertain, so it detaches rather than
//! killing an already-acknowledged wrapper. A receipt only observes exit.
use super::ProcessPlan;
use crate::proto::core::{SessionBinding, SpawnAttemptId};
use std::{
    fs::File,
    io,
    sync::{Arc, Mutex, MutexGuard},
    thread,
    time::Duration,
};
use tokio::sync::watch;

#[cfg(unix)]
#[path = "wrapper_startup_unix.rs"]
mod native;
#[cfg(windows)]
#[path = "wrapper_startup_windows.rs"]
mod native;

/// Internal bootstrap metadata, not a provider environment variable. The binary
/// must adopt it before registration and strip it from every provider launch.
pub const STARTUP_JOB_ENV: &str = "MANY_AI_CLI_INTERNAL_STARTUP_JOB";

/// Created only by the core after consuming the exact one-use launch proof.
/// No public constructor, Clone, deserialization or label-based conversion.
pub struct SpawnRegistrationReceipt {
    attempt: SpawnAttemptId,
    binding: SessionBinding,
    pid: u32,
}
impl SpawnRegistrationReceipt {
    pub(crate) fn proof_bound(attempt: SpawnAttemptId, binding: SessionBinding, pid: u32) -> Self {
        Self {
            attempt,
            binding,
            pid,
        }
    }
    pub fn attempt(&self) -> SpawnAttemptId {
        self.attempt
    }
    pub fn binding(&self) -> SessionBinding {
        self.binding
    }
    pub fn pid(&self) -> u32 {
        self.pid
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WrapperExit {
    pub code: Option<i64>,
    pub signal: Option<i32>,
}
#[derive(Clone)]
struct Failure(io::ErrorKind, String);
type ExitResult = Result<WrapperExit, Failure>;

/// Passive, repeatable exit observation. It holds no process termination right.
#[derive(Clone)]
pub struct ReapReceipt {
    pid: u32,
    result: watch::Receiver<Option<ExitResult>>,
}
impl ReapReceipt {
    pub fn pid(&self) -> u32 {
        self.pid
    }
    pub async fn wait(&self) -> io::Result<WrapperExit> {
        let mut receiver = self.result.clone();
        loop {
            if let Some(result) = receiver.borrow_and_update().clone() {
                return result.map_err(|Failure(kind, message)| io::Error::new(kind, message));
            }
            receiver
                .changed()
                .await
                .map_err(|_| io::Error::other("wrapper reaper ended without an exit result"))?;
        }
    }
}

#[derive(PartialEq, Eq)]
enum Phase {
    Starting,
    Provisional,
    Transferred,
    Terminating,
}
struct State {
    child: Option<native::Child>,
    phase: Phase,
}
fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    // Cleanup must still run if an owner unwinds while holding the lock.
    state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A unique startup owner. Keep it inside the lifecycle task, not an HTTP
/// request future whose cancellation says nothing about a registered session.
pub struct WrapperStartup {
    attempt: SpawnAttemptId,
    state: Arc<Mutex<State>>,
    receipt: ReapReceipt,
}
impl WrapperStartup {
    /// The caller supplies already-open private append-only files from its
    /// selected runtime root. This layer never reopens a path or uses pipes.
    pub fn spawn(
        attempt: SpawnAttemptId,
        plan: &ProcessPlan,
        stdout: File,
        stderr: File,
    ) -> io::Result<Self> {
        if !plan.stdin.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "detached wrapper startup requires null stdin",
            ));
        }
        let state = Arc::new(Mutex::new(State {
            child: None,
            phase: Phase::Starting,
        }));
        let shared = state.clone();
        let (result_tx, result) = watch::channel(None);
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        // Reserve the waiter before creating a process. A thread creation failure
        // cannot strand a child; an exec failure closes ready_tx and ends it.
        thread::Builder::new()
            .name("many-ai-wrapper-reaper".into())
            .spawn(move || {
                if ready_rx.recv().is_err() {
                    return;
                }
                loop {
                    let result = {
                        let mut state = lock(&shared);
                        let child = state
                            .child
                            .as_mut()
                            .expect("startup child installed before wake");
                        match child.poll() {
                            Ok(Some(exit)) => Some(Ok(exit)),
                            Ok(None) => None,
                            Err(error) => {
                                child.terminate();
                                // Reap on this dedicated thread even if the Hub Tokio
                                // runtime has already shut down.
                                let _ = child.wait();
                                Some(Err(Failure(error.kind(), error.to_string())))
                            }
                        }
                    };
                    if let Some(result) = result {
                        result_tx.send_replace(Some(result));
                        return;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
            })?;
        let child = native::Child::spawn(plan, stdout, stderr)?;
        let pid = child.pid();
        lock(&state).child = Some(child);
        let owner = Self {
            attempt,
            state,
            receipt: ReapReceipt { pid, result },
        };
        ready_tx
            .send(())
            .map_err(|_| io::Error::other("wrapper reaper did not start"))?;
        Ok(owner)
    }
    pub fn pid(&self) -> u32 {
        self.receipt.pid
    }
    pub fn receipt(&self) -> ReapReceipt {
        self.receipt.clone()
    }
    pub async fn wait_for_exit(&self) -> io::Result<WrapperExit> {
        self.receipt.wait().await
    }

    /// Consume a core-created proof receipt after the pending-attempt owner has
    /// been installed, before the first registration ACK is written. The caller
    /// must arrange that installation wait; this module has no session registry.
    pub fn begin_registration(
        self,
        receipt: SpawnRegistrationReceipt,
    ) -> io::Result<ProvisionalWrapper> {
        if receipt.attempt != self.attempt || receipt.pid != self.pid() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "registration does not own this wrapper startup",
            ));
        }
        {
            let mut state = lock(&self.state);
            if state.phase != Phase::Starting {
                return Err(io::Error::other("wrapper startup is no longer pending"));
            }
            let child = state.child.as_mut().expect("started wrapper child");
            if child.poll()?.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "wrapper exited before startup transfer",
                ));
            }
            child.disarm();
            state.phase = Phase::Provisional;
        }
        Ok(ProvisionalWrapper {
            state: self.state.clone(),
            receipt: self.receipt.clone(),
            binding: receipt.binding,
        })
    }

    /// Failure/timeout cleanup returns the same passive receipt used by the
    /// caller's bounded reap wait. Cleanup never waits for inherited pipes.
    pub fn abort(self) -> ReapReceipt {
        self.terminate_startup();
        self.receipt.clone()
    }
    fn terminate_startup(&self) {
        let mut state = lock(&self.state);
        if state.phase == Phase::Starting {
            state
                .child
                .as_mut()
                .expect("started wrapper child")
                .terminate();
            state.phase = Phase::Terminating;
        }
    }
}
impl Drop for WrapperStartup {
    fn drop(&mut self) {
        self.terminate_startup();
    }
}

/// Startup termination is already disarmed. This unique guard retains an
/// explicit abort right for a known pre-ACK failure. Delivery-uncertain/drop
/// paths detach because the wrapper may have already started its provider.
pub struct ProvisionalWrapper {
    state: Arc<Mutex<State>>,
    receipt: ReapReceipt,
    binding: SessionBinding,
}
impl ProvisionalWrapper {
    pub fn binding(&self) -> SessionBinding {
        self.binding
    }
    pub fn acknowledged(self) -> ReapReceipt {
        self.detach();
        self.receipt.clone()
    }
    /// Only use when no successful registration ACK could have reached the
    /// wrapper. For uncertain delivery, drop this guard and use the normal core
    /// disconnect/dismissal policy. The provider PTY is a separate owner/group.
    pub fn abort(self) -> ReapReceipt {
        let mut state = lock(&self.state);
        if state.phase == Phase::Provisional {
            state
                .child
                .as_mut()
                .expect("started wrapper child")
                .terminate();
            state.phase = Phase::Terminating;
        }
        self.receipt.clone()
    }
    fn detach(&self) {
        let mut state = lock(&self.state);
        if state.phase == Phase::Provisional {
            state
                .child
                .as_mut()
                .expect("started wrapper child")
                .transfer();
            state.phase = Phase::Transferred;
        }
    }
}
impl Drop for ProvisionalWrapper {
    fn drop(&mut self) {
        self.detach();
    }
}

#[cfg(test)]
#[path = "wrapper_startup_tests.rs"]
mod tests;
