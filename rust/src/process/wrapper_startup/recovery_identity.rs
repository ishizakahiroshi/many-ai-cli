//! Persistent recovery identities are captured only from an owned spawn.
use std::io;

fn mismatch() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "probe process identity changed",
    )
}

#[cfg(windows)]
pub(super) fn from_owned_handle(handle: &std::os::windows::io::OwnedHandle) -> io::Result<String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::{Foundation::FILETIME, System::Threading::GetProcessTimes};
    let mut created = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let mut exited = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let mut kernel = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let mut user = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    // SAFETY: the retained process handle is live and outputs are distinct.
    if unsafe {
        GetProcessTimes(
            handle.as_raw_handle(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(format!(
        "windows:{:08x}{:08x}",
        created.dwHighDateTime, created.dwLowDateTime
    ))
}

#[cfg(target_os = "linux")]
fn linux_identity(pid: u32) -> io::Result<String> {
    let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?;
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let fields = stat.rsplit_once(')').ok_or_else(mismatch)?.1;
    let started = fields.split_whitespace().nth(19).ok_or_else(mismatch)?;
    let started = started.parse::<u64>().map_err(|_| mismatch())?;
    Ok(format!("linux:{}:{started}", boot.trim()))
}

/// Only called before the exclusive waiter receives the newly spawned child.
#[cfg(target_os = "linux")]
pub(super) fn from_unreaped_child(pid: u32) -> io::Result<String> {
    linux_identity(pid)
}

pub(crate) fn terminate(pid: u32, expected: &str) -> io::Result<()> {
    if pid == 0 || pid == std::process::id() || expected.is_empty() {
        return Err(mismatch());
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        use windows_sys::Win32::{
            Foundation::ERROR_INVALID_PARAMETER,
            System::Threading::{
                OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE, TerminateProcess,
            },
        };
        // SAFETY: fixed rights; the returned handle is immediately owned.
        let raw = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE,
                0,
                pid,
            )
        };
        if raw.is_null() {
            let error = io::Error::last_os_error();
            return if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) {
                Ok(())
            } else {
                Err(error)
            };
        }
        // SAFETY: OpenProcess returned a new live process handle.
        let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
        if from_owned_handle(&handle)? != expected {
            return Err(mismatch());
        }
        // SAFETY: this same retained handle was compared to the spawn identity.
        if unsafe { TerminateProcess(handle.as_raw_handle(), 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
        // SAFETY: fixed syscall. A pidfd retains the target during validation.
        let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
        if raw < 0 {
            let error = io::Error::last_os_error();
            return if error.raw_os_error() == Some(libc::ESRCH) {
                Ok(())
            } else {
                Err(error)
            };
        }
        // SAFETY: pidfd_open returned a new valid owned descriptor.
        let handle = unsafe { OwnedFd::from_raw_fd(raw as i32) };
        match linux_identity(pid) {
            Ok(identity) if identity == expected => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            _ => return Err(mismatch()),
        }
        // SAFETY: the validated pidfd is retained; no numeric PID is reopened.
        let sent = unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                handle.as_raw_fd(),
                libc::SIGKILL,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        };
        if sent < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(error);
            }
        }
        Ok(())
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "native process identity unavailable",
        ))
    }
}
