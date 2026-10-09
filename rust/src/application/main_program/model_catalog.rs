//! Source catalog owner: actual fetch attempts, positive/negative TTLs and the
//! fixed Go group order. Dependencies are explicit; construction performs no IO.
use crate::{
    config::{Config, ConfigStore, RuntimePaths},
    process::Cancellation,
    proto::{
        core::CoreFuture,
        time::{Timestamp, format_go_rfc3339_layout},
    },
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, Mutex},
    time::Duration,
};
pub mod native;
mod parsers;
#[cfg(test)]
mod tests;
pub const NVIDIA_URL: &str = "https://integrate.api.nvidia.com/v1/models";
/// Go io.LimitReader returns a prefix and then EOF, without inspecting the tail.
/// Reaching the limit terminates reading; JSON validation applies to that prefix.
pub(super) fn append_prefix(bytes: &mut Vec<u8>, chunk: &[u8], cap: usize) -> bool {
    let remaining = cap.saturating_sub(bytes.len());
    bytes.extend_from_slice(&chunk[..remaining.min(chunk.len())]);
    bytes.len() == cap
}
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct Model {
    pub id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(skip)]
    pub remote_host: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct Group {
    pub label: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub provider: String,
    pub route: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub hosted: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub trial: bool,
    pub models: Vec<Model>,
}
#[derive(Debug, Serialize)]
pub struct CatalogResponse {
    pub groups: Vec<Group>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub cached_at: String,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub sources: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}
