use super::*;
use crate::hub::task_owner::HubTaskOwner;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct Backend {
    calls: AtomicUsize,
    fail: AtomicBool,
    gate: tokio::sync::Semaphore,
    bodies: Mutex<Vec<(axum::http::HeaderMap, Vec<u8>)>>,
}
async fn receive(
    axum::extract::State(state): axum::extract::State<Arc<Backend>>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> axum::http::StatusCode {
    state.calls.fetch_add(1, Ordering::SeqCst);
    lock(&state.bodies).push((headers, body.to_vec()));
    let _permit = state.gate.acquire().await.unwrap();
    if state.fail.load(Ordering::SeqCst) {
        axum::http::StatusCode::BAD_GATEWAY
    } else {
        axum::http::StatusCode::NO_CONTENT
    }
}
async fn fixture(
    kind: &str,
    permits: usize,
    paths: Option<&RuntimePaths>,
) -> (
    Arc<Manager>,
    HubTaskOwner,
    Arc<Backend>,
    crate::process::Cancellation,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let state = Arc::new(Backend {
        calls: 0.into(),
        fail: false.into(),
        gate: tokio::sync::Semaphore::new(permits),
        bodies: Mutex::default(),
    });
    let app = axum::Router::new()
        .fallback(axum::routing::post(receive))
        .with_state(state.clone());
    let cancel = crate::process::Cancellation::default();
    let stopping = cancel.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move { stopping.cancelled().await })
            .await
            .unwrap();
    });
    let owner = HubTaskOwner::new(tokio::runtime::Handle::current());
    let config = NotifyConfig {
        backends: Some(vec![NotifyBackendConfig {
            r#type: kind.into(),
            url: format!("http://{address}"),
            topic: "synthetic-topic".into(),
        }]),
        events: Some(vec![" approval ".into(), "done".into()]),
        include_body: true,
    };
    let manager = match paths {
        Some(paths) => Manager::new_for_runtime(config, owner.handle(), Arc::new(|_, _| {}), paths),
        None => Manager::new(config, owner.handle(), Arc::new(|_, _| {})),
    }
    .unwrap();
    (manager, owner, state, cancel, server)
}
async fn calls(state: &Backend, count: usize) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while state.calls.load(Ordering::SeqCst) < count {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn actual_ntfy_delivery_deduplicates_pending_and_sent_and_preserves_safe_actions() {
    let (manager, owner, state, cancel, server) = fixture("ntfy", 0, None).await;
    let payload = ApprovalPayload {
        id: "same".into(),
        session_id: 4,
        title: "approval".into(),
        body: "a".repeat(198) + "ああ",
        risk: "HIGH".into(),
        approve_url: "https://synthetic.example/approve".into(),
        reject_url: "https://synthetic.example/reject".into(),
        open_url: {
            let mut url = url::Url::parse("http://synthetic.example/open").unwrap();
            url.set_username("user").unwrap();
            url.set_password(Some("password")).unwrap();
            url.to_string()
        },
    };
    assert!(manager.send_approval(payload.clone()).unwrap());
    calls(&state, 1).await;
    assert!(!manager.send_approval(payload.clone()).unwrap());
    state.gate.add_permits(1);
    owner.drain_effects().await;
    assert!(!manager.send_approval(payload).unwrap());
    {
        let bodies = lock(&state.bodies);
        assert_eq!(bodies[0].1.len(), 198);
        assert_eq!(
            bodies[0].0["actions"],
            "http, Reject, https://synthetic.example/reject, method=POST"
        );
    }
    cancel.cancel();
    server.await.unwrap();
}
#[tokio::test]
async fn total_failure_is_retryable_and_done_webhook_carries_kind() {
    let (manager, owner, state, cancel, server) = fixture("webhook", 1, None).await;
    state.fail.store(true, Ordering::SeqCst);
    let payload = DonePayload {
        id: "retry".into(),
        session_id: 7,
        title: "session title".into(),
        summary: "completed".into(),
        kind: "success".into(),
    };
    assert!(manager.send_done(payload.clone()).unwrap());
    owner.drain_effects().await;
    state.fail.store(false, Ordering::SeqCst);
    assert!(manager.send_done(payload.clone()).unwrap());
    owner.drain_effects().await;
    assert!(!manager.send_done(payload).unwrap());
    assert_eq!(state.calls.load(Ordering::SeqCst), 2);
    let json: serde_json::Value = serde_json::from_slice(&lock(&state.bodies)[1].1).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"title":"[成功] session title","body":"completed","kind":"success"})
    );
    cancel.cancel();
    server.await.unwrap();
}
#[tokio::test]
async fn abort_releases_claim_and_effect_admission_failure_leaves_no_pending_id() {
    let (manager, owner, state, cancel, server) = fixture("webhook", 0, None).await;
    let payload = DonePayload {
        id: "cancelled".into(),
        ..Default::default()
    };
    assert!(manager.send_done(payload.clone()).unwrap());
    calls(&state, 1).await;
    owner.stop_requests();
    owner.drain_requests().await;
    owner.stop_effects().unwrap();
    owner.abort_effects().unwrap();
    owner.drain_effects().await;
    assert!(lock(&manager.state).pending.is_empty());
    assert!(manager.send_done(payload).is_err());
    assert!(lock(&manager.state).pending.is_empty());
    state.gate.add_permits(1);
    cancel.cancel();
    server.await.unwrap();
}

