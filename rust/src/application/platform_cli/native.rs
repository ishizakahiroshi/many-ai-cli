use super::*;
use crate::{
    application::diagnostics::{DiagnosticIo, NativeDiagnosticIo},
    files::safe_fs::Dir,
};
use std::{
    process::{Command, Stdio},
    time::Duration,
};
pub struct NativePlatformIo {
    pub actor: NativeDiagnosticIo,
}
pub fn quote(value: &str) -> io::Result<String> {
    if value.contains(['\0', '\r', '\n']) {
        return Err(io::Error::other("invalid Windows shortcut value"));
    }
    Ok(format!("'{}'", value.replace('\'', "''")))
}
impl NativePlatformIo {
    fn checked(&self, path: &Path) -> io::Result<()> {
        if !self.actor.paths.is_trial() {
            return Ok(());
        }
        self.actor.paths.relative_to_selected_root(path).map(|_| ())
    }
    fn directory(&self, path: &Path, create: bool) -> io::Result<Dir> {
        if !self.actor.paths.is_trial() {
            return if create {
                Dir::open_or_create_private(path)
            } else {
                Dir::open(path)
            };
        }
        let relative = self.actor.paths.relative_to_selected_root(path)?;
        let mut directory = Dir::open(self.actor.paths.root())?;
        for component in relative.components() {
            let std::path::Component::Normal(name) = component else {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "unclean trial platform path",
                ));
            };
            directory = directory.child_dir(
                name.to_str()
                    .ok_or_else(|| io::Error::other("invalid platform directory component"))?,
                create,
            )?;
        }
        Ok(directory)
    }
    fn file_parent(&self, path: &Path) -> io::Result<(Dir, String)> {
        self.checked(path)?;
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::other("platform path lacks parent"))?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| io::Error::other("invalid platform filename"))?;
        Ok((self.directory(parent, false)?, name.to_owned()))
    }
    pub async fn locations(
        &self,
        executable: PathBuf,
        home: PathBuf,
        cancel: &Cancellation,
    ) -> io::Result<Locations> {
        if TargetPlatform::native() != TargetPlatform::Windows {
            let desktop = if TargetPlatform::native() == TargetPlatform::MacOS {
                home.join("Desktop")
            } else {
                let fallback = home.join("Desktop");
                self.linux_desktop(cancel)
                    .await
                    .or_else(|| fallback.is_dir().then_some(fallback))
                    .unwrap_or_default()
            };
            return Ok(Locations {
                config: home.join(".many-ai-cli"),
                data: home.join(".local/share/applications"),
                desktop,
                startup: None,
                executable,
            });
        }
        let env = |name: &str| {
            self.actor.environment.iter().find_map(|item| {
                item.split_once('=')
                    .filter(|(key, _)| key.eq_ignore_ascii_case(name))
                    .map(|(_, value)| PathBuf::from(value))
            })
        };
        let local =
            env("LOCALAPPDATA").ok_or_else(|| io::Error::other("LOCALAPPDATA is not set"))?;
        let desktop = self
            .folder("Desktop", cancel)
            .await
            .unwrap_or_else(|| home.join("Desktop"));
        let startup = self.folder("Startup", cancel).await.or_else(|| {
            env("APPDATA")
                .filter(|value| !value.as_os_str().is_empty())
                .map(|p| p.join("Microsoft/Windows/Start Menu/Programs/Startup"))
        });
        Ok(Locations {
            config: home.join(".many-ai-cli"),
            data: local.join("ManyAICLI"),
            desktop,
            startup,
            executable,
        })
    }
    async fn linux_desktop(&self, cancel: &Cancellation) -> Option<PathBuf> {
        let executable = self.actor.look_path("xdg-user-dir").ok()?;
        let output = self
            .actor
            .command(
                &executable,
                vec!["DESKTOP".into()],
                &self.actor.cwd,
                Duration::ZERO,
                cancel,
            )
            .await
            .ok()?;
        if !output.success {
            return None;
        }
        let value = String::from_utf8(output.bytes).ok()?;
        let path = PathBuf::from(value.trim());
        (!value.trim().is_empty() && path.exists()).then_some(path)
    }
    async fn folder(&self, name: &str, cancel: &Cancellation) -> Option<PathBuf> {
        let executable = self.actor.look_path("powershell").ok()?;
        let output = self
            .actor
            .command(
                &executable,
                vec![
                    "-NoProfile".into(),
                    "-NonInteractive".into(),
                    "-Command".into(),
                    format!("[Environment]::GetFolderPath('{}')", name),
                ],
                &self.actor.cwd,
                Duration::ZERO,
                cancel,
            )
            .await
            .ok()?;
        if !output.success {
            return None;
        }
        let value = String::from_utf8(output.bytes).ok()?;
        (!value.trim().is_empty()).then(|| PathBuf::from(value.trim()))
    }
}

