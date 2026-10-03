use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize};

#[derive(Clone, Default)]
struct Recording {
    frames: Arc<Mutex<Vec<WireFrame>>>,
    fail_at: Option<usize>,
}
impl FrameWriter for Recording {
    fn write<'a>(&'a mut self, frame: WireFrame) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            let mut frames = self.frames.lock().unwrap();
            if self.fail_at == Some(frames.len()) {
                return Err(SessionError::Transport("synthetic writer failure".into()));
            }
            frames.push(frame);
            Ok(())
        })
    }
}
fn registry() -> Arc<SocketRegistry> {
    let registry = Arc::new(SocketRegistry::default());
    assert!(registry.authority.set(Arc::new(|_| true)).is_ok());
    registry
}
fn wrapper(connection: u64) -> SessionBinding {
    SessionBinding {
        session: LiveSessionId(1),
        incarnation: SessionIncarnation(connection),
        wrapper: WrapperConnectionId(connection),
    }
}
fn ui() -> UiBinding {
    UiBinding {
        connection: UiConnectionId(4),
        auth_epoch: AuthEpoch(2),
    }
}
fn message(kind: &str) -> proto::Message {
    proto::Message {
        r#type: kind.into(),
        ..Default::default()
    }
}
fn types(recording: &Recording) -> Vec<String> {
    recording
        .frames
        .lock()
        .unwrap()
        .iter()
        .map(|frame| match frame {
            WireFrame::Text(text) => {
                serde_json::from_str::<serde_json::Value>(text).unwrap()["type"]
                    .as_str()
                    .unwrap()
                    .into()
            }
            WireFrame::Close => "CLOSE".into(),
        })
        .collect()
}
#[tokio::test]
async fn register_ack_precedes_waiting_wrapper_effects() {
    let registry = registry();
    let recording = Recording::default();
    let binding = wrapper(1);
    registry
        .insert_wrapper(binding, Box::new(recording.clone()))
        .unwrap();
    let waiting = tokio::spawn({
        let registry = registry.clone();
        async move { registry.send(binding, message("pty_input")).await }
    });
    tokio::task::yield_now().await;
    assert!(types(&recording).is_empty());
    registry
        .acknowledge_wrapper(binding, message("registered"))
        .await
        .unwrap();
    waiting.await.unwrap().unwrap();
    assert_eq!(types(&recording), ["registered", "pty_input"]);
    assert!(
        registry
            .acknowledge_wrapper(binding, message("registered"))
            .await
            .is_err()
    );
}
#[tokio::test]
async fn close_exact_stale_wrapper_retains_replacement_and_unblocks_gate() {
    let registry = registry();
    let old = Recording::default();
    let new = Recording::default();
    let old_binding = wrapper(1);
    let new_binding = wrapper(2);
    registry
        .insert_wrapper(old_binding, Box::new(old.clone()))
        .unwrap();
    registry
        .insert_wrapper(new_binding, Box::new(new.clone()))
        .unwrap();
    let waiting = tokio::spawn({
        let registry = registry.clone();
        async move { registry.send(old_binding, message("pty_input")).await }
    });
    registry.close_wrapper(old_binding).await.unwrap();
    assert!(waiting.await.unwrap().is_err());
    registry
        .acknowledge_wrapper(new_binding, message("reattach_ack"))
        .await
        .unwrap();
    registry
        .send(new_binding, message("pty_input"))
        .await
        .unwrap();
    assert_eq!(types(&old), ["CLOSE"]);
    assert_eq!(types(&new), ["reattach_ack", "pty_input"]);
    let forged = SessionBinding {
        incarnation: SessionIncarnation(99),
        ..new_binding
    };
    registry.close_wrapper(forged).await.unwrap();
    registry
        .send(new_binding, message("pty_resize"))
        .await
        .unwrap();
}
#[tokio::test]
async fn authority_is_rechecked_after_writer_lock_wait() {
    let registry = Arc::new(SocketRegistry::default());
    let live = Arc::new(AtomicBool::new(true));
    assert!(
        registry
            .authority
            .set({
                let live = live.clone();
                Arc::new(move |_| live.load(Ordering::SeqCst))
            })
            .is_ok()
    );
    let recording = Recording::default();
    registry
        .insert_ui(ui(), Box::new(recording.clone()))
        .unwrap();
    let socket = registry.socket(Binding::Ui(ui())).unwrap();
    let lock = socket.writer.lock().await;
    let pending = tokio::spawn({
        let registry = registry.clone();
        async move { registry.send_ui(ui(), message("approval")).await }
    });
    tokio::task::yield_now().await;
    live.store(false, Ordering::SeqCst);
    drop(lock);
    assert_eq!(
        pending.await.unwrap(),
        Err(SessionError::AuthenticationExpired)
    );
    assert!(types(&recording).is_empty());
}
#[tokio::test]
async fn hub_shutdown_preserves_socket_and_never_sends_dismissal() {
    let registry = registry();
    let recording = Recording::default();
    let binding = wrapper(1);
    registry
        .insert_wrapper(binding, Box::new(recording.clone()))
        .unwrap();
    registry
        .acknowledge_wrapper(binding, message("registered"))
        .await
        .unwrap();
    registry.notify_hub_shutdown("ui_shutdown").await.unwrap();
    assert_eq!(types(&recording), ["registered", "hub_shutdown"]);
    assert!(registry.socket(Binding::Wrapper(binding)).is_ok());
    assert!(
        registry
            .cancel_session(binding, StopReason::HubShutdown)
            .await
            .is_err()
    );
    registry
        .cancel_session(binding, StopReason::Dismissed)
        .await
        .unwrap();
    assert_eq!(
        types(&recording),
        ["registered", "hub_shutdown", "session_dismissed", "CLOSE"]
    );
    let frames = recording.frames.lock().unwrap();
    let WireFrame::Text(text) = &frames[2] else {
        panic!("intent frame expected")
    };
    let value: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(value["reason"], "ui_dismiss");
}
struct Stuck;
impl FrameWriter for Stuck {
    fn write<'a>(&'a mut self, _: WireFrame) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(std::future::pending())
    }
}
#[tokio::test]
async fn write_timeout_cancels_the_reader_and_does_not_report_success() {
    let registry = registry();
    let cancelled = registry.insert_ui(ui(), Box::new(Stuck)).unwrap();
    assert_eq!(
        registry
            .send_frame(
                Binding::Ui(ui()),
                WireFrame::Text("{}".into()),
                Some(Duration::from_millis(10)),
                true
            )
            .await,
        Err(SessionError::TimedOut)
    );
    assert!(cancelled.is_cancelled());
}
#[test]
fn no_authority_is_fail_closed_and_connection_ids_are_unique() {
    let registry = SocketRegistry::default();
    assert!(!registry.authorized(Binding::Ui(ui())));
    assert_eq!(registry.next_ui().unwrap().0, 1);
    assert_eq!(registry.next_wrapper().unwrap().0, 2);
    assert!(stop_reason(StopReason::User).is_err());
}
#[derive(Default)]
struct Dependencies {
    log: Mutex<Vec<&'static str>>,
    fail_persist: AtomicBool,
    observations: AtomicUsize,
}
impl PersistenceEffectSink for Dependencies {
    fn apply_bound(
        &self,
        binding: SessionBinding,
        scope: PersistenceBindingScope,
        _: PersistenceEffect,
    ) -> Result<(), SessionError> {
        assert_eq!(binding, wrapper(1));
        assert_eq!(scope, PersistenceBindingScope::ExactWrapper);
        self.log.lock().unwrap().push("persist_bound");
        if self.fail_persist.load(Ordering::SeqCst) {
            Err(SessionError::Transport("synthetic bound failure".into()))
        } else {
            Ok(())
        }
    }
    fn apply(&self, _: PersistenceEffect) -> Result<(), SessionError> {
        self.log.lock().unwrap().push("persist");
        if self.fail_persist.load(Ordering::SeqCst) {
            Err(SessionError::Transport(
                "synthetic persistence failure".into(),
            ))
        } else {
            Ok(())
        }
    }
}
impl CoreEventPublisher for Dependencies {
    fn publish(&self, _: CoreEvent) -> Result<EventSequence, SessionError> {
        assert!(self.observations.load(Ordering::SeqCst) > 0);
        self.log.lock().unwrap().push("publish");
        Ok(EventSequence(1))
    }
}
impl OrderedEventObserver for Dependencies {
    fn observe<'a>(&'a self, _: &'a CoreEvent) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            tokio::task::yield_now().await;
            self.observations.fetch_add(1, Ordering::SeqCst);
            self.log.lock().unwrap().push("observe");
            Ok(())
        })
    }
}
#[tokio::test]
async fn ordered_batch_reports_failed_index_and_never_applies_later_effects() {
    let registry = registry();
    let recording = Recording::default();
    registry
        .insert_ui(ui(), Box::new(recording.clone()))
        .unwrap();
    let dependencies = Arc::new(Dependencies::default());
    dependencies.fail_persist.store(true, Ordering::SeqCst);
    let driver = EffectDriver::new(
        registry,
        dependencies.clone(),
        dependencies.clone(),
        dependencies.clone(),
    );
    let result = driver
        .apply(CoreEffects(vec![
            CoreEffect::SendUi {
                binding: ui(),
                message: message("first"),
            },
            CoreEffect::Persist(PersistenceEffect::ClearSessionHistory(LiveSessionId(1))),
            CoreEffect::Notify(CoreEvent::HistoryReset(LiveSessionId(1))),
            CoreEffect::SendUi {
                binding: ui(),
                message: message("last"),
            },
        ]))
        .await
        .unwrap_err();
    assert_eq!(result.index, 1);
    assert_eq!(types(&recording), ["first"]);
    assert_eq!(*dependencies.log.lock().unwrap(), ["persist"]);
}
#[tokio::test]
async fn ordered_observer_finishes_before_event_publication() {
    let dependencies = Arc::new(Dependencies::default());
    let driver = EffectDriver::new(
        registry(),
        dependencies.clone(),
        dependencies.clone(),
        dependencies.clone(),
    );
    driver
        .apply(CoreEffects(vec![CoreEffect::Notify(
            CoreEvent::GitTurnCapture {
                binding: wrapper(1),
                started_at: "2026-01-01T00:00:00Z".into(),
                ended_at: None,
            },
        )]))
        .await
        .unwrap();
    assert_eq!(*dependencies.log.lock().unwrap(), ["observe", "publish"]);
}
#[tokio::test]
async fn broadcast_without_core_priming_is_rejected() {
    let dependencies = Arc::new(Dependencies::default());
    let driver = EffectDriver::new(
        registry(),
        dependencies.clone(),
        dependencies.clone(),
        dependencies,
    );
    let failure = driver
        .apply(CoreEffects(vec![CoreEffect::Broadcast(message("update"))]))
        .await
        .unwrap_err();
    assert_eq!(failure.index, 0);
    assert!(matches!(failure.error, SessionError::InvalidRequest(_)));
}
#[tokio::test]
async fn snapshot_envelope_does_not_inherit_message_zero_fields() {
    let registry = registry();
    let recording = Recording::default();
    registry
        .insert_ui(ui(), Box::new(recording.clone()))
        .unwrap();
    registry
        .send_ui_value(
            ui(),
            &serde_json::json!({"type":"snapshot","hub_instance":"synthetic","sessions":[]}),
        )
        .await
        .unwrap();
    let frames = recording.frames.lock().unwrap();
    let WireFrame::Text(text) = &frames[0] else {
        panic!("snapshot frame expected")
    };
    assert!(!text.contains("token_statusbar"));
}

