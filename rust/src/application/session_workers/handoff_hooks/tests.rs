use super::*;
use crate::{
    application::event_observer::ApplicationEventObserver,
    config::Config,
    hub::{
        ServiceRouter,
        sockets::{EffectDriver, FrameWriter, SocketRegistry, WireFrame},
        task_owner::HubTaskOwner,
        transport,
        websocket::WebSocketService,
    },
    process::Cancellation,
    terminal::{
        events::CoreEventBus,
        journal::{JournalOptions, SessionJournal},
        session::EngineOptions,
    },
};
use futures_util::{SinkExt, StreamExt};
use std::sync::Mutex;

#[derive(Clone, Default)]
struct Recording(Arc<Mutex<Vec<WireFrame>>>);
impl FrameWriter for Recording {
    fn write<'a>(&'a mut self, frame: WireFrame) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            self.0.lock().unwrap().push(frame);
            Ok(())
        })
    }
}
impl Recording {
    fn messages(&self) -> Vec<proto::Message> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .map(|frame| match frame {
                WireFrame::Text(text) => serde_json::from_str(text).unwrap(),
                WireFrame::Close => panic!("successful handoff must keep the UI connected"),
            })
            .collect()
    }
}
struct NoSpawn;
impl WrappedSessionSpawner for NoSpawn {
    fn spawn_and_wait<'a>(
        &'a self,
        _: WrappedSpawnSpec,
        _: Duration,
        _: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome> {
        Box::pin(async { panic!("synthetic test forbids providers") })
    }
}
struct Fixture {
    _root: tempfile::TempDir,
    _tasks: HubTaskOwner,
    paths: RuntimePaths,
    config: Arc<ConfigStore>,
    core: Arc<SessionEngine>,
    effects: Arc<EffectDriver>,
    sockets: Arc<SocketRegistry>,
    workers: Arc<SessionWorkers>,
    warnings: Arc<Mutex<Vec<String>>>,
}
impl Fixture {
    fn new(port: u16) -> Self {
        let root = tempfile::tempdir().unwrap();
        let runtime = root.path().join("trial");
        std::fs::create_dir(&runtime).unwrap();
        let paths = RuntimePaths::trial(&runtime, port, &root.path().join("installed")).unwrap();
        let tasks = HubTaskOwner::new(tokio::runtime::Handle::current());
        let warnings = Arc::new(Mutex::new(Vec::new()));
        let warning: EventWarning = {
            let warnings = warnings.clone();
            Arc::new(move |operation, _| warnings.lock().unwrap().push(operation.into()))
        };
        let config = Arc::new(
            ConfigStore::new(
                paths.clone(),
                Config {
                    token: "synthetic-handoff-token".into(),
                    ..Default::default()
                },
            )
            .unwrap(),
        );
        let files = Arc::new(FilesService::new(runtime, paths.clone()));
        let workers = SessionWorkers::new(
            config.clone(),
            paths.clone(),
            files.clone(),
            tasks.handle(),
            warning.clone(),
        );
        let observer = Arc::new(ApplicationEventObserver::new(
            config.clone(),
            paths.clone(),
            files,
            workers.clone(),
            warning.clone(),
            tasks.handle(),
        ));
        let sockets = Arc::new(SocketRegistry::default());
        let journal = Arc::new(SessionJournal::new(
            paths.clone(),
            None,
            JournalOptions::default(),
        ));
        let events = CoreEventBus::new(16).unwrap();
        let effects = Arc::new(
            EffectDriver::new(
                sockets.clone(),
                journal.clone(),
                Arc::new(events.clone()),
                observer.clone(),
            )
            .with_warning_handler(warning.clone()),
        );
        let core = Arc::new(SessionEngine::new(
            EngineOptions {
                warning,
                ..Default::default()
            },
            journal,
            sockets.clone(),
            effects.clone(),
            Arc::new(NoSpawn),
            events,
        ));
        let trait_core: Arc<dyn SessionCore> = core.clone();
        let trait_effects: Arc<dyn CoreEffectSink> = effects.clone();
        sockets.bind_core(Arc::downgrade(&trait_core)).unwrap();
        workers
            .bind(Arc::downgrade(&core), Arc::downgrade(&trait_effects))
            .unwrap();
        observer
            .bind(Arc::downgrade(&core), Arc::downgrade(&trait_effects))
            .unwrap();
        Self {
            _root: root,
            _tasks: tasks,
            paths,
            config,
            core,
            effects,
            sockets,
            workers,
            warnings,
        }
    }
    async fn register(&self) -> SessionBinding {
        self.core
            .register(
                RegisterRequest {
                    message: proto::Message {
                        provider: "claude".into(),
                        cwd: self.paths.root().to_string_lossy().into_owned(),
                        pid: 12,
                        cols: 80,
                        rows: 24,
                        ..Default::default()
                    },
                    spawn_proof: None,
                },
                self.sockets.next_wrapper().unwrap(),
                Timestamp::now(),
            )
            .await
            .unwrap()
            .binding
    }
}
#[tokio::test]
async fn memo_receipt_checks_file_and_primes_real_delivery_without_reading_body() {
    let fixture = Fixture::new(49678);
    let binding = fixture.register().await;
    let ui = UiBinding {
        connection: fixture.sockets.next_ui().unwrap(),
        auth_epoch: fixture.core.auth_epoch(),
    };
    let recording = Recording::default();
    fixture
        .sockets
        .insert_ui(ui, Box::new(recording.clone()))
        .unwrap();
    let priming = fixture.core.attach_ui(ui, None, None).unwrap();
    let store = HandoffStore::new(fixture.paths.clone());
    let path = store.note_path_for(binding.session.0).unwrap();
    fixture
        .workers
        .handoff_note_written(binding, &path)
        .await
        .unwrap();
    assert!(store.read_session(binding.session.0).unwrap().is_empty());
    assert!(
        recording.messages().is_empty(),
        "handoff notification waits for UI priming"
    );
    for message in priming.ordered_frames {
        fixture.sockets.send_ui(ui, message).await.unwrap();
    }
    fixture
        .effects
        .apply(fixture.core.finish_ui_priming(ui).unwrap())
        .await
        .unwrap();
    assert!(fixture.core.finish_ui_priming(ui).unwrap().0.is_empty());
    let messages = recording.messages();
    assert_eq!(messages[0].r#type, "approval_snapshot");
    let message = messages.last().unwrap();
    assert_eq!(message.r#type, "handoff_note");
    assert!(!message.note_ok);
    assert!(message.note_path.is_empty());

    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    // Invalid UTF-8 body proves this route records filesystem identity only.
    std::fs::write(&path, b"\xff\xfePRIVATE-MEMO-BODY-SENTINEL").unwrap();
    fixture
        .workers
        .handoff_note_written(binding, &path)
        .await
        .unwrap();
    let records = store.read_session(binding.session.0).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].kind, crate::orchestration::handoff::KIND_NOTE);
    assert_eq!(records[0].note, path.to_string_lossy());
    assert!(
        !serde_json::to_string(&records)
            .unwrap()
            .contains("PRIVATE-MEMO-BODY-SENTINEL")
    );
    let messages = recording.messages();
    assert_eq!(messages.len(), 3);
    let message = messages.last().unwrap();
    assert_eq!(message.r#type, "handoff_note");
    assert!(message.note_ok);
    assert_eq!(message.note_path, path.to_string_lossy());
    assert!(fixture.warnings.lock().unwrap().is_empty());
}

type Client = tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>;
async fn client(port: u16) -> Client {
    let stream = tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
    tokio_tungstenite::client_async(format!("ws://127.0.0.1:{port}/ws"), stream)
        .await
        .unwrap()
        .0
}
async fn send(client: &mut Client, value: serde_json::Value) {
    client
        .send(tokio_tungstenite::tungstenite::Message::Text(
            value.to_string().into(),
        ))
        .await
        .unwrap();
}
async fn until(client: &mut Client, kind: &str) -> serde_json::Value {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let frame = client
                .next()
                .await
                .expect("connection closed before expected frame")
                .unwrap();
            let tokio_tungstenite::tungstenite::Message::Text(text) = frame else {
                panic!("unexpected frame before {kind}: {frame:?}");
            };
            let value: serde_json::Value = serde_json::from_str(&text).unwrap();
            if value["type"] == kind {
                return value;
            }
        }
    })
    .await
    .unwrap()
}
struct Server {
    cancel: Cancellation,
    task: Option<tokio::task::JoinHandle<std::io::Result<()>>>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}
