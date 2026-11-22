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
pub struct ExitSnapshot {
    pub count: usize,
    pub runtime: Option<crate::application::hub_runtime::RuntimeData>,
}
#[cfg(test)]
mod tests;
#[cfg(windows)]
pub(crate) mod windows;
/// Per-home identity shared by the tray mutex/window and the launch probe.
pub(crate) fn instance_key(root: &std::path::Path) -> String {
    use sha2::Digest;
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let root = crate::config::paths::equivalent_root_prefix(&root);
    let root = root.to_string_lossy();
    #[cfg(windows)]
    let root = root.to_lowercase();
    let digest: String = sha2::Sha256::digest(root.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("ManyAICLITray-{digest}")
}
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
    /// `many-ai-cli setup` once, so the migrated Hub survives the next sign-in.
    /// Non-native owners reject; `migrate_hub` surfaces that error unchanged.
    fn setup(&self) -> CoreFuture<'_, io::Result<()>> {
        Box::pin(async move {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "tray setup is unavailable",
            ))
        })
    }
    /// The console Hub's bound port, when a second tray forwards `--replace`.
    /// The existing owner must serve on that port, not the port it was started with.
    fn accept_migration_port(&self, _port: u16) {}
    fn running_url(&self) -> CoreFuture<'_, io::Result<Option<String>>>;
    fn start_hub(&self) -> CoreFuture<'_, io::Result<()>>;
    fn open_url<'a>(&'a self, url: &'a str) -> CoreFuture<'a, io::Result<()>>;
    fn stop_hub(&self) -> CoreFuture<'_, io::Result<()>>;
    /// Connected live sessions, including idle AI terminals. A failed query is not zero.
    fn active_sessions(&self) -> CoreFuture<'_, io::Result<ExitSnapshot>> {
        Box::pin(async { Err(io::Error::other("session status is unavailable")) })
    }
    /// End every connected session, then the Hub. Must not stop the Hub on failure.
    fn exit_app(&self, _snapshot: ExitSnapshot) -> CoreFuture<'_, io::Result<()>> {
        Box::pin(async { Err(io::Error::other("application exit is unavailable")) })
    }
    /// Stop used only by `migrate_hub`, so a later tray-menu stop keeps its own source.
    fn stop_for_migration(&self) -> CoreFuture<'_, io::Result<()>> {
        self.stop_hub()
    }
}
#[cfg(test)]
type TestErrorReporter = Arc<dyn Fn(&str) + Send + Sync>;
pub struct TrayOwner {
    hooks: Arc<dyn TrayHooks>,
    cancel: Cancellation,
    runs: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    runtime: tokio::runtime::Handle,
    actions: tokio::sync::Mutex<()>,
    opening: AtomicBool,
    exiting: AtomicBool,
    last_open: Mutex<Option<tokio::time::Instant>>,
    /// Visible window handle. `0` means the resident loop has no window yet,
    /// including every test that never creates one.
    #[cfg(windows)]
    presence_hwnd: std::sync::atomic::AtomicIsize,
    #[cfg(test)]
    test_error_reporter: Option<TestErrorReporter>,
    #[cfg(test)]
    test_exit_answer: AtomicBool,
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
            exiting: AtomicBool::new(false),
            last_open: Mutex::new(None),
            #[cfg(windows)]
            presence_hwnd: std::sync::atomic::AtomicIsize::new(0),
            #[cfg(test)]
            test_error_reporter: None,
            #[cfg(test)]
            test_exit_answer: AtomicBool::new(false),
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
    /// `tray --replace`: install setup first so a failure leaves the old Hub
    /// alone, then end the console Hub, start the detached one and reopen the
    /// browser. The URL wait mirrors open_hub's deadline (50 x 200ms).
    pub async fn migrate_hub(&self) -> io::Result<()> {
        if self.opening.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        let _request = OpenRequest(&self.opening);
        let _action = self.actions.lock().await;
        if self.cancel.is_cancelled() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "tray closed"));
        }
        let result = tokio::select! {
            biased;
            _ = self.cancel.cancelled() => Err(io::Error::new(io::ErrorKind::Interrupted, "tray closed")),
            result = tokio::time::timeout(Duration::from_secs(60), async {
                self.hooks.setup().await?;
                self.hooks.stop_for_migration().await?;
                self.hooks.start_hub().await?;
                for _ in 0..50 {
                    if let Some(url) = self.hooks.running_url().await? {
                        return self.hooks.open_url(&url).await.map_err(|error| io::Error::new(error.kind(), format!("ブラウザを開けませんでした。Hub は起動しています。既定のブラウザを確認してください: {error}")));
                    }
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
                Err(io::Error::new(io::ErrorKind::TimedOut, "Hub が起動しませんでした。起動状態・使用中のポートを確認してください。"))
            }) => result.unwrap_or_else(|_| Err(io::Error::new(io::ErrorKind::TimedOut, "Hub 移行が制限時間内に完了しませんでした。小さい窓から開き直してください。"))),
        };
        *self.last_open.lock().unwrap_or_else(|p| p.into_inner()) =
            Some(tokio::time::Instant::now());
        result
    }
    /// `REPLACE_REQUEST` carries the console Hub port in `port`. `None` keeps the owner's port.
    pub fn command_replace(self: &Arc<Self>, port: Option<u16>) {
        if let Some(port) = port.filter(|port| *port > 0) {
            self.hooks.accept_migration_port(port);
        }
        self.command(5);
    }
    pub fn command(self: &Arc<Self>, id: u32) {
        if id == 6 {
            if self.exiting.swap(true, Ordering::AcqRel) {
                return;
            }
        } else if self.exiting.load(Ordering::Acquire) {
            return;
        }
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
                result = async {
                    let result = match id {
                        1 => owner.open_hub().await,
                        2 => owner.stop_hub().await,
                        4 => Err(io::Error::new(io::ErrorKind::AddrInUse, "同じ保存先の小さい窓が別のポートで開いています。その窓のプロセスだけをタスクマネージャーで終了し、開き直してください。Hub は自動では停止しません。")),
                        5 => owner.migrate_hub().await,
                        6 => owner.exit_app().await.map(|exited| {
                            if !exited {
                                owner.exiting.store(false, Ordering::Release);
                            }
                        }),
                        _ => Ok(()),
                    };
                    // Command 5 is the settings switch. The status line would
                    // stay on the pre-switch value without this publish.
                    // There is no timer.
                    #[cfg(windows)]
                    if matches!(id, 1 | 2 | 5) {
                        owner.publish_presence().await;
                    }
                    result
                } => result,
            };
            if id == 6 && result.is_err() {
                owner.exiting.store(false, Ordering::Release);
            }
            if let Err(error) = result
                && !owner.cancel.is_cancelled()
            {
                let message = error.to_string();
                #[cfg(test)]
                if let Some(report) = &owner.test_error_reporter {
                    report(&message);
                    return;
                }
                let _ = tokio::task::spawn_blocking(move || report_error(&message)).await;
            }
        }));
    }
    async fn exit_app(&self) -> io::Result<bool> {
        let _action = self.actions.lock().await;
        let snapshot = self.hooks.active_sessions().await?;
        let count = snapshot.count;
        if count > 0 {
            #[cfg(test)]
            let confirmed = self.test_exit_answer.load(Ordering::Acquire);
            #[cfg(all(windows, not(test)))]
            let confirmed = {
                let hwnd = self.presence_window();
                tokio::task::spawn_blocking(move || windows::confirm_exit(hwnd, count))
                    .await
                    .map_err(io::Error::other)??
            };
            #[cfg(all(not(windows), not(test)))]
            let confirmed = false;
            if !confirmed {
                return Ok(false);
            }
        }
        self.hooks.exit_app(snapshot).await?;
        #[cfg(windows)]
        windows::post_presence_exit(self.presence_window())?;
        Ok(true)
    }
    #[cfg(windows)]
    fn bind_presence_window(&self, hwnd: isize) {
        self.presence_hwnd.store(hwnd, Ordering::Release);
    }
    #[cfg(windows)]
    fn presence_window(&self) -> isize {
        self.presence_hwnd.load(Ordering::Acquire)
    }
    /// Posts running/stopped to the visible window. A missing handle, including
    /// every test that never creates the window, does nothing and does not read
    /// the URL into the message.
    #[cfg(windows)]
    async fn publish_presence(&self) {
        let hwnd = self.presence_window();
        if hwnd == 0 {
            return;
        }
        let running = self.hooks.running_url().await.ok().flatten().is_some();
        windows::post_presence_state(hwnd, running);
    }
    #[cfg(windows)]
    fn kick_presence_refresh(self: &Arc<Self>) {
        if self.presence_window() == 0 {
            return;
        }
        let owner = self.clone();
        let task = self.runtime.spawn(async move {
            owner.publish_presence().await;
        });
        self.runs
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .push(task);
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
        self.run_with_open(trial, false, false).await
    }
    pub async fn run_with_open(
        self: &Arc<Self>,
        trial: bool,
        open: bool,
        replace: bool,
    ) -> io::Result<()> {
        if trial {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "trial tray requires synthetic message owner",
            ));
        }
        #[cfg(windows)]
        {
            let owner = self.clone();
            let result = tokio::task::spawn_blocking(move || windows::run(owner, open, replace))
                .await
                .map_err(io::Error::other)?;
            self.shutdown().await;
            result
        }
        #[cfg(not(windows))]
        {
            let _ = (open, replace);
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
