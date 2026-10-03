use std::{
    io,
    path::{Component, Path, PathBuf},
};

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
}
#[derive(Clone, Debug)]
pub struct RuntimePaths {
    root: PathBuf,
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
            trial: false,
            port: 47777,
        })
    }
    pub fn trial(root: &Path, port: u16, installed_root: &Path) -> io::Result<Self> {
        if !root.is_absolute() || port == 0 || port == 47777 {
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
        let installed =
            std::fs::canonicalize(installed_root).unwrap_or_else(|_| installed_root.to_path_buf());
        if root == installed || root.starts_with(&installed) || installed.starts_with(&root) {
            return Err(io::Error::other(
                "trial and installed roots must be disjoint",
            ));
        }
        if !root.is_dir() {
            return Err(io::Error::other("trial root must be a directory"));
        }
        Ok(Self {
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
        let name = match resource {
            Resource::Config => "config.yaml",
            Resource::Database => "any-ai-cli.db",
            Resource::Logs => "logs",
            Resource::Token => "tokens",
            Resource::LauncherProfiles => "launcher-profiles.yaml",
            Resource::LauncherActive => "launcher-active.json",
            Resource::Locks => "locks",
            Resource::Temporary => "tmp",
            Resource::Profiles => "profiles",
            Resource::Subscriptions => "subscriptions",
            Resource::Models => "models",
            Resource::Runtime => "runtime",
            Resource::Update => "update",
            Resource::UsageHooks => "usage-hooks",
            Resource::Pid => "pids",
            Resource::Attachments => "attachments",
            Resource::Routines => "routines.json",
            Resource::Memos => "memos.json",
            Resource::MemoImages => "memo-images",
            Resource::Push => "push.json",
            Resource::Orchestration => "orchestration",
            Resource::Handoff => "handoff",
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
}
