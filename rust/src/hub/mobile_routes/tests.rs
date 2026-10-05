use super::*;
use crate::{
    application::mobile_connect::{CommandOutput, MobileIdentity, MobileIo, Program},
    config::{Config, ConfigStore, RuntimePaths},
    launcher::ExportIdentity,
    proto::core::CoreFuture,
};
use std::sync::atomic::{AtomicUsize, Ordering};
struct Io {
    calls: AtomicUsize,
}
impl MobileIo for Io {
    fn windows(&self) -> bool {
        true
    }
    fn available(&self, _: Program) -> bool {
        false
    }
    fn run<'a>(
        &'a self,
        _: Program,
        _: &'a [&'a str],
        _: bool,
        _: &'a Cancellation,
    ) -> CoreFuture<'a, CommandOutput> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            CommandOutput::default()
        })
    }
}
#[tokio::test]
async fn method_guard_precedes_service_work_and_source_post_delete_ignore_all_body_bytes() {
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49328, installed.path()).unwrap();
    let config = Arc::new(
        ConfigStore::new(
            paths.clone(),
            Config {
                token: "synthetic".into(),
                ..Config::defaults(&paths)
            },
        )
        .unwrap(),
    );
    let io = Arc::new(Io {
        calls: AtomicUsize::new(0),
    });
    let owner = MobileConnect::new(
        config,
        io.clone(),
        MobileIdentity::from_export(ExportIdentity::default(), &[]),
        49328,
        Arc::new(|_| {}),
    );
    let http = MobileHttp::new(owner);
    let cancel = Cancellation::default();
    let response = http
        .handle_authenticated(
            &Request {
                method: "PUT".into(),
                path: "/api/mobile-connect".into(),
                ..Default::default()
            },
            &cancel,
        )
        .await;
    assert_eq!(response.status, 405);
    assert_eq!(io.calls.load(Ordering::SeqCst), 0);
    for method in ["POST", "DELETE"] {
        let response = http
            .handle_authenticated(
                &Request {
                    method: method.into(),
                    path: "/api/mobile-connect/tailscale/serve".into(),
                    body: vec![0xff; 1024 * 1024 + 10],
                    ..Default::default()
                },
                &cancel,
            )
            .await;
        assert_eq!(response.status, 200);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&response.body).unwrap()["ok"],
            false
        );
        assert!(!response.headers.contains_key("Cache-Control"));
    }
    let response = http
        .handle_authenticated(
            &Request {
                method: "GET".into(),
                path: "/api/mobile-connect/tailscale".into(),
                ..Default::default()
            },
            &cancel,
        )
        .await;
    assert_eq!(response.status, 200);
    assert_eq!(
        response.headers.get("Cache-Control").map(String::as_str),
        Some("no-store")
    );
    assert!(
        !String::from_utf8(response.body)
            .unwrap()
            .contains("synthetic")
    );
}
