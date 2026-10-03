//! Synthetic caller transactions using the real SessionEngine and journal.
use super::*;
use many_ai_cli::approval::token::{ONE_TAP_TTL, OneTapAction, OneTapManager, TokenError};

async fn native_session(f: &Fixture, command: &str, yes: &str, focus_yes: bool) -> SessionBinding {
    let registered = f
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
    f.sink.apply(registered.after_registered).await.unwrap();
    repaint(f, registered.binding, command, yes, focus_yes).await;
    registered.binding
}
async fn repaint(f: &Fixture, binding: SessionBinding, command: &str, yes: &str, focus_yes: bool) {
    let text = format!(
        "\x1b[2J\x1b[HRun: {command}\r\nDo you want to proceed?\r\n{} 1. {yes}\r\n{} 2. No",
        if focus_yes { "❯" } else { " " },
        if focus_yes { " " } else { "❯" }
    );
    f.sink
        .apply(
            f.engine
                .observe_output(
                    binding,
                    OutputChunk {
                        total_pty_bytes: text.len() as i64,
                        bytes: text.into_bytes(),
                    },
                    now(),
                )
                .unwrap(),
        )
        .await
        .unwrap();
}
fn binding(f: &Fixture, session: SessionBinding) -> ApprovalActionBinding {
    let record = f
        .engine
        .details(session.session)
        .unwrap()
        .approval
        .record
        .unwrap();
    let data = record.data();
    ApprovalActionBinding {
        session,
        candidate_key: data.candidate.key.clone(),
        source_epoch: data.candidate.source_epoch,
        sig: data.sig.clone(),
    }
}
fn request(
    binding: ApprovalActionBinding,
    selection: NativeActionSelection,
) -> NativeActionRequest {
    NativeActionRequest {
        binding,
        selection,
        origin: NativeActionOrigin::Batch,
    }
}
fn pending(f: &Fixture, session: SessionBinding) -> bool {
    f.engine
        .details(session.session)
        .unwrap()
        .approval
        .record
        .is_some()
}
fn sent(f: &Fixture) -> Vec<Vec<u8>> {
    f.transport
        .frames
        .lock()
        .unwrap()
        .iter()
        .map(|(_, frame)| frame.data.clone())
        .collect()
}

#[tokio::test]
async fn one_shot_intent_uses_focus_observed_after_fifo_acquisition() {
    for (selection, focus_before, focus_after, expected) in [
        (
            NativeActionSelection::ApproveOnce,
            true,
            false,
            b"1\r".as_slice(),
        ),
        (
            NativeActionSelection::RejectOnce,
            false,
            true,
            b"2\r".as_slice(),
        ),
    ] {
        let f = fixture();
        let session = native_session(&f, "git status", "Yes", focus_before).await;
        let bound = binding(&f, session);
        let cancel = TaskCancellation::default();
        // Reserving this earlier input without polling it gives deterministic
        // FIFO ordering without sleeps or a second queue in the caller.
        let predecessor = f.engine.submit(
            session,
            InputRequest {
                bytes: b"unsent predecessor".to_vec(),
                authority: InputAuthority::Internal,
            },
            now(),
            &cancel,
        );
        let mut action = f
            .engine
            .prepare_and_send(request(bound.clone(), selection), &cancel);
        assert!(matches!(
            futures_util::poll!(&mut action),
            std::task::Poll::Pending
        ));
        repaint(&f, session, "git status", "Yes", focus_after).await;
        assert_eq!(
            binding(&f, session),
            bound,
            "focus must not create another candidate or epoch"
        );
        drop(predecessor);
        let reserved = action.await.unwrap();
        assert_eq!(sent(&f), vec![expected.to_vec()]);
        assert_eq!(reserved.selected_text.as_bytes(), expected);
        assert!(pending(&f, session));
        f.engine.release(reserved);
    }
}

