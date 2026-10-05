//! Hub-owned instruction injection lifetime and crash-recovery journal.
mod native;
pub use native::NativeInstructionRootResolver;
#[cfg(test)]
mod tests;
use crate::{
    config::{ConfigError, ConfigStore, RuntimePaths},
    process::Cancellation,
    proto::{
        core::*,
        unicode::simple_lower,
        wire::{self, Field, Schema},
    },
    terminal::session::SessionEngine,
    wrapper::approval_injection::InstructionFiles,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::{Path, PathBuf},
    sync::Arc,
};
pub type Warning = dyn Fn(&'static str) + Send + Sync;
pub trait InstructionRootResolver: Send + Sync {
    fn root<'a>(&'a self, cwd: &'a Path, cancel: &'a Cancellation) -> CoreFuture<'a, PathBuf>;
}
#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct Target {
    #[serde(deserialize_with = "null_default")]
    path: PathBuf,
    #[serde(deserialize_with = "null_default")]
    providers: Vec<String>,
    #[serde(deserialize_with = "null_default")]
    mode: String,
}
impl Target {
    fn provider(&self) -> &str {
        if self.mode == "claude_import" {
            "claude"
        } else {
            "codex"
        }
    }
}
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Journal {
    #[serde(deserialize_with = "null_default")]
    version: i64,
    #[serde(deserialize_with = "null_default")]
    targets: Vec<Target>,
}
fn null_default<'de, D: serde::Deserializer<'de>, T: Deserialize<'de> + Default>(
    deserializer: D,
) -> Result<T, D::Error> {
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}
fn key(path: &Path) -> String {
    let cleaned = crate::files::scope::clean(path)
        .to_string_lossy()
        .into_owned();
    if cfg!(windows) {
        simple_lower(&cleaned)
    } else {
        cleaned
    }
}
fn providers(raw: &[String]) -> Vec<String> {
    let mut seen = raw
        .iter()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .collect::<BTreeSet<_>>();
    let mut out = Vec::new();
    for p in ["claude", "codex", "copilot", "cursor-agent"] {
        if seen.remove(p) {
            out.push(p.into());
        }
    }
    for p in raw.iter().map(|v| v.trim()) {
        if seen.remove(p) {
            out.push(p.into());
        }
    }
    out
}
fn merge(targets: Vec<Target>) -> Vec<Target> {
    let mut out: Vec<Target> = Vec::new();
    let mut indices = BTreeMap::new();
    for mut target in targets {
        if target.path.to_string_lossy().trim().is_empty() {
            continue;
        }
        target.path = crate::files::scope::clean(&target.path);
        let k = key(&target.path);
        if let Some(&index) = indices.get(&k) {
            let old: &mut Target = &mut out[index];
            old.providers.extend(target.providers);
            old.providers = providers(&old.providers);
        } else {
            target.providers = providers(&target.providers);
            indices.insert(k, out.len());
            out.push(target);
        }
    }
    out
}
fn environment<'a>(values: &'a [String], name: &str) -> Option<&'a str> {
    values.iter().rev().find_map(|v| {
        v.split_once('=')
            .filter(|(k, _)| {
                if cfg!(windows) {
                    k.eq_ignore_ascii_case(name)
                } else {
                    *k == name
                }
            })
            .map(|(_, v)| v)
    })
}
pub struct InstructionRules {
    config: Arc<ConfigStore>,
    core: Arc<SessionEngine>,
    files: InstructionFiles,
    home: PathBuf,
    cwd: PathBuf,
    environment: Vec<String>,
    journal: PathBuf,
    resolver: Arc<dyn InstructionRootResolver>,
    known: tokio::sync::Mutex<BTreeMap<String, Target>>,
    warning: Arc<Warning>,
}
impl InstructionRules {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        paths: RuntimePaths,
        home: PathBuf,
        environment: Vec<String>,
        cwd: PathBuf,
        config: Arc<ConfigStore>,
        core: Arc<SessionEngine>,
        resolver: Arc<dyn InstructionRootResolver>,
        warning: Arc<Warning>,
    ) -> io::Result<Arc<Self>> {
        if !cwd.is_absolute() {
            return Err(io::Error::other("explicit instruction cwd required"));
        }
        Ok(Arc::new(Self {
            journal: paths.root().join("approval-rule-targets.json"),
            files: InstructionFiles::new(paths, home.clone())?,
            home,
            cwd,
            environment,
            config,
            core,
            resolver,
            known: tokio::sync::Mutex::new(BTreeMap::new()),
            warning,
        }))
    }
    fn absolute(&self, path: PathBuf) -> PathBuf {
        crate::files::scope::clean(&if path.is_absolute() {
            path
        } else {
            self.cwd.join(path)
        })
    }
    async fn targets(&self, cancel: &Cancellation) -> (Vec<Target>, Vec<Target>) {
        let mut active = Vec::new();
        let mut legacy = Vec::new();
        for id in self.core.registered_session_ids() {
            let Some(details) = self.core.details(id) else {
                continue;
            };
            let s = &details.snapshot;
            if !details.connected
                || matches!(s.state.as_str(), "completed" | "error" | "disconnected")
            {
                continue;
            }
            let provider = s.provider.as_str();
            let target = match provider {
                "claude" => {
                    let selected = if !details.transcript.claude_dir.trim().is_empty() {
                        details.transcript.claude_dir.as_str()
                    } else {
                        environment(&self.environment, "CLAUDE_CONFIG_DIR").unwrap_or("")
                    };
                    let root = if selected.trim().is_empty() {
                        self.home.join(".claude")
                    } else {
                        self.absolute(selected.into())
                    };
                    Some(Target {
                        path: root.join("CLAUDE.md"),
                        providers: vec![provider.into()],
                        mode: "claude_import".into(),
                    })
                }
                "codex" => {
                    let selected = if !details.transcript.codex_home.trim().is_empty() {
                        details.transcript.codex_home.as_str()
                    } else {
                        environment(&self.environment, "CODEX_HOME").unwrap_or("")
                    };
                    let root = if selected.trim().is_empty() {
                        self.home.join(".codex")
                    } else {
                        self.absolute(selected.into())
                    };
                    Some(Target {
                        path: root.join("AGENTS.md"),
                        providers: vec![provider.into()],
                        mode: "shared_block".into(),
                    })
                }
                "copilot" | "cursor-agent" | "grok" | "opencode" => {
                    if s.cwd.trim().is_empty() {
                        None
                    } else {
                        Some(Target {
                            path: self
                                .resolver
                                .root(Path::new(s.cwd.trim()), cancel)
                                .await
                                .join("AGENTS.md"),
                            providers: vec![provider.into()],
                            mode: "shared_block".into(),
                        })
                    }
                }
                _ => None,
            };
            if let Some(target) = target {
                active.push(target);
            }
            if provider == "codex" && !s.cwd.trim().is_empty() {
                legacy.push(Target {
                    path: self
                        .resolver
                        .root(Path::new(s.cwd.trim()), cancel)
                        .await
                        .join("AGENTS.md"),
                    providers: vec!["codex".into()],
                    mode: "shared_block".into(),
                });
            }
        }
        (merge(active), merge(legacy))
    }
    fn persist(&self, known: &BTreeMap<String, Target>) {
        if known.is_empty() {
            if self.files.remove_file(&self.journal).is_err() {
                (self.warning)("approval instruction journal removal failed");
            }
            return;
        }
        let state = Journal {
            version: 1,
            targets: known.values().cloned().collect(),
        };
        if serde_json::to_vec_pretty(&state)
            .and_then(|v| {
                self.files
                    .write(&self.journal, &v, true)
                    .map_err(serde_json::Error::io)
            })
            .is_err()
        {
            (self.warning)("approval instruction journal persistence failed");
        }
    }
    fn remove(&self, targets: Vec<Target>, known: &mut BTreeMap<String, Target>, delegation: bool) {
        for target in merge(targets) {
            if self
                .files
                .remove(target.provider(), &target.path, false)
                .is_err()
            {
                (self.warning)("approval instruction removal failed");
                continue;
            }
            if delegation
                && self
                    .files
                    .delegation(target.provider(), &target.path, false)
                    .is_err()
            {
                (self.warning)("delegation instruction removal failed");
            }
            known.remove(&key(&target.path));
        }
        self.persist(known);
    }
    /// Startup recovery occurs before accepting wrappers. Corrupt journals never
    /// authorize mutations, and failed targets remain for the next startup.
    pub async fn recover(&self) {
        const SCHEMAS: &[Schema] = &[
            Schema {
                name: "InstructionJournal",
                fields: &[
                    Field {
                        name: "version",
                        kind: "int",
                    },
                    Field {
                        name: "targets",
                        kind: "[]InstructionTarget",
                    },
                ],
            },
            Schema {
                name: "InstructionTarget",
                fields: &[
                    Field {
                        name: "path",
                        kind: "string",
                    },
                    Field {
                        name: "providers",
                        kind: "[]string",
                    },
                    Field {
                        name: "mode",
                        kind: "string",
                    },
                ],
            },
        ];
        let mut known = self.known.lock().await;
        let bytes = match self.files.read(&self.journal) {
            Ok(v) => v,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return,
            Err(_) => {
                (self.warning)("approval instruction journal read failed");
                return;
            }
        };
        let state = wire::decode_go_json_members(&bytes)
            .ok()
            .and_then(|_| {
                wire::decode_http_schema::<Option<Journal>>(&bytes, "InstructionJournal", SCHEMAS)
                    .ok()
            })
            .map(Option::unwrap_or_default);
        let Some(state) = state else {
            (self.warning)("approval instruction journal corrupt");
            return;
        };
        for target in state.targets {
            known.insert(key(&target.path), target);
        }
        let targets = known.values().cloned().collect();
        self.remove(targets, &mut known, false);
    }
    /// Register, reattach, settings publication and startup call the same sweep.
    pub async fn registered(&self, cancel: &Cancellation) {
        // Read the published flag only after owning the mutation lane: a sweep
        // waiting behind disable must not re-inject from an older snapshot.
        let mut known = self.known.lock().await;
        let Ok(config) = self.config.snapshot() else {
            (self.warning)("approval instruction settings unavailable");
            return;
        };
        if !config.config.approval.enabled {
            return;
        }
        let (active, legacy) = self.targets(cancel).await;
        for target in &active {
            if self.files.inject(target.provider(), &target.path).is_err() {
                (self.warning)("approval instruction injection failed");
                continue;
            }
            if self
                .files
                .delegation(
                    target.provider(),
                    &target.path,
                    config.config.user_prefs.spawn.delegation_auto,
                )
                .is_err()
            {
                (self.warning)("delegation instruction injection failed");
            }
            let k = key(&target.path);
            if let Some(existing) = known.get_mut(&k) {
                existing.providers.extend(target.providers.clone());
                existing.providers = providers(&existing.providers);
            } else {
                known.insert(k, target.clone());
            }
        }
        self.persist(&known);
        let keys = active.iter().map(|v| key(&v.path)).collect::<BTreeSet<_>>();
        let inactive = merge(legacy.into_iter().chain(known.values().cloned()).collect())
            .into_iter()
            .filter(|v| !keys.contains(&key(&v.path)))
            .collect();
        self.remove(inactive, &mut known, true);
    }
    pub async fn ended(&self, cancel: &Cancellation) {
        let mut known = self.known.lock().await;
        let (active, legacy) = self.targets(cancel).await;
        let keys = active.iter().map(|v| key(&v.path)).collect::<BTreeSet<_>>();
        let inactive = merge(legacy.into_iter().chain(known.values().cloned()).collect())
            .into_iter()
            .filter(|v| !keys.contains(&key(&v.path)))
            .collect();
        self.remove(inactive, &mut known, true);
    }
    pub async fn shutdown(&self, cancel: &Cancellation) {
        let mut known = self.known.lock().await;
        let (active, legacy) = self.targets(cancel).await;
        let all = known
            .values()
            .cloned()
            .chain(active)
            .chain(legacy)
            .collect();
        self.remove(all, &mut known, true);
    }
    pub fn status(&self) -> Result<serde_json::Value, ConfigError> {
        let config = self.config.snapshot()?;
        Ok(
            serde_json::json!({"enabled":config.config.approval.enabled,"first_launch_shown":config.config.approval.first_launch_shown}),
        )
    }
    /// Source publication precedes file operations, then best-effort config Save.
    pub async fn change(&self, action: &str, cancel: &Cancellation) -> Result<(), ConfigError> {
        let published = loop {
            let snapshot = self.config.snapshot()?;
            let mut next = snapshot.config;
            match action {
                "enable" => {
                    next.approval.enabled = true;
                    next.approval.first_launch_shown = true;
                }
                "disable" => next.approval.enabled = false,
                _ => next.approval.first_launch_shown = true,
            }
            match self
                .config
                .publish_legacy_without_persist(snapshot.revision, next)
            {
                Ok(v) => break v,
                Err(ConfigError::Conflict { .. }) => continue,
                Err(e) => return Err(e),
            }
        };
        match action {
            "enable" => self.registered(cancel).await,
            "disable" => self.shutdown(cancel).await,
            _ => {}
        }
        if self
            .config
            .persist_published_legacy(published.revision)
            .is_err()
        {
            (self.warning)("approval instruction settings persistence failed");
        }
        Ok(())
    }
}
