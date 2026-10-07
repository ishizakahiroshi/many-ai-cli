//! Owned, cancellable subprocesses. Output readers start before prompt writes.
pub mod execpath;
pub mod pty;
#[cfg(unix)]
mod pty_unix;
#[cfg(windows)]
mod pty_windows;
pub mod wrapper_startup;
use std::{
    collections::BTreeMap,
    ffi::OsString,
    io,
    path::PathBuf,
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
    sync::{broadcast, watch},
};

/// Go hubruntime.PIDAlive compatibility. Liveness alone never establishes
/// identity: callers must also authenticate/probe the recorded endpoint.
pub fn pid_alive(pid: i64) -> bool {
    if pid <= 0 {
        return false;
    }
    #[cfg(unix)]
    {
        let Ok(pid) = libc::pid_t::try_from(pid) else {
            return false;
        };
        // Signal zero performs a liveness/permission check, never sends a signal.
        unsafe { libc::kill(pid, 0) == 0 }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::{
            Foundation::CloseHandle,
            System::Threading::{
                GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            },
        };
        // Go converts its int to uint32 for OpenProcess; retain that wire quirk.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid as u32) };
        if handle.is_null() {
            return false;
        }
        let mut code = 0;
        let ok = unsafe { GetExitCodeProcess(handle, &mut code) } != 0;
        unsafe {
            CloseHandle(handle);
        }
        // Source deliberately accepts the documented STILL_ACTIVE/exit259 ambiguity.
        ok && code == 259
    }
    #[cfg(not(any(unix, windows)))]
    false
}

/// Generate the same 32-byte lowercase hex token shape as the Go baseline.
pub fn random_token() -> io::Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|_| io::Error::other("operating-system random source failed"))?;
    use std::fmt::Write;
    let mut token = String::with_capacity(64);
    for byte in bytes {
        write!(&mut token, "{byte:02x}").expect("write to String");
    }
    Ok(token)
}

