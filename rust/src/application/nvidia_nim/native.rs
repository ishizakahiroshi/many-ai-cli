use super::*;
use reqwest::dns::{Name, Resolve, Resolving};
use std::{
    net::SocketAddr,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
const BODY_CAP: usize = 2 * 1024 * 1024;
struct NvidiaResolver;
impl Resolve for NvidiaResolver {
    fn resolve(&self, name: Name) -> Resolving {
        Box::pin(async move {
            if name.as_str() != "integrate.api.nvidia.com" {
                return Err(Box::new(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "unexpected NVIDIA authority",
                ))
                    as Box<dyn std::error::Error + Send + Sync>);
            }
            let addresses: Vec<SocketAddr> = tokio::net::lookup_host((name.as_str(), 443))
                .await?
                .collect();
            let addresses = public_addresses(&addresses)?;
            Ok(Box::new(addresses.into_iter()) as Box<dyn Iterator<Item = SocketAddr> + Send>)
        })
    }
}
fn public_addresses(addresses: &[SocketAddr]) -> io::Result<Vec<SocketAddr>> {
    use crate::hub::network::{self, Policy};
    let url =
        network::validate_url(CATALOG_URL, Policy::ExternalHttps).map_err(io::Error::other)?;
    let ips = addresses.iter().map(SocketAddr::ip).collect::<Vec<_>>();
    network::validated_target(url, &ips, Policy::ExternalHttps)
        .map(|target| target.addresses)
        .map_err(io::Error::other)
}
trait CatalogBody: Send {
    fn next(&mut self) -> CoreFuture<'_, Result<Option<Vec<u8>>, CatalogError>>;
}
impl CatalogBody for reqwest::Response {
    fn next(&mut self) -> CoreFuture<'_, Result<Option<Vec<u8>>, CatalogError>> {
        Box::pin(async move {
            self.chunk()
                .await
                .map(|chunk| chunk.map(|bytes| bytes.to_vec()))
                .map_err(|_| CatalogError::Request)
        })
    }
}
async fn read_body(body: &mut dyn CatalogBody, parse: bool) -> Result<Vec<u8>, CatalogError> {
    let mut bytes = vec![];
    let mut received = 0;
    while received < BODY_CAP {
        let chunk = match body.next().await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break,
            Err(_) if !parse => break,
            Err(error) => return Err(error),
        };
        let count = chunk.len().min(BODY_CAP - received);
        received += count;
        if parse {
            bytes.extend_from_slice(&chunk[..count]);
        }
    }
    Ok(bytes)
}
pub struct NativeNvidiaIo {
    pub paths: RuntimePaths,
}
fn redirect_target(current: &url::Url, location: &str) -> Result<url::Url, CatalogError> {
    // URL parsing normalizes an explicit default :443 away; inspect raw authority first.
    if let Some((_, rest)) = location.split_once("://") {
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        if authority != "integrate.api.nvidia.com" {
            return Err(CatalogError::Request);
        }
    } else if let Some(rest) = location.strip_prefix("//")
        && rest.split(['/', '?', '#']).next().unwrap_or("") != "integrate.api.nvidia.com"
    {
        return Err(CatalogError::Request);
    }
    let next = current.join(location).map_err(|_| CatalogError::Request)?;
    if next.scheme() != "https"
        || next.host_str() != Some("integrate.api.nvidia.com")
        || next.port().is_some()
        || !next.username().is_empty()
        || next.password().is_some()
    {
        return Err(CatalogError::Request);
    }
    Ok(next)
}
impl NvidiaIo for NativeNvidiaIo {
    fn catalog<'a>(
        &'a self,
        key: &'a str,
        parse_body: bool,
        cancel: &'a Cancellation,
    ) -> CoreFuture<'a, Result<Vec<u8>, CatalogError>> {
        Box::pin(async move {
            if self.paths.is_trial() {
                return Err(CatalogError::Request);
            }
            if key.trim().is_empty() || validate(key).is_err() {
                return Err(CatalogError::Request);
            }
            let headers_success = AtomicBool::new(false);
            let operation = async {
                let client = reqwest::Client::builder()
                    .no_proxy()
                    .redirect(reqwest::redirect::Policy::none())
                    .dns_resolver(Arc::new(NvidiaResolver))
                    .timeout(Duration::from_secs(10))
                    .build()
                    .map_err(|_| CatalogError::Request)?;
                let mut url = url::Url::parse(CATALOG_URL).map_err(|_| CatalogError::Request)?;
                for hop in 0..3 {
                    let mut response = client
                        .get(url.clone())
                        .bearer_auth(key.trim())
                        .header(reqwest::header::ACCEPT, "application/json")
                        .send()
                        .await
                        .map_err(|e| {
                            if parse_body {
                                CatalogError::Request
                            } else if e.is_timeout() {
                                CatalogError::Timeout
                            } else {
                                CatalogError::Request
                            }
                        })?;
                    if matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308)
                        && let Some(location) = response.headers().get(reqwest::header::LOCATION)
                    {
                        if hop == 2 {
                            return Err(CatalogError::Request);
                        }
                        let location = location.to_str().map_err(|_| CatalogError::Request)?;
                        url = redirect_target(&url, location)?;
                        continue;
                    }
                    if !response.status().is_success() {
                        return Err(CatalogError::Http(response.status().as_u16()));
                    }
                    headers_success.store(true, Ordering::Relaxed);
                    return read_body(&mut response, parse_body).await;
                }
                Err(CatalogError::Request)
            };
            tokio::select! {_=cancel.cancelled()=>Err(CatalogError::Cancelled),result=tokio::time::timeout(Duration::from_secs(10),operation)=>match result {Ok(reply)=>reply,Err(_)if parse_body=>Err(CatalogError::Request),Err(_)if headers_success.load(Ordering::Relaxed)=>Ok(vec![]),Err(_)=>Err(CatalogError::Timeout)}}
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn redirect_budget_target_validator_rejects_default_ports_credentials_external_and_cleartext() {
        let current = url::Url::parse(CATALOG_URL).unwrap();

        let mut credentials =
            url::Url::parse("https://integrate.api.nvidia.com/v1/models").unwrap();
        credentials.set_username("user").unwrap();
        assert!(redirect_target(&current, credentials.as_str()).is_err());
        for input in [
            "http://integrate.api.nvidia.com/v1/models",
            "https://integrate.api.nvidia.com:443/v1/models",
            "https://attacker.invalid/models",
            "//integrate.api.nvidia.com:443/models",
        ] {
            assert!(redirect_target(&current, input).is_err(), "{input}");
        }
        assert_eq!(
            redirect_target(&current, "/next").unwrap().as_str(),
            "https://integrate.api.nvidia.com/next"
        );
    }
    struct Chunks(std::collections::VecDeque<Result<Vec<u8>, CatalogError>>);
    impl CatalogBody for Chunks {
        fn next(&mut self) -> CoreFuture<'_, Result<Option<Vec<u8>>, CatalogError>> {
            let reply = self.0.pop_front().transpose();
            Box::pin(async move { reply })
        }
    }
    #[tokio::test]
    async fn catalog_body_prefix_cap_accepts_valid_json_with_overflow_padding_but_rejects_cut_json()
    {
        let mut bytes = br#"{"data":[{"id":"vendor/model"}]}"#.to_vec();
        bytes.resize(BODY_CAP + 100, b' ');
        let mut input = Chunks([Ok(bytes)].into());
        let prefix = read_body(&mut input, true).await.unwrap();
        assert_eq!(prefix.len(), BODY_CAP);
        assert_eq!(parse_models(&prefix).unwrap(), vec!["vendor/model"]);
        let mut bytes = br#"{"data":[{"id":"vendor/"#.to_vec();
        bytes.resize(BODY_CAP + 100, b'x');
        let mut input = Chunks([Ok(bytes)].into());
        assert_eq!(
            parse_models(&read_body(&mut input, true).await.unwrap()),
            Err(CatalogError::Parse)
        );
    }
    #[tokio::test]
    async fn successful_header_check_ignores_partial_body_error_and_fetch_reports_it() {
        let chunks = [Ok(b"partial".to_vec()), Err(CatalogError::Request)];
        assert_eq!(
            read_body(&mut Chunks(chunks.clone().into()), false).await,
            Ok(vec![])
        );
        assert_eq!(
            read_body(&mut Chunks(chunks.into()), true).await,
            Err(CatalogError::Request)
        );
    }
    #[test]
    fn fixed_authority_dns_rejects_every_private_or_mixed_answer_before_dial() {
        let public = "8.8.8.8:443".parse().unwrap();
        assert_eq!(public_addresses(&[public]).unwrap(), vec![public]);
        assert!(public_addresses(&[]).is_err());
        let private = std::net::SocketAddr::new(
            std::net::IpAddr::V4(std::net::Ipv4Addr::new(10, 1, 2, 3)),
            443,
        );
        assert!(public_addresses(&[private]).is_err());
        assert!(public_addresses(&[public, private]).is_err());
        for raw in [
            "127.0.0.1:443",
            "169.254.1.2:443",
            "[::1]:443",
            "[::ffff:127.0.0.1]:443",
            "[fd00::1]:443",
        ] {
            let private = raw.parse().unwrap();
            assert!(public_addresses(&[private]).is_err());
            assert!(public_addresses(&[public, private]).is_err());
        }
    }
}
