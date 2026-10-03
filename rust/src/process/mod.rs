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
    sync::watch,
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
) -> io::Result<()> {
    let mut chunk = [0u8; 8192];
    loop {
        let n = reader.read(&mut chunk).await?;
        if n == 0 {
            return Ok(());
        }
        let mut capture = buffer
            .lock()
            .map_err(|_| io::Error::other("output lock poisoned"))?;
        let left = cap.saturating_sub(capture.bytes.len());
        capture.bytes.extend_from_slice(&chunk[..n.min(left)]);
        capture.truncated |= n > left;
    }
}

pub async fn run_capped(plan: &ProcessPlan, cancel: &Cancellation) -> io::Result<ProcessOutput> {
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
    let mut child = cmd.spawn()?;
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
    let out = Arc::new(Mutex::new(Capture::default()));
    let err = Arc::new(Mutex::new(Capture::default()));
    let out_task = tokio::spawn(read_capped(
        child.stdout.take().unwrap(),
        out.clone(),
        plan.output_cap,
    ));
    let err_task = tokio::spawn(read_capped(
        child.stderr.take().unwrap(),
        err.clone(),
        plan.output_cap,
    ));
    let mut stdin = child.stdin.take().unwrap();
    let prompt = plan.stdin.clone();
    let input_task = tokio::spawn(async move {
        stdin.write_all(&prompt).await?;
        stdin.shutdown().await
    });
    let mut forced = false;
    let outcome = tokio::select! {
        status=child.wait()=>{let status=status?;
            #[cfg(unix)] let signal={use std::os::unix::process::ExitStatusExt;status.signal()};
            #[cfg(not(unix))] let signal=None;
            ExitOutcome::Exited{code:status.code(),signal}
        },
        _=cancel.cancelled()=>ExitOutcome::Cancelled,
        _=tokio::time::sleep(plan.timeout)=>ExitOutcome::TimedOut,
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
    use std::{io, mem};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::JobObjects::*,
    };
    pub struct OwnedJob(HANDLE);
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
                let owned = Self(job);
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
                TerminateJobObject(self.0, 1);
            }
        }
    }
    impl Drop for OwnedJob {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
