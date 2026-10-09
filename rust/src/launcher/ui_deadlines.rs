//! Per-request launcher HTTP deadlines over Hyper's existing transport.
//!
//! The header timer supplies one absolute read deadline to the body reader.
//! The write deadline starts after headers, as Go net/http's readRequest does;
//! it limits socket writes/flushes, not the duration of application handlers.
use std::{
    future::Future,
    io,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::{Duration, Instant},
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

pub(super) const READ_TIMEOUT: Duration = Duration::from_secs(30);
const WRITE_TIMEOUT: Duration = Duration::from_secs(60);

struct State {
    read: tokio::time::Instant,
    write: Option<tokio::time::Instant>,
}
#[derive(Clone)]
pub(super) struct Deadlines(Arc<Mutex<State>>);
impl Deadlines {
    pub(super) fn new() -> Self {
        Self(Arc::new(Mutex::new(State {
            read: tokio::time::Instant::now() + READ_TIMEOUT,
            write: None,
        })))
    }
    fn headers(&self, deadline: Instant) {
        let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        state.read = deadline.into();
        state.write = None;
    }
    pub(super) fn begin_response(&self) -> tokio::time::Instant {
        let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        state.write = Some(tokio::time::Instant::now() + WRITE_TIMEOUT);
        state.read
    }
}

pub(super) struct HeaderTimer(pub(super) Deadlines);
impl hyper::rt::Timer for HeaderTimer {
    fn sleep(&self, duration: Duration) -> Pin<Box<dyn hyper::rt::Sleep>> {
        self.sleep_until(self.now() + duration)
    }
    fn sleep_until(&self, deadline: Instant) -> Pin<Box<dyn hyper::rt::Sleep>> {
        self.0.headers(deadline);
        hyper_util::rt::TokioTimer::new().sleep_until(deadline)
    }
    fn now(&self) -> Instant {
        tokio::time::Instant::now().into()
    }
    fn reset(&self, sleep: &mut Pin<Box<dyn hyper::rt::Sleep>>, deadline: Instant) {
        self.0.headers(deadline);
        hyper_util::rt::TokioTimer::new().reset(sleep, deadline);
    }
}

pub(super) struct DeadlineIo<T> {
    inner: T,
    deadlines: Deadlines,
    write_timer: Option<Pin<Box<tokio::time::Sleep>>>,
}
impl<T> DeadlineIo<T> {
    pub(super) fn new(inner: T, deadlines: Deadlines) -> Self {
        Self {
            inner,
            deadlines,
            write_timer: None,
        }
    }
    fn check_write(&mut self, cx: &mut Context<'_>) -> io::Result<()> {
        let deadline = self
            .deadlines
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .write;
        let Some(deadline) = deadline else {
            self.write_timer = None;
            return Ok(());
        };
        let timer = self
            .write_timer
            .get_or_insert_with(|| Box::pin(tokio::time::sleep_until(deadline)));
        if timer.deadline() != deadline {
            timer.as_mut().reset(deadline);
        }
        if timer.as_mut().poll(cx).is_ready() {
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "launcher HTTP response write deadline exceeded",
            ))
        } else {
            Ok(())
        }
    }
}
impl<T: AsyncRead + Unpin> AsyncRead for DeadlineIo<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_read(cx, buffer)
    }
}
impl<T: AsyncWrite + Unpin> AsyncWrite for DeadlineIo<T> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        this.check_write(cx)?;
        Pin::new(&mut this.inner).poll_write(cx, bytes)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.check_write(cx)?;
        Pin::new(&mut this.inner).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.check_write(cx)?;
        Pin::new(&mut this.inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyper::rt::Timer;
    use tokio::io::AsyncWriteExt;

    #[tokio::test(start_paused = true)]
    async fn write_deadline_includes_handler_time_and_stalled_socket() {
        let deadlines = Deadlines::new();
        deadlines.begin_response();
        tokio::time::advance(Duration::from_secs(59)).await;
        let (stream, _unread_client) = tokio::io::duplex(1);
        let mut stream = DeadlineIo::new(stream, deadlines);
        let writer = tokio::spawn(async move { stream.write_all(b"blocked").await });
        tokio::task::yield_now().await;
        assert!(!writer.is_finished());
        tokio::time::advance(Duration::from_secs(1)).await;
        assert_eq!(
            writer.await.unwrap().unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
    }

    struct StalledFlush;
    impl AsyncWrite for StalledFlush {
        fn poll_write(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            bytes: &[u8],
        ) -> Poll<io::Result<usize>> {
            Poll::Ready(Ok(bytes.len()))
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Pending
        }
        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }
    #[tokio::test(start_paused = true)]
    async fn response_flush_uses_the_same_absolute_write_deadline() {
        let deadlines = Deadlines::new();
        deadlines.begin_response();
        let mut stream = DeadlineIo::new(StalledFlush, deadlines);
        let writer = tokio::spawn(async move {
            stream.write_all(b"accepted").await.unwrap();
            stream.flush().await
        });
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(59)).await;
        assert!(!writer.is_finished());
        tokio::time::advance(Duration::from_secs(1)).await;
        assert_eq!(
            writer.await.unwrap().unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
    }

    #[tokio::test(start_paused = true)]
    async fn new_request_resets_read_and_write_deadlines() {
        let deadlines = Deadlines::new();
        let timer = HeaderTimer(deadlines.clone());
        let _headers = timer.sleep(READ_TIMEOUT);
        tokio::time::advance(Duration::from_secs(20)).await;
        assert_eq!(
            deadlines.begin_response() - tokio::time::Instant::now(),
            Duration::from_secs(10)
        );
        tokio::time::advance(Duration::from_secs(70)).await;
        let mut headers = timer.sleep(READ_TIMEOUT);
        tokio::time::advance(Duration::from_secs(5)).await;
        timer.reset(&mut headers, timer.now() + READ_TIMEOUT);
        assert_eq!(
            deadlines.begin_response() - tokio::time::Instant::now(),
            READ_TIMEOUT
        );
        let (stream, mut client) = tokio::io::duplex(64);
        let mut stream = DeadlineIo::new(stream, deadlines);
        stream.write_all(b"new response").await.unwrap();
        use tokio::io::AsyncReadExt;
        let mut bytes = [0; 12];
        client.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"new response");
    }
}
