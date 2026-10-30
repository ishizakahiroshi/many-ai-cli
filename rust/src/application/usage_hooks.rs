//! Hub-owned Codex usage hooks; source usage_hook_handler.go.
use crate::{
    config::{Config, ConfigStore, RuntimePaths},
    terminal::session::SessionEngine,
    wrapper::hooks::codex::{CodexStopHooks, HookParams},
};
use std::{
    io,
    path::Path,
    sync::{Arc, Mutex},
};
pub type UsageHookWarning = dyn Fn(&'static str) + Send + Sync;
pub struct UsageHooks {
    config: Arc<ConfigStore>,
    core: Arc<SessionEngine>,
    codex: CodexStopHooks,
    executable: std::path::PathBuf,
    port: u16,
    operations: Mutex<()>,
    warning: Arc<UsageHookWarning>,
}
impl UsageHooks {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: Arc<ConfigStore>,
        core: Arc<SessionEngine>,
        paths: RuntimePaths,
        home: &Path,
        environment: &[String],
        cwd: &Path,
        executable: &Path,
        port: u16,
        warning: Arc<UsageHookWarning>,
    ) -> io::Result<Arc<Self>> {
        if port == 0 || (paths.is_trial() && port != paths.port()) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "usage hooks runtime port mismatch",
            ));
        }
        Ok(Arc::new(Self {
            codex: CodexStopHooks::for_context(paths, home, environment, cwd)?,
            config,
            core,
            executable: executable.into(),
            port,
            operations: Mutex::new(()),
            warning,
        }))
    }
    /// Invoke on Hub startup and after canonical register/reattach commit.
    /// Configuration is copied before taking the core or hook locks.
    pub fn registered(&self) {
        let config = match self.config.snapshot() {
            Ok(v) => v.config,
            Err(_) => {
                (self.warning)("usage hook configuration snapshot failed");
                return;
            }
        };
        self.inject(&config);
    }
    fn inject(&self, config: &Config) {
        if !config.user_prefs.token_statusbar.is_enabled() {
            return;
        }
        let Ok(_guard) = self.operations.lock() else {
            (self.warning)("usage hook operation lock failed");
            return;
        };
        let url = format!("http://127.0.0.1:{}", self.port);
        // Canonical registry, including hidden usage-probe sessions. No second
        // session map and no per-wrapper remove that can erase a sibling hook.
        for snapshot in self.core.usage_hook_sessions() {
            if snapshot.provider == "codex"
                && self
                    .codex
                    .inject(&HookParams {
                        hub_url: &url,
                        token: &config.token,
                        session: snapshot.id.0,
                        executable: &self.executable,
                    })
                    .is_err()
            {
                (self.warning)("Codex usage hook injection failed");
            }
        }
    }
    /// Invoke after canonical ended/dismiss/disconnect state was committed.
    pub fn ended(&self, provider: &str) {
        if provider != "codex" {
            return;
        }
        let Ok(_guard) = self.operations.lock() else {
            (self.warning)("usage hook operation lock failed");
            return;
        };
        if self
            .core
            .usage_hook_sessions()
            .iter()
            .any(|s| s.provider == "codex")
        {
            return;
        }
        if self.codex.remove().is_err() {
            (self.warning)("Codex inactive usage hook removal failed");
        }
    }
    /// Call after wrappers have ended and the Hub is shutting down.
    pub fn shutdown(&self) {
        let Ok(_guard) = self.operations.lock() else {
            (self.warning)("usage hook operation lock failed");
            return;
        };
        if self.codex.remove().is_err() {
            (self.warning)("Codex shutdown usage hook removal failed");
        }
    }
}
#[cfg(test)]
mod tests;
