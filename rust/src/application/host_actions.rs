//! Native Go host dispatch. Pickers are Hub owned; successfully dispatched
//! user applications have independent OS lifetimes and are only reaped.
use crate::{
    config::RuntimePaths,
    hub::task_owner::HubTaskHandle,
    process::{self, ExitOutcome, ManagedProcess, ProcessPlan, SpawnOptions},
    proto::core::CoreFuture,
};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
    time::Duration,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenKind {
    File,
    Directory,
    Reveal,
    Terminal,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerKind {
    Directory,
    File { executable: bool },
}
pub trait HostDispatch: Send + Sync {
    fn pick<'a>(&'a self, kind: PickerKind) -> CoreFuture<'a, io::Result<String>>;
    fn open(&self, kind: OpenKind, path: &Path, app: &str) -> io::Result<()>;
}
pub struct NativeHostDispatch {
    paths: RuntimePaths,
    cwd: PathBuf,
    environment: Vec<String>,
    tasks: HubTaskHandle,
}
impl NativeHostDispatch {
    pub fn new(
        paths: RuntimePaths,
        cwd: PathBuf,
        environment: Vec<String>,
        tasks: HubTaskHandle,
    ) -> io::Result<Arc<Self>> {
        if !cwd.is_absolute() {
            return Err(io::Error::other("host dispatch cwd must be absolute"));
        }
        Ok(Arc::new(Self {
            paths,
            cwd,
            environment,
            tasks,
        }))
    }
    fn allowed(&self) -> io::Result<()> {
        if self.paths.is_trial() {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "native host dispatch disabled in trial",
            ))
        } else {
            Ok(())
        }
    }
    fn lookup(&self, name: &str) -> io::Result<String> {
        process::execpath::Resolver::new(
            process::execpath::Platform::native(),
            &self.environment,
            &self.cwd,
            &process::execpath::NativeFs,
        )
        .look_path(name)
    }
    fn env(&self) -> BTreeMap<OsString, Option<OsString>> {
        self.environment
            .iter()
            .filter_map(|s| s.split_once('='))
            .map(|(k, v)| (k.into(), Some(v.into())))
            .collect()
    }
    fn plan(&self, name: &str, args: Vec<OsString>) -> io::Result<ProcessPlan> {
        Ok(ProcessPlan {
            executable: self.lookup(name)?.into(),
            args,
            cwd: self.cwd.clone(),
            env: self.env(),
            stdin: vec![],
            timeout: Duration::ZERO,
            output_cap: 256 * 1024,
            pipe_drain_timeout: Duration::from_secs(2),
        })
    }
    /// The receiver owns no cancellation guard. Hub shutdown does not terminate
    /// a user's editor/terminal after a successful Start receipt.
    fn detached(&self, name: &str, args: Vec<OsString>) -> io::Result<()> {
        let executable = self.lookup(name)?;
        #[cfg(windows)]
        {
            // Go invokes CreateProcess directly; unlike Rust Command's batch
            // convenience, configured scripts must not introduce cmd parsing.
            let lower = executable.to_ascii_lowercase();
            if lower.ends_with(".cmd") || lower.ends_with(".bat") {
                return Err(io::Error::from_raw_os_error(193));
            }
        }
        let (sender, receiver) = std::sync::mpsc::sync_channel::<std::process::Child>(1);
        std::thread::Builder::new()
            .name("many-ai-host-reaper".into())
            .spawn(move || {
                if let Ok(mut child) = receiver.recv() {
                    let _ = child.wait();
                }
            })?;
        let mut cmd = Command::new(executable);
        cmd.args(args)
            .current_dir(&self.cwd)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        for (key, value) in self.env() {
            if let Some(value) = value {
                cmd.env(key, value);
            }
        }
        let child = cmd.spawn()?;
        sender
            .send(child)
            .map_err(|_| io::Error::other("host dispatch reaper ended"))?;
        Ok(())
    }
    #[cfg(not(windows))]
    fn linux_open(&self, path: &Path) -> io::Result<()> {
        for (name, prefix) in [
            ("xdg-open", None),
            ("gio", Some("open")),
            ("gnome-open", None),
        ] {
            if self.lookup(name).is_ok() {
                let mut args = vec![];
                if let Some(prefix) = prefix {
                    args.push(prefix.into());
                }
                args.push(path.as_os_str().to_owned());
                return self.detached(name, args);
            }
        }
        Err(io::Error::other(
            "no opener available (install xdg-utils, glib2-bin, or gnome-open)",
        ))
    }
}
impl HostDispatch for NativeHostDispatch {
    fn pick<'a>(&'a self, kind: PickerKind) -> CoreFuture<'a, io::Result<String>> {
        Box::pin(async move {
            self.allowed()?;
            #[cfg(windows)]
            let plan = {
                let scripts: serde_json::Value =
                    serde_json::from_str(include_str!("host_actions/picker_scripts.json"))
                        .expect("fixed Go picker scripts");
                let key = match kind {
                    PickerKind::Directory => "directory",
                    PickerKind::File { executable: true } => "exe",
                    PickerKind::File { executable: false } => "file",
                };
                self.plan(
                    "powershell.exe",
                    [
                        "-NoProfile",
                        "-STA",
                        "-NonInteractive",
                        "-Command",
                        scripts[key].as_str().expect("fixed script"),
                    ]
                    .into_iter()
                    .map(OsString::from)
                    .collect(),
                )?
            };
            #[cfg(not(windows))]
            let plan = match kind {
                PickerKind::Directory if cfg!(target_os = "macos") => self.plan(
                    "osascript",
                    vec!["-e".into(), "POSIX path of (choose folder)".into()],
                )?,
                PickerKind::Directory if self.lookup("zenity").is_ok() => self.plan(
                    "zenity",
                    vec!["--file-selection".into(), "--directory".into()],
                )?,
                PickerKind::Directory => {
                    if self.lookup("kdialog").is_err() {
                        return Err(io::Error::other(
                            "no folder picker available (install zenity or kdialog)",
                        ));
                    }
                    self.plan("kdialog", vec!["--getexistingdirectory".into()])?
                }
                PickerKind::File { executable } => {
                    if self.lookup("zenity").is_err() {
                        return Err(io::Error::other(
                            "no file picker available (install zenity)",
                        ));
                    }
                    let mut args = vec!["--file-selection".into()];
                    if executable {
                        args.push("--file-filter=*.AppImage;*.elf;*.sh".into());
                    }
                    self.plan("zenity", args)?
                }
            };
            let mut selected = pick_owned(plan, &self.tasks).await?;
            if cfg!(not(windows)) && kind == PickerKind::Directory {
                selected = selected.trim_end_matches('/').into();
            }
            Ok(selected)
        })
    }
    fn open(&self, kind: OpenKind, path: &Path, app: &str) -> io::Result<()> {
        self.allowed()?;
        if kind == OpenKind::Terminal && !app.is_empty() {
            return self.detached(app, vec![path.as_os_str().to_owned()]);
        }
        #[cfg(windows)]
        {
            let clean = crate::files::scope::clean(path);
            let value = clean.as_os_str().to_owned();
            match kind {
                OpenKind::File => shell_open(&clean),
                OpenKind::Directory => self.detached("explorer.exe", vec![value]),
                OpenKind::Reveal => {
                    let mut arg = OsString::from("/select,");
                    arg.push(value);
                    self.detached("explorer.exe", vec![arg])
                }
                OpenKind::Terminal => self
                    .detached("wt.exe", vec!["-d".into(), path.as_os_str().to_owned()])
                    .or_else(|_| {
                        self.detached(
                            "powershell.exe",
                            vec![
                                "-NoExit".into(),
                                "-Command".into(),
                                "Set-Location -LiteralPath $args[0]".into(),
                                path.as_os_str().to_owned(),
                            ],
                        )
                    }),
            }
        }
        #[cfg(not(windows))]
        {
            if cfg!(target_os = "macos") {
                let mut args = match kind {
                    OpenKind::Reveal => vec!["-R".into()],
                    OpenKind::Terminal => vec!["-a".into(), "Terminal".into()],
                    _ => vec![],
                };
                args.push(path.as_os_str().to_owned());
                return self.detached("open", args);
            }
            match kind {
                OpenKind::Terminal => {
                    self.detached("x-terminal-emulator", vec![path.as_os_str().to_owned()])
                }
                OpenKind::Reveal => self.linux_open(path.parent().unwrap_or(Path::new("."))),
                _ => self.linux_open(path),
            }
        }
    }
}
/// Retained effect work owns the picker even after its HTTP observer disappears.
async fn pick_owned(plan: ProcessPlan, tasks: &HubTaskHandle) -> io::Result<String> {
    let permit = tasks
        .effect_permit()
        .map_err(|_| io::Error::new(io::ErrorKind::Interrupted, "Hub task admission stopped"))?;
    let cancel = permit.cancellation();
    let waiter=permit.start(async move {
        let (mut child,_)=ManagedProcess::spawn_owned_with_options(plan,1,SpawnOptions{no_window:true,stdin_null:true,env_clear:true,..Default::default()});
        let output=tokio::select!{result=child.wait()=>result,_=cancel.token().cancelled()=>{child.close();child.wait().await}}?;
        if output.stdout_truncated{return Err(io::Error::other("picker output exceeds limit"));}
        if !matches!(output.outcome,ExitOutcome::Exited{code:Some(0),..}){return Err(io::Error::other("native picker failed"));}
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    });
    waiter
        .wait()
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::Interrupted, "picker owner ended"))?
}
#[cfg(windows)]
fn shell_open(path: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn ShellExecuteW(
            hwnd: *mut std::ffi::c_void,
            operation: *const u16,
            file: *const u16,
            parameters: *const u16,
            directory: *const u16,
            show: i32,
        ) -> *mut std::ffi::c_void;
    }
    let mut value: Vec<u16> = path.as_os_str().encode_wide().collect();
    if value.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path contains NUL",
        ));
    }
    value.push(0);
    let verb: [u16; 5] = [111, 112, 101, 110, 0];
    let receipt = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            value.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
        )
    } as isize;
    if receipt <= 32 {
        Err(io::Error::from_raw_os_error(receipt as i32))
    } else {
        Ok(())
    }
}
pub fn terminal_description(app: &str) -> String {
    if !app.is_empty() {
        return format!("{app} <dir>");
    }
    if cfg!(windows) {
        "wt.exe -d <dir> (fallback: powershell.exe -NoExit -Command \"Set-Location -LiteralPath $args[0]\" <dir>)".into()
    } else if cfg!(target_os = "macos") {
        "open -a Terminal <dir>".into()
    } else {
        "x-terminal-emulator <dir>".into()
    }
}
#[cfg(test)]
mod tests;
