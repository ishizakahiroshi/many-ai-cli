//! Persisted provider layers from the frozen Go provider stores.
//!
//! Every root comes from RuntimePaths. Reads retain source diagnostics, writes
//! validate before touching files, and reload publishes one immutable snapshot
//! only after all fatal source reads succeed. Distribution networking/acceptance
//! and provider execution deliberately do not belong to this persistence layer.
mod accepted;
mod distribution;
mod history;
#[cfg(test)]
mod tests;

use super::{
    registry::{Registry, default_adapters, diag, embedded_definitions},
    validation::{valid_id, validate_definition},
};
use crate::{
    config::{ConfigStore, Resource, RuntimePaths, legacy_provider_definitions},
    files::safe_fs::Dir,
    proto::provider::*,
};
pub use accepted::AcceptedDistributionStore;
pub use distribution::{DistributionPointerSnapshot, DistributionStore, diff_distribution};
pub use history::{HistoryStore, definition_digest, revision_id};
use serde::Serialize;
use std::{
    fmt, io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock},
};

#[derive(Debug)]
pub enum StoreError {
    AlreadyExists,
    Invalid(String),
    RevisionConflict { expected: String, current: String },
    OverrideClearsValue(Vec<String>),
    Io(io::Error),
    Decode(String),
    Unavailable(&'static str),
}
impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyExists => f.write_str("provider already exists"),
            Self::Invalid(s) | Self::Decode(s) => f.write_str(s),
            Self::RevisionConflict { expected, current } => write!(
                f,
                "provider revision conflict: expected {expected:?}, current {current:?}"
            ),
            Self::OverrideClearsValue(fields) => write!(
                f,
                "provider override cannot clear a distributed value: {}",
                fields.join(", ")
            ),
            Self::Io(e) => e.fmt(f),
            Self::Unavailable(s) => f.write_str(s),
        }
    }
}
impl std::error::Error for StoreError {}
impl From<io::Error> for StoreError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<serde_json::Error> for StoreError {
    fn from(e: serde_json::Error) -> Self {
        Self::Decode(e.to_string())
    }
}
impl StoreError {
    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::Io(e) if e.kind() == io::ErrorKind::NotFound)
    }
}
pub type Result<T> = std::result::Result<T, StoreError>;

/// Lazily acquire a directory capability. An absent read never creates a root.
struct Root {
    path: PathBuf,
    held: Mutex<Option<Arc<Dir>>>,
}
impl Root {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            held: Mutex::new(None),
        }
    }
    fn open(&self, create: bool) -> Result<Arc<Dir>> {
        let mut held = self
            .held
            .lock()
            .map_err(|_| StoreError::Unavailable("provider directory lock poisoned"))?;
        if let Some(dir) = &*held {
            return Ok(dir.clone());
        }
        let dir = Arc::new(if create {
            Dir::open_or_create_private(&self.path)?
        } else {
            Dir::open(&self.path)?
        });
        *held = Some(dir.clone());
        Ok(dir)
    }
}
fn recover(dir: &Dir, name: &str) {
    let shadow = format!("{name}.atomic-replace-shadow");
    match dir.metadata(name) {
        Ok(_) => {
            let _ = dir.remove_file(&shadow);
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let _ = dir.rename_to(&shadow, dir, name);
        }
        Err(_) => {}
    }
}
fn read_recover(dir: &Dir, name: &str, cap: usize) -> Result<Vec<u8>> {
    recover(dir, name);
    Ok(dir.read(name, cap)?)
}
fn content_digest(raw: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(raw)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn pretty<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    // Match json.MarshalIndent's declaration order and HTML/JavaScript escapes.
    let value = serde_json::to_string_pretty(value)?;
    Ok(value
        .replace('&', "\\u0026")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
        .into_bytes())
}
fn write_json<T: Serialize>(dir: &Dir, name: &str, value: &T) -> Result<()> {
    let raw = pretty(value)?;
    recover(dir, name);
    Ok(dir.replace(name, &raw, 0o600)?)
}
fn sorted_entries(dir: &Dir) -> Result<Vec<String>> {
    let mut entries = dir.entries()?;
    entries.sort();
    Ok(entries)
}
pub fn is_builtin(id: &str) -> bool {
    BUILTIN_PROVIDER_IDS.contains(&id)
}
pub fn validate_user_id(id: &str) -> Result<()> {
    valid_id(id).map_err(StoreError::Invalid)?;
    if is_builtin(id) || id == "shell" {
        return Err(StoreError::Invalid(format!(
            "provider id {id:?} is reserved"
        )));
    }
    Ok(())
}
fn validate_history_id(id: &str) -> Result<()> {
    valid_id(id).map_err(StoreError::Invalid)
}
fn validate_user(def: &Definition) -> Result<Vec<u8>> {
    let raw = pretty(def)?;
    let (_, diagnostics) =
        validate_definition(&raw, &default_adapters()).map_err(StoreError::Decode)?;
    if diagnostics.iter().any(Diagnostic::is_error) {
        return Err(StoreError::Invalid("provider definition is invalid".into()));
    }
    validate_user_id(&def.id)?;
    Ok(raw)
}

