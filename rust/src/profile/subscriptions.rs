//! Spawn-time profile selection and named-entry seeding. No auth/status/usage
//! command exists in this module, and credentials are never part of its seed set.
pub mod diagnostics;
mod links;
mod seed;
#[cfg(test)]
mod tests;
use crate::{
    config::{self, Config, Resource, RuntimePaths},
    files::safe_fs::Dir,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::{Component, Path, PathBuf},
    sync::Mutex,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SeedNotice {
    pub applied: Vec<String>,
    pub failed: Vec<String>,
    pub degraded: Vec<String>,
    pub synced: Vec<String>,
}
pub struct SubscriptionLauncher {
    paths: RuntimePaths,
    home: PathBuf,
    hub_cwd: PathBuf,
    // Per-provider round robin, not session ownership or a quota ledger.
    round_robin: Mutex<BTreeMap<String, usize>>,
}
impl SubscriptionLauncher {
    pub fn new(paths: RuntimePaths, home: PathBuf, hub_cwd: PathBuf) -> Self {
        Self {
            paths,
            home,
            hub_cwd,
            round_robin: Mutex::new(BTreeMap::new()),
        }
    }
    /// CRUD creation uses the same named-entry seed authority as spawn, but
    /// does not select another profile or produce session environment entries.
    pub fn seed_existing_profile(
        &self,
        provider: &str,
        profile: &config::SubscriptionProfile,
        base_environment: &[String],
    ) -> io::Result<SeedNotice> {
        if adapter(provider).is_none() {
            return Err(invalid("provider does not support subscription profiles"));
        }
        let path = config::resolve_subscription_profile_dir(
            &self.paths,
            provider,
            profile,
            Some(&self.home),
        )
        .map_err(|_| invalid("invalid subscription profile directory"))?;
        check_path(&self.paths, &path)?;
        let destination = existing_target(&path);
        let directory = Dir::open_or_create_private(&destination)?;
        seed::run(
            &self.paths,
            &self.home,
            &self.hub_cwd,
            base_environment,
            provider,
            &directory,
            profile,
        )
    }
    pub fn launch(
        &self,
        cfg: &Config,
        provider: &str,
        raw_id: &str,
        base_environment: &[String],
    ) -> io::Result<(Vec<String>, Option<SeedNotice>)> {
        let mut id = config::normalize_subscription_id(raw_id);
        if id.is_empty() {
            return Ok((vec![], None));
        }
        if id == "auto" {
            let candidates = selectable(cfg, provider);
            if candidates.is_empty() {
                return Err(invalid("no enabled subscription profile to choose from"));
            }
            let mut rr = self
                .round_robin
                .lock()
                .map_err(|_| io::Error::other("subscription selection lock failed"))?;
            let next = rr.entry(provider.into()).or_default();
            let index = *next % candidates.len();
            *next = (index + 1) % candidates.len();
            id = candidates[index].clone();
        }
        let env_key = adapter(provider)
            .ok_or_else(|| invalid("provider does not support subscription profiles"))?;
        let profile = config::find_subscription(&cfg.subscriptions, provider, &id)
            .ok_or_else(|| invalid("subscription profile not found"))?;
        if !profile.enabled.unwrap_or(true) {
            return Err(invalid("subscription profile is disabled"));
        }
        let path = config::resolve_subscription_profile_dir(
            &self.paths,
            provider,
            &profile,
            Some(&self.home),
        )
        .map_err(|_| invalid("invalid subscription profile directory"))?;
        check_path(&self.paths, &path)?;
        // Keep the requested lexical path in the child environment even when a
        // production profile is a symlink. The pinned writer follows only the
        // explicitly resolved production target; trial escapes are rejected.
        let destination = existing_target(&path);
        let directory = Dir::open_or_create_private(&destination)?;
        let notice = seed::run(
            &self.paths,
            &self.home,
            &self.hub_cwd,
            base_environment,
            provider,
            &directory,
            &profile,
        )?;
        Ok((
            vec![
                format!("{env_key}={}", path.to_string_lossy()),
                format!("MANY_AI_CLI_SUBSCRIPTION_ID={id}"),
            ],
            Some(notice),
        ))
    }
}
pub fn selectable(cfg: &Config, provider: &str) -> Vec<String> {
    if adapter(provider).is_none() {
        return vec![];
    }
    let mut seen = BTreeSet::new();
    cfg.subscriptions
        .get(provider)
        .into_iter()
        .flatten()
        .filter_map(|profile| {
            let id = config::normalize_subscription_id(&profile.id);
            (config::validate_subscription_id(&id).is_ok()
                && profile.enabled.unwrap_or(true)
                && seen.insert(id.clone()))
            .then_some(id)
        })
        .collect()
}
fn adapter(provider: &str) -> Option<&'static str> {
    match provider {
        "claude" => Some("CLAUDE_CONFIG_DIR"),
        "codex" => Some("CODEX_HOME"),
        "opencode" => Some("XDG_DATA_HOME"),
        "grok" => Some("GROK_HOME"),
        _ => None,
    }
}
fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
fn env_value<'a>(environment: &'a [String], name: &str) -> Option<&'a str> {
    environment.iter().rev().find_map(|entry| {
        entry
            .split_once('=')
            .filter(|(key, _)| *key == name)
            .map(|(_, value)| value)
    })
}
fn clean(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(out.components().next_back(), Some(Component::Normal(_))) {
                    out.pop();
                } else if !out.has_root() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}
