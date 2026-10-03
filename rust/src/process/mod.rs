//! Owned, cancellable subprocesses. Output readers start before prompt writes.
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
impl ManagedProcess {
    pub fn spawn_owned(
        plan: ProcessPlan,
        event_capacity: usize,
    ) -> (Self, broadcast::Receiver<ProcessEvent>) {
        let cancel = Cancellation::default();
        let (events, receiver) = broadcast::channel(event_capacity.max(1));
        let task_cancel = cancel.clone();
        let task = tokio::spawn(async move {
            run_observed(&plan, &task_cancel, Some(events))
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
    run_observed(plan, cancel, None).await
}
async fn run_observed(
    plan: &ProcessPlan,
    cancel: &Cancellation,
    events: Option<broadcast::Sender<ProcessEvent>>,
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
        cmd.creation_flags(windows_sys::Win32::System::Threading::CREATE_SUSPENDED);
    }
    let mut child = cmd.spawn()?;
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
    let out_task = tokio::spawn(read_capped(
        child.stdout.take().unwrap(),
        out.clone(),
        plan.output_cap,
        events.clone().map(|sender| (sender, OutputStream::Stdout)),
    ));
    let err_task = tokio::spawn(read_capped(
        child.stderr.take().unwrap(),
        err.clone(),
        plan.output_cap,
        events.map(|sender| (sender, OutputStream::Stderr)),
    ));
    let mut stdin = child.stdin.take().unwrap();
    let prompt = plan.stdin.clone();
    let input_task = tokio::spawn(async move {
        stdin.write_all(&prompt).await?;
        stdin.shutdown().await
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
mod windows_job {
    use std::mem::size_of;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::{io, mem};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::JobObjects::*,
    };
    pub struct OwnedJob(OwnedHandle);
    impl OwnedJob {
        pub fn attach(child: &tokio::process::Child) -> io::Result<Self> {
            unsafe {
                let raw = child
                    .raw_handle()
                    .ok_or_else(|| io::Error::other("missing child handle"))?;
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
                    || AssignProcessToJobObject(job, raw as HANDLE) == 0
                {
                    return Err(io::Error::last_os_error());
                }
                Ok(owned)
            }
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