#[derive(Clone)]
pub struct Cancellation {
    sender: Arc<watch::Sender<bool>>,
}
impl Default for Cancellation {
    fn default() -> Self {
        let (sender, _) = watch::channel(false);
        Self {
            sender: Arc::new(sender),
        }
    }
}
impl Cancellation {
    pub fn cancel(&self) {
        self.sender.send_replace(true);
    }
    pub fn is_cancelled(&self) -> bool {
        *self.sender.borrow()
    }
    pub async fn cancelled(&self) {
        let mut rx = self.sender.subscribe();
        loop {
            if *rx.borrow_and_update() {
                return;
            }
            if rx.changed().await.is_err() {
                return;
            }
        }
    }
}
#[derive(Clone)]
pub struct ProcessPlan {
    pub executable: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
    /// Inherit the parent then set/remove individual variables. Never mutate the system environment.
    pub env: BTreeMap<OsString, Option<OsString>>,
    pub stdin: Vec<u8>,
    /// Zero explicitly means no deadline (Go headless timeout=0 / long-lived SSH).
    /// Cancellation, owner drop and bounded pipe draining still apply.
    pub timeout: Duration,
    pub output_cap: usize,
    pub pipe_drain_timeout: Duration,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct SpawnOptions {
    /// Windows background launcher children must not create a console window.
    /// This augments, never replaces, suspended Job-assignment containment.
    pub no_window: bool,
    /// Share one native pipe for stdout and stderr, preserving write order.
    pub combined_output: bool,
    pub stdout_null: bool,
    pub stderr_null: bool,
    pub stdin_null: bool,
    /// Use only ProcessPlan.env, with no ambient environment inheritance.
    pub env_clear: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExitOutcome {
    Exited {
        code: Option<i32>,
        signal: Option<i32>,
    },
    Cancelled,
    TimedOut,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub pipes_forced_closed: bool,
    pub outcome: ExitOutcome,
}
#[derive(Default)]
struct Capture {
    bytes: Vec<u8>,
    truncated: bool,
}
async fn read_capped<R: AsyncRead + Unpin>(
    mut reader: R,
    buffer: Arc<Mutex<Capture>>,
    cap: usize,
    events: Option<(broadcast::Sender<ProcessEvent>, OutputStream)>,
) -> io::Result<()> {
    let mut chunk = [0u8; 8192];
    loop {
        let n = reader.read(&mut chunk).await?;
        if n == 0 {
            return Ok(());
        }
        if let Some((events, stream)) = &events {
            let _ = events.send(ProcessEvent::Output {
                stream: *stream,
                bytes: chunk[..n].to_vec(),
            });
        }
        let mut capture = buffer
            .lock()
            .map_err(|_| io::Error::other("output lock poisoned"))?;
        let left = cap.saturating_sub(capture.bytes.len());
        capture.bytes.extend_from_slice(&chunk[..n.min(left)]);
        capture.truncated |= n > left;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputStream {
    Stdout,
    Stderr,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProcessEvent {
    Started {
        pid: u32,
    },
    Output {
        stream: OutputStream,
        bytes: Vec<u8>,
    },
}
#[derive(Clone)]
struct ProcessFailure {
    kind: io::ErrorKind,
    message: String,
}
impl From<io::Error> for ProcessFailure {
    fn from(error: io::Error) -> Self {
        Self {
            kind: error.kind(),
            message: error.to_string(),
        }
    }
}
/// Lifecycle handle for streaming callers such as the SSH launcher. Receiver lag
/// is explicit (`broadcast::error::RecvError::Lagged`), never silent success.
/// Dropping this owner aborts its operation and releases containment/IO guards.
pub struct ManagedProcess {
    cancel: Cancellation,
    task: Option<tokio::task::JoinHandle<Result<ProcessOutput, ProcessFailure>>>,
    cached: Option<Result<ProcessOutput, ProcessFailure>>,
}
enum InputCommand {
    Write(Vec<u8>, tokio::sync::oneshot::Sender<io::Result<()>>),
    Close(tokio::sync::oneshot::Sender<io::Result<()>>),
}
/// Writes are acknowledged after Tokio's write_all completes. On Windows this
/// can acknowledge a queued blocking write, before its OS write has completed.
#[derive(Clone)]
pub struct InputSender(tokio::sync::mpsc::Sender<InputCommand>);
impl InputSender {
    pub async fn write(&self, bytes: Vec<u8>) -> io::Result<()> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.0
            .send(InputCommand::Write(bytes, sender))
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "process input closed"))?;
        receiver
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "process input closed"))?
    }
    pub async fn close(&self) -> io::Result<()> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.0
            .send(InputCommand::Close(sender))
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "process input closed"))?;
        receiver
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "process input closed"))?
    }
}
impl ManagedProcess {
    pub fn spawn_interactive_owned_with_options(
        plan: ProcessPlan,
        event_capacity: usize,
        options: SpawnOptions,
    ) -> (Self, InputSender, broadcast::Receiver<ProcessEvent>) {
        let cancel = Cancellation::default();
        let (events, receiver) = broadcast::channel(event_capacity.max(1));
        let (input, input_receiver) = tokio::sync::mpsc::channel(16);
        let task_cancel = cancel.clone();
        let task = tokio::spawn(async move {
            run_observed(
                &plan,
                &task_cancel,
                Some(events),
                options,
                Some(input_receiver),
                None,
            )
            .await
            .map_err(ProcessFailure::from)
        });
        (
            Self {
                cancel,
                task: Some(task),
                cached: None,
            },
            InputSender(input),
            receiver,
        )
    }
    pub fn spawn_owned(
        plan: ProcessPlan,
        event_capacity: usize,
    ) -> (Self, broadcast::Receiver<ProcessEvent>) {
        Self::spawn_owned_with_options(plan, event_capacity, SpawnOptions::default())
    }
    pub fn spawn_owned_with_options(
        plan: ProcessPlan,
        event_capacity: usize,
        options: SpawnOptions,
    ) -> (Self, broadcast::Receiver<ProcessEvent>) {
        let cancel = Cancellation::default();
        let (events, receiver) = broadcast::channel(event_capacity.max(1));
        let task_cancel = cancel.clone();
        #[cfg(all(test, windows))]
        let test_assignment = windows_job::testing::current();
        let task = tokio::spawn(async move {
            #[cfg(all(test, windows))]
            if let Some(assignment) = test_assignment {
                return windows_job::testing::with_job_assignment(
                    assignment,
                    run_observed(&plan, &task_cancel, Some(events), options, None, None),
                )
                .await
                .map_err(ProcessFailure::from);
            }
            run_observed(&plan, &task_cancel, Some(events), options, None, None)
                .await
                .map_err(ProcessFailure::from)
        });
        (
            Self {
                cancel,
                task: Some(task),
                cached: None,
            },
            receiver,
        )
    }
    pub fn close(&self) {
        self.cancel.cancel();
    }
    /// Routes both child streams directly to a caller-held private file. The
    /// contained process owns these duplicated handles until exit and reap.
    pub fn spawn_owned_with_log_file(
        plan: ProcessPlan,
        event_capacity: usize,
        options: SpawnOptions,
        file: std::fs::File,
    ) -> (Self, broadcast::Receiver<ProcessEvent>) {
        let cancel = Cancellation::default();
        let (events, receiver) = broadcast::channel(event_capacity.max(1));
        let task_cancel = cancel.clone();
        let task = tokio::spawn(async move {
            run_observed(&plan, &task_cancel, Some(events), options, None, Some(file))
                .await
                .map_err(ProcessFailure::from)
        });
        (
            Self {
                cancel,
                task: Some(task),
                cached: None,
            },
            receiver,
        )
    }
    pub async fn wait(&mut self) -> io::Result<ProcessOutput> {
        if self.cached.is_none() {
            let result = match self
                .task
                .as_mut()
                .expect("uncached process has a task")
                .await
            {
                Ok(result) => result,
                Err(error) => Err(ProcessFailure {
                    kind: io::ErrorKind::Interrupted,
                    message: format!("owned process task ended: {error}"),
                }),
            };
            self.task = None;
            self.cached = Some(result);
        }
        self.cached
            .as_ref()
            .unwrap()
            .clone()
            .map_err(|error| io::Error::new(error.kind, error.message))
    }
}
impl Drop for ManagedProcess {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

/// Owns containment and IO tasks even if the outer future is dropped by a caller.
struct OwnedRunGuard {
    #[cfg(unix)]
    group: u32,
    tasks: Vec<tokio::task::AbortHandle>,
}
impl Drop for OwnedRunGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        terminate_group(self.group);
        for task in &self.tasks {
            task.abort();
        }
    }
}

