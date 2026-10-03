use std::{
    io,
    path::{Component, Path, PathBuf},
};

/// Resolve only macOS's fixed root-owned system aliases before capability
/// walking. A user-controlled symlink in any later component is still rejected.
/// Do not canonicalize arbitrary caller paths: that would erase the boundary.
pub(crate) fn native_system_path(path: &Path) -> std::borrow::Cow<'_, Path> {
    #[cfg(target_os = "macos")]
    for name in ["var", "tmp", "etc"] {
        let alias = PathBuf::from("/").join(name);
        let Ok(rest) = path.strip_prefix(&alias) else {
            continue;
        };
        let physical = PathBuf::from("/private").join(name);
        let relative = PathBuf::from("private").join(name);
        if std::fs::read_link(&alias).is_ok_and(|target| target == physical || target == relative) {
            return std::borrow::Cow::Owned(physical.join(rest));
        }
    }
    std::borrow::Cow::Borrowed(path)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resource {
    Config,
    Database,
    Logs,
    Token,
    LauncherProfiles,
    LauncherActive,
    Locks,
    Temporary,
    Profiles,
    Subscriptions,
    Models,
    Runtime,
    Update,
    UsageHooks,
    Pid,
    Attachments,
    Routines,
    Memos,
    MemoImages,
    Push,
    Orchestration,
    Handoff,
    HubState,
    LauncherActiveLock,
    ProviderDefinitions,
    ProviderOverrides,
    ProviderBackups,
    ProviderDistributions,
    ProviderIcons,
    ApprovalPatterns,
    ApprovalRules,
    ApprovalRuleTargets,
    Delegation,
    Whisper,
    WhisperBin,
    WhisperTemporary,
    NotifySound,
    Avatar,
}
#[derive(Clone, Debug)]
pub struct RuntimePaths {
    root: PathBuf,
    log_dir: PathBuf,
    trial: bool,
    port: u16,
}
impl RuntimePaths {
    /// Caller chooses production mode explicitly. Resolving home is never a trial fallback.
    pub fn production(home: &Path) -> io::Result<Self> {
        if !home.is_absolute() {
            return Err(io::Error::other("production home must be absolute"));
        }
        Ok(Self {
            root: home.join(".many-ai-cli"),
            log_dir: home.join(".many-ai-cli").join("logs"),
            trial: false,
            port: 47777,
        })
    }
    /// Preserve the Go Windows-launcher-in-WSL log-home rule without invoking WSL
    /// or looking up a second home during trial mode. The platform caller supplies
    /// a successfully resolved synthetic/selected Windows home, or None on failure.
    pub fn production_launcher(
        home: &Path,
        is_wsl: bool,
        launcher_marker: bool,
        windows_home_as_unix: Option<&Path>,
    ) -> io::Result<Self> {
        let mut paths = Self::production(home)?;
        if is_wsl
            && launcher_marker
            && let Some(windows_home) = windows_home_as_unix.filter(|p| p.is_absolute())
        {
            paths.log_dir = windows_home.join(".many-ai-cli").join("logs");
        }
        Ok(paths)
    }
    /// Go stores history beside the configured log directory. Trial mode refuses
    /// an outside log override; all its database sidecars remain in the trial tree.
    pub fn with_log_dir(mut self, log_dir: &Path) -> io::Result<Self> {
        if log_dir.as_os_str().is_empty() {
            return Err(io::Error::other(
                "empty log directory cannot host the session database",
            ));
        }
        if self.trial && (log_dir != self.root.join("logs")) {
            return Err(io::Error::other("trial log directory cannot be overridden"));
        }
        self.log_dir = log_dir.into();
        Ok(self)
    }
    pub fn usage_hook_temporary_dir(&self, system_temporary: &Path) -> PathBuf {
        if self.trial {
            self.root.join("tmp")
        } else {
            system_temporary.into()
        }
    }
    pub fn trial(root: &Path, port: u16, installed_root: &Path) -> io::Result<Self> {
        if !root.is_absolute() || !installed_root.is_absolute() || port == 0 || port == 47777 {
            return Err(io::Error::other(
                "trial root and isolated port are required",
            ));
        }
        if root.components().any(|c| matches!(c, Component::ParentDir)) {
            return Err(io::Error::other(
                "trial root cannot contain parent traversal",
            ));
        }
        let root = std::fs::canonicalize(root)?;
        let installed = canonicalize_allow_missing(installed_root)?;
        if root == installed || root.starts_with(&installed) || installed.starts_with(&root) {
            return Err(io::Error::other(
                "trial and installed roots must be disjoint",
            ));
        }
        if !root.is_dir() {
            return Err(io::Error::other("trial root must be a directory"));
        }
        Ok(Self {
            log_dir: root.join("logs"),
            root,
            trial: true,
            port,
        })
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn is_trial(&self) -> bool {
        self.trial
    }
    pub fn port(&self) -> u16 {
        self.port
    }
    pub fn automatic_external_actions_allowed(&self) -> bool {
        !self.trial
    }
    pub fn resource(&self, resource: Resource) -> PathBuf {
        match resource {
            Resource::Logs => return self.log_dir.clone(),
            Resource::Database => {
                let clean = clean_path(&self.log_dir);
                let parent = clean
                    .parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or(Path::new("."));
                let base = if parent == Path::new(".")
                    || parent == Path::new(std::path::MAIN_SEPARATOR_STR)
                {
                    &clean
                } else {
                    parent
                };
                return base.join("any-ai-cli.db");
            }
            Resource::Update => return self.log_dir.join("cli-updates"),
            Resource::Models => return self.root.join("whisper").join("models"),
            Resource::WhisperBin => return self.root.join("whisper").join("bin"),
            Resource::WhisperTemporary => return self.root.join("whisper").join("tmp"),
            Resource::ProviderBackups => return self.root.join("backups").join("providers"),
            _ => {}
        }
        let name = match resource {
            Resource::Config => "config.yaml",
            Resource::Database => "any-ai-cli.db",
            Resource::Logs => "logs",
            Resource::Token => "config.yaml",
            Resource::LauncherProfiles => "launcher-profiles.yaml",
            Resource::LauncherActive => "launcher-active.json",
            Resource::Locks => "",
            Resource::Temporary => "tmp",
            Resource::Profiles => "subscriptions",
            Resource::Subscriptions => "subscriptions",
            Resource::Models => unreachable!("handled above"),
            Resource::Runtime => "hub-runtime.json",
            Resource::Update => "update",
            Resource::UsageHooks => "tmp",
            Resource::Pid => "hub-runtime.json",
            Resource::Attachments => "attachments",
            Resource::Routines => "routines.json",
            Resource::Memos => "memos.json",
            Resource::MemoImages => "memo-images",
            Resource::Push => "push_store.json",
            Resource::Orchestration => "orchestration",
            Resource::Handoff => "handoff",
            Resource::HubState => "hub.state",
            Resource::LauncherActiveLock => "launcher-active.json.lock",
            Resource::ProviderDefinitions => "providers.d",
            Resource::ProviderOverrides => "provider-overrides",
            Resource::ProviderBackups => unreachable!("handled above"),
            Resource::ProviderDistributions => "provider-distributions",
            Resource::ProviderIcons => "provider_icons",
            Resource::ApprovalPatterns => "approval-patterns",
            Resource::ApprovalRules => "approval-rules.md",
            Resource::ApprovalRuleTargets => "approval-rule-targets.json",
            Resource::Delegation => "delegation.md",
            Resource::Whisper => "whisper",
            Resource::WhisperBin => unreachable!("handled above"),
            Resource::WhisperTemporary => unreachable!("handled above"),
            Resource::NotifySound => "notify_sound_custom.bin",
            Resource::Avatar => "user_avatar.bin",
        };
        self.root.join(name)
    }
    /// Lexical plus existing-ancestor checks. Filesystem mutation APIs must still
    /// open relative to an owned directory handle to close rename/symlink races.
    pub fn checked_child(&self, resource: Resource, name: &Path) -> io::Result<PathBuf> {
        if name
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(io::Error::other(
                "resource key must contain only normal components",
            ));
        }
        let path = self.resource(resource).join(name);
        if self.trial {
            let relative = path.strip_prefix(&self.root).map_err(io::Error::other)?;
            let mut component_path = self.root.clone();
            for component in relative.components() {
                component_path.push(component.as_os_str());
                match std::fs::symlink_metadata(&component_path) {
                    Ok(metadata) if metadata.file_type().is_symlink() => {
                        // Unlike exists(), canonicalize also detects dangling
                        // symlinks before a later create could follow them out.
                        if !std::fs::canonicalize(&component_path)?.starts_with(&self.root) {
                            return Err(io::Error::other("resource symlink escapes trial root"));
                        }
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
        }
        let mut ancestor = path.as_path();
        while !ancestor.exists() {
            ancestor = ancestor
                .parent()
                .ok_or_else(|| io::Error::other("resource has no existing ancestor"))?;
        }
        if self.trial && !std::fs::canonicalize(ancestor)?.starts_with(&self.root) {
            return Err(io::Error::other("resource escapes trial root"));
        }
        Ok(path)
    }
}
fn canonicalize_allow_missing(path: &Path) -> io::Result<PathBuf> {
    let clean = clean_path(path);
    let mut ancestor = clean.as_path();
    let mut suffix = Vec::new();
    loop {
        match std::fs::symlink_metadata(ancestor) {
            Ok(_) => break,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                suffix.push(
                    ancestor
                        .file_name()
                        .ok_or_else(|| io::Error::other("no existing path ancestor"))?
                        .to_os_string(),
                );
                ancestor = ancestor
                    .parent()
                    .ok_or_else(|| io::Error::other("no existing path ancestor"))?;
            }
            Err(error) => return Err(error),
        }
    }
    let mut resolved = std::fs::canonicalize(ancestor)?;
    for part in suffix.into_iter().rev() {
        resolved.push(part);
    }
    Ok(resolved)
}
fn clean_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if out.file_name().is_some_and(|name| name != "..") {
                    out.pop();
                } else if !out.has_root() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn trial_resources_are_disjoint_and_keep_legacy_db_name() {
        let t = tempfile::tempdir().unwrap();
        let a = t.path().join("a");
        let b = t.path().join("b");
        let old = t.path().join("installed");
        for p in [&a, &b, &old] {
            std::fs::create_dir(p).unwrap();
        }
        let a = RuntimePaths::trial(&a, 49001, &old).unwrap();
        let b = RuntimePaths::trial(&b, 49002, &old).unwrap();
        for r in [
            Resource::Config,
            Resource::Database,
            Resource::Logs,
            Resource::Token,
            Resource::LauncherProfiles,
            Resource::LauncherActive,
            Resource::Locks,
            Resource::Temporary,
            Resource::Profiles,
            Resource::Subscriptions,
            Resource::Models,
            Resource::Runtime,
            Resource::Update,
            Resource::UsageHooks,
            Resource::Pid,
            Resource::Attachments,
            Resource::Routines,
            Resource::Memos,
            Resource::MemoImages,
            Resource::Push,
            Resource::Orchestration,
            Resource::Handoff,
        ] {
            assert_ne!(a.resource(r), b.resource(r));
            assert!(a.resource(r).starts_with(a.root()));
        }
        assert!(a.resource(Resource::Database).ends_with("any-ai-cli.db"));
        assert!(!a.automatic_external_actions_allowed());
        assert!(
            a.checked_child(Resource::Profiles, Path::new("../escape"))
                .is_err()
        );
        assert!(RuntimePaths::trial(&old, 49003, &old).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn trial_rejects_existing_symlink_escape() {
        let t = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let installed = tempfile::tempdir().unwrap();
        let p = RuntimePaths::trial(t.path(), 49001, installed.path()).unwrap();
        std::os::unix::fs::symlink(outside.path(), p.resource(Resource::Profiles)).unwrap();
        assert!(
            p.checked_child(Resource::Profiles, Path::new("synthetic/settings.json"))
                .is_err()
        );
    }
    #[test]
    fn production_log_and_runtime_layout_follows_source() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("linux-home");
        let win = t.path().join("windows-home");
        let p = RuntimePaths::production_launcher(&home, true, true, Some(&win)).unwrap();
        assert_eq!(
            p.resource(Resource::Config),
            home.join(".many-ai-cli/config.yaml")
        );
        assert_eq!(p.resource(Resource::Logs), win.join(".many-ai-cli/logs"));
        assert_eq!(
            p.resource(Resource::Database),
            win.join(".many-ai-cli/any-ai-cli.db")
        );
        assert_eq!(
            p.resource(Resource::Update),
            win.join(".many-ai-cli/logs/cli-updates")
        );
        assert_eq!(
            p.resource(Resource::Push),
            home.join(".many-ai-cli/push_store.json")
        );
        assert_eq!(
            p.resource(Resource::Runtime),
            home.join(".many-ai-cli/hub-runtime.json")
        );
        assert_eq!(
            p.resource(Resource::Models),
            home.join(".many-ai-cli/whisper/models")
        );
        let plain = RuntimePaths::production_launcher(&home, true, false, Some(&win)).unwrap();
        assert_eq!(
            plain.resource(Resource::Logs),
            home.join(".many-ai-cli/logs")
        );
        let changed = plain.with_log_dir(&t.path().join("custom/logs")).unwrap();
        assert_eq!(
            changed.resource(Resource::Database),
            t.path().join("custom/any-ai-cli.db")
        );
    }
    #[test]
    fn trial_never_redirects_log_or_hook_temporary_paths() {
        let t = tempfile::tempdir().unwrap();
        let old = tempfile::tempdir().unwrap();
        let p = RuntimePaths::trial(t.path(), 49001, old.path()).unwrap();
        assert!(p.clone().with_log_dir(&old.path().join("logs")).is_err());
        assert_eq!(p.usage_hook_temporary_dir(old.path()), p.root().join("tmp"));
    }
    #[test]
    fn database_location_matches_go_relative_and_root_parent_rules() {
        let t = tempfile::tempdir().unwrap();
        let root = RuntimePaths::production(t.path()).unwrap();
        for (log, expected) in [
            ("logs", "logs/any-ai-cli.db"),
            ("one/../logs", "logs/any-ai-cli.db"),
            ("one/logs", "one/any-ai-cli.db"),
        ] {
            assert_eq!(
                root.clone()
                    .with_log_dir(Path::new(log))
                    .unwrap()
                    .resource(Resource::Database),
                PathBuf::from(expected)
            );
        }
        assert!(root.clone().with_log_dir(Path::new("")).is_err());
        #[cfg(unix)]
        assert_eq!(
            root.with_log_dir(Path::new("/logs"))
                .unwrap()
                .resource(Resource::Database),
            PathBuf::from("/logs/any-ai-cli.db")
        );
    }
    #[cfg(unix)]
    #[test]
    fn trial_rejects_dangling_symlink_create_and_nonexistent_installed_alias() {
        let t = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let installed = tempfile::tempdir().unwrap();
        let p = RuntimePaths::trial(t.path(), 49001, installed.path()).unwrap();
        std::fs::create_dir(p.resource(Resource::Profiles)).unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("future.json"),
            p.resource(Resource::Profiles).join("future.json"),
        )
        .unwrap();
        assert!(
            p.checked_child(Resource::Profiles, Path::new("future.json"))
                .is_err()
        );
        let alias = outside.path().join("home-alias");
        std::os::unix::fs::symlink(t.path(), &alias).unwrap();
        assert!(RuntimePaths::trial(t.path(), 49001, &alias.join(".many-ai-cli")).is_err());
        assert!(RuntimePaths::trial(t.path(), 49001, Path::new("relative-installed")).is_err());
    }
}