#[derive(Clone)]
pub struct ObservedModels {
    pub models: Vec<Model>,
    pub at: Timestamp,
    pub failed: bool,
}
pub trait CatalogIo: Send + Sync {
    fn invalidate_local(&self) -> CoreFuture<'_, io::Result<()>>;
    fn defaults<'a>(
        &'a self,
        source: &'a str,
    ) -> CoreFuture<'a, io::Result<BTreeMap<String, Vec<Model>>>>;
    fn native<'a>(
        &'a self,
        provider: &'a str,
        force: bool,
    ) -> CoreFuture<'a, io::Result<Vec<Model>>>;
    fn nvidia<'a>(&'a self, key: &'a str) -> CoreFuture<'a, io::Result<Vec<Model>>>;
    fn nvidia_key(&self) -> io::Result<String>;
    fn local<'a>(
        &'a self,
        config: &'a Config,
        kind: super::model_cache::LocalCatalog,
        force: bool,
        cancel: &'a Cancellation,
    ) -> CoreFuture<'a, io::Result<ObservedModels>>;
}
struct CacheEntry<T> {
    value: T,
    at: Timestamp,
    instant: tokio::time::Instant,
    failed: bool,
    key: Vec<u8>,
}
struct Cache<T> {
    state: Mutex<CacheState<T>>,
    mode: FlightMode,
    positive: Duration,
    negative: Duration,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum FlightMode {
    Shared,
    GenerationWait,
    GenerationDetach,
}
struct CacheState<T> {
    entry: Option<CacheEntry<T>>,
    flight: Option<Arc<Flight<T>>>,
    generation: u64,
}
struct Flight<T> {
    generation: u64,
    key: Vec<u8>,
    result: Mutex<FlightResult<T>>,
    done: tokio::sync::Notify,
}
enum FlightResult<T> {
    Pending,
    Complete(Observed<T>),
    Abandoned,
}
struct FlightGuard<'a, T> {
    cache: &'a Cache<T>,
    flight: Arc<Flight<T>>,
}
impl<T> Drop for FlightGuard<'_, T> {
    fn drop(&mut self) {
        let mut state = self.cache.state.lock().unwrap();
        if state
            .flight
            .as_ref()
            .is_some_and(|flight| Arc::ptr_eq(flight, &self.flight))
        {
            state.flight = None;
        }
        let mut result = self.flight.result.lock().unwrap();
        if matches!(*result, FlightResult::Pending) {
            *result = FlightResult::Abandoned;
        }
        self.flight.done.notify_waiters();
    }
}
impl<T: Clone + Default> Cache<T> {
    fn new(positive: u64, negative: u64, mode: FlightMode) -> Self {
        Self {
            state: Mutex::new(CacheState {
                entry: None,
                flight: None,
                generation: 0,
            }),
            mode,
            positive: Duration::from_secs(positive),
            negative: Duration::from_secs(negative),
        }
    }
    fn invalidate(&self) {
        let mut state = self.state.lock().unwrap();
        state.entry = None;
        if self.mode != FlightMode::Shared {
            state.generation = state.generation.wrapping_add(1);
        }
        if self.mode == FlightMode::GenerationDetach {
            state.flight = None;
        }
    }
    async fn get<F>(&self, key: Vec<u8>, force: bool, keep_stale: bool, fetch: F) -> Observed<T>
    where
        F: std::future::Future<Output = io::Result<T>>,
    {
        let mut fetch = Some(fetch);
        loop {
            let (flight, starter, must_retry) = {
                let mut state = self.state.lock().unwrap();
                // Source checks fresh entries before the in-flight slot. A
                // different request may refresh without blocking this hit.
                if !force
                    && let Some(entry) = state.entry.as_ref().filter(|entry| {
                        entry.key == key
                            && entry.instant.elapsed()
                                < if entry.failed {
                                    self.negative
                                } else {
                                    self.positive
                                }
                    })
                {
                    return Observed {
                        value: entry.value.clone(),
                        at: entry.at,
                        failed: entry.failed,
                    };
                }
                if let Some(flight) = &state.flight {
                    let retry = self.mode == FlightMode::GenerationWait
                        && (flight.generation != state.generation || flight.key != key);
                    (flight.clone(), false, retry)
                } else {
                    let flight = Arc::new(Flight {
                        generation: state.generation,
                        key: key.clone(),
                        result: Mutex::new(FlightResult::Pending),
                        done: tokio::sync::Notify::new(),
                    });
                    state.flight = Some(flight.clone());
                    (flight, true, false)
                }
            };
            if starter {
                // Drop cancels this owner without stranding joined waiters;
                // a subsequent caller may take over. No detached fetch exists.
                let guard = FlightGuard {
                    cache: self,
                    flight: flight.clone(),
                };
                let result = fetch
                    .take()
                    .expect("only the elected owner consumes its fetch")
                    .await;
                let failed = result.is_err();
                let at = Timestamp::now();
                let mut state = self.state.lock().unwrap();
                let value = result.unwrap_or_else(|_| {
                    if keep_stale {
                        state
                            .entry
                            .as_ref()
                            .map(|entry| entry.value.clone())
                            .unwrap_or_default()
                    } else {
                        T::default()
                    }
                });
                let observed = Observed {
                    value: value.clone(),
                    at,
                    failed,
                };
                if state.generation == flight.generation {
                    state.entry = Some(CacheEntry {
                        value,
                        at,
                        instant: tokio::time::Instant::now(),
                        failed,
                        key: key.clone(),
                    });
                }
                *flight.result.lock().unwrap() = FlightResult::Complete(observed.clone());
                if state
                    .flight
                    .as_ref()
                    .is_some_and(|active| Arc::ptr_eq(active, &flight))
                {
                    state.flight = None;
                }
                drop(state);
                drop(guard);
                return observed;
            }
            loop {
                let notified = flight.done.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                let result = {
                    let result = flight.result.lock().unwrap();
                    match &*result {
                        FlightResult::Pending => None,
                        FlightResult::Complete(observed) => Some(Some(observed.clone())),
                        FlightResult::Abandoned => Some(None),
                    }
                };
                match result {
                    Some(Some(result)) if !must_retry => return result,
                    Some(_) => break,
                    None => notified.await,
                }
            }
        }
    }
}
#[derive(Clone, Debug)]
struct Observed<T> {
    value: T,
    at: Timestamp,
    failed: bool,
}
pub struct ModelsCatalog {
    config: Arc<ConfigStore>,
    io: Arc<dyn CatalogIo>,
    defaults: Cache<BTreeMap<String, Vec<Model>>>,
    cursor: Cache<Vec<Model>>,
    grok: Cache<Vec<Model>>,
    opencode: Cache<Vec<Model>>,
    nvidia: Cache<Vec<Model>>,
}
impl ModelsCatalog {
    pub fn new(config: Arc<ConfigStore>, io: Arc<dyn CatalogIo>) -> Self {
        Self {
            config,
            io,
            defaults: Cache::new(86400, 180, FlightMode::Shared),
            cursor: Cache::new(600, 180, FlightMode::GenerationDetach),
            grok: Cache::new(600, 180, FlightMode::GenerationDetach),
            opencode: Cache::new(600, 180, FlightMode::Shared),
            nvidia: Cache::new(600, 60, FlightMode::GenerationWait),
        }
    }
    pub async fn response(
        &self,
        force: bool,
        cancel: &Cancellation,
    ) -> io::Result<CatalogResponse> {
        if cancel.is_cancelled() {
            return Err(io::Error::other("catalog cancelled"));
        }
        let cfg = self
            .config
            .snapshot()
            .map_err(|_| io::Error::other("catalog config unavailable"))?
            .config;
        if force {
            self.io.invalidate_local().await?;
            for cache in [&self.cursor, &self.grok, &self.nvidia] {
                cache.invalidate();
            }
            self.defaults.invalidate();
        }
        let source = if cfg.models_source.is_empty() {
            crate::config::DEFAULT_MODELS_SOURCE
        } else {
            &cfg.models_source
        };
        let defaults = self
            .defaults
            .get(Vec::new(), force, true, self.io.defaults(source))
            .await
            .value;
        let mut response = CatalogResponse {
            groups: Vec::new(),
            cached_at: String::new(),
            sources: defaults
                .keys()
                .map(|key| (key.clone(), source.into()))
                .collect(),
            warnings: Vec::new(),
        };
        let mut newest: Option<Timestamp> = None;
        let mut group =
            |label: &str, provider: &str, route: &str, models: Vec<Model>, hosted: bool| {
                response.groups.push(Group {
                    label: label.into(),
                    provider: provider.into(),
                    route: route.into(),
                    models,
                    hosted,
                    trial: hosted,
                })
            };
        group(
            "Anthropic",
            "claude",
            "anthropic",
            defaults.get("anthropic").cloned().unwrap_or_default(),
            false,
        );
        group(
            "OpenAI",
            "codex",
            "openai",
            defaults.get("openai").cloned().unwrap_or_default(),
            false,
        );
        if let Some(models) = defaults.get("copilot").filter(|models| !models.is_empty()) {
            group("GitHub Copilot", "copilot", "", models.clone(), false);
        }
        for (provider, label, command, cache) in [
            (
                "cursor-agent",
                "Cursor Agent",
                "cursor-agent --list-models",
                &self.cursor,
            ),
            ("grok", "Grok Build", "grok models", &self.grok),
        ] {
            let native = cache
                .get(Vec::new(), force, false, self.io.native(provider, force))
                .await;
            let (models, source_value) = if !native.failed && !native.value.is_empty() {
                newest = max(newest, native.at);
                (native.value, command.into())
            } else {
                (
                    defaults.get(provider).cloned().unwrap_or_default(),
                    format!("{command} (fallback: {source})"),
                )
            };
            response.sources.insert(provider.into(), source_value);
            if !models.is_empty() {
                group(label, provider, "", models, false);
            }
        }
        let opencode = self
            .opencode
            .get(Vec::new(), force, false, self.io.native("opencode", force))
            .await;
        response.sources.insert(
            "opencode".into(),
            "opencode models opencode --verbose".into(),
        );
        if !opencode.value.is_empty() {
            newest = max(newest, opencode.at);
            group("OpenCode", "opencode", "", opencode.value, false);
        }
        if cfg.nvidia_nim.enabled
            && let Ok(key) = self.io.nvidia_key()
            && !key.trim().is_empty()
        {
            use sha2::Digest;
            let fingerprint = sha2::Sha256::digest(key.as_bytes()).to_vec();
            let nim = self
                .nvidia
                .get(fingerprint, force, false, self.io.nvidia(&key))
                .await;
            response
                .sources
                .insert("nvidia_nim".into(), NVIDIA_URL.into());
            if nim.failed {
                response.warnings.push("nvidia_nim_unreachable".into());
            } else if !nim.value.is_empty() {
                newest = max(newest, nim.at);
                group("NVIDIA NIM", "opencode", "nvidia-nim", nim.value, true);
            }
        }
        let local = self
            .io
            .local(
                &cfg,
                super::model_cache::LocalCatalog::Ollama,
                force,
                cancel,
            )
            .await?;
        let (cloud, mut locals): (Vec<_>, Vec<_>) = local
            .models
            .into_iter()
            .partition(|model| !model.remote_host.is_empty());
        response.sources.insert(
            "ollama_local".into(),
            format!(
                "{}/api/tags",
                crate::config::effective_ollama_base_url(&cfg.ollama.base_url)
            ),
        );
        response.sources.insert(
            "lm_studio".into(),
            format!(
                "{}/v1/models",
                crate::config::effective_lm_studio_base_url(&cfg.lm_studio.base_url)
            ),
        );
        response.sources.insert(
            "local_models_config".into(),
            "~/.many-ai-cli/config.yaml#local_models".into(),
        );
        if !cloud.is_empty() {
            newest = max(newest, local.at);
            group("Ollama Cloud", "", "ollama", cloud, false);
        }
        merge_local(&mut locals, &cfg.local_models);
        if local.failed {
            response.warnings.push("ollama_daemon_unreachable".into());
        }
        if !locals.is_empty() {
            newest = max(newest, local.at);
            group("Ollama Local", "", "ollama", locals, false);
        }
        let lm = self
            .io
            .local(
                &cfg,
                super::model_cache::LocalCatalog::LmStudio,
                force,
                cancel,
            )
            .await?;
        if lm.failed {
            response.warnings.push("lm_studio_unreachable".into());
        } else if !lm.models.is_empty() {
            newest = max(newest, lm.at);
            group("LM Studio", "", "lm-studio", lm.models, false);
        }
        if let Some(at) = newest {
            response.cached_at =
                format_go_rfc3339_layout(at, 0, false).map_err(io::Error::other)?;
        }
        Ok(response)
    }
    pub async fn handle(
        &self,
        request: &crate::hub::http::Request,
        cancel: &Cancellation,
    ) -> crate::hub::http::Response {
        use crate::hub::http::*;
        if let Err(response) = require_method(request, &["GET", "POST"]) {
            return response;
        }
        match self.response(request.method == "POST", cancel).await {
            Ok(response) => Response::json(200, &response),
            Err(_) => Response::error(503, "models_unavailable", "model catalog owner unavailable"),
        }
    }
}
fn max(current: Option<Timestamp>, at: Timestamp) -> Option<Timestamp> {
    Some(current.map_or(at, |current| current.max(at)))
}
fn merge_local(models: &mut Vec<Model>, config: &[crate::config::LocalModel]) {
    for item in config {
        let id = item.id.trim();
        if id.is_empty() {
            continue;
        }
        let label = if item.label.trim().is_empty() {
            id
        } else {
            item.label.trim()
        };
        if let Some(model) = models.iter_mut().find(|model| model.id == id) {
            model.label = label.into();
        } else {
            models.push(Model {
                id: id.into(),
                label: label.into(),
                ..Default::default()
            });
        }
    }
}
