//! Explicit on-demand executor over the shared managed process owner.
//! Construction does not start a CLI, inspect credentials, or create timers.
use super::{CliVersionCommand, CliVersionExecutor, CliVersionFailure, CliVersionProcessOutput};
use crate::{
    config::RuntimePaths,
    process::{
        ExitOutcome, ManagedProcess, ProcessEvent, ProcessPlan, SpawnOptions, execpath::Platform,
    },
    proto::core::CoreFuture,
};
use std::{collections::BTreeMap, ffi::OsString, path::Path, time::Duration};
use tokio::sync::broadcast;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TrialVersionPolicy {
    #[default]
    Disabled,
    /// Explicitly allow only a selected owned synthetic executable in the trial.
    OwnedSynthetic,
}
pub struct NativeCliVersionExecutor {
    paths: RuntimePaths,
    trial_policy: TrialVersionPolicy,
}
impl NativeCliVersionExecutor {
    pub fn new(paths: RuntimePaths, trial_policy: TrialVersionPolicy) -> Self {
        Self {
            paths,
            trial_policy,
        }
    }
    fn prepare(&self, command: &CliVersionCommand) -> Result<ProcessPlan, CliVersionFailure> {
        if command.platform != Platform::native() {
            return Err(CliVersionFailure::InvalidPlatform);
        }
        if !command.cwd.is_absolute()
            || !command.cwd.is_dir()
            || command.executable.contains('\0')
            || command.args.iter().any(|arg| arg.contains('\0'))
            || command.timeout.is_zero()
            || tokio::time::Instant::now()
                .checked_add(command.timeout)
                .is_none()
        {
            return Err(CliVersionFailure::InvalidContext);
        }
        let executable = Path::new(&command.executable);
        let executable = if executable.is_absolute() {
            executable.to_owned()
        } else {
            command.cwd.join(executable)
        };
        let mut env = exact_environment(&command.environment, command.platform)?;
        if self.paths.is_trial() {
            if self.trial_policy != TrialVersionPolicy::OwnedSynthetic {
                return Err(CliVersionFailure::TrialDisabled);
            }
            let check = |path: &Path| {
                crate::profile::subscriptions::check_path(&self.paths, path)
                    .map_err(|_| CliVersionFailure::TrialBoundary)
            };
            check(&command.cwd)?;
            check(&executable)?;
            for (key, relative) in [
                ("HOME", "home"),
                ("USERPROFILE", "home"),
                ("APPDATA", "home/appdata"),
                ("LOCALAPPDATA", "home/localappdata"),
                ("XDG_CONFIG_HOME", "home/config"),
                ("XDG_CACHE_HOME", "home/cache"),
                ("XDG_DATA_HOME", "home/data"),
                ("XDG_STATE_HOME", "home/state"),
                ("TMPDIR", "tmp"),
                ("TMP", "tmp"),
                ("TEMP", "tmp"),
                ("CODEX_HOME", "home/codex"),
                ("CLAUDE_CONFIG_DIR", "home/claude"),
                ("GROK_HOME", "home/grok"),
            ] {
                let path = self.paths.root().join(relative);
                check(&path)?;
                // Remove differently cased Windows overrides before setting the scope.
                if command.platform == Platform::Windows {
                    env.retain(|entry, _| !entry.to_string_lossy().eq_ignore_ascii_case(key));
                }
                env.insert(key.into(), Some(path.into_os_string()));
            }
        }
        Ok(ProcessPlan {
            executable,
            args: command.args.iter().map(Into::into).collect(),
            cwd: command.cwd.clone(),
            env,
            stdin: vec![],
            timeout: command.timeout,
            output_cap: command.output_cap,
            pipe_drain_timeout: Duration::from_millis(500),
        })
    }
}
fn exact_environment(
    entries: &[String],
    platform: Platform,
) -> Result<BTreeMap<OsString, Option<OsString>>, CliVersionFailure> {
    let mut normalized = BTreeMap::<String, (OsString, OsString)>::new();
    for entry in entries {
        let Some((key, value)) = entry.split_once('=') else {
            return Err(CliVersionFailure::InvalidEnvironment);
        };
        if key.is_empty() || key.contains('\0') || value.contains('\0') {
            return Err(CliVersionFailure::InvalidEnvironment);
        }
        let identity = if platform == Platform::Windows {
            key.to_uppercase()
        } else {
            key.into()
        };
        normalized.insert(identity, (key.into(), value.into()));
    }
    Ok(normalized
        .into_values()
        .map(|(key, value)| (key, Some(value)))
        .collect())
}
fn record_started(
    event: Result<ProcessEvent, broadcast::error::RecvError>,
    started: &mut bool,
) -> bool {
    match event {
        Ok(ProcessEvent::Started { .. }) | Err(broadcast::error::RecvError::Lagged(_)) => {
            *started = true;
            true
        }
        Ok(ProcessEvent::Output { .. }) => {
            *started = true;
            true
        }
        Err(broadcast::error::RecvError::Closed) => false,
    }
}
impl CliVersionExecutor for NativeCliVersionExecutor {
    fn execute(
        &self,
        command: CliVersionCommand,
    ) -> CoreFuture<'_, Result<CliVersionProcessOutput, CliVersionFailure>> {
        Box::pin(async move {
            let plan = self.prepare(&command)?;
            let (mut process, mut events) = ManagedProcess::spawn_owned_with_options(
                plan,
                16,
                SpawnOptions {
                    combined_output: true,
                    stdin_null: true,
                    env_clear: true,
                    no_window: true,
                    ..Default::default()
                },
            );
            let mut started = false;
            let output = {
                let wait = process.wait();
                tokio::pin!(wait);
                let mut events_open = true;
                loop {
                    tokio::select! {
                        event = events.recv(), if events_open => { events_open = record_started(event, &mut started); },
                        output = &mut wait => break output,
                    }
                }
            };
            // A fast child may complete before the event branch was polled.
            while let Ok(_) | Err(broadcast::error::TryRecvError::Lagged(_)) = events.try_recv() {
                started = true;
            }
            #[cfg(test)]
            if let Err(error) = &output {
                let thread = std::thread::current();
                let suffix_case = thread.name()
                    == Some(
                        "hub::cli_version::native::tests::owned_native_combined_output_null_stdin_exact_env_exit_and_cap",
                    );
                crate::logging::write_diagnostic(&format!(
                    "cli_version_process_diagnostic suffix_case={suffix_case} started={started} io_kind={:?}\n",
                    error.kind()
                ));
            }
            let output = match output {
                Ok(output) => output,
                Err(_) if !started => {
                    return Ok(CliVersionProcessOutput {
                        start_failed: true,
                        ..Default::default()
                    });
                }
                Err(_) => return Err(CliVersionFailure::ProcessUnavailable),
            };
            if output.pipes_forced_closed {
                return Err(CliVersionFailure::ProcessUnavailable);
            }
            let timed_out = matches!(output.outcome, ExitOutcome::TimedOut);
            let exit_code = match output.outcome {
                ExitOutcome::Exited { code, .. } => code.unwrap_or(-1),
                ExitOutcome::TimedOut => {
                    if cfg!(windows) {
                        1
                    } else {
                        -1
                    }
                }
                ExitOutcome::Cancelled => return Err(CliVersionFailure::OwnerDropped),
            };
            Ok(CliVersionProcessOutput {
                output: output.stdout,
                exit_code,
                timed_out,
                start_failed: false,
            })
        })
    }
}

#[cfg(test)]
mod tests;
