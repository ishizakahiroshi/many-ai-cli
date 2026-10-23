//! Source desktop setup and uninstall. Effects run only when the CLI invokes them.
use crate::{config::RuntimePaths, process::Cancellation, proto::core::CoreFuture};
use std::{
    future::Future,
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
    sync::Arc,
};
pub mod native;
#[cfg(test)]
mod tests;

pub const SHORTCUT: &str = "MANY-AI-CLI.lnk";
pub const LEGACY_SHORTCUT: &str = "Many AI Hub.lnk";
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TargetPlatform {
    Windows,
    Linux,
    MacOS,
}
impl TargetPlatform {
    pub fn native() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOS
        } else {
            Self::Linux
        }
    }
}
#[derive(Clone)]
pub struct Locations {
    pub config: PathBuf,
    pub data: PathBuf,
    pub desktop: PathBuf,
    pub startup: Option<PathBuf>,
    pub executable: PathBuf,
}
pub struct ShortcutOptions {
    pub open: bool,
    pub home: PathBuf,
    pub port: Option<u16>,
}
pub trait PlatformIo: Send + Sync {
    fn prepare_directory(&self, path: &Path) -> io::Result<()>;
    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()>;
    fn make_executable(&self, path: &Path) -> io::Result<()>;
    fn exists(&self, path: &Path) -> bool;
    fn remove_file(&self, path: &Path) -> io::Result<()>;
    fn remove_data(&self, path: &Path) -> io::Result<bool>;
    fn shortcut<'a>(
        &'a self,
        path: &'a Path,
        exe: &'a Path,
        cancel: &'a Cancellation,
    ) -> CoreFuture<'a, io::Result<()>>;
    fn shortcut_with_options<'a>(
        &'a self,
        path: &'a Path,
        exe: &'a Path,
        _options: &'a ShortcutOptions,
        cancel: &'a Cancellation,
    ) -> CoreFuture<'a, io::Result<()>> {
        self.shortcut(path, exe, cancel)
    }
    fn purge<'a>(
        &'a self,
        exe: &'a Path,
        cancel: &'a Cancellation,
    ) -> CoreFuture<'a, io::Result<()>>;
}
pub struct PlatformCli {
    pub paths: RuntimePaths,
    pub locations: Locations,
    pub io: Arc<dyn PlatformIo>,
}
pub fn cmd_contents(exe: &Path, args: &str) -> io::Result<String> {
    let exe = exe
        .to_str()
        .ok_or_else(|| io::Error::other("executable is not UTF-8"))?;
    // Batch quoting cannot preserve embedded percent expansion, quote or linebreak.
    if exe.contains(['%', '"', '\r', '\n', '\0']) {
        return Err(io::Error::other(
            "executable cannot be represented safely in a command shortcut",
        ));
    }
    Ok(format!(
        "@echo off\r\ncd /d %USERPROFILE%\r\ncall \"{exe}\" {args}\r\npause\r\n"
    ))
}
impl PlatformCli {
    pub async fn setup(&self, out: &mut impl Write, cancel: &Cancellation) -> io::Result<()> {
        self.setup_for(TargetPlatform::native(), out, cancel).await
    }
    pub async fn setup_with_port(
        &self,
        port: u16,
        out: &mut impl Write,
        cancel: &Cancellation,
    ) -> io::Result<()> {
        if port == 0 {
            return Err(io::Error::other("shortcut port must not be zero"));
        }
        self.setup_for_with_port(TargetPlatform::native(), Some(port), out, cancel)
            .await
    }
    pub async fn setup_for(
        &self,
        target: TargetPlatform,
        out: &mut impl Write,
        cancel: &Cancellation,
    ) -> io::Result<()> {
        self.setup_for_with_port(target, None, out, cancel).await
    }
    async fn setup_for_with_port(
        &self,
        target: TargetPlatform,
        port: Option<u16>,
        out: &mut impl Write,
        cancel: &Cancellation,
    ) -> io::Result<()> {
        if target != TargetPlatform::Windows {
            return self.setup_unix(target, out);
        }
        writeln!(
            out,
            "Many AI CLI setup\n\n[OK] executable: {}",
            self.locations.executable.display()
        )?;
        self.io.prepare_directory(&self.locations.config)?;
        writeln!(
            out,
            "[OK] config directory: {}",
            self.locations.config.display()
        )?;
        self.io.prepare_directory(&self.locations.data)?;
        let home = self
            .locations
            .config
            .parent()
            .filter(|home| home.is_absolute())
            .ok_or_else(|| io::Error::other("shortcut home must be absolute"))?;
        if let Some(port) = port {
            writeln!(
                out,
                "[OK] shortcut home: {}\n[OK] shortcut port: {port}",
                home.display()
            )?;
        }
        let mut failed = false;
        for (name, args) in [("start.cmd", "serve --open"), ("stop.cmd", "stop")] {
            let path = self.locations.data.join(name);
            let args = if name == "start.cmd" {
                port.map_or_else(|| args.to_owned(), |port| format!("{args} --port {port}"))
            } else {
                args.into()
            };
            let result = cmd_contents_for_home(&self.locations.executable, &args, home)
                .and_then(|body| self.io.write(&path, body.as_bytes()));
            failed |= print_result(out, &path, result)?;
        }
        if self.locations.desktop.as_os_str().is_empty() {
            return Err(io::Error::other("desktop directory not found"));
        }
        for directory in
            std::iter::once(&self.locations.desktop).chain(self.locations.startup.iter())
        {
            let path = directory.join(SHORTCUT);
            let options = ShortcutOptions {
                open: directory == &self.locations.desktop,
                home: home.to_path_buf(),
                port,
            };
            let result = self
                .io
                .shortcut_with_options(&path, &self.locations.executable, &options, cancel)
                .await;
            failed |= print_result(out, &path, result)?;
        }
        for directory in
            std::iter::once(&self.locations.desktop).chain(self.locations.startup.iter())
        {
            let old = directory.join(LEGACY_SHORTCUT);
            if self.io.remove_file(&old).is_ok() {
                writeln!(
                    out,
                    "[NOTE] {}: 旧名のショートカットを MANY-AI-CLI に置き換えました",
                    old.display()
                )?;
            }
        }
        for name in ["Many AI Hub Start.lnk", "Many AI Hub Stop.lnk"] {
            let path = self.locations.desktop.join(name);
            if self.io.exists(&path) {
                writeln!(
                    out,
                    "[NOTE] {}: 旧アイコンです。不要なら手動で削除してください（自動では消しません）",
                    path.display()
                )?;
            }
        }
        writeln!(
            out,
            "\nNext:\n  Double click \"MANY-AI-CLI\" on your desktop to start/reuse the Hub and open the browser.\n  Sign-in starts only the tray; it does not open the browser.\n  Tray exit keeps the Hub running; choose \"Hub を停止\" to stop the Hub.\n  To stop sign-in startup, delete \"MANY-AI-CLI\" from the Startup folder\n  (or turn it off in Task Manager > Startup apps)."
        )?;
        if failed {
            Err(io::Error::other("some shortcuts failed to create"))
        } else {
            Ok(())
        }
    }
    fn setup_unix(&self, target: TargetPlatform, out: &mut impl Write) -> io::Result<()> {
        writeln!(
            out,
            "Many AI CLI setup\n\n[OK] executable: {}",
            self.locations.executable.display()
        )?;
        self.io.prepare_directory(&self.locations.config)?;
        writeln!(
            out,
            "[OK] config directory: {}",
            self.locations.config.display()
        )?;
        let mut failed = false;
        let mut directories = vec![];
        if target == TargetPlatform::Linux {
            self.io.prepare_directory(&self.locations.data)?;
            directories.push(&self.locations.data);
        }
        if !self.locations.desktop.as_os_str().is_empty() {
            self.io.prepare_directory(&self.locations.desktop)?;
            directories.push(&self.locations.desktop);
        }
        if directories.is_empty() {
            return Err(io::Error::other("desktop directory not found"));
        }
        for directory in directories {
            for (name, args) in [
                ("Many AI Hub Start", "serve --open"),
                ("Many AI Hub Stop", "stop"),
            ] {
                let filename = if target == TargetPlatform::Linux {
                    if args == "stop" {
                        "many-ai-hub-stop.desktop"
                    } else {
                        "many-ai-hub-start.desktop"
                    }
                    .into()
                } else {
                    format!("{name}.command")
                };
                let path = directory.join(filename);
                let result = unix_contents(target, &self.locations.executable, name, args)
                    .and_then(|body| self.io.write(&path, body.as_bytes()))
                    .and_then(|_| self.io.make_executable(&path));
                failed |= print_result(out, &path, result)?;
            }
        }
        writeln!(
            out,
            "\nNext:\n  Double click \"Many AI Hub Start\" on your desktop."
        )?;
        if target == TargetPlatform::Linux {
            writeln!(
                out,
                "  (GNOME: right click the desktop icon and choose \"Allow Launching\" the first time.)"
            )?;
        }
        if failed {
            Err(io::Error::other("some shortcuts failed to create"))
        } else {
            Ok(())
        }
    }
    pub async fn uninstall(
        &self,
        purge: bool,
        input: &mut impl BufRead,
        out: &mut impl Write,
        cancel: &Cancellation,
    ) -> io::Result<()> {
        self.uninstall_with_prepare(purge, input, out, cancel, || async { Ok(()) })
            .await
    }

