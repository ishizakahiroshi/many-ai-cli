//! P004-C-N01/N02/N03: real core/observer/effect driver/notification manager,
//! synthetic completion callbacks and loopback receiver. This does not execute
//! the main-program routine and handoff callback implementation.
use super::*;
use crate::hub::sockets::{EffectDriver, FrameWriter, SocketRegistry, WireFrame};
use std::sync::atomic::AtomicUsize;

struct CompletionCallbacks {
    root: PathBuf,
    fail_handoff: AtomicBool,
    trace: Arc<Mutex<Vec<&'static str>>>,
}
impl GitTurnCompletionCallbacks for CompletionCallbacks {
    fn routine_completed<'a>(
        &'a self,
        _: SessionBinding,
        _: &'a proto::DoneSummary,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            std::fs::write(
                self.root.join("routine-result"),
                b"synthetic routine result",
            )
            .unwrap();
            self.trace.lock().unwrap().push("routine");
            Ok(())
        })
    }
    fn before_done_publish<'a>(
        &'a self,
        _: SessionBinding,
        _: &'a proto::DoneSummary,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            assert!(self.root.join("routine-result").is_file());
            std::fs::write(self.root.join("handoff-first"), b"synthetic first append").unwrap();
            self.trace.lock().unwrap().push("handoff");
            if self.fail_handoff.load(Ordering::SeqCst) {
                return Err(SessionError::InvalidRequest(
                    "synthetic later append failed".into(),
                ));
            }
            Ok(())
        })
    }
    fn handoff_note_written<'a>(
        &'a self,
        _: SessionBinding,
        _: &'a std::path::Path,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async { panic!("relay publication does not write a handoff note") })
    }
    fn inject_turn_summary<'a>(
        &'a self,
        _: SessionBinding,
        _: i64,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async { panic!("relay publication does not inject a Git summary") })
    }
    fn after_git_turn_broadcast<'a>(
        &'a self,
        _: SessionBinding,
        _: &'a GitTurnSnapshot,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async { panic!("relay publication does not publish a Git turn") })
    }
}
struct Writer(Arc<Mutex<Vec<&'static str>>>);
impl FrameWriter for Writer {
    fn write<'a>(&'a mut self, frame: WireFrame) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            if let WireFrame::Text(text) = frame {
                let message: proto::Message = serde_json::from_str(&text).unwrap();
                if message.r#type == "done_summary" {
                    self.0.lock().unwrap().push("live-summary");
                }
            }
            Ok(())
        })
    }
}
struct Publisher(Arc<Mutex<Vec<&'static str>>>);
impl PersistenceEffectSink for Publisher {
    fn apply(&self, _: PersistenceEffect) -> Result<(), SessionError> {
        panic!("relay summary has no persistence effect; callbacks own their writes")
    }
}
impl CoreEventPublisher for Publisher {
    fn publish(&self, event: CoreEvent) -> Result<EventSequence, SessionError> {
        if matches!(event, CoreEvent::Completed { .. }) {
            self.0.lock().unwrap().push("completed");
        }
        Ok(EventSequence(1))
    }
}

struct Publication {
    _root: tempfile::TempDir,
    core: Arc<SessionEngine>,
    callbacks: Arc<CompletionCallbacks>,
    driver: EffectDriver,
    owner: HubTaskOwner,
    summary: proto::DoneSummary,
    priming: UiBinding,
    trace: Arc<Mutex<Vec<&'static str>>>,
}
async fn publication(url: &str, gate: &str) -> Publication {
    let root = tempfile::tempdir().unwrap();
    let runtime = root.path().join("runtime");
    std::fs::create_dir(&runtime).unwrap();
    let paths = RuntimePaths::trial(&runtime, 49579, &root.path().join("installed")).unwrap();
    let trace = Arc::new(Mutex::new(Vec::new()));
    let callbacks = Arc::new(CompletionCallbacks {
        root: runtime.clone(),
        fail_handoff: AtomicBool::new(gate == "handoff-failure"),
        trace: trace.clone(),
    });
    let mut cfg = Config::default();
    cfg.user_prefs.done_summary_notify.enabled = Some(gate != "disabled");
    cfg.notify.backends = Some(if gate == "empty-backends" {
        Vec::new()
    } else {
        vec![crate::config::NotifyBackendConfig {
            r#type: "webhook".into(),
            url: url.into(),
            topic: String::new(),
        }]
    });
    cfg.notify.events = Some(vec![
        if gate == "no-done-event" {
            "approval"
        } else {
            "done"
        }
        .into(),
    ]);
    cfg.notify.include_body = true;
    let config = Arc::new(ConfigStore::new(paths.clone(), cfg.clone()).unwrap());
    let owner = HubTaskOwner::new(tokio::runtime::Handle::current());
    // The production notification policy receives only this test's loopback
    // URL; runtime files and all other owners stay in the synthetic trial root.
    let manager = if gate == "trial" {
        crate::notify::Manager::new_for_runtime(
            cfg.notify,
            owner.handle(),
            Arc::new(|_, _| {}),
            &paths,
        )
    } else {
        crate::notify::Manager::new(cfg.notify, owner.handle(), Arc::new(|_, _| {}))
    }
    .unwrap();
    let sink = Arc::new(Sink::default());
    let core = Arc::new(SessionEngine::new(
        EngineOptions::default(),
        Arc::new(SessionJournal::new(
            paths.clone(),
            None,
            JournalOptions {
                session_enabled: false,
                max_bytes: 0,
            },
        )),
        Arc::new(Transport),
        sink.clone(),
        Arc::new(NoSpawn),
        CoreEventBus::new(64).unwrap(),
    ));
    let registration = core
        .register(
            RegisterRequest {
                message: proto::Message {
                    provider: "codex".into(),
                    cwd: runtime.to_string_lossy().into_owned(),
                    pid: 7,
                    ..Default::default()
                },
                spawn_proof: None,
            },
            WrapperConnectionId(1),
            Timestamp::now(),
        )
        .await
        .unwrap();
    let sockets = Arc::new(SocketRegistry::default());
    let core_trait: Arc<dyn SessionCore> = core.clone();
    sockets.bind_core(Arc::downgrade(&core_trait)).unwrap();
    let live = UiBinding {
        connection: UiConnectionId(1),
        auth_epoch: core.auth_epoch(),
    };
    let priming = UiBinding {
        connection: UiConnectionId(2),
        auth_epoch: core.auth_epoch(),
    };
    core.attach_ui(live, None, None).unwrap();
    assert!(core.finish_ui_priming(live).unwrap().0.is_empty());
    core.attach_ui(priming, None, None).unwrap();
    sockets
        .insert_ui(live, Box::new(Writer(trace.clone())))
        .unwrap();
    let observer = Arc::new(
        ApplicationEventObserver::new(
            config,
            paths.clone(),
            Arc::new(FilesService::new(runtime, paths)),
            callbacks.clone(),
            Arc::new(|_, _| {}),
            owner.handle(),
        )
        .with_notifications(manager),
    );
    let effects: Arc<dyn CoreEffectSink> = sink;
    observer
        .bind(Arc::downgrade(&core), Arc::downgrade(&effects))
        .unwrap();
    let publisher = Arc::new(Publisher(trace.clone()));
    let driver = EffectDriver::new(sockets, publisher.clone(), publisher, observer);
    let summary = proto::DoneSummary {
        session_id: registration.binding.session.0,
        provider: "codex".into(),
        title: "synthetic relay stopped".into(),
        text: "synthetic relay finished".into(),
        kind: "relay".into(),
        fallback: gate == "fallback",
        ..Default::default()
    };
    Publication {
        _root: root,
        core,
        callbacks,
        driver,
        owner,
        summary,
        priming,
        trace,
    }
}

async fn receiver() -> (
    String,
    Arc<AtomicUsize>,
    Cancellation,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let calls = Arc::new(AtomicUsize::new(0));
    let app = axum::Router::new()
        .fallback(axum::routing::post(
            |axum::extract::State(calls): axum::extract::State<Arc<AtomicUsize>>| async move {
                calls.fetch_add(1, Ordering::SeqCst);
                axum::http::StatusCode::NO_CONTENT
            },
        ))
        .with_state(calls.clone());
    let cancel = Cancellation::default();
    let stopping = cancel.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move { stopping.cancelled().await })
            .await
            .unwrap();
    });
    (url, calls, cancel, server)
}

