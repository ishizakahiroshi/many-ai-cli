use super::*;
use crate::{
    config::RuntimePaths,
    terminal::{events::CoreEventBus, journal::JournalOptions},
};
struct Io;
impl WrapperTransport for Io {
    fn send<'a>(
        &'a self,
        _: SessionBinding,
        _: proto::Message,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async { Ok(()) })
    }
}
impl CoreEffectSink for Io {
    fn apply<'a>(&'a self, _: CoreEffects) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
        Box::pin(async { Ok(()) })
    }
}
impl WrappedSessionSpawner for Io {
    fn spawn_and_wait<'a>(
        &'a self,
        _: WrappedSpawnSpec,
        _: Duration,
        _: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome> {
        Box::pin(async { SpawnWaitOutcome::Failed("synthetic no provider".into()) })
    }
}
fn now() -> Timestamp {
    Timestamp::from_unix(1791158400, 0).unwrap()
}
fn fixture() -> (tempfile::TempDir, SessionEngine) {
    let root = tempfile::tempdir().unwrap();
    let runtime = root.path().join("trial");
    std::fs::create_dir(&runtime).unwrap();
    let paths = RuntimePaths::trial(&runtime, 49688, &root.path().join("installed")).unwrap();
    let core = SessionEngine::new(
        EngineOptions::default(),
        Arc::new(SessionJournal::new(paths, None, JournalOptions::default())),
        Arc::new(Io),
        Arc::new(Io),
        Arc::new(Io),
        CoreEventBus::new(64).unwrap(),
    );
    (root, core)
}
async fn register(core: &SessionEngine, pid: i64) -> SessionBinding {
    core.register(
        RegisterRequest {
            message: proto::Message {
                provider: "claude".into(),
                pid,
                cwd: "synthetic-observation-cwd".into(),
                cols: 120,
                rows: 30,
                ..Default::default()
            },
            spawn_proof: None,
        },
        WrapperConnectionId(pid as u64),
        now(),
    )
    .await
    .unwrap()
    .binding
}
fn output(core: &SessionEngine, binding: SessionBinding, bytes: Vec<u8>) -> CoreEffects {
    core.observe_output(
        binding,
        OutputChunk {
            bytes,
            total_pty_bytes: 0,
        },
        now(),
    )
    .unwrap()
}

