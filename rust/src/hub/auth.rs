//! Go-compatible token, host, origin, and logical-remote guards.
use super::http::{Request, Response, require_method};
use crate::config::Config;
use std::net::{IpAddr, SocketAddr};

pub const TOKEN_COOKIE: &str = "MANY_AI_CLI_token";
pub const PIN_COOKIE: &str = "MANY_AI_CLI_pin";

pub fn valid_token(got: &str, want: &str) -> bool {
    use subtle::ConstantTimeEq;
    if got.is_empty() || want.is_empty() {
        return false;
    }
    bool::from(got.as_bytes().ct_eq(want.as_bytes()))
}
pub fn request_token(request: &Request) -> String {
    let query = request.query("token");
    if !query.is_empty() {
        return query;
    }
    let auth = request.header("Authorization").trim();
    if let Some(value) = auth.strip_prefix("Bearer ") {
        return value.trim().into();
    }
    request
        .cookie(TOKEN_COOKIE)
        .unwrap_or_default()
        .trim()
        .into()
}

/// Go net.SplitHostPort semantics relevant to Host: raw IPv6 is portless; the
/// bracketed form permits a port. Empty ports are intentionally retained.
fn split_host_port(value: &str) -> Option<(&str, &str)> {
    if let Some(rest) = value.strip_prefix('[') {
        let (host, suffix) = rest.split_once(']')?;
        return suffix
            .strip_prefix(':')
            .filter(|p| !p.contains(':'))
            .map(|port| (host, port));
    }
    let (host, port) = value.split_once(':')?;
    if host.contains(['[', ']']) || port.contains([':', '[', ']']) {
        None
    } else {
        Some((host, port))
    }
}
pub fn remote_ip(value: &str) -> Option<IpAddr> {
    let value = value.trim();
    if let Ok(address) = value.parse::<SocketAddr>() {
        return Some(unmap(address.ip()));
    }
    let value = split_host_port(value)
        .map(|p| p.0)
        .unwrap_or(value)
        .trim_matches(['[', ']']);
    value.split('%').next()?.parse().ok().map(unmap)
}
fn unmap(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v) => v.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(ip),
        _ => ip,
    }
}
pub fn is_loopback(remote: &str) -> bool {
    remote_ip(remote)
        .map(unmap)
        .is_some_and(|v| v.is_loopback())
}
pub fn default_host(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}
fn configured_host(host: &str, hosts: &[String]) -> bool {
    hosts.iter().any(|raw| {
        let allowed = raw.trim().trim_matches(['[', ']']).to_lowercase();
        let allowed = allowed.strip_suffix('.').unwrap_or(&allowed);
        !allowed.is_empty() && allowed == host
    })
}
pub fn allowed_host(raw: &str, port: i64, hosts: &[String]) -> bool {
    let raw = raw.trim();
    if raw.is_empty() {
        return port <= 0;
    }
    let (host, got_port) = split_host_port(raw).unwrap_or((raw.trim_matches(['[', ']']), ""));
    let host = host.trim().to_lowercase();
    let host = host.strip_suffix('.').unwrap_or(&host);
    if !default_host(host) && !configured_host(host, hosts) {
        return false;
    }
    port <= 0 || got_port.is_empty() || got_port.parse::<i64>().is_ok_and(|p| p == port)
}
pub fn allowed_origin(raw: &str, port: i64, hosts: &[String]) -> bool {
    // URL normalizes some illegal Go URL inputs; reject whitespace/control and
    // backslashes before parsing so they cannot be silently repaired.
    if raw.chars().any(|c| c.is_control() || c == '\\' || c == ' ') {
        return false;
    }
    let Ok(url) = url::Url::parse(raw) else {
        return false;
    };
    if url.host_str().is_none() {
        return false;
    }
    // net/url preserves the spelling of hosts. In particular 127.1, integer
    // IPv4 and hexadecimal IPv4 must not become the allowed 127.0.0.1 host.
    let Some((_, rest)) = raw.split_once("://") else {
        return false;
    };
    let authority = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .rsplit('@')
        .next()
        .unwrap_or("");
    let (host, explicit_port) =
        split_host_port(authority).unwrap_or((authority.trim_matches(['[', ']']), ""));
    let host = host.to_lowercase();
    let host = host.strip_suffix('.').unwrap_or(&host);
    match url.scheme() {
        "http" => {
            let raw_port = if explicit_port.is_empty() {
                "80"
            } else {
                explicit_port
            };
            let authority = if host.contains(':') {
                format!("[{host}]:{raw_port}")
            } else {
                format!("{host}:{raw_port}")
            };
            allowed_host(&authority, port, hosts)
        }
        "https" => configured_host(host, hosts),
        _ => false,
    }
}
pub fn logically_remote(request: &Request) -> bool {
    if !is_loopback(&request.remote_addr) {
        return true;
    }
    let raw = request.host.trim();
    if raw.is_empty() {
        return false;
    }
    let host = split_host_port(raw)
        .map(|p| p.0)
        .unwrap_or(raw)
        .trim_matches(['[', ']'])
        .to_lowercase();
    let host = host.strip_suffix('.').unwrap_or(&host);
    if !host.is_empty() && !default_host(host) {
        return true;
    }
    request.headers.iter().any(|(k, v)| {
        let k = k.trim().to_ascii_lowercase();
        (k == "tailscale-user-login" || k.starts_with("x-forwarded-")) && !v.trim().is_empty()
    })
}
pub fn uses_https(request: &Request) -> bool {
    request.tls
        || (is_loopback(&request.remote_addr)
            && request
                .header("X-Forwarded-Proto")
                .trim()
                .eq_ignore_ascii_case("https"))
}

