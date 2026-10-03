//! Native ConPTY: create suspended, assign the shared kill-on-close Job, then
//! resume the retained primary thread. portable-pty's post-spawn handle cannot
//! establish this invariant. IO is serviced on separate cancellable threads.
//!
//! Native Windows acceptance is required on pre-24H2 and current Windows:
//! https://learn.microsoft.com/windows/console/creating-a-pseudoconsole-session
//! https://learn.microsoft.com/windows/console/closepseudoconsole
use super::{
    Cancellation, ProcessPlan,
    pty::{CLOSE_GRACE, PtyExit, PtySession, PtySize, REAP_GRACE},
    windows_job::OwnedJob,
};
use crate::proto::core::{CoreFuture, SPAWN_PROOF_ENV};
use std::{
    collections::{BTreeMap, VecDeque},
    ffi::{OsStr, OsString},
    io, mem,
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    ptr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
use tokio::sync::{mpsc, oneshot, watch};
use windows_sys::Win32::{
    Foundation::*,
    Storage::FileSystem::{ReadFile, WriteFile},
    System::{Console::*, IO::CancelSynchronousIo, Pipes::CreatePipe, Threading::*},
};

fn wide(s: &OsStr) -> io::Result<Vec<u16>> {
    let mut value: Vec<_> = s.encode_wide().collect();
    if value.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "NUL in PTY process argument",
        ));
    }
    value.push(0);
    Ok(value)
}
fn quote(arg: &OsStr) -> io::Result<Vec<u16>> {
    let arg = wide(arg)?;
    let arg = &arg[..arg.len() - 1];
    if arg.is_empty() {
        return Ok(vec![b'"' as u16, b'"' as u16]);
    }
    // Go syscall.EscapeArg: enclosing quotes only when spaces/tabs require
    // them. This distinction matters for the cmd.exe /c npm-shim fallback.
    let quoted = arg.iter().any(|ch| matches!(*ch, 32 | 9));
    let mut out = Vec::new();
    if quoted {
        out.push(b'"' as u16);
    }
    let mut slashes = 0;
    for ch in arg {
        if *ch == b'\\' as u16 {
            slashes += 1;
            continue;
        }
        if *ch == b'"' as u16 {
            out.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2 + 1));
        } else {
            out.extend(std::iter::repeat_n(b'\\' as u16, slashes));
        }
        slashes = 0;
        out.push(*ch);
    }
    out.extend(std::iter::repeat_n(
        b'\\' as u16,
        slashes * if quoted { 2 } else { 1 },
    ));
    if quoted {
        out.push(b'"' as u16);
    }
    Ok(out)
}

