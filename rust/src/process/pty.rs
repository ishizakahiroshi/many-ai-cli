//! PTY ownership is distinct from pipe-based ManagedProcess. Implementations
//! must establish containment before child code runs, and make IO cancellable.
use crate::{process::ProcessPlan, proto::core::CoreFuture};
use std::{io, sync::Arc, time::Duration};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PtySize {
    pub cols: u16,
    pub rows: u16,
}
impl Default for PtySize {
    fn default() -> Self {
        Self {
            cols: 200,
            rows: 50,
        }
    }
}
impl PtySize {
    pub fn from_wire(cols: i64, rows: i64) -> Option<Self> {
        if cols <= 0 || rows <= 0 {
            return None;
        }
        // Go casts positive wire values to uint16 at the PTY boundary.
        Some(Self {
            cols: cols as u16,
            rows: rows as u16,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PtyExit {
    pub code: i64,
    pub signal: String,
    pub forced: bool,
}
impl PtyExit {
    pub fn state(&self) -> &'static str {
        if self.code == 0 && self.signal.is_empty() {
            "completed"
        } else {
            "error"
        }
    }
}
/// Native observations cover only this process containment boundary. They do
/// not establish external effects, remote job cessation, or work success.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalCessation {
    scope: &'static str,
    root_pid: u32,
    root_waited: bool,
    active_processes: u32,
    checked_at_unix_ms: u64,
}
impl LocalCessation {
    pub fn scope(&self) -> &'static str {
        self.scope
    }
    pub fn root_pid(&self) -> u32 {
        self.root_pid
    }
    pub fn root_waited(&self) -> bool {
        self.root_waited
    }
    pub fn active_processes(&self) -> u32 {
        self.active_processes
    }
    pub fn checked_at_unix_ms(&self) -> u64 {
        self.checked_at_unix_ms
    }
    #[cfg(windows)]
    pub(super) fn windows_job(root_pid: u32, checked_at_unix_ms: u64) -> Self {
        Self {
            scope: "windows-owned-job",
            root_pid,
            root_waited: true,
            active_processes: 0,
            checked_at_unix_ms,
        }
    }
}

/// A single owner controls close and wait. Shared IO references cannot orphan
/// the child: dropping the last owner must still terminate and reap it.
pub trait PtySession: Send + Sync {
    fn pid(&self) -> u32;
    fn read<'a>(&'a self, target: &'a mut [u8]) -> CoreFuture<'a, io::Result<usize>>;
    fn write<'a>(&'a self, bytes: &'a [u8]) -> CoreFuture<'a, io::Result<usize>>;
    fn resize(&self, size: PtySize) -> io::Result<()>;
    fn close(&self);
    /// Optional owner-only hard-stop request. Success means the native request
    /// succeeded, not that cessation has been observed. Ordinary close is
    /// deliberately separate; unsupported platforms must fail explicitly.
    fn close_owned(&self) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "owned native stop unavailable",
        ))
    }
    fn wait(&self) -> CoreFuture<'_, io::Result<PtyExit>>;
    /// Optional checked termination of the exact owned containment boundary.
    /// An exit code or a Unix process group alone cannot prove all descendants
    /// ceased. Existing implementations therefore default to Unsupported.
    fn terminate_checked(&self, _timeout: Duration) -> CoreFuture<'_, io::Result<LocalCessation>> {
        Box::pin(async {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "checked local containment cessation unavailable",
            ))
        })
    }
}
pub trait PtyFactory: Send + Sync {
    fn spawn(&self, plan: &ProcessPlan, size: PtySize) -> io::Result<Arc<dyn PtySession>>;
}
#[derive(Default)]
pub struct NativePtyFactory;
impl PtyFactory for NativePtyFactory {
    fn spawn(&self, plan: &ProcessPlan, size: PtySize) -> io::Result<Arc<dyn PtySession>> {
        #[cfg(unix)]
        {
            super::pty_unix::spawn(plan, size)
        }
        #[cfg(windows)]
        {
            super::pty_windows::spawn(plan, size)
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (plan, size);
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "native PTY is unavailable on this platform",
            ))
        }
    }
}
pub const CLOSE_GRACE: Duration = Duration::from_secs(2);
pub const REAP_GRACE: Duration = Duration::from_secs(2);
