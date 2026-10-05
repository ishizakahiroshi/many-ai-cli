//! Explicit executable startup context. Trial data never falls back to a home.
use crate::{cli::TrialOptions, config::RuntimePaths, process::Cancellation};
use std::{
    io,
    path::{Path, PathBuf},
};

pub fn user_home() -> io::Result<PathBuf> {
    let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    let value = std::env::var_os(key)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| io::Error::other("user home environment is unavailable"))?;
    let home = PathBuf::from(value);
    if !home.is_absolute() {
        return Err(io::Error::other("user home must be absolute"));
    }
    Ok(home)
}

pub fn runtime_paths(
    trial: Option<&TrialOptions>,
    installed_home: &Path,
) -> io::Result<RuntimePaths> {
    match trial {
        Some(trial) => RuntimePaths::trial(
            &trial.root,
            trial.port,
            &installed_home.join(".many-ai-cli"),
        ),
        None => RuntimePaths::production(installed_home),
    }
}

/// Own only this command's signal waiter. Dropping it never changes another
/// task's cancellation token or terminates a process by name/PID-file discovery.
pub struct ShutdownSignals {
    task: tokio::task::JoinHandle<()>,
}
impl ShutdownSignals {
    pub fn install(cancel: Cancellation) -> io::Result<Self> {
        #[cfg(unix)]
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let task = tokio::spawn(async move {
            let signal = async {
                #[cfg(unix)]
                tokio::select! {
                    result = tokio::signal::ctrl_c() => result,
                    received = terminate.recv() => received.ok_or_else(|| io::Error::other("shutdown signal stream closed")),
                }
                #[cfg(not(unix))]
                tokio::signal::ctrl_c().await
            };
            wait_for_shutdown_signal(signal, cancel).await;
        });
        Ok(Self { task })
    }
}
async fn wait_for_shutdown_signal(
    signal: impl std::future::Future<Output = io::Result<()>>,
    cancel: Cancellation,
) {
    // A failed signal registration or a closed stream is not a user signal.
    if signal.await.is_ok() {
        cancel.cancel();
    }
}
impl Drop for ShutdownSignals {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        future::Future,
        task::{Context, Poll, Waker},
    };

    async fn signal_result(result: io::Result<()>, expected_cancelled: bool) {
        let cancel = Cancellation::default();
        let mut wait = std::pin::pin!(cancel.cancelled());
        assert_eq!(
            wait.as_mut().poll(&mut Context::from_waker(Waker::noop())),
            Poll::Pending
        );
        let (send, receive) = tokio::sync::oneshot::channel();
        let owned_cancel = cancel.clone();
        let task = tokio::spawn(async move {
            wait_for_shutdown_signal(async { receive.await.unwrap() }, owned_cancel).await;
        });
        send.send(result).unwrap();
        task.await.unwrap();
        assert_eq!(cancel.is_cancelled(), expected_cancelled);
        assert_eq!(
            wait.as_mut().poll(&mut Context::from_waker(Waker::noop())),
            if expected_cancelled {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        );
    }

    #[tokio::test]
    async fn received_signal_cancels_the_owned_shutdown_waiter() {
        signal_result(Ok(()), true).await;
    }

    #[tokio::test]
    async fn signal_registration_failure_does_not_cancel_the_owned_shutdown_waiter() {
        signal_result(
            Err(io::Error::other("synthetic registration failure")),
            false,
        )
        .await;
    }
}
