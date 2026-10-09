//! Distinct outbound trust policies. The returned addresses are the addresses
//! which the client must actually dial; the hostname is never resolved twice.
use std::net::{IpAddr, SocketAddr};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    ExternalHttps,
    ManagedLoopbackHttp,
    NotificationHttp,
}
#[derive(Clone, Debug)]
pub struct ValidatedTarget {
    pub url: url::Url,
    pub addresses: Vec<SocketAddr>,
    pub tls_host: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkError(pub &'static str);
impl std::fmt::Display for NetworkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for NetworkError {}
fn unmapped(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v) => v.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(ip),
        _ => ip,
    }
}
pub fn blocked_public(ip: IpAddr) -> bool {
    match unmapped(ip) {
        IpAddr::V4(v) => {
            v.is_unspecified()
                || v.is_loopback()
                || v.is_private()
                || v.is_link_local()
                || v.is_multicast()
        }
        IpAddr::V6(v) => {
            v.is_unspecified()
                || v.is_loopback()
                || v.is_unique_local()
                || v.is_unicast_link_local()
                || v.is_multicast()
        }
    }
}
pub fn validate_url(raw: &str, policy: Policy) -> Result<url::Url, NetworkError> {
    let url = url::Url::parse(raw).map_err(|_| NetworkError("invalid request URL"))?;
    let scheme_ok = match policy {
        Policy::ExternalHttps => url.scheme() == "https",
        Policy::ManagedLoopbackHttp => url.scheme() == "http",
        Policy::NotificationHttp => matches!(url.scheme(), "http" | "https"),
    };
    if !scheme_ok {
        return Err(NetworkError(match policy {
            Policy::ExternalHttps => "non-https request blocked",
            Policy::ManagedLoopbackHttp => "non-http loopback request blocked",
            Policy::NotificationHttp => "unsupported notification URL scheme",
        }));
    }
    let authority = raw
        .split_once("://")
        .map(|(_, rest)| rest.split(['/', '?', '#']).next().unwrap_or(""))
        .filter(|s| !s.is_empty())
        .ok_or(NetworkError("missing request host"))?;
    let host = url.host_str().ok_or(NetworkError("missing request host"))?;
    if policy != Policy::NotificationHttp
        && (!url.username().is_empty() || url.password().is_some() || authority.contains('@'))
    {
        return Err(NetworkError("request credentials blocked"));
    }
    if policy == Policy::ExternalHttps
        && host
            .trim_matches(['[', ']'])
            .parse::<IpAddr>()
            .is_ok_and(blocked_public)
    {
        return Err(NetworkError("private network host blocked"));
    }
    Ok(url)
}
pub fn validated_target(
    url: url::Url,
    addresses: &[IpAddr],
    policy: Policy,
) -> Result<ValidatedTarget, NetworkError> {
    // All public-transport DNS answers are checked before any dial, matching the
    // baseline defense against a mixed public/private rebinding response.
    if addresses.is_empty() {
        return Err(NetworkError("no ip resolved"));
    }
    for ip in addresses {
        match policy {
            Policy::ExternalHttps if blocked_public(*ip) => {
                return Err(NetworkError("private network host blocked"));
            }
            Policy::ManagedLoopbackHttp if !unmapped(*ip).is_loopback() => {
                return Err(NetworkError("non-loopback host blocked"));
            }
            _ => {}
        }
    }
    let port = url
        .port_or_known_default()
        .ok_or(NetworkError("missing request port"))?;
    let host = url
        .host_str()
        .ok_or(NetworkError("missing request host"))?
        .trim_matches(['[', ']'])
        .to_string();
    Ok(ValidatedTarget {
        url,
        addresses: addresses
            .iter()
            .copied()
            .map(|ip| SocketAddr::new(ip, port))
            .collect(),
        tls_host: host,
    })
}
pub fn validate_redirect(
    raw: &str,
    policy: Policy,
    previous_requests: usize,
) -> Result<url::Url, NetworkError> {
    if previous_requests >= 3 {
        return Err(NetworkError("too many redirects"));
    }
    validate_url(raw, policy)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_dns_mixed_answers_and_mapped_private_are_blocked_before_dial() {
        let url = validate_url("https://example.invalid/data", Policy::ExternalHttps).unwrap();
        for ip in [
            "127.0.0.1",
            "10.0.0.1", // secrets-scan: allow 10.0.0.1 -- synthetic RFC1918 rejection fixture, never dialed
            "169.254.169.254",
            "::1",
            "::ffff:127.0.0.1",
            "fc00::1",
            "fe80::1",
            "224.0.0.1",
        ] {
            let ips = ["203.0.113.8".parse().unwrap(), ip.parse().unwrap()];
            assert!(
                validated_target(url.clone(), &ips, Policy::ExternalHttps).is_err(),
                "{ip}"
            );
        }
    }
    #[test]
    fn checked_dns_addresses_and_original_tls_host_are_retained() {
        let url = validate_url("https://example.invalid:8443/x", Policy::ExternalHttps).unwrap();
        let target = validated_target(
            url,
            &["198.51.100.4".parse().unwrap()],
            Policy::ExternalHttps,
        )
        .unwrap();
        assert_eq!(target.addresses[0].to_string(), "198.51.100.4:8443");
        assert_eq!(target.tls_host, "example.invalid");
    }
    #[test]
    fn notification_lan_is_distinct_from_public_and_managed_policies() {
        let lan_url = "http://192.168.2.3/topic"; // secrets-scan: allow 192.168.2.3 -- synthetic LAN policy fixture, never dialed
        let lan_address = "192.168.2.3"; // secrets-scan: allow 192.168.2.3 -- same synthetic LAN address
        let url = validate_url(lan_url, Policy::NotificationHttp).unwrap();
        assert!(
            validated_target(
                url.clone(),
                &[lan_address.parse().unwrap()],
                Policy::NotificationHttp
            )
            .is_ok()
        );
        assert!(validate_url(url.as_str(), Policy::ExternalHttps).is_err());
        assert!(
            validated_target(
                url,
                &[lan_address.parse().unwrap()],
                Policy::ManagedLoopbackHttp
            )
            .is_err()
        );
    }
    #[test]
    fn loopback_dns_and_redirects_fail_closed() {
        let url = validate_url(
            "http://localhost:48889/inference",
            Policy::ManagedLoopbackHttp,
        )
        .unwrap();
        assert!(
            validated_target(
                url.clone(),
                &["::ffff:127.0.0.1".parse().unwrap()],
                Policy::ManagedLoopbackHttp
            )
            .is_ok()
        );
        assert!(
            validated_target(
                url,
                &["203.0.113.8".parse().unwrap()],
                Policy::ManagedLoopbackHttp
            )
            .is_err()
        );
        assert!(
            validate_redirect("https://example.invalid", Policy::ManagedLoopbackHttp, 1).is_err()
        );
        assert!(validate_redirect("https://example.invalid", Policy::ExternalHttps, 3).is_err());
        assert!(
            validate_url("https://user:secret@example.invalid", Policy::ExternalHttps).is_err()
        );
    }
}
