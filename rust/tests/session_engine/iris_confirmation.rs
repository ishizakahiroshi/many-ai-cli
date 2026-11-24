use super::*;
use many_ai_cli::approval::marker;
use many_ai_cli::terminal::session::iris_confirmation::{
    IrisAnswerOutcome, iris_confirmation_question,
};

async fn publish(f: &Fixture, session: SessionBinding, body: &str) -> ImmutableApprovalRecord {
    let block = format!("{}\n{body}\n{}", marker::OPEN, marker::CLOSE);
    let effects = f
        .engine
        .observe_transcript_marker(session, marker::extract(&block), now())
        .unwrap();
    f.sink.apply(effects).await.unwrap();
    f.engine
        .details(session.session)
        .unwrap()
        .approval
        .record
        .unwrap()
}

async fn apply(
    f: &Fixture,
    session: SessionBinding,
    record: &ImmutableApprovalRecord,
) -> IrisAnswerOutcome {
    let data = record.data();
    f.engine
        .apply_iris_confirmation(
            session,
            &data.candidate,
            &data.block,
            data.detected_at,
            &TaskCancellation::default(),
        )
        .await
}

#[tokio::test]
async fn only_explicit_single_work_questions_can_be_answered() {
    let f = fixture();
    let session = register(&f).await.binding;
    let record = publish(&f, session, "Read the example? (Y:1/N:0)").await;
    assert_eq!(
        iris_confirmation_question(record.data()).as_deref(),
        Some("Read the example?")
    );
    for body in [
        "Read the example?",
        "Choose one\n1. Read\n2. Skip",
        "Proceed? (Y:1/N:0)\n1. Different choice",
        "First? Second? (Y:1/N:0)",
        "Proceed? (Y:0/N:1)",
        "Proceed? (Y:1/N:0) extra",
        "Use (Y:1/N:0) or skip? (Y:1/N:0)",
        "1. Read 2. Write? (Y:1/N:0)",
    ] {
        let mut data = record.data().clone();
        data.block = format!("{}\n{body}\n{}", marker::OPEN, marker::CLOSE);
        assert!(iris_confirmation_question(&data).is_none(), "{body}");
    }
    let mut data = record.data().clone();
    data.origin = "native".to_owned();
    assert!(iris_confirmation_question(&data).is_none());
}

#[tokio::test]
async fn applied_requires_the_bound_pty_ack_and_duplicate_decisions_do_not_write() {
    let f = fixture();
    let session = register(&f).await.binding;
    let record = publish(&f, session, "Read the example? (Y:1/N:0)").await;
    let engine = f.engine.clone();
    *f.transport.on_send.lock().unwrap() = Some(Arc::new(move |binding, message| {
        let engine = engine.clone();
        Box::pin(async move {
            engine.acknowledge(binding, InputSeq(message.input_seq));
        })
    }));
    assert_eq!(
        apply(&f, session, &record).await,
        IrisAnswerOutcome::Applied
    );
    assert_eq!(
        apply(&f, session, &record).await,
        IrisAnswerOutcome::Expired
    );
    let frames = f.transport.frames.lock().unwrap();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].1.data, b"1\r");
}