fn environment(plan: &ProcessPlan) -> io::Result<Vec<u16>> {
    let mut values: BTreeMap<String, (OsString, OsString)> = std::env::vars_os()
        .map(|(k, v)| (k.to_string_lossy().to_uppercase(), (k, v)))
        .collect();
    for (key, value) in &plan.env {
        let normalized = key.to_string_lossy().to_uppercase();
        if let Some(value) = value {
            values.insert(normalized, (key.clone(), value.clone()));
        } else {
            values.remove(&normalized);
        }
    }
    values.remove(SPAWN_PROOF_ENV);
    let mut block = Vec::new();
    for (_, (key, value)) in values {
        let mut entry = key;
        entry.push("=");
        entry.push(value);
        block.extend(wide(&entry)?);
    }
    block.push(0);
    if block.len() == 1 {
        block.push(0);
    }
    Ok(block)
}
fn pipe() -> io::Result<(OwnedHandle, OwnedHandle)> {
    let (mut read, mut write) = (ptr::null_mut(), ptr::null_mut());
    if unsafe { CreatePipe(&mut read, &mut write, ptr::null(), 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe {
        (
            OwnedHandle::from_raw_handle(read),
            OwnedHandle::from_raw_handle(write),
        )
    })
}
struct Console(HPCON);
impl Drop for Console {
    fn drop(&mut self) {
        unsafe {
            ClosePseudoConsole(self.0);
        }
    }
}
struct Attributes {
    storage: Vec<usize>,
    initialized: bool,
}
impl Attributes {
    fn new(console: HPCON) -> io::Result<Self> {
        let mut count = 0;
        unsafe {
            InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut count);
        }
        if count == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut list = Self {
            storage: vec![0; count.div_ceil(mem::size_of::<usize>())],
            initialized: false,
        };
        if unsafe { InitializeProcThreadAttributeList(list.as_mut(), 1, 0, &mut count) } == 0 {
            return Err(io::Error::last_os_error());
        }
        list.initialized = true;
        if unsafe {
            UpdateProcThreadAttribute(
                list.as_mut(),
                0,
                PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
                console as *const _,
                mem::size_of::<HPCON>(),
                ptr::null_mut(),
                ptr::null(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(list)
    }
    fn as_mut(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.storage.as_mut_ptr().cast()
    }
}
impl Drop for Attributes {
    fn drop(&mut self) {
        if self.initialized {
            unsafe {
                DeleteProcThreadAttributeList(self.as_mut());
            }
        }
    }
}
#[derive(Clone)]
struct Failure(io::ErrorKind, String);
type ExitResult = Result<PtyExit, Failure>;
struct ReadState {
    receiver: mpsc::Receiver<io::Result<Vec<u8>>>,
    pending: VecDeque<u8>,
}
struct WriteRequest {
    bytes: Vec<u8>,
    result: oneshot::Sender<io::Result<usize>>,
}
struct PipeThreads {
    reader: Option<thread::JoinHandle<()>>,
    writer: Option<thread::JoinHandle<()>>,
    stop: Arc<AtomicBool>,
    writer_stop: Arc<AtomicBool>,
}
impl PipeThreads {
    fn cancel(&self) {
        self.stop.store(true, Ordering::Release);
        self.writer_stop.store(true, Ordering::Release);
        for task in [&self.reader, &self.writer].into_iter().flatten() {
            // Cancellation is requested repeatedly until actual thread completion;
            // ERROR_NOT_FOUND is not evidence that a later read cannot start.
            unsafe {
                CancelSynchronousIo(task.as_raw_handle());
            }
        }
    }
    fn done(&self) -> bool {
        [&self.reader, &self.writer]
            .into_iter()
            .flatten()
            .all(|task| task.is_finished())
    }
    fn join_finished(&mut self) {
        for task in [&mut self.reader, &mut self.writer] {
            if task.as_ref().is_some_and(|task| task.is_finished()) {
                let _ = task.take().unwrap().join();
            }
        }
    }
}
impl Drop for PipeThreads {
    fn drop(&mut self) {
        self.cancel();
        self.join_finished();
    }
}
struct NativePty {
    pid: u32,
    console: Arc<Mutex<Option<Console>>>,
    reads: tokio::sync::Mutex<ReadState>,
    writes: mpsc::Sender<WriteRequest>,
    closing: Cancellation,
    result: watch::Receiver<Option<ExitResult>>,
}
fn start_reader(
    handle: OwnedHandle,
    tx: mpsc::Sender<io::Result<Vec<u8>>>,
    stop: Arc<AtomicBool>,
) -> io::Result<thread::JoinHandle<()>> {
    thread::Builder::new().name("many-ai-pty-read".into()).spawn(move || {
        while !stop.load(Ordering::Acquire) {
            let mut bytes = vec![0;4096]; let mut count = 0;
            let ok = unsafe { ReadFile(handle.as_raw_handle(), bytes.as_mut_ptr(), bytes.len() as u32, &mut count, ptr::null_mut()) };
            let mut item = if ok == 0 {
                let error = io::Error::last_os_error();
                if matches!(error.raw_os_error(), Some(code) if code == ERROR_BROKEN_PIPE as i32 || code == ERROR_OPERATION_ABORTED as i32) { break; }
                Err(error)
            } else if count == 0 { break; } else { bytes.truncate(count as usize); Ok(bytes) };
            let failed = item.is_err();
            loop {
                match tx.try_send(item) {
                    Ok(()) => break,
                    Err(mpsc::error::TrySendError::Closed(_)) => return,
                    Err(mpsc::error::TrySendError::Full(value)) => item = value,
                }
                if stop.load(Ordering::Acquire) { return; }
                thread::sleep(Duration::from_millis(1));
            }
            if failed { break; }
        }
    })
}
fn start_writer(
    handle: OwnedHandle,
    mut rx: mpsc::Receiver<WriteRequest>,
    stop: Arc<AtomicBool>,
) -> io::Result<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name("many-ai-pty-write".into())
        .spawn(move || {
            while !stop.load(Ordering::Acquire) {
                let request = match rx.try_recv() {
                    Ok(request) => request,
                    Err(mpsc::error::TryRecvError::Disconnected) => break,
                    Err(mpsc::error::TryRecvError::Empty) => {
                        thread::sleep(Duration::from_millis(1));
                        continue;
                    }
                };
                let mut written = 0;
                let ok = unsafe {
                    WriteFile(
                        handle.as_raw_handle(),
                        request.bytes.as_ptr(),
                        request.bytes.len().min(u32::MAX as usize) as u32,
                        &mut written,
                        ptr::null_mut(),
                    )
                };
                let result = if ok != 0 {
                    Ok(written as usize)
                } else {
                    Err(io::Error::last_os_error())
                };
                let _ = request.result.send(result);
            }
        })
}
struct OwnedChild {
    process: OwnedHandle,
    job: OwnedJob,
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        self.job.terminate();
    }
}
pub fn spawn(plan: &ProcessPlan, dimensions: PtySize) -> io::Result<Arc<dyn PtySession>> {
    spawn_with_job(plan, dimensions, OwnedJob::attach_raw)
}
fn spawn_with_job(
    plan: &ProcessPlan,
    dimensions: PtySize,
    attach: impl FnOnce(HANDLE) -> io::Result<OwnedJob>,
) -> io::Result<Arc<dyn PtySession>> {
    let executable = wide(plan.executable.as_os_str())?;
    let cwd = wide(plan.cwd.as_os_str())?;
    let mut command = quote(plan.executable.as_os_str())?;
    for arg in &plan.args {
        command.push(b' ' as u16);
        command.extend(quote(arg)?);
    }
    command.push(0);
    if command.len() > 32767 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "PTY command exceeds Windows command-line limit",
        ));
    }
    let environment = environment(plan)?;
    let (input_read, input_write) = pipe()?;
    let (output_read, output_write) = pipe()?;
    let mut raw_console = 0;
    let hr = unsafe {
        CreatePseudoConsole(
            COORD {
                X: dimensions.cols as i16,
                Y: dimensions.rows as i16,
            },
            input_read.as_raw_handle(),
            output_write.as_raw_handle(),
            0,
            &mut raw_console,
        )
    };
    if hr < 0 {
        return Err(io::Error::other(format!(
            "CreatePseudoConsole failed: {hr:#x}"
        )));
    }
    let console = Console(raw_console);
    let mut attributes = Attributes::new(raw_console)?;
    let mut startup: STARTUPINFOEXW = unsafe { mem::zeroed() };
    startup.StartupInfo.cb = mem::size_of::<STARTUPINFOEXW>() as u32;
    startup.lpAttributeList = attributes.as_mut();
    let mut info: PROCESS_INFORMATION = unsafe { mem::zeroed() };
    let ok = unsafe {
        CreateProcessW(
            executable.as_ptr(),
            command.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            0,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT | CREATE_SUSPENDED,
            environment.as_ptr().cast(),
            cwd.as_ptr(),
            &startup.StartupInfo,
            &mut info,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let process = unsafe { OwnedHandle::from_raw_handle(info.hProcess) };
    let primary = unsafe { OwnedHandle::from_raw_handle(info.hThread) };
    let job = match attach(process.as_raw_handle()) {
        Ok(job) => job,
        Err(error) => {
            unsafe {
                TerminateProcess(process.as_raw_handle(), 1);
                WaitForSingleObject(process.as_raw_handle(), 2000);
            }
            return Err(error);
        }
    };
    let owner = OwnedChild { process, job };
    drop(input_read);
    drop(output_write);
    // Start both IO channels while the child is still suspended. Any thread
    // setup failure drops the Job before a single provider instruction executes.
    let stop = Arc::new(AtomicBool::new(false));
    let writer_stop = Arc::new(AtomicBool::new(false));
    let (read_tx, read_rx) = mpsc::channel(1024);
    let (write_tx, write_rx) = mpsc::channel(1);
    let reader = start_reader(output_read, read_tx, stop.clone())?;
    let mut pipes = PipeThreads {
        reader: Some(reader),
        writer: None,
        stop: stop.clone(),
        writer_stop: writer_stop.clone(),
    };
    pipes.writer = Some(start_writer(input_write, write_rx, writer_stop)?);
    if unsafe { ResumeThread(primary.as_raw_handle()) } != 1 {
        owner.job.terminate();
        pipes.cancel();
        return Err(io::Error::other("cannot resume Job-contained PTY child"));
    }
    drop(primary);
    let console = Arc::new(Mutex::new(Some(console)));
    let closing = Cancellation::default();
    let closed = closing.clone();
    let owned_console = console.clone();
    let (tx, result) = watch::channel(None);
    tokio::spawn(async move {
        let owner = owner;
        let mut close_task = None;
        let mut close_at = None;
        let mut exit = None;
        loop {
            let waited = unsafe { WaitForSingleObject(owner.process.as_raw_handle(), 0) };
            if waited == WAIT_OBJECT_0 && exit.is_none() {
                let mut code = 0;
                exit = Some(
                    if unsafe { GetExitCodeProcess(owner.process.as_raw_handle(), &mut code) } != 0
                    {
                        Ok(PtyExit {
                            code: i64::from(code),
                            signal: String::new(),
                            forced: false,
                        })
                    } else {
                        Err(Failure(
                            io::ErrorKind::Other,
                            "cannot read PTY process exit status".into(),
                        ))
                    },
                );
                owner.job.terminate(); // includes detached descendants in this Job
            }
            if (closed.is_cancelled() || exit.is_some()) && close_at.is_none() {
                close_at = Some(tokio::time::Instant::now());
                let pc = owned_console
                    .lock()
                    .ok()
                    .and_then(|mut console| console.take());
                close_task = Some(tokio::task::spawn_blocking(move || drop(pc)));
            }
            if close_at.is_some_and(|at| at.elapsed() >= CLOSE_GRACE) {
                owner.job.terminate();
                pipes.cancel();
                if let Some(Ok(exit)) = &mut exit {
                    exit.forced = true;
                }
            }
            let console_done = close_task.as_ref().is_some_and(|task| task.is_finished());
            if console_done {
                pipes.writer_stop.store(true, Ordering::Release);
                if let Some(writer) = &pipes.writer {
                    unsafe {
                        CancelSynchronousIo(writer.as_raw_handle());
                    }
                }
            }
            if exit.is_some() && console_done {
                // ClosePseudoConsole completed; buffered output can drain before
                // the IO threads end naturally. Repeated cancellation closes any
                // inherited or stalled synchronous IO after the bounded grace.
                if pipes.done() {
                    pipes.join_finished();
                    break;
                }
            }
            if close_at.is_some_and(|at| at.elapsed() >= CLOSE_GRACE + REAP_GRACE) {
                owner.job.terminate();
                pipes.cancel();
                exit = Some(Err(Failure(
                    io::ErrorKind::TimedOut,
                    "native ConPTY teardown did not complete within its deadline".into(),
                )));
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        // A timeout is reported as a failure, never native cleanup acceptance.
        tx.send_replace(Some(exit.unwrap_or_else(|| {
            Err(Failure(
                io::ErrorKind::Other,
                "PTY ended without exit status".into(),
            ))
        })));
    });
    Ok(Arc::new(NativePty {
        pid: info.dwProcessId,
        console,
        reads: tokio::sync::Mutex::new(ReadState {
            receiver: read_rx,
            pending: VecDeque::new(),
        }),
        writes: write_tx,
        closing,
        result,
    }))
}
impl PtySession for NativePty {
    fn pid(&self) -> u32 {
        self.pid
    }
    fn read<'a>(&'a self, bytes: &'a mut [u8]) -> CoreFuture<'a, io::Result<usize>> {
        Box::pin(async move {
            if bytes.is_empty() {
                return Ok(0);
            }
            let mut state = self.reads.lock().await;
            if state.pending.is_empty() {
                match state.receiver.recv().await {
                    Some(Ok(chunk)) => state.pending.extend(chunk),
                    Some(Err(error)) => return Err(error),
                    None => return Ok(0),
                }
            }
            let count = bytes.len().min(state.pending.len());
            for target in &mut bytes[..count] {
                *target = state.pending.pop_front().unwrap();
            }
            Ok(count)
        })
    }
    fn write<'a>(&'a self, bytes: &'a [u8]) -> CoreFuture<'a, io::Result<usize>> {
        Box::pin(async move {
            let (tx, rx) = oneshot::channel();
            tokio::select! {
                _ = self.closing.cancelled() => return Err(io::Error::new(io::ErrorKind::BrokenPipe, "PTY closed")),
                result = self.writes.send(WriteRequest { bytes: bytes.to_vec(), result: tx }) => result.map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "PTY input ended"))?,
            }
            tokio::select! {
                _ = self.closing.cancelled() => Err(io::Error::new(io::ErrorKind::BrokenPipe, "PTY closed")),
                result = rx => result.map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "PTY input ended"))?,
            }
        })
    }
    fn resize(&self, dimensions: PtySize) -> io::Result<()> {
        let console = self
            .console
            .lock()
            .map_err(|_| io::Error::other("PTY console lock poisoned"))?;
        let console = console
            .as_ref()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "PTY closed"))?;
        let hr = unsafe {
            ResizePseudoConsole(
                console.0,
                COORD {
                    X: dimensions.cols as i16,
                    Y: dimensions.rows as i16,
                },
            )
        };
        if hr < 0 {
            Err(io::Error::other(format!(
                "ResizePseudoConsole failed: {hr:#x}"
            )))
        } else {
            Ok(())
        }
    }
    fn close(&self) {
        self.closing.cancel();
    }
    fn wait(&self) -> CoreFuture<'_, io::Result<PtyExit>> {
        Box::pin(async move {
            let mut rx = self.result.clone();
            loop {
                if let Some(result) = rx.borrow_and_update().clone() {
                    return result.map_err(|Failure(kind, message)| io::Error::new(kind, message));
                }
                rx.changed()
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn command_line_quote_matches_go_escape_arg_shapes() {
        for (input, expected) in [
            ("plain", "plain"),
            ("/c", "/c"),
            ("", "\"\""),
            ("two words", "\"two words\""),
            ("a\"b", "a\\\"b"),
            ("tail\\", "tail\\"),
            ("space tail\\", "\"space tail\\\\\""),
        ] {
            assert_eq!(
                String::from_utf16(&quote(OsStr::new(input)).unwrap()).unwrap(),
                expected
            );
        }
    }
    #[test]
    fn windows_exit_dword_does_not_turn_into_posix_signal() {
        let status = PtyExit {
            code: i64::from(0xc0000005u32),
            signal: String::new(),
            forced: false,
        };
        assert_eq!(status.code, 3221225477);
        assert_eq!(status.state(), "error");
        assert!(status.signal.is_empty());
    }
    #[test]
    fn suspended_child_test_helper() {
        if let Some(marker) = std::env::var_os("MANY_AI_SYNTHETIC_PTY_MARKER") {
            std::fs::write(marker, b"executed").unwrap();
            std::process::exit(0);
        }
    }
    #[tokio::test]
    async fn failed_job_attachment_cannot_execute_provider_code() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("must-not-exist");
        let mut env = BTreeMap::new();
        env.insert(
            "MANY_AI_SYNTHETIC_PTY_MARKER".into(),
            Some(marker.as_os_str().to_owned()),
        );
        let plan = ProcessPlan {
            executable: std::env::current_exe().unwrap(),
            args: vec![
                "--exact".into(),
                "process::pty_windows::tests::suspended_child_test_helper".into(),
                "--nocapture".into(),
            ],
            cwd: root.path().into(),
            env,
            stdin: vec![],
            timeout: Duration::ZERO,
            output_cap: 1024,
            pipe_drain_timeout: Duration::from_secs(1),
        };
        let result = spawn_with_job(&plan, PtySize::default(), |_| {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "synthetic Job assignment denial",
            ))
        });
        assert!(result.is_err());
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!marker.exists());
    }
}