pub struct LoadedDefinitions {
    pub definitions: Vec<Definition>,
    pub diagnostics: Vec<Diagnostic>,
}
impl LoadedDefinitions {
    fn empty() -> Self {
        Self {
            definitions: vec![],
            diagnostics: vec![],
        }
    }
}

pub struct FileStore {
    root: Root,
    gate: Mutex<()>,
}
impl FileStore {
    pub fn new(paths: &RuntimePaths) -> Self {
        Self {
            root: Root::new(paths.resource(Resource::ProviderDefinitions)),
            gate: Mutex::new(()),
        }
    }
    pub fn load(&self) -> Result<LoadedDefinitions> {
        let _guard = self
            .gate
            .lock()
            .map_err(|_| StoreError::Unavailable("provider store lock poisoned"))?;
        let root = match self.root.open(false) {
            Ok(r) => r,
            Err(e) if e.is_not_found() => return Ok(LoadedDefinitions::empty()),
            Err(e) => return Err(e),
        };
        let mut loaded = LoadedDefinitions::empty();
        for name in sorted_entries(&root)? {
            if Path::new(&name).extension().is_none_or(|x| x != "json")
                || root.child_dir(&name, false).is_ok()
            {
                continue;
            }
            let raw = root.read(&name, usize::MAX)?;
            let mut definition = match crate::proto::decode_wire::<Definition>(&raw) {
                Ok(d) => d,
                Err(_) => {
                    loaded.diagnostics.push(diag(
                        "invalid_user_definition",
                        "error",
                        &name,
                        "definition could not be decoded",
                    ));
                    continue;
                }
            };
            definition.source.origin = ORIGIN_USER.into();
            let (_, mut diagnostics) = match validate_definition(&raw, &default_adapters()) {
                Ok(v) => v,
                Err(_) => {
                    loaded.diagnostics.push(diag(
                        "invalid_user_definition",
                        "error",
                        &name,
                        "definition could not be validated",
                    ));
                    continue;
                }
            };
            let invalid = diagnostics.iter().any(Diagnostic::is_error);
            for d in &mut diagnostics {
                d.field = format!("{name}.{}", d.field);
            }
            loaded.diagnostics.extend(diagnostics);
            if !invalid {
                loaded.definitions.push(definition);
            }
        }
        Ok(loaded)
    }
    pub fn create_new(&self, def: &Definition) -> Result<()> {
        let raw = validate_user(def)?;
        let _guard = self
            .gate
            .lock()
            .map_err(|_| StoreError::Unavailable("provider store lock poisoned"))?;
        let root = self.root.open(true)?;
        match root.create_new(&format!("{}.json", def.id), &raw, 0o600) {
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(StoreError::AlreadyExists),
            other => other.map_err(StoreError::Io),
        }
    }
    pub fn save(&self, def: &Definition) -> Result<()> {
        let raw = validate_user(def)?;
        let _guard = self
            .gate
            .lock()
            .map_err(|_| StoreError::Unavailable("provider store lock poisoned"))?;
        let root = self.root.open(true)?;
        let name = format!("{}.json", def.id);
        recover(&root, &name);
        Ok(root.replace(&name, &raw, 0o600)?)
    }
    pub fn delete(&self, id: &str) -> Result<()> {
        validate_user_id(id)?;
        let _guard = self
            .gate
            .lock()
            .map_err(|_| StoreError::Unavailable("provider store lock poisoned"))?;
        let result = self.root.open(false).and_then(|dir| {
            dir.remove_file(&format!("{id}.json"))
                .map_err(StoreError::Io)
        });
        match result {
            Err(e) if e.is_not_found() => Err(StoreError::Invalid(format!(
                "provider {id:?} is not stored"
            ))),
            other => other,
        }
    }
}

