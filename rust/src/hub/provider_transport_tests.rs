use super::{ServiceRouter, http::Request, transport};
use crate::{
    config::{Config, ConfigStore, RuntimePaths},
    process::Cancellation,
    profile::store::ProviderRegistryStore,
    proto::core::{CoreEffectFailure, CoreEffectSink, CoreEffects, CoreFuture},
};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};

struct Sink;
impl CoreEffectSink for Sink {
    fn apply<'a>(&'a self, effects: CoreEffects) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
        assert!(effects.0.is_empty());
        Box::pin(async { Ok(()) })
    }
}
struct Cancel(Cancellation);
impl Drop for Cancel {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
#[tokio::test]
async fn served_provider_crud_uses_persisted_registry_and_ignores_delete_body() {
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let root = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(
        root.path(),
        port,
        &root.path().join("../installed-provider-transport"),
    )
    .unwrap();
    let config = Arc::new(
        ConfigStore::new(
            paths.clone(),
            Config {
                token: "synthetic-provider-http-token".into(),
                ..Default::default()
            },
        )
        .unwrap(),
    );
    let store = Arc::new(ProviderRegistryStore::new(&paths, config.clone()).unwrap());
    let router = Arc::new(
        ServiceRouter::isolated_at_port(config.clone(), paths.clone(), port)
            .unwrap()
            .with_provider_routes(
                store.clone(),
                Arc::new(|_| false),
                Arc::new(|_, _, _| panic!("unexpected provider warning")),
            ),
    );
    let cancelled = Cancel(Cancellation::default());
    let server = tokio::spawn(transport::serve(
        listener,
        router,
        Arc::new(Sink),
        cancelled.0.clone(),
    ));
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let base = format!("http://127.0.0.1:{port}/api/providers");
    let token = "token=synthetic-provider-http-token";
    assert_eq!(client.get(&base).send().await.unwrap().status(), 401);
    let created = client.post(format!("{base}?{token}")).json(&json!({"schema_version":1,"id":"fixture-http","display_name":"HTTP fixture","launch":{"executable":"fixture-only-no-execution"}})).send().await.unwrap();
    assert_eq!(created.status(), 201);
    let created: Value = created.json().await.unwrap();
    let revision = created["revision"].as_str().unwrap();
    let listed = client.get(format!("{base}?{token}")).send().await.unwrap();
    assert_eq!(listed.headers()["cache-control"], "no-store");
    let listed: Value = listed.json().await.unwrap();
    assert!(
        listed["providers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == "fixture-http")
    );
    assert!(
        listed["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "command_missing" && d["field"] == "fixture-http")
    );
    let restarted = ProviderRegistryStore::new(&paths, config).unwrap();
    assert!(
        restarted
            .snapshot()
            .unwrap()
            .registry
            .lookup("fixture-http")
            .is_some()
    );
    let removed = client
        .delete(format!(
            "{base}/fixture-http?{token}&expected_revision={revision}"
        ))
        .header("Content-Length", "4000")
        .body(reqwest::Body::wrap_stream(futures_util::stream::pending::<
            Result<axum::body::Bytes, std::io::Error>,
        >()))
        .send()
        .await
        .unwrap();
    assert_eq!(removed.status(), 200);
    assert!(
        store
            .snapshot()
            .unwrap()
            .registry
            .lookup("fixture-http")
            .is_none()
    );
    assert_eq!(
        client
            .get(format!("{base}/fixture-http?{token}"))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    cancelled.0.cancel();
    server.await.unwrap().unwrap();
}

#[test]
fn provider_prefix_retains_source_path_method_and_auth_order() {
    let root = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(
        root.path(),
        49891,
        &root.path().join("../installed-provider-prefix"),
    )
    .unwrap();
    let config = Arc::new(
        ConfigStore::new(
            paths.clone(),
            Config {
                token: "synthetic-prefix-token".into(),
                ..Default::default()
            },
        )
        .unwrap(),
    );
    let router = ServiceRouter::isolated(config, paths);
    for (path, method, expected) in [
        ("/api/providers/fixture", "POST", 405),
        ("/api/providers/fixture/unknown", "GET", 404),
        ("/api/providers/", "GET", 401),
        ("/api/providers", "DELETE", 401),
        ("/api/providers/validate", "PUT", 401),
        ("/api/providers//history", "GET", 401),
    ] {
        let request = Request {
            method: method.into(),
            path: path.into(),
            host: "evil.invalid".into(),
            ..Default::default()
        };
        assert_eq!(
            router.preflight(&request, 1).unwrap_err().status,
            expected,
            "{path}"
        );
        assert_eq!(
            router.handle(&request, 1).response.status,
            expected,
            "{path}"
        );
    }
    let request = Request {
        method: "PATCH".into(),
        path: "/api/providers/".into(),
        host: "127.0.0.1:49891".into(),
        query: "token=synthetic-prefix-token".into(),
        ..Default::default()
    };
    assert!(router.preflight(&request, 1).is_ok());
    assert_eq!(router.handle(&request, 1).response.status, 404);
}
