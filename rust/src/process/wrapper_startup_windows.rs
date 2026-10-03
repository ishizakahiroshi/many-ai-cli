//! Suspended wrapper creation with an explicit inherited-handle allowlist.
//! The Job policy itself is shared with every other owned process adapter.
use super::{ProcessPlan, STARTUP_JOB_ENV, WrapperExit};
use crate::process::windows_job::OwnedJob;
use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    fs::{File, OpenOptions},
    io, mem,
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    ptr,
};
use windows_sys::Win32::{Foundation::*, System::Threading::*};

fn wide(value: &OsStr) -> io::Result<Vec<u16>> {
    let mut result: Vec<_> = value.encode_wide().collect();
    if result.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "NUL in wrapper startup argument",
        ));
    }
    result.push(0);
    Ok(result)
}
fn quote(value: &OsStr) -> io::Result<Vec<u16>> {
    let value = wide(value)?;
    let value = &value[..value.len() - 1];
    // Go syscall.EscapeArg; wrapper argv contains no shell script layer.
    let quoted = value.is_empty() || value.iter().any(|c| matches!(*c, 32 | 9));
    let mut result = Vec::new();
    if quoted {
        result.push(b'"' as u16);
    }
    let mut slashes = 0;
    for c in value {
        if *c == b'\\' as u16 {
            slashes += 1;
            continue;
        }
        result.extend(std::iter::repeat_n(
            b'\\' as u16,
            if *c == b'"' as u16 {
                slashes * 2 + 1
            } else {
                slashes
            },
        ));
        result.push(*c);
        slashes = 0;
    }
    result.extend(std::iter::repeat_n(
        b'\\' as u16,
        slashes * if quoted { 2 } else { 1 },
    ));
    if quoted {
        result.push(b'"' as u16);
    }
    Ok(result)
}
fn environment(plan: &ProcessPlan, job: HANDLE) -> io::Result<Vec<u16>> {
    let mut values: BTreeMap<String, (OsString, OsString)> = std::env::vars_os()
        .map(|(key, value)| (key.to_string_lossy().to_uppercase(), (key, value)))
        .collect();
    for (key, value) in &plan.env {
        let normalized = key.to_string_lossy().to_uppercase();
        if let Some(value) = value {
            values.insert(normalized, (key.clone(), value.clone()));
        } else {
            values.remove(&normalized);
        }
    }
    // A caller override cannot redirect adoption to a foreign handle.
    values.insert(
        STARTUP_JOB_ENV.into(),
        (STARTUP_JOB_ENV.into(), (job as usize).to_string().into()),
    );
    let mut block = Vec::new();
    for (_, (mut key, value)) in values {
        key.push("=");
        key.push(value);
        block.extend(wide(&key)?);
    }
    block.push(0);
    Ok(block)
}
fn inheritable(handle: HANDLE) -> io::Result<OwnedHandle> {
    let mut inherited = ptr::null_mut();
    if unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            handle,
            GetCurrentProcess(),
            &mut inherited,
            0,
            1,
            DUPLICATE_SAME_ACCESS,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { OwnedHandle::from_raw_handle(inherited) })
}
struct Attributes {
    storage: Vec<usize>,
    initialized: bool,
}
impl Attributes {
    fn new(handles: &[HANDLE]) -> io::Result<Self> {
        let mut bytes = 0;
        unsafe {
            InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut bytes);
        }
        if bytes == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut this = Self {
            storage: vec![0; bytes.div_ceil(mem::size_of::<usize>())],
            initialized: false,
        };
        if unsafe { InitializeProcThreadAttributeList(this.raw(), 1, 0, &mut bytes) } == 0 {
            return Err(io::Error::last_os_error());
        }
        this.initialized = true;
        // HANDLE_LIST requires inheritable real handles and bInheritHandles=TRUE.
        // `handles` and its owned backing handles live through CreateProcessW.
        if unsafe {
            UpdateProcThreadAttribute(
                this.raw(),
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                handles.as_ptr().cast(),
                mem::size_of_val(handles),
                ptr::null_mut(),
                ptr::null(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(this)
    }
    fn raw(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.storage.as_mut_ptr().cast()
    }
}
impl Drop for Attributes {
    fn drop(&mut self) {
        if self.initialized {
            unsafe {
                DeleteProcThreadAttributeList(self.raw());
            }
        }
    }
}

pub(super) struct Child {
    process: OwnedHandle,
    job: Option<OwnedJob>,
    pid: u32,
    startup: bool,
    exit: Option<WrapperExit>,
}
impl Child {
    pub(super) fn spawn(plan: &ProcessPlan, stdout: File, stderr: File) -> io::Result<Self> {
        Self::spawn_with_assignment(plan, stdout, stderr, |job, process| job.assign(process))
    }
    fn spawn_with_assignment(
        plan: &ProcessPlan,
        stdout: File,
        stderr: File,
        assign: impl FnOnce(&OwnedJob, HANDLE) -> io::Result<()>,
    ) -> io::Result<Self> {
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
                "wrapper command exceeds Windows command-line limit",
            ));
        }
        let job = OwnedJob::create()?;
        let inherited_job = job.inherited_query_handle()?;
        let environment = environment(plan, inherited_job.as_raw_handle())?;
        let null = OpenOptions::new().read(true).write(true).open("NUL")?;
        let input = inheritable(null.as_raw_handle())?;
        let output = inheritable(stdout.as_raw_handle())?;
        let error = inheritable(stderr.as_raw_handle())?;
        let handles = [
            inherited_job.as_raw_handle(),
            input.as_raw_handle(),
            output.as_raw_handle(),
            error.as_raw_handle(),
        ];
        let mut attributes = Attributes::new(&handles)?;
        let mut startup: STARTUPINFOEXW = unsafe { mem::zeroed() };
        startup.StartupInfo.cb = mem::size_of::<STARTUPINFOEXW>() as u32;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = input.as_raw_handle();
        startup.StartupInfo.hStdOutput = output.as_raw_handle();
        startup.StartupInfo.hStdError = error.as_raw_handle();
        startup.lpAttributeList = attributes.raw();
        let mut info: PROCESS_INFORMATION = unsafe { mem::zeroed() };
        let flags = EXTENDED_STARTUPINFO_PRESENT
            | CREATE_UNICODE_ENVIRONMENT
            | CREATE_SUSPENDED
            | CREATE_NEW_PROCESS_GROUP
            | CREATE_NO_WINDOW;
        if unsafe {
            CreateProcessW(
                executable.as_ptr(),
                command.as_mut_ptr(),
                ptr::null(),
                ptr::null(),
                1,
                flags,
                environment.as_ptr().cast(),
                cwd.as_ptr(),
                &startup.StartupInfo,
                &mut info,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let process = unsafe { OwnedHandle::from_raw_handle(info.hProcess) };
        let primary = unsafe { OwnedHandle::from_raw_handle(info.hThread) };
        // Before assignment the child is still suspended and cannot create any
        // descendants. Failed assignment must explicitly kill that direct child.
        if let Err(error) = assign(&job, process.as_raw_handle()) {
            unsafe {
                TerminateProcess(process.as_raw_handle(), 1);
                WaitForSingleObject(process.as_raw_handle(), 2000);
            }
            return Err(error);
        }
        let owner = Self {
            process,
            job: Some(job),
            pid: info.dwProcessId,
            startup: true,
            exit: None,
        };
        if unsafe { ResumeThread(primary.as_raw_handle()) } != 1 {
            owner.job.as_ref().unwrap().terminate();
            unsafe {
                WaitForSingleObject(owner.process.as_raw_handle(), 2000);
            }
            return Err(io::Error::other("cannot resume Job-contained wrapper"));
        }
        // Parent inheritable duplicates close here; the suspended child received
        // only the four allowlisted handles, never an ambient inheritable handle.
        Ok(owner)
    }
    pub(super) fn pid(&self) -> u32 {
        self.pid
    }
    pub(super) fn terminate(&mut self) {
        if let Some(job) = &self.job {
            job.terminate();
        }
    }
    pub(super) fn disarm(&mut self) {
        self.startup = false;
    }
    pub(super) fn transfer(&mut self) {
        self.job.take();
    }
    pub(super) fn poll(&mut self) -> io::Result<Option<WrapperExit>> {
        if let Some(exit) = &self.exit {
            return Ok(Some(exit.clone()));
        }
        match unsafe { WaitForSingleObject(self.process.as_raw_handle(), 0) } {
            WAIT_OBJECT_0 => {
                let exit = self.wait()?;
                if self.startup {
                    self.terminate();
                }
                Ok(Some(exit))
            }
            WAIT_TIMEOUT => Ok(None),
            _ => Err(io::Error::last_os_error()),
        }
    }
    pub(super) fn wait(&mut self) -> io::Result<WrapperExit> {
        if let Some(exit) = &self.exit {
            return Ok(exit.clone());
        }
        if unsafe { WaitForSingleObject(self.process.as_raw_handle(), INFINITE) } != WAIT_OBJECT_0 {
            return Err(io::Error::last_os_error());
        }
        let mut code = 0;
        if unsafe { GetExitCodeProcess(self.process.as_raw_handle(), &mut code) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let exit = WrapperExit {
            code: Some(i64::from(code)),
            signal: None,
        };
        self.exit = Some(exit.clone());
        Ok(exit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::private_io;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[test]
    fn failed_job_assignment_never_resumes_the_owned_child() {
        let root = tempfile::tempdir().unwrap();
        let plan = ProcessPlan {
            executable: std::env::current_exe().unwrap(),
            args: vec![
                "--exact".into(),
                "process::wrapper_startup::tests::startup_fixture".into(),
                "--nocapture".into(),
            ],
            cwd: root.path().into(),
            env: BTreeMap::from([
                (
                    "MANY_AI_SYNTHETIC_WRAPPER_MODE".into(),
                    Some("early_exit".into()),
                ),
                (
                    "MANY_AI_SYNTHETIC_WRAPPER_ROOT".into(),
                    Some(root.path().as_os_str().into()),
                ),
                ("HOME".into(), Some(root.path().as_os_str().into())),
                ("USERPROFILE".into(), Some(root.path().as_os_str().into())),
            ]),
            stdin: vec![],
            timeout: std::time::Duration::ZERO,
            output_cap: 0,
            pipe_drain_timeout: std::time::Duration::from_millis(100),
        };
        let pid = AtomicU32::new(0);
        let log = private_io::open_append(&root.path().join("spawn.log")).unwrap();
        let result =
            Child::spawn_with_assignment(&plan, log.try_clone().unwrap(), log, |_, process| {
                pid.store(unsafe { GetProcessId(process) }, Ordering::SeqCst);
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "synthetic Job attachment failure",
                ))
            });
        assert!(result.is_err());
        assert_ne!(pid.load(Ordering::SeqCst), 0);
        assert!(!crate::process::pid_alive(i64::from(
            pid.load(Ordering::SeqCst)
        )));
        assert!(!root.path().join("ready").exists());
    }
}
