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
}
impl TrayHooks for NativeTrayHooks {
    fn running_url(&self) -> CoreFuture<'_, io::Result<Option<String>>> {
        Box::pin(async move {
            let snapshot = self.config.snapshot().map_err(io::Error::other)?;
            let port = RuntimeLedger::open(&self.paths)?
                .running_port(snapshot.config.hub.port, &snapshot.config.token)
                .await;
            port.map(|port| {
                let mut url = url::Url::parse(&format!("http://127.0.0.1:{port}/"))
                    .map_err(io::Error::other)?;
                url.query_pairs_mut()
                    .append_pair("token", &snapshot.config.token);
                Ok(url.into())
            })
            .transpose()
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
            for (key, value) in self.environment.iter().filter_map(|s| s.split_once('=')) {
                command.env(key, value);
            }
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
            crate::launcher::open_browser(url)
        })
    }
    fn stop_hub(&self) -> CoreFuture<'_, io::Result<()>> {
        Box::pin(async move { self.stop.run(&Cancellation::default()).await })
    }
}
