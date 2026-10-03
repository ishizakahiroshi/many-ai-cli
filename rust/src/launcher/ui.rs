//! Authenticated five-route selection UI using the frozen embedded HTML.
use super::{ConnectionManager, LauncherError, Profile, profile::SCHEMAS};
use crate::{
    hub::http::{Request, Response},
    process::Cancellation,
    proto::wire::{GoWire, Schema},
};
use serde::Deserialize;
use serde_json::json;
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicU16, Ordering},
    },
    time::Duration,
};
use subtle::ConstantTimeEq;

#[derive(Default, Deserialize)]
#[serde(default)]
struct ProfilesRequest {
    profiles: Option<Vec<Profile>>,
}
impl GoWire for ProfilesRequest {
    const GO_TYPE: &'static str = "LauncherProfilesRequest";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct ConnectRequest {
    name: String,
}
impl GoWire for ConnectRequest {
    const GO_TYPE: &'static str = "LauncherConnectRequest";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct DisconnectRequest {
    name: String,
    mode: String,
}
impl GoWire for DisconnectRequest {
    const GO_TYPE: &'static str = "LauncherDisconnectRequest";
    const SCHEMAS: &'static [Schema] = SCHEMAS;
}
pub struct UiServer {
    pub manager: Arc<ConnectionManager>,
    token: String,
    port: AtomicU16,
}
impl UiServer {
    pub fn new(manager: Arc<ConnectionManager>) -> io::Result<Arc<Self>> {
        let token = crate::process::random_token()?[..32].to_owned();
        Ok(Arc::new(Self {
            manager,
            token,
            port: AtomicU16::new(0),
        }))
    }
    pub fn from_token(
        manager: Arc<ConnectionManager>,
        token: String,
        port: u16,
    ) -> io::Result<Arc<Self>> {
        if token.is_empty() {
            return Err(io::Error::other("UI token cannot be empty"));
        }
        Ok(Arc::new(Self {
            manager,
            token,
            port: AtomicU16::new(port),
        }))
    }
    pub fn page_url(&self) -> String {
        format!(
            "http://127.0.0.1:{}/?token={}",
            self.port.load(Ordering::Acquire),
            super::query_escape(&self.token)
        )
    }
    fn valid_token(&self, request: &Request) -> bool {
        let query = request.query("token");
        let header = request.header("Authorization").trim();
        let token = if !query.is_empty() {
            query.as_str()
        } else {
            header.strip_prefix("Bearer ").unwrap_or_default().trim()
        };
        !token.is_empty() && bool::from(token.as_bytes().ct_eq(self.token.as_bytes()))
    }
    pub fn preflight(&self, request: &Request) -> Result<(), Response> {
        if request.path == "/" {
            if !matches!(request.method.as_str(), "GET" | "HEAD") {
                return Err(text_error(405, "method not allowed"));
            }
            return if self.valid_token(request) {
                Ok(())
            } else {
                Err(text_error(401, "unauthorized"))
            };
        }
        if !matches!(
            request.path.as_str(),
            "/api/profiles" | "/api/connect" | "/api/connect/status" | "/api/disconnect"
        ) {
            return Err(text_error(404, "404 page not found"));
        }
        if !self.valid_token(request) {
            return Err(ui_error(401, "unauthorized"));
        }
        if !matches!(request.method.as_str(), "GET" | "HEAD" | "OPTIONS") {
            if !allowed_ui_host(&request.host, self.port.load(Ordering::Acquire)) {
                return Err(ui_error(403, "host not allowed"));
            }
            let origin = request.header("Origin").trim();
            if !origin.is_empty() && !allowed_ui_origin(origin, self.port.load(Ordering::Acquire)) {
                return Err(ui_error(403, "origin not allowed"));
            }
        }
        Ok(())
    }
    pub async fn handle(self: &Arc<Self>, request: &Request, cancel: &Cancellation) -> Response {
        if let Err(response) = self.preflight(request) {
            return response;
        }
        let result: Result<Response, LauncherError> = async {
            match (request.path.as_str(), request.method.as_str()) {
                ("/", "GET" | "HEAD") => Ok(index_response(request)),
                ("/api/profiles", "GET") => {
                    Ok(Response::json(200, &self.manager.profiles().await?))
                }
                ("/api/profiles", "POST") => {
                    let profiles: ProfilesRequest = decode(request, 512 * 1024)?;
                    self.manager.replace_profiles(profiles.profiles).await?;
                    Ok(Response::json(200, &json!({"ok":true})))
                }
                ("/api/connect", "POST") => {
                    let request: ConnectRequest = decode(request, 4 * 1024)?;
                    self.manager.connect(&request.name).await?;
                    Ok(Response::json(
                        200,
                        &json!({"ok":true,"status":"connecting"}),
                    ))
                }
                ("/api/connect/status", "GET") => {
                    Ok(Response::json(200, &self.manager.status().await))
                }
                ("/api/disconnect", "POST") => {
                    let request: DisconnectRequest = decode(request, 4 * 1024)?;
                    let warning = self
                        .manager
                        .disconnect(&request.name, &request.mode, cancel, "launcher")
                        .await?;
                    let mut response = json!({"ok":true});
                    if let Some(warning) = warning {
                        response["warning"] = warning.into();
                    }
                    Ok(Response::json(200, &response))
                }
                _ => Ok(ui_error(405, "method not allowed")),
            }
        }
        .await;
        result.unwrap_or_else(|error| ui_error(error.status, &error.detail))
    }
    pub async fn serve(self: &Arc<Self>, cancel: Cancellation) -> io::Result<UiServerHandle> {
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        self.port
            .store(listener.local_addr()?.port(), Ordering::Release);
        let server = self.clone();
        let url = self.page_url();
        let stop = cancel.clone();
        let task = tokio::spawn(async move {
            use axum::routing::any;
            let router = axum::Router::new()
                .fallback(any(transport_handle))
                .with_state(server.clone());
            let mut connections = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    _=stop.cancelled()=>break,
                    Some(_)=connections.join_next(),if !connections.is_empty()=>{},
                    accepted=listener.accept()=>{let(stream,_)=accepted?;let service=hyper_util::service::TowerToHyperService::new(router.clone());connections.spawn(async move{let mut builder=hyper::server::conn::http1::Builder::new();builder.timer(hyper_util::rt::TokioTimer::new()).header_read_timeout(Duration::from_secs(30));builder.serve_connection(hyper_util::rt::TokioIo::new(stream),service).await});}
                }
            }
            connections.abort_all();
            while connections.join_next().await.is_some() {}
            server.manager.close_all().await;
            Ok(())
        });
        Ok(UiServerHandle {
            url,
            cancel,
            task: Some(task),
        })
    }
}
pub struct UiServerHandle {
    pub url: String,
    cancel: Cancellation,
    task: Option<tokio::task::JoinHandle<io::Result<()>>>,
}
impl UiServerHandle {
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
    pub async fn wait(&mut self) -> io::Result<()> {
        if let Some(task) = self.task.as_mut() {
            let result = task.await.map_err(io::Error::other);
            self.task = None;
            result?
        } else {
            Ok(())
        }
    }
}
impl Drop for UiServerHandle {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
fn decode<T: GoWire>(request: &Request, cap: usize) -> Result<T, LauncherError> {
    crate::proto::decode_http_json(&request.body[..request.body.len().min(cap)])
        .map_err(|_| LauncherError::new(400, "bad_request", "invalid json"))
}
fn ui_error(status: u16, detail: &str) -> Response {
    Response::json(status, &json!({"ok":false,"error":detail}))
}
fn text_error(status: u16, detail: &str) -> Response {
    Response::bytes(
        status,
        "text/plain; charset=utf-8",
        format!("{detail}\n").into_bytes(),
    )
}
pub fn allowed_ui_host(hostport: &str, port: u16) -> bool {
    let hostport = hostport.trim();
    if hostport.is_empty() {
        return false;
    }
    let (host, raw_port) = if let Some(rest) = hostport.strip_prefix('[') {
        if let Some((host, tail)) = rest.split_once(']') {
            if let Some(port) = tail.strip_prefix(':') {
                (host, port)
            } else {
                (hostport.trim_matches(['[', ']']), "")
            }
        } else {
            (hostport, "")
        }
    } else if hostport.matches(':').count() == 1 {
        hostport.split_once(':').unwrap()
    } else {
        (hostport.trim_matches(['[', ']']), "")
    };
    let host = host.trim().to_ascii_lowercase();
    let host = host.strip_suffix('.').unwrap_or(&host);
    if !matches!(host, "127.0.0.1" | "localhost" | "::1") {
        return false;
    }
    port == 0 || raw_port.is_empty() || raw_port.parse::<u16>().is_ok_and(|p| p == port)
}
pub fn allowed_ui_origin(origin: &str, port: u16) -> bool {
    // net/url preserves an omitted port; url::Url normalizes :80 away. Keep the
    // original authority so explicit wrong ports cannot become implicit success.
    let Some((scheme, rest)) = origin.split_once("://") else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("http") {
        return false;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if url::Url::parse(origin).is_err() {
        return false;
    }
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    allowed_ui_host(authority, port)
}
async fn transport_handle(
    axum::extract::State(server): axum::extract::State<Arc<UiServer>>,
    raw: axum::extract::Request,
) -> axum::response::Response {
    use axum::body::HttpBody;
    use std::{future::poll_fn, pin::Pin};
    let (parts, mut body) = raw.into_parts();
    let mut request = Request {
        method: parts.method.to_string(),
        path: parts.uri.path().into(),
        query: parts.uri.query().unwrap_or_default().into(),
        host: parts
            .uri
            .authority()
            .map(|a| a.as_str())
            .unwrap_or_else(|| {
                parts
                    .headers
                    .get("host")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or_default()
            })
            .into(),
        headers: parts
            .headers
            .iter()
            .map(|(k, v)| {
                (
                    k.to_string(),
                    String::from_utf8_lossy(v.as_bytes()).into_owned(),
                )
            })
            .collect(),
        ..Request::default()
    };
    if let Err(response) = server.preflight(&request) {
        return crate::hub::transport::response(response);
    }
    if request.method == "POST" {
        let cap = if request.path == "/api/profiles" {
            512 * 1024
        } else {
            4 * 1024
        };
        let read = async {
            while let Some(frame) = poll_fn(|cx| Pin::new(&mut body).poll_frame(cx)).await {
                let frame = frame.map_err(io::Error::other)?;
                if let Ok(bytes) = frame.into_data() {
                    let remaining = (cap + 1usize).saturating_sub(request.body.len());
                    request
                        .body
                        .extend_from_slice(&bytes[..bytes.len().min(remaining)]);
                    let repaired = String::from_utf8_lossy(&request.body);
                    let mut decoder = serde_json::Deserializer::from_str(&repaired);
                    if Box::<serde_json::value::RawValue>::deserialize(&mut decoder).is_ok() {
                        break;
                    }
                    if request.body.len() > cap {
                        break;
                    }
                }
            }
            Ok::<_, io::Error>(())
        };
        if !matches!(
            tokio::time::timeout(Duration::from_secs(30), read).await,
            Ok(Ok(()))
        ) {
            return crate::hub::transport::response(ui_error(400, "invalid json"));
        }
    }
    let response = server.handle(&request, &Cancellation::default()).await;
    crate::hub::transport::response(response)
}

fn index_response(request: &Request) -> Response {
    let bytes = crate::assets::LAUNCHER_UI;
    let size = bytes.len();
    let mut response = Response::bytes(200, "text/html; charset=utf-8", Vec::new());
    // The frozen launcher asset has inline scripts; Hub's CSP is inapplicable.
    response.headers.remove("Content-Security-Policy");
    let if_match = request.header("If-Match").trim();
    if !if_match.is_empty() && if_match != "*" {
        response.status = 412;
        response.headers.remove("Content-Type");
        return response;
    }
    if request
        .header("If-None-Match")
        .split(',')
        .any(|tag| tag.trim() == "*")
    {
        response.status = 304;
        response.headers.remove("Content-Type");
        return response;
    }
    let range = if request.header("If-Range").is_empty() {
        request.header("Range")
    } else {
        ""
    };
    let mut ranges = match parse_ranges(range, size) {
        Ok(ranges) => ranges,
        Err(no_overlap) => {
            let mut response = text_error(
                416,
                if no_overlap {
                    "invalid range: failed to overlap"
                } else {
                    "invalid range"
                },
            );
            if no_overlap {
                response
                    .headers
                    .insert("Content-Range".into(), format!("bytes */{size}"));
            }
            return response;
        }
    };
    if ranges.iter().map(|(_, len)| *len).sum::<usize>() > size {
        ranges.clear();
    }
    match ranges.as_slice() {
        [] => response.body = bytes.to_vec(),
        &[(start, len)] => {
            response.status = 206;
            response.headers.insert(
                "Content-Range".into(),
                format!("bytes {start}-{}/{size}", start as i64 + len as i64 - 1),
            );
            response.body = bytes[start..start + len].to_vec();
        }
        _ => {
            let boundary = match crate::process::random_token() {
                Ok(value) => value[..60].to_owned(),
                Err(_) => return text_error(500, "internal error"),
            };
            response.status = 206;
            response.headers.insert(
                "Content-Type".into(),
                format!("multipart/byteranges; boundary={boundary}"),
            );
            for (i, (start, len)) in ranges.iter().enumerate() {
                response.body.extend_from_slice(format!("{}--{boundary}\r\nContent-Range: bytes {start}-{}/{size}\r\nContent-Type: text/html; charset=utf-8\r\n\r\n", if i == 0 { "" } else { "\r\n" }, *start as i64 + *len as i64 - 1).as_bytes());
                response.body.extend_from_slice(&bytes[*start..start + len]);
            }
            response
                .body
                .extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        }
    }
    response
        .headers
        .insert("Accept-Ranges".into(), "bytes".into());
    response
        .headers
        .insert("Content-Length".into(), response.body.len().to_string());
    if request.method == "HEAD" {
        response.body.clear();
    }
    response
}
fn parse_ranges(raw: &str, size: usize) -> Result<Vec<(usize, usize)>, bool> {
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    let Some(raw) = raw.strip_prefix("bytes=") else {
        return Err(false);
    };
    let mut ranges = Vec::new();
    let mut no_overlap = false;
    for part in raw
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let Some((start, end)) = part.split_once('-') else {
            return Err(false);
        };
        let (start, end) = (start.trim(), end.trim());
        if start.is_empty() {
            let n = end.parse::<i64>().map_err(|_| false)?;
            if n < 0 {
                return Err(false);
            }
            let len = (n as usize).min(size);
            ranges.push((size - len, len));
        } else {
            let start = start.parse::<i64>().map_err(|_| false)?;
            if start < 0 {
                return Err(false);
            }
            if start as usize >= size {
                no_overlap = true;
                continue;
            }
            let end = if end.is_empty() {
                size as i64 - 1
            } else {
                end.parse::<i64>().map_err(|_| false)?
            };
            if end < start {
                return Err(false);
            }
            ranges.push((
                start as usize,
                (end as usize).min(size - 1) - start as usize + 1,
            ));
        }
    }
    if no_overlap && ranges.is_empty() {
        Err(true)
    } else {
        Ok(ranges)
    }
}