    /// Run pre-removal work only after the user explicitly confirms once.
    pub async fn uninstall_with_prepare<F, Fut>(
        &self,
        purge: bool,
        input: &mut impl BufRead,
        out: &mut impl Write,
        cancel: &Cancellation,
        prepare: F,
    ) -> io::Result<()>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = io::Result<()>>,
    {
        writeln!(
            out,
            "以下を削除します:\n  設定・ログ: {}",
            self.locations.config.display()
        )?;
        if purge {
            writeln!(out, "  バイナリ:   {}", self.locations.executable.display())?;
        }
        write!(out, "\n続行しますか? [y/N]: ")?;
        out.flush()?;
        let mut line = String::new();
        input.read_line(&mut line)?;
        if !matches!(line.trim().to_lowercase().as_str(), "y" | "yes") {
            writeln!(out, "アンインストールをキャンセルしました。")?;
            return Ok(());
        }
        prepare().await?;
        let removed = self.io.remove_data(&self.locations.config)?;
        writeln!(
            out,
            "{}: {}",
            if removed {
                "削除しました"
            } else {
                "存在しないためスキップ"
            },
            self.locations.config.display()
        )?;
        if let Some(startup) = &self.locations.startup {
            for name in [SHORTCUT, LEGACY_SHORTCUT] {
                let path = startup.join(name);
                if self.io.remove_file(&path).is_ok() {
                    writeln!(out, "削除しました: {}", path.display())?;
                }
            }
        }
        writeln!(
            out,
            "\nブラウザに保存された UI 設定（テーマ・言語・お気に入り・レイアウト等）は残ります。\n消去するには Hub を開いていたタブで DevTools コンソール（F12）を開き、次を実行してください:\n  localStorage.clear()"
        )?;
        if purge {
            self.io.purge(&self.locations.executable, cancel).await?;
            writeln!(
                out,
                "{}: {}",
                if TargetPlatform::native() == TargetPlatform::Windows {
                    "バイナリを削除中"
                } else {
                    "バイナリを削除しました"
                },
                self.locations.executable.display()
            )?;
        } else {
            writeln!(
                out,
                "\nバイナリを手動で削除してください:\n  {}",
                self.locations.executable.display()
            )?;
        }
        writeln!(out, "\nアンインストール完了。")
    }
}
fn cmd_contents_for_home(exe: &Path, args: &str, home: &Path) -> io::Result<String> {
    let home = home
        .to_str()
        .ok_or_else(|| io::Error::other("shortcut home is not UTF-8"))?;
    if home.contains(['%', '"', '\r', '\n', '\0']) {
        return Err(io::Error::other(
            "home cannot be represented safely in a command shortcut",
        ));
    }
    let contents =
        cmd_contents(exe, args)?.replace("cd /d %USERPROFILE%\r\n", "cd /d \"%USERPROFILE%\"\r\n");
    Ok(contents.replacen("@echo off\r\n", &format!("@echo off\r\nsetlocal DisableDelayedExpansion\r\nset \"USERPROFILE={home}\"\r\nset \"HOME={home}\"\r\n"), 1))
}
pub fn unix_contents(
    target: TargetPlatform,
    exe: &Path,
    name: &str,
    args: &str,
) -> io::Result<String> {
    let exe = exe
        .to_str()
        .ok_or_else(|| io::Error::other("executable is not UTF-8"))?;
    if exe.contains(['\0', '\r', '\n']) {
        return Err(io::Error::other("invalid desktop executable"));
    }
    match target {
        TargetPlatform::Linux => {
            let escaped = exe
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('`', "\\`")
                .replace('$', "\\$")
                .replace('%', "%%");
            Ok(format!(
                "[Desktop Entry]\nType=Application\nName={name}\nExec=\"{escaped}\" {args}\nTerminal=true\nCategories=Development;\n"
            ))
        }
        TargetPlatform::MacOS => Ok(format!(
            "#!/bin/sh\n'{}' {args}\n",
            exe.replace('\'', "'\\''")
        )),
        TargetPlatform::Windows => Err(io::Error::other("Windows requires a command shortcut")),
    }
}
fn print_result(out: &mut impl Write, path: &Path, result: io::Result<()>) -> io::Result<bool> {
    match result {
        Ok(()) => {
            writeln!(out, "[OK] created: {}", path.display())?;
            Ok(false)
        }
        Err(error) => {
            writeln!(out, "[FAIL] {}: {error}", path.display())?;
            Ok(true)
        }
    }
}
