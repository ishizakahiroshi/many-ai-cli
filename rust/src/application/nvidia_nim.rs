//! Fixed NVIDIA authority and private key state; keys never enter protocol DTOs.
mod native;
use crate::{
    config::{ConfigError, ConfigStore, RuntimePaths},
    files::safe_fs::Dir,
    process::Cancellation,
    proto::core::CoreFuture,
};
pub use native::NativeNvidiaIo;
use serde::Serialize;
use std::{collections::BTreeSet, io, sync::Arc};
pub const CATALOG_URL: &str = "https://integrate.api.nvidia.com/v1/models";
const KEY_LIMIT: usize = 1024 * 1024;
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum KeySource {
    None,
    Env,
    File,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyError {
    Empty,
    Invalid,
    Read,
    Save,
    Delete,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogError {
    Timeout,
    Http(u16),
    Request,
    Parse,
    Cancelled,
}
impl CatalogError {
    pub fn test_code(self) -> &'static str {
        match self {
            Self::Timeout | Self::Http(408) => "timeout",
            Self::Http(401) => "unauthorized",
            Self::Http(402) => "payment_required",
            Self::Http(403) => "forbidden",
            Self::Http(404) => "not_found",
            Self::Http(429) => "rate_limited",
            Self::Http(status) if status >= 500 => "server_error",
            Self::Http(_) => "request_rejected",
            _ => "connection_failed",
        }
    }
}
fn validate(key: &str) -> Result<(), KeyError> {
    if key.contains(['\0', '\r', '\n']) {
        Err(KeyError::Invalid)
    } else {
        Ok(())
    }
}
fn resolve(root: &Dir, environment: &[String]) -> Result<(String, KeySource), KeyError> {
    let env = environment
        .iter()
        .rev()
        .find_map(|entry| {
            entry
                .split_once('=')
                .filter(|(name, _)| {
                    if cfg!(windows) {
                        name.eq_ignore_ascii_case("NVIDIA_API_KEY")
                    } else {
                        *name == "NVIDIA_API_KEY"
                    }
                })
                .map(|(_, v)| v.trim())
        })
        .unwrap_or("");
    if !env.is_empty() {
        validate(env)?;
        return Ok((env.into(), KeySource::Env));
    }
    let bytes = match root
        .child_dir("secrets", false)
        .and_then(|dir| dir.read("nvidia_api_key", KEY_LIMIT + 1))
    {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Ok((String::new(), KeySource::None));
        }
        Err(_) => return Err(KeyError::Read),
    };
    if bytes.len() > KEY_LIMIT {
        return Err(KeyError::Read);
    }
    let key = String::from_utf8_lossy(&bytes).trim().to_owned();
    validate(&key)?;
    let source = if key.is_empty() {
        KeySource::None
    } else {
        KeySource::File
    };
    Ok((key, source))
}
/// Presence-only Doctor seam; never returns or formats a key.
pub fn resolve_key(paths: &RuntimePaths, environment: &[String]) -> io::Result<String> {
    let root = Dir::open(paths.root())?;
    resolve(&root, environment)
        .map(|(key, _)| key)
        .map_err(|_| io::Error::other("NVIDIA API key status unavailable"))
}
pub fn key_configured(paths: &RuntimePaths, environment: &[String]) -> io::Result<bool> {
    resolve_key(paths, environment).map(|key| !key.is_empty())
}

