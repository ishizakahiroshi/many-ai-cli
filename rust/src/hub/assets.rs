#[path = "asset_content.rs"]
mod asset_content;
use super::{
    auth::*,
    http::{Request, Response, content_security_policy, random_hex, require_method},
    pin::AuthService,
};
use crate::config::Config;
pub trait AssetSource: Send + Sync {
    fn static_response(&self, request: &Request) -> Response;
    fn index_bytes(&self) -> std::io::Result<Vec<u8>>;
}
/// Preserve the pinned FileServer directory/range/HEAD semantics for a live
/// source asset table assembled by the explicitly selected developer owner.
pub fn file_table_response(request: &Request, assets: &[(&str, &[u8])]) -> Response {
    let mut response = asset_content::file_server(request, assets);
    response.security_headers();
    response
}
pub const STATIC_PATHS: &[&str] = &[
    "/app-entry.js",
    "/app.js",
    "/app/",
    "/debug/",
    "/styles.css",
    "/styles/",
    "/icon.svg",
    "/icons/",
    "/manifest.webmanifest",
    "/sw.js",
    "/whisper-recorder-worklet.js",
    "/i18n.js",
    "/i18n/",
    "/vendor/",
];
pub fn is_static(path: &str) -> bool {
    STATIC_PATHS.iter().any(|p| {
        if p.ends_with('/') {
            path.starts_with(p)
        } else {
            path == *p
        }
    })
}
fn mime(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "webmanifest" => "application/manifest+json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "html" => "text/html; charset=utf-8",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}
pub fn static_asset(request: &Request) -> Response {
    // Static assets intentionally do not inherit API token/Host/PIN guards.
    let mut response = asset_content::file_server(request, crate::assets::WEB_ASSETS);
    response.security_headers();
    response
}
pub fn index(
    request: &Request,
    config: &Config,
    port: i64,
    auth: &AuthService,
    now: i64,
) -> Response {
    index_with_source(request, config, port, auth, now, None)
}
pub fn index_with_source(
    request: &Request,
    config: &Config,
    port: i64,
    auth: &AuthService,
    now: i64,
    source: Option<&dyn AssetSource>,
) -> Response {
    // The page holds nothing secret and is served without the token: the
    // dashboard presents the token it keeps in a header and in the first
    // WebSocket message, never in a cookie (U05).
    if let Err(e) = require_method(request, &["GET"]) {
        return e;
    }
    if let Err(e) = require_host(request, config, port) {
        return e;
    }
    // A launch code is traded on the sign-in page (U04).
    if !request.query("launch").is_empty() {
        return reauth();
    }
    let bytes = source.map_or_else(
        || {
            crate::assets::web_asset("index.html")
                .map(<[u8]>::to_vec)
                .ok_or_else(|| std::io::Error::other("embedded index missing"))
        },
        |source| source.index_bytes(),
    );
    let Ok(bytes) = bytes else {
        return Response::bytes(
            500,
            "text/plain; charset=utf-8",
            b"asset missing\n".to_vec(),
        );
    };
    let mut response = Response::bytes(200, "text/html; charset=utf-8", bytes);
    response
        .headers
        .insert("Cache-Control".into(), "no-store, must-revalidate".into());
    response.headers.insert(
        "Content-Security-Policy".into(),
        content_security_policy(&config.hub.allowed_hosts),
    );
    // A token cookie an earlier version left behind is deleted.
    if request.cookie(TOKEN_COOKIE).is_some() {
        response
            .cookies
            .push(super::pin::cookie(TOKEN_COOKIE, "", false, 0, request));
    }
    if let Some(cookie) = auth.issue_ui_cookie(request, config, now) {
        response.cookies.push(cookie);
    }
    response
}
fn reauth() -> Response {
    let Ok(nonce) = random_hex(16) else {
        return Response::error(401, "unauthorized", "unauthorized");
    };
    let html = include_str!("reauth.html")
        .replace("__NONCE__", &nonce)
        .replace("__TOKEN_KEY__", "many-ai-cli-token")
        .replace("__GUARD_KEY__", "many-ai-cli-reauth");
    let mut response = Response::bytes(401, "text/html; charset=utf-8", html.into_bytes());
    response
        .headers
        .insert("Cache-Control".into(), "no-store, must-revalidate".into());
    response.headers.insert("Content-Security-Policy".into(),format!("default-src 'none'; script-src 'nonce-{nonce}'; style-src 'nonce-{nonce}'; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"));
    response
}