#[tokio::test]
async fn reattach_only_restart_event_follows_existing_registered_in_after_ack_effects() {
    let (_root, core) = fixture();
    let registration = core
        .register(
            RegisterRequest {
                message: proto::Message {
                    provider: "claude".into(),
                    pid: 1711,
                    cwd: "synthetic-observation-cwd".into(),
                    cols: 120,
                    rows: 30,
                    ..Default::default()
                },
                spawn_proof: None,
            },
            WrapperConnectionId(1711),
            now(),
        )
        .await
        .unwrap();
    assert_eq!(registration.registered.r#type, "registered");
    assert!(registration.after_registered.0.iter().any(|effect|matches!(effect,CoreEffect::Notify(CoreEvent::Registered(binding)) if *binding==registration.binding)));
    assert!(
        !registration
            .after_registered
            .0
            .iter()
            .any(|effect| matches!(effect, CoreEffect::Notify(CoreEvent::Reattached(_))))
    );
    let binding = registration.binding;
    let started = registration.snapshot.started_at.clone();
    // Release the first receipt's ordered persistence permit before reattach.
    drop(registration);
    let receipt = core
        .reattach(
            ReattachRequest {
                restored_metadata: None,
                message: proto::Message {
                    session_id: binding.session.0,
                    provider: "claude".into(),
                    pid: 1711,
                    cwd: "synthetic-observation-cwd".into(),
                    started_at: started,
                    cols: 120,
                    rows: 30,
                    ..Default::default()
                },
            },
            WrapperConnectionId(1811),
            now(),
        )
        .await
        .unwrap();
    assert_eq!(receipt.reattached.r#type, "reattach_ack");
    assert_eq!(receipt.binding.incarnation, binding.incarnation);
    assert_ne!(receipt.binding.wrapper, binding.wrapper);
    let events = receipt
        .after_reattached
        .0
        .iter()
        .filter_map(|effect| {
            if let CoreEffect::Notify(event) = effect {
                Some(event)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert!(
        matches!(events.as_slice(),[CoreEvent::Registered(first),CoreEvent::Reattached(second)] if *first==receipt.binding&&*second==receipt.binding)
    );
    assert!(
        matches!(receipt.after_reattached.0.last(),Some(CoreEffect::Notify(CoreEvent::Reattached(current))) if *current==receipt.binding)
    );
}
fn role(core: &SessionEngine, binding: SessionBinding, parent: LiveSessionId, role: &str) {
    let mut state = lock(&core.state);
    let s = state.session(binding).unwrap();
    s.snapshot.orchestration_id = OrchestrationId("synthetic-orch".into());
    s.snapshot.parent_session_id = parent;
    s.snapshot.role = role.into();
}
async fn submit(core: &SessionEngine, binding: SessionBinding, at: Timestamp) {
    let ui = UiBinding {
        connection: UiConnectionId(99),
        auth_epoch: core.auth_epoch(),
    };
    if !lock(&core.state).uis.contains_key(&ui.connection) {
        core.attach_ui(ui, None, None).unwrap();
    }
    let receipt = core
        .submit(
            binding,
            InputRequest {
                bytes: b"confirmed task\r".to_vec(),
                authority: InputAuthority::Ui(ui),
            },
            at,
            &TaskCancellation::default(),
        )
        .await;
    assert!(matches!(
        receipt.disposition,
        InputDisposition::TransportWritten { .. }
    ));
}
#[tokio::test]
async fn atomic_scrollback_view_and_turn_admission_allow_new_output_but_reject_next_turn() {
    let (_root, core) = fixture();
    let binding = register(&core, 801).await;
    let lines = (0..250)
        .map(|n| format!("line-{n}\r\n"))
        .collect::<String>();
    output(&core, binding, lines.into_bytes());
    let view = core.session_observation_snapshot(binding).unwrap();
    assert_eq!(view.tail.len(), 200);
    assert!(view.tail.iter().any(|s| s.contains("line-249")));
    assert_eq!(view.output_generation, 1);
    output(&core, binding, b"new output same turn".to_vec());
    core.apply_observation_for_turn(
        binding,
        view.confirmed_turn,
        SessionObservation::Workflow(proto::WorkflowProgress::default()),
        now(),
    )
    .unwrap();
    submit(&core, binding, now() + Duration::from_secs(5)).await;
    assert!(matches!(
        core.apply_observation_for_turn(
            binding,
            view.confirmed_turn,
            SessionObservation::Workflow(proto::WorkflowProgress::default()),
            now()
        ),
        Err(SessionError::StaleBinding)
    ));
    let stale = SessionBinding {
        wrapper: WrapperConnectionId(999),
        ..binding
    };
    assert!(matches!(
        core.session_observation_snapshot(stale),
        Err(SessionError::StaleBinding)
    ));
}
#[tokio::test(start_paused = true)]
async fn running_children_keep_cutoff_while_unknown_children_allow_next_actual_submit() {
    let (_root, core) = fixture();
    let binding = register(&core, 811).await;
    core.apply_observation(
        binding,
        SessionObservation::Subagents(proto::SubagentTree {
            nodes: vec![proto::SubagentNode {
                state: "running".into(),
                ..Default::default()
            }],
            ..Default::default()
        }),
        now(),
    )
    .unwrap();
    submit(&core, binding, now() + Duration::from_secs(5)).await;
    let view = core.session_observation_snapshot(binding).unwrap();
    assert_eq!(view.confirmed_turn, 1);
    assert_eq!(view.turn_started_at, now());
    core.apply_observation_for_turn(
        binding,
        view.confirmed_turn,
        SessionObservation::Subagents(proto::SubagentTree {
            nodes: vec![proto::SubagentNode {
                state: "unknown".into(),
                ..Default::default()
            }],
            ..Default::default()
        }),
        now(),
    )
    .unwrap();
    submit(&core, binding, now() + Duration::from_secs(10)).await;
    assert_eq!(
        core.session_observation_snapshot(binding)
            .unwrap()
            .turn_started_at,
        now() + Duration::from_secs(10)
    );
}
#[tokio::test]
async fn vt_header_redirect_masking_duplicate_reset_and_last_50_are_canonical() {
    let (_root, core) = fixture();
    let conductor = register(&core, 821).await;
    let receiver = register(&core, 822).await;
    role(&core, conductor, LiveSessionId(0), "conductor");
    role(&core, receiver, conductor.session, "reviewer");
    let header = format!("Message from sk-{}", "A".repeat(40));
    let effects = output(&core, receiver, header.as_bytes().to_vec());
    assert!(effects.0.iter().any(|e|matches!(e,CoreEffect::Notify(CoreEvent::CrossSessionMessage{binding,..}) if *binding==conductor)));
    assert!(
        core.snapshot(receiver.session)
            .unwrap()
            .cross_session_messages
            .is_empty()
    );
    let messages = core
        .snapshot(conductor.session)
        .unwrap()
        .cross_session_messages;
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].receiver_session_id, receiver.session.0);
    assert_eq!(messages[0].receiver_role, "reviewer");
    assert!(!messages[0].text.contains(&"A".repeat(40)));
    output(&core, receiver, b"\r".to_vec());
    assert_eq!(
        core.snapshot(conductor.session)
            .unwrap()
            .cross_session_messages
            .len(),
        1
    );
    output(
        &core,
        receiver,
        b"\x1b[2J\x1b[HMessage from second".to_vec(),
    );
    output(
        &core,
        receiver,
        format!("\x1b[2J\x1b[H{header}").into_bytes(),
    );
    assert_eq!(
        core.snapshot(conductor.session)
            .unwrap()
            .cross_session_messages
            .len(),
        3
    );
    for n in 0..55 {
        core.apply_observation(
            receiver,
            SessionObservation::CrossSessionMessage(proto::CrossSessionMessage {
                text: format!("Message from sender-{n}"),
                sender: "forged".into(),
                receiver_session_id: 999,
                ..Default::default()
            }),
            now(),
        )
        .unwrap();
    }
    let messages = core
        .snapshot(conductor.session)
        .unwrap()
        .cross_session_messages;
    assert_eq!(messages.len(), 50);
    assert_eq!(messages[0].sender, "sender-5");
    assert_eq!(messages[49].sender, "sender-54");
    assert!(
        messages
            .iter()
            .all(|m| m.receiver_session_id == receiver.session.0)
    );
    core.apply_observation(
        receiver,
        SessionObservation::CrossSessionMessage(proto::CrossSessionMessage {
            text: "Message from sender-54".into(),
            ..Default::default()
        }),
        now(),
    )
    .unwrap();
    assert_eq!(
        core.snapshot(conductor.session)
            .unwrap()
            .cross_session_messages
            .len(),
        50
    );
}
#[tokio::test]
async fn missing_orchestration_or_parent_never_records_body_or_redirects_elsewhere() {
    let (_root, core) = fixture();
    let receiver = register(&core, 831).await;
    output(&core, receiver, b"Message from teammate".to_vec());
    assert!(
        core.snapshot(receiver.session)
            .unwrap()
            .cross_session_messages
            .is_empty()
    );
    role(&core, receiver, LiveSessionId(9999), "child");
    output(
        &core,
        receiver,
        b"\x1b[2J\x1b[HMessage from another".to_vec(),
    );
    assert!(
        core.snapshot(receiver.session)
            .unwrap()
            .cross_session_messages
            .is_empty()
    );
    role(&core, receiver, LiveSessionId(0), "conductor");
    core.apply_observation(
        receiver,
        SessionObservation::CrossSessionMessage(proto::CrossSessionMessage {
            text: "ordinary private body".into(),
            ..Default::default()
        }),
        now(),
    )
    .unwrap();
    assert!(
        core.snapshot(receiver.session)
            .unwrap()
            .cross_session_messages
            .is_empty()
    );
}

#[tokio::test]
async fn empty_native_tree_clears_canonical_state_and_initial_ui_replay() {
    let (_root, core) = fixture();
    let binding = register(&core, 1911).await;
    core.apply_observation(
        binding,
        SessionObservation::Subagents(proto::SubagentTree {
            nodes: vec![proto::SubagentNode {
                id: "child".into(),
                state: "running".into(),
                ..Default::default()
            }],
            ..Default::default()
        }),
        now(),
    )
    .unwrap();
    assert!(core.details(binding.session).unwrap().subagents.is_some());
    let effects = core
        .apply_observation(
            binding,
            SessionObservation::Subagents(proto::SubagentTree::default()),
            now(),
        )
        .unwrap();
    assert!(effects.0.iter().any(|effect| matches!(effect, CoreEffect::Notify(CoreEvent::SubagentsChanged { tree, .. }) if tree.nodes.is_empty())));
    assert!(core.details(binding.session).unwrap().subagents.is_none());
    let ui = core
        .attach_ui(
            UiBinding {
                connection: UiConnectionId(1911),
                auth_epoch: core.auth_epoch(),
            },
            None,
            None,
        )
        .unwrap();
    assert!(
        !ui.ordered_frames
            .iter()
            .any(|frame| frame.r#type == "subagent_tree")
    );
}

#[tokio::test]
async fn terminal_workflow_frame_is_timeout_once_without_completion_notification() {
    for disconnect in [false, true] {
        let (_root, core) = fixture();
        let binding = register(&core, 1921).await;
        let ui = UiBinding {
            connection: UiConnectionId(1921),
            auth_epoch: core.auth_epoch(),
        };
        core.attach_ui(ui, None, None).unwrap();
        core.finish_ui_priming(ui).unwrap();
        let original = proto::WorkflowProgress {
            detected: true,
            source: "journal".into(),
            done: 1,
            total: 3,
            running: 1,
            pending: 1,
            waiting_dynamic: 2,
            percent: 33,
            ..Default::default()
        };
        core.apply_observation(
            binding,
            SessionObservation::Workflow(original.clone()),
            now(),
        )
        .unwrap();
        let effects = if disconnect {
            core.disconnected(binding, now()).unwrap()
        } else {
            core.observe_end(
                binding,
                SessionEnd {
                    declared_state: "completed".into(),
                    exit_code: 0,
                    reason: String::new(),
                },
                now(),
            )
            .unwrap()
        };
        let stored = core.details(binding.session).unwrap().workflow.unwrap();
        assert!(stored.settled);
        assert_eq!(stored.settled_by, "timeout");
        assert_eq!(
            (stored.running, stored.pending, stored.waiting_dynamic),
            (0, 0, 0)
        );
        assert_eq!((stored.done, stored.total, stored.percent), (1, 3, 33));
        assert!(effects.0.iter().any(|effect| matches!(effect, CoreEffect::SendUiBestEffort { message: frame, .. } if frame.r#type == "workflow_progress" && frame.workflow_progress.as_ref() == Some(&stored))));
        assert!(!effects.0.iter().any(|effect| matches!(
            effect,
            CoreEffect::Notify(CoreEvent::WorkflowChanged { .. })
        )));
        assert!(matches!(
            core.apply_observation_for_turn(
                binding,
                0,
                SessionObservation::Workflow(original),
                now()
            ),
            Err(SessionError::StaleBinding)
        ));
        let repeated = core.disconnected(binding, now()).unwrap();
        assert!(!repeated.0.iter().any(|effect| matches!(effect, CoreEffect::SendUiBestEffort { message: frame, .. } if frame.r#type == "workflow_progress")));
    }
}

#[tokio::test]
async fn partial_profile_mirror_failure_reloads_successful_phrase_into_actual_detector() {
    let (root, core) = fixture();
    let core = Arc::new(core);
    let paths = RuntimePaths::trial(
        &root.path().join("trial"),
        49688,
        &root.path().join("installed"),
    )
    .unwrap();
    let config = Arc::new(
        crate::config::ConfigStore::new(paths.clone(), crate::config::Config::default()).unwrap(),
    );
    let warnings = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = warnings.clone();
    let owner = crate::application::approval_patterns::ApprovalPatterns::new(
        &paths,
        config,
        Arc::downgrade(&core),
        Arc::new(move |warning| captured.lock().unwrap().push(warning)),
    )
    .unwrap();
    owner.sync().unwrap();
    let prompt = b"synthetic decisive gate\r\n\xe2\x9d\xaf 1. Yes\r\n  2. No\r\n";
    let before = register(&core, 2041).await;
    let effects = output(&core, before, prompt.to_vec());
    assert!(
        !effects
            .0
            .iter()
            .any(|effect| matches!(effect, CoreEffect::Notify(CoreEvent::ApprovalOpened { .. })))
    );
    // Release the inspected persistence ticket before the next registration
    // waits for its ordered lane; the fixture does not apply these effects.
    drop(effects);
    let folder = paths.root().join("approval-patterns");
    std::fs::write(
        folder.join(format!("{}.{}.json", "claude", "official")),
        b"[\"synthetic decisive gate\"]",
    )
    .unwrap();
    std::fs::write(
        folder.join(format!("{}.{}.json", "codex", "official")),
        b"malformed later profile",
    )
    .unwrap();
    owner.refresh();
    assert!(
        warnings
            .lock()
            .unwrap()
            .contains(&"approval pattern mirrors unavailable")
    );
    assert_eq!(
        serde_json::from_slice::<Vec<String>>(&owner.asset("claude.json").unwrap()).unwrap(),
        vec!["synthetic decisive gate"]
    );
    let after = register(&core, 2042).await;
    let effects = output(&core, after, prompt.to_vec());
    assert!(effects.0.iter().any(|effect|matches!(effect,CoreEffect::Notify(CoreEvent::ApprovalOpened { session,record }) if *session==after.session && record.data().question=="synthetic decisive gate")));
}
