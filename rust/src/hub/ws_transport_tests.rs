//! Actual HTTP upgrade and RFC6455 frames over task-owned loopback sockets.
//! Session state uses the real engine; only provider launch and service events
//! unrelated to these tests have explicit synthetic transports.
use super::*;
use crate::{
    config::{ConfigStore, RuntimePaths},
    hub::{
        sockets::{EffectDriver, OrderedEventObserver},
        transport,
    },
    terminal::{
        events::CoreEventBus,
        journal::{JournalOptions, SessionJournal},
        session::{EngineOptions, SessionEngine},
    },
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
struct NoSpawn;
impl WrappedSessionSpawner for NoSpawn {
    fn spawn_and_wait<'a>(
        &'a self,
        _: WrappedSpawnSpec,
        _: Duration,
        _: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome> {
        Box::pin(async { SpawnWaitOutcome::Failed("synthetic fixture has no launcher".into()) })
    }
}
struct SyntheticObserver;
impl OrderedEventObserver for SyntheticObserver {
    fn observe<'a>(&'a self, _: &'a CoreEvent) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async { Ok(()) })
    }
}
struct Fixture {
    _root: tempfile::TempDir,
    port: u16,
    core: Arc<dyn SessionCore>,
    sockets: Arc<SocketRegistry>,
    effects: Arc<EffectDriver>,
    cancel: Cancellation,
    server: Option<tokio::task::JoinHandle<std::io::Result<()>>>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
impl Fixture {
    async fn stop(mut self) {
        self.cancel.cancel();
        self.server.take().unwrap().await.unwrap().unwrap();
    }
}
async fn fixture() -> Fixture {
    fixture_with_observer(Arc::new(SyntheticObserver)).await
}
async fn fixture_with_observer(observer: Arc<dyn OrderedEventObserver>) -> Fixture {
    fixture_with_preparation(observer, None).await
}
async fn fixture_with_preparation(
    observer: Arc<dyn OrderedEventObserver>,
    preparation: Option<Arc<dyn RegistrationPreparation>>,
) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let trial = root.path().join("trial");
    std::fs::create_dir(&trial).unwrap();
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let paths = RuntimePaths::trial(&trial, port, &root.path().join("installed")).unwrap();
    let config = Arc::new(
        ConfigStore::new(
            paths.clone(),
            Config {
                token: "synthetic-websocket-token".into(),
                ..Default::default()
            },
        )
        .unwrap(),
    );
    let journal = Arc::new(SessionJournal::new(
        paths.clone(),
        None,
        JournalOptions::default(),
    ));
    let bus = CoreEventBus::new(128).unwrap();
    let sockets = Arc::new(SocketRegistry::default());
    let effects = Arc::new(EffectDriver::new(
        sockets.clone(),
        journal.clone(),
        Arc::new(bus.clone()),
        observer,
    ));
    let core: Arc<dyn SessionCore> = Arc::new(SessionEngine::new(
        EngineOptions {
            hub_instance: "synthetic-ws-hub".into(),
            submit_timing: SubmitTiming {
                idle_settle: Duration::from_millis(1),
                minimum: Duration::from_millis(1),
                slow_minimum: Duration::from_millis(1),
                maximum: Duration::from_millis(10),
                poll: Duration::from_millis(1),
                confirm_window: Duration::from_millis(2),
            },
            warning: Arc::new(|_, _| {}),
            ..Default::default()
        },
        journal,
        sockets.clone(),
        effects.clone(),
        Arc::new(NoSpawn),
        bus,
    ));
    sockets.bind_core(Arc::downgrade(&core)).unwrap();
    let services = Arc::new(ServiceRouter::new(config, paths, core.clone(), port).unwrap());
    let cancel = Cancellation::default();
    let mut websocket = WebSocketService::new(
        services.clone(),
        core.clone(),
        sockets.clone(),
        effects.clone(),
        cancel.clone(),
        Arc::new(|_, _| {}),
    );
    if let Some(preparation) = preparation {
        websocket = websocket.with_registration_preparation(preparation);
    }
    let websocket = Arc::new(websocket);
    let server = tokio::spawn(transport::serve_with_websockets(
        listener,
        services,
        effects.clone(),
        websocket,
        cancel.clone(),
    ));
    Fixture {
        _root: root,
        port,
        core,
        sockets,
        effects,
        cancel,
        server: Some(server),
    }
}
struct Client(tokio::net::TcpStream);
impl Client {
    async fn connect(port: u16, extra_headers: &str) -> (Self, String) {
        let mut stream = tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))
            .await
            .unwrap();
        // A deterministic 16-byte synthetic nonce is protocol framing only.
        use base64::Engine as _;
        let key = base64::engine::general_purpose::STANDARD.encode([42; 16]);
        let request = format!(
            "GET /ws HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: {key}\r\n{extra_headers}\r\n"
        );
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut response = Vec::new();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                response.push(stream.read_u8().await.unwrap());
                if response.ends_with(b"\r\n\r\n") {
                    break;
                }
                assert!(response.len() < 16384);
            }
        })
        .await
        .unwrap();
        (Self(stream), String::from_utf8(response).unwrap())
    }
    async fn json(&mut self, value: serde_json::Value) {
        self.raw(&serde_json::to_vec(&value).unwrap()).await;
    }
    async fn raw(&mut self, payload: &[u8]) {
        let mut frame = vec![0x81];
        if payload.len() < 126 {
            frame.push(0x80 | payload.len() as u8);
        } else if payload.len() <= u16::MAX as usize {
            frame.push(0x80 | 126);
            frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        } else {
            frame.push(0x80 | 127);
            frame.extend_from_slice(&(payload.len() as u64).to_be_bytes());
        }
        let mask = [1, 2, 3, 4];
        frame.extend_from_slice(&mask);
        frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        self.0.write_all(&frame).await.unwrap();
    }
    async fn frame(&mut self) -> Option<serde_json::Value> {
        tokio::time::timeout(Duration::from_secs(2), async {
            let first = match self.0.read_u8().await {
                Ok(v) => v,
                Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return None,
                Err(error) => panic!("frame header: {error}"),
            };
            let second = self.0.read_u8().await.unwrap();
            assert_eq!(second & 0x80, 0, "server frames are never masked");
            let len = match second & 0x7f {
                126 => self.0.read_u16().await.unwrap() as usize,
                127 => self.0.read_u64().await.unwrap() as usize,
                n => n as usize,
            };
            assert!(len <= MAX_PAYLOAD_BYTES);
            let mut payload = vec![0; len];
            self.0.read_exact(&mut payload).await.unwrap();
            if first & 0x0f == 8 {
                return None;
            }
            assert_eq!(first & 0x0f, 1);
            Some(serde_json::from_slice(&payload).unwrap())
        })
        .await
        .unwrap()
    }
    async fn until(&mut self, kind: &str) -> serde_json::Value {
        for _ in 0..30 {
            let value = self
                .frame()
                .await
                .expect("connection closed before expected frame");
            if value["type"] == kind {
                return value;
            }
        }
        panic!("frame {kind} never arrived")
    }
}
#[tokio::test]
async fn loopback_first_frame_auth_real_core_output_input_and_shutdown() {
    let fixture = fixture().await;
    let (mut ui, status) = Client::connect(fixture.port, "").await;
    assert!(status.starts_with("HTTP/1.1 101"), "{status}");
    ui.json(serde_json::json!({"role":"ui","type":"anything","token":"synthetic-websocket-token","cols":120,"rows":40})).await;
    assert_eq!(ui.frame().await.unwrap()["type"], "snapshot");
    assert_eq!(ui.frame().await.unwrap()["type"], "approval_snapshot");
    let (mut wrapper, status) = Client::connect(fixture.port, "").await;
    assert!(status.starts_with("HTTP/1.1 101"));
    wrapper.json(serde_json::json!({"type":"register","provider":"shell","cwd":fixture._root.path(),"pid":123,"token":"synthetic-websocket-token"})).await;
    let ack = wrapper.frame().await.unwrap();
    assert_eq!(ack["type"], "registered");
    assert_eq!(ack["cols"], 120);
    assert_eq!(ack["rows"], 40);
    let id = ack["session_id"].as_i64().unwrap();
    wrapper
        .json(serde_json::json!({"type":"pty_data","session_id":999999,"data":[0,255,120]}))
        .await;
    let output = ui.until("pty_data").await;
    assert_eq!(
        output["session_id"], id,
        "wrapper-provided live ID must not redirect output"
    );
    assert_eq!(output["data"], "AP94");
    ui.json(serde_json::json!({"type":"pty_input","session_id":id,"text":"hello","data":"aWdub3JlZC1kYXRh"}))
        .await;
    let input = wrapper.until("pty_input").await;
    assert_eq!(input["data"], "aGVsbG8=");
    assert!(input["input_seq"].as_i64().unwrap() > 0);
    wrapper.json(serde_json::json!({"type":"pty_input_ack","session_id":999999,"input_seq":input["input_seq"]})).await;
    fixture
        .sockets
        .notify_hub_shutdown("synthetic_shutdown")
        .await
        .unwrap();
    assert_eq!(
        wrapper.until("hub_shutdown").await["reason"],
        "synthetic_shutdown"
    );
    assert!(fixture.core.details(LiveSessionId(id)).unwrap().connected);
    fixture.stop().await;
}
#[tokio::test]
async fn loopback_cookie_fallback_and_strict_whole_frame_json() {
    let fixture = fixture().await;
    let (mut ui, status) = Client::connect(
        fixture.port,
        "Cookie: MANY_AI_CLI_token=synthetic-websocket-token\r\n",
    )
    .await;
    assert!(status.starts_with("HTTP/1.1 101"));
    ui.json(serde_json::json!({"role":"ui","token":"stale-storage-token"}))
        .await;
    assert_eq!(ui.frame().await.unwrap()["type"], "snapshot");
    let (mut invalid, status) = Client::connect(fixture.port, "").await;
    assert!(status.starts_with("HTTP/1.1 101"));
    invalid
        .raw(br#"{"role":"ui","token":"synthetic-websocket-token"} {}"#)
        .await;
    assert!(invalid.frame().await.is_none());
    let (_, status) = Client::connect(fixture.port, "X-Forwarded-Proto: https\r\n").await;
    assert!(
        status.starts_with("HTTP/1.1 403"),
        "origin-less logically remote upgrade must fail: {status}"
    );
    fixture.stop().await;
}
#[tokio::test]
async fn loopback_reattach_closes_only_old_exact_connection() {
    let fixture = fixture().await;
    let (mut old, _) = Client::connect(fixture.port, "").await;
    let registration = serde_json::json!({"type":"register","provider":"shell","cwd":fixture._root.path(),"pid":123,"token":"synthetic-websocket-token"});
    old.json(registration.clone()).await;
    let ack = old.frame().await.unwrap();
    let id = ack["session_id"].as_i64().unwrap();
    let (mut replacement, _) = Client::connect(fixture.port, "").await;
    let mut reattach = registration;
    reattach["type"] = "reattach".into();
    reattach["session_id"] = id.into();
    reattach["started_at"] = ack["started_at"].clone();
    reattach["cols"] = 100.into();
    reattach["rows"] = 30.into();
    replacement.json(reattach).await;
    let reattached = replacement.frame().await.unwrap();
    assert_eq!(
        reattached,
        serde_json::json!({"type":"reattach_ack","session_id":id,"token_statusbar":false})
    );
    assert!(old.frame().await.is_none());
    replacement
        .json(serde_json::json!({"type":"pty_data","data":"bmV3"}))
        .await;
    // A subsequent UI snapshot proves stale reader cleanup did not disconnect
    // the surviving replacement, without sleeping for a scheduling guess.
    let (mut ui, _) = Client::connect(fixture.port, "").await;
    ui.json(serde_json::json!({"role":"ui","token":"synthetic-websocket-token"}))
        .await;
    let snapshot = ui.frame().await.unwrap();
    assert_eq!(snapshot["sessions"].as_array().unwrap().len(), 1);
    assert_ne!(snapshot["sessions"][0]["state"], "disconnected");
    assert!(fixture.core.details(LiveSessionId(id)).unwrap().connected);
    fixture.stop().await;
}

struct GateObserver {
    reached: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
impl OrderedEventObserver for GateObserver {
    fn observe<'a>(&'a self, event: &'a CoreEvent) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            if matches!(event, CoreEvent::GitTurnCapture { ended_at: None, .. }) {
                self.reached.notify_one();
                self.release.notified().await;
            }
            Ok(())
        })
    }
}
#[tokio::test]
async fn revoke_waits_for_accepted_paste_and_enter_without_fifo_drain_deadlock() {
    let gate = Arc::new(GateObserver {
        reached: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    let fixture = fixture_with_observer(gate.clone()).await;
    let (mut wrapper, _) = Client::connect(fixture.port, "").await;
    wrapper.json(serde_json::json!({"type":"register","provider":"shell","cwd":fixture._root.path(),"pid":123,"token":"synthetic-websocket-token"})).await;
    let id = wrapper.frame().await.unwrap()["session_id"]
        .as_i64()
        .unwrap();
    let (mut ui, _) = Client::connect(fixture.port, "").await;
    ui.json(serde_json::json!({"role":"ui","token":"synthetic-websocket-token"}))
        .await;
    ui.until("approval_snapshot").await;
    ui.json(
        serde_json::json!({"type":"pty_input","session_id":id,"text":"\x1b[200~hello\x1b[201~\r"}),
    )
    .await;
    tokio::time::timeout(Duration::from_secs(2), gate.reached.notified())
        .await
        .unwrap();
    let revoke = tokio::spawn({
        let core = fixture.core.clone();
        let effects = fixture.effects.clone();
        async move { effects.apply(core.invalidate_all_ui()).await }
    });
    tokio::task::yield_now().await;
    assert!(
        !revoke.is_finished(),
        "revocation must drain accepted input before acknowledging"
    );
    gate.release.notify_one();
    let paste = wrapper.until("pty_input").await;
    assert_ne!(paste["data"], "DQ==");
    let enter = wrapper.until("pty_input").await;
    assert_eq!(enter["data"], "DQ==");
    tokio::time::timeout(Duration::from_secs(2), revoke)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(fixture.core.details(LiveSessionId(id)).unwrap().connected);
    fixture.stop().await;
}

#[tokio::test]
async fn loopback_reattach_rejection_and_declared_exit_keep_go_wire_shape() {
    let fixture = fixture().await;
    let (mut invalid, _) = Client::connect(fixture.port, "").await;
    invalid.json(serde_json::json!({"type":"reattach","session_id":-1,"token":"synthetic-websocket-token"})).await;
    let rejected = invalid.frame().await.unwrap();
    assert_eq!(rejected["type"], "reattach_reject");
    assert_eq!(rejected["reason"], "invalid session_id");
    assert!(rejected.get("session_id").is_none());
    assert!(invalid.frame().await.is_none());
    let (mut wrapper, _) = Client::connect(fixture.port, "").await;
    wrapper.json(serde_json::json!({"type":"register","provider":"shell","cwd":fixture._root.path(),"pid":123,"token":"synthetic-websocket-token"})).await;
    let id = wrapper.frame().await.unwrap()["session_id"]
        .as_i64()
        .unwrap();
    let mut events = fixture.core.subscribe();
    let waiter = HubShutdownCancellation::default();
    let code = i64::MAX;
    wrapper.json(serde_json::json!({"type":"session_end","session_id":999999,"state":"completed","exit_code":code,"reason":"synthetic reason","signal":"ignored Go field"})).await;
    let end = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match events.next(&waiter).await {
                CoreEventPoll::Event(event) => {
                    if let CoreEvent::Ended { binding, end } = event.event {
                        assert_eq!(binding.session, LiveSessionId(id));
                        break end;
                    }
                }
                _ => panic!("expected live event bus"),
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(end.declared_state, "completed");
    assert_eq!(end.exit_code, i64::MAX);
    assert_eq!(end.reason, "synthetic reason");
    fixture.stop().await;
}

#[tokio::test]
async fn real_reattach_ack_precedes_unacked_replay_and_disconnected_input_flush() {
    let fixture = fixture().await;
    let (mut wrapper, _) = Client::connect(fixture.port, "").await;
    let registration = serde_json::json!({"type":"register","provider":"shell","cwd":fixture._root.path(),"pid":123,"token":"synthetic-websocket-token"});
    wrapper.json(registration.clone()).await;
    let ack = wrapper.frame().await.unwrap();
    let id = ack["session_id"].as_i64().unwrap();
    // The fixed Go source replays inflight frames only after ACK capability
    // has been observed; a nonpositive ACK advertises it without removing data.
    wrapper
        .json(serde_json::json!({"type":"pty_input_ack","input_seq":0}))
        .await;
    let (mut ui, _) = Client::connect(fixture.port, "").await;
    ui.json(serde_json::json!({"role":"ui","token":"synthetic-websocket-token"}))
        .await;
    ui.until("approval_snapshot").await;
    ui.json(serde_json::json!({"type":"pty_input","session_id":id,"text":"first"}))
        .await;
    let first = wrapper.until("pty_input").await;
    drop(wrapper); // Deliberately lose ACK after receiving the original bytes.
    tokio::time::timeout(Duration::from_secs(2), async {
        while fixture.core.details(LiveSessionId(id)).unwrap().connected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    ui.json(serde_json::json!({"type":"pty_input","session_id":id,"text":"pending"}))
        .await;
    ui.until("input_deferred").await;
    let (mut next, _) = Client::connect(fixture.port, "").await;
    let mut reattach = registration;
    reattach["type"] = "reattach".into();
    reattach["session_id"] = id.into();
    reattach["started_at"] = ack["started_at"].clone();
    next.json(reattach).await;
    assert_eq!(next.frame().await.unwrap()["type"], "reattach_ack");
    let replay = next.until("pty_input").await;
    assert_eq!(replay["data"], first["data"]);
    assert_eq!(replay["input_seq"], first["input_seq"]);
    let pending = next.until("pty_input").await;
    assert_eq!(pending["data"], "cGVuZGluZw==");
    assert!(pending["input_seq"].as_i64().unwrap() > first["input_seq"].as_i64().unwrap());
    for frame in [&replay, &pending] {
        next.json(serde_json::json!({"type":"pty_input_ack","input_seq":frame["input_seq"]}))
            .await;
    }
    ui.json(serde_json::json!({"type":"pty_input","session_id":id,"text":"fresh"}))
        .await;
    assert_eq!(next.until("pty_input").await["data"], "ZnJlc2g=");
    fixture.stop().await;
}

struct PreparationBarrier {
    entered: tokio::sync::Notify,
    release: tokio::sync::Semaphore,
}
impl RegistrationPreparation for PreparationBarrier {
    fn prepare<'a>(&'a self) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            self.entered.notify_one();
            self.release
                .acquire()
                .await
                .map_err(|_| SessionError::Shutdown)?
                .forget();
            Ok(())
        })
    }
}
#[tokio::test]
async fn register_ack_and_registered_publication_wait_for_preparation_but_reattach_ack_does_not() {
    let barrier = Arc::new(PreparationBarrier {
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Semaphore::new(0),
    });
    let fixture =
        fixture_with_preparation(Arc::new(SyntheticObserver), Some(barrier.clone())).await;
    let (mut wrapper, _) = Client::connect(fixture.port, "").await;
    wrapper
        .json(serde_json::json!({"type":"register","provider":"shell",
        "cwd":fixture._root.path(),"pid":321,"token":"synthetic-websocket-token"}))
        .await;
    tokio::time::timeout(Duration::from_secs(2), barrier.entered.notified())
        .await
        .unwrap();
    // A readiness check consumes no bytes. No registration ACK or pending input
    // can pass the actual socket caller while its preparation barrier is held.
    let mut peek = [0u8; 1];
    assert!(
        tokio::time::timeout(Duration::from_millis(25), wrapper.0.peek(&mut peek))
            .await
            .is_err()
    );
    barrier.release.add_permits(1);
    let ack = wrapper.frame().await.unwrap();
    assert_eq!(ack["type"], "registered");
    let id = ack["session_id"].as_i64().unwrap();
    let (mut reattach, _) = Client::connect(fixture.port, "").await;
    reattach
        .json(serde_json::json!({"type":"reattach","session_id":id,
        "provider":"shell","cwd":fixture._root.path(),"pid":321,
        "token":"synthetic-websocket-token"}))
        .await;
    assert_eq!(reattach.frame().await.unwrap()["type"], "reattach_ack");
    tokio::time::timeout(Duration::from_secs(2), barrier.entered.notified())
        .await
        .unwrap();
    barrier.release.add_permits(1);
    drop(wrapper);
    drop(reattach);
    fixture.stop().await;
}