impl PlatformIo for NativePlatformIo {
    fn prepare_directory(&self, path: &Path) -> io::Result<()> {
        self.directory(path, true).map(|_| ())
    }
    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        let (parent, name) = self.file_parent(path)?;
        parent.replace(&name, bytes, 0o644)
    }
    fn make_executable(&self, path: &Path) -> io::Result<()> {
        self.checked(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let (parent, name) = self.file_parent(path)?;
            parent
                .open_file(&name, false)?
                .set_permissions(std::fs::Permissions::from_mode(0o755))
        }
        #[cfg(not(unix))]
        {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Unix executable permissions require a Unix filesystem owner",
            ))
        }
    }
    fn exists(&self, path: &Path) -> bool {
        if !self.actor.paths.is_trial() {
            return path.exists();
        }
        self.directory(path, false).is_ok()
            || self
                .file_parent(path)
                .and_then(|(parent, name)| parent.open_file(&name, false))
                .is_ok()
    }
    fn remove_file(&self, path: &Path) -> io::Result<()> {
        let (parent, name) = self.file_parent(path)?;
        parent.remove_file(&name)
    }
    fn remove_data(&self, path: &Path) -> io::Result<bool> {
        let (parent, name) = self.file_parent(path)?;
        match parent.child_dir(&name, false) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error),
            Ok(_) => {}
        }
        // std's recursive deletion removes reparse points themselves; it never
        // traverses subscription junctions into the user's original CLI home.
        // The held parent denies ancestor replacement on Windows.
        #[cfg(windows)]
        std::fs::remove_dir_all(parent.path().join(name))?;
        #[cfg(not(windows))]
        parent.remove_tree(&name)?;
        Ok(true)
    }
    fn shortcut<'a>(
        &'a self,
        path: &'a Path,
        exe: &'a Path,
        cancel: &'a Cancellation,
    ) -> CoreFuture<'a, io::Result<()>> {
        Box::pin(async move {
            self.checked(path)?;
            if self.actor.paths.is_trial() {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "trial shortcuts require injected synthetic COM owner",
                ));
            }
            let convert = |p: &Path| {
                quote(
                    p.to_str()
                        .ok_or_else(|| io::Error::other("shortcut path is not UTF-8"))?,
                )
            };
            let script = format!(
                "$s=(New-Object -ComObject WScript.Shell).CreateShortcut({});$s.TargetPath={};$s.Arguments='tray';$s.WorkingDirectory={};$s.WindowStyle=7;$s.IconLocation={};$s.Save()",
                convert(path)?,
                convert(exe)?,
                convert(
                    exe.parent()
                        .ok_or_else(|| io::Error::other("executable lacks parent"))?
                )?,
                quote(&format!("{},0", exe.display()))?
            );
            let executable = self.actor.look_path("powershell")?;
            let output = self
                .actor
                .command(
                    &executable,
                    vec![
                        "-NoProfile".into(),
                        "-NonInteractive".into(),
                        "-Command".into(),
                        script,
                    ],
                    &self.actor.cwd,
                    Duration::ZERO,
                    cancel,
                )
                .await?;
            if output.success {
                Ok(())
            } else {
                Err(io::Error::other("Windows shortcut creation failed"))
            }
        })
    }
    fn purge<'a>(
        &'a self,
        exe: &'a Path,
        cancel: &'a Cancellation,
    ) -> CoreFuture<'a, io::Result<()>> {
        Box::pin(async move {
            if cancel.is_cancelled() {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "uninstall cancelled",
                ));
            }
            if TargetPlatform::native() != TargetPlatform::Windows {
                return self.remove_file(exe);
            }
            if self.actor.paths.is_trial() {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "trial cannot schedule binary deletion",
                ));
            }
            let executable = self.actor.look_path("powershell")?;
            let script = format!(
                "Start-Sleep -Seconds 2; Remove-Item -LiteralPath {} -Force",
                quote(
                    exe.to_str()
                        .ok_or_else(|| io::Error::other("executable is not UTF-8"))?
                )?
            );
            // Successful launch deliberately survives CLI exit; a dedicated OS thread owns Wait.
            let (send, receive) = std::sync::mpsc::sync_channel::<std::process::Child>(1);
            std::thread::Builder::new()
                .name("many-ai-uninstall-reaper".into())
                .spawn(move || {
                    if let Ok(mut child) = receive.recv() {
                        let _ = child.wait();
                    }
                })?;
            let mut command = Command::new(executable);
            command
                .args([
                    "-WindowStyle",
                    "Hidden",
                    "-NonInteractive",
                    "-Command",
                    &script,
                ])
                .env_clear()
                .current_dir(&self.actor.cwd)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            for (key, value) in self
                .actor
                .environment
                .iter()
                .filter_map(|s| s.split_once('='))
            {
                command.env(key, value);
            }
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
            }
            send.send(command.spawn()?)
                .map_err(|_| io::Error::other("uninstall reaper ended"))?;
            Ok(())
        })
    }
}
