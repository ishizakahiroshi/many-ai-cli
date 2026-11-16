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

struct Idle;
impl crate::hub::sockets::OrderedEventObserver for Idle {
    fn observe<'a>(
        &'a self,
        _: &'a crate::proto::core::CoreEvent,
    ) -> CoreFuture<'a, Result<(), crate::proto::core::SessionError>> {
        Box::pin(async { Ok(()) })
    }
}
impl crate::proto::core::WrappedSessionSpawner for Idle {
    fn spawn_and_wait<'a>(
        &'a self,
        _: crate::proto::core::WrappedSpawnSpec,
        _: Duration,
        _: &'a crate::proto::core::HttpWaitCancellation,
    ) -> CoreFuture<'a, crate::proto::core::SpawnWaitOutcome> {
        Box::pin(async { panic!("provider routes never spawn") })
    }
}
struct NullUi;
impl crate::hub::sockets::FrameWriter for NullUi {
    fn write<'a>(
        &'a mut self,
        _: crate::hub::sockets::WireFrame,
    ) -> CoreFuture<'a, Result<(), crate::proto::core::SessionError>> {
        Box::pin(async { Ok(()) })
    }
}
/// With one primed dashboard attached, one Hub broadcast reaches it as one send.
fn broadcasts(effects: &CoreEffects) -> Vec<crate::proto::Message> {
    effects
        .0
        .iter()
        .filter_map(|effect| match effect {
            crate::proto::core::CoreEffect::SendUiBestEffort { message, .. } => {
                Some(message.clone())
            }
            _ => None,
        })
        .collect()
}
#[test]
fn registry_mutations_broadcast_one_providers_updated_and_failures_none() {
    use crate::{
        hub::sockets::{EffectDriver, SocketRegistry},
        proto::core::SessionCore,
        terminal::{
            events::CoreEventBus,
            journal::{JournalOptions, SessionJournal},
            session::{EngineOptions, SessionEngine},
        },
    };
    let root = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(
        root.path(),
        49893,
        &root.path().join("../installed-provider-broadcast"),
    )
    .unwrap();
    let config = Arc::new(
        ConfigStore::new(
            paths.clone(),
            Config {
                token: "synthetic-broadcast-token".into(),
                ..Default::default()
            },
        )
        .unwrap(),
    );
    let sockets = Arc::new(SocketRegistry::default());
    let journal = Arc::new(SessionJournal::new(
        paths.clone(),
        None,
        JournalOptions::default(),
    ));
    let events = CoreEventBus::new(16).unwrap();
    let effects = Arc::new(EffectDriver::new(
        sockets.clone(),
        journal.clone(),
        Arc::new(events.clone()),
        Arc::new(Idle),
    ));
    let core = Arc::new(SessionEngine::new(
        EngineOptions::default(),
        journal,
        sockets.clone(),
        effects,
        Arc::new(Idle),
        events,
    ));
    let trait_core: Arc<dyn SessionCore> = core.clone();
    sockets.bind_core(Arc::downgrade(&trait_core)).unwrap();
    let ui = crate::proto::core::UiBinding {
        connection: sockets.next_ui().unwrap(),
        auth_epoch: core.auth_epoch(),
    };
    sockets.insert_ui(ui, Box::new(NullUi)).unwrap();
    core.attach_ui(ui, None, None).unwrap();
    assert!(core.finish_ui_priming(ui).unwrap().0.is_empty());
    let store = Arc::new(ProviderRegistryStore::new(&paths, config.clone()).unwrap());
    let changed: Arc<std::sync::Mutex<Vec<(String, &'static str)>>> = Default::default();
    let router = ServiceRouter::new(config, paths, trait_core.clone(), 49893)
        .unwrap()
        .with_provider_routes(
            store.clone(),
            Arc::new(|_| true),
            Arc::new(|_, _, _| panic!("unexpected provider warning")),
        )
        .with_provider_changed({
            let changed = changed.clone();
            Arc::new(move |change| {
                changed
                    .lock()
                    .unwrap()
                    .push((change.id.clone(), change.action));
            })
        });
    let call = |method: &str, path: &str, query: &str, body: Value| {
        let request = Request {
            method: method.into(),
            path: path.into(),
            query: format!("token=synthetic-broadcast-token&{query}"),
            host: "127.0.0.1:49893".into(),
            remote_addr: "127.0.0.1:50000".into(),
            body: serde_json::to_vec(&body).unwrap(),
            ..Default::default()
        };
        router.handle(&request, 1)
    };
    let definition = json!({"schema_version":1,"id":"fixture-bcast","display_name":"Broadcast fixture","launch":{"executable":"fixture-only-no-execution"}});

    let created = call("POST", "/api/providers", "", definition.clone());
    assert_eq!(created.response.status, 201);
    let sent = broadcasts(&created.effects);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].r#type, "providers_updated");
    assert_eq!(sent[0].providers, ["fixture-bcast"]);
    assert_eq!(sent[0].reason, "created");

    let duplicate = call("POST", "/api/providers", "", definition.clone());
    assert_eq!(duplicate.response.status, 409);
    assert!(duplicate.effects.0.is_empty());
    let validated = call("POST", "/api/providers/validate", "", definition);
    assert_eq!(validated.response.status, 200);
    assert!(validated.effects.0.is_empty());
    let stale = call(
        "DELETE",
        "/api/providers/fixture-bcast",
        "expected_revision=stale",
        json!(null),
    );
    assert_eq!(stale.response.status, 409);
    assert!(stale.effects.0.is_empty());

    let revision = store.snapshot().unwrap().registry.revision().to_owned();
    let removed = call(
        "DELETE",
        "/api/providers/fixture-bcast",
        &format!("expected_revision={revision}"),
        json!(null),
    );
    assert_eq!(removed.response.status, 200);
    let sent = broadcasts(&removed.effects);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].providers, ["fixture-bcast"]);
    assert_eq!(sent[0].reason, "removed");
    // The change hook ran once per successful mutation, never for a refusal or a
    // validate-only call.
    assert_eq!(
        *changed.lock().unwrap(),
        [
            ("fixture-bcast".to_owned(), "created"),
            ("fixture-bcast".to_owned(), "removed")
        ]
    );
}
