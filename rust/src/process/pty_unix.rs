//! portable-pty establishes setsid/TIOCSCTTY in pre_exec. Nonblocking master IO
//! avoids blocking-reader tasks that survive cancellation or inherited PTY fds.
use crate::process::pty::{CLOSE_GRACE, PtyExit, PtySession, PtySize, REAP_GRACE};
use crate::{
    process::{Cancellation, ProcessPlan},
    proto::core::{CoreFuture, SPAWN_PROOF_ENV},
};
use portable_pty::{CommandBuilder, MasterPty, PtySize as NativeSize, native_pty_system};
use std::{
    fs::File,
    io,
    os::fd::{AsRawFd, FromRawFd},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{io::unix::AsyncFd, sync::watch};

fn lock<T>(m: &Mutex<T>) -> io::Result<std::sync::MutexGuard<'_, T>> {
    m.lock()
        .map_err(|_| io::Error::other("PTY ownership lock poisoned"))
}
fn size(value: PtySize) -> NativeSize {
    NativeSize {
        cols: value.cols,
        rows: value.rows,
        pixel_width: 0,
        pixel_height: 0,
    }
}
#[derive(Clone)]
struct Failure(io::ErrorKind, String);
type ResultValue = Result<PtyExit, Failure>;
struct NativePty {
    pid: u32,
    io: Arc<AsyncFd<File>>,
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    closing: Cancellation,
    result: watch::Receiver<Option<ResultValue>>,
}
/// The reaper owns the child independently of the public handle. Owner drop
/// requests cancellation, but never aborts the task responsible for waitpid.
struct ChildGuard {
    child: Option<Box<std::process::Child>>,
    pid: u32,
    terminal: Arc<AsyncFd<File>>,
    reaped: bool,
}
impl ChildGuard {
    fn signal(&self, signal: i32) {
        // Only the just-created setsid process group is targeted. No PID-file,
        // process-name search, or caller-supplied PID is accepted here.
        unsafe {
            let foreground = libc::tcgetpgrp(self.terminal.get_ref().as_raw_fd());
            if foreground > 0
                && foreground != self.pid as i32
                && libc::getsid(foreground) == self.pid as i32
            {
                libc::kill(-foreground, signal);
            }
            libc::kill(-(self.pid as i32), signal);
        }
    }
    fn poll(&mut self, forced: bool) -> io::Result<Option<PtyExit>> {
        use std::os::unix::process::ExitStatusExt;
        let result = self
            .child
            .as_mut()
            .expect("owned Unix child")
            .try_wait()?
            .map(|s| {
                let signal = s.signal().map(signal_name).unwrap_or_default();
                PtyExit {
                    code: i64::from(s.code().unwrap_or(-1)),
                    signal,
                    forced,
                }
            });
        if result.is_some() {
            self.reaped = true;
        }
        Ok(result)
    }
}
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if !self.reaped {
            self.signal(libc::SIGKILL);
            if let Some(mut child) = self.child.take() {
                let _ = child.kill();
                // The runtime is being dropped. Reaping the already killed
                // direct child remains owned by this dedicated thread.
                let _ = std::thread::Builder::new()
                    .name("many-ai-pty-reaper".into())
                    .spawn(move || {
                        let _ = child.wait();
                    });
            }
        }
    }
}
fn signal_name(signal: i32) -> String {
    // Go syscall.Signal.String is a generated English table, not locale-aware
    // strsignal. Preserve its platform-specific spelling on the supported OSes.
    #[cfg(target_os = "linux")]
    const NAMES: &[&str] = &[
        "",
        "hangup",
        "interrupt",
        "quit",
        "illegal instruction",
        "trace/breakpoint trap",
        "aborted",
        "bus error",
        "floating point exception",
        "killed",
        "user defined signal 1",
        "segmentation fault",
        "user defined signal 2",
        "broken pipe",
        "alarm clock",
        "terminated",
        "stack fault",
        "child exited",
        "continued",
        "stopped (signal)",
        "stopped",
        "stopped (tty input)",
        "stopped (tty output)",
        "urgent I/O condition",
        "CPU time limit exceeded",
        "file size limit exceeded",
        "virtual timer expired",
        "profiling timer expired",
        "window changed",
        "I/O possible",
        "power failure",
        "bad system call",
    ];
    #[cfg(target_os = "macos")]
    const NAMES: &[&str] = &[
        "",
        "hangup",
        "interrupt",
        "quit",
        "illegal instruction",
        "trace/BPT trap",
        "abort trap",
        "EMT trap",
        "floating point exception",
        "killed",
        "bus error",
        "segmentation fault",
        "bad system call",
        "broken pipe",
        "alarm clock",
        "terminated",
        "urgent I/O condition",
        "suspended (signal)",
        "suspended",
        "continued",
        "child exited",
        "stopped (tty input)",
        "stopped (tty output)",
        "I/O possible",
        "cputime limit exceeded",
        "filesize limit exceeded",
        "virtual timer expired",
        "profiling timer expired",
        "window size changes",
        "information request",
        "user defined signal 1",
        "user defined signal 2",
    ];
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    const NAMES: &[&str] = &[];
    usize::try_from(signal)
        .ok()
        .and_then(|n| NAMES.get(n))
        .filter(|s| !s.is_empty())
        .map(|name| (*name).into())
        .unwrap_or_else(|| format!("signal {signal}"))
}

