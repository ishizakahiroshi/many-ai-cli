use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

struct Authority(AtomicBool);
impl UiAuthorization for Authority {
    fn is_valid(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}
struct PausedWriter {
    entered: Arc<tokio::sync::Notify>,
    resume: Arc<tokio::sync::Notify>,
    frames: Arc<Mutex<Vec<WireFrame>>>,
    pause: bool,
}
impl FrameWriter for PausedWriter {
    fn write<'a>(&'a mut self, _: WireFrame) -> CoreFuture<'a, Result<(), SessionError>> {
        panic!("PIN UI text must use guarded dispatch")
    }
    fn write_guarded<'a>(
        &'a mut self,
        frame: WireFrame,
        guard: InputSendGuard<'a>,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            if std::mem::take(&mut self.pause) {
                self.entered.notify_one();
                self.resume.notified().await;
            }
            guard(&mut || {
                self.frames.lock().unwrap().push(frame.clone());
                Ok(())
            })
        })
    }
}

#[tokio::test]
async fn pin_ui_writer_rechecks_after_async_wait_but_allows_close_after_revocation() {
    let entered = Arc::new(tokio::sync::Notify::new());
    let resume = Arc::new(tokio::sync::Notify::new());
    let frames = Arc::new(Mutex::new(Vec::new()));
    let authority = Arc::new(Authority(AtomicBool::new(true)));
    let mut writer = PinUiWriter {
        authorization: authority.clone(),
        inner: Box::new(PausedWriter {
            entered: entered.clone(),
            resume: resume.clone(),
            frames: frames.clone(),
            pause: true,
        }),
    };
    let writing = tokio::spawn(async move {
        let result = writer
            .write(WireFrame::Text("must not disclose".into()))
            .await;
        (writer, result)
    });
    entered.notified().await;
    authority.0.store(false, Ordering::Release);
    resume.notify_one();
    let (mut writer, result) = writing.await.unwrap();
    assert_eq!(result, Err(SessionError::AuthenticationExpired));
    assert!(frames.lock().unwrap().is_empty());
    writer.write(WireFrame::Close).await.unwrap();
    assert_eq!(*frames.lock().unwrap(), vec![WireFrame::Close]);
    writer.authorization = Arc::new(Authority(AtomicBool::new(true)));
    writer
        .write(WireFrame::Text("fresh connection control".into()))
        .await
        .unwrap();
    assert_eq!(
        frames.lock().unwrap()[1],
        WireFrame::Text("fresh connection control".into())
    );
}
