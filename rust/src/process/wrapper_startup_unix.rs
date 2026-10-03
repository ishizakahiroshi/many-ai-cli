use super::{ProcessPlan, STARTUP_JOB_ENV, WrapperExit};
use std::{
    fs::File,
    io,
    os::unix::process::{CommandExt, ExitStatusExt},
    process::{Command, Stdio},
};

pub(super) struct Child {
    child: std::process::Child,
    startup: bool,
    retained: bool,
    exit: Option<WrapperExit>,
}
impl Child {
    pub(super) fn spawn(plan: &ProcessPlan, stdout: File, stderr: File) -> io::Result<Self> {
        let mut command = Command::new(&plan.executable);
        command
            .args(&plan.args)
            .current_dir(&plan.cwd)
            .stdin(Stdio::null())
            .stdout(stdout)
            .stderr(stderr);
        for (key, value) in &plan.env {
            if let Some(value) = value {
                command.env(key, value);
            } else {
                command.env_remove(key);
            }
        }
        command.env_remove(STARTUP_JOB_ENV);
        // SAFETY: only async-signal-safe setsid runs between fork and exec. It
        // detaches the wrapper as in the fixed Go setCmdSysProcAttr oracle.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        Ok(Self {
            child: command.spawn()?,
            startup: true,
            retained: true,
            exit: None,
        })
    }
    pub(super) fn pid(&self) -> u32 {
        self.child.id()
    }
    pub(super) fn terminate(&mut self) {
        if self.exit.is_none() && self.retained {
            // The direct child has not been reaped, so this owned session leader
            // PID cannot have been reused. Never target a name or caller PID.
            unsafe {
                libc::kill(-(self.child.id() as i32), libc::SIGKILL);
            }
        }
    }
    pub(super) fn disarm(&mut self) {
        self.startup = false;
    }
    pub(super) fn transfer(&mut self) {}
    pub(super) fn poll(&mut self) -> io::Result<Option<WrapperExit>> {
        if let Some(exit) = &self.exit {
            return Ok(Some(exit.clone()));
        }
        // Observe without reaping. Startup cleanup must kill the group while its
        // leader is still retained; try_wait then kill would permit PID reuse.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                self.child.id(),
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if result != 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ECHILD) {
                // A foreign child reaper broke exclusive ownership. Do not signal
                // this numeric PID again after its retention guarantee is lost.
                self.retained = false;
            }
            if error.kind() == io::ErrorKind::Interrupted {
                return Ok(None);
            }
            return Err(error);
        }
        if unsafe { info.si_pid() } == 0 {
            return Ok(None);
        }
        if self.startup {
            self.terminate();
        }
        self.wait().map(Some)
    }
    pub(super) fn wait(&mut self) -> io::Result<WrapperExit> {
        if let Some(exit) = &self.exit {
            return Ok(exit.clone());
        }
        let status = self.child.wait().inspect_err(|error| {
            if error.raw_os_error() == Some(libc::ECHILD) {
                self.retained = false;
            }
        })?;
        self.retained = false;
        let exit = WrapperExit {
            code: status.code().map(i64::from),
            signal: status.signal(),
        };
        self.exit = Some(exit.clone());
        Ok(exit)
    }
}
