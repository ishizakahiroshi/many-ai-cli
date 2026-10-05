//! Actual local-daemon cache owner. Go initializes entries absent and spawn
//! reads the last observed IDs without fetching. Refresh is request-driven,
//! with a 60 second TTL; there is no source background polling cadence.
use crate::{
    application::spawn_policy::LocalModelSnapshot,
    config::{self, Config},
    hub::network,
    process::Cancellation,
};
use std::{
    collections::BTreeSet,
    io,
    sync::Mutex,
    time::{Duration, Instant},
};
#[derive(Clone, Copy)]
pub enum LocalCatalog {
    Ollama,
    LmStudio,
}
struct Entry {
    url: String,
    allow_private: bool,
    fetched: Instant,
    models: Vec<super::model_catalog::Model>,
    at: crate::proto::time::Timestamp,
    failed: bool,
}
#[derive(Default)]
struct State {
    ollama: Option<Entry>,
    lm_studio: Option<Entry>,
}
#[derive(Default)]
pub struct LocalModelCache {
    state: Mutex<State>,
    refresh: tokio::sync::Mutex<()>,
}
impl LocalModelCache {
    pub async fn invalidate(&self) -> io::Result<()> {
        *self
            .state
            .lock()
            .map_err(|_| io::Error::other("model cache unavailable"))? = State::default();
        Ok(())
    }
    pub fn snapshot(&self) -> io::Result<LocalModelSnapshot> {
        let state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("model cache unavailable"))?;
        Ok(LocalModelSnapshot {
            ollama: state
                .ollama
                .as_ref()
                .map(|entry| entry.models.iter().map(|model| model.id.clone()).collect())
                .unwrap_or_default(),
            lm_studio: state
                .lm_studio
                .as_ref()
                .map(|entry| entry.models.iter().map(|model| model.id.clone()).collect())
                .unwrap_or_default(),
        })
    }
    pub async fn refresh(
        &self,
        config: &Config,
        kind: LocalCatalog,
        force: bool,
        cancel: &Cancellation,
    ) -> io::Result<BTreeSet<String>> {
        if force {
            self.invalidate().await?;
        }
        let result = self.catalog(config, kind, force, cancel).await?;
        if result.failed {
            Err(io::Error::other("local model fetch failed"))
        } else {
            Ok(result.models.into_iter().map(|model| model.id).collect())
        }
    }
    pub async fn catalog(
        &self,
        config: &Config,
        kind: LocalCatalog,
        force: bool,
        cancel: &Cancellation,
    ) -> io::Result<super::model_catalog::ObservedModels> {
        let (url, allow_private) = match kind {
            LocalCatalog::Ollama => (
                format!(
                    "{}/api/tags",
                    config::effective_ollama_base_url(&config.ollama.base_url)
                ),
                config.ollama.allow_private_hosts,
            ),
            LocalCatalog::LmStudio => (
                format!(
                    "{}/v1/models",
                    config::effective_lm_studio_base_url(&config.lm_studio.base_url)
                ),
                config.lm_studio.allow_private_hosts,
            ),
        };
        let cached = || -> io::Result<Option<super::model_catalog::ObservedModels>> {
            let state = self
                .state
                .lock()
                .map_err(|_| io::Error::other("model cache unavailable"))?;
            let entry = match kind {
                LocalCatalog::Ollama => &state.ollama,
                LocalCatalog::LmStudio => &state.lm_studio,
            };
            if let Some(entry) = entry.as_ref().filter(|entry| {
                (!force || matches!(kind, LocalCatalog::Ollama))
                    && entry.url == url
                    && entry.allow_private == allow_private
                    && entry.fetched.elapsed() < Duration::from_secs(60)
            }) {
                return Ok(Some(super::model_catalog::ObservedModels {
                    models: entry.models.clone(),
                    at: entry.at,
                    failed: entry.failed,
                }));
            }
            Ok(None)
        };
        if let Some(observed) = cached()? {
            return Ok(observed);
        }
        // Go Ollama shares one fetch. LM Studio has no in-flight slot and may
        // fetch concurrently; neither backend holds the other's gate.
        let _refresh = match kind {
            LocalCatalog::Ollama => Some(
                tokio::select! {_=cancel.cancelled()=>return Err(io::Error::other("model refresh cancelled")),gate=self.refresh.lock()=>gate},
            ),
            LocalCatalog::LmStudio => None,
        };
        if matches!(kind, LocalCatalog::Ollama)
            && let Some(observed) = cached()?
        {
            return Ok(observed);
        }
        let result = tokio::select! { _ = cancel.cancelled() => return Err(io::Error::other("model refresh cancelled")), result = fetch(&url, allow_private, kind) => result };
        let entry = Entry {
            url,
            allow_private,
            fetched: Instant::now(),
            models: result.as_ref().cloned().unwrap_or_default(),
            at: crate::proto::time::Timestamp::now(),
            failed: result.is_err(),
        };
        let observed = super::model_catalog::ObservedModels {
            models: entry.models.clone(),
            at: entry.at,
            failed: entry.failed,
        };
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("model cache unavailable"))?;
        match kind {
            LocalCatalog::Ollama => state.ollama = Some(entry),
            LocalCatalog::LmStudio => state.lm_studio = Some(entry),
        };
        Ok(observed)
    }
}
async fn fetch(
    raw: &str,
    allow_private: bool,
    kind: LocalCatalog,
) -> io::Result<Vec<super::model_catalog::Model>> {
    let started = Instant::now();
    let mut url = url::Url::parse(raw).map_err(|_| io::Error::other("invalid local model URL"))?;
    let original_loopback = url.host_str().is_some_and(loopback_host);
    for redirects in 0..3 {
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(io::Error::other("invalid local model URL"));
        }
        let host = url
            .host_str()
            .ok_or_else(|| io::Error::other("missing model host"))?
            .trim_matches(['[', ']'])
            .to_owned();
        if original_loopback && (url.scheme() != "http" || !loopback_host(&host)) {
            return Err(io::Error::other("non-loopback model redirect blocked"));
        }
        let port = url
            .port_or_known_default()
            .ok_or_else(|| io::Error::other("missing model port"))?;
        let remaining = Duration::from_secs(3)
            .checked_sub(started.elapsed())
            .ok_or_else(|| io::Error::other("model fetch timed out"))?;
        let addresses: Vec<_> =
            tokio::time::timeout(remaining, tokio::net::lookup_host((host.as_str(), port)))
                .await
                .map_err(|_| io::Error::other("model DNS timed out"))??
                .collect();
        if addresses.is_empty() {
            return Err(io::Error::other("model host has no addresses"));
        }
        let loopback = loopback_host(&host);
        if addresses.iter().any(|address| {
            (loopback && !loopback_ip(address.ip()))
                || (!loopback && !allow_private && network::blocked_public(address.ip()))
        }) {
            return Err(io::Error::other("model target address blocked"));
        }
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(remaining)
            .resolve_to_addrs(&host, &addresses)
            .build()
            .map_err(|_| io::Error::other("model HTTP client unavailable"))?;
        let mut response = client
            .get(url.clone())
            .send()
            .await
            .map_err(|_| io::Error::other("local model fetch failed"))?;
        if response.status().is_redirection() {
            if redirects == 2 {
                return Err(io::Error::other("too many model redirects"));
            }
            let target = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| io::Error::other("invalid model redirect"))?;
            url = url
                .join(target)
                .map_err(|_| io::Error::other("invalid model redirect"))?;
            continue;
        }
        if !response.status().is_success() {
            return Err(io::Error::other("local model HTTP failure"));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| io::Error::other("model response read failed"))?
        {
            if super::model_catalog::append_prefix(&mut bytes, &chunk, 2 * 1024 * 1024) {
                break;
            }
        }
        return parse_models(&bytes, kind);
    }
    unreachable!()
}
fn loopback_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V6(ip) => ip
            .to_ipv4_mapped()
            .map_or_else(|| ip.is_loopback(), |ip| ip.is_loopback()),
        ip => ip.is_loopback(),
    }
}
fn loopback_host(host: &str) -> bool {
    host.trim_end_matches('.').eq_ignore_ascii_case("localhost")
        || host.trim_matches(['[', ']']).parse().is_ok_and(loopback_ip)
}
#[cfg(test)]
fn parse(bytes: &[u8], kind: LocalCatalog) -> io::Result<BTreeSet<String>> {
    Ok(parse_models(bytes, kind)?
        .into_iter()
        .map(|model| model.id)
        .collect())
}
fn parse_models(bytes: &[u8], kind: LocalCatalog) -> io::Result<Vec<super::model_catalog::Model>> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| io::Error::other("invalid model response"))?;
    if !value.is_null() && !value.is_object() {
        return Err(io::Error::other("invalid model response object"));
    }
    let (items, key) = match kind {
        LocalCatalog::Ollama => (&value["models"], "name"),
        LocalCatalog::LmStudio => (&value["data"], "id"),
    };
    if !items.is_null() && !items.is_array() {
        return Err(io::Error::other("invalid model collection"));
    }
    for item in items.as_array().into_iter().flatten() {
        if !item.is_null() && !item.is_object() {
            return Err(io::Error::other("invalid model entry"));
        }
        for field in match kind {
            LocalCatalog::Ollama => &["name", "model", "remote_host"][..],
            LocalCatalog::LmStudio => &["id", "object"][..],
        } {
            if !item[*field].is_null() && !item[*field].is_string() {
                return Err(io::Error::other("invalid model identifier"));
            }
        }
    }
    let mut seen = BTreeSet::new();
    Ok(items
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let mut id = item[key].as_str().unwrap_or("").trim();
            if id.is_empty() && matches!(kind, LocalCatalog::Ollama) {
                id = item["model"].as_str().unwrap_or("").trim();
            }
            (!id.is_empty() && seen.insert(id.to_owned())).then(|| super::model_catalog::Model {
                id: id.into(),
                label: id.into(),
                remote_host: if matches!(kind, LocalCatalog::Ollama) {
                    item["remote_host"].as_str().unwrap_or("").trim().into()
                } else {
                    String::new()
                },
            })
        })
        .collect())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_body_limit_accepts_complete_json_prefix_and_rejects_partial_id() {
        let cap = 2 * 1024 * 1024;
        for (body, kind) in [
            (
                br#"{"models":[{"name":"synthetic"}]}"#.as_slice(),
                LocalCatalog::Ollama,
            ),
            (
                br#"{"data":[{"id":"synthetic"}]}"#.as_slice(),
                LocalCatalog::LmStudio,
            ),
        ] {
            let mut body = body.to_vec();
            body.resize(cap, b' ');
            body.extend_from_slice(b"ignored tail");
            let mut prefix = Vec::new();
            assert!(super::super::model_catalog::append_prefix(
                &mut prefix,
                &body,
                cap
            ));
            assert_eq!(parse_models(&prefix, kind).unwrap()[0].id, "synthetic");
        }
        let mut incomplete = br#"{"data":[{"id":""#.to_vec();
        incomplete.resize(cap + 1, b'x');
        incomplete.extend_from_slice(br#""}]}"#);
        let mut prefix = Vec::new();
        assert!(super::super::model_catalog::append_prefix(
            &mut prefix,
            &incomplete,
            cap
        ));
        assert!(parse_models(&prefix, LocalCatalog::LmStudio).is_err());
    }
    #[test]
    fn startup_is_unobserved_and_parser_retains_cloud_alias_and_fallback_name() {
        let cache = LocalModelCache::default();
        assert!(cache.state.lock().unwrap().ollama.is_none());
        assert!(cache.snapshot().unwrap().ollama.is_empty());
        let ids = parse(br#"{"models":[{"name":" cloud-alias ","remote_host":"https://example.invalid"},{"name":"","model":"local"},{"name":"local"}]}"#, LocalCatalog::Ollama).unwrap();
        assert_eq!(ids, BTreeSet::from(["cloud-alias".into(), "local".into()]));
        assert!(parse(br#"{"data":{}}"#, LocalCatalog::LmStudio).is_err());
        assert!(parse(br#"{"data":[{"id":7}]}"#, LocalCatalog::LmStudio).is_err());
        assert!(parse(b"[]", LocalCatalog::LmStudio).is_err());
    }
    #[tokio::test]
    async fn actual_owned_loopback_refresh_populates_cache_and_ttl_skips_second_request() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let count = socket.read(&mut request).await.unwrap();
            assert!(String::from_utf8_lossy(&request[..count]).starts_with("GET /api/tags "));
            let body = r#"{"models":[{"name":"synthetic-model","remote_host":"https://example.invalid"}]}"#;
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });
        let mut config = Config::default();
        config.ollama.base_url = format!("http://{address}");
        let cache = LocalModelCache::default();
        let cancel = Cancellation::default();
        let observed = cache
            .refresh(&config, LocalCatalog::Ollama, false, &cancel)
            .await
            .unwrap();
        server.await.unwrap();
        assert!(observed.contains("synthetic-model"));
        assert_eq!(cache.snapshot().unwrap().ollama, observed);
        assert_eq!(
            cache
                .refresh(&config, LocalCatalog::Ollama, false, &cancel)
                .await
                .unwrap(),
            observed
        );
        assert!(
            cache
                .refresh(&config, LocalCatalog::Ollama, true, &cancel)
                .await
                .is_err()
        );
        assert!(cache.snapshot().unwrap().ollama.is_empty());
    }
}
