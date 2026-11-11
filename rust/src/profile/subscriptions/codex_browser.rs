//! Codex Desktop unpacks its bundled plugin marketplace (`openai-bundled`,
//! which carries the Browser and Chrome plugins and their skills) only into
//! the default Codex home, under `.tmp/bundled-marketplaces`. A subscription
//! profile is a different `CODEX_HOME`: the synced `config.toml` names the
//! marketplace, but Codex ignores it there, refuses to add `openai-bundled`
//! again ("reserved"), and a session cannot find the browser skills.
//!
//! Linking the profile's `.tmp/bundled-marketplaces` to the default one makes
//! the marketplace visible to the profile, and `codex plugin add` then
//! installs the plugins into the profile itself (checked on 2026-11-10 with
//! Codex CLI 0.167.0 and Codex Desktop 26.1103). Without Codex Desktop there
//! is nothing to link and nothing is done.
use super::{SeedNotice, links};
use std::{
    io,
    path::Path,
    time::{Duration, Instant},
};

pub(super) const MARKETPLACE: &str = "openai-bundled";
pub(super) const PLUGINS: &[&str] = &["browser", "chrome"];
pub(super) const LINK_LABEL: &str = ".tmp/bundled-marketplaces";
const INSTALL_TIMEOUT: Duration = Duration::from_secs(60);

/// Installs one `<plugin>@openai-bundled` selector into the profile at the
/// given path. Arguments: profile directory, selector, the session
/// environment and the Hub cwd used to resolve the `codex` command.
pub(super) type Installer = dyn Fn(&Path, &str, &[String], &Path) -> io::Result<()> + Send + Sync;

/// `bundled` is the default home's `.tmp/bundled-marketplaces`. Runs on every
/// seed, so once the link and the plugins exist it only checks for them.
pub(super) fn ensure(
    bundled: &Path,
    profile: &Path,
    environment: &[String],
    cwd: &Path,
    install: &Installer,
    notice: &mut SeedNotice,
) {
    if !bundled.join(MARKETPLACE).is_dir() {
        return;
    }
    let tmp = profile.join(".tmp");
    let target = tmp.join("bundled-marketplaces");
    if std::fs::symlink_metadata(&target).is_err() {
        let linked =
            std::fs::create_dir_all(&tmp).and_then(|()| links::directory(bundled, &target));
        if linked.is_err() {
            notice.failed.push(LINK_LABEL.into());
            return;
        }
        notice.applied.push(LINK_LABEL.into());
    }
    let cache = profile.join("plugins").join("cache").join(MARKETPLACE);
    for plugin in PLUGINS {
        if cache.join(plugin).is_dir() {
            continue;
        }
        let selector = format!("{plugin}@{MARKETPLACE}");
        match install(profile, &selector, environment, cwd) {
            Ok(()) => notice.applied.push(format!("plugin {selector}")),
            Err(_) => notice.failed.push(format!("plugin {selector}")),
        }
    }
}

/// Runs `codex plugin add <selector>` with `CODEX_HOME` set to the profile.
/// The command is resolved like other CLI jobs (a missing `codex` is an
/// error, never a bare name), gets only the given environment, shows no
/// window and is stopped after `INSTALL_TIMEOUT`.
pub(super) fn install_with_codex_cli(
    profile: &Path,
    selector: &str,
    environment: &[String],
    cwd: &Path,
) -> io::Result<()> {
    use crate::process::execpath::{NativeFs, Platform, Resolver};
    use std::process::{Command, Stdio};
    let args = [
        "plugin".to_string(),
        "add".to_string(),
        selector.to_string(),
    ];
    let resolved = Resolver::new(Platform::native(), environment, cwd, &NativeFs)
        .resolve_command("codex", &args)?;
    let mut command = Command::new(&resolved.executable);
    command
        .args(&resolved.args)
        .env_clear()
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for (key, value) in environment.iter().filter_map(|s| s.split_once('=')) {
        // Windows exports drive-current-directory entries as =C:=C:\dir.
        if !key.is_empty() {
            command.env(key, value);
        }
    }
    command.env("CODEX_HOME", profile);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    }
    let mut child = command.spawn()?;
    let deadline = Instant::now() + INSTALL_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait()? {
            return if status.success() {
                Ok(())
            } else {
                Err(io::Error::other("codex plugin add failed"))
            };
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "codex plugin add timed out",
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
