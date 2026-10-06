//! Reattach failures must not consume the retained session or its journal owner.
use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};

fn request(session: LiveSessionId, replay: &[u8]) -> ReattachRequest {
    ReattachRequest {
        restored_metadata: None,
        message: proto::Message {
            session_id: session.0,
            provider: "copilot".into(),
            cwd: "/fixture/project".into(),
            label: "replacement-label".into(),
            pid: 7,
            cols: 120,
            rows: 30,
            // A valid wire timestamp bypasses the invalid-clock fallback. Use a
            // different day so an accidental journal rebind is also observable.
            started_at: proto::time::format_rfc3339(now() + Duration::from_secs(86400)).unwrap(),
            replay_b64: STANDARD.encode(replay),
            pty_bytes: replay.len() as i64,
            input_seq_high_watermark: 90,
            ..Default::default()
        },
    }
}

fn invalid_clock() -> Timestamp {
    // This is a host-clock fault, not a malformed wrapper-controlled timestamp.
    Timestamp::from_unix(i64::MAX, 0).unwrap()
}

#[tokio::test]
async fn warm_invalid_clock_preserves_binding_replay_approval_and_queued_input() {
    let f = fixture();
    let old = register(&f).await.binding;
    let cancel = TaskCancellation::default();
    f.engine.acknowledge(old, InputSeq(0));
    let sent = f
        .engine
        .submit(
            old,
            InputRequest {
                bytes: b"inflight".to_vec(),
                authority: InputAuthority::Internal,
            },
            now(),
            &cancel,
        )
        .await;
    assert!(matches!(
        sent.disposition,
        InputDisposition::TransportWritten { .. }
    ));
    let sequence = f.transport.frames.lock().unwrap()[0].1.input_seq;
    let marker = b"[MANY-AI-CLI]\r\nQ1 Choose a mode?\r\n1. Keep\r\n2. Change\r\n[/MANY-AI-CLI]";
    f.sink
        .apply(
            f.engine
                .observe_output(
                    old,
                    OutputChunk {
                        bytes: marker.to_vec(),
                        total_pty_bytes: marker.len() as i64,
                    },
                    now(),
                )
                .unwrap(),
        )
        .await
        .unwrap();
    f.engine.set_initial_gate(old, now()).unwrap();
    let queued = f
        .engine
        .submit(
            old,
            InputRequest {
                bytes: b"queued input".to_vec(),
                authority: InputAuthority::Internal,
            },
            now(),
            &cancel,
        )
        .await;
    assert!(matches!(
        queued.disposition,
        InputDisposition::Deferred {
            reason: DeferredReason::InitialPrompt,
            ..
        }
    ));
    let before = f.engine.details(old.session).unwrap();
    assert!(before.approval.record.is_some());

    let result = f
        .engine
        .reattach(
            request(old.session, b""),
            WrapperConnectionId(2),
            invalid_clock(),
        )
        .await;
    assert!(
        matches!(result, Err(SessionError::InvalidRequest(ref detail))
        if detail == "timestamp is outside the supported range")
    );
    assert!(f.engine.is_current(old));
    let after = f.engine.details(old.session).unwrap();
    assert!(after.connected);
    assert!(after.snapshot == before.snapshot);
    assert!(after.approval == before.approval);
    assert_eq!(after.transcript, before.transcript);
    assert_eq!(after.db_id, before.db_id);
    assert_eq!(after.last_output_at, before.last_output_at);
    let priming = f
        .engine
        .attach_ui(
            UiBinding {
                connection: UiConnectionId(81),
                auth_epoch: f.engine.auth_epoch(),
            },
            Some(old.session),
            None,
        )
        .unwrap();
    assert!(
        priming
            .ordered_frames
            .iter()
            .any(|frame| frame.r#type == "pty_data" && frame.data == marker)
    );
    assert_eq!(
        f.engine.acknowledge(old, InputSeq(sequence)),
        AckDisposition::Removed
    );
    assert!(f.engine.clear_initial_gate_scoped(old));
    f.engine.flush(old, &cancel).await;
    let frames = f.transport.frames.lock().unwrap();
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[1].0, old);
    assert_eq!(frames[1].1.data, b"queued input");
    assert_eq!(frames[1].1.input_seq, sequence + 1);
}

#[tokio::test]
async fn warm_invalid_clock_preserves_exact_journal_binding_and_database_identity() {
    let f = fixture();
    let original = register(&f).await;
    let old = original.binding;
    let before = f
        .store
        .session_overview_by_live_session(old.session)
        .unwrap();
    let replacement = request(old.session, b"");
    let replacement_path = f
        .sink
        .journal
        .paths_for_timestamp(
            old.session,
            "copilot",
            "/fixture/project",
            &replacement.message.started_at,
        )
        .unwrap()
        .jsonl;
    assert_ne!(
        replacement_path,
        std::path::PathBuf::from(&original.snapshot.jsonl_path)
    );
    let result = f
        .engine
        .reattach(replacement, WrapperConnectionId(2), invalid_clock())
        .await;
    assert!(matches!(result, Err(SessionError::InvalidRequest(_))));

    // Incarnation-only history would also accept a replacement wrapper. Check
    // the old exact binding to catch fixes which preserve memory but rebind I/O.
    let event = serde_json::json!({
        "type": "reattach_failure_probe",
        "ts": proto::time::format_rfc3339(now()).unwrap(),
        "session_id": old.session.0,
    });
    f.sink
        .journal
        .apply_bound(
            old,
            PersistenceBindingScope::ExactWrapper,
            PersistenceEffect::Event {
                session: old.session,
                event: HistoryEvent(event.as_object().unwrap().clone()),
            },
        )
        .unwrap();
    let records = std::fs::read_to_string(&original.snapshot.jsonl_path).unwrap();
    assert!(records.lines().any(
        |line| serde_json::from_str::<serde_json::Value>(line).unwrap()["type"]
            == "reattach_failure_probe"
    ));
    assert!(!replacement_path.exists());
    let after = f
        .store
        .session_overview_by_live_session(old.session)
        .unwrap();
    assert_eq!(after.id, before.id);
    assert_eq!(after.started_at, before.started_at);
    assert_eq!(after.jsonl_path, before.jsonl_path);
}

