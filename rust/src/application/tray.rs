//! Independent Windows tray lifetime; Hub actions are supplied by the actual CLI composition.
use crate::{process::Cancellation, proto::core::CoreFuture};
use std::{
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
pub mod native;
#[cfg(test)]
mod tests;
#[cfg(windows)]
mod windows;
pub trait TrayHooks: Send + Sync {
    fn instance_key(&self) -> String {
        "ManyAICLITray".into()
    }
    fn instance_port(&self) -> Option<u16> {
        None
    }
    fn requested_port(&self) -> Option<u16> {
        None
    }
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
    actions: tokio::sync::Mutex<()>,
    opening: AtomicBool,
    last_open: Mutex<Option<tokio::time::Instant>>,
}
impl TrayOwner {
    pub fn new(hooks: Arc<dyn TrayHooks>) -> Arc<Self> {
        Arc::new(Self {
            hooks,
            cancel: Cancellation::default(),
            runs: Mutex::new(vec![]),
            runtime: tokio::runtime::Handle::current(),
            actions: tokio::sync::Mutex::new(()),
            opening: AtomicBool::new(false),
            last_open: Mutex::new(None),
        })
    }
    pub async fn open_hub(&self) -> io::Result<()> {
        // All clicks, including requests forwarded by other processes, pass
        // through this one owner. Coalesce pending and just-completed opens.
        if self.opening.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        let _request = OpenRequest(&self.opening);
        let _action = self.actions.lock().await;
        if self.cancel.is_cancelled() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "tray closed"));
        }
        if self
            .last_open
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_some_and(|at| at.elapsed() < Duration::from_millis(1500))
        {
            return Ok(());
        }
        let result = tokio::select! {
            biased;
            _ = self.cancel.cancelled() => Err(io::Error::new(io::ErrorKind::Interrupted, "tray closed")),
            result = tokio::time::timeout(Duration::from_secs(20), async {
                if let Some(url) = self.hooks.running_url().await? {
                    return self.hooks.open_url(&url).await.map_err(|error| io::Error::new(error.kind(), format!("ブラウザを開けませんでした。既定のブラウザを確認してください: {error}")));
                }
                self.hooks.start_hub().await?;
                loop {
                    if let Some(url) = self.hooks.running_url().await? {
                        return self.hooks.open_url(&url).await.map_err(|error| io::Error::new(error.kind(), format!("ブラウザを開けませんでした。Hub は起動しています。既定のブラウザを確認してください: {error}")));
                    }
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            }) => result.unwrap_or_else(|_| Err(io::Error::new(io::ErrorKind::TimedOut, "Hub が20秒以内に応答しませんでした。起動状態・使用中のポートを確認してください。"))),
        };
        *self.last_open.lock().unwrap_or_else(|p| p.into_inner()) =
            Some(tokio::time::Instant::now());
        result
    }
    pub async fn stop_hub(&self) -> io::Result<()> {
        let _action = self.actions.lock().await;
        if self.hooks.running_url().await?.is_some() {
            self.hooks.stop_hub().await?;
        }
        *self.last_open.lock().unwrap_or_else(|p| p.into_inner()) = None;
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
        runs.push(self.runtime.spawn(async move {
            let result = tokio::select! {
                biased;
                _ = owner.cancel.cancelled() => return,
                result = async { match id {
                    1 => owner.open_hub().await,
                    2 => owner.stop_hub().await,
                    4 => Err(io::Error::new(io::ErrorKind::AddrInUse, "同じ保存先のトレイが別のポートで起動しています。既存トレイだけを終了し、使用するアイコンを開き直してください。Hub は自動では停止しません。")),
                    _ => Ok(())
                } } => result,
            };
            if let Err(error) = result {
                if !owner.cancel.is_cancelled() {
                    let message = error.to_string();
                    let _ = tokio::task::spawn_blocking(move || report_error(&message)).await;
                }
            }
        }));
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
        self.run_with_open(trial, false).await
    }
    pub async fn run_with_open(self: &Arc<Self>, trial: bool, open: bool) -> io::Result<()> {
        if trial {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "trial tray requires synthetic message owner",
            ));
        }
        #[cfg(windows)]
        {
            let owner = self.clone();
            let result = tokio::task::spawn_blocking(move || windows::run(owner, open))
                .await
                .map_err(io::Error::other)?;
            self.shutdown().await;
            result
        }
        #[cfg(not(windows))]
        {
            let _ = open;
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "tray is only available on Windows",
            ))
        }
    }
}
struct OpenRequest<'a>(&'a AtomicBool);
impl Drop for OpenRequest<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
/// A tray invocation may have detached its console. Always retain a visible
/// error route; native dialogs run on a blocking thread, never a Tokio worker.
pub fn report_error(message: &str) {
    #[cfg(windows)]
    windows::show_error(message);
    #[cfg(not(windows))]
    eprintln!("{message}");
}
