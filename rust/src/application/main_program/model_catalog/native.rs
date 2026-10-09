//! Real managed native commands and address-pinned external catalog HTTP.
use super::*;
use crate::{
    hub::{
        cli_version::{
            CliVersionCommand, CliVersionExecutor,
            native::{NativeCliVersionExecutor, TrialVersionPolicy},
        },
        network::{self, Policy},
    },
    process::execpath::{self, Platform},
};
use std::path::PathBuf;
pub struct NativeCatalogIo {
    pub paths: RuntimePaths,
    pub environment: Vec<String>,
    pub cwd: PathBuf,
    pub locals: Arc<super::super::model_cache::LocalModelCache>,
    pub executor: Arc<dyn CliVersionExecutor>,
}
impl NativeCatalogIo {
    pub fn new(
        paths: RuntimePaths,
        environment: Vec<String>,
        cwd: PathBuf,
        locals: Arc<super::super::model_cache::LocalModelCache>,
    ) -> Self {
        let executor = Arc::new(NativeCliVersionExecutor::new(
            paths.clone(),
            TrialVersionPolicy::Disabled,
        ));
        Self {
            paths,
            environment,
            cwd,
            locals,
            executor,
        }
    }
}
impl CatalogIo for NativeCatalogIo {
    fn invalidate_local(&self) -> CoreFuture<'_, io::Result<()>> {
        Box::pin(async move { self.locals.invalidate().await })
    }
    fn defaults<'a>(
        &'a self,
        source: &'a str,
    ) -> CoreFuture<'a, io::Result<BTreeMap<String, Vec<Model>>>> {
        Box::pin(async move {
            parsers::defaults(&external(source, None, Duration::from_secs(10), 256 * 1024).await?)
        })
    }
    fn native<'a>(
        &'a self,
        provider: &'a str,
        force: bool,
    ) -> CoreFuture<'a, io::Result<Vec<Model>>> {
        Box::pin(async move {
            let mut args: Vec<String> = match provider {
                "cursor-agent" => vec!["--list-models".into()],
                "grok" => vec!["models".into()],
                "opencode" => vec!["models".into(), "opencode".into(), "--verbose".into()],
                _ => return Err(io::Error::other("unsupported native catalog")),
            };
            if provider == "opencode" && force {
                args.push("--refresh".into());
            }
            let executable = execpath::Resolver::new(
                Platform::native(),
                &self.environment,
                &self.cwd,
                &execpath::NativeFs,
            )
            .look_path(provider)
            .map_err(|_| io::Error::other("native catalog unavailable"))?;
            let output = self
                .executor
                .execute(CliVersionCommand {
                    executable,
                    args,
                    cwd: self.cwd.clone(),
                    environment: self.environment.clone(),
                    platform: Platform::native(),
                    timeout: Duration::from_secs(if provider == "opencode" { 30 } else { 15 }),
                    output_cap: 2 * 1024 * 1024,
                })
                .await
                .map_err(|_| io::Error::other("native catalog failed"))?;
            if output.exit_code != 0 || output.timed_out || output.start_failed {
                return Err(io::Error::other("native catalog failed"));
            }
            parsers::native(&output.output, provider)
        })
    }
    fn nvidia<'a>(&'a self, key: &'a str) -> CoreFuture<'a, io::Result<Vec<Model>>> {
        Box::pin(async move {
            use crate::application::nvidia_nim::NvidiaIo;
            let io = crate::application::nvidia_nim::NativeNvidiaIo {
                paths: self.paths.clone(),
            };
            parsers::nvidia(
                &io.catalog(key, true, &Cancellation::default())
                    .await
                    .map_err(|_| io::Error::other("NVIDIA catalog unavailable"))?,
            )
        })
    }
    fn nvidia_key(&self) -> io::Result<String> {
        crate::application::nvidia_nim::resolve_key(&self.paths, &self.environment)
    }
    fn local<'a>(
        &'a self,
        config: &'a Config,
        kind: super::super::model_cache::LocalCatalog,
        force: bool,
        cancel: &'a Cancellation,
    ) -> CoreFuture<'a, io::Result<ObservedModels>> {
        Box::pin(async move { self.locals.catalog(config, kind, force, cancel).await })
    }
}
async fn external(
    source: &str,
    key: Option<&str>,
    timeout: Duration,
    cap: usize,
) -> io::Result<Vec<u8>> {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut raw = source.to_owned();
    for redirects in 0..3 {
        let url = network::validate_url(&raw, Policy::ExternalHttps).map_err(io::Error::other)?;
        let host = url
            .host_str()
            .ok_or_else(|| io::Error::other("catalog host unavailable"))?
            .trim_matches(['[', ']'])
            .to_owned();
        let port = url
            .port_or_known_default()
            .ok_or_else(|| io::Error::other("catalog port unavailable"))?;
        let addresses: Vec<_> =
            tokio::time::timeout_at(deadline, tokio::net::lookup_host((host.as_str(), port)))
                .await
                .map_err(|_| io::Error::other("catalog timeout"))??
                .map(|address| address.ip())
                .collect();
        let target = network::validated_target(url, &addresses, Policy::ExternalHttps)
            .map_err(io::Error::other)?;
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(deadline.saturating_duration_since(tokio::time::Instant::now()))
            .resolve_to_addrs(&target.tls_host, &target.addresses)
            .build()
            .map_err(|_| io::Error::other("catalog client unavailable"))?;
        let mut request = client.get(target.url.clone());
        if let Some(key) = key {
            request = request
                .bearer_auth(key)
                .header(reqwest::header::ACCEPT, "application/json");
        }
        let mut response = request
            .send()
            .await
            .map_err(|_| io::Error::other("catalog request failed"))?;
        if response.status().is_redirection() {
            if key.is_some() || redirects == 2 {
                return Err(io::Error::other("catalog redirect blocked"));
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| io::Error::other("invalid catalog redirect"))?;
            raw = target
                .url
                .join(location)
                .map_err(|_| io::Error::other("invalid catalog redirect"))?
                .into();
            continue;
        }
        if !response.status().is_success() {
            return Err(io::Error::other("catalog HTTP failure"));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = tokio::time::timeout_at(deadline, response.chunk())
            .await
            .map_err(|_| io::Error::other("catalog timeout"))?
            .map_err(|_| io::Error::other("catalog response failed"))?
        {
            if append_prefix(&mut bytes, &chunk, cap) {
                break;
            }
        }
        return Ok(bytes);
    }
    Err(io::Error::other("catalog redirect limit"))
}
