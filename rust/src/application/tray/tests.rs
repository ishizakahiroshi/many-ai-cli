use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
#[derive(Default)]
struct Hooks {
    running: AtomicBool,
    started: AtomicUsize,
    opened: AtomicUsize,
    stopped: AtomicUsize,
}
impl TrayHooks for Hooks {
    fn running_url(&self) -> CoreFuture<'_, io::Result<Option<String>>> {
        Box::pin(async move {
            Ok(self
                .running
                .load(Ordering::SeqCst)
                .then(|| "http://127.0.0.1:49351/".into()))
        })
    }
    fn start_hub(&self) -> CoreFuture<'_, io::Result<()>> {
        Box::pin(async move {
            self.started.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }
    fn open_url<'a>(&'a self, url: &'a str) -> CoreFuture<'a, io::Result<()>> {
        Box::pin(async move {
            assert_eq!(url, "http://127.0.0.1:49351/");
            self.opened.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }
    fn stop_hub(&self) -> CoreFuture<'_, io::Result<()>> {
        Box::pin(async move {
            self.stopped.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }
}
#[tokio::test(start_paused = true)]
async fn startup_waits_for_real_url_and_deadline_never_opens_empty_browser() {
    let hooks = Arc::new(Hooks::default());
    let owner = TrayOwner::new(hooks.clone());
    let task = {
        let owner = owner.clone();
        tokio::spawn(async move { owner.open_hub().await })
    };
    tokio::task::yield_now().await;
    assert_eq!(hooks.started.load(Ordering::SeqCst), 1);
    tokio::time::advance(Duration::from_secs(21)).await;
    assert_eq!(
        task.await.unwrap().unwrap_err().kind(),
        io::ErrorKind::TimedOut
    );
    assert_eq!(hooks.opened.load(Ordering::SeqCst), 0);
    owner.shutdown().await;
}
#[tokio::test]
async fn existing_hub_opens_without_spawn_stop_keeps_tray_and_shutdown_keeps_hub() {
    let hooks = Arc::new(Hooks::default());
    hooks.running.store(true, Ordering::SeqCst);
    let owner = TrayOwner::new(hooks.clone());
    owner.open_hub().await.unwrap();
    owner.stop_hub().await.unwrap();
    assert_eq!(hooks.started.load(Ordering::SeqCst), 0);
    assert_eq!(hooks.opened.load(Ordering::SeqCst), 1);
    assert_eq!(hooks.stopped.load(Ordering::SeqCst), 1);
    owner.command(1);
    owner.shutdown().await;
    assert_eq!(hooks.stopped.load(Ordering::SeqCst), 1);
    owner.command(2);
    assert!(owner.runs.lock().unwrap().is_empty());
}
