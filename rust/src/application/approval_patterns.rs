//! Private source pattern profiles and the live detector cache share one owner.
use crate::{
    config::{ConfigStore, RuntimePaths},
    files::safe_fs::Dir,
    terminal::session::SessionEngine,
};
use std::{
    collections::BTreeMap,
    io,
    path::PathBuf,
    sync::{Arc, Mutex, Weak},
};
#[cfg(test)]
mod tests;
pub const PROVIDERS: &[&str] = &[
    "claude",
    "codex",
    "copilot",
    "cursor-agent",
    "opencode",
    "grok",
    "command-code",
    "common",
];
pub type Warning = dyn Fn(&'static str) + Send + Sync;
const PATTERN_FILE_CAP: usize = 2 * 1024 * 1024;

/// Validate the exact bytes that the writer and bounded reader share.
pub(crate) fn encode_pattern_file(list: &[String]) -> io::Result<Vec<u8>> {
    let body = serde_json::to_string_pretty(list)
        .map_err(io::Error::other)?
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    let bytes = format!("{body}\n").into_bytes();
    if bytes.len() > PATTERN_FILE_CAP {
        return Err(io::Error::other("approval pattern file exceeds size limit"));
    }
    Ok(bytes)
}

pub struct ApprovalPatterns {
    dir: Dir,
    custom_dir: Dir,
    config: Arc<ConfigStore>,
    core: Weak<SessionEngine>,
    gate: Mutex<()>,
    warning: Arc<Warning>,
}
fn defaults(provider: &str) -> Vec<String> {
    let mut map: BTreeMap<String, Vec<String>> =
        serde_json::from_str(include_str!("approval_patterns/defaults.json"))
            .expect("fixed Go defaults");
    map.remove(provider).unwrap_or_default()
}
pub fn known(provider: &str) -> bool {
    PROVIDERS.contains(&provider)
}
/// CLI-owned patterns are separate from legacy mirrors and built-in profiles.
/// In particular, a custom id such as `claude.custom` must not name a profile.
pub fn custom_pattern_path(paths: &RuntimePaths, provider: &str) -> PathBuf {
    paths
        .root()
        .join("approval-patterns")
        .join("custom-providers")
        .join(format!("{provider}.json"))
}
fn json(bytes: &[u8]) -> io::Result<Vec<String>> {
    if bytes.is_empty() {
        return Ok(vec![]);
    }
    let list: PatternList = crate::proto::wire::decode(bytes)
        .map_err(|_| io::Error::other("invalid approval patterns"))?;
    Ok(list.0.unwrap_or_default())
}
#[derive(serde::Deserialize)]
#[serde(transparent)]
pub struct PatternList(pub Option<Vec<String>>);
impl crate::proto::wire::GoWire for PatternList {
    const GO_TYPE: &'static str = "[]string";
}
impl ApprovalPatterns {
    pub fn new(
        paths: &RuntimePaths,
        config: Arc<ConfigStore>,
        core: Weak<SessionEngine>,
        warning: Arc<Warning>,
    ) -> io::Result<Arc<Self>> {
        let dir = Dir::open(paths.root())?.child_dir("approval-patterns", true)?;
        let custom_dir = dir.child_dir("custom-providers", true)?;
        Ok(Arc::new(Self {
            dir,
            custom_dir,
            config,
            core,
            gate: Mutex::new(()),
            warning,
        }))
    }
    fn read_file(&self, name: &str) -> io::Result<Vec<u8>> {
        Self::read_in(&self.dir, name)
    }
    fn read_in(dir: &Dir, name: &str) -> io::Result<Vec<u8>> {
        let bytes = dir.read(name, PATTERN_FILE_CAP + 1)?;
        if bytes.len() > PATTERN_FILE_CAP {
            return Err(io::Error::other("approval pattern file exceeds size limit"));
        }
        Ok(bytes)
    }
    fn custom_provider(&self, provider: &str) -> io::Result<bool> {
        let snapshot = self.config.snapshot().map_err(io::Error::other)?;
        Ok(
            crate::config::effective_custom_providers(&snapshot.config.custom_providers)
                .iter()
                .any(|p| p.id == provider),
        )
    }
    fn custom_active_bytes(&self, provider: &str) -> io::Result<Vec<u8>> {
        let name = format!("{provider}.json");
        match Self::read_in(&self.custom_dir, &name) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => self.read_file(&name),
            result => result,
        }
    }
    fn custom_active(&self, provider: &str) -> io::Result<Vec<String>> {
        match self.custom_active_bytes(provider) {
            Ok(bytes) => json(&bytes),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(vec![]),
            Err(e) => Err(e),
        }
    }
    fn read(&self, provider: &str, profile: &str) -> io::Result<Vec<String>> {
        if !known(provider) && !self.custom_provider(provider)? {
            return Err(io::Error::other("unknown provider"));
        }
        if !known(provider) {
            // Existing custom sources use the raw file as their official copy.
            // A user override never writes that file or a built-in profile.
            let name = format!("{provider}.json");
            let bytes = if profile == "custom" {
                Self::read_in(&self.custom_dir, &name)
            } else {
                self.read_file(&name)
            };
            return match bytes {
                Ok(bytes) => json(&bytes),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(vec![]),
                Err(e) => Err(e),
            };
        }
        let suffix = if profile == "custom" {
            "custom"
        } else {
            "official"
        };
        match self.read_file(&format!("{provider}.{suffix}.json")) {
            Ok(bytes) => json(&bytes),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(if suffix == "official" {
                defaults(provider)
            } else {
                vec![]
            }),
            Err(e) => Err(e),
        }
    }
    fn write(&self, name: &str, list: &[String]) -> io::Result<()> {
        Self::write_in(&self.dir, name, list)
    }
    fn write_in(dir: &Dir, name: &str, list: &[String]) -> io::Result<()> {
        dir.replace(name, &encode_pattern_file(list)?, 0o600)
    }
    pub fn sync(&self) -> io::Result<()> {
        let _guard = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        for provider in PROVIDERS {
            let legacy = format!("{provider}.json");
            let custom = format!("{provider}.custom.json");
            if self.dir.metadata(&legacy).is_ok() {
                if self.dir.metadata(&custom).is_err() {
                    // Held-directory copy/remove keeps each target private and
                    // never follows a user-controlled symlink during migration.
                    let bytes = self.read_file(&legacy)?;
                    self.dir.replace(&custom, &bytes, 0o600)?;
                }
                let _ = self.dir.remove_file(&legacy);
            }
            let official = format!("{provider}.official.json");
            if self.dir.metadata(&official).is_err() {
                self.write(&official, &defaults(provider))?;
            }
        }
        self.refresh_inner()
    }
    pub fn active(&self) -> io::Result<BTreeMap<String, Vec<String>>> {
        let snapshot = self.config.snapshot().map_err(io::Error::other)?;
        let mut active = PROVIDERS
            .iter()
            .map(|p| {
                self.read(p, snapshot.config.approval_profiles.for_provider(p))
                    .map(|v| ((*p).into(), v))
            })
            .collect::<io::Result<BTreeMap<_, _>>>()?;
        for provider in crate::config::effective_custom_providers(&snapshot.config.custom_providers) {
            if !known(&provider.id) {
                active.insert(provider.id.clone(), self.custom_active(&provider.id)?);
            }
        }
        Ok(active)
    }
    fn refresh_inner(&self) -> io::Result<()> {
        let snapshot = self.config.snapshot().map_err(io::Error::other)?;
        for provider in PROVIDERS {
            let list = self.read(
                provider,
                snapshot.config.approval_profiles.for_provider(provider),
            )?;
            self.write(&format!("{provider}.json"), &list)?;
        }
        self.reload();
        Ok(())
    }
    pub fn refresh(&self) {
        let _guard = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        if self.refresh_inner().is_err() {
            (self.warning)("approval pattern mirrors unavailable");
            self.reload();
        }
    }
    pub fn reload(&self) {
        let mut set = BTreeMap::new();
        let mut providers: Vec<String> = PROVIDERS.iter().map(|p| (*p).into()).collect();
        if let Ok(snapshot) = self.config.snapshot() {
            providers.extend(
                crate::config::effective_custom_providers(&snapshot.config.custom_providers)
                    .into_iter()
                    .map(|p| p.id),
            );
        }
        for provider in providers {
            if set.contains_key(&provider) {
                continue;
            }
            let bytes = if known(&provider) {
                self.read_file(&format!("{provider}.json"))
            } else {
                self.custom_active_bytes(&provider)
            };
            let list = bytes
                .ok()
                .and_then(|b| json(&b).ok())
                .unwrap_or_default();
            set.insert(
                provider,
                list.into_iter()
                    .map(|p| crate::proto::unicode::simple_lower(p.trim()))
                    .filter(|p| !p.is_empty())
                    .collect(),
            );
        }
        if let Some(core) = self.core.upgrade() {
            core.set_approval_phrases(set);
        }
    }
    pub fn write_custom(&self, provider: &str, list: &[String]) -> io::Result<()> {
        if !known(provider) && !self.custom_provider(provider)? {
            return Err(io::Error::other("unknown provider"));
        }
        if !known(provider) {
            let _guard = self.gate.lock().unwrap_or_else(|e| e.into_inner());
            Self::write_in(&self.custom_dir, &format!("{provider}.json"), list)?;
            self.reload();
            return Ok(());
        }
        self.write(&format!("{provider}.custom.json"), list)?;
        self.refresh();
        Ok(())
    }
    pub fn copy_official(&self, provider: &str) -> io::Result<()> {
        let list = self.read(provider, "official")?;
        self.write_custom(provider, &list)
    }
    pub fn asset(&self, name: &str) -> io::Result<Vec<u8>> {
        let name = name.trim();
        if name.is_empty()
            || name.starts_with('.')
            || name.contains(['/', '\\'])
            || !name.ends_with(".json")
        {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        let raw_provider = &name[..name.len() - 5];
        let provider = raw_provider
            .strip_suffix(".official")
            .or_else(|| raw_provider.strip_suffix(".custom"))
            .unwrap_or(raw_provider);
        let custom = self.custom_provider(raw_provider).unwrap_or(false);
        if !known(provider) && !custom {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        if !known(provider) && custom {
            return self.custom_active_bytes(raw_provider);
        }
        self.read_file(name)
    }
    pub async fn remote_sync(
        &self,
        io: Arc<dyn super::slash_commands::SlashIo>,
        cancel: &crate::process::Cancellation,
        effects: &dyn crate::proto::core::CoreEffectSink,
    ) {
        let Ok(snapshot) = self.config.snapshot() else {
            return;
        };
        let sources =
            serde_json::to_value(snapshot.config.approval_pattern_sources.clone().effective())
                .expect("source fields");
        let builtin = PROVIDERS
            .iter()
            .filter_map(|p| {
                sources[*p]
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                    .map(|s| ((*p).into(), s.into(), true))
            })
            .collect();
        let custom = crate::config::effective_custom_providers(&snapshot.config.custom_providers)
            .into_iter()
            .filter(|p| !p.approval_pattern_source.trim().is_empty())
            .map(|p| (p.id, p.approval_pattern_source, false))
            .collect();
        // Go starts both independent owners together. Every configured custom
        // source starts before its own deadline; builtin work cannot starve it.
        tokio::join!(
            self.remote_batch(builtin, io.clone(), cancel, effects, &snapshot.config),
            self.remote_batch(custom, io, cancel, effects, &snapshot.config)
        );
    }
    async fn remote_batch(
        &self,
        targets: Vec<(String, String, bool)>,
        io: Arc<dyn super::slash_commands::SlashIo>,
        cancel: &crate::process::Cancellation,
        effects: &dyn crate::proto::core::CoreEffectSink,
        config: &crate::config::Config,
    ) {
        use futures_util::{StreamExt, stream};
        let concurrency = targets.len().max(1);
        let operations = targets.into_iter().map(|(provider, source, official)| {
            let io = io.clone();
            async move { (provider, official, io.read(&source).await) }
        });
        // Retain all task futures in this startup owner; dropping on
        // the source30s deadline cancels every pending HTTP future together.
        let mut operations = stream::iter(operations).buffer_unordered(concurrency);
        let deadline = tokio::time::sleep(std::time::Duration::from_secs(30));
        tokio::pin!(deadline);
        let mut changed = vec![];
        loop {
            let next = tokio::select! {
                result=operations.next()=>result,
                _=cancel.cancelled()=>return,
                _=&mut deadline=>{(self.warning)("approval pattern source synchronization timed out");break;},
            };
            let Some((provider, official, body)) = next else {
                break;
            };
            let Ok(body) = body else {
                (self.warning)("approval pattern source unavailable");
                continue;
            };
            let list = parse_markdown(&crate::proto::wire::go_utf8_lossy(&body));
            if list.is_empty() {
                continue;
            }
            // Recheck after the fetch under the same gate as write_custom:
            // an override saved while the source was in flight wins as well.
            let _guard = self.gate.lock().unwrap_or_else(|e| e.into_inner());
            if !official {
                match self.custom_dir.metadata(&format!("{provider}.json")) {
                    Ok(_) => continue,
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    Err(_) => {
                        (self.warning)("approval pattern profile unavailable");
                        continue;
                    }
                }
            }
            let current = if official {
                self.read(&provider, "official")
            } else {
                self.read_file(&format!("{provider}.json"))
                    .and_then(|b| json(&b))
            };
            if official && current.is_err() {
                (self.warning)("approval pattern profile unavailable");
                continue;
            }
            if current.is_ok_and(|current| current == list) {
                continue;
            }
            let name = if official {
                format!("{provider}.official.json")
            } else {
                format!("{provider}.json")
            };
            if self.write(&name, &list).is_err() {
                (self.warning)("approval pattern source write failed");
                continue;
            }
            changed.push((provider, official));
        }
        if changed.is_empty() {
            return;
        }
        if changed.iter().any(|(p, official)| {
            *official && config.approval_profiles.for_provider(p) == "official"
        }) {
            self.refresh();
        } else {
            self.reload();
        }
        let Some(core) = self.core.upgrade() else {
            (self.warning)("approval pattern update delivery failed");
            return;
        };
        // SessionCore queues updates behind each UI's initial snapshot and
        // expands live delivery before the transport effect owner sees it.
        let publication = core.broadcast_ui(crate::proto::Message {
            r#type: "approval_patterns_updated".into(),
            providers: changed.into_iter().map(|(p, _)| p).collect(),
            ..Default::default()
        });
        if effects.apply(publication).await.is_err() {
            (self.warning)("approval pattern update delivery failed");
        }
    }
}
fn parse_markdown(text: &str) -> Vec<String> {
    static PATTERN: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let pattern = PATTERN.get_or_init(|| {
        regex::Regex::new("(?m)^[ \\t]*[-*][ \\t]+`([^`]+)`[ \\t]*$")
            .expect("fixed Go pattern expression")
    });
    let mut seen = std::collections::BTreeSet::new();
    let mut result = vec![];
    for captures in pattern.captures_iter(text) {
        let value = captures[1].trim();
        if !value.is_empty() && seen.insert(value.to_owned()) {
            result.push(value.to_owned());
        }
    }
    result
}
