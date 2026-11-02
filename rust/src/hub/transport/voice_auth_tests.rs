//! HTTP-N07: real transport/router authority, gated uploads and a loopback-only
//! recognizer. No actual provider, managed process, Hub listener or user data.
use super::*;
use crate::{
    config::{Config, ConfigStore, RuntimePaths, VoiceWhisperConfig},
    hub::{auth::PIN_COOKIE, task_owner::HubTaskOwner, voice_routes::ManagedWhisper},
    proto::core::{CoreEffects, CoreFuture},
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::{Notify, Semaphore, oneshot};

const TOKEN: &str = "synthetic-voice-authority";
const PIN: &str = "123456";
const AUDIO: &[u8] = b"synthetic voice payload";

fn seconds_now() -> i64 {
    crate::proto::time::Timestamp::now()
        .duration_since(crate::proto::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}
async fn bounded(future: impl Future<Output = ()>) {
    tokio::time::timeout(Duration::from_secs(20), future)
        .await
        .expect("synthetic voice scenario exceeded its 20-second outer deadline");
}

#[derive(Default)]
struct Managed {
    calls: AtomicUsize,
}
impl ManagedWhisper for Managed {
    fn ensure<'a>(
        &'a self,
        config: VoiceWhisperConfig,
        _: &'a Cancellation,
    ) -> CoreFuture<'a, Result<VoiceWhisperConfig, Response>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move { Ok(config) })
    }
}
struct NoEffects;
impl CoreEffectSink for NoEffects {
    fn apply<'a>(&'a self, _: CoreEffects) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
        panic!("voice transport must not dispatch unrelated core effects")
    }
}
struct Recognizer {
    calls: AtomicUsize,
    accepted: Notify,
    responses: Semaphore,
}
async fn recognize(
    State(fake): State<Arc<Recognizer>>,
    body: Bytes,
) -> axum::Json<serde_json::Value> {
    assert!(body.windows(AUDIO.len()).any(|part| part == AUDIO));
    fake.calls.fetch_add(1, Ordering::SeqCst);
    fake.accepted.notify_one();
    fake.responses.acquire().await.unwrap().forget();
    axum::Json(serde_json::json!({"text":"synthetic recognized text"}))
}
struct Fixture {
    services: Arc<ServiceRouter>,
    tasks: HubTaskOwner,
    managed: Arc<Managed>,
    recognizer: Arc<Recognizer>,
    stop: Cancellation,
    server: Option<tokio::task::JoinHandle<()>>,
    _reserved_port: tokio::net::TcpListener,
    _root: tempfile::TempDir,
    _installed: tempfile::TempDir,
}
impl Fixture {
    async fn new(block_response: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let installed = tempfile::tempdir().unwrap();
        let reserved = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = reserved.local_addr().unwrap().port();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let recognizer_port = listener.local_addr().unwrap().port();
        let recognizer = Arc::new(Recognizer {
            calls: AtomicUsize::new(0),
            accepted: Notify::new(),
            responses: Semaphore::new(if block_response { 0 } else { 32 }),
        });
        let stop = Cancellation::default();
        let stopped = stop.clone();
        let router = Router::new()
            .route("/recognize", axum::routing::post(recognize))
            .with_state(recognizer.clone());
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async move { stopped.cancelled().await })
                .await
                .unwrap();
        });
        let paths = RuntimePaths::trial(root.path(), port, installed.path()).unwrap();
        let mut config = Config {
            token: TOKEN.into(),
            remote_pin_hash: bcrypt::hash(PIN, 4).unwrap(),
            auth_cookie_secret: "synthetic-voice-pin-secret".into(),
            ..Default::default()
        };
        config.voice.whisper = VoiceWhisperConfig {
            managed: true,
            server_url: format!("http://127.0.0.1:{recognizer_port}"),
            request_path: "/recognize".into(),
            timeout_seconds: 15,
            ..Default::default()
        };
        let config = Arc::new(ConfigStore::new(paths.clone(), config).unwrap());
        let tasks = HubTaskOwner::new(tokio::runtime::Handle::current());
        let managed = Arc::new(Managed::default());
        let services = Arc::new(
            ServiceRouter::isolated_at_port(config.clone(), paths, port)
                .unwrap()
                .with_task_owner(tasks.handle())
                .with_voice(Arc::new(super::super::voice_routes::VoiceHttp::new(
                    config,
                    managed.clone(),
                ))),
        );
        Self {
            services,
            tasks,
            managed,
            recognizer,
            stop,
            server: Some(server),
            _reserved_port: reserved,
            _root: root,
            _installed: installed,
        }
    }
    fn request(&self, cookie: Option<&str>) -> Request {
        let token = self.services.config.snapshot().unwrap().config.token;
        let host = format!("127.0.0.1:{}", self.services.bound_port());
        let mut headers = vec![
            ("Origin".into(), format!("http://{host}")),
            ("User-Agent".into(), "Chrome/100 Windows".into()),
            ("X-Forwarded-For".into(), "203.0.113.19".into()),
        ];
        if let Some(cookie) = cookie {
            headers.push(("Cookie".into(), cookie.into()));
        }
        Request {
            method: "POST".into(),
            path: super::super::voice_routes::PATH.into(),
            query: format!("token={token}"),
            host,
            remote_addr: "127.0.0.1:49321".into(),
            headers,
            ..Default::default()
        }
    }
    fn login(&self, pin: &str, now: i64) -> String {
        let mut request = self.request(None);
        request.path = "/api/auth/login".into();
        request.body = serde_json::to_vec(&serde_json::json!({"pin":pin})).unwrap();
        let response = self.services.handle(&request, now).response;
        assert_eq!(response.status, 200);
        response
            .cookies
            .iter()
            .find(|value| value.starts_with(&format!("{PIN_COOKIE}=")))
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .into()
    }
    fn revoke(&self, revocation: Revocation, cookie: &str) {
        match revocation {
            Revocation::Logout | Revocation::PinChange => {
                let mut request = self.request(Some(cookie));
                request.path = if matches!(revocation, Revocation::Logout) {
                    "/api/auth/logout".into()
                } else {
                    request.body = br#"{"pin":"654321"}"#.to_vec();
                    "/api/auth/set-pin".into()
                };
                assert_eq!(
                    self.services
                        .handle(&request, seconds_now())
                        .response
                        .status,
                    200
                );
            }
            Revocation::TokenRotation => {
                // Isolate the token predicate from PIN nonce/secret rotation.
                let mut snapshot = self.services.config.snapshot().unwrap();
                snapshot.config.token = "synthetic-replacement-voice-token".into();
                self.services
                    .config
                    .persist(snapshot.revision, snapshot.config)
                    .unwrap();
            }
        }
    }
    fn upload(&self, request: &Request) -> Upload {
        let (polled, started) = oneshot::channel();
        let (release, audio) = oneshot::channel();
        let polls = Arc::new(AtomicUsize::new(0));
        let body = Body::new(UploadBody {
            polled: Some(polled),
            polls: polls.clone(),
            audio,
            finished: false,
        });
        let mut raw = axum::http::Request::builder()
            .method("POST")
            .uri(format!("{}?{}", request.path, request.query))
            .header("Host", &request.host);
        for (name, value) in &request.headers {
            raw = raw.header(name, value);
        }
        let state = TransportState {
            services: self.services.clone(),
            sink: Arc::new(NoEffects),
            websockets: None,
        };
        Upload {
            started,
            polls,
            release: Some(release),
            task: Some(tokio::spawn(handle(
                State(state),
                ConnectInfo(request.remote_addr.parse().unwrap()),
                raw.body(body).unwrap(),
            ))),
        }
    }
    async fn close(mut self) {
        self.tasks.stop_requests();
        assert_eq!(self.tasks.drain_requests().await.running, 0);
        self.tasks.stop_effects().unwrap();
        assert_eq!(self.tasks.drain_effects().await.running, 0);
        self.stop.cancel();
        tokio::time::timeout(Duration::from_secs(3), self.server.as_mut().unwrap())
            .await
            .unwrap()
            .unwrap();
        self.server.take();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(server) = &self.server {
            server.abort();
        }
    }
}
struct UploadBody {
    polled: Option<oneshot::Sender<()>>,
    polls: Arc<AtomicUsize>,
    audio: oneshot::Receiver<Vec<u8>>,
    finished: bool,
}
impl HttpBody for UploadBody {
    type Data = Bytes;
    type Error = io::Error;
    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Bytes>, io::Error>>> {
        self.polls.fetch_add(1, Ordering::SeqCst);
        if self.finished {
            return Poll::Ready(None);
        }
        if let Some(polled) = self.polled.take() {
            let _ = polled.send(());
        }
        match Pin::new(&mut self.audio).poll(cx) {
            Poll::Ready(audio) => {
                self.finished = true;
                Poll::Ready(Some(
                    audio
                        .map(|audio| http_body::Frame::data(Bytes::from(audio)))
                        .map_err(|_| io::Error::other("synthetic upload discarded")),
                ))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}
struct Upload {
    started: oneshot::Receiver<()>,
    polls: Arc<AtomicUsize>,
    release: Option<oneshot::Sender<Vec<u8>>>,
    task: Option<tokio::task::JoinHandle<AxumResponse>>,
}
impl Upload {
    async fn waiting_for_audio(&mut self) {
        tokio::time::timeout(Duration::from_secs(5), &mut self.started)
            .await
            .expect("transport never reached the gated audio body")
            .expect("transport rejected before reaching the audio body");
    }
    fn finish_audio(&mut self) {
        self.release.take().unwrap().send(AUDIO.to_vec()).unwrap();
    }
    async fn finish(mut self) -> (u16, serde_json::Value) {
        let response = tokio::time::timeout(Duration::from_secs(5), self.task.as_mut().unwrap())
            .await
            .expect("voice response did not finish")
            .unwrap();
        self.task.take();
        let status = response.status().as_u16();
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        (status, serde_json::from_slice(&body).unwrap())
    }
}
impl Drop for Upload {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}
#[derive(Clone, Copy, Debug)]
enum Revocation {
    Logout,
    PinChange,
    TokenRotation,
}
async fn rejected_after_upload(revocation: Revocation) {
    let f = Fixture::new(false).await;
    let cookie = f.login(PIN, seconds_now());
    let request = f.request(Some(&cookie));
    let mut upload = f.upload(&request);
    upload.waiting_for_audio().await;
    assert_eq!(f.managed.calls.load(Ordering::SeqCst), 1);
    assert_eq!(f.recognizer.calls.load(Ordering::SeqCst), 0);
    f.revoke(revocation, &cookie);
    let before = f.services.config.snapshot().unwrap();
    let expected = f.services.preflight(&request, seconds_now()).unwrap_err();
    assert_eq!(expected.status, 401);
    let expected_body: serde_json::Value = serde_json::from_slice(&expected.body).unwrap();
    upload.finish_audio();
    let (status, body) = upload.finish().await;
    assert_eq!(status, expected.status, "{revocation:?}");
    assert_eq!(body["error"], expected_body["error"], "{revocation:?}");
    assert_eq!(f.recognizer.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        f.services.config.snapshot().unwrap().revision,
        before.revision
    );

    let pin = if matches!(revocation, Revocation::PinChange) {
        "654321"
    } else {
        PIN
    };
    let fresh = f.login(pin, seconds_now());
    let mut control = f.upload(&f.request(Some(&fresh)));
    control.waiting_for_audio().await;
    control.finish_audio();
    let (status, body) = control.finish().await;
    assert_eq!(status, 200);
    assert_eq!(body["text"], "synthetic recognized text");
    assert_eq!(f.recognizer.calls.load(Ordering::SeqCst), 1);
    f.close().await;
}
#[tokio::test]
async fn voice_upload_logout_rejects_before_recognition_and_relogin_succeeds() {
    bounded(rejected_after_upload(Revocation::Logout)).await;
}
#[tokio::test]
async fn voice_upload_pin_change_rejects_before_recognition_and_new_pin_succeeds() {
    bounded(rejected_after_upload(Revocation::PinChange)).await;
}
#[tokio::test]
async fn voice_upload_token_rotation_rejects_before_recognition_and_new_authority_succeeds() {
    bounded(rejected_after_upload(Revocation::TokenRotation)).await;
}
#[tokio::test]
async fn voice_upload_pin_expiry_requires_fresh_native_time_before_recognition() {
    bounded(async {
        let f = Fixture::new(false).await;
        // Use the real PIN issuer's existing time input. The fixed 12-hour PIN
        // contract yields a cookie expiring ten real seconds from now, without
        // changing a clock or adding a test-only authentication implementation.
        let cookie = f.login(PIN, seconds_now() - 12 * 60 * 60 + 10);
        let expiry: i64 = cookie
            .split_once('=')
            .unwrap()
            .1
            .split('.')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        let request = f.request(Some(&cookie));
        let mut upload = f.upload(&request);
        upload.waiting_for_audio().await;
        let admitted_at = seconds_now();
        assert!(admitted_at < expiry);
        assert!(f.services.preflight(&request, admitted_at).is_ok());
        while seconds_now() < expiry {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(seconds_now() >= expiry);
        // Do not call authentication after reaching expiry: another check could
        // prune the nonce and make a stale captured-time implementation appear
        // correct. Only the transport's fresh-time recheck may reject this work.
        upload.finish_audio();
        let (status, body) = upload.finish().await;
        assert_eq!(status, 401);
        assert_eq!(body["error"], "pin_required");
        assert_eq!(f.recognizer.calls.load(Ordering::SeqCst), 0);
        f.close().await;
    })
    .await;
}
#[tokio::test]
async fn voice_recognition_already_accepted_completes_after_authority_is_revoked() {
    bounded(async {
        for revocation in [
            Revocation::Logout,
            Revocation::PinChange,
            Revocation::TokenRotation,
        ] {
            let f = Fixture::new(true).await;
            let cookie = f.login(PIN, seconds_now());
            let request = f.request(Some(&cookie));
            let mut upload = f.upload(&request);
            upload.waiting_for_audio().await;
            upload.finish_audio();
            tokio::time::timeout(Duration::from_secs(5), f.recognizer.accepted.notified())
                .await
                .unwrap();
            assert_eq!(f.recognizer.calls.load(Ordering::SeqCst), 1);
            f.revoke(revocation, &cookie);
            assert_eq!(
                f.services
                    .preflight(&request, seconds_now())
                    .unwrap_err()
                    .status,
                401
            );
            f.recognizer.responses.add_permits(1);
            let (status, body) = upload.finish().await;
            assert_eq!(status, 200, "{revocation:?}");
            assert_eq!(body["text"], "synthetic recognized text");
            assert_eq!(f.recognizer.calls.load(Ordering::SeqCst), 1);
            f.close().await;
        }
    })
    .await;
}
#[tokio::test]
async fn voice_unauthorized_headers_reject_without_polling_upload_or_preparing_service() {
    bounded(async {
        let f = Fixture::new(false).await;
        let mut request = f.request(None);
        request.query = "token=wrong-synthetic-token".into();
        let upload = f.upload(&request);
        let polls = upload.polls.clone();
        let (status, body) = upload.finish().await;
        assert_eq!(status, 401);
        assert_eq!(body["error"], "unauthorized");
        assert_eq!(polls.load(Ordering::SeqCst), 0);
        assert_eq!(f.managed.calls.load(Ordering::SeqCst), 0);
        assert_eq!(f.recognizer.calls.load(Ordering::SeqCst), 0);
        f.close().await;
    })
    .await;
}