fn existing_target(path: &Path) -> PathBuf {
    if let Ok(real) = path.canonicalize() {
        return real;
    }
    if let Some(parent) = path.parent()
        && let Some(name) = path.file_name()
    {
        return existing_target(parent).join(name);
    }
    path.into()
}
/// Constrain every trial read/write (including a default-settings symlink) to
/// the explicitly supplied runtime root, before opening any vendor file.
pub fn check_path(paths: &RuntimePaths, path: &Path) -> io::Result<()> {
    if !path.is_absolute() {
        return Err(invalid("vendor configuration path must be absolute"));
    }
    if !paths.is_trial() {
        return Ok(());
    }
    if path
        .components()
        .any(|part| matches!(part, Component::ParentDir))
    {
        return Err(invalid(
            "trial vendor configuration path contains parent traversal",
        ));
    }
    // Canonical ancestor comparison accepts native spelling aliases (Windows
    // verbatim disk roots and macOS /var -> /private/var) without treating a
    // merely similar lexical prefix as confinement. Missing tails are safe
    // only when their existing ancestor is already inside this trial root.
    let root = paths.root().canonicalize()?;
    let mut at = path;
    loop {
        match std::fs::symlink_metadata(at) {
            Ok(_) => {
                let real = at.canonicalize()?;
                if !real.starts_with(root) {
                    return Err(invalid("vendor configuration symlink escapes trial root"));
                }
                break;
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                at = at
                    .parent()
                    .ok_or_else(|| invalid("trial path has no ancestor"))?;
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}
fn default_dir(
    paths: &RuntimePaths,
    home: &Path,
    hub_cwd: &Path,
    environment: &[String],
    provider: &str,
) -> io::Result<PathBuf> {
    let key = adapter(provider).ok_or_else(|| invalid("unsupported seed provider"))?;
    let explicit = env_value(environment, key).unwrap_or("").trim();
    let path = clean(Path::new(explicit));
    let default =
        if !explicit.is_empty() && !path.starts_with(paths.resource(Resource::Subscriptions)) {
            path
        } else {
            home.join(format!(".{provider}"))
        };
    actor_path(paths, hub_cwd, &default)
}
/// Source filesystem operations run as the Hub, even when the unchanged child
/// vendor env will later be interpreted relative to a different child cwd.
/// Resolve only the I/O path and preserve the source's raw env/result spelling.
pub(crate) fn actor_path(paths: &RuntimePaths, hub_cwd: &Path, raw: &Path) -> io::Result<PathBuf> {
    if !hub_cwd.is_absolute() {
        return Err(invalid("Hub cwd must be explicitly absolute"));
    }
    let path = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        clean(&hub_cwd.join(raw))
    };
    check_path(paths, &path)?;
    Ok(path)
}