/// Diagnostics from source loading are separate from Registry::diagnostics:
/// Go's registry owns only build diagnostics; callers receive load diagnostics.
#[derive(Clone)]
pub struct ProviderSnapshot {
    pub registry: Arc<Registry>,
    pub diagnostics: Vec<Diagnostic>,
}
#[derive(Clone, Copy)]
enum LoadMode {
    Startup,
    Reload,
    Baseline,
}
pub struct ProviderRegistryStore {
    config: Arc<ConfigStore>,
    pub definitions: FileStore,
    pub history: HistoryStore,
    pub accepted: AcceptedDistributionStore,
    snapshot: RwLock<Option<ProviderSnapshot>>,
    reload_gate: Mutex<()>,
    /// Serialize a service mutation and its following reload as one caller unit.
    pub(crate) mutation_gate: Mutex<()>,
}
impl ProviderRegistryStore {
    pub fn new(paths: &RuntimePaths, config: Arc<ConfigStore>) -> Result<Self> {
        let store = Self {
            config,
            definitions: FileStore::new(paths),
            history: HistoryStore::new(paths),
            accepted: AcceptedDistributionStore::new(paths),
            snapshot: RwLock::new(None),
            reload_gate: Mutex::new(()),
            mutation_gate: Mutex::new(()),
        };
        // NewServer starts with available embedded/legacy layers and records a
        // user-store failure as a diagnostic instead of aborting the Hub.
        let initial = store.build(LoadMode::Startup)?;
        *store
            .snapshot
            .write()
            .map_err(|_| StoreError::Unavailable("provider registry lock poisoned"))? =
            Some(initial);
        Ok(store)
    }
    pub fn snapshot(&self) -> Result<ProviderSnapshot> {
        self.snapshot
            .read()
            .map_err(|_| StoreError::Unavailable("provider registry lock poisoned"))?
            .clone()
            .ok_or(StoreError::Unavailable("provider registry is unavailable"))
    }
    fn build(&self, mode: LoadMode) -> Result<ProviderSnapshot> {
        let user = match self.definitions.load() {
            Ok(loaded) => loaded,
            Err(error) => match mode {
                LoadMode::Reload => return Err(error),
                LoadMode::Startup => LoadedDefinitions {
                    definitions: vec![],
                    diagnostics: vec![diag(
                        "provider_store_load",
                        "error",
                        "providers.d",
                        "user provider definitions could not be loaded",
                    )],
                },
                LoadMode::Baseline => LoadedDefinitions::empty(),
            },
        };
        let overrides = match mode {
            LoadMode::Baseline => LoadedDefinitions::empty(),
            _ => match self.history.load_overrides() {
                Ok(loaded) => loaded,
                Err(error) => match mode {
                    LoadMode::Reload => return Err(error),
                    // NewServer intentionally discards this load error.
                    _ => LoadedDefinitions::empty(),
                },
            },
        };
        let accepted = self.accepted.load_definitions();
        let cfg = self
            .config
            .snapshot()
            .map_err(|_| StoreError::Unavailable("configuration unavailable"))?;
        let (legacy, legacy_diagnostics) =
            legacy_provider_definitions(&cfg.config.custom_providers);
        let embedded = embedded_definitions()?;
        let mut embedded_diagnostics = vec![];
        for definition in &embedded {
            let (_, mut diagnostics) =
                validate_definition(&to_go_json(definition)?, &default_adapters())
                    .map_err(StoreError::Decode)?;
            for diagnostic in &mut diagnostics {
                diagnostic.field = format!("{}.{}", definition.id, diagnostic.field);
            }
            embedded_diagnostics.extend(diagnostics);
        }
        let registry = Arc::new(Registry::build(
            Layers {
                embedded: Some(embedded),
                legacy: Some(legacy),
                user: Some(user.definitions),
                overrides: Some(overrides.definitions),
                accepted_distribution: Some(accepted.definitions),
                ..Default::default()
            },
            &default_adapters(),
        ));
        let mut diagnostics = registry.diagnostics();
        diagnostics.extend(embedded_diagnostics);
        diagnostics.extend(legacy_diagnostics);
        // Exact source caller projection: startup appends successful load
        // diagnostics; reload intentionally drops them unless the load failed.
        if matches!(mode, LoadMode::Startup) {
            diagnostics.extend(user.diagnostics);
            diagnostics.extend(overrides.diagnostics);
        }
        diagnostics.extend(accepted.diagnostics);
        Ok(ProviderSnapshot {
            registry,
            diagnostics,
        })
    }
    pub fn reload(&self) -> Result<ProviderSnapshot> {
        let _guard = self
            .reload_gate
            .lock()
            .map_err(|_| StoreError::Unavailable("provider reload lock poisoned"))?;
        let next = self.build(LoadMode::Reload)?;
        *self
            .snapshot
            .write()
            .map_err(|_| StoreError::Unavailable("provider registry lock poisoned"))? =
            Some(next.clone());
        Ok(next)
    }
    pub fn baseline(&self, id: &str) -> Result<Definition> {
        Ok(self
            .build(LoadMode::Baseline)?
            .registry
            .lookup(id)
            .map(|d| d.definition)
            .unwrap_or_else(|| Definition {
                id: id.into(),
                ..Default::default()
            }))
    }
}