#[tokio::test]
async fn relay_completion_notification_gates_preserve_earlier_callbacks_and_live_summary() {
    let (url, calls, cancel, server) = receiver().await;
    for gate in [
        "disabled",
        "fallback",
        "no-done-event",
        "empty-backends",
        "trial",
        "enabled",
    ] {
        let f = publication(&url, gate).await;
        let before = calls.load(Ordering::SeqCst);
        f.driver
            .apply(f.core.publish_relay_done(f.summary.clone()))
            .await
            .unwrap();
        f.owner.drain_effects().await;
        assert_eq!(
            calls.load(Ordering::SeqCst) - before,
            usize::from(gate == "enabled"),
            "{gate}"
        );
        assert_eq!(
            *f.trace.lock().unwrap(),
            ["routine", "handoff", "live-summary", "completed"]
        );
        assert_eq!(
            std::fs::read(f.callbacks.root.join("routine-result")).unwrap(),
            b"synthetic routine result"
        );
        assert_eq!(
            std::fs::read(f.callbacks.root.join("handoff-first")).unwrap(),
            b"synthetic first append"
        );
    }
    cancel.cancel();
    server.await.unwrap();
}

#[tokio::test]
async fn relay_callback_failure_keeps_prior_writes_and_priming_but_stops_live_and_notification() {
    let (url, calls, cancel, server) = receiver().await;
    let f = publication(&url, "handoff-failure").await;
    let error = f
        .driver
        .apply(f.core.publish_relay_done(f.summary.clone()))
        .await
        .unwrap_err();
    assert_eq!(error.index, 1);
    assert_eq!(*f.trace.lock().unwrap(), ["routine", "handoff"]);
    assert_eq!(
        std::fs::read(f.callbacks.root.join("routine-result")).unwrap(),
        b"synthetic routine result"
    );
    assert_eq!(
        std::fs::read(f.callbacks.root.join("handoff-first")).unwrap(),
        b"synthetic first append"
    );
    f.owner.drain_effects().await;
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    // SessionEngine queued this during route(), before the failed callback.
    // Draining exposes it; the failed effect batch did not roll it back.
    let pending = f.core.finish_ui_priming(f.priming).unwrap();
    assert_eq!(pending.0.len(), 1);
    assert!(
        matches!(&pending.0[0], CoreEffect::SendUi { binding, message }
        if *binding == f.priming && message.r#type == "done_summary")
    );
    cancel.cancel();
    server.await.unwrap();
}