pub trait NvidiaIo: Send + Sync {
    fn catalog<'a>(
        &'a self,
        key: &'a str,
        parse_body: bool,
        cancel: &'a Cancellation,
    ) -> CoreFuture<'a, Result<Vec<u8>, CatalogError>>;
}
pub struct NvidiaDependencies {
    pub paths: RuntimePaths,
    pub config: Arc<ConfigStore>,
    pub environment: Vec<String>,
    pub io: Arc<dyn NvidiaIo>,
}
pub struct NvidiaNim {
    root: Dir,
    config: Arc<ConfigStore>,
    environment: Vec<String>,
    io: Arc<dyn NvidiaIo>,
}
#[derive(Serialize, Debug)]
pub struct SettingsStatus {
    pub enabled: bool,
    pub api_key_configured: bool,
    pub api_key_source: KeySource,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsError {
    Status,
    Environment,
    InvalidKey,
    SaveKey,
    SaveConfig,
    DeleteKey,
}
impl NvidiaNim {
    pub fn new(deps: NvidiaDependencies) -> io::Result<Arc<Self>> {
        Ok(Arc::new(Self {
            root: Dir::open(deps.paths.root())?,
            config: deps.config,
            environment: deps.environment,
            io: deps.io,
        }))
    }
    pub fn status(&self) -> Result<SettingsStatus, SettingsError> {
        let cfg = self.config.snapshot().map_err(|_| SettingsError::Status)?;
        let (key, source) =
            resolve(&self.root, &self.environment).map_err(|_| SettingsError::Status)?;
        Ok(SettingsStatus {
            enabled: cfg.config.nvidia_nim.enabled,
            api_key_configured: !key.is_empty(),
            api_key_source: source,
        })
    }
    pub fn save_settings(&self, enabled: bool, key: &str) -> Result<SettingsStatus, SettingsError> {
        let status = self.status()?;
        if !key.is_empty() {
            if status.api_key_source == KeySource::Env {
                return Err(SettingsError::Environment);
            }
            let key = key.trim();
            if key.is_empty() || validate(key).is_err() {
                return Err(SettingsError::InvalidKey);
            }
            self.root
                .child_dir("secrets", true)
                .and_then(|dir| dir.replace("nvidia_api_key", key.as_bytes(), 0o600))
                .map_err(|_| SettingsError::SaveKey)?;
        }
        loop {
            let mut cfg = self
                .config
                .snapshot()
                .map_err(|_| SettingsError::SaveConfig)?;
            cfg.config.nvidia_nim.enabled = enabled;
            match self
                .config
                .publish_then_persist_legacy(cfg.revision, cfg.config)
            {
                Ok(_) => break,
                Err(ConfigError::Conflict { .. }) => continue,
                Err(_) => return Err(SettingsError::SaveConfig),
            }
        }
        self.status()
    }
    pub fn delete_key(&self) -> Result<SettingsStatus, SettingsError> {
        if self.status()?.api_key_source == KeySource::Env {
            return Err(SettingsError::Environment);
        }
        match self
            .root
            .child_dir("secrets", false)
            .and_then(|dir| dir.remove_file("nvidia_api_key"))
        {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return Err(SettingsError::DeleteKey),
        }
        self.status()
    }
    pub async fn test(
        &self,
        cancel: &Cancellation,
    ) -> Result<Result<(), CatalogError>, SettingsError> {
        let (key, _) = resolve(&self.root, &self.environment).map_err(|_| SettingsError::Status)?;
        if key.is_empty() {
            return Err(SettingsError::InvalidKey);
        }
        Ok(self.io.catalog(&key, false, cancel).await.map(|_| ()))
    }
    pub async fn models(&self, cancel: &Cancellation) -> Result<Vec<String>, CatalogError> {
        let (key, _) = resolve(&self.root, &self.environment).map_err(|_| CatalogError::Request)?;
        if key.is_empty() {
            return Err(CatalogError::Request);
        }
        parse_models(&self.io.catalog(&key, true, cancel).await?)
    }
}
pub fn parse_models(bytes: &[u8]) -> Result<Vec<String>, CatalogError> {
    #[derive(serde::Deserialize)]
    struct Item {
        #[serde(default)]
        id: String,
    }
    #[derive(serde::Deserialize)]
    struct Catalog {
        data: Option<Vec<Item>>,
    }
    let parsed: Catalog = serde_json::from_slice(bytes).map_err(|_| CatalogError::Parse)?;
    let mut ids = vec![];
    let mut seen = BTreeSet::new();
    for item in parsed.data.ok_or(CatalogError::Parse)? {
        let id = item.id.trim();
        if id.is_empty()
            || id.len() > 256
            || id.split('/').any(|part| {
                part.is_empty()
                    || part == "."
                    || part == ".."
                    || !part
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
            })
            || !seen.insert(id.to_owned())
        {
            return Err(CatalogError::Parse);
        }
        ids.push(id.into());
    }
    Ok(ids)
}
#[cfg(test)]
mod tests;
