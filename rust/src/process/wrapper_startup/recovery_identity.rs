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

/// The identity of the process now holding `pid`, or None if none does.
pub(crate) fn of_pid(pid: u32) -> io::Result<Option<String>> {
    if pid == 0 {
        return Ok(None);
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::{FromRawHandle, OwnedHandle};
        use windows_sys::Win32::{
            Foundation::ERROR_INVALID_PARAMETER,
            System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
        };
        // SAFETY: fixed query right; the returned handle is immediately owned.
        let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if raw.is_null() {
            let error = io::Error::last_os_error();
            return if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) {
                Ok(None)
            } else {
                Err(error)
            };
        }
        // SAFETY: OpenProcess returned a new live process handle.
        let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
        from_owned_handle(&handle).map(Some)
    }
    #[cfg(target_os = "linux")]
    {
        match linux_identity(pid) {
            Ok(identity) => Ok(Some(identity)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "native process identity unavailable",
        ))
    }
}

/// When the process an identity describes was created, in nanoseconds since
/// the Unix epoch. None for a form this platform cannot convert.
pub(crate) fn created_unix_nanos(identity: &str) -> Option<i128> {
    if let Some(filetime) = identity.strip_prefix("windows:") {
        // FILETIME counts 100 ns intervals since 1601-01-01.
        let filetime = u64::from_str_radix(filetime, 16).ok()?;
        return Some((i128::from(filetime) - 116_444_736_000_000_000) * 100);
    }
    #[cfg(target_os = "linux")]
    if let Some(rest) = identity.strip_prefix("linux:") {
        let ticks: u64 = rest.rsplit_once(':')?.1.parse().ok()?;
        let stat = std::fs::read_to_string("/proc/stat").ok()?;
        let boot: i128 = stat
            .lines()
            .find_map(|line| line.strip_prefix("btime "))?
            .trim()
            .parse()
            .ok()?;
        // SAFETY: sysconf has no preconditions.
        let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        if hz <= 0 {
            return None;
        }
        return Some(boot * 1_000_000_000 + i128::from(ticks) * 1_000_000_000 / i128::from(hz));
    }
    None
}

pub(crate) fn terminate(pid: u32, expected: &str) -> io::Result<()> {
    if expected.is_empty() {
        return Err(mismatch());
    }
    if terminate_if(pid, |identity| identity == expected)? {
        Ok(())
    } else {
        Err(mismatch())
    }
}

/// Opens the process once, lets `accept` judge its identity, and ends that
/// same process only if accepted. Ok(true): ended, or already gone. Ok(false):
/// another process holds the PID and was left running.
pub(crate) fn terminate_if(pid: u32, accept: impl Fn(&str) -> bool) -> io::Result<bool> {
    if pid == 0 || pid == std::process::id() {
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
                Ok(true)
            } else {
                Err(error)
            };
        }
        // SAFETY: OpenProcess returned a new live process handle.
        let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
        if !accept(&from_owned_handle(&handle)?) {
            return Ok(false);
        }
        // SAFETY: this same retained handle was judged by `accept`.
        if unsafe { TerminateProcess(handle.as_raw_handle(), 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(true)
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
        // SAFETY: fixed syscall. A pidfd retains the target during validation.
        let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
        if raw < 0 {
            let error = io::Error::last_os_error();
            return if error.raw_os_error() == Some(libc::ESRCH) {
                Ok(true)
            } else {
                Err(error)
            };
        }
        // SAFETY: pidfd_open returned a new valid owned descriptor.
        let handle = unsafe { OwnedFd::from_raw_fd(raw as i32) };
        match linux_identity(pid) {
            Ok(identity) if accept(&identity) => {}
            Ok(_) => return Ok(false),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(true),
            Err(_) => return Err(mismatch()),
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
        Ok(true)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = &accept;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "native process identity unavailable",
        ))
    }
}

/// A real process unrelated to the Hub, for tests that must not end the wrong
/// process. Ended and reaped on drop.
#[cfg(all(test, any(windows, target_os = "linux")))]
pub(crate) mod test_support {
    use std::{
        process::{Child, Command, Stdio},
        time::{Duration, Instant},
    };

    pub(crate) struct Bystander(Child);
    impl Bystander {
        pub(crate) fn spawn() -> Self {
            #[cfg(windows)]
            let mut command = {
                let mut command = Command::new("ping");
                command.args(["-n", "60", "127.0.0.1"]);
                command
            };
            #[cfg(target_os = "linux")]
            let mut command = {
                let mut command = Command::new("sleep");
                command.arg("60");
                command
            };
            let child = command
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn a bystander process");
            Self(child)
        }
        pub(crate) fn pid(&self) -> u32 {
            self.0.id()
        }
        pub(crate) fn running(&mut self) -> bool {
            self.0.try_wait().expect("poll the bystander").is_none()
        }
        /// Whether something else ended it within `limit`.
        pub(crate) fn ends_within(&mut self, limit: Duration) -> bool {
            let deadline = Instant::now() + limit;
            loop {
                if !self.running() {
                    return true;
                }
                if Instant::now() >= deadline {
                    return false;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
    impl Drop for Bystander {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}
