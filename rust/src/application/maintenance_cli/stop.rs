//! Go Stop graceful request/poll/forced stop, adapted to the actual RuntimeLedger.
use crate::{
    application::hub_runtime::{self, Owner, RuntimeData, RuntimeLedger},
    config::{ConfigStore, RuntimePaths},
    process::Cancellation,
    proto::core::CoreFuture,
};
use std::{io, sync::Arc, time::Duration};
pub trait StopIo: Send + Sync {
    /// Whether the Hub that wrote `data` is still running.
    fn alive(&self, data: &RuntimeData) -> bool;
    fn request<'a>(
        &'a self,
        port: u16,
        token: &'a str,
        // The CLI's local proof, so a Hub with a PIN set lets stop through (U09).
        proof: Option<&'a str>,
        cancel: &'a Cancellation,
    ) -> CoreFuture<'a, io::Result<bool>>;
    /// Ends the Hub that wrote `data`. Ok(true): ended, or already gone.
    /// Ok(false): another process now holds the PID and was left running.
    /// Err: not ended, including when the process could not be identified.
    fn kill(&self, data: &RuntimeData) -> io::Result<bool>;
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
        let config = self.config.snapshot().map_err(io::Error::other)?.config;
        let token = config.token.clone();
        let proof = crate::hub::pin::cli_local_proof(&config);
        let _ = (self.log)("graceful shutdown requested", data.pid, port);
        if self
            .io
            .request(port, &token, proof.as_deref(), cancel)
            .await
            .unwrap_or(false)
        {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            loop {
                if !self.io.alive(&data) {
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
        // The record is removed only once its Hub is known to be gone; a
        // failed or refused kill keeps it for the next stop (U11).
        if self.io.kill(&data)? {
            return self.ledger.remove_if_pid(data.pid);
        }
        let _ = (self.log)(
            "stale hub runtime record removed; its PID belongs to another process, left running",
            data.pid,
            port,
        );
        self.ledger.remove_if_pid(data.pid)?;
        Err(io::Error::other(format!(
            "the Hub was not running: PID {} from its runtime record now belongs to another process, which was left running; the stale record was removed",
            data.pid
        )))
    }
}
pub struct NativeStopIo {
    pub paths: RuntimePaths,
}
impl NativeStopIo {
    fn unconfirmed(&self, pid: i64) -> io::Error {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "could not confirm that PID {pid} is the Hub, so it was not force-stopped; if that process is many-ai-cli, end it yourself, otherwise the Hub has already ended and {} is a stale record you can delete",
                self.paths.root().join(hub_runtime::FILE).display()
            ),
        )
    }
}
impl StopIo for NativeStopIo {
    fn alive(&self, data: &RuntimeData) -> bool {
        matches!(
            hub_runtime::owner(data),
            Owner::Recorded | Owner::Unverified
        )
    }
    fn request<'a>(
        &'a self,
        port: u16,
        token: &'a str,
        proof: Option<&'a str>,
        cancel: &'a Cancellation,
    ) -> CoreFuture<'a, io::Result<bool>> {
        Box::pin(async move {
            if self.paths.is_trial() && port != self.paths.port() {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "trial shutdown port refused",
                ));
            }
            // The token goes only to a listener this OS user holds (U01).
            crate::process::listener::require_own_loopback_listener(port)?;
            let url = format!("http://127.0.0.1:{port}/api/shutdown");
            let client = reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(2))
                .build()
                .map_err(io::Error::other)?;
            tokio::select! {_=cancel.cancelled()=>Err(io::Error::new(io::ErrorKind::Interrupted,"stop cancelled")),reply=client.post(url).query(&[("token",token)]).headers(crate::application::hub_runtime::local_proof_headers(proof)).header(reqwest::header::CONTENT_TYPE,"application/json").header(crate::hub::lifecycle::STOP_SOURCE_HEADER,"stop_command").send()=>reply.map(|response|response.status()==200).map_err(|_|io::Error::other("shutdown request failed"))}
        })
    }
    fn kill(&self, data: &RuntimeData) -> io::Result<bool> {
        let pid = data.pid;
        if self.paths.is_trial() || pid <= 0 || pid == i64::from(std::process::id()) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "unowned shutdown process refused",
            ));
        }
        #[cfg(any(windows, target_os = "linux"))]
        {
            let native = u32::try_from(pid).map_err(|_| io::Error::other("invalid hub pid"))?;
            // The process is judged and ended through one retained handle or
            // pidfd, so another process cannot take the PID in between.
            let judged = std::cell::Cell::new(None);
            let ended = crate::process::wrapper_startup::recovery_identity::terminate_if(
                native,
                |identity| {
                    let same = hub_runtime::same_process(data, identity);
                    judged.set(same);
                    same == Some(true)
                },
            );
            match (ended, judged.get()) {
                (Ok(true), _) => Ok(true),
                (Ok(false), Some(false)) => Ok(false),
                // The Hub was identified but could not be ended.
                (Err(error), Some(true)) => Err(error),
                // The process could not be opened or identified.
                _ => Err(self.unconfirmed(pid)),
            }
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        {
            // No process identity here: a PID alone may name an unrelated
            // process, so only a PID that is gone counts as stopped.
            if crate::process::pid_alive(pid) {
                Err(self.unconfirmed(pid))
            } else {
                Ok(true)
            }
        }
    }
}