#[tokio::test]
async fn handoff_marker_keeps_later_effects_and_real_wrapper_connection_alive() {
    use base64::Engine as _;
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let fixture = Fixture::new(port);
    let services = Arc::new(
        ServiceRouter::new(
            fixture.config.clone(),
            fixture.paths.clone(),
            fixture.core.clone(),
            port,
        )
        .unwrap(),
    );
    let cancel = Cancellation::default();
    let websocket = Arc::new(WebSocketService::new(
        services.clone(),
        fixture.core.clone(),
        fixture.sockets.clone(),
        fixture.effects.clone(),
        cancel.clone(),
        {
            let warnings = fixture.warnings.clone();
            Arc::new(move |operation, _| warnings.lock().unwrap().push(operation.into()))
        },
    ));
    let mut server = Server {
        cancel: cancel.clone(),
        task: Some(tokio::spawn(transport::serve_with_websockets(
            listener,
            services,
            fixture.effects.clone(),
            websocket,
            cancel,
        ))),
    };
    let mut ui = client(port).await;
    send(
        &mut ui,
        serde_json::json!({"role":"ui", "token":"synthetic-handoff-token"}),
    )
    .await;
    until(&mut ui, "snapshot").await;
    until(&mut ui, "approval_snapshot").await;
    let mut wrapper = client(port).await;
    send(
        &mut wrapper,
        serde_json::json!({
            "type":"register", "provider":"claude", "cwd":fixture.paths.root(),
            "pid":12, "cols":80, "rows":24, "token":"synthetic-handoff-token"
        }),
    )
    .await;
    let ack = until(&mut wrapper, "registered").await;
    let id = LiveSessionId(ack["session_id"].as_i64().unwrap());
    let binding = fixture.core.details(id).unwrap().binding;
    let store = HandoffStore::new(fixture.paths.clone());
    let path = store.note_path_for(id.0).unwrap();
    for exists in [false, true] {
        if exists {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"synthetic private memo").unwrap();
        }
        fixture
            .core
            .begin_handoff_note(binding, path.clone())
            .unwrap();
        let marker = b"[MANY-AI-CLI-HANDOFF-NOTE] written [/MANY-AI-CLI-HANDOFF-NOTE]\n";
        send(
            &mut wrapper,
            serde_json::json!({"type":"pty_data", "data":marker.as_slice()}),
        )
        .await;
        let note = until(&mut ui, "handoff_note").await;
        assert_eq!(note["note_ok"].as_bool().unwrap_or(false), exists);
        assert_eq!(note["session_id"], id.0);
        // This pty_data is later in the same actual observe_output effect batch
        // than HandoffNoteWritten. A rejected inner Broadcast drops this frame.
        let output = until(&mut ui, "pty_data").await;
        assert_eq!(
            output["data"],
            base64::engine::general_purpose::STANDARD.encode(marker)
        );
        assert_eq!(store.read_session(id.0).unwrap().len(), usize::from(exists));
        let next = b"synthetic output after handoff\n";
        send(
            &mut wrapper,
            serde_json::json!({"type":"pty_data", "data":next.as_slice()}),
        )
        .await;
        assert_eq!(
            until(&mut ui, "pty_data").await["data"],
            base64::engine::general_purpose::STANDARD.encode(next)
        );
        assert!(fixture.core.is_current(binding));
        fixture
            .sockets
            .send(
                binding,
                proto::Message {
                    r#type: "pty_resize".into(),
                    session_id: id.0,
                    cols: 101,
                    rows: 31,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(until(&mut wrapper, "pty_resize").await["cols"], 101);
        assert!(fixture.warnings.lock().unwrap().is_empty());
    }
    drop(ui);
    drop(wrapper);
    server.cancel.cancel();
    tokio::time::timeout(Duration::from_secs(5), server.task.take().unwrap())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
