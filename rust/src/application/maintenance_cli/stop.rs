//! Go Stop graceful request/poll/forced stop, adapted to the actual RuntimeLedger.
use crate::{
    application::hub_runtime::RuntimeLedger,
    config::{ConfigStore, RuntimePaths},
    process::Cancellation,
    proto::core::CoreFuture,
};
use std::{io, sync::Arc, time::Duration};
pub trait StopIo: Send + Sync {
    fn alive(&self, pid: i64) -> bool;
    fn request<'a>(
        &'a self,
        port: u16,
        token: &'a str,
        cancel: &'a Cancellation,
    ) -> CoreFuture<'a, io::Result<bool>>;
    fn kill(&self, pid: i64) -> io::Result<()>;
}
pub type StopLog = Arc<dyn Fn(&str, i64, u16) -> io::Result<()> + Send + Sync>;
pub struct StopCommand {
    pub ledger: RuntimeLedger,
    pub config: Arc<ConfigStore>,
    pub io: Arc<dyn StopIo>,
    pub log: StopLog,
}
impl StopCommand {
    pub async fn run(&self, cancel: &Cancellation) -> io::Result<()> {
        if cancel.is_cancelled() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "stop cancelled"));
        }
        let data = self
            .ledger
            .read()?
            .ok_or_else(|| io::Error::other("hub pid not found"))?;
        let port = u16::try_from(data.port)
            .ok()
            .filter(|port| *port > 0)
            .ok_or_else(|| io::Error::other("invalid hub runtime port"))?;
        let token = self
            .config
            .snapshot()
            .map_err(io::Error::other)?
            .config
            .token;
        let _ = (self.log)("graceful shutdown requested", data.pid, port);
        if self.io.request(port, &token, cancel).await.unwrap_or(false) {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            loop {
                if !self.io.alive(data.pid) {
                    self.ledger.remove_if_pid(data.pid)?;
                    return Ok(());
                }
                if tokio::time::Instant::now() >= deadline {
                    break;
                }
                tokio::select! {_=cancel.cancelled()=>return Err(io::Error::new(io::ErrorKind::Interrupted,"stop cancelled")),_=tokio::time::sleep(Duration::from_millis(100))=>{}}
            }
        }
        if cancel.is_cancelled() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "stop cancelled"));
        }
        if self.ledger.read()?.as_ref() != Some(&data) {
            return Err(io::Error::other("hub runtime owner changed during stop"));
        }
        let _ = (self.log)("MANY-AI-CLI stopping: cli_stop", data.pid, port);
        let result = self.io.kill(data.pid);
        let cleanup = self.ledger.remove_if_pid(data.pid);
        result.and(cleanup)
    }
}
pub struct NativeStopIo {
    pub paths: RuntimePaths,
}
impl StopIo for NativeStopIo {
    fn alive(&self, pid: i64) -> bool {
        crate::process::pid_alive(pid)
    }
    fn request<'a>(
        &'a self,
        port: u16,
        token: &'a str,
        cancel: &'a Cancellation,
    ) -> CoreFuture<'a, io::Result<bool>> {
        Box::pin(async move {
            if self.paths.is_trial() && port != self.paths.port() {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "trial shutdown port refused",
                ));
            }
            let url = format!("http://127.0.0.1:{port}/api/shutdown");
            let client = reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(2))
                .build()
                .map_err(io::Error::other)?;
            tokio::select! {_=cancel.cancelled()=>Err(io::Error::new(io::ErrorKind::Interrupted,"stop cancelled")),reply=client.post(url).query(&[("token",token)]).header(reqwest::header::CONTENT_TYPE,"application/json").send()=>reply.map(|response|response.status()==200).map_err(|_|io::Error::other("shutdown request failed"))}
        })
    }
    fn kill(&self, pid: i64) -> io::Result<()> {
        if self.paths.is_trial() || pid <= 0 || pid == i64::from(std::process::id()) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "unowned shutdown process refused",
            ));
        }
        #[cfg(unix)]
        {
            let pid =
                libc::pid_t::try_from(pid).map_err(|_| io::Error::other("invalid hub pid"))?;
            if unsafe { libc::kill(pid, libc::SIGKILL) } != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }
        #[cfg(windows)]
        {
            use windows_sys::Win32::{
                Foundation::CloseHandle,
                System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess},
            };
            let pid = u32::try_from(pid).map_err(|_| io::Error::other("invalid hub pid"))?;
            let handle = unsafe { OpenProcess(PROCESS_TERMINATE, 0, pid) };
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }
            let killed = unsafe { TerminateProcess(handle, 1) };
            let error = if killed == 0 {
                Some(io::Error::last_os_error())
            } else {
                None
            };
            unsafe { CloseHandle(handle) };
            error.map_or(Ok(()), Err)
        }
        #[cfg(not(any(unix, windows)))]
        {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "native hub stop unavailable",
            ))
        }
    }
}