async fn assert_trial_delivery_denied(
    manager: &Arc<Manager>,
    owner: &HubTaskOwner,
    state: &Backend,
) {
    let before = owner.snapshot();
    assert!(matches!(
        manager.send_approval(ApprovalPayload {
            id: "trial-approval".into(),
            body: "synthetic approval".into(),
            ..Default::default()
        }),
        Ok(false)
    ));
    assert!(matches!(
        manager.send_done(DonePayload {
            id: "trial-done".into(),
            summary: "synthetic completion".into(),
            ..Default::default()
        }),
        Ok(false)
    ));
    assert!(matches!(
        manager.send_security("synthetic title".into(), "synthetic warning".into()),
        Ok(false)
    ));
    let backend = lock(&manager.config).backends.as_ref().unwrap()[0].clone();
    assert!(
        !manager
            .send_test(backend, "synthetic test".into(), "synthetic body".into())
            .await
    );
    owner.drain_effects().await;
    assert_eq!(
        owner.snapshot(),
        before,
        "trial must not acquire an effect permit"
    );
    assert!(lock(&manager.state).pending.is_empty());
    assert!(lock(&manager.state).sent.is_empty());
    assert_eq!(state.calls.load(Ordering::SeqCst), 0);
    assert!(lock(&state.bodies).is_empty());
}

#[tokio::test]
async fn trial_denies_all_backends_before_admission_and_transport_even_after_config_updates() {
    let temporary = tempfile::tempdir().unwrap();
    let trial = temporary.path().join("trial");
    std::fs::create_dir(&trial).unwrap();
    let paths = RuntimePaths::trial(&trial, 49179, &temporary.path().join("installed")).unwrap();
    for backend in ["webhook", "ntfy"] {
        // A listening synthetic receiver makes an accidental send observable;
        // success cannot be explained by an unreachable external service.
        let (manager, owner, state, cancel, server) = fixture(backend, 1, Some(&paths)).await;
        assert_trial_delivery_denied(&manager, &owner, &state).await;

        let mut enabled = lock(&manager.config).clone();
        manager.update_config(NotifyConfig::default());
        enabled.events = Some(vec!["approval".into(), "done".into()]);
        enabled.include_body = true;
        manager.update_config(enabled);
        assert_trial_delivery_denied(&manager, &owner, &state).await;

        // Trial denial precedes even a closed effect lane, rather than creating
        // a dedup claim and relying on effect admission to prevent delivery.
        owner.stop_requests();
        owner.drain_requests().await;
        owner.stop_effects().unwrap();
        assert_trial_delivery_denied(&manager, &owner, &state).await;
        cancel.cancel();
        server.await.unwrap();
    }
}

#[tokio::test]
async fn production_runtime_retains_all_notification_delivery_paths() {
    let temporary = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::production(temporary.path()).unwrap();
    for backend in ["webhook", "ntfy"] {
        let (manager, owner, state, cancel, server) = fixture(backend, 1, Some(&paths)).await;
        assert!(
            manager
                .send_approval(ApprovalPayload {
                    id: "production-approval".into(),
                    ..Default::default()
                })
                .unwrap()
        );
        assert!(
            manager
                .send_done(DonePayload {
                    id: "production-done".into(),
                    ..Default::default()
                })
                .unwrap()
        );
        assert!(
            manager
                .send_security("synthetic title".into(), "synthetic warning".into())
                .unwrap()
        );
        let test_backend = lock(&manager.config).backends.as_ref().unwrap()[0].clone();
        assert!(
            manager
                .send_test(
                    test_backend,
                    "synthetic test".into(),
                    "synthetic body".into()
                )
                .await
        );
        owner.drain_effects().await;
        assert_eq!(state.calls.load(Ordering::SeqCst), 4);
        assert_eq!(owner.snapshot().effects.completed, 3);
        assert_eq!(lock(&manager.state).sent.len(), 3);
        assert!(lock(&manager.state).pending.is_empty());
        cancel.cancel();
        server.await.unwrap();
    }
}

#[test]
fn labels_actions_utf8_and_random_topics_follow_source_contract() {
    assert_eq!(kind_label(" needs_action "), "要判断");
    assert_eq!(kind_label("other"), "完了");
    assert_eq!(truncate("あいう", 5), "あ");
    assert!(!safe_action("https://synthetic.example/a#fragment"));
    assert!(safe_action("https://synthetic.example/a#"));
    assert!(!safe_action("https://@synthetic.example/a"));
    assert!(!safe_action("file:///a"));
    let topic = random_topic().unwrap();
    assert_eq!(topic.len(), 33);
    assert!(topic.starts_with("anyaicli-"));
    assert!(
        topic[9..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    );
}