#[tokio::test]
async fn cold_invalid_clock_releases_admission_and_allows_a_later_reattach() {
    // Empty ordinary replay reaches history formatting; nonempty replay reaches
    // last_output_at formatting, including probes which produce no history.
    for (replay, usage_probe) in [
        (b"retained".as_slice(), false),
        (b"".as_slice(), false),
        (b"retained".as_slice(), true),
    ] {
        let f = fixture();
        let id = LiveSessionId(7);
        let mut failed = request(id, replay);
        failed.message.usage_probe = usage_probe;
        let path = f
            .sink
            .journal
            .paths_for_timestamp(
                id,
                "copilot",
                "/fixture/project",
                &failed.message.started_at,
            )
            .unwrap()
            .jsonl;
        let result = f
            .engine
            .reattach(failed, WrapperConnectionId(2), invalid_clock())
            .await;
        assert!(
            matches!(result, Err(SessionError::InvalidRequest(ref detail))
            if detail == "timestamp is outside the supported range")
        );
        assert_eq!(f.engine.registered_session_count(), 0);
        assert_eq!(f.engine.provider_session_count("copilot"), 0);
        let update = f
            .engine
            .begin_provider_update("copilot")
            .expect("failed reattach must not leave PendingSpawns or registered admission");
        f.engine.end_provider_update(update);
        assert!(!path.exists());

        let mut valid = request(id, replay);
        valid.message.usage_probe = usage_probe;
        let attached = f
            .engine
            .reattach(valid, WrapperConnectionId(3), now())
            .await
            .unwrap();
        let current = attached.binding;
        assert_eq!(current.session, id);
        assert_eq!(
            attached.snapshot.last_output_at,
            if replay.is_empty() {
                String::new()
            } else {
                proto::time::format_rfc3339(now()).unwrap()
            }
        );
        f.sink.apply(attached.after_reattached).await.unwrap();
        assert!(f.engine.is_current(current));
        assert_eq!(f.engine.provider_session_count("copilot"), 1);
        f.sink
            .apply(f.engine.dismiss(id, now()).unwrap())
            .await
            .unwrap();
        let update = f
            .engine
            .begin_provider_update("copilot")
            .expect("successful retry must consume its one pending lease");
        f.engine.end_provider_update(update);
    }
}

#[tokio::test]
async fn warm_reattach_admission_uses_parent_changed_during_initialization() {
    let f = fixture();
    let old = register(&f).await.binding;
    let parent = register(&f).await.binding.session;
    let replacement = request(old.session, b"");
    let replacement_path = f
        .sink
        .journal
        .paths_for_timestamp(
            old.session,
            "copilot",
            "/fixture/project",
            &replacement.message.started_at,
        )
        .unwrap()
        .jsonl;
    // A directory at the proposed file path makes optional writer creation warn
    // synchronously. Use its existing callback to change the retained parent
    // after initialize received its metadata, without a production test hook.
    std::fs::create_dir(&replacement_path).unwrap();
    let engine = Arc::downgrade(&f.engine);
    let warnings = Arc::new(AtomicUsize::new(0));
    let observed = warnings.clone();
    f.sink.journal.set_warning_handler(Arc::new(move |id, _| {
        assert_eq!(id, old.session);
        if observed.fetch_add(1, Ordering::SeqCst) == 0 {
            let effects = engine
                .upgrade()
                .unwrap()
                .apply_relay_binding(
                    old,
                    parent,
                    OrchestrationId("reattach-parent-change".into()),
                    "synthetic-board".into(),
                    "review".into(),
                    now(),
                )
                .unwrap();
            // This assertion is about the live transaction. Do not apply a
            // persistence effect while its journal callback holds I/O ordering.
            drop(effects);
        }
    }));

    let attached = f
        .engine
        .reattach(replacement, WrapperConnectionId(2), now())
        .await
        .unwrap();
    assert_eq!(warnings.load(Ordering::SeqCst), 1);
    assert_eq!(attached.snapshot.parent_session_id, parent);
    assert_eq!(attached.snapshot.role, "review");
    f.sink.apply(attached.after_reattached).await.unwrap();
    assert!(matches!(
        f.engine.reserve_children(
            AdmissionRequest {
                parent,
                slots: 1,
                origin: VerifiedSpawnOrigin::Autonomous,
                replace: None,
            },
            AdmissionLimits {
                max_children_per_parent: 1,
                max_total_sessions: 100,
            },
        ),
        Err(AdmissionError::ChildrenPerParent {
            used: 1,
            maximum: 1,
            ..
        })
    ));
}
