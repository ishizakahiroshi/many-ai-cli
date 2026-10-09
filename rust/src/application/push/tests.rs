use super::*;
use crate::hub::task_owner::HubTaskOwner;
use std::sync::atomic::{AtomicU16, AtomicUsize, Ordering};
struct Client {
    code: AtomicU16,
    calls: AtomicUsize,
    fail: std::sync::atomic::AtomicBool,
}
impl PushHttpClient for Client {
    fn send<'a>(
        &'a self,
        _: &'a str,
        request: crypto::EncodedPush,
        _: &'a TaskCancellation,
    ) -> CoreFuture<'a, io::Result<u16>> {
        assert_eq!(request.body.len(), 4096);
        assert!(request.authorization.starts_with("vapid t="));
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            if self.fail.load(Ordering::SeqCst) {
                Err(io::Error::other("synthetic-only transport failure"))
            } else {
                Ok(self.code.load(Ordering::SeqCst))
            }
        })
    }
}
fn sub(endpoint: &str) -> Subscription {
    let vectors: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/web-push/go-encryption-vectors.json"
    ))
    .unwrap();
    Subscription {
        endpoint: endpoint.into(),
        keys: Keys {
            auth: vectors[0]["auth"].as_str().unwrap().into(),
            p256dh: vectors[0]["p256dh"].as_str().unwrap().into(),
        },
        ..Default::default()
    }
}
struct Fixture {
    manager: Arc<PushManager>,
    client: Arc<Client>,
    tasks: HubTaskOwner,
    paths: RuntimePaths,
    _root: tempfile::TempDir,
    _installed: tempfile::TempDir,
}
fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49272, installed.path()).unwrap();
    let tasks = HubTaskOwner::new(tokio::runtime::Handle::current());
    let client = Arc::new(Client {
        code: AtomicU16::new(201),
        calls: AtomicUsize::new(0),
        fail: std::sync::atomic::AtomicBool::new(false),
    });
    let manager =
        PushManager::new(&paths, tasks.handle(), client.clone(), Arc::new(|_, _| {})).unwrap();
    manager
        .upsert(
            sub("https://example.invalid/synthetic-secret-endpoint"),
            Timestamp::now(),
        )
        .unwrap();
    Fixture {
        manager,
        client,
        tasks,
        paths,
        _root: root,
        _installed: installed,
    }
}
async fn send(f: &Fixture, id: &str) {
    f.manager
        .send_approval(ApprovalPayload {
            id: id.into(),
            body: "synthetic approval".into(),
            ..Default::default()
        })
        .unwrap();
    f.tasks.drain_effects().await;
}
#[tokio::test]
async fn dedup_only_success_all_failures_and_429_retry_expiry_prunes_persisted_store() {
    let f = fixture();
    f.client.fail.store(true, Ordering::SeqCst);
    send(&f, "retry").await;
    send(&f, "retry").await;
    assert_eq!(f.client.calls.load(Ordering::SeqCst), 2);
    f.client.fail.store(false, Ordering::SeqCst);
    f.client.code.store(429, Ordering::SeqCst);
    send(&f, "retry").await;
    send(&f, "retry").await;
    assert_eq!(f.client.calls.load(Ordering::SeqCst), 4);
    f.client.code.store(201, Ordering::SeqCst);
    send(&f, "retry").await;
    send(&f, "retry").await;
    assert_eq!(f.client.calls.load(Ordering::SeqCst), 5);
    f.client.code.store(410, Ordering::SeqCst);
    send(&f, "expired").await;
    assert_eq!(f.manager.status().subscriptions, 0);
    assert!(!f.manager.state.lock().unwrap().sent.contains_key("expired"));
    let reloaded = PushManager::new(
        &f.paths,
        f.tasks.handle(),
        f.client.clone(),
        Arc::new(|_, _| {}),
    )
    .unwrap();
    assert_eq!(reloaded.status().subscriptions, 0);
    assert_eq!(reloaded.public_key(), f.manager.public_key());
}
#[tokio::test]
async fn security_has_no_approval_dedup_and_public_status_never_exposes_secrets() {
    let f = fixture();
    for _ in 0..2 {
        f.manager
            .send_security("synthetic title".into(), "body".into())
            .unwrap();
        f.tasks.drain_effects().await;
    }
    assert_eq!(f.client.calls.load(Ordering::SeqCst), 2);
    let status = serde_json::to_value(f.manager.status()).unwrap();
    assert!(status.get("vapid_private_key").is_none());
    assert!(!status.to_string().contains("synthetic-secret-endpoint"));
    assert!(f.manager.state.lock().unwrap().sent.is_empty());
    assert!(
        f.manager
            .upsert(sub("http://127.0.0.1/push"), Timestamp::now())
            .is_err()
    );
}
#[tokio::test]
async fn native_trial_refuses_host_delivery_and_abort_does_not_mark_success() {
    let f = fixture();
    let store = f.manager.state.lock().unwrap().store.clone();
    let sub = &store.subscriptions[0];
    let request = crypto::encode(
        &store,
        &sub.keys,
        &sub.endpoint,
        b"synthetic",
        hash("trial", 24),
        Timestamp::now().unix_seconds(),
    )
    .unwrap();
    let native = native::NativePushHttpClient::new(&f.paths);
    let result = native
        .send(&sub.endpoint, request, &TaskCancellation::default())
        .await;
    assert_eq!(
        result.err().unwrap().kind(),
        io::ErrorKind::PermissionDenied
    );
    let cancel = TaskCancellation::default();
    cancel.cancel();
    f.manager
        .deliver(
            store,
            "cancelled".into(),
            b"synthetic".to_vec(),
            true,
            &cancel,
        )
        .await;
    assert_eq!(f.client.calls.load(Ordering::SeqCst), 0);
    assert!(
        !f.manager
            .state
            .lock()
            .unwrap()
            .sent
            .contains_key("cancelled")
    );
}
#[test]
fn utf8_truncation_never_splits_scalar_and_preserves_source_trim() {
    assert_eq!(truncate("aあz", 3), "a");
    assert_eq!(truncate("  abc xyz", 5), "abc");
    assert_eq!(hash("approval", 24).len(), 24);
}
