//! Starts the agent as a child process and connects the client to its stdio.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};

use super::client::{AcpClient, AcpEvents, Incoming};
use crate::process::{
    ExitOutcome, ManagedProcess, OutputStream, ProcessEvent, ProcessPlan, SpawnOptions,
};

/// Environment variables the agent may inherit. Everything else is dropped
/// (`SpawnOptions::env_clear`), so the Hub's own variables and anything that
/// looks like a secret never reach the child. The agent finds its own signed-in
/// state through the user's home and application-data folders.
const ENV_ALLOWLIST: &[&str] = &[
    "PATH",
    "PATHEXT",
    "SYSTEMROOT",
    "SYSTEMDRIVE",
    "WINDIR",
    "COMSPEC",
    "TEMP",
    "TMP",
    "TMPDIR",
    "HOME",
    "USERPROFILE",
    "HOMEDRIVE",
    "HOMEPATH",
    "APPDATA",
    "LOCALAPPDATA",
    "PROGRAMDATA",
    "PROGRAMFILES",
    "PROGRAMFILES(X86)",
    "USER",
    "USERNAME",
    "LANG",
    "LC_ALL",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_STATE_HOME",
    "XDG_CACHE_HOME",
];

/// Known flags that make the agent approve tool calls on its own. A person
/// must answer permission requests, so a launch whose arguments contain one
/// (also inside a single `cmd.exe /c` string a shim was wrapped into) is
/// refused. This is a guard against a known flag, not a full check: the launch
/// definition itself is written by the user and trusted as such.
const REFUSED_ARGS: &[&str] = &["--dangerously-skip-permissions"];

/// Keeps only the allow-listed variables (names compared without case).
pub fn minimal_env(
    ambient: impl IntoIterator<Item = (OsString, OsString)>,
) -> BTreeMap<OsString, Option<OsString>> {
    ambient
        .into_iter()
        .filter(|(name, _)| {
            let upper = name.to_string_lossy().to_uppercase();
            ENV_ALLOWLIST.contains(&upper.as_str())
        })
        .map(|(name, value)| (name, Some(value)))
        .collect()
}

pub struct AcpProcessSpec {
    /// Absolute path of the agent executable.
    pub executable: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
}

fn describe_exit(outcome: &io::Result<crate::process::ProcessOutput>) -> String {
    match outcome {
        Ok(output) => match &output.outcome {
            ExitOutcome::Exited {
                code: Some(code), ..
            } => format!("the agent process exited (code {code})"),
            ExitOutcome::Exited { signal, .. } => {
                format!("the agent process exited (signal {signal:?})")
            }
            ExitOutcome::Cancelled => "the agent process was stopped".into(),
            ExitOutcome::TimedOut => "the agent process timed out".into(),
        },
        Err(error) => format!("the agent process ended: {error}"),
    }
}

/// Starts the agent and returns the client. Must run inside a Tokio runtime.
/// Dropping every clone of the client stops the process.
pub fn spawn_process(spec: AcpProcessSpec) -> io::Result<(AcpClient, AcpEvents)> {
    if spec.args.iter().any(|arg| {
        let arg = arg.to_string_lossy();
        REFUSED_ARGS.iter().any(|refused| arg.contains(refused))
    }) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "an argument that approves tool calls automatically is not allowed",
        ));
    }
    let plan = ProcessPlan {
        executable: spec.executable,
        args: spec.args,
        cwd: spec.cwd,
        env: minimal_env(std::env::vars_os()),
        stdin: vec![],
        timeout: Duration::ZERO,
        // The pump below reads the event stream; the captured copy is unused.
        output_cap: 64 * 1024,
        pipe_drain_timeout: Duration::from_secs(2),
    };
    let (mut child, input, mut events) = ManagedProcess::spawn_interactive_owned_with_options(
        plan,
        1024,
        SpawnOptions {
            no_window: true,
            stderr_null: true,
            env_clear: true,
            ..Default::default()
        },
    );
    let (out_tx, mut out_rx) = mpsc::channel::<Vec<u8>>(64);
    let (in_tx, in_rx) = mpsc::channel::<Incoming>(256);
    let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        let mut stopped = false;
        let reason = loop {
            tokio::select! {
                _ = &mut stop_rx, if !stopped => {
                    stopped = true;
                    child.close();
                }
                line = out_rx.recv(), if !stopped => match line {
                    Some(bytes) => {
                        if let Err(error) = input.write(bytes).await {
                            child.close();
                            break format!("writing to the agent failed: {error}");
                        }
                    }
                    None => {
                        stopped = true;
                        child.close();
                    }
                },
                event = events.recv() => match event {
                    Ok(ProcessEvent::Output { stream: OutputStream::Stdout, bytes }) => {
                        if in_tx.send(Incoming::Bytes(bytes)).await.is_err() {
                            child.close();
                            break "the client is gone".to_string();
                        }
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        child.close();
                        break "the agent's output could not be read in time".to_string();
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        let outcome = tokio::time::timeout(Duration::from_secs(5), child.wait())
                            .await
                            .unwrap_or_else(|_| Err(io::Error::other("did not exit in time")));
                        break describe_exit(&outcome);
                    }
                },
            }
        };
        let _ = in_tx.send(Incoming::Closed(reason)).await;
        child.close();
        let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
    });
    Ok(AcpClient::from_transport(out_tx, in_rx, Some(stop_tx)))
}
