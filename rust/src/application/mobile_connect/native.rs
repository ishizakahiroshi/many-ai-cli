//! Native probes use only the actor's supplied lookup/environment context.
//! Trial execution is deliberately disabled, including read-only service probes:
//! invoking a real Tailscale client would address the user's daemon/configuration.
use super::{CommandOutput, MobileIo, Program};
use crate::{
    config::RuntimePaths,
    process::{
        Cancellation, ExitOutcome, ManagedProcess, ProcessPlan, SpawnOptions,
        execpath::{NativeFs, Platform, Resolver},
    },
    proto::core::CoreFuture,
};
use std::{collections::BTreeMap, ffi::OsString, io, path::PathBuf, time::Duration};
pub struct NativeMobileIo {
    paths: RuntimePaths,
    environment: Vec<String>,
    cwd: PathBuf,
    hub_cancel: Cancellation,
}
impl NativeMobileIo {
    pub fn new(
        paths: RuntimePaths,
        environment: Vec<String>,
        cwd: PathBuf,
        actor_home: PathBuf,
        hub_cancel: Cancellation,
    ) -> io::Result<Self> {
        if !cwd.is_absolute() || !cwd.is_dir() || !actor_home.is_absolute() {
            return Err(io::Error::other("mobile probe context must be absolute"));
        }
        let mut normalized = BTreeMap::<String, (String, String)>::new();
        for entry in environment {
            let Some((key, value)) = entry.split_once('=') else {
                return Err(io::Error::other("invalid mobile probe environment"));
            };
            if key.is_empty() || entry.contains('\0') {
                return Err(io::Error::other("invalid mobile probe environment"));
            }
            normalized.insert(
                if cfg!(windows) {
                    key.to_ascii_uppercase()
                } else {
                    key.into()
                },
                (key.into(), value.into()),
            );
        }
        // Retain explicit actor-specific vendor/config directories; provide only
        // actor-home defaults, never fill missing values from ambient process env.
        for (key, value) in [
            ("HOME", actor_home.clone()),
            ("USERPROFILE", actor_home.clone()),
            ("APPDATA", actor_home.join("AppData/Roaming")),
            ("LOCALAPPDATA", actor_home.join("AppData/Local")),
            ("XDG_CONFIG_HOME", actor_home.join(".config")),
        ] {
            normalized
                .entry(key.into())
                .or_insert_with(|| (key.into(), value.to_string_lossy().into_owned()));
        }
        let environment = normalized
            .into_values()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        Ok(Self {
            paths,
            environment,
            cwd,
            hub_cancel,
        })
    }
    fn env(&self, key: &str) -> Option<&str> {
        self.environment.iter().find_map(|entry| {
            entry
                .split_once('=')
                .filter(|(k, _)| {
                    if cfg!(windows) {
                        k.eq_ignore_ascii_case(key)
                    } else {
                        *k == key
                    }
                })
                .map(|(_, v)| v)
        })
    }
    fn executable(&self, program: Program) -> Option<PathBuf> {
        if self.paths.is_trial() {
            return None;
        }
        let resolver = Resolver::new(Platform::native(), &self.environment, &self.cwd, &NativeFs);
        let lookup = |name: &str| resolver.look_path(name).ok().map(PathBuf::from);
        let present = |path: PathBuf| {
            let path = if path.is_absolute() {
                path
            } else {
                self.cwd.join(path)
            };
            std::fs::metadata(&path)
                .ok()
                .filter(|m| !m.is_dir())
                .map(|_| path)
        };
        match program {
            Program::Tailscale => {
                if cfg!(windows) {
                    for key in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
                        if let Some(base) = self.env(key).filter(|v| !v.is_empty())
                            && let Some(path) =
                                present(PathBuf::from(base).join("Tailscale/tailscale.exe"))
                        {
                            return Some(path);
                        }
                    }
                    lookup("tailscale.exe").or_else(|| lookup("tailscale"))
                } else {
                    lookup("tailscale").or_else(|| {
                        [
                            "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
                            "/usr/bin/tailscale",
                            "/usr/local/bin/tailscale",
                            "/opt/homebrew/bin/tailscale",
                        ]
                        .into_iter()
                        .find_map(|p| present(p.into()))
                    })
                }
            }
            Program::ServiceControl => lookup("sc"),
            Program::Systemctl => lookup("systemctl"),
            Program::Pgrep => lookup("pgrep"),
            Program::Sshd => lookup("sshd").or_else(|| {
                [
                    "/usr/sbin/sshd",
                    "/usr/bin/sshd",
                    "/sbin/sshd",
                    "/usr/local/sbin/sshd",
                ]
                .into_iter()
                .find_map(|p| present(p.into()))
            }),
        }
    }
}
impl MobileIo for NativeMobileIo {
    fn available(&self, program: Program) -> bool {
        self.executable(program).is_some()
    }
    fn windows(&self) -> bool {
        cfg!(windows)
    }
    fn run<'a>(
        &'a self,
        program: Program,
        args: &'a [&'a str],
        combined: bool,
        cancel: &'a Cancellation,
    ) -> CoreFuture<'a, CommandOutput> {
        Box::pin(async move {
            if self.hub_cancel.is_cancelled() || cancel.is_cancelled() {
                return CommandOutput::default();
            }
            let Some(executable) = self.executable(program) else {
                return CommandOutput::default();
            };
            let env = self
                .environment
                .iter()
                .filter_map(|entry| entry.split_once('='))
                .map(|(k, v)| (OsString::from(k), Some(OsString::from(v))))
                .collect();
            let plan = ProcessPlan {
                executable,
                args: args.iter().map(|arg| OsString::from(*arg)).collect(),
                cwd: self.cwd.clone(),
                env,
                stdin: vec![],
                timeout: Duration::from_secs(5),
                output_cap: 64 * 1024,
                pipe_drain_timeout: Duration::from_millis(500),
            };
            let (mut process, _events) = ManagedProcess::spawn_owned_with_options(
                plan,
                16,
                SpawnOptions {
                    combined_output: combined,
                    stdin_null: true,
                    env_clear: true,
                    no_window: true,
                    ..Default::default()
                },
            );
            let result = tokio::select! {result=process.wait()=>Some(result),_=cancel.cancelled()=>None,_=self.hub_cancel.cancelled()=>None};
            let result = if let Some(result) = result {
                result
            } else {
                process.close();
                process.wait().await
            };
            let Ok(output) = result else {
                return CommandOutput::default();
            };
            CommandOutput {
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                success: matches!(output.outcome, ExitOutcome::Exited { code: Some(0), .. })
                    && !output.stdout_truncated
                    && !output.stderr_truncated
                    && !output.pipes_forced_closed,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn exact_actor_environment_and_precancelled_owner_never_resolve_or_execute_daemons() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("actor");
        let paths = RuntimePaths::production(&home).unwrap();
        let hub = Cancellation::default();
        hub.cancel();
        let io = NativeMobileIo::new(
            paths,
            vec!["ONLY_EXPLICIT=synthetic".into()],
            root.path().into(),
            home.clone(),
            hub,
        )
        .unwrap();
        assert_eq!(io.env("PATH"), None);
        assert_eq!(io.env("ONLY_EXPLICIT"), Some("synthetic"));
        assert_eq!(io.env("HOME"), Some(home.to_str().unwrap()));
        assert_eq!(io.env("USERPROFILE"), Some(home.to_str().unwrap()));
        assert!(
            !io.run(
                Program::Tailscale,
                &["serve", "--bg", "49327"],
                false,
                &Cancellation::default()
            )
            .await
            .success
        );
    }
}