#[tokio::test]
async fn only_human_exact_selection_carries_supplied_text_and_persistent_option() {
    let f = fixture();
    let session = native_session(&f, "git status", "Yes, and don't ask again", true).await;
    let bound = binding(&f, session);
    let cancel = TaskCancellation::default();
    let exact = NativeActionSelection::Exact {
        selected_text: "human-selected persistent permission".into(),
        send_text: "\r".into(),
    };
    for origin in [
        NativeActionOrigin::Batch,
        NativeActionOrigin::OneTap {
            nonce: "synthetic-unused".into(),
        },
        NativeActionOrigin::Automatic {
            rule_id: "synthetic-rule".into(),
        },
    ] {
        assert!(matches!(
            f.engine
                .prepare_and_send(
                    NativeActionRequest {
                        binding: bound.clone(),
                        selection: exact.clone(),
                        origin,
                    },
                    &cancel
                )
                .await,
            Err(ApprovalActionError::PersistentOption)
        ));
    }
    assert!(matches!(
        f.engine
            .prepare_and_send(
                request(bound.clone(), NativeActionSelection::ApproveOnce),
                &cancel
            )
            .await,
        Err(ApprovalActionError::PersistentOption)
    ));
    assert!(sent(&f).is_empty());
    let reserved = f
        .engine
        .prepare_and_send(
            NativeActionRequest {
                binding: bound,
                selection: exact,
                origin: NativeActionOrigin::User,
            },
            &cancel,
        )
        .await
        .unwrap();
    assert_eq!(sent(&f), vec![b"\r".to_vec()]);
    f.sink
        .apply(f.engine.commit(reserved, now()).unwrap())
        .await
        .unwrap();
    let rows = f
        .store
        .approvals_by_live_session(session.session, 10, false)
        .unwrap()
        .unwrap();
    assert_eq!(
        rows[0].selected_text,
        "human-selected persistent permission"
    );
}

#[tokio::test]
async fn one_shot_high_risk_approval_is_refused_but_rejection_remains_available() {
    let f = fixture();
    let session = native_session(&f, "rm -rf /synthetic/owned-output", "Yes", true).await;
    let bound = binding(&f, session);
    let record = f
        .engine
        .details(session.session)
        .unwrap()
        .approval
        .record
        .unwrap();
    assert_eq!(record.data().summary.risk, "high");
    let cancel = TaskCancellation::default();
    assert!(matches!(
        f.engine
            .prepare_and_send(
                request(bound.clone(), NativeActionSelection::ApproveOnce),
                &cancel
            )
            .await,
        Err(ApprovalActionError::HighRisk)
    ));
    assert!(sent(&f).is_empty());
    let rejected = f
        .engine
        .prepare_and_send(request(bound, NativeActionSelection::RejectOnce), &cancel)
        .await
        .unwrap();
    assert_eq!(sent(&f), vec![b"2\r".to_vec()]);
    f.engine.release(rejected);
}

#[tokio::test]
async fn native_action_reservation_spans_send_until_release_or_commit() {
    let f = fixture();
    let session = native_session(&f, "git status", "Yes", true).await;
    let bound = binding(&f, session);
    let cancel = TaskCancellation::default();
    let first = f
        .engine
        .prepare_and_send(
            request(bound.clone(), NativeActionSelection::ApproveOnce),
            &cancel,
        )
        .await
        .unwrap();
    assert!(pending(&f, session));
    assert!(matches!(
        f.engine
            .prepare_and_send(
                request(bound.clone(), NativeActionSelection::RejectOnce),
                &cancel
            )
            .await,
        Err(ApprovalActionError::Reserved)
    ));
    assert_eq!(sent(&f), vec![b"\r".to_vec()]);
    f.engine.release(first);
    assert!(pending(&f, session));
    let second = f
        .engine
        .prepare_and_send(
            request(bound.clone(), NativeActionSelection::RejectOnce),
            &cancel,
        )
        .await
        .unwrap();
    f.sink
        .apply(f.engine.commit(second, now()).unwrap())
        .await
        .unwrap();
    assert!(!pending(&f, session));
    assert!(matches!(
        f.engine
            .prepare_and_send(request(bound, NativeActionSelection::ApproveOnce), &cancel)
            .await,
        Err(ApprovalActionError::StaleCandidate)
    ));
    assert_eq!(sent(&f), vec![b"\r".to_vec(), b"2\r".to_vec()]);
}

