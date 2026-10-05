//! Loopback HTTP adapter. Core effects have an explicit driver; file downloads
//! retain the already-open descriptor and stream at a bounded 64 KiB per chunk.
use super::{
    ServiceRouter,
    http::{self, Request, Response},
};
use crate::{
    process::Cancellation,
    proto::core::{CoreEffectFailure, CoreEffectSink},
};
use axum::{
    Router,
    body::{Body, Bytes, HttpBody},
    extract::{ConnectInfo, FromRequestParts, State, ws::WebSocketUpgrade},
    http::{HeaderName, HeaderValue},
    response::{IntoResponse, Response as AxumResponse},
    routing::any,
};
use std::{
    future::Future,
    io::{self, Read, Seek, SeekFrom},
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

#[derive(Clone)]
struct TransportState {
    services: Arc<ServiceRouter>,
    sink: Arc<dyn CoreEffectSink>,
    websockets: Option<Arc<super::websocket::WebSocketService>>,
}
struct HttpWaitGuard(crate::proto::core::HttpWaitCancellation);
impl Drop for HttpWaitGuard {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
pub fn router(services: Arc<ServiceRouter>, sink: Arc<dyn CoreEffectSink>) -> Router {
    Router::new()
        .fallback(any(handle))
        .with_state(TransportState {
            services,
            sink,
            websockets: None,
        })
}
pub async fn serve(
    listener: tokio::net::TcpListener,
    services: Arc<ServiceRouter>,
    sink: Arc<dyn CoreEffectSink>,
    cancel: Cancellation,
) -> io::Result<()> {
    serve_inner(listener, services.clone(), router(services, sink), cancel).await
}
/// The registered transport stays usable without WebSockets during incremental
/// migration; production wiring supplies this actual service explicitly.
pub fn router_with_websockets(
    services: Arc<ServiceRouter>,
    sink: Arc<dyn CoreEffectSink>,
    websockets: Arc<super::websocket::WebSocketService>,
) -> Router {
    Router::new()
        .fallback(any(handle))
        .with_state(TransportState {
            services,
            sink,
            websockets: Some(websockets),
        })
}
pub async fn serve_with_websockets(
    listener: tokio::net::TcpListener,
    services: Arc<ServiceRouter>,
    sink: Arc<dyn CoreEffectSink>,
    websockets: Arc<super::websocket::WebSocketService>,
    cancel: Cancellation,
) -> io::Result<()> {
    serve_inner(
        listener,
        services.clone(),
        router_with_websockets(services, sink, websockets),
        cancel,
    )
    .await
}
async fn serve_inner(
    listener: tokio::net::TcpListener,
    services: Arc<ServiceRouter>,
    router: Router,
    cancel: Cancellation,
) -> io::Result<()> {
    let local = listener.local_addr()?;
    if local.ip() != std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
        || local.port() != services.bound_port()
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Hub listener must match the explicit IPv4 loopback runtime port",
        ));
    }
    let mut connections = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            _=cancel.cancelled()=>break,
            Some(_)=connections.join_next(),if !connections.is_empty()=>{},
            accepted=listener.accept()=>{
                let(stream,peer)=accepted?;
                let service=hyper_util::service::TowerToHyperService::new(router.clone().layer(axum::Extension(ConnectInfo(peer))));
                connections.spawn(async move{
                    let mut builder=hyper::server::conn::http1::Builder::new();
                    builder.timer(hyper_util::rt::TokioTimer::new()).header_read_timeout(std::time::Duration::from_secs(10)).max_buf_size(1024*1024+4096);
                    builder.serve_connection(hyper_util::rt::TokioIo::new(stream),service).with_upgrades().await
                });
            }
        }
    }
    // Go Run uses http.Server.Close, not an unbounded graceful-body drain.
    connections.abort_all();
    while connections.join_next().await.is_some() {}
    Ok(())
}
async fn handle(
    State(state): State<TransportState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    raw: axum::extract::Request,
) -> AxumResponse {
    let (mut parts, body) = raw.into_parts();
    let mut request = Request {
        method: parts.method.to_string(),
        path: match decode_path(parts.uri.path()) {
            Some(p) => p,
            None => return response(Response::error(400, "bad_request", "invalid request path")),
        },
        query: parts.uri.query().unwrap_or("").into(),
        host: parts
            .uri
            .authority()
            .map(|a| a.as_str())
            .unwrap_or_else(|| {
                parts
                    .headers
                    .get("host")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
            })
            .into(),
        remote_addr: peer.to_string(),
        tls: false,
        headers: parts
            .headers
            .iter()
            .map(|(k, v)| {
                (
                    k.as_str().into(),
                    String::from_utf8_lossy(v.as_bytes()).into_owned(),
                )
            })
            .collect(),
        body: vec![],
    };
    if request.path == "/ws"
        && let Some(websockets) = state.websockets
    {
        if let Err(error) = websockets.handshake(&request) {
            return response(error);
        }
        let upgrade = match WebSocketUpgrade::from_request_parts(&mut parts, &()).await {
            Ok(upgrade) => upgrade,
            Err(error) => return error.into_response(),
        };
        return upgrade
            .max_message_size(super::websocket::MAX_PAYLOAD_BYTES)
            .max_frame_size(super::websocket::MAX_PAYLOAD_BYTES)
            .on_upgrade(move |socket| websockets.run(socket, request))
            .into_response();
    }
    let captured_at = crate::proto::time::Timestamp::now();
    let now = captured_at
        .duration_since(crate::proto::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    // Rejections must not wait for an attacker to finish a request body.
    if let Err(rejection) = state.services.preflight(&request, now) {
        return response(rejection);
    }
    if request.path == super::voice_routes::PATH {
        let Some(voice) = state.services.voice.clone() else {
            return response(Response::error(
                503,
                "whisper_unavailable",
                "voice service is not initialized",
            ));
        };
        let permit = match state.services.owned_request_permit() {
            Ok(permit) => permit,
            Err(_) => {
                return response(Response::error(
                    503,
                    "whisper_unavailable",
                    "voice service is shutting down",
                ));
            }
        };
        let cancellation = permit.cancellation();
        let waiter = crate::proto::core::HttpWaitCancellation::default();
        let _guard = HttpWaitGuard(waiter.clone());
        let waiter_token = waiter.token().clone();
        let completion=permit.start(async move {
            let operation=async {
                // Go starts or verifies a managed service before reading raw
                // audio; failures therefore don't wait on a slow upload.
                let config=match voice.prepare(cancellation.token()).await {Ok(config)=>config,Err(response)=>return response};
                let audio=match read_body(body,super::voice_routes::PATH).await {Ok(audio)=>audio,Err(_)=>return Response::error(400,"bad_request","read audio failed")};
                voice.transcribe_audio(&config,&audio,cancellation.token()).await
            };
            tokio::select! {
                result=operation=>result,
                _=waiter_token.cancelled()=>Response::error(502,"whisper_unreachable","whisper unreachable: request cancelled"),
                _=cancellation.token().cancelled()=>Response::error(503,"whisper_unavailable","voice service is shutting down"),
            }
        });
        return response(completion.wait().await.unwrap_or_else(|_| {
            Response::error(503, "whisper_unavailable", "voice request owner ended")
        }));
    }
    let reads_body = !matches!(request.method.as_str(), "GET" | "HEAD" | "OPTIONS")
        && request.path.starts_with("/api/")
        && !request
            .path
            .starts_with(super::approval_actions::ONE_TAP_PREFIX)
        && !(request.method == "DELETE"
            && (request.path == "/api/routines" || request.path.starts_with("/api/routines/")))
        && !(request.method == "DELETE" && request.path.starts_with("/api/providers/"))
        && !(request.method == "DELETE"
            && matches!(
                request.path.as_str(),
                "/api/nvidia-nim/key" | "/api/mobile-connect/tailscale/serve"
            ))
        && !(request.method == "DELETE"
            && request.path == super::preference_media::AVATAR_UPLOAD_PATH)
        && !(request.method == "POST"
            && matches!(
                request.path.as_str(),
                "/api/models"
                    | "/api/session-store/reset"
                    | "/api/session-store/prune-transcript-noise"
                    | "/api/logs/purge"
                    | "/api/kill-all"
                    | "/api/agent-log/open"
                    | "/api/approval/enable"
                    | "/api/approval/disable"
                    | "/api/approval/dismiss"
                    | "/api/nvidia-nim/test"
                    | "/api/nvidia-nim/key"
                    | "/api/mobile-connect/tailscale/serve"
                    | "/api/shutdown"
                    | "/api/notify-generate-topic"
                    | "/api/slash-commands"
                    | "/api/provider-distributions/check"
                    | "/api/provider-distributions/accept"
                    | "/api/provider-distributions/rollback"
            ))
        && !matches!(
            request.path.as_str(),
            "/api/auth/logout"
                | "/api/auth/revoke-all"
                | "/api/notify-generate-topic"
                | "/api/pick-directory"
                | "/api/pick-file"
        );
    if reads_body {
        request.body = match read_body(body, &request.path).await {
            Ok(v) => v,
            Err(_) if request.path.starts_with("/api/provider-icons/") => {
                return response(Response::error(400, "bad_request", "could not read image"));
            }
            Err(_) => return response(Response::error(400, "bad_request", "invalid json")),
        };
    }
    if super::router::needs_owned_request(&request.path) {
        let permit = match state.services.owned_request_permit() {
            Ok(permit) => permit,
            Err(_) => {
                return response(Response::error(
                    503,
                    "approval_unavailable",
                    "approval service is not accepting requests",
                ));
            }
        };
        let cancellation = permit.cancellation();
        let waiter = crate::proto::core::HttpWaitCancellation::default();
        let _wait_guard = HttpWaitGuard(waiter.clone());
        // The lifecycle owns the whole accepted operation, including every batch
        // item and its committed effects. A dropped socket waits for nothing and
        // cannot cancel this task. Captured services hold only weak task handles.
        let completion = permit.start(async move {
            let dispatch = state
                .services
                .handle_owned_async_at(&request, captured_at, &cancellation, &waiter)
                .await;
            match state.sink.apply(dispatch.effects).await {
                Ok(()) => dispatch.response,
                Err(failure) => effect_failure(dispatch.response, failure),
            }
        });
        return response(match completion.wait().await {
            Ok(result) => result,
            Err(_) => Response::error(500, "operation_failed", "Hub operation did not complete"),
        });
    }
    let dispatch = state.services.handle_async_at(&request, captured_at).await;
    match state.sink.apply(dispatch.effects).await {
        Ok(()) => response(dispatch.response),
        Err(failure) => response(effect_failure(dispatch.response, failure)),
    }
}
fn effect_failure(original: Response, failure: CoreEffectFailure) -> Response {
    let mut error = serde_json::json!({"ok":false,"error":"core_effect_failed","detail":"request side effects could not be completed","failed_step":failure.index});
    // A persisted token rotation must not strand its authorized caller if a
    // later socket/persistence effect fails. Preserve the newly issued address,
    // report the failed batch, and never retry already-applied effects.
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&original.body) {
        for key in ["token", "hub_url"] {
            if let Some(value) = value.get(key) {
                error[key] = value.clone();
            }
        }
    }
    Response::json(500, &error).no_store()
}
fn decode_path(path: &str) -> Option<String> {
    let mut out = Vec::new();
    let b = path.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let a = (*b.get(i + 1)? as char).to_digit(16)?;
            let c = (*b.get(i + 2)? as char).to_digit(16)?;
            out.push((a * 16 + c) as u8);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    Some(String::from_utf8_lossy(&out).into_owned())
}
fn body_policy(path: &str) -> (usize, bool) {
    if path.starts_with("/api/provider-icons/") {
        return (crate::application::provider_assets::ICON_LIMIT, false);
    }
    match path {
        "/api/logs/legacy-notice" => (1 << 16, true),
        super::jev_routes::PATH => (8192, false),
        "/api/attach" => (11 * 1024 * 1024, false),
        "/api/voice/transcribe" => (25 * 1024 * 1024, false),
        "/api/memo-images" | "/api/memo-images/" => (10 * 1024 * 1024, false),
        "/api/user-prefs/avatar" => (5 * 1024 * 1024, false),
        "/api/user-prefs/notify-sound-custom" => (2 * 1024 * 1024, false),
        "/api/files-save" => (2 * 1024 * 1024, false),
        _ => (http::JSON_BODY_LIMIT, true),
    }
}
async fn read_body(mut body: Body, path: &str) -> io::Result<Vec<u8>> {
    let (limit, first_value) = body_policy(path);
    let mut bytes = Vec::new();
    while let Some(frame) = std::future::poll_fn(|cx| Pin::new(&mut body).poll_frame(cx)).await {
        let frame = frame.map_err(io::Error::other)?;
        if let Ok(chunk) = frame.into_data() {
            let remaining = (limit + 1).saturating_sub(bytes.len());
            bytes.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
            if first_value {
                // Only a syntax probe to stop reading after the first complete JSON
                // object; typed Go decoding and Unicode normalization remain shared.
                let repaired = String::from_utf8_lossy(&bytes);
                let mut decoder = serde_json::Deserializer::from_str(&repaired);
                use serde::Deserialize;
                if let Ok(value) = Box::<serde_json::value::RawValue>::deserialize(&mut decoder)
                    && matches!(value.get().as_bytes().first(), Some(b'{') | Some(b'n'))
                {
                    break;
                }
            }
            if bytes.len() > limit {
                break;
            }
        }
    }
    Ok(bytes)
}
pub fn response(mut value: Response) -> AxumResponse {
    let body = match value.file.take() {
        Some(file) => Body::new(FileBody::new(*file)),
        None => Body::from(value.body),
    };
    let mut response = AxumResponse::new(body);
    *response.status_mut() = axum::http::StatusCode::from_u16(value.status)
        .unwrap_or(axum::http::StatusCode::INTERNAL_SERVER_ERROR);
    for (name, value) in value.headers {
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(&value),
        ) {
            response.headers_mut().insert(name, value);
        }
    }
    for cookie in value.cookies {
        if let Ok(value) = HeaderValue::from_str(&cookie) {
            response
                .headers_mut()
                .append(axum::http::header::SET_COOKIE, value);
        }
    }
    response
}
enum Segment {
    Bytes(Vec<u8>),
    File { offset: u64, len: u64 },
}
type PendingFileRead = tokio::task::JoinHandle<io::Result<(std::fs::File, Vec<u8>)>>;
struct FileBody {
    file: Option<std::fs::File>,
    segments: std::collections::VecDeque<Segment>,
    remaining: u64,
    pending: Option<PendingFileRead>,
    active: Option<(u64, u64)>,
}
impl FileBody {
    fn new(spec: http::StreamedFile) -> Self {
        let mut segments = std::collections::VecDeque::new();
        let mut remaining = 0u64;
        for range in spec.ranges {
            remaining = remaining
                .saturating_add(range.prefix.len() as u64)
                .saturating_add(range.len);
            if !range.prefix.is_empty() {
                segments.push_back(Segment::Bytes(range.prefix));
            }
            if range.len > 0 {
                segments.push_back(Segment::File {
                    offset: range.offset,
                    len: range.len,
                });
            }
        }
        if !spec.suffix.is_empty() {
            remaining = remaining.saturating_add(spec.suffix.len() as u64);
            segments.push_back(Segment::Bytes(spec.suffix));
        }
        Self {
            file: Some(spec.file),
            segments,
            remaining,
            pending: None,
            active: None,
        }
    }
}
impl HttpBody for FileBody {
    type Data = Bytes;
    type Error = io::Error;
    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        if self.remaining == 0 {
            return Poll::Ready(None);
        }
        if self.pending.is_none() {
            match self.segments.pop_front() {
                Some(Segment::Bytes(bytes)) => {
                    self.remaining -= bytes.len() as u64;
                    return Poll::Ready(Some(Ok(http_body::Frame::data(Bytes::from(bytes)))));
                }
                Some(Segment::File { offset, len }) => {
                    let Some(mut file) = self.file.take() else {
                        self.remaining = 0;
                        return Poll::Ready(Some(Err(io::Error::other(
                            "file stream lost its descriptor",
                        ))));
                    };
                    let count = len.min(64 * 1024) as usize;
                    self.active = Some((offset, len));
                    self.pending = Some(tokio::task::spawn_blocking(move || {
                        file.seek(SeekFrom::Start(offset))?;
                        let mut bytes = vec![0; count];
                        let n = file.read(&mut bytes)?;
                        bytes.truncate(n);
                        Ok((file, bytes))
                    }));
                }
                None => {
                    self.remaining = 0;
                    return Poll::Ready(Some(Err(io::Error::other("file stream length mismatch"))));
                }
            }
        }
        let result = match Pin::new(self.pending.as_mut().unwrap()).poll(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(result) => result,
        };
        self.pending = None;
        match result {
            Ok(Ok((file, bytes))) => {
                self.file = Some(file);
                if bytes.is_empty() {
                    self.remaining = 0;
                    return Poll::Ready(Some(Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "file changed during download",
                    ))));
                }
                let (offset, len) = self.active.take().unwrap();
                let count = bytes.len() as u64;
                self.remaining -= count;
                if count < len {
                    self.segments.push_front(Segment::File {
                        offset: offset + count,
                        len: len - count,
                    });
                }
                Poll::Ready(Some(Ok(http_body::Frame::data(Bytes::from(bytes)))))
            }
            Ok(Err(error)) => {
                self.remaining = 0;
                Poll::Ready(Some(Err(error)))
            }
            Err(error) => {
                self.remaining = 0;
                Poll::Ready(Some(Err(io::Error::other(error))))
            }
        }
    }
    fn is_end_stream(&self) -> bool {
        self.remaining == 0
    }
    fn size_hint(&self) -> http_body::SizeHint {
        http_body::SizeHint::with_exact(self.remaining)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::{Config, ConfigStore, RuntimePaths},
        proto::core::{CoreEffects, CoreFuture},
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    #[tokio::test]
    async fn managed_voice_failure_returns_before_incomplete_audio_upload() {
        struct FailingManaged;
        impl crate::hub::voice_routes::ManagedWhisper for FailingManaged {
            fn ensure<'a>(
                &'a self,
                _: crate::config::VoiceWhisperConfig,
                _: &'a Cancellation,
            ) -> CoreFuture<'a, Result<crate::config::VoiceWhisperConfig, Response>> {
                Box::pin(async {
                    Err(Response::error(
                        503,
                        "synthetic_managed_failure",
                        "synthetic unavailable",
                    ))
                })
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let installed = tempfile::tempdir().unwrap();
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let paths = RuntimePaths::trial(dir.path(), port, installed.path()).unwrap();
        let mut cfg = Config {
            token: "synthetic-voice".into(),
            ..Default::default()
        };
        cfg.voice.whisper.managed = true;
        let config = Arc::new(ConfigStore::new(paths.clone(), cfg).unwrap());
        let tasks = super::super::task_owner::HubTaskOwner::new(tokio::runtime::Handle::current());
        let services = Arc::new(
            ServiceRouter::isolated_at_port(config.clone(), paths, port)
                .unwrap()
                .with_task_owner(tasks.handle())
                .with_voice(Arc::new(super::super::voice_routes::VoiceHttp::new(
                    config,
                    Arc::new(FailingManaged),
                ))),
        );
        let cancel = Cancellation::default();
        let server = tokio::spawn(serve(
            listener,
            services,
            Arc::new(EmptyEffects),
            cancel.clone(),
        ));
        let mut socket = tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))
            .await
            .unwrap();
        socket.write_all(format!("POST /api/voice/transcribe?token=synthetic-voice HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 1000000\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
        let mut bytes = vec![0; 8192];
        let count =
            tokio::time::timeout(std::time::Duration::from_secs(3), socket.read(&mut bytes))
                .await
                .unwrap()
                .unwrap();
        let response = String::from_utf8_lossy(&bytes[..count]);
        assert!(response.starts_with("HTTP/1.1 503"), "{response}");
        assert!(response.contains("synthetic_managed_failure"), "{response}");
        drop(socket);
        cancel.cancel();
        tokio::time::timeout(std::time::Duration::from_secs(3), server)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(tasks.snapshot().requests.running, 0);
    }
    struct EmptyEffects;
    impl CoreEffectSink for EmptyEffects {
        fn apply<'a>(
            &'a self,
            effects: CoreEffects,
        ) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
            Box::pin(async move {
                assert!(
                    effects.0.is_empty(),
                    "fixture must not acknowledge actual core effects"
                );
                Ok(())
            })
        }
    }
    async fn fixture() -> (
        tempfile::TempDir,
        u16,
        Cancellation,
        tokio::task::JoinHandle<io::Result<()>>,
    ) {
        fixture_with_routines(false).await
    }
    async fn fixture_with_routines(
        routines: bool,
    ) -> (
        tempfile::TempDir,
        u16,
        Cancellation,
        tokio::task::JoinHandle<io::Result<()>>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("trial");
        std::fs::create_dir(&root).unwrap();
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let paths = RuntimePaths::trial(&root, port, &dir.path().join("installed")).unwrap();
        let config = Arc::new(
            ConfigStore::new(
                paths.clone(),
                Config {
                    token: "synthetic-http".into(),
                    ..Default::default()
                },
            )
            .unwrap(),
        );
        let mut services = ServiceRouter::isolated(config, paths);
        let request_owner = routines
            .then(|| crate::hub::task_owner::HubTaskOwner::new(tokio::runtime::Handle::current()));
        if let Some(owner) = &request_owner {
            services = services.with_task_owner(owner.handle());
        }
        if routines {
            let store = Arc::new(crate::routine::store::RoutineStore::open(Arc::new(
                crate::files::safe_fs::Dir::open(&root).unwrap(),
            )));
            services = services.with_routines(Arc::new(super::super::routines::RoutineHttp {
                store,
                runner: None,
                home: dir.path().to_owned(),
            }));
        }
        let services = Arc::new(services);
        let cancel = Cancellation::default();
        let server_cancel = cancel.clone();
        let task = tokio::spawn(async move {
            let result = serve(listener, services, Arc::new(EmptyEffects), server_cancel).await;
            if let Some(owner) = request_owner {
                owner.stop_requests();
                owner.cancel_requests().unwrap();
                owner.drain_requests().await;
                owner.stop_effects().unwrap();
                owner.cancel_effects().unwrap();
                owner.drain_effects().await;
            }
            result
        });
        (dir, port, cancel, task)
    }
    async fn exchange(port: u16, raw: String) -> String {
        let mut stream = tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))
            .await
            .unwrap();
        stream.write_all(raw.as_bytes()).await.unwrap();
        let mut bytes = vec![];
        tokio::time::timeout(
            std::time::Duration::from_secs(3),
            stream.read_to_end(&mut bytes),
        )
        .await
        .unwrap()
        .unwrap();
        String::from_utf8(bytes).unwrap()
    }
    #[tokio::test]
    async fn real_http_auth_precedes_method_host_and_incomplete_body() {
        let (_dir, port, cancel, task) = fixture().await;
        let response=exchange(port,"DELETE /api/input-config HTTP/1.1\r\nHost: evil.invalid\r\nContent-Length: 1048576\r\nConnection: close\r\n\r\n".to_string()).await;
        assert!(response.starts_with("HTTP/1.1 401"), "{response}");
        assert!(response.contains("unauthorized"));
        let response=exchange(port,"DELETE /api/input-config?token=synthetic-http HTTP/1.1\r\nHost: evil.invalid\r\nConnection: close\r\n\r\n".to_string()).await;
        assert!(response.starts_with("HTTP/1.1 405"));
        cancel.cancel();
        task.await.unwrap().unwrap();
    }
    #[tokio::test]
    async fn real_http_settings_decode_one_value_without_waiting_for_trailing_body() {
        let (_dir, port, cancel, task) = fixture().await;
        let response=exchange(port,format!("POST /api/input-config?token=synthetic-http HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 1048576\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{{\"DEFERRED_ENTER_MS\":42,\"ignored\":true}}")).await;
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        let response=exchange(port,format!("GET /api/input-config?token=synthetic-http HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n")).await;
        assert!(response.contains("\"deferred_enter_ms\":42"), "{response}");
        cancel.cancel();
        task.await.unwrap().unwrap();
    }
    #[tokio::test]
    async fn real_http_slow_header_is_closed_at_ten_seconds() {
        let (_dir, port, cancel, task) = fixture().await;
        let mut stream = tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))
            .await
            .unwrap();
        let begin = std::time::Instant::now();
        stream.write_all(b"GET / HTTP/1.1\r\nHost:").await.unwrap();
        let mut bytes = vec![];
        tokio::time::timeout(
            std::time::Duration::from_secs(13),
            stream.read_to_end(&mut bytes),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(begin.elapsed() >= std::time::Duration::from_secs(9));
        assert!(begin.elapsed() < std::time::Duration::from_secs(13));
        cancel.cancel();
        task.await.unwrap().unwrap();
    }
    #[tokio::test]
    async fn streamed_multipart_uses_one_held_file_and_bounded_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data");
        let data: Vec<_> = (0..200000).map(|i| (i % 251) as u8).collect();
        std::fs::write(&path, &data).unwrap();
        let file = std::fs::File::open(&path).unwrap();
        let spec = http::StreamedFile {
            file,
            ranges: vec![
                http::StreamedRange {
                    prefix: b"first:".to_vec(),
                    offset: 4,
                    len: 70000,
                },
                http::StreamedRange {
                    prefix: b"second:".to_vec(),
                    offset: 100,
                    len: 3,
                },
            ],
            suffix: b"end".to_vec(),
        };
        let mut body = Body::new(FileBody::new(spec));
        let mut output = vec![];
        while let Some(frame) = std::future::poll_fn(|cx| Pin::new(&mut body).poll_frame(cx)).await
        {
            let chunk = frame.unwrap().into_data().unwrap();
            assert!(chunk.len() <= 64 * 1024);
            output.extend_from_slice(&chunk);
        }
        let mut expected = b"first:".to_vec();
        expected.extend_from_slice(&data[4..70004]);
        expected.extend_from_slice(b"second:");
        expected.extend_from_slice(&data[100..103]);
        expected.extend_from_slice(b"end");
        assert_eq!(output, expected);
    }
    #[test]
    fn partial_effect_failure_preserves_new_auth_address_without_success() {
        let original = Response::json(
            200,
            &serde_json::json!({"token":"new-synthetic","hub_url":"http://127.0.0.1:48889/?token=new-synthetic"}),
        );
        let failed = effect_failure(
            original,
            CoreEffectFailure {
                index: 2,
                error: crate::proto::core::SessionError::Shutdown,
            },
        );
        let v: serde_json::Value = serde_json::from_slice(&failed.body).unwrap();
        assert_eq!(failed.status, 500);
        assert_eq!(v["ok"], false);
        assert_eq!(v["failed_step"], 2);
        assert_eq!(v["token"], "new-synthetic");
    }
    #[tokio::test]
    async fn served_routine_crud_persists_and_delete_does_not_wait_for_a_body() {
        let (dir, port, cancel, server) = fixture_with_routines(true).await;
        let cwd = dir.path().join("project");
        std::fs::create_dir(&cwd).unwrap();
        let definition=serde_json::json!({"name":"synthetic routine","provider":"claude","cwd":cwd,"prompt":"owned fixture"}).to_string();
        let created=exchange(port,format!("POST /api/routines?token=synthetic-http HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",definition.len(),definition)).await;
        assert!(created.starts_with("HTTP/1.1 200"), "{created}");
        let body: serde_json::Value =
            serde_json::from_str(created.split_once("\r\n\r\n").unwrap().1).unwrap();
        let id = body["routine"]["id"].as_str().unwrap();
        let listed=exchange(port,format!("GET /api/routines?token=synthetic-http HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n")).await;
        assert!(listed.contains("synthetic routine"));
        let deleted=exchange(port,format!("DELETE /api/routines/{id}?token=synthetic-http HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 900000\r\nConnection: close\r\n\r\n")).await;
        assert!(deleted.starts_with("HTTP/1.1 200"), "{deleted}");
        let reopened = crate::routine::store::RoutineStore::open(Arc::new(
            crate::files::safe_fs::Dir::open(&dir.path().join("trial")).unwrap(),
        ));
        assert!(reopened.definitions().unwrap().is_empty());
        cancel.cancel();
        server.await.unwrap().unwrap();
    }
}
