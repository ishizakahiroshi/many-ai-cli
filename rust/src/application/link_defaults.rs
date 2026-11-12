//! Request-driven link catalogs with source positive/negative TTL and stale fallback.
use crate::{
    config::RuntimePaths,
    hub::network,
    proto::{
        core::CoreFuture,
        wire::{Field, GoWire, Schema},
    },
};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io, sync::Arc, time::Duration};
pub trait LinkIo: Send + Sync {
    fn fetch<'a>(&'a self, source: &'a str) -> CoreFuture<'a, io::Result<Vec<u8>>>;
}
struct PublicDns;
impl reqwest::dns::Resolve for PublicDns {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let name = name.as_str().to_owned();
        Box::pin(async move {
            let addresses: Vec<_> = tokio::net::lookup_host((name.as_str(), 0)).await?.collect();
            if addresses.is_empty() || addresses.iter().any(|a| network::blocked_public(a.ip())) {
                return Err(io::Error::other("public DNS required").into());
            }
            Ok(Box::new(addresses.into_iter()) as reqwest::dns::Addrs)
        })
    }
}
pub struct NativeLinkIo {
    client: reqwest::Client,
    paths: RuntimePaths,
}
impl NativeLinkIo {
    pub fn new(paths: RuntimePaths) -> io::Result<Self> {
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(10))
            .dns_resolver(Arc::new(PublicDns))
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() >= 3 {
                    attempt.error("too many redirects")
                } else if network::validate_url(
                    attempt.url().as_str(),
                    network::Policy::ExternalHttps,
                )
                .is_err()
                {
                    attempt.error("redirect blocked")
                } else {
                    attempt.follow()
                }
            }))
            .build()
            .map_err(io::Error::other)?;
        Ok(Self { client, paths })
    }
}
impl LinkIo for NativeLinkIo {
    fn fetch<'a>(&'a self, source: &'a str) -> CoreFuture<'a, io::Result<Vec<u8>>> {
        Box::pin(async move {
            if self.paths.is_trial() {
                return Err(io::Error::other("external link request disabled in trial"));
            }
            let url = network::validate_url(source, network::Policy::ExternalHttps)
                .map_err(io::Error::other)?;
            let response = self
                .client
                .get(url)
                .send()
                .await
                .map_err(io::Error::other)?;
            if !response.status().is_success() {
                return Err(io::Error::other("link catalog status"));
            }
            let mut bytes = Vec::new();
            let mut stream = response.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(io::Error::other)?;
                let remain = (64 * 1024usize).saturating_sub(bytes.len());
                bytes.extend_from_slice(&chunk[..chunk.len().min(remain)]);
                if bytes.len() == 64 * 1024 {
                    break;
                }
            }
            Ok(bytes)
        })
    }
}
#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
struct Usage {
    claude: String,
    codex: String,
    #[serde(rename = "command-code")]
    command_code: String,
    copilot: String,
    #[serde(rename = "cursor-agent")]
    cursor_agent: String,
    grok: String,
    ollama: String,
    opencode: String,
}
impl GoWire for Usage {
    const GO_TYPE: &'static str = "UsageLinkDefaults";
    const SCHEMAS: &'static [Schema] = &[Schema {
        name: "UsageLinkDefaults",
        fields: &[
            Field {
                name: "claude",
                kind: "string",
            },
            Field {
                name: "codex",
                kind: "string",
            },
            Field {
                name: "command-code",
                kind: "string",
            },
            Field {
                name: "copilot",
                kind: "string",
            },
            Field {
                name: "cursor-agent",
                kind: "string",
            },
            Field {
                name: "grok",
                kind: "string",
            },
            Field {
                name: "ollama",
                kind: "string",
            },
            Field {
                name: "opencode",
                kind: "string",
            },
        ],
    }];
}
#[derive(Deserialize)]
#[serde(transparent)]
struct Install(Option<BTreeMap<String, String>>);
impl GoWire for Install {
    const GO_TYPE: &'static str = "map[string]string";
}
struct Cache {
    data: Option<serde_json::Value>,
    fetched: tokio::time::Instant,
    failed: Option<tokio::time::Instant>,
}
impl Default for Cache {
    fn default() -> Self {
        Self {
            data: None,
            fetched: tokio::time::Instant::now(),
            failed: None,
        }
    }
}
pub struct LinkDefaults {
    io: Arc<dyn LinkIo>,
    usage: tokio::sync::Mutex<Cache>,
    install: tokio::sync::Mutex<Cache>,
}
impl LinkDefaults {
    pub fn new(io: Arc<dyn LinkIo>) -> Arc<Self> {
        Arc::new(Self {
            io,
            usage: Default::default(),
            install: Default::default(),
        })
    }
    pub async fn get(&self, install: bool) -> serde_json::Value {
        let mut cache = if install {
            self.install.lock().await
        } else {
            self.usage.lock().await
        };
        if cache.data.is_some() && cache.fetched.elapsed() < Duration::from_secs(24 * 3600)
            || cache
                .failed
                .is_some_and(|at| at.elapsed() < Duration::from_secs(180))
        {
            return cache.data.clone().unwrap_or_else(|| fallback(install));
        }
        let source = if install {
            crate::config::DEFAULT_INSTALL_LINK_SOURCE
        } else {
            crate::config::DEFAULT_USAGE_LINK_SOURCE
        };
        let fetched = match self.io.fetch(source).await {
            Ok(bytes) if install => crate::proto::wire::decode::<Install>(&bytes).map(|map| {
                serde_json::json!(
                    map.0
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|(_, value)| value.starts_with("https://")
                            && url::Url::parse(value).is_ok())
                        .collect::<BTreeMap<_, _>>()
                )
            }),
            Ok(bytes) => crate::proto::wire::decode::<Usage>(&bytes)
                .and_then(serde_json::to_value)
                .map(https_only),
            Err(_) => {
                cache.failed = Some(tokio::time::Instant::now());
                return cache.data.clone().unwrap_or_else(|| fallback(install));
            }
        };
        match fetched {
            Ok(data) => {
                cache.data = Some(data.clone());
                cache.fetched = tokio::time::Instant::now();
                cache.failed = None;
                data
            }
            Err(_) => {
                cache.failed = Some(tokio::time::Instant::now());
                cache.data.clone().unwrap_or_else(|| fallback(install))
            }
        }
    }
}
/// The usage links the dashboard puts behind its buttons, as fetched from the
/// repository: only `https` URLs stay, like the install links above. A value
/// with another scheme (`javascript:`, one an app on the machine registered)
/// becomes empty, which the dashboard reads as "no default" and keeps its
/// built-in link (audit N36).
fn https_only(mut links: serde_json::Value) -> serde_json::Value {
    if let Some(map) = links.as_object_mut() {
        for value in map.values_mut() {
            let kept = value
                .as_str()
                .is_some_and(|url| url.starts_with("https://") && url::Url::parse(url).is_ok());
            if !kept && value.is_string() {
                *value = serde_json::Value::String(String::new());
            }
        }
    }
    links
}
fn fallback(install: bool) -> serde_json::Value {
    if install {
        serde_json::json!({})
    } else {
        serde_json::json!({"claude":"https://claude.ai/settings/usage","codex":"https://chatgpt.com/codex/cloud/settings/analytics#usage","command-code":"https://commandcode.ai/usage","copilot":"https://github.com/settings/billing","cursor-agent":"https://cursor.com/dashboard","grok":"https://grok.com/?_s=usage","ollama":"https://ollama.com/settings","opencode":"https://opencode.ai/go"})
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Fake {
        count: std::sync::atomic::AtomicUsize,
        values: std::sync::Mutex<std::collections::VecDeque<io::Result<Vec<u8>>>>,
    }
    impl LinkIo for Fake {
        fn fetch<'a>(&'a self, _: &'a str) -> CoreFuture<'a, io::Result<Vec<u8>>> {
            Box::pin(async move {
                self.count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                self.values.lock().unwrap().pop_front().unwrap()
            })
        }
    }
    #[tokio::test(start_paused = true)]
    async fn negative_cache_retains_success_then_retries_and_full_json_rejects_trailing() {
        let io = Arc::new(Fake {
            count: Default::default(),
            values: std::sync::Mutex::new(
                [
                    Ok(br#"{"codex":"https://example.com/source"}"#.to_vec()),
                    Err(io::Error::other("synthetic")),
                    Ok(br#"{"codex":"https://example.com/replacement"} trailing"#.to_vec()),
                    Ok(br#"{"CODEX":"https://example.com/next"}"#.to_vec()),
                ]
                .into(),
            ),
        });
        let cache = LinkDefaults::new(io.clone());
        assert_eq!(
            cache.get(false).await["codex"],
            "https://example.com/source"
        );
        tokio::time::advance(Duration::from_secs(86401)).await;
        assert_eq!(
            cache.get(false).await["codex"],
            "https://example.com/source"
        );
        assert_eq!(
            cache.get(false).await["codex"],
            "https://example.com/source"
        );
        assert_eq!(io.count.load(std::sync::atomic::Ordering::SeqCst), 2);
        tokio::time::advance(Duration::from_secs(181)).await;
        assert_eq!(
            cache.get(false).await["codex"],
            "https://example.com/source"
        );
        tokio::time::advance(Duration::from_secs(181)).await;
        assert_eq!(cache.get(false).await["codex"], "https://example.com/next");
    }
    #[tokio::test]
    async fn audit_n36_usage_links_keep_only_https_urls() {
        let io = Arc::new(Fake {
            count: Default::default(),
            values: std::sync::Mutex::new(
                [Ok(br#"{"claude":"javascript:alert(1)","codex":"ms-settings:privacy","copilot":"http://example.com/usage","grok":"https://example.com/usage","opencode":"https://"}"#.to_vec())]
                    .into(),
            ),
        });
        let got = LinkDefaults::new(io).get(false).await;
        assert_eq!(got["grok"], "https://example.com/usage");
        for dropped in ["claude", "codex", "copilot", "opencode"] {
            assert_eq!(got[dropped], "", "{dropped}");
        }
    }
    #[tokio::test]
    async fn install_catalog_filters_executable_links_and_keeps_case_sensitive_ids() {
        let io=Arc::new(Fake{count:Default::default(),values:std::sync::Mutex::new([Ok(br#"{"Claude":"https://example.com/install","claude":"javascript:alert(1)","codex":"http://example.com"}"#.to_vec())].into())});
        let got = LinkDefaults::new(io).get(true).await;
        assert_eq!(
            got,
            serde_json::json!({"Claude":"https://example.com/install"})
        );
    }
}
