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
    fn config_path(&self, provider: &str, environment: &[String]) -> io::Result<PathBuf> {
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
