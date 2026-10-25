use super::*;
use std::sync::atomic::AtomicBool;

struct Authority(AtomicBool);
impl UiAuthorization for Authority {
    fn is_valid(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}
fn authorized(f: &Fixture, connection: u64, authority: Arc<Authority>) -> UiBinding {
    let ui = UiBinding {
        connection: UiConnectionId(connection),
        auth_epoch: f.engine.auth_epoch(),
    };
    f.engine
        .attach_ui_authorized(ui, None, None, authority)
        .unwrap();
    ui
}

#[tokio::test]
async fn external_ui_authority_rechecks_after_fifo_wait_and_never_mutates_rejected_input() {
    let f = fixture();
    let session = register(&f).await.binding;
    let authority = Arc::new(Authority(AtomicBool::new(true)));
    let ui = authorized(&f, 71, authority.clone());
    let before = f.engine.snapshot(session.session).unwrap();
    let cancel = TaskCancellation::default();
    let predecessor = f.engine.submit(
        session,
        InputRequest {
            bytes: b"unpolled predecessor".to_vec(),
            authority: InputAuthority::Internal,
        },
        now(),
        &cancel,
    );
    let mut queued = f.engine.submit(
        session,
        InputRequest {
            bytes: b"must not send\r".to_vec(),
            authority: InputAuthority::Ui(ui),
        },
        now(),
        &cancel,
    );
    assert!(futures_util::poll!(&mut queued).is_pending());
    authority.0.store(false, Ordering::Release);
    drop(predecessor);
    assert_eq!(
        queued.await.disposition,
        InputDisposition::AuthenticationExpired
    );
    assert!(f.transport.frames.lock().unwrap().is_empty());
    assert_eq!(
        f.engine.snapshot(session.session).unwrap().first_message,
        before.first_message
    );
    assert!(f.engine.authorize_ui_work(ui).is_err());
    assert!(
        f.engine
            .reset_history_from_ui(ui, session.session, now())
            .is_err()
    );
    let fresh = authorized(&f, 72, Arc::new(Authority(AtomicBool::new(true))));
    let accepted = f
        .engine
        .submit(
            session,
            InputRequest {
                bytes: b"fresh control".to_vec(),
                authority: InputAuthority::Ui(fresh),
            },
            now(),
            &cancel,
        )
        .await;
    assert!(matches!(
        accepted.disposition,
        InputDisposition::TransportWritten { .. }
    ));
    assert_eq!(
        f.transport.frames.lock().unwrap()[0].1.data,
        b"fresh control"
    );
}

#[tokio::test]
async fn external_ui_revocation_drains_accepted_paste_but_denies_later_input() {
    let f = fixture();
    let session = register(&f).await.binding;
    let authority = Arc::new(Authority(AtomicBool::new(true)));
    let ui = authorized(&f, 73, authority.clone());
    f.transport.pause_call.store(1, Ordering::SeqCst);
    let engine = f.engine.clone();
    let accepted = tokio::spawn(async move {
        engine
            .submit(
                session,
                InputRequest {
                    bytes: b"\x1b[200~accepted\x1b[201~\r".to_vec(),
                    authority: InputAuthority::Ui(ui),
                },
                now(),
                &TaskCancellation::default(),
            )
            .await
    });
    f.transport.entered.notified().await;
    authority.0.store(false, Ordering::Release);
    let effects = f.engine.detach_ui(ui);
    assert!(
        matches!(effects.0.as_slice(), [CoreEffect::DrainUi(binding), CoreEffect::CloseUi(closed)] if *binding == ui && *closed == ui)
    );
    let engine = f.engine.clone();
    let rejected = tokio::spawn(async move {
        engine
            .submit(
                session,
                InputRequest {
                    bytes: b"must not send".to_vec(),
                    authority: InputAuthority::Ui(ui),
                },
                now(),
                &TaskCancellation::default(),
            )
            .await
    });
    tokio::task::yield_now().await;
    assert!(
        !rejected.is_finished(),
        "queued input must wait behind accepted work"
    );
    let engine = f.engine.clone();
    let drain = tokio::spawn(async move { engine.drain_ui_work(ui).await });
    tokio::task::yield_now().await;
    assert!(!drain.is_finished());
    f.transport.resume.notify_one();
    assert!(matches!(
        accepted.await.unwrap().disposition,
        InputDisposition::TransportWritten { .. }
    ));
    assert_eq!(
        rejected.await.unwrap().disposition,
        InputDisposition::AuthenticationExpired
    );
    drain.await.unwrap();
    let frames = f.transport.frames.lock().unwrap();
    // No synthetic output: preserve the accepted submit's one Enter retry.
    assert_eq!(frames.len(), 3);
    assert_eq!(frames[1].1.data, b"\r");
    assert_eq!(frames[2].1.data, b"\r");
    assert!(f.engine.details(session.session).unwrap().connected);
}

#[tokio::test]
async fn external_ui_authority_is_checked_at_attach_and_before_priming_completes() {
    let f = fixture();
    let authority = Arc::new(Authority(AtomicBool::new(false)));
    let ui = UiBinding {
        connection: UiConnectionId(74),
        auth_epoch: f.engine.auth_epoch(),
    };
    assert!(matches!(
        f.engine
            .attach_ui_authorized(ui, None, None, authority.clone()),
        Err(SessionError::AuthenticationExpired)
    ));
    assert_eq!(f.engine.subscribe_ui_presence().borrow().count, 0);
    authority.0.store(true, Ordering::Release);
    f.engine
        .attach_ui_authorized(ui, None, None, authority.clone())
        .unwrap();
    authority.0.store(false, Ordering::Release);
    assert!(matches!(
        f.engine.finish_ui_priming(ui),
        Err(SessionError::AuthenticationExpired)
    ));
    assert!(f.engine.authorize_ui_work(ui).is_err());
}
