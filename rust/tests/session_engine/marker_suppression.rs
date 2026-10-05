//! Actual SessionEngine caller behavior, independent of corpus simulations.
use super::*;
use many_ai_cli::approval::marker::{self, Marker};

fn corrupt(text: &str) -> Marker {
    marker::extract(&format!("{}\n3. {text}\n{}", marker::OPEN, marker::CLOSE)).unwrap()
}
fn notices(f: &Fixture) -> Vec<proto::Message> {
    f.sink
        .ui
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, message)| message.r#type == "approval_marker_suppressed")
        .map(|(_, message)| message.clone())
        .collect()
}
async fn transcript(f: &Fixture, binding: SessionBinding, marker: Option<Marker>, at: Timestamp) {
    let effects = f
        .engine
        .observe_transcript_marker(binding, marker, at)
        .unwrap();
    f.sink.apply(effects).await.unwrap();
}
async fn vt(f: &Fixture, binding: SessionBinding, marker: &Marker, at: Timestamp) {
    f.sink
        .apply(
            f.engine
                .observe_output(
                    binding,
                    OutputChunk {
                        bytes: format!("\x1b[2J\x1b[H{}", marker.block).into_bytes(),
                        total_pty_bytes: 0,
                    },
                    at,
                )
                .unwrap(),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn corrupt_markers_keep_pending_record_and_share_vt_transcript_throttle() {
    let f = fixture();
    let binding = register(&f).await.binding;
    ui(&f, 1);
    let valid = marker::extract(&format!(
        "{}\nChoose a synthetic action\n1. First\n2. Other\n{}",
        marker::OPEN,
        marker::CLOSE
    ))
    .unwrap();
    transcript(&f, binding, Some(valid), now()).await;
    let record = f
        .engine
        .details(binding.session)
        .unwrap()
        .approval
        .record
        .unwrap();
    let a = corrupt("Synthetic missing first option");
    let b = corrupt("Another synthetic missing first option");
    vt(&f, binding, &a, now() + Duration::from_secs(1)).await;
    transcript(
        &f,
        binding,
        Some(b.clone()),
        now() + Duration::from_secs(10),
    )
    .await;
    transcript(
        &f,
        binding,
        Some(a.clone()),
        now() + Duration::from_secs(11),
    )
    .await;
    assert_eq!(notices(&f).len(), 1);
    transcript(
        &f,
        binding,
        Some(a.clone()),
        now() + Duration::from_secs(40),
    )
    .await;
    assert_eq!(notices(&f).len(), 1);
    transcript(
        &f,
        binding,
        Some(b.clone()),
        now() + Duration::from_secs(41),
    )
    .await;
    assert_eq!(notices(&f).len(), 2);
    transcript(&f, binding, None, now() + Duration::from_secs(80)).await;
    vt(&f, binding, &b, now() + Duration::from_secs(81)).await;
    assert_eq!(notices(&f).len(), 2);
    let current = f
        .engine
        .details(binding.session)
        .unwrap()
        .approval
        .record
        .unwrap();
    assert!(current.data() == record.data());
    for notice in notices(&f) {
        assert_eq!(notice.reason, "option_start");
        assert!(
            serde_json::to_value(&notice)
                .unwrap()
                .get("block")
                .is_none()
        );
        assert!(!notice.approval_sig.is_empty());
    }
    assert_eq!(notices(&f)[0].approval_source, "go_vt");
    assert_eq!(notices(&f)[1].approval_source, "transcript");
}

#[tokio::test]
async fn warm_reattach_preserves_suppression_and_reset_history_reopens_notice() {
    let f = fixture();
    let old = register(&f).await.binding;
    ui(&f, 1);
    let marker = corrupt("Synthetic warm reconnect");
    transcript(&f, old, Some(marker.clone()), now()).await;
    let current = reattach(&f, old, b"", 0, 2, 0).await.binding;
    transcript(
        &f,
        current,
        Some(marker.clone()),
        now() + Duration::from_secs(60),
    )
    .await;
    assert_eq!(notices(&f).len(), 1);
    assert!(matches!(
        f.engine
            .observe_transcript_marker(old, Some(marker.clone()), now()),
        Err(SessionError::StaleBinding)
    ));
    let missing = SessionBinding {
        session: LiveSessionId(999_999),
        ..current
    };
    assert!(matches!(
        f.engine
            .observe_transcript_marker(missing, Some(marker.clone()), now()),
        Err(SessionError::NotFound(_))
    ));
    assert_eq!(notices(&f).len(), 1);
    f.sink
        .apply(f.engine.reset_history(current.session, now()).unwrap())
        .await
        .unwrap();
    transcript(&f, current, Some(marker), now() + Duration::from_secs(61)).await;
    assert_eq!(notices(&f).len(), 2);
}

#[tokio::test]
async fn suppression_warnings_reenter_engine_after_unlock_and_omit_marker_body() {
    let weak = Arc::new(Mutex::new(std::sync::Weak::<SessionEngine>::new()));
    let warnings = Arc::new(Mutex::new(Vec::new()));
    let callback_weak = weak.clone();
    let callback_warnings = warnings.clone();
    let warning = Arc::new(move |operation: &str, error: &SessionError| {
        let engine = callback_weak.lock().unwrap().upgrade().unwrap();
        // This read acquires the same mutex, proving the callback runs unlocked.
        assert!(engine.registered_session_count() > 0);
        callback_warnings
            .lock()
            .unwrap()
            .push(format!("{operation} {error:?}"));
    });
    let f = fixture_with_warning(Arc::new(NoSpawn), warning);
    *weak.lock().unwrap() = Arc::downgrade(&f.engine);
    let binding = register(&f).await.binding;
    ui(&f, 1);
    let marker = corrupt("PRIVATE_SYNTHETIC_BODY");
    vt(&f, binding, &marker, now()).await;
    transcript(&f, binding, Some(marker), now() + Duration::from_secs(60)).await;
    transcript(
        &f,
        binding,
        Some(corrupt("OTHER_PRIVATE_SYNTHETIC_BODY")),
        now() + Duration::from_secs(61),
    )
    .await;
    let warnings = warnings.lock().unwrap();
    assert_eq!(warnings.len(), 2);
    assert!(
        warnings
            .iter()
            .all(|warning| warning.contains("reason=option_start")
                && !warning.contains("PRIVATE_SYNTHETIC_BODY"))
    );
    assert_eq!(notices(&f).len(), 2);
}