fn trusted_remote(remote: &str, networks: &[String]) -> bool {
    let Some(ip) = remote_ip(remote).map(unmap) else {
        return false;
    };
    networks.iter().any(|raw| {
        let Some((network, prefix)) = raw.trim().split_once('/') else {
            return false;
        };
        let (Ok(network), Ok(prefix)) = (network.parse::<IpAddr>(), prefix.parse::<u32>()) else {
            return false;
        };
        match (ip, network) {
            (IpAddr::V4(ip), IpAddr::V4(net)) if prefix <= 32 => {
                let mask = u32::MAX.checked_shl(32 - prefix).unwrap_or(0);
                u32::from(ip) & mask == u32::from(net) & mask
            }
            (IpAddr::V6(ip), IpAddr::V6(net)) if prefix <= 128 => {
                let mask = u128::MAX.checked_shl(128 - prefix).unwrap_or(0);
                u128::from(ip) & mask == u128::from(net) & mask
            }
            _ => false,
        }
    })
}
pub fn token_or_trusted(request: &Request, config: &Config) -> bool {
    valid_token(&request_token(request), &config.token)
        || (config.hub.allow_loopback_without_token
            && ((!logically_remote(request) && is_loopback(&request.remote_addr))
                || trusted_remote(&request.remote_addr, &config.hub.trusted_networks)))
}
pub fn require_host(request: &Request, config: &Config, port: i64) -> Result<(), Response> {
    if allowed_host(&request.host, port, &config.hub.allowed_hosts) {
        Ok(())
    } else {
        Err(Response::error(403, "forbidden", "host not allowed"))
    }
}
pub fn require_origin(request: &Request, config: &Config, port: i64) -> Result<(), Response> {
    let origin = request.header("Origin").trim();
    let allowed = if !origin.is_empty() {
        allowed_origin(origin, port, &config.hub.allowed_hosts)
    } else {
        matches!(
            request
                .header("Sec-Fetch-Site")
                .trim()
                .to_ascii_lowercase()
                .as_str(),
            "" | "none" | "same-origin"
        )
    };
    if allowed {
        Ok(())
    } else {
        Err(Response::error(403, "forbidden", "origin not allowed"))
    }
}
pub fn guard_base(
    request: &Request,
    config: &Config,
    port: i64,
    methods: &[&str],
) -> Result<(), Response> {
    if !token_or_trusted(request, config) {
        return Err(Response::error(401, "unauthorized", "unauthorized"));
    }
    require_method(request, methods)?;
    require_host(request, config, port)?;
    if !matches!(request.method.as_str(), "GET" | "HEAD" | "OPTIONS") {
        require_origin(request, config, port)?;
    }
    Ok(())
}

pub fn websocket_origin(request: &Request, config: &Config, port: i64) -> Result<(), Response> {
    require_host(request, config, port)?;
    let origin = request.header("Origin");
    if origin.is_empty() {
        if logically_remote(request) {
            Err(Response::error(
                403,
                "forbidden",
                "origin required for remote websocket clients",
            ))
        } else {
            Ok(())
        }
    } else if allowed_origin(origin, port, &config.hub.allowed_hosts) {
        Ok(())
    } else {
        Err(Response::error(403, "forbidden", "origin not allowed"))
    }
}