#[tokio::test]
async fn failed_send_releases_reservation_and_nonce_is_consumed_only_before_commit() {
    let f = fixture();
    let session = native_session(&f, "git status", "Yes", true).await;
    let bound = binding(&f, session);
    let tokens = OneTapManager::new().unwrap();
    let token = tokens
        .issue(
            session.session,
            &bound.sig,
            &bound.sig,
            bound.source_epoch,
            OneTapAction::Approve,
            now(),
        )
        .unwrap();
    let claim = tokens.verify(&token, now()).unwrap();
    let action_request = || NativeActionRequest {
        binding: bound.clone(),
        selection: NativeActionSelection::ApproveOnce,
        origin: NativeActionOrigin::OneTap {
            nonce: claim.nonce().into(),
        },
    };
    let cancel = TaskCancellation::default();
    f.transport.fail.store(1, Ordering::SeqCst);
    assert!(matches!(
        f.engine.prepare_and_send(action_request(), &cancel).await,
        Err(ApprovalActionError::Transport(_))
    ));
    assert!(pending(&f, session));
    assert!(sent(&f).is_empty());
    f.transport.fail.store(0, Ordering::SeqCst);
    let reserved = f
        .engine
        .prepare_and_send(action_request(), &cancel)
        .await
        .unwrap();
    assert!(pending(&f, session));
    tokens.consume(&claim, now()).unwrap();
    assert_eq!(tokens.consume(&claim, now()), Err(TokenError::Consumed));
    f.sink
        .apply(f.engine.commit(reserved, now()).unwrap())
        .await
        .unwrap();
    assert!(!pending(&f, session));
    assert_eq!(sent(&f), vec![b"\r".to_vec()]);
}

#[tokio::test]
async fn failed_nonce_consume_releases_only_lease_without_clearing_pending_record() {
    let f = fixture();
    let session = native_session(&f, "git status", "Yes", true).await;
    let bound = binding(&f, session);
    let tokens = OneTapManager::new().unwrap();
    let token = tokens
        .issue(
            session.session,
            &bound.sig,
            &bound.sig,
            bound.source_epoch,
            OneTapAction::Approve,
            now(),
        )
        .unwrap();
    let claim = tokens.verify(&token, now()).unwrap();
    let cancel = TaskCancellation::default();
    let reserved = f
        .engine
        .prepare_and_send(
            NativeActionRequest {
                binding: bound.clone(),
                selection: NativeActionSelection::ApproveOnce,
                origin: NativeActionOrigin::OneTap {
                    nonce: claim.nonce().into(),
                },
            },
            &cancel,
        )
        .await
        .unwrap();
    assert_eq!(
        tokens.consume(&claim, now() + ONE_TAP_TTL),
        Err(TokenError::Expired)
    );
    f.engine.release(reserved);
    assert!(pending(&f, session));
    assert_eq!(binding(&f, session), bound);
    let next = f
        .engine
        .prepare_and_send(request(bound, NativeActionSelection::RejectOnce), &cancel)
        .await
        .unwrap();
    f.engine.release(next);
    assert!(pending(&f, session));
    assert_eq!(sent(&f), vec![b"\r".to_vec(), b"2\r".to_vec()]);
}

#[tokio::test]
async fn dropping_action_during_send_releases_reservation_and_unsent_frame() {
    let f = fixture();
    let session = native_session(&f, "git status", "Yes", true).await;
    let bound = binding(&f, session);
    f.transport.pause_call.store(1, Ordering::SeqCst);
    let engine = f.engine.clone();
    let action_request = request(bound.clone(), NativeActionSelection::ApproveOnce);
    let running = tokio::spawn(async move {
        engine
            .prepare_and_send(action_request, &TaskCancellation::default())
            .await
    });
    f.transport.entered.notified().await;
    running.abort();
    assert!(running.await.is_err());
    assert!(pending(&f, session));
    assert!(sent(&f).is_empty());
    let next = f
        .engine
        .prepare_and_send(
            request(bound, NativeActionSelection::RejectOnce),
            &TaskCancellation::default(),
        )
        .await
        .unwrap();
    assert_eq!(sent(&f), vec![b"2\r".to_vec()]);
    f.engine.release(next);
}

