//! Fixed Go wrapper/input/reconnect contracts with owned synthetic processes.
#[cfg(unix)]
use many_ai_cli::{
    process::pty::NativePtyFactory,
    wrapper::{input::write_all, transport::LoopbackConnector},
};
use many_ai_cli::{
    process::{
        Cancellation, ProcessPlan,
        pty::{PtyFactory, PtySession, PtySize},
    },
    proto::{
        Message,
        core::{CoreFuture, SPAWN_PROOF_ENV},
    },
    wrapper::{
        input::{InputStep, InputWatermarks, input_steps},
        runtime::{self, WrapperOptions},
        transport::{HubConnector, HubSocket},
    },
};
use std::{
    collections::{BTreeMap, VecDeque},
    io,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::mpsc;

fn plan(root: &std::path::Path) -> ProcessPlan {
    ProcessPlan {
        executable: "/bin/sh".into(),
        args: vec!["-c".into(), "printf synthetic".into()],
        cwd: root.into(),
        env: BTreeMap::new(),
        stdin: vec![],
        timeout: Duration::ZERO,
        output_cap: 1024 * 1024,
        pipe_drain_timeout: Duration::from_secs(1),
    }
}
#[test]
fn wrapper_input_watermark_distinguishes_received_from_written() {
    let mut marks = InputWatermarks::default();
    marks.processed(41);
    marks.received(42);
    marks.received(3);
    assert_eq!(marks.high_watermark(), 42);
    assert!(!marks.duplicate(42));
    assert!(marks.duplicate(41));
    assert!(!marks.duplicate(0));
    marks.processed(42);
    marks.processed(2);
    assert!(marks.duplicate(42));
}
#[test]
fn wrapper_input_preserves_utf8_chunks_clear_prefix_and_submit_delays() {
    let text = format!("{}日本語\r", "x".repeat(1023));
    let steps = input_steps("codex", text.as_bytes());
    let mut joined: Vec<u8> = Vec::new();
    for step in &steps {
        if let InputStep::Bytes(bytes) = step {
            assert!(std::str::from_utf8(bytes).is_ok());
            joined.extend(bytes);
        }
    }
    assert_eq!(joined, text.as_bytes());
    assert!(steps.contains(&InputStep::Pause(Duration::from_millis(180))));
    let clear = input_steps("cursor-agent", b"\x15hello\r");
    assert_eq!(clear[0], InputStep::Bytes(vec![0x15]));
    assert_eq!(clear[1], InputStep::Pause(Duration::from_millis(20)));
    assert_eq!(
        input_steps("claude", b"\r"),
        vec![InputStep::Bytes(vec![b'\r'])]
    );
    assert_eq!(
        input_steps("opencode", b"\r")[0],
        InputStep::Pause(Duration::from_millis(180))
    );
    assert!(
        !input_steps("claude", b"@mention\rtext\r")
            .contains(&InputStep::Pause(Duration::from_millis(150)))
    );
    assert!(
        input_steps("claude", b"@/synthetic/image.png\rtext\r")
            .contains(&InputStep::Pause(Duration::from_millis(150)))
    );
}

#[cfg(unix)]
#[tokio::test]
async fn wrapper_native_pty_observes_env_cwd_argv_and_idempotent_wait() {
    let root = tempfile::tempdir().unwrap();
    let mut launch = plan(root.path());
    launch.args = vec!["-c".into(), "printf '%s|%s|%s|%s' \"$PWD\" \"$SYNTHETIC_VALUE\" \"${MANY_AI_CLI_INTERNAL_SPAWN_PROOF-unset}\" \"$1\"".into(), "sh".into(), "quoted 日本語 ; literal".into()];
    launch
        .env
        .insert("SYNTHETIC_VALUE".into(), Some("owned-value".into()));
    launch.env.insert(
        SPAWN_PROOF_ENV.into(),
        Some("synthetic-not-provider-visible".into()),
    );
    let pty = NativePtyFactory
        .spawn(&launch, PtySize { cols: 93, rows: 31 })
        .unwrap();
    let mut all = vec![];
    let mut bytes = [0; 4096];
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let n = pty.read(&mut bytes).await.unwrap();
            if n == 0 {
                break;
            }
            all.extend_from_slice(&bytes[..n]);
        }
    })
    .await
    .unwrap();
    assert_eq!(
        String::from_utf8(all).unwrap(),
        format!(
            "{}|owned-value|unset|quoted 日本語 ; literal",
            root.path().canonicalize().unwrap().display()
        )
    );
    let first = pty.wait().await.unwrap();
    assert_eq!(first.code, 0);
    assert_eq!(first, pty.wait().await.unwrap());
    pty.close();
    pty.close();
    assert_eq!(first, pty.wait().await.unwrap());
}
#[cfg(unix)]
#[tokio::test]
async fn wrapper_native_pty_startup_size_resize_input_and_termination() {
    let root = tempfile::tempdir().unwrap();
    let mut launch = plan(root.path());
    launch.args = vec![
        "-c".into(),
        "stty -echo; stty size; read value; stty size; printf 'value:%s' \"$value\"".into(),
    ];
    let pty = NativePtyFactory
        .spawn(&launch, PtySize { cols: 93, rows: 31 })
        .unwrap();
    let mut bytes = [0; 4096];
    let n = tokio::time::timeout(Duration::from_secs(3), pty.read(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert!(String::from_utf8_lossy(&bytes[..n]).contains("31 93"));
    pty.resize(PtySize {
        cols: 110,
        rows: 42,
    })
    .unwrap();
    write_all(pty.as_ref(), b"synthetic\n").await.unwrap();
    let mut all = vec![];
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let n = pty.read(&mut bytes).await.unwrap();
            if n == 0 {
                break;
            }
            all.extend_from_slice(&bytes[..n]);
        }
    })
    .await
    .unwrap();
    let text = String::from_utf8_lossy(&all);
    assert!(text.contains("42 110"));
    assert!(text.contains("value:synthetic"));
    assert_eq!(pty.wait().await.unwrap().code, 0);
}
#[cfg(unix)]
#[tokio::test]
async fn wrapper_native_close_is_bounded_and_keeps_unrelated_child() {
    let root = tempfile::tempdir().unwrap();
    let mut launch = plan(root.path());
    launch.args = vec!["-c".into(), "trap '' TERM; sleep 30 & wait".into()];
    let owned = NativePtyFactory.spawn(&launch, PtySize::default()).unwrap();
    let other = NativePtyFactory.spawn(&launch, PtySize::default()).unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    owned.close();
    owned.close();
    let exit = tokio::time::timeout(Duration::from_secs(5), owned.wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(exit.code, -1);
    assert!(!exit.signal.is_empty());
    assert!(many_ai_cli::process::pid_alive(i64::from(other.pid())));
    other.close();
    tokio::time::timeout(Duration::from_secs(5), other.wait())
        .await
        .unwrap()
        .unwrap();
}

struct ScriptSocket {
    incoming: mpsc::Receiver<Message>,
    sent: mpsc::Sender<Message>,
}
impl HubSocket for ScriptSocket {
    fn send<'a>(&'a mut self, frame: &'a Message) -> CoreFuture<'a, io::Result<()>> {
        Box::pin(async move {
            self.sent
                .send(frame.clone())
                .await
                .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "test socket closed"))
        })
    }
    fn receive(&mut self) -> CoreFuture<'_, io::Result<Message>> {
        Box::pin(async move {
            self.incoming
                .recv()
                .await
                .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "test socket closed"))
        })
    }
}
struct ScriptConnector {
    sockets: Mutex<VecDeque<ScriptSocket>>,
    proofs: Arc<Mutex<Vec<Option<String>>>>,
    alive: bool,
}
impl HubConnector for ScriptConnector {
    fn connect<'a>(
        &'a self,
        proof: Option<&'a str>,
    ) -> CoreFuture<'a, io::Result<Box<dyn HubSocket>>> {
        Box::pin(async move {
            self.proofs.lock().unwrap().push(proof.map(str::to_owned));
            self.sockets
                .lock()
                .unwrap()
                .pop_front()
                .map(|s| Box::new(s) as Box<dyn HubSocket>)
                .ok_or_else(|| io::Error::new(io::ErrorKind::ConnectionRefused, "test unavailable"))
        })
    }
    fn probe(&self) -> CoreFuture<'_, bool> {
        Box::pin(async move { self.alive })
    }
}
fn socket_pair() -> (ScriptSocket, mpsc::Sender<Message>, mpsc::Receiver<Message>) {
    let (send, incoming) = mpsc::channel(64);
    let (sent, receive) = mpsc::channel(64);
    (ScriptSocket { incoming, sent }, send, receive)
}
async fn receive_type(rx: &mut mpsc::Receiver<Message>, wanted: &str) -> Message {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let frame = rx.recv().await.expect("wrapper frame");
            if frame.r#type == wanted {
                return frame;
            }
        }
    })
    .await
    .expect("expected wrapper frame timed out")
}
#[cfg(unix)]
#[tokio::test]
async fn wrapper_actual_runtime_register_input_ack_reconnect_replay_and_dismiss() {
    let root = tempfile::tempdir().unwrap();
    let mut launch = plan(root.path());
    launch.args = vec![
        "-c".into(),
        "stty -echo; printf ready; while IFS= read -r line; do printf 'seen:%s\n' \"$line\"; done"
            .into(),
    ];
    let (first, first_tx, mut first_rx) = socket_pair();
    let (second, second_tx, mut second_rx) = socket_pair();
    let proofs = Arc::new(Mutex::new(vec![]));
    let connector = Arc::new(ScriptConnector {
        sockets: Mutex::new(VecDeque::from([first, second])),
        proofs: proofs.clone(),
        alive: false,
    });
    let mut options = WrapperOptions::new(
        Message {
            provider: "synthetic".into(),
            token: "fixture-token".into(),
            ..Default::default()
        },
        launch,
    );
    options.initial_proof = Some("one-use-synthetic-proof".into());
    options.reconnect_interval = Duration::from_millis(10);
    options.reconnect_grace = Duration::from_secs(5);
    let runner = tokio::spawn(async move {
        runtime::run(
            options,
            connector.as_ref(),
            &NativePtyFactory,
            &Cancellation::default(),
            |_, _| Ok(()),
        )
        .await
    });
    assert_eq!(
        receive_type(&mut first_rx, "register").await.role,
        "wrapper"
    );
    first_tx
        .send(Message {
            r#type: "registered".into(),
            session_id: 7,
            cols: 80,
            rows: 24,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(receive_type(&mut first_rx, "pty_data").await.data, b"ready");
    first_tx
        .send(Message {
            r#type: "pty_input".into(),
            input_seq: 3,
            data: b"once\n".to_vec(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(
        receive_type(&mut first_rx, "pty_input_ack").await.input_seq,
        3
    );
    drop(first_tx);
    let reattach = receive_type(&mut second_rx, "reattach").await;
    assert_eq!(reattach.session_id, 7);
    assert_eq!(reattach.input_seq_high_watermark, 3);
    assert!(reattach.pty_bytes >= 5);
    assert!(!reattach.replay_b64.is_empty());
    // Go reattach permits input and resize before the acknowledgement.
    second_tx
        .send(Message {
            r#type: "pty_input".into(),
            input_seq: 3,
            data: b"once\n".to_vec(),
            ..Default::default()
        })
        .await
        .unwrap();
    second_tx
        .send(Message {
            r#type: "pty_resize".into(),
            cols: 100,
            rows: 40,
            ..Default::default()
        })
        .await
        .unwrap();
    second_tx
        .send(Message {
            r#type: "reattach_ack".into(),
            session_id: 9,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(
        receive_type(&mut second_rx, "pty_input_ack")
            .await
            .input_seq,
        3
    );
    second_tx
        .send(Message {
            r#type: "session_dismissed".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(6), runner)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(result.session_id, 9);
    assert_eq!(result.reconnects, 1);
    assert_eq!(
        *proofs.lock().unwrap(),
        vec![Some("one-use-synthetic-proof".into()), None]
    );
}

#[cfg(unix)]
#[tokio::test]
async fn wrapper_loopback_websocket_transmits_initial_proof_only_in_header() {
    use axum::{
        Router,
        extract::{State, WebSocketUpgrade, ws::Message as AxumMessage},
        http::HeaderMap,
        response::IntoResponse,
        routing::get,
    };
    #[derive(Clone)]
    struct StateData {
        proof: Arc<Mutex<Option<String>>>,
    }
    async fn upgrade(
        State(state): State<StateData>,
        headers: HeaderMap,
        ws: WebSocketUpgrade,
    ) -> impl IntoResponse {
        *state.proof.lock().unwrap() = headers
            .get("x-many-ai-internal-spawn-proof")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        ws.on_upgrade(move |mut socket| async move {
            let Some(Ok(AxumMessage::Text(text))) = socket.recv().await else {
                return;
            };
            assert!(!text.contains("only-in-header"));
            let frame: Message = serde_json::from_str(&text).unwrap();
            assert_eq!(frame.r#type, "register");
            let registered = Message {
                r#type: "registered".into(),
                session_id: 12,
                ..Default::default()
            };
            socket
                .send(AxumMessage::Text(
                    serde_json::to_string(&registered).unwrap().into(),
                ))
                .await
                .unwrap();
        })
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let proof = Arc::new(Mutex::new(None));
    let app = Router::new()
        .route("/ws", get(upgrade))
        .with_state(StateData {
            proof: proof.clone(),
        });
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let connector =
        LoopbackConnector::new(port, "fixture-token".into(), Duration::from_secs(2)).unwrap();
    let mut socket = connector.connect(Some("only-in-header")).await.unwrap();
    socket
        .send(&Message {
            r#type: "register".into(),
            token: "fixture-token".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(socket.receive().await.unwrap().session_id, 12);
    assert_eq!(*proof.lock().unwrap(), Some("only-in-header".into()));
    server.abort();
}

struct FakePty {
    writes: Mutex<Vec<u8>>,
    chunks: Mutex<VecDeque<Vec<u8>>>,
    fail_write: bool,
    fail_resize: bool,
    write_attempts: std::sync::atomic::AtomicUsize,
    closed: std::sync::atomic::AtomicBool,
    notify: tokio::sync::Notify,
}
impl FakePty {
    fn new(chunks: Vec<Vec<u8>>, fail_write: bool) -> Self {
        Self {
            writes: Mutex::new(vec![]),
            chunks: Mutex::new(chunks.into()),
            fail_write,
            fail_resize: false,
            write_attempts: std::sync::atomic::AtomicUsize::new(0),
            closed: std::sync::atomic::AtomicBool::new(false),
            notify: tokio::sync::Notify::new(),
        }
    }
}
impl PtySession for FakePty {
    fn pid(&self) -> u32 {
        42
    }
    fn read<'a>(&'a self, target: &'a mut [u8]) -> CoreFuture<'a, io::Result<usize>> {
        Box::pin(async move {
            if let Some(chunk) = self.chunks.lock().unwrap().pop_front() {
                let n = chunk.len().min(target.len());
                target[..n].copy_from_slice(&chunk[..n]);
                return Ok(n);
            }
            loop {
                let notified = self.notify.notified();
                if self.closed.load(std::sync::atomic::Ordering::Acquire) {
                    return Ok(0);
                }
                notified.await;
            }
        })
    }
    fn write<'a>(&'a self, bytes: &'a [u8]) -> CoreFuture<'a, io::Result<usize>> {
        Box::pin(async move {
            self.write_attempts
                .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            if self.fail_write {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "synthetic PTY failure",
                ));
            }
            let n = bytes.len().min(3);
            self.writes.lock().unwrap().extend_from_slice(&bytes[..n]);
            Ok(n)
        })
    }
    fn resize(&self, _: PtySize) -> io::Result<()> {
        if self.fail_resize {
            Err(io::Error::other("synthetic resize failure"))
        } else {
            Ok(())
        }
    }
    fn close(&self) {
        self.closed
            .store(true, std::sync::atomic::Ordering::Release);
        self.notify.notify_waiters();
    }
    fn wait(&self) -> CoreFuture<'_, io::Result<many_ai_cli::process::pty::PtyExit>> {
        Box::pin(async move {
            loop {
                let notified = self.notify.notified();
                if self.closed.load(std::sync::atomic::Ordering::Acquire) {
                    return Ok(many_ai_cli::process::pty::PtyExit {
                        code: 1,
                        signal: String::new(),
                        forced: false,
                    });
                }
                notified.await;
            }
        })
    }
}
struct FakeFactory(Arc<FakePty>);
impl PtyFactory for FakeFactory {
    fn spawn(&self, plan: &ProcessPlan, _: PtySize) -> io::Result<Arc<dyn PtySession>> {
        assert!(
            plan.env
                .get(std::ffi::OsStr::new(SPAWN_PROOF_ENV))
                .is_some_and(Option::is_none)
        );
        Ok(self.0.clone())
    }
}
#[tokio::test]
async fn wrapper_short_writes_complete_before_ack_and_duplicates_do_not_rewrite() {
    let root = tempfile::tempdir().unwrap();
    let pty = Arc::new(FakePty::new(vec![], false));
    let retained = pty.clone();
    let (socket, tx, mut rx) = socket_pair();
    let connector = ScriptConnector {
        sockets: Mutex::new(VecDeque::from([socket])),
        proofs: Arc::new(Mutex::new(vec![])),
        alive: true,
    };
    let options = WrapperOptions::new(
        Message {
            provider: "synthetic".into(),
            ..Default::default()
        },
        plan(root.path()),
    );
    let run = tokio::spawn(async move {
        runtime::run(
            options,
            &connector,
            &FakeFactory(pty),
            &Cancellation::default(),
            |_, _| Ok(()),
        )
        .await
    });
    receive_type(&mut rx, "register").await;
    tx.send(Message {
        r#type: "registered".into(),
        session_id: 1,
        ..Default::default()
    })
    .await
    .unwrap();
    for _ in 0..2 {
        tx.send(Message {
            r#type: "pty_input".into(),
            input_seq: 5,
            data: b"full input".to_vec(),
            ..Default::default()
        })
        .await
        .unwrap();
        receive_type(&mut rx, "pty_input_ack").await;
    }
    assert_eq!(*retained.writes.lock().unwrap(), b"full input");
    tx.send(Message {
        r#type: "session_dismissed".into(),
        ..Default::default()
    })
    .await
    .unwrap();
    run.await.unwrap().unwrap();
}
#[tokio::test]
async fn wrapper_failed_write_never_acks_and_keeps_session_until_explicit_dismissal() {
    let root = tempfile::tempdir().unwrap();
    let pty = Arc::new(FakePty::new(vec![], true));
    let retained = pty.clone();
    let (socket, tx, mut rx) = socket_pair();
    let connector = ScriptConnector {
        sockets: Mutex::new(VecDeque::from([socket])),
        proofs: Arc::new(Mutex::new(vec![])),
        alive: true,
    };
    let options = WrapperOptions::new(
        Message {
            provider: "synthetic".into(),
            ..Default::default()
        },
        plan(root.path()),
    );
    let run = tokio::spawn(async move {
        runtime::run(
            options,
            &connector,
            &FakeFactory(pty),
            &Cancellation::default(),
            |_, _| Ok(()),
        )
        .await
    });
    receive_type(&mut rx, "register").await;
    tx.send(Message {
        r#type: "registered".into(),
        session_id: 1,
        ..Default::default()
    })
    .await
    .unwrap();
    tx.send(Message {
        r#type: "pty_input".into(),
        input_seq: 5,
        data: b"failure".to_vec(),
        ..Default::default()
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while retained
            .write_attempts
            .load(std::sync::atomic::Ordering::Acquire)
            == 0
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(80), rx.recv())
            .await
            .is_err()
    );
    assert!(!retained.closed.load(std::sync::atomic::Ordering::Acquire));
    tx.send(Message {
        r#type: "session_dismissed".into(),
        ..Default::default()
    })
    .await
    .unwrap();
    assert_eq!(receive_type(&mut rx, "session_end").await.state, "error");
    run.await.unwrap().unwrap();
    assert!(retained.closed.load(std::sync::atomic::Ordering::Acquire));
    while let Ok(frame) = rx.try_recv() {
        assert_ne!(frame.r#type, "pty_input_ack");
    }
}
#[tokio::test]
async fn wrapper_output_backpressure_reconnects_even_with_live_hub_auto_shutdown() {
    let root = tempfile::tempdir().unwrap();
    let pty = Arc::new(FakePty::new(vec![vec![b'x'; 4096]; 20], false));
    let (first, first_tx, mut first_rx) = socket_pair();
    let (second, second_tx, mut second_rx) = socket_pair();
    let connector = ScriptConnector {
        sockets: Mutex::new(VecDeque::from([first, second])),
        proofs: Arc::new(Mutex::new(vec![])),
        alive: true,
    };
    let mut options = WrapperOptions::new(
        Message {
            provider: "synthetic".into(),
            ..Default::default()
        },
        plan(root.path()),
    );
    options.output_capacity = 1;
    options.auto_shutdown = true;
    options.reconnect_interval = Duration::from_millis(1);
    options.reconnect_grace = Duration::from_secs(2);
    let run = tokio::spawn(async move {
        runtime::run(
            options,
            &connector,
            &FakeFactory(pty),
            &Cancellation::default(),
            |_, _| Ok(()),
        )
        .await
    });
    receive_type(&mut first_rx, "register").await;
    first_tx
        .send(Message {
            r#type: "registered".into(),
            session_id: 1,
            ..Default::default()
        })
        .await
        .unwrap();
    let replay = receive_type(&mut second_rx, "reattach").await;
    assert_eq!(replay.pty_bytes, 4096 * 20);
    second_tx
        .send(Message {
            r#type: "reattach_ack".into(),
            session_id: 1,
            ..Default::default()
        })
        .await
        .unwrap();
    second_tx
        .send(Message {
            r#type: "session_dismissed".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(run.await.unwrap().unwrap().reconnects, 1);
    drop(first_tx);
}

struct NeverFactory;
impl PtyFactory for NeverFactory {
    fn spawn(&self, _: &ProcessPlan, _: PtySize) -> io::Result<Arc<dyn PtySession>> {
        panic!("provider must not start without registration ACK")
    }
}
#[tokio::test]
async fn wrapper_lost_registration_ack_consumes_no_provider_and_never_retries_proof() {
    let root = tempfile::tempdir().unwrap();
    let (socket, tx, mut rx) = socket_pair();
    let proofs = Arc::new(Mutex::new(vec![]));
    let trace = proofs.clone();
    let connector = ScriptConnector {
        sockets: Mutex::new(VecDeque::from([socket])),
        proofs,
        alive: true,
    };
    let mut options = WrapperOptions::new(
        Message {
            provider: "synthetic".into(),
            ..Default::default()
        },
        plan(root.path()),
    );
    options.initial_proof = Some("single-attempt-synthetic".into());
    options.write_timeout = Duration::from_secs(1);
    let run = tokio::spawn(async move {
        runtime::run(
            options,
            &connector,
            &NeverFactory,
            &Cancellation::default(),
            |_, _| panic!("preparation also waits for ACK"),
        )
        .await
    });
    receive_type(&mut rx, "register").await;
    drop(tx);
    assert!(run.await.unwrap().is_err());
    assert_eq!(
        *trace.lock().unwrap(),
        vec![Some("single-attempt-synthetic".into())]
    );
}
#[tokio::test]
async fn wrapper_explicit_hub_shutdown_keeps_provider_for_manual_restart() {
    let root = tempfile::tempdir().unwrap();
    let pty = Arc::new(FakePty::new(vec![], false));
    let retained = pty.clone();
    let (first, first_tx, mut first_rx) = socket_pair();
    let (second, second_tx, mut second_rx) = socket_pair();
    let connector = ScriptConnector {
        sockets: Mutex::new(VecDeque::from([first, second])),
        proofs: Arc::new(Mutex::new(vec![])),
        alive: true,
    };
    let mut options = WrapperOptions::new(
        Message {
            provider: "synthetic".into(),
            ..Default::default()
        },
        plan(root.path()),
    );
    options.auto_shutdown = true;
    options.reconnect_interval = Duration::from_millis(1);
    options.reconnect_grace = Duration::from_secs(2);
    let run = tokio::spawn(async move {
        runtime::run(
            options,
            &connector,
            &FakeFactory(pty),
            &Cancellation::default(),
            |_, _| Ok(()),
        )
        .await
    });
    receive_type(&mut first_rx, "register").await;
    first_tx
        .send(Message {
            r#type: "registered".into(),
            session_id: 1,
            ..Default::default()
        })
        .await
        .unwrap();
    first_tx
        .send(Message {
            r#type: "hub_shutdown".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    receive_type(&mut second_rx, "reattach").await;
    assert!(!retained.closed.load(std::sync::atomic::Ordering::Acquire));
    second_tx
        .send(Message {
            r#type: "reattach_ack".into(),
            session_id: 2,
            ..Default::default()
        })
        .await
        .unwrap();
    second_tx
        .send(Message {
            r#type: "session_dismissed".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(run.await.unwrap().unwrap().session_id, 2);
}

#[tokio::test]
async fn wrapper_failed_native_resize_does_not_interrupt_subsequent_input() {
    let root = tempfile::tempdir().unwrap();
    let mut fake = FakePty::new(vec![], false);
    fake.fail_resize = true;
    let pty = Arc::new(fake);
    let retained = pty.clone();
    let (socket, tx, mut rx) = socket_pair();
    let connector = ScriptConnector {
        sockets: Mutex::new(VecDeque::from([socket])),
        proofs: Arc::new(Mutex::new(vec![])),
        alive: true,
    };
    let options = WrapperOptions::new(
        Message {
            provider: "synthetic".into(),
            ..Default::default()
        },
        plan(root.path()),
    );
    let run = tokio::spawn(async move {
        runtime::run(
            options,
            &connector,
            &FakeFactory(pty),
            &Cancellation::default(),
            |_, _| Ok(()),
        )
        .await
    });
    receive_type(&mut rx, "register").await;
    tx.send(Message {
        r#type: "registered".into(),
        session_id: 1,
        ..Default::default()
    })
    .await
    .unwrap();
    tx.send(Message {
        r#type: "pty_resize".into(),
        cols: 120,
        rows: 40,
        ..Default::default()
    })
    .await
    .unwrap();
    for _ in 0..2 {
        tx.send(Message {
            r#type: "pty_input".into(),
            input_seq: 5,
            data: b"full input".to_vec(),
            ..Default::default()
        })
        .await
        .unwrap();
        receive_type(&mut rx, "pty_input_ack").await;
    }
    assert_eq!(*retained.writes.lock().unwrap(), b"full input");
    tx.send(Message {
        r#type: "session_dismissed".into(),
        ..Default::default()
    })
    .await
    .unwrap();
    run.await.unwrap().unwrap();
}