#[tokio::test]
async fn only_explicit_best_effort_ui_failure_continues_the_batch() {
    let registry = registry();
    let writer = Recording {
        fail_at: Some(0),
        ..Default::default()
    };
    let closed = registry.insert_ui(ui(), Box::new(writer)).unwrap();
    let dependencies = Arc::new(Dependencies::default());
    let warnings = Arc::new(AtomicUsize::new(0));
    let driver = EffectDriver::new(
        registry.clone(),
        dependencies.clone(),
        dependencies.clone(),
        dependencies.clone(),
    )
    .with_warning_handler({
        let warnings = warnings.clone();
        Arc::new(move |_, _| {
            warnings.fetch_add(1, Ordering::SeqCst);
        })
    });
    driver
        .apply(CoreEffects(vec![
            CoreEffect::SendUiBestEffort {
                binding: ui(),
                message: message("broadcast"),
            },
            CoreEffect::Persist(PersistenceEffect::ClearSessionHistory(LiveSessionId(1))),
        ]))
        .await
        .unwrap();
    assert!(closed.is_cancelled());
    assert_eq!(warnings.load(Ordering::SeqCst), 1);
    assert_eq!(*dependencies.log.lock().unwrap(), ["persist"]);
    let failure = driver
        .apply(CoreEffects(vec![
            CoreEffect::SendUi {
                binding: ui(),
                message: message("direct"),
            },
            CoreEffect::Persist(PersistenceEffect::ClearSessionHistory(LiveSessionId(1))),
        ]))
        .await
        .unwrap_err();
    assert_eq!(failure.index, 0);
    assert_eq!(*dependencies.log.lock().unwrap(), ["persist"]);
}

