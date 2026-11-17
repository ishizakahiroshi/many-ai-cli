//! Native tray actions keep a real Hub child alive after tray shutdown.
use super::*;
use crate::{
    application::{hub_runtime::RuntimeLedger, maintenance_cli::stop::StopCommand},
    config::{ConfigStore, RuntimePaths},
};
use std::{
    path::PathBuf,
    process::{Command, Stdio},
};
pub struct NativeTrayHooks {
    pub paths: RuntimePaths,
    pub config: Arc<ConfigStore>,
    pub executable: PathBuf,
    pub cwd: PathBuf,
    pub environment: Vec<String>,
    pub stop: Arc<StopCommand>,
    /// Used only by `stop_for_migration`. Menu stops keep `stop`.
    pub migration_stop: Arc<StopCommand>,
    pub preferred_port: Option<u16>,
    /// Port carried by `REPLACE_REQUEST`. Overrides `preferred_port` for this process.
    pub migration_port: std::sync::Mutex<Option<u16>>,
}
impl NativeTrayHooks {
    /// The console Hub port when a replace was forwarded, otherwise the port this tray was given.
    fn launch_port(&self) -> Option<u16> {
        self.migration_port
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .or(self.preferred_port)
    }
}
impl TrayHooks for NativeTrayHooks {
    fn requested_port(&self) -> Option<u16> {
        self.preferred_port
    }
    fn accept_migration_port(&self, port: u16) {
        if port > 0 {
            *self
                .migration_port
                .lock()
                .unwrap_or_else(|poison| poison.into_inner()) = Some(port);
        }
    }
    fn instance_port(&self) -> Option<u16> {
        self.launch_port().or_else(|| {
            self.config.snapshot().ok().and_then(|snapshot| {
                u16::try_from(snapshot.config.hub.port)
                    .ok()
                    .filter(|port| *port > 0)
            })
        })
    }
    fn instance_key(&self) -> String {
        super::instance_key(self.paths.root())
    }
    fn setup(&self) -> CoreFuture<'_, io::Result<()>> {
        Box::pin(async move {
            if self.paths.is_trial() {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "trial tray requires synthetic setup",
                ));
            }
            let executable = self.executable.canonicalize()?;
            let mut command = Command::new(executable);
            command
                .arg("setup")
                .current_dir(&self.cwd)
                .env_clear()
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            if let Some(port) = self.launch_port() {
                command.args(["--port", &port.to_string()]);
            }
            for (key, value) in self.environment.iter().filter_map(|s| s.split_once('=')) {
                command.env(key, value);
            }
            command.env_remove(crate::process::wrapper_startup::STARTUP_JOB_ENV);
            command.env_remove("MANY_AI_CLI_INTERNAL_SPAWN_PROOF");
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                command.creation_flags(
                    windows_sys::Win32::System::Threading::CREATE_NEW_PROCESS_GROUP
                        | windows_sys::Win32::System::Threading::CREATE_NO_WINDOW,
                );
            }
            let status = tokio::task::spawn_blocking(move || command.status())
                .await
                .map_err(io::Error::other)??;
            status
                .success()
                .then_some(())
                .ok_or_else(|| io::Error::other(format!("setup exited with {status}")))
        })
    }
    fn running_url(&self) -> CoreFuture<'_, io::Result<Option<String>>> {
        Box::pin(async move {
            let snapshot = self.config.snapshot().map_err(io::Error::other)?;
            let proof = crate::hub::pin::cli_local_proof(&snapshot.config);
            let expected_port = self.launch_port();
            let port = RuntimeLedger::open(&self.paths)?
                .running_port(
                    expected_port
                        .map(i64::from)
                        .unwrap_or(snapshot.config.hub.port),
                    &snapshot.config.token,
                    proof.as_deref(),
                )
                .await;
            if let (Some(expected), Some(actual)) = (expected_port, port)
                && expected != actual
            {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    format!(
                        "同じ保存先の Hub はポート {actual} で起動しています。アイコンの指定ポート {expected} を確認してください。自動で切り替えたり停止したりはしません。"
                    ),
                ));
            }
            // The browser gets a one-time launch code, not the token (U04).
            let Some(port) = port else {
                return Ok(None);
            };
            let code = crate::application::hub_runtime::request_launch_code(
                port,
                &snapshot.config.token,
                proof.as_deref(),
            )
            .await;
            Ok(Some(crate::application::hub_runtime::dashboard_url(
                port,
                code.as_deref(),
            )))
        })
    }
    fn start_hub(&self) -> CoreFuture<'_, io::Result<()>> {
        Box::pin(async move {
            if self.paths.is_trial() {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "trial tray requires injected synthetic Hub startup",
                ));
            }
            let executable = self.executable.canonicalize()?;
            let (send, receive) = std::sync::mpsc::sync_channel::<std::process::Child>(1);
            std::thread::Builder::new()
                .name("many-ai-tray-hub-reaper".into())
                .spawn(move || {
                    if let Ok(mut child) = receive.recv() {
                        let _ = child.wait();
                    }
                })?;
            let mut command = Command::new(executable);
            command
                .arg("serve")
                .current_dir(&self.cwd)
                .env_clear()
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            if let Some(port) = self.launch_port() {
                command.args(["--port", &port.to_string(), "--strict-port"]);
            }
            for (key, value) in self.environment.iter().filter_map(|s| s.split_once('=')) {
                command.env(key, value);
            }
            command.env_remove(crate::process::wrapper_startup::STARTUP_JOB_ENV);
            command.env_remove("MANY_AI_CLI_INTERNAL_SPAWN_PROOF");
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                command.creation_flags(
                    windows_sys::Win32::System::Threading::CREATE_NEW_PROCESS_GROUP
                        | windows_sys::Win32::System::Threading::CREATE_NO_WINDOW,
                );
            }
            send.send(command.spawn()?)
                .map_err(|_| io::Error::other("tray Hub reaper ended"))?;
            Ok(())
        })
    }
    fn open_url<'a>(&'a self, url: &'a str) -> CoreFuture<'a, io::Result<()>> {
        Box::pin(async move {
            if self.paths.is_trial() {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "trial cannot open host browser",
                ));
            }
            #[cfg(windows)]
            {
                let url = url.to_owned();
                tokio::task::spawn_blocking(move || {
                    use windows_sys::Win32::UI::{
                        Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL,
                    };
                    let verb: Vec<u16> = "open".encode_utf16().chain(Some(0)).collect();
                    let url: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
                    // ShellExecute resolves the user's default browser without an
                    // intermediary command shell or a token-bearing diagnostic.
                    let result = unsafe {
                        ShellExecuteW(
                            std::ptr::null_mut(),
                            verb.as_ptr(),
                            url.as_ptr(),
                            std::ptr::null(),
                            std::ptr::null(),
                            SW_SHOWNORMAL,
                        )
                    } as isize;
                    if result <= 32 {
                        return Err(io::Error::other(format!(
                            "Windows browser dispatch failed ({result})"
                        )));
                    }
                    Ok(())
                })
                .await
                .map_err(io::Error::other)?
            }
            #[cfg(not(windows))]
            crate::launcher::open_browser(url)
        })
    }
    fn stop_hub(&self) -> CoreFuture<'_, io::Result<()>> {
        Box::pin(async move { self.stop.run(&Cancellation::default()).await })
    }
    fn stop_for_migration(&self) -> CoreFuture<'_, io::Result<()>> {
        Box::pin(async move { self.migration_stop.run(&Cancellation::default()).await })
    }
}
