//! Public-only HTTPS transport: all DNS answers checked once and pinned per dial.
use super::{PushHttpClient, crypto::EncodedPush};
use crate::{
    config::RuntimePaths,
    hub::network::{self, Policy},
    proto::core::{CoreFuture, TaskCancellation},
};
use std::{io, time::Duration};
pub struct NativePushHttpClient {
    trial: bool,
}
impl NativePushHttpClient {
    pub fn new(paths: &RuntimePaths) -> Self {
        Self {
            trial: paths.is_trial(),
        }
    }
}
fn error() -> io::Error {
    io::Error::other("web push HTTPS transport failed")
}
impl PushHttpClient for NativePushHttpClient {
    fn send<'a>(
        &'a self,
        endpoint: &'a str,
        request: EncodedPush,
        cancel: &'a TaskCancellation,
    ) -> CoreFuture<'a, io::Result<u16>> {
        Box::pin(async move {
            if self.trial {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "trial refuses native Web Push",
                ));
            }
            let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
            let mut target_url =
                network::validate_url(endpoint, Policy::ExternalHttps).map_err(|_| error())?;
            let mut method = reqwest::Method::POST;
            let mut body = Some(request.body);
            let original = target_url.host_str().unwrap_or("").to_owned();
            let mut authorization = true;
            for redirects in 0..3 {
                if cancel.token().is_cancelled() {
                    return Err(error());
                }
                let host = target_url
                    .host_str()
                    .ok_or_else(error)?
                    .trim_matches(['[', ']'])
                    .to_owned();
                let port = target_url.port_or_known_default().ok_or_else(error)?;
                let resolved = tokio::select! {result=tokio::time::timeout_at(deadline,tokio::net::lookup_host((host.as_str(),port)))=>result.map_err(|_|error())?.map_err(|_|error())?,_=cancel.token().cancelled()=>return Err(error())};
                let ips = resolved.map(|addr| addr.ip()).collect::<Vec<_>>();
                let validated =
                    network::validated_target(target_url.clone(), &ips, Policy::ExternalHttps)
                        .map_err(|_| error())?;
                let client = reqwest::Client::builder()
                    .no_proxy()
                    .redirect(reqwest::redirect::Policy::none())
                    .timeout(deadline.saturating_duration_since(tokio::time::Instant::now()))
                    .resolve_to_addrs(&validated.tls_host, &validated.addresses)
                    .build()
                    .map_err(|_| error())?;
                let mut send = client
                    .request(method.clone(), validated.url.clone())
                    .header("Content-Encoding", "aes128gcm")
                    .header("Content-Type", "application/octet-stream")
                    .header("TTL", "300")
                    .header("Topic", &request.topic);
                if authorization {
                    send = send.header("Authorization", &request.authorization);
                }
                if let Some(body) = &body {
                    send = send.body(body.clone());
                }
                let response = tokio::select! {response=send.send()=>response.map_err(|_|error())?,_=cancel.token().cancelled()=>return Err(error())};
                let code = response.status().as_u16();
                if matches!(code, 301 | 302 | 303 | 307 | 308) {
                    let Some(location) = response.headers().get(reqwest::header::LOCATION) else {
                        return Ok(code);
                    };
                    if redirects == 2 {
                        return Err(error());
                    }
                    let next = target_url
                        .join(location.to_str().map_err(|_| error())?)
                        .map_err(|_| error())?;
                    target_url = network::validate_url(next.as_str(), Policy::ExternalHttps)
                        .map_err(|_| error())?;
                    let next_host = target_url.host_str().unwrap_or("");
                    authorization =
                        next_host == original || next_host.ends_with(&format!(".{original}"));
                    if matches!(code, 301..=303) {
                        method = reqwest::Method::GET;
                        body = None;
                    }
                    continue;
                }
                return Ok(code);
            }
            Err(error())
        })
    }
}