#[tokio::test]
async fn reserved_connection_covers_core_publication_before_full_binding() {
    let registry = registry();
    let recording = Recording::default();
    let binding = wrapper(1);
    registry
        .reserve_wrapper(binding.wrapper, Box::new(recording.clone()))
        .unwrap();
    // This is the window after core publishes the session but before register
    // returns its full binding to the transport. It must wait, not fail/drop.
    let input = tokio::spawn({
        let registry = registry.clone();
        async move { registry.send(binding, message("pty_input")).await }
    });
    tokio::task::yield_now().await;
    assert!(!input.is_finished());
    registry.bind_wrapper(binding).unwrap();
    assert!(types(&recording).is_empty());
    registry
        .acknowledge_wrapper(binding, message("registered"))
        .await
        .unwrap();
    input.await.unwrap().unwrap();
    assert_eq!(types(&recording), ["registered", "pty_input"]);
    let failed = wrapper(2);
    registry
        .reserve_wrapper(failed.wrapper, Box::new(Recording::default()))
        .unwrap();
    let waiting = tokio::spawn({
        let registry = registry.clone();
        async move { registry.send(failed, message("pty_resize")).await }
    });
    registry
        .close_pending_wrapper(failed.wrapper)
        .await
        .unwrap();
    assert!(waiting.await.unwrap().is_err());
}