#[tokio::test(start_paused = true)]
async fn missing_ack_is_unknown_and_consent_is_not_replayed() {
    let f = fixture();
    let session = register(&f).await.binding;
    let record = publish(&f, session, "Read the example? (Y:1/N:0)").await;
    let engine = f.engine.clone();
    *f.transport.on_send.lock().unwrap() = Some(Arc::new(move |mut binding, message| {
        let engine = engine.clone();
        Box::pin(async move {
            binding.wrapper = WrapperConnectionId(binding.wrapper.0 + 1);
            engine.acknowledge(binding, InputSeq(message.input_seq));
        })
    }));
    assert_eq!(
        apply(&f, session, &record).await,
        IrisAnswerOutcome::Unknown
    );
    assert_eq!(
        apply(&f, session, &record).await,
        IrisAnswerOutcome::Expired
    );
    assert_eq!(f.transport.frames.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn changed_question_is_rejected_at_dispatch() {
    let f = fixture();
    let session = register(&f).await.binding;
    let record = publish(&f, session, "Read the example? (Y:1/N:0)").await;
    f.transport.pause_call.store(1, Ordering::SeqCst);
    let (outcome, ()) = tokio::join!(apply(&f, session, &record), async {
        f.transport.entered.notified().await;
        publish(&f, session, "Read the appendix? (Y:1/N:0)").await;
        f.transport.resume.notify_one();
    });
    assert_eq!(outcome, IrisAnswerOutcome::Expired);
    assert!(f.transport.frames.lock().unwrap().is_empty());
}

#[tokio::test]
async fn locally_started_answer_prevents_remote_input() {
    let f = fixture();
    let session = register(&f).await.binding;
    let record = publish(&f, session, "Read the example? (Y:1/N:0)").await;
    let receipt = f
        .engine
        .submit(
            session,
            InputRequest {
                bytes: b"0".to_vec(),
                authority: InputAuthority::Ui(ui(&f, 77)),
            },
            now(),
            &TaskCancellation::default(),
        )
        .await;
    assert!(matches!(
        receipt.disposition,
        InputDisposition::TransportWritten { .. }
    ));
    assert_eq!(
        apply(&f, session, &record).await,
        IrisAnswerOutcome::Expired
    );
    assert_eq!(f.transport.frames.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn binding_epoch_and_registration_guard_the_fixed_answer() {
    let f = fixture();
    let session = register(&f).await.binding;
    let record = publish(&f, session, "Read the example? (Y:1/N:0)").await;
    let mut stale = session;
    stale.incarnation.0 += 1;
    assert_eq!(apply(&f, stale, &record).await, IrisAnswerOutcome::Expired);
    let mut stale = record.data().clone();
    stale.candidate.source_epoch.0 += 1;
    assert_eq!(
        apply(&f, session, &ImmutableApprovalRecord::new(stale)).await,
        IrisAnswerOutcome::Expired
    );
    let mut stale = record.data().clone();
    stale.detected_at = stale.detected_at + Duration::from_secs(1);
    assert_eq!(
        apply(&f, session, &ImmutableApprovalRecord::new(stale)).await,
        IrisAnswerOutcome::Expired
    );
    f.engine
        .set_initial_gate(session, Timestamp::now())
        .unwrap();
    assert_eq!(
        apply(&f, session, &record).await,
        IrisAnswerOutcome::Expired
    );
    assert!(f.transport.frames.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_disappeared_vt_question_cannot_answer_an_empty_composer() {
    let f = fixture();
    let binding = register(&f).await.binding;
    let block = format!(
        "{}\r\nRead the example? (Y:1/N:0)\r\n{}",
        marker::OPEN,
        marker::CLOSE
    );
    for (i, display) in [block.as_str(), "Ready", block.as_str()]
        .into_iter()
        .enumerate()
    {
        let effects = f
            .engine
            .observe_output(
                binding,
                OutputChunk {
                    bytes: format!("\x1b[2J\x1b[H{display}").into_bytes(),
                    total_pty_bytes: 0,
                },
                now(),
            )
            .unwrap();
        f.sink.apply(effects).await.unwrap();
        let current = f
            .engine
            .details(binding.session)
            .unwrap()
            .approval
            .record
            .unwrap();
        let data = current.data();
        assert_eq!(
            f.engine.iris_confirmation_is_current(
                binding,
                &data.candidate,
                &data.block,
                data.detected_at
            ),
            i == 0
        );
        if i > 0 {
            assert_eq!(
                apply(&f, binding, &current).await,
                IrisAnswerOutcome::Expired
            );
        }
    }
    assert!(f.transport.frames.lock().unwrap().is_empty());
}

#[tokio::test]
async fn newer_transcript_message_retires_a_cached_question() {
    use many_ai_cli::approval::transcript::parser::AgentChatMessage;
    let f = fixture();
    let binding = f
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
        .unwrap()
        .binding;
    let mut message = AgentChatMessage {
        role: "assistant".into(),
        text: format!(
            "{}\nRead the example? (Y:1/N:0)\n{}",
            marker::OPEN,
            marker::CLOSE
        ),
        ..Default::default()
    };
    let path = std::path::PathBuf::from("fixture.jsonl");
    let effects = f
        .engine
        .observe_transcript_batch(
            binding,
            path.clone(),
            std::slice::from_ref(&message),
            false,
            now(),
        )
        .unwrap();
    f.sink.apply(effects).await.unwrap();
    let current = f
        .engine
        .details(binding.session)
        .unwrap()
        .approval
        .record
        .unwrap();
    message.text = "Return to the previous task.".into();
    let effects = f
        .engine
        .observe_transcript_batch(binding, path, &[message], false, now())
        .unwrap();
    f.sink.apply(effects).await.unwrap();
    assert_eq!(
        apply(&f, binding, &current).await,
        IrisAnswerOutcome::Expired
    );
    assert!(f.transport.frames.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn stalled_socket_before_or_after_dispatch_releases_the_lane() {
    for dispatched in [false, true] {
        let f = fixture();
        let session = register(&f).await.binding;
        let record = publish(&f, session, "Read the example? (Y:1/N:0)").await;
        if dispatched {
            *f.transport.on_send.lock().unwrap() =
                Some(Arc::new(|_, _| Box::pin(std::future::pending())));
        } else {
            f.transport.pause_call.store(1, Ordering::SeqCst);
        }
        assert_eq!(
            apply(&f, session, &record).await,
            if dispatched {
                IrisAnswerOutcome::Unknown
            } else {
                IrisAnswerOutcome::Expired
            }
        );
        assert_eq!(
            f.transport.frames.lock().unwrap().len(),
            usize::from(dispatched)
        );
    }
}
