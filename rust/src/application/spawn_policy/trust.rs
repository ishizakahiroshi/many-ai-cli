//! CLI-native folder trust, with explicit vendor home and the exact child env.
//! Source: internal/clitrust at the fixed Go oracle. These writes never select a
//! profile, execute a CLI, or turn a previous Codex refusal into approval.
mod claude;
mod codex;
mod project;
use super::{invalid, routes::env_value};
use crate::{
    config::RuntimePaths,
    profile::subscriptions::{actor_path, check_path},
};
use std::{
    io,
    path::{Path, PathBuf},
    sync::Mutex,
};
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FolderTrustResult {
    pub written: bool,
    pub config_path: PathBuf,
    pub key: String,
    pub existing: String,
}
/// The owner keeps Rust's canonical (verbatim on Windows) spelling. The Claude
/// key is the plain drive spelling; a path that cannot be safely simplified
/// stays verbatim and is refused as unsupported.
fn exact_claude_folder(cwd: &Path) -> io::Result<PathBuf> {
    let text = cwd
        .to_str()
        .ok_or_else(|| invalid("folder trust requires a UTF-8 working directory"))?;
    #[cfg(windows)]
    let text = project::simplify_verbatim(text);
    let exact = PathBuf::from(text);
    if !exact.is_absolute() || project::clean(&exact) != exact || project::unsupported(&exact) {
        return Err(invalid(
            "folder trust requires a supported absolute working directory",
        ));
    }
    Ok(exact)
}
pub struct NativeFolderTrust {
    paths: RuntimePaths,
    home: PathBuf,
    hub_cwd: PathBuf,
    codex_writer: Mutex<()>,
}
impl NativeFolderTrust {
    pub fn new(paths: RuntimePaths, home: PathBuf, hub_cwd: PathBuf) -> Self {
        Self {
            paths,
            home,
            hub_cwd,
            codex_writer: Mutex::new(()),
        }
    }
    pub fn grant(
        &self,
        provider: &str,
        environment: &[String],
        cwd: &Path,
    ) -> io::Result<FolderTrustResult> {
        self.grant_checked(provider, environment, cwd, false)
    }
    /// Iris real bounded launch only (the one exception to "the Iris launch
    /// path synthesizes no folder trust"). Same config-path rule, lock, atomic
    /// replacement and post-write verification as [`Self::grant`], narrowed:
    /// Claude only; `CLAUDE_CONFIG_DIR` must be explicit and absolute in the
    /// child's exact environment (no home fallback, no relative actor path);
    /// the written key must be `cwd` itself, never a repository root or any
    /// other folder; and the result must leave `cwd` trusted, either written
    /// now or already trusted by Claude's own lookup. Anything else is an error
    /// and the caller must not launch.
    /// The key [`Self::grant_exact_claude`] writes for `cwd`, computed without
    /// any I/O so the caller can record its intent before the write.
    pub fn exact_claude_key(cwd: &Path) -> io::Result<String> {
        exact_claude_folder(cwd).map(|exact| project::claude_key(&exact))
    }
    pub fn grant_exact_claude(
        &self,
        environment: &[String],
        cwd: &Path,
    ) -> io::Result<FolderTrustResult> {
        let key = "CLAUDE_CONFIG_DIR";
        let spelled = environment
            .iter()
            .filter_map(|entry| entry.split_once('='))
            .filter(|(name, _)| name.eq_ignore_ascii_case(key))
            .collect::<Vec<_>>();
        if spelled.is_empty()
            || spelled.iter().any(|(name, _)| *name != key)
            || !env_value(environment, key)
                .map(str::trim)
                .is_some_and(|value| !value.is_empty() && Path::new(value).is_absolute())
        {
            return Err(invalid(
                "exact folder trust requires an explicit absolute Claude configuration directory",
            ));
        }
        let exact = exact_claude_folder(cwd)?;
        let result = self.grant_checked("claude", environment, &exact, true)?;
        if !result.written && result.existing != "trusted" {
            return Err(io::Error::other(
                "exact folder trust left the folder untrusted",
            ));
        }
        Ok(result)
    }
    fn grant_checked(
        &self,
        provider: &str,
        environment: &[String],
        cwd: &Path,
        exact: bool,
    ) -> io::Result<FolderTrustResult> {
        let raw_path = self.config_path(provider, environment)?;
        let path = actor_path(&self.paths, &self.hub_cwd, &raw_path)?;
        if !cwd.is_absolute() || project::unsupported(cwd) {
            return Err(invalid(
                "folder trust requires a supported absolute working directory",
            ));
        }
        check_path(&self.paths, &path)?;
        // Git metadata reads are bounded to the approved child's cwd/repository.
        // A trial additionally confines all such metadata to its owned tree.
        if self.paths.is_trial() {
            check_path(&self.paths, cwd)?;
        }
        let plan = project::plan(provider, cwd, &self.paths)?;
        if plan.target.parent().is_none() || project::same_path(&plan.target, &self.home) {
            return Err(invalid(
                "folder trust target is the home folder or filesystem root",
            ));
        }
        // Decided on the same plan that is written: a repository root found
        // above `cwd` is never trusted in its place.
        if exact && !project::same_path(&plan.target, cwd) {
            return Err(invalid(
                "folder trust target is not exactly the approved folder",
            ));
        }
        let mut result = match provider {
            "claude" => claude::grant(&path, &plan, &self.paths)?,
            "codex" => {
                let _guard = self
                    .codex_writer
                    .lock()
                    .map_err(|_| io::Error::other("folder trust lock failed"))?;
                codex::grant(&path, &plan)?
            }
            _ => return Err(invalid("unsupported folder trust provider")),
        };
        result.config_path = raw_path;
        Ok(result)
    }
    /// The vendor file this writer would use for `provider` with the given
    /// child environment. Read-only: nothing is opened or created.
    pub fn config_path(&self, provider: &str, environment: &[String]) -> io::Result<PathBuf> {
        let (key, fallback, filename) = match provider {
            "claude" => ("CLAUDE_CONFIG_DIR", self.home.clone(), ".claude.json"),
            "codex" => ("CODEX_HOME", self.home.join(".codex"), "config.toml"),
            _ => return Err(invalid("unsupported folder trust provider")),
        };
        let directory = env_value(environment, key).unwrap_or("").trim();
        Ok(project::clean(
            &if directory.is_empty() {
                fallback
            } else {
                PathBuf::from(directory)
            }
            .join(filename),
        ))
    }
    /// Caller must invoke at Hub startup for the selected default/profile
    /// environments. This is the recovery half of Claude atomic temp writes.
    pub fn reclaim(&self, environments: &[Vec<String>]) -> io::Result<(usize, usize)> {
        let mut seen = std::collections::BTreeSet::new();
        let mut counts = (0, 0);
        for environment in environments {
            let raw_path = self.config_path("claude", environment)?;
            let path = actor_path(&self.paths, &self.hub_cwd, &raw_path)?;
            let target = path.canonicalize().unwrap_or(path);
            if seen.insert(target.clone()) {
                let count = claude::reclaim(&target);
                counts.0 += count.0;
                counts.1 += count.1;
            }
        }
        Ok(counts)
    }
}
