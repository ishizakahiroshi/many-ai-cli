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
            #[cfg(unix)]
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {},
                _ = terminate.recv() => {},
            }
            #[cfg(not(unix))]
            let _ = tokio::signal::ctrl_c().await;
            cancel.cancel();
        });
        Ok(Self { task })
    }
}
impl Drop for ShutdownSignals {
    fn drop(&mut self) {
        self.task.abort();
    }
}
