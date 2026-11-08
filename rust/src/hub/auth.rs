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
/// The token a request presents: in the query or a Bearer header. The token is
/// no longer kept in a cookie: a browser sends a cookie for 127.0.0.1 to every
/// port there, including another user's server (U05).
pub fn request_token(request: &Request) -> String {
    let query = request.query("token");
    if !query.is_empty() {
        return query;
    }
    let auth = request.header("Authorization").trim();
    if let Some(value) = auth.strip_prefix("Bearer ") {
        return value.trim().into();
    }
    String::new()
}

/// How long an asset ticket lasts. The dashboard renews it every hour.
pub const ASSET_TICKET_TTL: i64 = 12 * 60 * 60;
/// Routes an `<img>`, `<video>`, `<audio>` or download link loads. They cannot
/// send a header, so they may present an asset ticket instead of the token.
pub fn is_asset_route(path: &str) -> bool {
    matches!(
        path,
        "/api/files-asset"
            | "/api/files-download"
            | "/api/avatar"
            | "/api/user-prefs/notify-sound-custom"
    ) || path.starts_with("/api/memo-images/")
        || path.starts_with("/api/provider-icons/")
}
fn asset_signature(token: &str, expiry: i64) -> String {
    use hmac::{Hmac, KeyInit, Mac};
    let mut mac =
        Hmac::<sha2::Sha256>::new_from_slice(token.as_bytes()).expect("HMAC arbitrary key length");
    mac.update(format!("asset-ticket-v1:{expiry}").as_bytes());
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
/// A read-only ticket for the asset routes, bound to the token: rotating the
/// token ends every ticket made from it.
pub fn asset_ticket(token: &str, now: i64) -> String {
    let expiry = now + ASSET_TICKET_TTL;
    format!("{expiry}.{}", asset_signature(token, expiry))
}
pub fn valid_asset_ticket(ticket: &str, token: &str, now: i64) -> bool {
    let Some((expiry, signature)) = ticket.trim().split_once('.') else {
        return false;
    };
    let Ok(expiry) = expiry.parse::<i64>() else {
        return false;
    };
    !token.is_empty()
        && expiry > now
        && expiry <= now + ASSET_TICKET_TTL
        && valid_token(signature, &asset_signature(token, expiry))
}
fn asset_ticket_ok(request: &Request, config: &Config) -> bool {
    if !matches!(request.method.as_str(), "GET" | "HEAD") || !is_asset_route(&request.path) {
        return false;
    }
    let ticket = request.query("ticket");
    if ticket.is_empty() {
        return false;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64);
    valid_asset_ticket(&ticket, &config.token, now)
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
/// `addressed` is the authority the request addressed (`request_authority`).
/// An https origin must name exactly that host and port: a page on the same
/// machine name at another port is a different origin, and the browser sends
/// the Hub's cookies to it all the same (U06).
pub fn allowed_origin(raw: &str, port: i64, hosts: &[String], addressed: &str) -> bool {
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
        "https" => {
            let origin_port = if explicit_port.is_empty() {
                "443"
            } else {
                explicit_port
            };
            configured_host(host, hosts) && same_https_authority(host, origin_port, addressed)
        }
        _ => false,
    }
}
/// The authority the browser addressed: X-Forwarded-Host when a loopback
/// proxy (`tailscale serve`) sets it, otherwise Host. A browser page cannot
/// set either header on a WebSocket or a CORS-free request.
pub fn request_authority(request: &Request) -> &str {
    let forwarded = request.header("X-Forwarded-Host").trim();
    if !forwarded.is_empty() && is_loopback(&request.remote_addr) {
        forwarded.split(',').next().unwrap_or("").trim()
    } else {
        request.host.trim()
    }
}
fn same_https_authority(host: &str, port: &str, authority: &str) -> bool {
    let (authority_host, authority_port) =
        split_host_port(authority).unwrap_or((authority.trim_matches(['[', ']']), ""));
    let authority_host = authority_host.to_lowercase();
    let authority_host = authority_host.strip_suffix('.').unwrap_or(&authority_host);
    let authority_port = if authority_port.is_empty() {
        "443"
    } else {
        authority_port
    };
    !authority_host.is_empty()
        && authority_host == host
        && matches!(
            (authority_port.parse::<u16>(), port.parse::<u16>()),
            (Ok(addressed), Ok(origin)) if addressed == origin
        )
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
        || asset_ticket_ok(request, config)
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
        allowed_origin(
            origin,
            port,
            &config.hub.allowed_hosts,
            request_authority(request),
        )
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
    } else if allowed_origin(
        origin,
        port,
        &config.hub.allowed_hosts,
        request_authority(request),
    ) {
        Ok(())
    } else {
        Err(Response::error(403, "forbidden", "origin not allowed"))
    }
}
