//! Deterministic caller tests use isolated journal/database roots and injected
//! transports. They do not assert native PTY/provider/UI acceptance.
use many_ai_cli::proto::time::{Timestamp, UNIX_EPOCH};
#[path = "session_engine/approval_actions.rs"]
mod approval_actions;
#[path = "session_engine/branch_persistence.rs"]
mod branch_persistence;
#[path = "session_engine/marker_suppression.rs"]
mod marker_suppression;
#[path = "session_engine/reattach_transaction.rs"]
mod reattach_transaction;
#[path = "session_engine/startup_registration.rs"]
mod startup_registration;
use many_ai_cli::{
    config::{Resource, RuntimePaths},
    proto::{self, core::*},
    storage::SqliteSessionStorage,
    terminal::{
        events::CoreEventBus,
        journal::{JournalOptions, SessionJournal},
        session::{EngineOptions, SessionEngine},
    },
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
fn now() -> Timestamp {
    UNIX_EPOCH + Duration::from_secs(1767323045)
}
type SendHook =
    Arc<dyn Fn(SessionBinding, proto::Message) -> CoreFuture<'static, ()> + Send + Sync>;
#[derive(Default)]
struct Transport {
    frames: Mutex<Vec<(SessionBinding, proto::Message)>>,
    calls: AtomicUsize,
    fail: AtomicUsize,
    pause_call: AtomicUsize,
    entered: tokio::sync::Notify,
    resume: tokio::sync::Notify,
    on_send: Mutex<Option<SendHook>>,
}
impl WrapperTransport for Transport {
    fn send<'a>(
        &'a self,
        b: SessionBinding,
        m: proto::Message,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if self.pause_call.load(Ordering::SeqCst) == n {
                self.entered.notify_one();
                self.resume.notified().await;
            }
            if self.fail.load(Ordering::SeqCst) == n {
                return Err(SessionError::Transport("synthetic write failure".into()));
            }
            self.frames.lock().unwrap().push((b, m.clone()));
            let hook = self.on_send.lock().unwrap().clone();
            if let Some(hook) = hook {
                hook(b, m).await;
            }
            Ok(())
        })
    }
}
struct Sink {
    journal: Arc<SessionJournal>,
    bus: CoreEventBus,
    ui: Mutex<Vec<(UiBinding, proto::Message)>>,
    events: Mutex<Vec<&'static str>>,
}
impl CoreEffectSink for Sink {
    fn apply<'a>(&'a self, effects: CoreEffects) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
        Box::pin(async move {
            for (index, effect) in effects.0.into_iter().enumerate() {
                let result = match effect {
                    CoreEffect::Persist(p) => self.journal.apply(p),
                    CoreEffect::PersistBound {
                        binding,
                        scope,
                        effect,
                        order,
                    } => {
                        order.wait().await;
                        let result = self.journal.apply_bound(binding, scope, effect);
                        drop(order);
                        result
                    }
                    CoreEffect::Notify(e) => {
                        self.events.lock().unwrap().push(match &e {
                            CoreEvent::Registered(_) => "registered",
                            CoreEvent::GitTurnCapture { .. } => "git",
                            _ => "other",
                        });
                        self.bus.publish(e).map(|_| ())
                    }
                    CoreEffect::SendUi { binding, message }
                    | CoreEffect::SendUiBestEffort { binding, message } => {
                        self.ui.lock().unwrap().push((binding, message));
                        Ok(())
                    }
                    CoreEffect::Broadcast(_) => panic!("core broadcast escaped priming owner"),
                    _ => Ok(()),
                };
                if let Err(error) = result {
                    return Err(CoreEffectFailure { index, error });
                }
            }
            Ok(())
        })
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
        Box::pin(async { SpawnWaitOutcome::Failed("synthetic test has no launcher".into()) })
    }
}
struct Fixture {
    _root: tempfile::TempDir,
    _installed: tempfile::TempDir,
    engine: Arc<SessionEngine>,
    transport: Arc<Transport>,
    sink: Arc<Sink>,
    store: Arc<SqliteSessionStorage>,
}
fn fixture() -> Fixture {
    fixture_with_spawner(Arc::new(NoSpawn))
}
fn fixture_with_spawner(spawner: Arc<dyn WrappedSessionSpawner>) -> Fixture {
    fixture_with_warning(spawner, Arc::new(|_, _| {}))
}
fn fixture_with_warning(
    spawner: Arc<dyn WrappedSessionSpawner>,
    warning: many_ai_cli::terminal::session::EngineWarningHandler,
) -> Fixture {
    fixture_with_branch_lookup(
        spawner,
        warning,
        true,
        EngineOptions::default().branch_lookup,
    )
}
fn fixture_with_branch_lookup(
    spawner: Arc<dyn WrappedSessionSpawner>,
    warning: many_ai_cli::terminal::session::EngineWarningHandler,
    session_enabled: bool,
    branch_lookup: many_ai_cli::terminal::session::BranchLookup,
) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49121, installed.path()).unwrap();
    let store = Arc::new(
        SqliteSessionStorage::open(
            &paths,
            StorageOptions::baseline(paths.resource(Resource::Logs)),
        )
        .unwrap(),
    );
    let journal = Arc::new(SessionJournal::new(
        paths,
        Some(store.clone()),
        JournalOptions {
            session_enabled,
            max_bytes: 0,
        },
    ));
    let transport = Arc::new(Transport::default());
    let bus = CoreEventBus::new(100).unwrap();
    let sink = Arc::new(Sink {
        journal: journal.clone(),
        bus: bus.clone(),
        ui: Mutex::new(Vec::new()),
        events: Mutex::new(Vec::new()),
    });
    let options = EngineOptions {
        hub_instance: "synthetic-hub".into(),
        submit_timing: SubmitTiming {
            idle_settle: Duration::from_millis(1),
            minimum: Duration::from_millis(1),
            slow_minimum: Duration::from_millis(1),
            maximum: Duration::from_millis(10),
            poll: Duration::from_millis(1),
            confirm_window: Duration::from_millis(2),
        },
        warning,
        branch_lookup,
        ..Default::default()
    };
    let engine = Arc::new(SessionEngine::new(
        options,
        journal,
        transport.clone(),
        sink.clone(),
        spawner,
        bus,
    ));
    Fixture {
        _root: root,
        _installed: installed,
        engine,
        transport,
        sink,
        store,
    }
}
async fn register_unapplied(f: &Fixture) -> Registration {
    f.engine
        .register(
            RegisterRequest {
                message: proto::Message {
                    provider: "copilot".into(),
                    display_name: "Synthetic".into(),
                    cwd: "/fixture/project".into(),
                    label: "launch-label".into(),
                    pid: 7,
                    cols: 120,
                    rows: 30,
                    ..Default::default()
                },
                spawn_proof: None,
            },
            WrapperConnectionId(1),
            now(),
        )
        .await
        .unwrap()
}
async fn register(f: &Fixture) -> Registration {
    let mut r = register_unapplied(f).await;
    f.sink
        .apply(std::mem::take(&mut r.after_registered))
        .await
        .unwrap();
    r
}