#[tokio::test]
async fn absent_live_automatic_policy_releases_reservation_before_any_send() {
    let f = fixture();
    let session = native_session(&f, "git status", "Yes", true).await;
    let bound = binding(&f, session);
    let cancel = TaskCancellation::default();
    assert!(matches!(
        f.engine
            .prepare_and_send(
                NativeActionRequest {
                    binding: bound.clone(),
                    selection: NativeActionSelection::ApproveOnce,
                    origin: NativeActionOrigin::Automatic {
                        rule_id: "synthetic-rule".into()
                    },
                },
                &cancel
            )
            .await,
        Err(ApprovalActionError::PolicyChanged)
    ));
    assert!(sent(&f).is_empty());
    let next = f
        .engine
        .prepare_and_send(request(bound, NativeActionSelection::ApproveOnce), &cancel)
        .await
        .unwrap();
    f.engine.release(next);
}

#[tokio::test]
async fn commit_from_replaced_wrapper_cannot_clear_restored_pending_record() {
    let f = fixture();
    let session = native_session(&f, "git status", "Yes", true).await;
    let cancel = TaskCancellation::default();
    let old = f
        .engine
        .prepare_and_send(
            request(binding(&f, session), NativeActionSelection::ApproveOnce),
            &cancel,
        )
        .await
        .unwrap();
    let restored = f
        .engine
        .reattach(
            ReattachRequest {
                message: proto::Message {
                    session_id: session.session.0,
                    provider: "claude".into(),
                    cwd: "/fixture/project".into(),
                    pid: 7,
                    cols: 120,
                    rows: 30,
                    started_at: proto::time::format_rfc3339(now()).unwrap(),
                    ..Default::default()
                },
            },
            WrapperConnectionId(2),
            now(),
        )
        .await
        .unwrap();
    let replacement = restored.binding;
    f.sink.apply(restored.after_reattached).await.unwrap();
    assert!(matches!(
        f.engine.commit(old, now()),
        Err(ApprovalActionError::Reserved)
    ));
    assert!(pending(&f, replacement));
    let next = f
        .engine
        .prepare_and_send(
            request(binding(&f, replacement), NativeActionSelection::RejectOnce),
            &cancel,
        )
        .await
        .unwrap();
    f.engine.release(next);
    let frames = f.transport.frames.lock().unwrap();
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].0, session);
    assert_eq!(frames[1].0, replacement);
    assert!(pending(&f, replacement));
}

