//! Hub launch-shape detection (`GET /api/hub/launch-state`) and the detached
//! `tray --replace` spawn behind `POST /api/tray-migrate`.
use crate::{
    application::{
        diagnostics::NativeDiagnosticIo,
        main_program::MainContext,
        platform_cli::{PlatformIo, SHORTCUT, native::NativePlatformIo},
    },
    config::RuntimePaths,
    proto::core::CoreFuture,
};
use std::{
    io,
    path::PathBuf,
    process::{Command, Stdio},
    sync::Arc,
};
/// Whether this Hub was launched in a console window, whether `setup` already
/// placed its tray shortcut, and whether a tray is already running.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LaunchState {
    pub console: bool,
    pub setup: bool,
    pub tray: bool,
}
impl LaunchState {
    /// The offer is only meaningful for a console Hub that has never been set
    /// up and is not already tray-managed.
    pub fn migration_offer(&self) -> bool {
        self.console && !self.setup && !self.tray
    }
}
pub trait LaunchProbe: Send + Sync {
    fn probe(&self) -> CoreFuture<'_, io::Result<LaunchState>>;
    /// One-time serve hint. Must not spawn a shell; the settings probe still
    /// resolves the real Startup folder.
    fn migration_hint(&self) -> bool {
        false
    }
}
/// `POST /api/tray-migrate` spawns the detached `tray --replace` and answers
/// before the old Hub stops, so the response still reaches the asking page.
pub trait TrayMigration: Send + Sync {
    fn spawn_replace(&self, port: u16) -> io::Result<()>;
}
pub struct NativeLaunchProbe {
    pub paths: RuntimePaths,
    pub platform: Arc<NativePlatformIo>,
    pub executable: PathBuf,
    pub vendor_home: PathBuf,
}
impl LaunchProbe for NativeLaunchProbe {
    fn probe(&self) -> CoreFuture<'_, io::Result<LaunchState>> {
        Box::pin(async move {
            if self.paths.is_trial() {
                return Ok(LaunchState::default());
            }
            let cancel = crate::process::Cancellation::default();
            let locations = self
                .platform
                .locations(self.executable.clone(), self.vendor_home.clone(), &cancel)
                .await?;
            Ok(LaunchState {
                console: console_attached(),
                setup: locations
                    .startup
                    .is_some_and(|directory| self.platform.exists(&directory.join(SHORTCUT))),
                tray: tray_running(&self.paths),
            })
        })
    }
    fn migration_hint(&self) -> bool {
        if self.paths.is_trial() || !console_attached() || tray_running(&self.paths) {
            return false;
        }
        match startup_shortcut_from_env(&self.platform.actor.environment) {
            Some(path) => !self.platform.exists(&path),
            None => false,
        }
    }
}
#[cfg(windows)]
fn console_attached() -> bool {
    unsafe { !windows_sys::Win32::System::Console::GetConsoleWindow().is_null() }
}
#[cfg(not(windows))]
fn console_attached() -> bool {
    false
}
/// The tray's single-instance mutex names the running tray for the same home.
#[cfg(windows)]
fn tray_running(paths: &RuntimePaths) -> bool {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE},
    };
    let key = crate::application::tray::instance_key(paths.root());
    let name = crate::application::tray::windows::wide(&format!("Local\\{key}"));
    let handle = unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, 0, name.as_ptr()) };
    if handle.is_null() {
        return false;
    }
    unsafe {
        CloseHandle(handle);
    }
    true
}
#[cfg(not(windows))]
fn tray_running(_: &RuntimePaths) -> bool {
    false
}
pub struct NativeTrayMigration {
    pub executable: PathBuf,
    pub cwd: PathBuf,
    pub environment: Vec<String>,
}
impl TrayMigration for NativeTrayMigration {
    fn spawn_replace(&self, port: u16) -> io::Result<()> {
        let executable = self.executable.canonicalize()?;
        // A reaped child stays visible to its own exit path after this Hub is
        // gone; the tray window keeps its own lifetime on the desktop.
        let (send, receive) = std::sync::mpsc::sync_channel::<std::process::Child>(1);
        std::thread::Builder::new()
            .name("many-ai-tray-migrate-reaper".into())
            .spawn(move || {
                if let Ok(mut child) = receive.recv() {
                    let _ = child.wait();
                }
            })?;
        let mut command = Command::new(executable);
        command
            .args(["tray", "--replace", "--port", &port.to_string()])
            .current_dir(&self.cwd)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
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
            .map_err(|_| io::Error::other("tray migration reaper ended"))?;
        Ok(())
    }
}
pub fn probe(context: &MainContext) -> Arc<dyn LaunchProbe> {
    Arc::new(NativeLaunchProbe {
        paths: context.paths.clone(),
        platform: Arc::new(NativePlatformIo {
            actor: NativeDiagnosticIo {
                paths: context.paths.clone(),
                cwd: context.cwd.clone(),
                environment: context.environment.clone(),
            },
        }),
        executable: context.executable.clone(),
        vendor_home: context.vendor_home.clone(),
    })
}
pub fn migration(context: &MainContext) -> Arc<dyn TrayMigration> {
    Arc::new(NativeTrayMigration {
        executable: context.executable.clone(),
        cwd: context.cwd.clone(),
        environment: context.environment.clone(),
    })
}
const HINT_MARKER: &str = "tray-migration-hint-shown";
/// Startup shortcut implied by APPDATA. The serve hint uses this instead of
/// launching PowerShell; a missing APPDATA suppresses the hint.
pub(crate) fn startup_shortcut_from_env(environment: &[String]) -> Option<PathBuf> {
    let appdata = environment.iter().find_map(|item| {
        let (key, value) = item.split_once('=')?;
        key.eq_ignore_ascii_case("APPDATA")
            .then(|| PathBuf::from(value))
    })?;
    if appdata.as_os_str().is_empty() {
        return None;
    }
    Some(
        appdata
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join("Startup")
            .join(SHORTCUT),
    )
}
fn hint_dir(paths: &RuntimePaths) -> Option<crate::files::safe_fs::Dir> {
    crate::files::safe_fs::Dir::open_or_create_private_components(paths.root()).ok()
}
/// True when the hint must not be printed, including when the marker cannot be read.
pub fn hint_recorded(paths: &RuntimePaths) -> bool {
    let Some(dir) = hint_dir(paths) else {
        return true;
    };
    dir.open_file(HINT_MARKER, false).is_ok()
}
/// Record that the hint line was written. Call only after stdout accepted it.
pub fn mark_hint_shown(paths: &RuntimePaths) -> bool {
    let Some(dir) = hint_dir(paths) else {
        return false;
    };
    dir.replace(HINT_MARKER, b"1", 0o644).is_ok()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn migration_offer_is_only_an_unset_console_launch() {
        assert!(
            LaunchState {
                console: true,
                setup: false,
                tray: false,
            }
            .migration_offer()
        );
        for (console, setup, tray) in [
            (false, false, false),
            (false, false, true),
            (false, true, false),
            (false, true, true),
            (true, true, false),
            (true, false, true),
            (true, true, true),
        ] {
            assert!(
                !LaunchState {
                    console,
                    setup,
                    tray,
                }
                .migration_offer(),
                "{console}/{setup}/{tray}"
            );
        }
    }
    #[test]
    fn startup_shortcut_uses_appdata_without_a_shell() {
        let path = startup_shortcut_from_env(&["APPDATA=C:\\srv\\project\\Roaming".into()])
            .expect("appdata");
        assert!(path.starts_with(r"C:\srv\project\Roaming"));
        assert!(path.ends_with(SHORTCUT));
        assert!(path.parent().unwrap().ends_with("Startup"));
        assert!(startup_shortcut_from_env(&["APPDATA=".into()]).is_none());
        assert!(startup_shortcut_from_env(&[]).is_none());
    }
    #[test]
    fn hint_marker_is_absent_until_marked() {
        let home = std::env::temp_dir().join(format!(
            "many-ai-cli-hint-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = std::fs::create_dir_all(&home);
        let paths = RuntimePaths::production(&home).unwrap();
        assert!(!hint_recorded(&paths));
        assert!(mark_hint_shown(&paths));
        assert!(hint_recorded(&paths));
        let _ = std::fs::remove_dir_all(&home);
    }
    #[cfg(not(windows))]
    #[test]
    fn non_windows_probe_parts_report_no_console_and_no_tray() {
        let paths = RuntimePaths::production(std::path::Path::new("/tmp/synthetic")).unwrap();
        assert!(!console_attached());
        assert!(!tray_running(&paths));
    }
}