fn ui(f: &Fixture, n: u64) -> UiBinding {
    let b = UiBinding {
        connection: UiConnectionId(n),
        auth_epoch: f.engine.auth_epoch(),
    };
    f.engine.attach_ui(b, None, None).unwrap();
    assert!(f.engine.finish_ui_priming(b).unwrap().0.is_empty());
    b
}
#[tokio::test(start_paused = true)]
async fn ui_idle_timer_generation_rejects_reconnect_and_only_claims_current_last_disconnect() {
    let fixture = fixture();
    let registration = register(&fixture).await;
    let presence = fixture.engine.subscribe_ui_presence();
    assert!(presence.borrow().disconnected_at.is_none());
    let first = ui(&fixture, 700);
    assert_eq!(presence.borrow().count, 1);
    let second = ui(&fixture, 701);
    fixture.engine.detach_ui(first);
    assert_eq!(presence.borrow().count, 1);
    assert!(presence.borrow().disconnected_at.is_none());
    fixture.engine.detach_ui(second);
    let stale_generation = presence.borrow().generation;
    assert_eq!(
        presence.borrow().disconnected_at,
        Some(tokio::time::Instant::now())
    );
    tokio::time::advance(Duration::from_secs(60)).await;
    let new_ui = ui(&fixture, 702);
    assert!(fixture.engine.expire_ui_idle(stale_generation).0.is_empty());
    fixture.engine.detach_ui(new_ui);
    let current_generation = presence.borrow().generation;
    let effects = fixture.engine.expire_ui_idle(current_generation);
    assert!(
        matches!(effects.0.as_slice(),[CoreEffect::CancelSession{binding,reason:StopReason::IdleTimeout}] if *binding == registration.binding)
    );
    assert!(
        fixture
            .engine
            .expire_ui_idle(current_generation)
            .0
            .is_empty()
    );
    fixture.engine.restart_ui_idle_timer();
    assert!(presence.borrow().disconnected_at.is_some());
    assert_ne!(presence.borrow().generation, current_generation);
}
async fn reattach(
    f: &Fixture,
    b: SessionBinding,
    bytes: &[u8],
    total: i64,
    connection: u64,
    watermark: i64,
) -> Reattachment {
    use base64::Engine;
    let mut r = f
        .engine
        .reattach(
            ReattachRequest {
                restored_metadata: None,
                message: proto::Message {
                    session_id: b.session.0,
                    provider: "copilot".into(),
                    cwd: "/fixture/project".into(),
                    label: "launch-label".into(),
                    pid: 7,
                    cols: 120,
                    rows: 30,
                    started_at: proto::time::format_rfc3339(now()).unwrap(),
                    replay_b64: base64::engine::general_purpose::STANDARD.encode(bytes),
                    pty_bytes: total,
                    input_seq_high_watermark: watermark,
                    ..Default::default()
                },
            },
            WrapperConnectionId(connection),
            now(),
        )
        .await
        .unwrap();
    f.sink
        .apply(std::mem::take(&mut r.after_reattached))
        .await
        .unwrap();
    r
}
#[tokio::test]
async fn registration_ack_is_separate_and_persistence_metadata_precedes_body_effects() {
    let f = fixture();
    let r = register_unapplied(&f).await;
    assert_eq!(r.registered.r#type, "registered");
    assert_eq!(f.transport.calls.load(Ordering::SeqCst), 0);
    assert!(f.sink.events.lock().unwrap().is_empty());
    assert_eq!(
        f.store
            .session_overview_by_live_session(r.binding.session)
            .unwrap()
            .label,
        "launch-label"
    );
    f.sink.apply(r.after_registered).await.unwrap();
    assert_eq!(f.sink.events.lock().unwrap()[0], "registered");
}
#[tokio::test]
async fn reverse_polled_input_futures_keep_invocation_fifo_and_dropped_waiter_does_not_overtake() {
    let f = fixture();
    let b = register(&f).await.binding;
    let c = TaskCancellation::default();
    let request = |text: &str| InputRequest {
        bytes: text.as_bytes().to_vec(),
        authority: InputAuthority::Internal,
    };
    let first = f.engine.submit(b, request("first"), now(), &c);
    let canceled = f.engine.submit(b, request("drop"), now(), &c);
    let last = f.engine.submit(b, request("last"), now(), &c);
    drop(canceled);
    let (r3, r1) = tokio::join!(last, first);
    assert!(matches!(
        r1.disposition,
        InputDisposition::TransportWritten { .. }
    ));
    assert!(matches!(
        r3.disposition,
        InputDisposition::TransportWritten { .. }
    ));
    let frames = f.transport.frames.lock().unwrap();
    assert_eq!(
        frames
            .iter()
            .map(|(_, m)| m.data.as_slice())
            .collect::<Vec<_>>(),
        [b"first".as_slice(), b"last".as_slice()]
    );
}
#[tokio::test]
async fn ack_reconnect_resends_original_sequence_before_pending_and_cold_watermark_is_adopted() {
    let f = fixture();
    let b = register(&f).await.binding;
    let c = TaskCancellation::default();
    f.engine.acknowledge(b, InputSeq(0));
    f.engine
        .submit(
            b,
            InputRequest {
                bytes: b"one".to_vec(),
                authority: InputAuthority::Internal,
            },
            now(),
            &c,
        )
        .await;
    let original = f.transport.frames.lock().unwrap()[0].1.input_seq;
    let effects = f.engine.disconnected(b, now()).unwrap();
    f.sink.apply(effects).await.unwrap();
    f.engine
        .submit(
            b,
            InputRequest {
                bytes: b"two".to_vec(),
                authority: InputAuthority::Internal,
            },
            now(),
            &c,
        )
        .await;
    let r = reattach(&f, b, b"", 0, 2, 40).await;
    f.engine.flush(r.binding, &c).await;
    let frames = f.transport.frames.lock().unwrap();
    assert_eq!(frames[1].1.input_seq, original);
    assert_eq!(frames[1].1.data, b"one");
    assert_eq!(frames[2].1.input_seq, 41);
    assert_eq!(frames[2].1.data, b"two");
    drop(frames);
    assert_eq!(
        f.engine.acknowledge(b, InputSeq(41)),
        AckDisposition::WrongConnection
    );
    assert_eq!(
        f.engine.acknowledge(r.binding, InputSeq(41)),
        AckDisposition::Removed
    );
}
#[tokio::test]
async fn initial_gate_bypass_overtakes_gated_input_exactly_once() {
    let f = fixture();
    let b = register(&f).await.binding;
    let c = TaskCancellation::default();
    f.engine.set_initial_gate(b, now()).unwrap();
    let r = f
        .engine
        .submit(
            b,
            InputRequest {
                bytes: b"later".to_vec(),
                authority: InputAuthority::Internal,
            },
            now(),
            &c,
        )
        .await;
    assert!(matches!(
        r.disposition,
        InputDisposition::Deferred {
            reason: DeferredReason::InitialPrompt,
            ..
        }
    ));
    f.engine
        .submit(
            b,
            InputRequest {
                bytes: b"initial".to_vec(),
                authority: InputAuthority::InitialPrompt,
            },
            now(),
            &c,
        )
        .await;
    f.engine.clear_initial_gate(b.session);
    f.engine.flush(b, &c).await;
    let frames = f.transport.frames.lock().unwrap();
    assert_eq!(
        frames
            .iter()
            .map(|(_, m)| m.data.as_slice())
            .collect::<Vec<_>>(),
        [b"initial".as_slice(), b"later".as_slice()]
    );
}
#[tokio::test]
async fn failed_delayed_enter_never_replays_paste_body() {
    let f = fixture();
    let b = register(&f).await.binding;
    let c = TaskCancellation::default();
    f.transport.fail.store(2, Ordering::SeqCst);
    let r = f
        .engine
        .submit(
            b,
            InputRequest {
                bytes: b"\x1b[200~body\x1b[201~\r".to_vec(),
                authority: InputAuthority::Internal,
            },
            now(),
            &c,
        )
        .await;
    assert!(
        matches!(r.disposition,InputDisposition::Failed{unsent_remainder,..}if unsent_remainder==b"\r")
    );
    f.transport.fail.store(0, Ordering::SeqCst);
    f.engine.flush(b, &c).await;
    let frames = f.transport.frames.lock().unwrap();
    assert_eq!(
        frames
            .iter()
            .filter(|(_, m)| m.data.windows(4).any(|x| x == b"body"))
            .count(),
        1
    );
    assert_eq!(frames[1].1.data, b"\r");
}
#[tokio::test]
async fn priming_drain_keeps_later_frames_behind_batch_until_empty_and_revocation_drops_queued_work()
 {
    let f = fixture();
    let b = register(&f).await.binding;
    let ui = UiBinding {
        connection: UiConnectionId(1),
        auth_epoch: f.engine.auth_epoch(),
    };
    f.engine.attach_ui(ui, Some(b.session), None).unwrap();
    assert!(
        f.engine
            .broadcast_ui(proto::Message {
                r#type: "first".into(),
                ..Default::default()
            })
            .0
            .is_empty()
    );
    let batch = f.engine.finish_ui_priming(ui).unwrap();
    assert_eq!(batch.0.len(), 1);
    assert!(
        f.engine
            .broadcast_ui(proto::Message {
                r#type: "second".into(),
                ..Default::default()
            })
            .0
            .is_empty()
    );
    assert_eq!(f.engine.finish_ui_priming(ui).unwrap().0.len(), 1);
    assert!(f.engine.finish_ui_priming(ui).unwrap().0.is_empty());
    assert_eq!(
        f.engine
            .broadcast_ui(proto::Message {
                r#type: "live".into(),
                ..Default::default()
            })
            .0
            .len(),
        1
    );
    f.engine.invalidate_all_ui();
    assert!(matches!(
        f.engine.finish_ui_priming(ui),
        Err(SessionError::AuthenticationExpired)
    ));
    assert!(f.engine.is_current(b));
}
#[tokio::test]
async fn zero_size_claim_transfers_control_without_creating_approval_epoch() {
    let f = fixture();
    let b = register(&f).await.binding;
    let one = ui(&f, 1);
    let two = ui(&f, 2);
    let epoch = f.engine.details(b.session).unwrap().approval.version;
    assert_eq!(
        f.engine
            .resize(
                one,
                b.session,
                TerminalSize {
                    cols: 100,
                    rows: 25
                },
                now()
            )
            .0,
        ResizeOutcome::Applied
    );
    f.engine
        .claim_ui_session(two, b.session, Some(TerminalSize::default()), now())
        .unwrap();
    assert_eq!(
        f.engine
            .resize(one, b.session, TerminalSize { cols: 90, rows: 24 }, now())
            .0,
        ResizeOutcome::NotController
    );
    assert_eq!(
        f.engine
            .resize(two, b.session, TerminalSize { cols: 80, rows: 24 }, now())
            .0,
        ResizeOutcome::Applied
    );
    assert_eq!(f.engine.details(b.session).unwrap().approval.version, epoch);
}
#[tokio::test]
async fn stale_disconnect_cannot_end_replacement_and_terminal_tail_cannot_revive_completed() {
    let f = fixture();
    let b = register(&f).await.binding;
    let r = reattach(&f, b, b"abc", 3, 2, 0).await;
    assert!(f.engine.disconnected(b, now()).unwrap().0.is_empty());
    assert!(f.engine.is_current(r.binding));
    f.engine
        .observe_end(
            r.binding,
            SessionEnd {
                declared_state: "completed".into(),
                exit_code: 2,
                reason: "declared".into(),
            },
            now(),
        )
        .unwrap();
    f.engine
        .observe_output(
            r.binding,
            OutputChunk {
                bytes: b"tail".to_vec(),
                total_pty_bytes: 7,
            },
            now(),
        )
        .unwrap();
    assert_eq!(f.engine.snapshot(b.session).unwrap().state, "completed");
}
#[tokio::test]
async fn revoked_ui_input_reserved_but_not_polled_cannot_write_or_mutate_history() {
    let f = fixture();
    let b = register(&f).await.binding;
    let u = ui(&f, 1);
    let c = TaskCancellation::default();
    let future = f.engine.submit(
        b,
        InputRequest {
            bytes: b"secret turn\r".to_vec(),
            authority: InputAuthority::Ui(u),
        },
        now(),
        &c,
    );
    f.engine.invalidate_all_ui();
    assert_eq!(
        future.await.disposition,
        InputDisposition::AuthenticationExpired
    );
    assert!(f.transport.frames.lock().unwrap().is_empty());
    assert!(
        f.engine
            .snapshot(b.session)
            .unwrap()
            .first_message
            .is_empty()
    );
}

#[tokio::test]
async fn initial_ui_size_without_active_session_controls_first_wrapper_geometry() {
    let f = fixture();
    let u = UiBinding {
        connection: UiConnectionId(1),
        auth_epoch: f.engine.auth_epoch(),
    };
    f.engine
        .attach_ui(
            u,
            None,
            Some(TerminalSize {
                cols: 132,
                rows: 41,
            }),
        )
        .unwrap();
    let registration = register(&f).await;
    assert_eq!(
        (registration.registered.cols, registration.registered.rows),
        (132, 41)
    );
}

#[tokio::test]
async fn restored_card_label_is_distinct_from_wrapper_launch_identity() {
    let f = fixture();
    let b = register(&f).await.binding;
    // This assertion concerns metadata restored before the reattach event.
    // The baseline asynchronous lifecycle event can overwrite a later rename;
    // observe its registration commit before performing the card edit here.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let events = f
                .store
                .timeline_by_live_session(b.session, 10)
                .unwrap()
                .unwrap_or_default();
            if events.iter().any(|event| event.r#type == "session_start") {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("registration history event did not commit before the card edit");
    let effects = f
        .engine
        .update_card_meta(
            b.session,
            SessionCardMeta {
                label: "renamed card".into(),
                pinned: true,
                note: "synthetic note".into(),
                ..Default::default()
            },
        )
        .unwrap();
    f.sink.apply(effects).await.unwrap();
    f.sink
        .apply(f.engine.disconnected(b, now()).unwrap())
        .await
        .unwrap();
    let r = reattach(&f, b, b"", 0, 2, 0).await;
    assert_eq!(r.snapshot.label, "renamed card");
    assert_eq!(r.snapshot.launch_label, "launch-label");
    assert!(r.snapshot.pinned);
    assert_eq!(r.snapshot.note, "synthetic note");
}

#[tokio::test]
async fn revoked_reset_and_dismiss_do_not_mutate_retained_wrapper_or_replay() {
    let f = fixture();
    let b = register(&f).await.binding;
    let u = ui(&f, 1);
    f.engine
        .observe_output(
            b,
            OutputChunk {
                bytes: b"retained".to_vec(),
                total_pty_bytes: 8,
            },
            now(),
        )
        .unwrap();
    f.engine.invalidate_all_ui();
    assert!(matches!(
        f.engine.reset_history_from_ui(u, b.session, now()),
        Err(SessionError::AuthenticationExpired)
    ));
    assert!(matches!(
        f.engine.dismiss_from_ui(u, b.session, now()),
        Err(SessionError::AuthenticationExpired)
    ));
    assert!(f.engine.is_current(b));
    let new = UiBinding {
        connection: UiConnectionId(2),
        auth_epoch: f.engine.auth_epoch(),
    };
    let replay = f.engine.attach_ui(new, Some(b.session), None).unwrap();
    assert!(replay.ordered_frames.iter().any(|m| m.data == b"retained"));
}

#[tokio::test]
async fn advisory_native_legacy_sig_consumes_record_without_sending_and_persists_sent_text() {
    let f = fixture();
    let r = f
        .engine
        .register(
            RegisterRequest {
                message: proto::Message {
                    provider: "claude".into(),
                    cwd: "/fixture/project".into(),
                    pid: 7,
                    cols: 120,
                    rows: 30,
                    ..Default::default()
                },
                spawn_proof: None,
            },
            WrapperConnectionId(1),
            now(),
        )
        .await
        .unwrap();
    f.sink.apply(r.after_registered).await.unwrap();
    let b = r.binding;
    let u = ui(&f, 1);
    let bytes = "Run: git status\r\nDo you want to proceed?\r\n❯ 1. Yes\r\n 2. No"
        .as_bytes()
        .to_vec();
    let effects = f
        .engine
        .observe_output(
            b,
            OutputChunk {
                total_pty_bytes: bytes.len() as i64,
                bytes,
            },
            now(),
        )
        .unwrap();
    f.sink.apply(effects).await.unwrap();
    let record = f
        .engine
        .details(b.session)
        .unwrap()
        .approval
        .record
        .unwrap();
    let effects = f
        .engine
        .consume_approval(
            u,
            proto::Message {
                session_id: b.session.0,
                approval_sig: record.data().sig.clone(),
                sent_text: "yes sent once".into(),
                ..Default::default()
            },
            now(),
        )
        .unwrap();
    f.sink.apply(effects).await.unwrap();
    assert!(
        f.engine
            .details(b.session)
            .unwrap()
            .approval
            .record
            .is_none()
    );
    assert!(f.transport.frames.lock().unwrap().is_empty());
    let rows = f
        .store
        .approvals_by_live_session(b.session, 10, false)
        .unwrap()
        .unwrap();
    assert_eq!(rows[0].selected_text, "yes sent once");
}

#[tokio::test]
async fn revocation_waits_for_accepted_paste_but_rejects_unaccepted_queued_input() {
    let f = fixture();
    let b = register(&f).await.binding;
    let u = ui(&f, 1);
    f.transport.pause_call.store(1, Ordering::SeqCst);
    let engine = f.engine.clone();
    let accepted = tokio::spawn(async move {
        engine
            .submit(
                b,
                InputRequest {
                    bytes: b"\x1b[200~accepted\x1b[201~\r".to_vec(),
                    authority: InputAuthority::Ui(u),
                },
                now(),
                &TaskCancellation::default(),
            )
            .await
    });
    f.transport.entered.notified().await;
    let c = TaskCancellation::default();
    let unaccepted = f.engine.submit(
        b,
        InputRequest {
            bytes: b"must not send".to_vec(),
            authority: InputAuthority::Ui(u),
        },
        now(),
        &c,
    );
    let effects = f.engine.invalidate_all_ui();
    assert!(matches!(effects.0[0],CoreEffect::DrainUi(binding) if binding==u));
    let engine = f.engine.clone();
    let drain = tokio::spawn(async move {
        engine.drain_ui_work(u).await;
    });
    tokio::task::yield_now().await;
    assert!(!drain.is_finished());
    f.transport.resume.notify_one();
    assert!(matches!(
        accepted.await.unwrap().disposition,
        InputDisposition::TransportWritten { .. }
    ));
    assert_eq!(
        unaccepted.await.disposition,
        InputDisposition::AuthenticationExpired
    );
    drain.await.unwrap();
    let frames = f.transport.frames.lock().unwrap();
    assert!(frames.iter().any(|(_, m)| m.data == b"\r"));
    assert!(!frames.iter().any(|(_, m)| m.data == b"must not send"));
}

#[tokio::test]
async fn abort_during_transport_write_releases_only_unsent_frame_and_fifo_ticket() {
    let f = fixture();
    let b = register(&f).await.binding;
    f.engine.acknowledge(b, InputSeq(0));
    f.transport.pause_call.store(1, Ordering::SeqCst);
    let engine = f.engine.clone();
    let sending = tokio::spawn(async move {
        engine
            .submit(
                b,
                InputRequest {
                    bytes: b"never written".to_vec(),
                    authority: InputAuthority::Internal,
                },
                now(),
                &TaskCancellation::default(),
            )
            .await
    });
    f.transport.entered.notified().await;
    sending.abort();
    assert!(sending.await.unwrap_err().is_cancelled());
    let r = reattach(&f, b, b"", 0, 2, 0).await;
    f.engine
        .flush(r.binding, &TaskCancellation::default())
        .await;
    assert!(f.transport.frames.lock().unwrap().is_empty());
    let receipt = f
        .engine
        .submit(
            r.binding,
            InputRequest {
                bytes: b"next".to_vec(),
                authority: InputAuthority::Internal,
            },
            now(),
            &TaskCancellation::default(),
        )
        .await;
    assert!(matches!(
        receipt.disposition,
        InputDisposition::TransportWritten { .. }
    ));
}

#[tokio::test]
async fn reattach_ack_is_exact_minimal_go_frame_and_dismissal_precedes_bad_replay() {
    let f = fixture();
    let b = register(&f).await.binding;
    let r = reattach(&f, b, b"", 0, 2, 0).await;
    assert_eq!(
        serde_json::to_value(&r.reattached).unwrap(),
        serde_json::from_str::<serde_json::Value>(include_str!(
            "fixtures/core/session/handshake-oracle-21d0bc7.json"
        ))
        .unwrap()["reattach_ack"]
    );
    f.engine.disconnected(r.binding, now()).unwrap();
    f.engine.dismiss(b.session, now()).unwrap();
    let result = f
        .engine
        .reattach(
            ReattachRequest {
                restored_metadata: None,
                message: proto::Message {
                    session_id: b.session.0,
                    replay_b64: "not valid base64!".into(),
                    ..Default::default()
                },
            },
            WrapperConnectionId(3),
            now(),
        )
        .await;
    assert!(
        matches!(result,Err(SessionError::InvalidRequest(detail))if detail=="session dismissed")
    );
}

#[tokio::test]
async fn output_history_keeps_every_raw_byte_across_split_utf8_chunks() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let f = fixture();
    let r = register(&f).await;
    let b = r.binding;
    let raw = "始め 👩🏽‍🚀 e\u{301} 終わり".as_bytes();
    for (i, byte) in raw.iter().enumerate() {
        let effects = f
            .engine
            .observe_output(
                b,
                OutputChunk {
                    bytes: vec![*byte],
                    total_pty_bytes: (i + 1) as i64,
                },
                now(),
            )
            .unwrap();
        f.sink.apply(effects).await.unwrap();
    }
    let records = std::fs::read_to_string(&r.snapshot.jsonl_path).unwrap();
    let mut replay = Vec::new();
    for line in records.lines() {
        let event: serde_json::Value = serde_json::from_str(line).unwrap();
        if event["type"] == "pty_output" {
            replay.extend(
                STANDARD
                    .decode(event["data_b64"].as_str().unwrap())
                    .unwrap(),
            );
        }
    }
    assert_eq!(replay, raw);
}

#[tokio::test]
async fn delayed_old_disconnect_cannot_close_rebound_journal_or_end_restored_database_row() {
    let f = fixture();
    let b = register(&f).await.binding;
    let old_effects = f.engine.disconnected(b, now()).unwrap();
    let reattaching = reattach(&f, b, b"", 0, 2, 0);
    tokio::pin!(reattaching);
    assert!(
        tokio::time::timeout(Duration::from_millis(2), &mut reattaching)
            .await
            .is_err()
    );
    f.sink.apply(old_effects).await.unwrap();
    let r = reattaching.await;
    let row = f.store.session_overview_by_live_session(b.session).unwrap();
    assert_eq!(row.state, "running");
    assert!(row.ended_at.is_empty());
    f.sink
        .apply(
            f.engine
                .observe_output(
                    r.binding,
                    OutputChunk {
                        bytes: b"new connection output".to_vec(),
                        total_pty_bytes: 21,
                    },
                    now(),
                )
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        std::fs::read_to_string(r.snapshot.jsonl_path)
            .unwrap()
            .contains("new connection output")
    );
}

#[tokio::test]
async fn provider_output_during_send_is_persisted_before_post_send_user_history() {
    let f = fixture();
    let r = register(&f).await;
    let b = r.binding;
    let u = ui(&f, 1);
    let weak = Arc::downgrade(&f.engine);
    let sink = f.sink.clone();
    *f.transport.on_send.lock().unwrap() = Some(Arc::new(move |binding, _message| {
        let weak = weak.clone();
        let sink = sink.clone();
        Box::pin(async move {
            let engine = weak.upgrade().unwrap();
            let effects = engine
                .observe_output(
                    binding,
                    OutputChunk {
                        bytes: b"provider output".to_vec(),
                        total_pty_bytes: 15,
                    },
                    now(),
                )
                .unwrap();
            sink.apply(effects).await.unwrap();
        })
    }));
    let receipt = f
        .engine
        .submit(
            b,
            InputRequest {
                bytes: b"send this turn\r".to_vec(),
                authority: InputAuthority::Ui(u),
            },
            now(),
            &TaskCancellation::default(),
        )
        .await;
    assert!(matches!(
        receipt.disposition,
        InputDisposition::TransportWritten { .. }
    ));
    let lines = std::fs::read_to_string(r.snapshot.jsonl_path).unwrap();
    let kinds: Vec<String> = lines
        .lines()
        .map(|line| {
            serde_json::from_str::<serde_json::Value>(line).unwrap()["type"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    let output = kinds.iter().position(|kind| kind == "pty_output").unwrap();
    let input = kinds.iter().position(|kind| kind == "user_input").unwrap();
    assert!(output < input);
}

#[tokio::test]
async fn marker_answer_and_history_reset_clear_waiting_activity_immediately() {
    let f = fixture();
    let b = register(&f).await.binding;
    let u = ui(&f, 1);
    let bytes =
        b"[MANY-AI-CLI]\r\nQ1 Choose a mode?\r\n1. Keep\r\n2. Change\r\n[/MANY-AI-CLI]".to_vec();
    f.sink
        .apply(
            f.engine
                .observe_output(
                    b,
                    OutputChunk {
                        total_pty_bytes: bytes.len() as i64,
                        bytes: bytes.clone(),
                    },
                    now(),
                )
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        f.engine
            .details(b.session)
            .unwrap()
            .approval
            .record
            .is_some()
    );
    assert!(
        f.engine
            .snapshot(b.session)
            .unwrap()
            .activity
            .awaiting_approval
    );
    f.engine
        .submit(
            b,
            InputRequest {
                bytes: b"1\r".to_vec(),
                authority: InputAuthority::Ui(u),
            },
            now(),
            &TaskCancellation::default(),
        )
        .await;
    assert!(
        f.engine
            .details(b.session)
            .unwrap()
            .approval
            .record
            .is_none()
    );
    assert!(
        !f.engine
            .snapshot(b.session)
            .unwrap()
            .activity
            .awaiting_approval
    );
    f.sink
        .apply(f.engine.reset_history(b.session, now()).unwrap())
        .await
        .unwrap();
    f.sink
        .apply(
            f.engine
                .observe_output(
                    b,
                    OutputChunk {
                        total_pty_bytes: (bytes.len() * 2) as i64,
                        bytes,
                    },
                    now(),
                )
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        f.engine
            .snapshot(b.session)
            .unwrap()
            .activity
            .awaiting_approval
    );
    f.sink
        .apply(f.engine.reset_history(b.session, now()).unwrap())
        .await
        .unwrap();
    assert!(
        f.engine
            .details(b.session)
            .unwrap()
            .approval
            .record
            .is_none()
    );
    assert!(
        !f.engine
            .snapshot(b.session)
            .unwrap()
            .activity
            .awaiting_approval
    );
}

#[tokio::test]
async fn warm_reattach_preserves_conversation_activity_and_original_output_clock() {
    use base64::Engine;
    // Exercise both empty replay and replay with a gap: neither is a new live
    // output observation when the conversation already has retained state.
    for replay in [b"".as_slice(), b"\r\nreplayed gap".as_slice()] {
        let f = fixture();
        let registered = register(&f).await;
        let binding = registered.binding;
        let output_at = now() + Duration::from_secs(1);
        let marker =
            b"[MANY-AI-CLI]\r\nQ1 Choose a mode?\r\n1. Keep\r\n2. Change\r\n[/MANY-AI-CLI]";
        f.sink
            .apply(
                f.engine
                    .observe_output(
                        binding,
                        OutputChunk {
                            bytes: marker.to_vec(),
                            total_pty_bytes: marker.len() as i64,
                        },
                        output_at,
                    )
                    .unwrap(),
            )
            .await
            .unwrap();
        for observation in [
            SessionObservation::Messages {
                first: "first conversation message".into(),
                last: "latest conversation message".into(),
            },
            SessionObservation::Transcript {
                path: f._root.path().join("synthetic-transcript.jsonl"),
                agent_session_id: "synthetic-agent".into(),
                safe_offset: 17,
                grew_at: "2026-01-02T03:04:07Z".into(),
            },
            SessionObservation::BoardNotifyPending(true),
            SessionObservation::CrossSessionMessage(proto::CrossSessionMessage {
                sender: "synthetic-sender".into(),
                text: "keep this conversation notice".into(),
                ..Default::default()
            }),
        ] {
            f.sink
                .apply(
                    f.engine
                        .apply_observation(binding, observation, output_at)
                        .unwrap(),
                )
                .await
                .unwrap();
        }
        f.sink
            .apply(
                f.engine
                    .observe_end(
                        binding,
                        SessionEnd {
                            declared_state: "completed".into(),
                            exit_code: 0,
                            reason: "retained completion reason".into(),
                        },
                        output_at,
                    )
                    .unwrap(),
            )
            .await
            .unwrap();
        let before = f.engine.snapshot(binding.session).unwrap();
        assert!(before.activity.awaiting_approval);
        assert!(!before.activity.output_idle);
        let reattached_at = now() + Duration::from_secs(20);
        let result = f
            .engine
            .reattach(
                ReattachRequest {
                    restored_metadata: None,
                    message: proto::Message {
                        session_id: binding.session.0,
                        provider: "copilot".into(),
                        cwd: "/fixture/project".into(),
                        label: "launch-label".into(),
                        pid: 7,
                        cols: 120,
                        rows: 30,
                        // The source keeps the nonempty old StartedAt on a warm reattach.
                        started_at: proto::time::format_rfc3339(reattached_at).unwrap(),
                        replay_b64: base64::engine::general_purpose::STANDARD.encode(replay),
                        pty_bytes: (marker.len() + replay.len()) as i64,
                        ..Default::default()
                    },
                },
                WrapperConnectionId(2),
                reattached_at,
            )
            .await
            .unwrap();
        let after = &result.snapshot;
        assert_eq!(after.state, "running");
        assert!(after.activity == before.activity);
        assert_eq!(after.last_output_at, before.last_output_at);
        assert_eq!(after.transcript_grew_at, before.transcript_grew_at);
        assert_eq!(after.started_at, before.started_at);
        assert_eq!(after.first_message, before.first_message);
        assert_eq!(after.last_message, before.last_message);
        assert_eq!(after.end_reason, before.end_reason);
        assert!(after.board_notify_pending);
        assert!(after.cross_session_messages == before.cross_session_messages);
        assert!(
            f.engine
                .details(binding.session)
                .unwrap()
                .approval
                .record
                .is_some()
        );
        f.sink.apply(result.after_reattached).await.unwrap();
        // This also verifies the private last_output clock was retained: using
        // the reconnect time would leave output non-idle for another 3 seconds.
        f.sink
            .apply(
                f.engine
                    .evaluate_idle(reattached_at + Duration::from_secs(1)),
            )
            .await
            .unwrap();
        assert!(
            f.engine
                .snapshot(binding.session)
                .unwrap()
                .activity
                .output_idle
        );
    }
}

#[tokio::test]
async fn exhausted_registration_id_does_not_leak_provider_admission() {
    let f = fixture();
    let restored = f
        .engine
        .reattach(
            ReattachRequest {
                restored_metadata: None,
                message: proto::Message {
                    session_id: i64::MAX,
                    provider: "copilot".into(),
                    cwd: "/fixture/restored".into(),
                    cols: 80,
                    rows: 24,
                    ..Default::default()
                },
            },
            WrapperConnectionId(1),
            now(),
        )
        .await
        .unwrap();
    assert_eq!(restored.binding.session, LiveSessionId(i64::MAX));
    // Dropping unapplied effects releases only their ordering tickets.
    drop(restored);
    let failed = f
        .engine
        .register(
            RegisterRequest {
                message: proto::Message {
                    provider: "codex".into(),
                    cwd: "/fixture/new".into(),
                    ..Default::default()
                },
                spawn_proof: None,
            },
            WrapperConnectionId(2),
            now(),
        )
        .await;
    assert!(
        matches!(failed, Err(SessionError::InvalidRequest(message)) if message == "session identity exhausted")
    );
    let update = f
        .engine
        .begin_provider_update("codex")
        .expect("failed registration must not reserve provider spawn");
    f.engine.end_provider_update(update);
}

#[tokio::test]
async fn failed_separate_submit_enter_retains_settle_and_confirmation_on_flush() {
    let f = fixture();
    let binding = register(&f).await.binding;
    let cancel = TaskCancellation::default();
    let request = |bytes: &[u8]| InputRequest {
        bytes: bytes.to_vec(),
        authority: InputAuthority::Internal,
    };
    let body = f
        .engine
        .submit(binding, request(b"\x1b[200~body\x1b[201~"), now(), &cancel)
        .await;
    assert_eq!(body.submit, SubmitReceipt::PendingEnter);
    f.transport.fail.store(2, Ordering::SeqCst);
    let failed = f
        .engine
        .submit(binding, request(b"\r"), now(), &cancel)
        .await;
    assert!(
        matches!(failed.disposition, InputDisposition::Failed { unsent_remainder, .. } if unsent_remainder == b"\r")
    );
    f.transport.fail.store(0, Ordering::SeqCst);
    f.engine.flush(binding, &cancel).await;
    let frames = f.transport.frames.lock().unwrap();
    assert_eq!(
        frames
            .iter()
            .map(|(_, frame)| frame.data.as_slice())
            .collect::<Vec<_>>(),
        [
            b"\x1b[200~body\x1b[201~".as_slice(),
            b"\r".as_slice(),
            b"\r".as_slice()
        ]
    );
    assert_eq!(frames[2].1.input_seq, frames[1].1.input_seq + 1);
}

#[tokio::test]
async fn details_exposes_exact_output_clock_without_rounding_for_routines() {
    let f = fixture();
    let binding = register(&f).await.binding;
    assert_eq!(
        f.engine.details(binding.session).unwrap().last_output_at,
        None
    );
    let precise = now() + Duration::from_nanos(123_456_789);
    let effects = f
        .engine
        .observe_output(
            binding,
            OutputChunk {
                bytes: b"clock".to_vec(),
                total_pty_bytes: 5,
            },
            precise,
        )
        .unwrap();
    f.sink.apply(effects).await.unwrap();
    assert_eq!(
        f.engine.details(binding.session).unwrap().last_output_at,
        Some(precise)
    );
}
#[path = "session_engine/native_start.rs"]
mod native_start;

#[tokio::test]
async fn recovered_relay_metadata_is_persisted_before_ack_and_warm_reattach_does_not_erase_it() {
    let f = fixture();
    let at = now();
    let mut message = proto::Message {
        session_id: 42,
        provider: "copilot".into(),
        cwd: "/synthetic-project".into(),
        pid: 42,
        started_at: many_ai_cli::proto::time::format_rfc3339(at).unwrap(),
        ..Default::default()
    };
    let cold = f
        .engine
        .reattach(
            ReattachRequest {
                message: message.clone(),
                restored_metadata: Some(SpawnRegistrationMetadata {
                    role: "review".into(),
                    auto: true,
                    depth: 1,
                    orchestration: OrchestrationId("synthetic-relay".into()),
                    board_path: "synthetic-board".into(),
                    ..Default::default()
                }),
            },
            WrapperConnectionId(42),
            at,
        )
        .await
        .unwrap();
    let stored = f
        .store
        .session_overview_by_live_session(cold.binding.session)
        .unwrap();
    assert_eq!(stored.role, "review");
    assert_eq!(
        stored.orchestration_id,
        OrchestrationId("synthetic-relay".into())
    );
    // Drain the cold registration's ordered effects before enqueueing a later
    // metadata write; retaining its persistence ticket would block that write.
    f.sink.apply(cold.after_reattached).await.unwrap();
    let parent = register(&f).await;
    let effects = f
        .engine
        .apply_relay_binding(
            cold.binding,
            parent.binding.session,
            OrchestrationId("synthetic-relay".into()),
            "synthetic-board".into(),
            "review".into(),
            at,
        )
        .unwrap();
    f.sink.apply(effects).await.unwrap();
    assert_eq!(
        f.store
            .session_overview_by_session_id(stored.id)
            .unwrap()
            .parent_session_id,
        parent.binding.session
    );
    assert!(
        f.engine
            .reserve_children(
                AdmissionRequest {
                    parent: parent.binding.session,
                    slots: 1,
                    origin: VerifiedSpawnOrigin::Autonomous,
                    replace: None
                },
                AdmissionLimits {
                    max_children_per_parent: 1,
                    max_total_sessions: 100
                }
            )
            .is_err()
    );
    message.session_id = cold.binding.session.0;
    let warm = f
        .engine
        .reattach(
            ReattachRequest {
                message,
                restored_metadata: None,
            },
            WrapperConnectionId(43),
            at,
        )
        .await
        .unwrap();
    assert_eq!(warm.snapshot.role, "review");
    assert_eq!(
        f.store
            .session_overview_by_session_id(stored.id)
            .unwrap()
            .parent_session_id,
        parent.binding.session
    );
    assert!(matches!(
        f.engine.apply_relay_binding(
            cold.binding,
            LiveSessionId(999),
            OrchestrationId("wrong".into()),
            "wrong".into(),
            "review".into(),
            at
        ),
        Err(SessionError::StaleBinding)
    ));
}