async fn shortcut_session(f: &Fixture) -> SessionBinding {
    let registered = f
        .engine
        .register(
            RegisterRequest {
                message: proto::Message {
                    provider: "codex".into(),
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
    f.sink.apply(registered.after_registered).await.unwrap();
    shortcut_repaint(f, registered.binding, "git status").await;
    registered.binding
}
async fn shortcut_repaint(f: &Fixture, session: SessionBinding, command: &str) {
    let text =
        format!("\x1b[2J\x1b[HRun: {command}\r\nWould you like to run?\r\nApprove (y)\r\nDeny (n)");
    f.sink
        .apply(
            f.engine
                .observe_output(
                    session,
                    OutputChunk {
                        total_pty_bytes: text.len() as i64,
                        bytes: text.into_bytes(),
                    },
                    now(),
                )
                .unwrap(),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn batch_snapshot_redetects_summary_without_mutating_immutable_record() {
    let f = fixture();
    let session = shortcut_session(&f).await;
    let original = f.engine.details(session.session).unwrap().approval;
    assert_eq!(original.record.as_ref().unwrap().data().summary.risk, "low");
    let first = f.engine.pending_native_approval_actions();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].provider, "codex");
    assert_eq!(first[0].cwd, "/fixture/project");
    assert!(first[0].connected);
    shortcut_repaint(&f, session, "git branch foo").await;
    // Shortcut identity excludes command context, so the immutable pending
    // record still carries the original summary while the live VT changed.
    let retained = f.engine.details(session.session).unwrap().approval;
    assert!(retained == original);
    let fresh = f.engine.pending_native_approval_actions();
    assert_eq!(fresh.len(), 1);
    assert_eq!(fresh[0].binding, first[0].binding);
    assert_eq!(fresh[0].summary.command, "git branch foo");
    assert_eq!(fresh[0].summary.risk, "mid");
    assert!(f.engine.details(session.session).unwrap().approval == retained);
    assert!(sent(&f).is_empty());
}

#[tokio::test]
async fn batch_and_automatic_recheck_low_risk_after_fifo_but_one_tap_allows_mid() {
    for origin in [
        NativeActionOrigin::Batch,
        NativeActionOrigin::Automatic {
            rule_id: "synthetic-rule".into(),
        },
    ] {
        let f = fixture();
        let session = shortcut_session(&f).await;
        let bound = binding(&f, session);
        let cancel = TaskCancellation::default();
        let predecessor = f.engine.submit(
            session,
            InputRequest {
                bytes: b"unsent predecessor".to_vec(),
                authority: InputAuthority::Internal,
            },
            now(),
            &cancel,
        );
        let mut queued = f.engine.prepare_and_send(
            NativeActionRequest {
                binding: bound.clone(),
                selection: NativeActionSelection::ApproveOnce,
                origin,
            },
            &cancel,
        );
        assert!(matches!(
            futures_util::poll!(&mut queued),
            std::task::Poll::Pending
        ));
        shortcut_repaint(&f, session, "git branch foo").await;
        assert_eq!(
            f.engine.pending_native_approval_actions()[0].summary.risk,
            "mid"
        );
        assert_eq!(binding(&f, session), bound);
        drop(predecessor);
        assert!(matches!(
            queued.await,
            Err(ApprovalActionError::StaleCandidate)
        ));
        assert!(sent(&f).is_empty());
        assert!(pending(&f, session));
        let manual = f
            .engine
            .prepare_and_send(
                NativeActionRequest {
                    binding: bound,
                    selection: NativeActionSelection::ApproveOnce,
                    origin: NativeActionOrigin::OneTap {
                        nonce: "synthetic-mid-intent".into(),
                    },
                },
                &cancel,
            )
            .await
            .unwrap();
        assert_eq!(sent(&f), vec![b"y".to_vec()]);
        f.engine.release(manual);
    }
}

#[tokio::test]
async fn batch_snapshot_excludes_vanished_prompts_and_notices_without_options() {
    let f = fixture();
    let session = native_session(&f, "git status", "Yes", true).await;
    for text in [
        "\x1b[2J\x1b[HWorking...",
        "\x1b[2J\x1b[HChoose a mode?\r\n❯ 1. Type something.\r\n 2. Chat about this",
    ] {
        f.sink
            .apply(
                f.engine
                    .observe_output(
                        session,
                        OutputChunk {
                            total_pty_bytes: text.len() as i64,
                            bytes: text.as_bytes().to_vec(),
                        },
                        now(),
                    )
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(
            pending(&f, session),
            "stored record is retained independently of live matching"
        );
        assert!(f.engine.pending_native_approval_actions().is_empty());
    }
    assert!(
        f.engine
            .details(session.session)
            .unwrap()
            .approval
            .record
            .unwrap()
            .data()
            .options
            .is_empty()
    );
}

#[tokio::test]
async fn captured_batch_match_remains_counted_when_connection_disappears_before_apply() {
    let f = fixture();
    let session = shortcut_session(&f).await;
    let matched = f.engine.pending_native_approval_actions();
    assert_eq!(matched.len(), 1);
    f.sink
        .apply(f.engine.disconnected(session, now()).unwrap())
        .await
        .unwrap();
    assert!(matches!(
        f.engine
            .prepare_and_send(
                request(
                    matched[0].binding.clone(),
                    NativeActionSelection::ApproveOnce
                ),
                &TaskCancellation::default()
            )
            .await,
        Err(ApprovalActionError::StaleWrapper)
    ));
    assert_eq!(matched.len(), 1);
    assert!(sent(&f).is_empty());
    assert!(f.engine.pending_native_approval_actions().is_empty());
}