pub fn spawn(plan: &ProcessPlan, dimensions: PtySize) -> io::Result<Arc<dyn PtySession>> {
    let pair = native_pty_system()
        .openpty(size(dimensions))
        .map_err(io::Error::other)?;
    let fd = pair
        .master
        .as_raw_fd()
        .ok_or_else(|| io::Error::other("PTY master has no fd"))?;
    // SAFETY: duplicate only our opened master, set nonblocking before spawn so
    // no failure after exec can leave a child unowned.
    let fd = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let file = unsafe { File::from_raw_fd(fd) };
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    let descriptor = Arc::new(AsyncFd::new(file)?);
    let mut command = CommandBuilder::new(&plan.executable);
    command.args(&plan.args);
    command.cwd(&plan.cwd);
    for (key, value) in &plan.env {
        match value {
            Some(value) => command.env(key, value),
            None => command.env_remove(key),
        }
    }
    // Proof is wrapper-only even if a caller supplies a conflicting env override.
    command.env_remove(SPAWN_PROOF_ENV);
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().eq_ignore_ascii_case(SPAWN_PROOF_ENV) {
            command.env_remove(key);
        }
    }
    for key in plan.env.keys() {
        if key.to_string_lossy().eq_ignore_ascii_case(SPAWN_PROOF_ENV) {
            command.env_remove(key);
        }
    }
    let mut child: Box<dyn portable_pty::Child> = pair
        .slave
        .spawn_command(command)
        .map_err(io::Error::other)?;
    drop(pair.slave);
    let Some(pid) = child.process_id() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(io::Error::other("spawned PTY has no PID"));
    };
    // This backend promises std::process::Child. On an unexpected adapter,
    // kill and reap before refusing it rather than leaving a child unowned.
    if !child.is::<std::process::Child>() {
        let _ = child.kill();
        let _ = child.wait();
        return Err(io::Error::other("unexpected native Unix PTY child adapter"));
    }
    let child = child
        .downcast::<std::process::Child>()
        .map_err(|_| io::Error::other("PTY child downcast failed"))?;
    let guard = ChildGuard {
        child: Some(child),
        pid,
        terminal: descriptor.clone(),
        reaped: false,
    };
    let closing = Cancellation::default();
    let closed = closing.clone();
    let (tx, result) = watch::channel(None);
    tokio::spawn(async move {
        let mut guard = guard;
        let mut close_at = None;
        let mut kill_at = None;
        loop {
            if closed.is_cancelled() && close_at.is_none() {
                guard.signal(libc::SIGTERM);
                close_at = Some(tokio::time::Instant::now());
            }
            if close_at.is_some_and(|at| at.elapsed() >= CLOSE_GRACE) && kill_at.is_none() {
                guard.signal(libc::SIGKILL);
                let _ = guard.child.as_mut().expect("owned child").kill();
                kill_at = Some(tokio::time::Instant::now());
            }
            match guard.poll(kill_at.is_some()) {
                Ok(Some(exit)) => {
                    // Direct-child exit does not establish descendant exit.
                    // Stop only this session group before releasing ownership.
                    guard.signal(libc::SIGKILL);
                    tx.send_replace(Some(Ok(exit)));
                    break;
                }
                Err(e) => {
                    tx.send_replace(Some(Err(Failure(e.kind(), e.to_string()))));
                    break;
                }
                Ok(None) => {}
            }
            if kill_at.is_some_and(|at| at.elapsed() >= REAP_GRACE) {
                tx.send_replace(Some(Err(Failure(
                    io::ErrorKind::TimedOut,
                    "PTY child did not reap after termination".into(),
                ))));
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });
    Ok(Arc::new(NativePty {
        pid,
        io: descriptor,
        master: Mutex::new(Some(pair.master)),
        closing,
        result,
    }))
}
impl PtySession for NativePty {
    fn pid(&self) -> u32 {
        self.pid
    }
    fn read<'a>(&'a self, target: &'a mut [u8]) -> CoreFuture<'a, io::Result<usize>> {
        Box::pin(async move {
            loop {
                let mut ready = tokio::select! {
                    _ = self.closing.cancelled() => return Ok(0),
                    ready = self.io.readable() => ready?,
                };
                match ready.try_io(|fd| {
                    let n = unsafe {
                        libc::read(
                            fd.get_ref().as_raw_fd(),
                            target.as_mut_ptr().cast(),
                            target.len(),
                        )
                    };
                    if n < 0 {
                        let e = io::Error::last_os_error();
                        if e.raw_os_error() == Some(libc::EIO) {
                            Ok(0)
                        } else {
                            Err(e)
                        }
                    } else {
                        Ok(n as usize)
                    }
                }) {
                    Ok(result) => return result,
                    Err(_) => continue,
                }
            }
        })
    }
    fn write<'a>(&'a self, bytes: &'a [u8]) -> CoreFuture<'a, io::Result<usize>> {
        Box::pin(async move {
            loop {
                let mut ready = tokio::select! {
                    _ = self.closing.cancelled() => return Err(io::Error::new(io::ErrorKind::BrokenPipe, "PTY closed")),
                    ready = self.io.writable() => ready?,
                };
                match ready.try_io(|fd| {
                    let n = unsafe {
                        libc::write(fd.get_ref().as_raw_fd(), bytes.as_ptr().cast(), bytes.len())
                    };
                    if n < 0 {
                        Err(io::Error::last_os_error())
                    } else {
                        Ok(n as usize)
                    }
                }) {
                    Ok(result) => return result,
                    Err(_) => continue,
                }
            }
        })
    }
    fn resize(&self, dimensions: PtySize) -> io::Result<()> {
        lock(&self.master)?
            .as_ref()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "PTY closed"))?
            .resize(size(dimensions))
            .map_err(io::Error::other)
    }
    fn close(&self) {
        self.closing.cancel();
        if let Ok(mut master) = self.master.lock() {
            master.take();
        }
    }
    fn wait(&self) -> CoreFuture<'_, io::Result<PtyExit>> {
        Box::pin(async move {
            let mut result = self.result.clone();
            loop {
                if let Some(value) = result.borrow_and_update().clone() {
                    return value.map_err(|Failure(kind, message)| io::Error::new(kind, message));
                }
                result
                    .changed()
                    .await
                    .map_err(|_| io::Error::other("PTY reaper ended without a result"))?;
            }
        })
    }
}
impl Drop for NativePty {
    fn drop(&mut self) {
        self.close();
    }
}