pub async fn run_capped(plan: &ProcessPlan, cancel: &Cancellation) -> io::Result<ProcessOutput> {
    run_observed(plan, cancel, None, SpawnOptions::default(), None, None).await
}
pub async fn run_capped_with_options(
    plan: &ProcessPlan,
    cancel: &Cancellation,
    options: SpawnOptions,
) -> io::Result<ProcessOutput> {
    run_observed(plan, cancel, None, options, None, None).await
}
async fn run_observed(
    plan: &ProcessPlan,
    cancel: &Cancellation,
    events: Option<broadcast::Sender<ProcessEvent>>,
    options: SpawnOptions,
    mut interactive_input: Option<tokio::sync::mpsc::Receiver<InputCommand>>,
    log_file: Option<std::fs::File>,
) -> io::Result<ProcessOutput> {
    if cancel.is_cancelled() {
        return Ok(ProcessOutput {
            stdout: vec![],
            stderr: vec![],
            stdout_truncated: false,
            stderr_truncated: false,
            pipes_forced_closed: false,
            outcome: ExitOutcome::Cancelled,
        });
    }
    let deadline = if plan.timeout.is_zero() {
        None
    } else {
        Some(
            tokio::time::Instant::now()
                .checked_add(plan.timeout)
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "process deadline is outside the supported range",
                    )
                })?,
        )
    };
    let deadline_wait = async move {
        match deadline {
            Some(at) => tokio::time::sleep_until(at).await,
            None => std::future::pending::<()>().await,
        }
    };
    let mut cmd = Command::new(&plan.executable);
    cmd.args(&plan.args)
        .current_dir(&plan.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if options.combined_output && (options.stdout_null || options.stderr_null) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "combined output cannot use null output streams",
        ));
    }
    let combined_reader = if let Some(file) = log_file {
        if options.combined_output || options.stdout_null || options.stderr_null {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "direct log output cannot combine pipe or null output",
            ));
        }
        cmd.stdout(Stdio::from(file.try_clone()?));
        cmd.stderr(Stdio::from(file));
        None
    } else if options.combined_output {
        let (reader, writer) = io::pipe()?;
        cmd.stdout(Stdio::from(writer.try_clone()?));
        cmd.stderr(Stdio::from(writer));
        #[cfg(unix)]
        let reader = std::process::ChildStdout::from(std::os::fd::OwnedFd::from(reader));
        #[cfg(windows)]
        let reader =
            std::process::ChildStdout::from(std::os::windows::io::OwnedHandle::from(reader));
        Some(tokio::process::ChildStdout::from_std(reader)?)
    } else {
        if options.stdout_null {
            cmd.stdout(Stdio::null());
        }
        if options.stderr_null {
            cmd.stderr(Stdio::null());
        }
        None
    };
    if options.stdin_null {
        cmd.stdin(Stdio::null());
    }
    if options.env_clear {
        cmd.env_clear();
    }
    for (key, value) in &plan.env {
        if let Some(value) = value {
            cmd.env(key, value);
        } else {
            cmd.env_remove(key);
        }
    }
    #[cfg(unix)]
    {
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        // No child code may run before Job attachment; otherwise it can escape
        // containment by spawning descendants in the start/attach gap.
        let mut flags = windows_sys::Win32::System::Threading::CREATE_SUSPENDED;
        if options.no_window {
            flags |= windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
        }
        cmd.creation_flags(flags);
    }
    let mut child = cmd.spawn()?;
    // The command retains custom writer handles; release them so EOF depends
    // only on the owned child and its descendants.
    drop(cmd);
    if let Some(events) = &events {
        let _ = events.send(ProcessEvent::Started {
            pid: child.id().unwrap_or(0),
        });
    }
    #[cfg(unix)]
    let owned_pid = child
        .id()
        .ok_or_else(|| io::Error::other("missing child PID"))?;
    #[cfg(windows)]
    let _job = match windows_job::OwnedJob::attach(&child) {
        Ok(job) => job,
        Err(error) => {
            let _ = child.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
            return Err(error);
        }
    };
    #[cfg(windows)]
    {
        if let Err(error) = windows_job::resume_primary(&child) {
            _job.terminate();
            let _ = child.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
            return Err(error);
        }
    }
    let mut ownership = OwnedRunGuard {
        #[cfg(unix)]
        group: owned_pid,
        tasks: Vec::new(),
    };
    let out = Arc::new(Mutex::new(Capture::default()));
    let err = Arc::new(Mutex::new(Capture::default()));
    let stdout = combined_reader.or_else(|| child.stdout.take());
    let stderr = child.stderr.take();
    let stdout_capture = out.clone();
    let stderr_capture = err.clone();
    let cap = plan.output_cap;
    let stdout_events = events.clone().map(|sender| (sender, OutputStream::Stdout));
    let stderr_events = events.map(|sender| (sender, OutputStream::Stderr));
    let out_task = tokio::spawn(async move {
        if let Some(stdout) = stdout {
            read_capped(stdout, stdout_capture, cap, stdout_events).await?;
        }
        Ok::<(), io::Error>(())
    });
    let err_task = tokio::spawn(async move {
        if let Some(stderr) = stderr {
            read_capped(stderr, stderr_capture, cap, stderr_events).await?;
        }
        Ok::<(), io::Error>(())
    });
    let stdin = child.stdin.take();
    let prompt = plan.stdin.clone();
    let input_finished = Cancellation::default();
    let input_stop = input_finished.clone();
    let input_task = tokio::spawn(async move {
        if let Some(mut stdin) = stdin {
            stdin.write_all(&prompt).await?;
            if let Some(ref mut receiver) = interactive_input {
                loop {
                    let command = tokio::select! {
                        _ = input_stop.cancelled() => break,
                        command = receiver.recv() => command,
                    };
                    match command {
                        Some(InputCommand::Write(bytes, receipt)) => {
                            let result = tokio::select! {
                                _ = input_stop.cancelled() => Err(io::Error::new(io::ErrorKind::BrokenPipe, "process exited")),
                                result = stdin.write_all(&bytes) => result,
                            };
                            let failed = result.is_err();
                            let _ = receipt.send(result);
                            if failed {
                                break;
                            }
                        }
                        Some(InputCommand::Close(receipt)) => {
                            let _ = receipt.send(stdin.shutdown().await);
                            return Ok(());
                        }
                        None => break,
                    }
                }
            }
            stdin.shutdown().await?;
        }
        Ok::<(), io::Error>(())
    });
    ownership.tasks.extend([
        out_task.abort_handle(),
        err_task.abort_handle(),
        input_task.abort_handle(),
    ]);
    let mut forced = false;
    let outcome = tokio::select! {
        status=child.wait()=>{let status=status?;
            #[cfg(unix)] let signal={use std::os::unix::process::ExitStatusExt;status.signal()};
            #[cfg(not(unix))] let signal=None;
            ExitOutcome::Exited{code:status.code(),signal}
        },
        _=cancel.cancelled()=>ExitOutcome::Cancelled,
        _=deadline_wait=>ExitOutcome::TimedOut,
    };
    input_finished.cancel();
    if !matches!(outcome, ExitOutcome::Exited { .. }) {
        #[cfg(unix)]
        terminate_group(owned_pid);
        #[cfg(windows)]
        _job.terminate();
        let _ = child.start_kill();
        if tokio::time::timeout(Duration::from_secs(2), child.wait())
            .await
            .is_err()
        {
            forced = true;
        }
    }
    // Waiting for inherited pipes is always bounded, including normal direct-child exits.
    let abort_out = out_task.abort_handle();
    let abort_err = err_task.abort_handle();
    let abort_input = input_task.abort_handle();
    let mut drain = tokio::spawn(async move { tokio::join!(out_task, err_task, input_task) });
    ownership.tasks.push(drain.abort_handle());
    match tokio::time::timeout(plan.pipe_drain_timeout, &mut drain).await {
        Ok(joined) => {
            let (a, b, c) = joined.map_err(io::Error::other)?;
            for result in [a, b, c] {
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) if error.kind() == io::ErrorKind::BrokenPipe => {}
                    Ok(Err(error)) => return Err(error),
                    Err(error) => return Err(io::Error::other(error)),
                }
            }
        }
        Err(_) => {
            forced = true;
            #[cfg(unix)]
            terminate_group(owned_pid);
            #[cfg(windows)]
            _job.terminate();
            abort_out.abort();
            abort_err.abort();
            abort_input.abort();
            let _ = drain.await;
        }
    }
    let out = out
        .lock()
        .map_err(|_| io::Error::other("output lock poisoned"))?;
    let err = err
        .lock()
        .map_err(|_| io::Error::other("output lock poisoned"))?;
    Ok(ProcessOutput {
        stdout: out.bytes.clone(),
        stderr: err.bytes.clone(),
        stdout_truncated: out.truncated,
        stderr_truncated: err.truncated,
        pipes_forced_closed: forced,
        outcome,
    })
}
#[cfg(unix)]
fn terminate_group(pid: u32) {
    // SAFETY: only a just-created, owned process-group ID is used; no name-based matching.
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
}
#[cfg(windows)]
pub(crate) mod windows_job {
    #[cfg(test)]
    pub(crate) mod testing;

