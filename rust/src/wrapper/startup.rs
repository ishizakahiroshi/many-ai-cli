//! Child-side adoption of the Hub's startup Job. This is a single process-lifetime
//! guard, not a handle registry. It must be acquired before initial registration,
//! and retained through `exit(code)` after wrapper/provider cleanup completes.
//!
//! ExitProcess terminates the process before closing its handles. Native tests
//! must verify the resulting status; Linux compilation is not that evidence.
//! https://learn.microsoft.com/windows/win32/api/processthreadsapi/nf-processthreadsapi-exitprocess
//! https://learn.microsoft.com/windows/win32/procthread/job-objects
//!
//! Dropping the last kill-on-close Job handle before that could instead terminate
//! this wrapper and replace its intended status. `exit` deliberately skips Rust
//! destructors at the final process boundary, as std::process::exit documents.
use crate::process::{ProcessPlan, wrapper_startup::STARTUP_JOB_ENV};
use std::{ffi::OsStr, io};

/// Remove bootstrap metadata from a provider plan, including a differently cased
/// Windows environment override. The inherited Job handle is separately made
/// noninheritable at adoption, before any provider can execute.
pub fn strip_provider_environment(plan: &mut ProcessPlan) {
    plan.env
        .retain(|key, _| !key.to_string_lossy().eq_ignore_ascii_case(STARTUP_JOB_ENV));
    plan.env.insert(STARTUP_JOB_ENV.into(), None);
}

pub struct StartupLifetime {
    #[cfg(windows)]
    job: Option<std::os::windows::io::OwnedHandle>,
}
impl StartupLifetime {
    /// Invoke once, at the binary entry, before registration or provider launch.
    /// The metadata is internal and must never be printed or persisted. No
    /// process-global environment mutation is needed in the threaded runtime.
    pub fn adopt_from_environment() -> io::Result<Self> {
        Self::adopt(std::env::var_os(STARTUP_JOB_ENV).as_deref())
    }
    pub fn adopt(value: Option<&OsStr>) -> io::Result<Self> {
        #[cfg(windows)]
        {
            adopt(value).map(|job| Self { job })
        }
        #[cfg(not(windows))]
        {
            if value.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "startup Job metadata is only valid on Windows",
                ));
            }
            Ok(Self {})
        }
    }
    /// Finish all application/provider cleanup first. This final operation keeps
    /// the adopted handle alive until the operating system exits the wrapper.
    pub fn exit(self, code: i32) -> ! {
        #[cfg(windows)]
        let _keep_job_alive = self.job.as_ref();
        std::process::exit(code)
    }
}

#[cfg(windows)]
fn adopt(value: Option<&OsStr>) -> io::Result<Option<std::os::windows::io::OwnedHandle>> {
    use std::{
        mem,
        os::windows::io::{FromRawHandle, OwnedHandle},
        ptr,
        sync::atomic::{AtomicBool, Ordering},
    };
    use windows_sys::Win32::{
        Foundation::*,
        System::{JobObjects::*, Threading::GetCurrentProcess},
    };
    static ADOPTED: AtomicBool = AtomicBool::new(false);
    let Some(value) = value else {
        return Ok(None);
    };
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid inherited wrapper startup Job",
        )
    };
    let number = value
        .to_str()
        .filter(|value| !value.is_empty() && value.bytes().all(|c| c.is_ascii_digit()))
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value != 0 && *value != usize::MAX)
        .ok_or_else(invalid)?;
    if ADOPTED
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "startup Job was already adopted",
        ));
    }
    let handle = number as HANDLE;
    let mut flags = 0;
    let mut member = 0;
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { mem::zeroed() };
    // Validate an inherited Job to which this process is actually assigned.
    // Never take ownership of an invalid or unrelated environment-supplied handle.
    if unsafe { GetHandleInformation(handle, &mut flags) } == 0
        || flags & HANDLE_FLAG_INHERIT == 0
        || unsafe { IsProcessInJob(GetCurrentProcess(), handle, &mut member) } == 0
        || member == 0
        || unsafe {
            QueryInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&mut limits as *mut JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                mem::size_of_val(&limits) as u32,
                ptr::null_mut(),
            )
        } == 0
        || limits.BasicLimitInformation.LimitFlags & JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE == 0
    {
        return Err(invalid());
    }
    if unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(Some(unsafe { OwnedHandle::from_raw_handle(handle) }))
}
