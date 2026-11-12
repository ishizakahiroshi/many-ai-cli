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
//!
//! The install runs `codex plugin add`, which can take a minute and fail
//! without a network. It runs in the background: a session never waits for
//! it, and a failure is not tried again on every start (audit N32).
use super::{SeedNotice, links};
use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub(super) const MARKETPLACE: &str = "openai-bundled";
pub(super) const PLUGINS: &[&str] = &["browser", "chrome"];
pub(super) const LINK_LABEL: &str = ".tmp/bundled-marketplaces";
const INSTALL_TIMEOUT: Duration = Duration::from_secs(60);
/// How long a failed install is left alone before a start tries again.
const RETRY_AFTER: Duration = Duration::from_secs(30 * 60);
/// What a seed reports for a plugin whose install it started.
pub(super) const INSTALLING: &str = " (installing in the background)";

/// The background install of a profile, by profile directory.
enum Job {
    Running,
    Failed(Instant),
}
static JOBS: Mutex<BTreeMap<PathBuf, Job>> = Mutex::new(BTreeMap::new());
fn jobs() -> std::sync::MutexGuard<'static, BTreeMap<PathBuf, Job>> {
    JOBS.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Installs one `<plugin>@openai-bundled` selector into the profile at the
/// given path. Arguments: profile directory, selector, the session
/// environment and the Hub cwd used to resolve the `codex` command.
pub(super) type Installer = dyn Fn(&Path, &str, &[String], &Path) -> io::Result<()> + Send + Sync;

/// `bundled` is the default home's `.tmp/bundled-marketplaces`. Runs on every
/// seed, so once the link and the plugins exist it only checks for them.
/// Missing plugins are installed in the background; the seed reports that it
/// started the install, and a later seed reports one that failed.
pub(super) fn ensure(
    bundled: &Path,
    profile: &Path,
    environment: &[String],
    cwd: &Path,
    install: &Arc<Installer>,
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
    let missing: Vec<String> = PLUGINS
        .iter()
        .filter(|plugin| !cache.join(plugin).is_dir())
        .map(|plugin| format!("{plugin}@{MARKETPLACE}"))
        .collect();
    if missing.is_empty() {
        return;
    }
    let key = profile.to_path_buf();
    {
        let mut jobs = jobs();
        match jobs.get(&key) {
            Some(Job::Running) => return,
            Some(Job::Failed(at)) if at.elapsed() < RETRY_AFTER => {
                notice
                    .failed
                    .extend(missing.iter().map(|selector| format!("plugin {selector}")));
                return;
            }
            _ => {}
        }
        jobs.insert(key.clone(), Job::Running);
    }
    notice.applied.extend(
        missing
            .iter()
            .map(|selector| format!("plugin {selector}{INSTALLING}")),
    );
    let (install, environment, cwd) = (install.clone(), environment.to_vec(), cwd.to_path_buf());
    let work = move || {
        let mut failed = false;
        for selector in &missing {
            failed |= install(&key, selector, &environment, &cwd).is_err();
        }
        let mut jobs = jobs();
        if failed {
            jobs.insert(key, Job::Failed(Instant::now()));
        } else {
            jobs.remove(&key);
        }
    };
    let started = std::thread::Builder::new()
        .name("codex-plugin-install".into())
        .spawn(work);
    if started.is_err() {
        jobs().insert(profile.to_path_buf(), Job::Failed(Instant::now()));
    }
}

/// The job key of `profile`, however the test spelled the directory.
#[cfg(test)]
fn job_key(profile: &Path) -> Option<PathBuf> {
    let wanted = std::fs::canonicalize(profile).ok()?;
    jobs()
        .keys()
        .find(|key| std::fs::canonicalize(key).ok().as_ref() == Some(&wanted))
        .cloned()
}
/// Whether an install runs for `profile` right now.
#[cfg(test)]
pub(super) fn running(profile: &Path) -> bool {
    job_key(profile).is_some_and(|key| matches!(jobs().get(&key), Some(Job::Running)))
}
/// Waits until no install runs for `profile`.
#[cfg(test)]
pub(super) fn settle(profile: &Path) {
    while running(profile) {
        std::thread::sleep(Duration::from_millis(5));
    }
}
/// Makes the next seed try a failed install again, as after `RETRY_AFTER`.
#[cfg(test)]
pub(super) fn forget_failure(profile: &Path) {
    if let Some(key) = job_key(profile) {
        jobs().remove(&key);
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