    use std::mem::size_of;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::{io, mem};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, DuplicateHandle, HANDLE},
        System::{JobObjects::*, Threading::GetCurrentProcess},
    };
    pub struct OwnedJob(OwnedHandle);
    impl OwnedJob {
        pub fn attach(child: &tokio::process::Child) -> io::Result<Self> {
            let raw = child
                .raw_handle()
                .ok_or_else(|| io::Error::other("missing child handle"))?;
            Self::attach_raw(raw)
        }
        /// Attach a still-owned, suspended child before any provider code runs.
        /// The caller retains the process handle and resumes only after success.
        pub(crate) fn attach_raw(raw: HANDLE) -> io::Result<Self> {
            let job = Self::create()?;
            job.assign(raw)?;
            Ok(job)
        }
        pub(crate) fn create() -> io::Result<Self> {
            unsafe {
                let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if job.is_null() {
                    return Err(io::Error::last_os_error());
                }
                let owned = Self(OwnedHandle::from_raw_handle(job));
                let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = mem::zeroed();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                if SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    &limits as *const _ as _,
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                ) == 0
                {
                    return Err(io::Error::last_os_error());
                }
                Ok(owned)
            }
        }
        pub(crate) fn assign(&self, process: HANDLE) -> io::Result<()> {
            // Caller retains its suspended process until assignment succeeds.
            #[cfg(test)]
            if let Some(assignment) = testing::current() {
                return assignment(self.as_raw_handle(), process);
            }
            if unsafe { AssignProcessToJobObject(self.as_raw_handle(), process) } == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }
        pub(crate) fn as_raw_handle(&self) -> HANDLE {
            self.0.as_raw_handle()
        }
        pub(crate) fn inherited_query_handle(&self) -> io::Result<OwnedHandle> {
            // JOB_OBJECT_QUERY (Win32 SystemServices): the child can validate
            // membership/limits and retain lifetime, but cannot change or kill
            // this Job through its inherited handle. No wait access is needed.
            const JOB_OBJECT_QUERY_ACCESS: u32 = 0x0004;
            let mut raw = std::ptr::null_mut();
            if unsafe {
                DuplicateHandle(
                    GetCurrentProcess(),
                    self.as_raw_handle(),
                    GetCurrentProcess(),
                    &mut raw,
                    JOB_OBJECT_QUERY_ACCESS,
                    1,
                    0,
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(unsafe { OwnedHandle::from_raw_handle(raw) })
        }
        pub fn terminate(&self) {
            unsafe {
                TerminateJobObject(self.0.as_raw_handle(), 1);
            }
        }
    }
    /// A CREATE_SUSPENDED child has only its primary thread until resumed.
    /// Enumeration is filtered to our still-owned child PID, and the thread is
    /// resumed only after the process is attached to the kill-on-close Job.
    pub fn resume_primary(child: &tokio::process::Child) -> io::Result<()> {
        use windows_sys::Win32::{
            Foundation::INVALID_HANDLE_VALUE,
            System::{
                Diagnostics::ToolHelp::*,
                Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME},
            },
        };
        let pid = child
            .id()
            .ok_or_else(|| io::Error::other("missing suspended child PID"))?;
        // SAFETY: snapshot/thread handles are closed on every path; only our own
        // suspended child's primary thread is selected.
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
            if snapshot == INVALID_HANDLE_VALUE {
                return Err(io::Error::last_os_error());
            }
            let mut entry: THREADENTRY32 = mem::zeroed();
            entry.dwSize = size_of::<THREADENTRY32>() as u32;
            let mut more = Thread32First(snapshot, &mut entry);
            while more != 0 {
                if entry.th32OwnerProcessID == pid {
                    let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                    CloseHandle(snapshot);
                    if thread.is_null() {
                        return Err(io::Error::last_os_error());
                    }
                    let previous = ResumeThread(thread);
                    let error = io::Error::last_os_error();
                    CloseHandle(thread);
                    return if previous == 1 {
                        Ok(())
                    } else if previous == u32::MAX {
                        Err(error)
                    } else {
                        Err(io::Error::other("unexpected child suspension state"))
                    };
                }
                more = Thread32Next(snapshot, &mut entry);
            }
            CloseHandle(snapshot);
            Err(io::Error::other("suspended child primary thread not found"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn stdio_plan(unix_script: &str, windows_script: &str) -> ProcessPlan {
        #[cfg(unix)]
        let (executable, args) = ("/bin/sh", vec!["-c".into(), unix_script.into()]);
        #[cfg(windows)]
        let (executable, args) = (
            "pwsh",
            vec![
                "-NoProfile".into(),
                "-Command".into(),
                windows_script.into(),
            ],
        );
        #[cfg(unix)]
        let _ = windows_script;
        #[cfg(windows)]
        let _ = unix_script;
        ProcessPlan {
            executable: executable.into(),
            args,
            cwd: std::env::temp_dir(),
            env: BTreeMap::new(),
            stdin: Vec::new(),
            timeout: Duration::from_secs(10),
            output_cap: 1024,
            pipe_drain_timeout: Duration::from_secs(1),
        }
    }
    #[tokio::test]
    async fn direct_log_keeps_both_streams_beyond_capture_cap() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("synthetic-server.log");
        let file = std::fs::File::create(&path).unwrap();
        let mut plan = stdio_plan(
            "printf stdout-payload; printf stderr-payload >&2",
            "[Console]::Out.Write('stdout-payload'); [Console]::Error.Write('stderr-payload')",
        );
        plan.output_cap = 1;
        let (mut owner, _) =
            ManagedProcess::spawn_owned_with_log_file(plan, 1, SpawnOptions::default(), file);
        let output = owner.wait().await.unwrap();
        assert!(matches!(
            output.outcome,
            ExitOutcome::Exited { code: Some(0), .. }
        ));
        assert!(output.stdout.is_empty() && output.stderr.is_empty());
        let bytes = std::fs::read_to_string(path).unwrap();
        assert!(bytes.contains("stdout-payload"));
        assert!(bytes.contains("stderr-payload"));
        assert_eq!(bytes.len(), 28);
    }
    #[tokio::test]
    async fn interactive_input_waits_for_each_reply_and_reaps_with_sender_alive() {
        let plan = stdio_plan(
            "read first; printf 'reply-one\\n'; read second; printf 'reply-two\\n'",
            "$a=[Console]::In.ReadLine(); [Console]::Out.WriteLine('reply-one'); $b=[Console]::In.ReadLine(); [Console]::Out.WriteLine('reply-two')",
        );
        let (mut owner, input, mut events) =
            ManagedProcess::spawn_interactive_owned_with_options(plan, 32, SpawnOptions::default());
        input.write(b"initialize\n".to_vec()).await.unwrap();
        let mut first = Vec::new();
        tokio::time::timeout(Duration::from_secs(5), async {
            while !first.contains(&b'\n') {
                if let ProcessEvent::Output {
                    stream: OutputStream::Stdout,
                    bytes,
                } = events.recv().await.unwrap()
                {
                    first.extend(bytes);
                }
            }
        })
        .await
        .unwrap();
        assert!(String::from_utf8_lossy(&first).contains("reply-one"));
        assert!(!owner.task.as_ref().unwrap().is_finished());
        input.write(b"account/read\n".to_vec()).await.unwrap();
        let output = owner.wait().await.unwrap();
        assert!(String::from_utf8_lossy(&output.stdout).contains("reply-two"));
        assert!(!output.pipes_forced_closed);
        assert!(input.write(b"late\n".to_vec()).await.is_err());
    }
    #[tokio::test]
    async fn combined_native_pipe_preserves_alternating_writes_and_eof() {
        let plan = stdio_plan(
            "printf first; printf second >&2; printf third; printf fourth >&2",
            "[Console]::Out.Write('first'); [Console]::Error.Write('second'); [Console]::Out.Write('third'); [Console]::Error.Write('fourth')",
        );
        let (mut owner, _) = ManagedProcess::spawn_owned_with_options(
            plan,
            16,
            SpawnOptions {
                combined_output: true,
                stdin_null: true,
                ..Default::default()
            },
        );
        let output = owner.wait().await.unwrap();
        assert_eq!(output.stdout, b"firstsecondthirdfourth");
        assert!(output.stderr.is_empty());
        assert!(!output.pipes_forced_closed);
        assert!(matches!(
            output.outcome,
            ExitOutcome::Exited { code: Some(0), .. }
        ));
    }
    #[tokio::test]
    async fn null_streams_close_stdin_and_discard_output() {
        let plan = stdio_plan(
            "if read line; then exit 9; fi; printf discarded; printf discarded >&2",
            "if ($null -ne [Console]::In.ReadLine()) { exit 9 }; [Console]::Out.Write('discarded'); [Console]::Error.Write('discarded')",
        );
        let (mut owner, _) = ManagedProcess::spawn_owned_with_options(
            plan,
            16,
            SpawnOptions {
                stdin_null: true,
                stdout_null: true,
                stderr_null: true,
                ..Default::default()
            },
        );
        let output = owner.wait().await.unwrap();
        assert!(output.stdout.is_empty() && output.stderr.is_empty());
        assert!(matches!(
            output.outcome,
            ExitOutcome::Exited { code: Some(0), .. }
        ));
    }
    #[tokio::test]
    async fn combined_pipe_retains_output_cap_and_cancellation() {
        let mut plan = stdio_plan(
            "printf abcdef >&2; sleep 30",
            "[Console]::Error.Write('abcdef'); Start-Sleep 30",
        );
        plan.output_cap = 3;
        let (mut owner, mut events) = ManagedProcess::spawn_owned_with_options(
            plan,
            16,
            SpawnOptions {
                combined_output: true,
                stdin_null: true,
                ..Default::default()
            },
        );
        loop {
            if matches!(events.recv().await.unwrap(), ProcessEvent::Output { .. }) {
                break;
            }
        }
        owner.close();
        let output = tokio::time::timeout(Duration::from_secs(4), owner.wait())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(output.stdout, b"abc");
        assert!(output.stdout_truncated);
        assert_eq!(output.outcome, ExitOutcome::Cancelled);
    }
    #[test]
    fn process_future_can_be_owned_by_a_multithread_runtime() {
        fn requires_send<T: Send>(_: T) {}
        let plan = ProcessPlan {
            executable: "synthetic-not-executed".into(),
            args: vec![],
            cwd: std::env::temp_dir(),
            env: BTreeMap::new(),
            stdin: vec![],
            timeout: Duration::from_secs(1),
            output_cap: 64,
            pipe_drain_timeout: Duration::from_secs(1),
        };
        let cancellation = Cancellation::default();
        requires_send(run_capped(&plan, &cancellation));
    }
    #[tokio::test]
    async fn cancel_before_subscribe_is_not_lost() {
        let c = Cancellation::default();
        c.cancel();
        c.cancel();
        tokio::time::timeout(Duration::from_millis(20), c.cancelled())
            .await
            .unwrap();
    }
    #[cfg(unix)]
    fn plan(script: &str) -> ProcessPlan {
        ProcessPlan {
            executable: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            cwd: std::env::temp_dir(),
            env: BTreeMap::new(),
            stdin: vec![],
            timeout: Duration::from_millis(100),
            output_cap: 64,
            pipe_drain_timeout: Duration::from_millis(100),
        }
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn actual_child_receives_environment_and_exit() {
        let mut p = plan(
            "printf '%s' \"$MANY_SYNTHETIC\"; printf '%s' \"${MANY_REMOVED-unset}\" >&2; exit 7",
        );
        p.env
            .insert("MANY_SYNTHETIC".into(), Some("日本語 two words".into()));
        p.env.insert("MANY_REMOVED".into(), None);
        let o = run_capped(&p, &Cancellation::default()).await.unwrap();
        assert_eq!(o.stdout, "日本語 two words".as_bytes());
        assert_eq!(o.stderr, b"unset");
        assert_eq!(
            o.outcome,
            ExitOutcome::Exited {
                code: Some(7),
                signal: None
            }
        );
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn timeout_closes_descendant_pipes() {
        let now = std::time::Instant::now();
        let o = run_capped(&plan("sleep 30 & wait"), &Cancellation::default())
            .await
            .unwrap();
        assert_eq!(o.outcome, ExitOutcome::TimedOut);
        assert!(now.elapsed() < Duration::from_secs(3));
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn normal_exit_with_descendant_pipe_is_bounded() {
        let now = std::time::Instant::now();
        let mut p = plan("sleep 30 & exit 0");
        p.timeout = Duration::from_secs(1);
        let o = run_capped(&p, &Cancellation::default()).await.unwrap();
        assert_eq!(
            o.outcome,
            ExitOutcome::Exited {
                code: Some(0),
                signal: None
            }
        );
        assert!(o.pipes_forced_closed);
        assert!(now.elapsed() < Duration::from_secs(3));
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn output_cap_does_not_block_prompt_write() {
        let mut p = plan(
            "i=0; while [ $i -lt 200 ]; do printf 'synthetic-output'; i=$((i+1)); done; cat >/dev/null",
        );
        p.timeout = Duration::from_secs(2);
        p.stdin = vec![b'x'; 256 * 1024];
        let o = run_capped(&p, &Cancellation::default()).await.unwrap();
        assert!(o.stdout_truncated);
        assert_eq!(o.stdout.len(), 64);
        assert_eq!(
            o.outcome,
            ExitOutcome::Exited {
                code: Some(0),
                signal: None
            }
        );
    }
}

#[cfg(all(test, unix))]
mod ownership_tests {
    use super::*;
    fn alive(pid: i32) -> bool {
        // A killed orphan may remain a zombie until its new parent reaps it.
        #[cfg(target_os = "linux")]
        if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat"))
            && stat
                .rsplit_once(") ")
                .is_some_and(|(_, tail)| tail.starts_with('Z'))
        {
            return false;
        }
        unsafe { libc::kill(pid, 0) == 0 }
    }
    fn plan(file: &std::path::Path, normal_exit: bool) -> ProcessPlan {
        let script = if normal_exit {
            "sleep 30 </dev/null >/dev/null 2>&1 & echo $! > \"$1\"; exit 0"
        } else {
            "sleep 30 </dev/null >/dev/null 2>&1 & echo $! > \"$1\"; wait"
        };
        ProcessPlan {
            executable: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                script.into(),
                "synthetic".into(),
                file.as_os_str().into(),
            ],
            cwd: file.parent().unwrap().into(),
            env: BTreeMap::new(),
            stdin: vec![],
            timeout: Duration::from_secs(10),
            output_cap: 64,
            pipe_drain_timeout: Duration::from_millis(100),
        }
    }
    async fn assert_owned_child_gone(file: &std::path::Path) {
        let pid = std::fs::read_to_string(file)
            .unwrap()
            .trim()
            .parse::<i32>()
            .unwrap();
        for _ in 0..100 {
            if !alive(pid) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        // Test cleanup is scoped to the exact child just created, never a name.
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
        panic!("owned synthetic descendant survived cleanup");
    }
    #[tokio::test]
    async fn dropped_owner_future_kills_descendant_and_aborts_io() {
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("child.pid");
        let p = plan(&f, false);
        let c = Cancellation::default();
        assert!(
            tokio::time::timeout(Duration::from_millis(200), run_capped(&p, &c))
                .await
                .is_err()
        );
        assert_owned_child_gone(&f).await;
    }
    #[tokio::test]
    async fn normal_exit_kills_detached_stdio_descendant() {
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("child.pid");
        let p = plan(&f, true);
        let out = run_capped(&p, &Cancellation::default()).await.unwrap();
        assert_eq!(
            out.outcome,
            ExitOutcome::Exited {
                code: Some(0),
                signal: None
            }
        );
        assert_owned_child_gone(&f).await;
    }
}

#[cfg(all(test, unix))]
mod streaming_tests {
    use super::*;
    fn plan(script: &str) -> ProcessPlan {
        ProcessPlan {
            executable: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            cwd: std::env::temp_dir(),
            env: BTreeMap::new(),
            stdin: vec![],
            timeout: Duration::from_secs(2),
            output_cap: 1024,
            pipe_drain_timeout: Duration::from_millis(100),
        }
    }
    #[tokio::test]
    async fn streaming_precedes_exit_and_wait_is_idempotent() {
        let directory = tempfile::tempdir().unwrap();
        let release = directory.path().join("release");
        let mut operation = plan(
            "printf synthetic-ready; while [ ! -f \"$1\" ]; do sleep 0.01; done; printf synthetic-done",
        );
        operation
            .args
            .extend(["synthetic".into(), release.as_os_str().into()]);
        operation.timeout = Duration::from_secs(5);
        let (mut process, mut events) = ManagedProcess::spawn_owned(operation, 16);
        assert!(matches!(events.recv().await.unwrap(),ProcessEvent::Started{pid} if pid>0));
        let first = tokio::time::timeout(Duration::from_secs(3), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            first,
            ProcessEvent::Output {
                stream: OutputStream::Stdout,
                bytes: b"synthetic-ready".to_vec()
            }
        );
        assert!(!process.task.as_ref().unwrap().is_finished());
        std::fs::write(release, b"continue").unwrap();
        let output = process.wait().await.unwrap();
        assert_eq!(output.stdout, b"synthetic-readysynthetic-done");
        assert_eq!(process.wait().await.unwrap(), output);
    }
    #[tokio::test]
    async fn close_and_start_failure_can_be_observed_twice() {
        let (mut process, _) = ManagedProcess::spawn_owned(plan("sleep 30"), 2);
        process.close();
        process.close();
        assert_eq!(
            process.wait().await.unwrap().outcome,
            ExitOutcome::Cancelled
        );
        assert_eq!(
            process.wait().await.unwrap().outcome,
            ExitOutcome::Cancelled
        );
        let mut missing = plan("");
        missing.executable = "/synthetic-not-present/executable".into();
        let (mut process, _) = ManagedProcess::spawn_owned(missing, 2);
        assert!(process.wait().await.is_err());
        assert!(process.wait().await.is_err());
    }
}

#[cfg(all(test, unix))]
mod deadline_tests {
    use super::*;
    fn plan(script: &str, timeout: Duration) -> ProcessPlan {
        ProcessPlan {
            executable: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            cwd: std::env::temp_dir(),
            env: BTreeMap::new(),
            stdin: vec![],
            timeout,
            output_cap: 1024,
            pipe_drain_timeout: Duration::from_millis(100),
        }
    }
    #[tokio::test]
    async fn zero_deadline_waits_for_exit_but_can_be_cancelled() {
        let output = run_capped(
            &plan("sleep 0.02; printf done", Duration::ZERO),
            &Cancellation::default(),
        )
        .await
        .unwrap();
        assert_eq!(output.stdout, b"done");
        assert!(matches!(
            output.outcome,
            ExitOutcome::Exited { code: Some(0), .. }
        ));
        let (mut owner, mut events) =
            ManagedProcess::spawn_owned(plan("sleep 30", Duration::ZERO), 4);
        assert!(matches!(
            events.recv().await.unwrap(),
            ProcessEvent::Started { .. }
        ));
        owner.close();
        let output = tokio::time::timeout(Duration::from_secs(2), owner.wait())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(output.outcome, ExitOutcome::Cancelled);
    }
    #[tokio::test]
    async fn impossible_deadline_fails_before_spawn_without_panicking() {
        let error = run_capped(&plan("exit 0", Duration::MAX), &Cancellation::default())
            .await
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }
}
