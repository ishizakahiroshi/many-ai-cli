use super::{
    auth::*,
    http::{Request, Response, content_security_policy, random_hex, require_method},
    pin::AuthService,
};
use crate::config::Config;
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
        _ => "application/octet-stream",
    }
}
pub fn static_asset(request: &Request) -> Response {
    // Static assets intentionally do not inherit API token/Host/PIN guards.
    if !matches!(request.method.as_str(), "GET" | "HEAD") {
        let mut r = Response::bytes(
            405,
            "text/plain; charset=utf-8",
            b"Method Not Allowed\n".to_vec(),
        );
        r.headers.insert("Allow".into(), "GET, HEAD".into());
        return r;
    }
    if request.path.split('/').any(|s| matches!(s, "." | "..")) {
        return Response::bytes(
            404,
            "text/plain; charset=utf-8",
            b"404 page not found\n".to_vec(),
        );
    }
    let Some(bytes) = crate::assets::web_asset(&request.path) else {
        return Response::bytes(
            404,
            "text/plain; charset=utf-8",
            b"404 page not found\n".to_vec(),
        );
    };
    let mut response = Response::bytes(
        200,
        mime(&request.path),
        if request.method == "HEAD" {
            vec![]
        } else {
            bytes.to_vec()
        },
    );
    response
        .headers
        .insert("Content-Length".into(), bytes.len().to_string());
    response
        .headers
        .insert("Accept-Ranges".into(), "bytes".into());
    response
}
pub fn index(
    request: &Request,
    config: &Config,
    port: i64,
    auth: &AuthService,
    now: i64,
) -> Response {
    let authed = token_or_trusted(request, config);
    let wants_document = request
        .header("Sec-Fetch-Dest")
        .trim()
        .eq_ignore_ascii_case("document")
        || request
            .header("Accept")
            .to_lowercase()
            .contains("text/html");
    if !authed && !wants_document {
        return Response::error(401, "unauthorized", "unauthorized");
    }
    if let Err(e) = require_method(request, &["GET"]) {
        return e;
    }
    if let Err(e) = require_host(request, config, port) {
        return e;
    }
    if !authed {
        return reauth();
    }
    let Some(bytes) = crate::assets::web_asset("index.html") else {
        return Response::bytes(
            500,
            "text/plain; charset=utf-8",
            b"asset missing\n".to_vec(),
        );
    };
    let mut response = Response::bytes(200, "text/html; charset=utf-8", bytes.to_vec());
    response
        .headers
        .insert("Cache-Control".into(), "no-store, must-revalidate".into());
    response.headers.insert(
        "Content-Security-Policy".into(),
        content_security_policy(&config.hub.allowed_hosts),
    );
    if valid_token(&request_token(request), &config.token) {
        response.cookies.push(super::pin::cookie(
            TOKEN_COOKIE,
            &config.token,
            false,
            86400,
            request,
        ));
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
    response.headers.insert("Content-Security-Policy".into(),format!("default-src 'none'; script-src 'nonce-{nonce}'; style-src 'nonce-{nonce}'; connect-src 'self'; base-uri 'none'; frame-ancestors 'none'"));
    response
}