struct Order(Arc<Dependencies>);
impl PersistenceOrder for Order {
    fn wait(&self) -> CoreFuture<'_, ()> {
        Box::pin(async {
            self.0.log.lock().unwrap().push("ticket_wait");
        })
    }
}
impl Drop for Order {
    fn drop(&mut self) {
        self.0.log.lock().unwrap().push("ticket_drop");
    }
}
#[tokio::test]
async fn bound_persistence_waits_and_releases_ticket_before_later_effects() {
    let dependencies = Arc::new(Dependencies::default());
    let driver = EffectDriver::new(
        registry(),
        dependencies.clone(),
        dependencies.clone(),
        dependencies.clone(),
    );
    driver
        .apply(CoreEffects(vec![
            CoreEffect::PersistBound {
                binding: wrapper(1),
                scope: PersistenceBindingScope::ExactWrapper,
                effect: PersistenceEffect::ClearSessionHistory(LiveSessionId(1)),
                order: Box::new(Order(dependencies.clone())),
            },
            CoreEffect::Notify(CoreEvent::HistoryReset(LiveSessionId(1))),
        ]))
        .await
        .unwrap();
    assert_eq!(
        *dependencies.log.lock().unwrap(),
        [
            "ticket_wait",
            "persist_bound",
            "ticket_drop",
            "observe",
            "publish"
        ]
    );
    dependencies.log.lock().unwrap().clear();
    let not_polled = driver.apply(CoreEffects(vec![CoreEffect::PersistBound {
        binding: wrapper(1),
        scope: PersistenceBindingScope::ExactWrapper,
        effect: PersistenceEffect::ClearSessionHistory(LiveSessionId(1)),
        order: Box::new(Order(dependencies.clone())),
    }]));
    drop(not_polled);
    assert_eq!(*dependencies.log.lock().unwrap(), ["ticket_drop"]);
}
#[tokio::test]
async fn typed_git_turn_keeps_all_zero_counts_and_avoids_message_fields() {
    let registry = registry();
    let recording = Recording::default();
    registry
        .insert_ui(ui(), Box::new(recording.clone()))
        .unwrap();
    let dependencies = Arc::new(Dependencies::default());
    let driver = EffectDriver::new(
        registry,
        dependencies.clone(),
        dependencies.clone(),
        dependencies,
    );
    driver
        .apply(CoreEffects(vec![CoreEffect::SendUiGitTurn {
            binding: ui(),
            event: GitTurnNotification {
                session_id: 7,
                turn: 2,
                started_at: "2026-01-02T00:00:00Z".into(),
                ended_at: "2026-01-02T00:01:00Z".into(),
                ..Default::default()
            },
            best_effort: false,
        }]))
        .await
        .unwrap();
    let frames = recording.frames.lock().unwrap();
    let WireFrame::Text(text) = &frames[0] else {
        panic!("expected turn frame")
    };
    let event: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(event["type"], "git_turn");
    for count in ["files_changed", "added", "removed"] {
        assert_eq!(event[count], 0);
    }
    assert_eq!(event["turn"], 2);
    assert!(event.get("token_statusbar").is_none());
}
