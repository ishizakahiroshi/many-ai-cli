//! Independent Windows tray lifetime; Hub actions are supplied by the actual CLI composition.
use crate::{process::Cancellation, proto::core::CoreFuture};
use std::{
    io,
    sync::{Arc, Mutex},
    time::Duration,
};
pub mod native;
#[cfg(test)]
mod tests;
#[cfg(windows)]
mod windows;
pub trait TrayHooks: Send + Sync {
    fn running_url(&self) -> CoreFuture<'_, io::Result<Option<String>>>;
    fn start_hub(&self) -> CoreFuture<'_, io::Result<()>>;
    fn open_url<'a>(&'a self, url: &'a str) -> CoreFuture<'a, io::Result<()>>;
    fn stop_hub(&self) -> CoreFuture<'_, io::Result<()>>;
}
pub struct TrayOwner {
    hooks: Arc<dyn TrayHooks>,
    cancel: Cancellation,
    runs: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    runtime: tokio::runtime::Handle,
}
impl TrayOwner {
    pub fn new(hooks: Arc<dyn TrayHooks>) -> Arc<Self> {
        Arc::new(Self {
            hooks,
            cancel: Cancellation::default(),
            runs: Mutex::new(vec![]),
            runtime: tokio::runtime::Handle::current(),
        })
    }
    pub async fn open_hub(&self) -> io::Result<()> {
        if let Some(url) = self.hooks.running_url().await? {
            return self.hooks.open_url(&url).await;
        }
        self.hooks.start_hub().await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(url) = self.hooks.running_url().await? {
                return self.hooks.open_url(&url).await;
            }
            tokio::select! {biased;_=self.cancel.cancelled()=>return Err(io::Error::new(io::ErrorKind::Interrupted,"tray closed")),_=tokio::time::sleep_until(deadline)=>return Err(io::Error::new(io::ErrorKind::TimedOut,"hub did not become reachable within 20s")),_=tokio::time::sleep(Duration::from_millis(250))=>{}}
        }
    }
    pub async fn stop_hub(&self) -> io::Result<()> {
        if self.hooks.running_url().await?.is_some() {
            self.hooks.stop_hub().await?;
        }
        Ok(())
    }
    pub fn command(self: &Arc<Self>, id: u32) {
        let owner = self.clone();
        let mut runs = self
            .runs
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if self.cancel.is_cancelled() {
            return;
        }
        runs.retain(|task| !task.is_finished());
        runs.push(self.runtime.spawn(async move {tokio::select!{biased;_=owner.cancel.cancelled()=>{},result=async{match id {1=>owner.open_hub().await,2=>owner.stop_hub().await,_=>Ok(())}}=>{let _=result;}}}));
    }
    pub async fn shutdown(&self) {
        self.cancel.cancel();
        let runs = std::mem::take(
            &mut *self
                .runs
                .lock()
                .unwrap_or_else(|poison| poison.into_inner()),
        );
        for task in runs {
            let _ = task.await;
        }
    }
    pub async fn run(self: &Arc<Self>, trial: bool) -> io::Result<()> {
        if trial {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "trial tray requires synthetic message owner",
            ));
        }
        #[cfg(windows)]
        {
            let owner = self.clone();
            let result = tokio::task::spawn_blocking(move || windows::run(owner))
                .await
                .map_err(io::Error::other)?;
            self.shutdown().await;
            result
        }
        #[cfg(not(windows))]
        {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "tray is only available on Windows",
            ))
        }
    }
}
