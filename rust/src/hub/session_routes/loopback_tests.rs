use super::*;
use crate::{
    config::ConfigStore,
    hub::{ServiceRouter, transport},
    process::Cancellation,
};

struct CancelOnDrop(Cancellation);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
#[tokio::test]
async fn registered_http_session_routes_preserve_guards_and_real_store_effects() {
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut f = fixture_at_port(true, port);
    f.config.token = "synthetic-session-http-token".into();
    let id = register(&f).await;
    let snapshot = f.core.snapshot(id).unwrap();
    std::fs::write(&snapshot.log_path, b"ordinary loopback log").unwrap();
    message(&f, id.0, "loopback message");
    let owner = HubTaskOwner::new(tokio::runtime::Handle::current());
    let config = Arc::new(ConfigStore::new(f.paths.clone(), f.config.clone()).unwrap());
    let router = Arc::new(
        ServiceRouter::new(config, f.paths.clone(), f.core.clone(), port)
            .unwrap()
            .with_task_owner(owner.handle())
            .with_session_routes(
                f.http.clone(),
                Arc::new(|http: &SessionHttp, config: &Config| {
                    Ok(http.handle_info_authenticated(config, &info_context()))
                }),
                f.sink.clone(),
            ),
    );
    let cancel = CancelOnDrop(Cancellation::default());
    let server = tokio::spawn(transport::serve(
        listener,
        router,
        f.sink.clone(),
        cancel.0.clone(),
    ));
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let base = format!("http://127.0.0.1:{port}");
    let token = "token=synthetic-session-http-token";

    let unauthorized = client.get(format!("{base}/api/info")).send().await.unwrap();
    assert_eq!(unauthorized.status(), 401);
    assert_eq!(unauthorized.headers()["cache-control"], "no-store");
    let wrong_method = client
        .post(format!("{base}/api/info?{token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(wrong_method.status(), 405);
    assert_eq!(wrong_method.headers()["cache-control"], "no-store");
    let info = client
        .get(format!("{base}/api/info?{token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(info.status(), 200);
    assert_eq!(info.headers()["cache-control"], "no-store");
    assert_eq!(info.json::<Value>().await.unwrap()["active_sessions"], 1);

    // The prefix's token/path decisions precede its ordinary method/Host guard.
    let path = format!("{base}/api/session/invalid/unknown");
    assert_eq!(client.get(&path).send().await.unwrap().status(), 401);
    let malformed = client
        .get(format!("{path}?{token}"))
        .header("Host", "invalid.example")
        .send()
        .await
        .unwrap();
    assert_eq!(malformed.status(), 404);
    let bad_id = client
        .get(format!("{base}/api/session/invalid/meta?{token}"))
        .header("Host", "invalid.example")
        .send()
        .await
        .unwrap();
    assert_eq!(bad_id.status(), 400);
    assert_eq!(
        client
            .get(format!("{base}/api/session/{}/meta?{token}", id.0))
            .send()
            .await
            .unwrap()
            .status(),
        405
    );

    let patched = client
        .patch(format!("{base}/api/sessions/{}/meta?{token}", id.0))
        .header("Origin", &base)
        .json(&json!({"label":"HTTP rename","pinned":true}))
        .send()
        .await
        .unwrap();
    assert_eq!(patched.status(), 200);
    assert_eq!(
        patched.json::<Value>().await.unwrap()["session_meta"]["label"],
        "HTTP rename"
    );
    assert_eq!(
        f.store
            .as_ref()
            .unwrap()
            .session_card_meta_by_live_session(id)
            .unwrap()
            .label,
        "HTTP rename"
    );
    for (path, query, key) in [
        (
            "/api/session-chat",
            format!("session_id={}", id.0),
            "messages",
        ),
        ("/api/session-history", String::new(), "sessions"),
        ("/api/session-search", "q=loopback".into(), "results"),
        ("/api/approval-history", String::new(), "approvals"),
    ] {
        let response = client
            .get(format!("{base}{path}?{token}&{query}"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200, "{path}");
        assert!(
            response.json::<Value>().await.unwrap()[key].is_array(),
            "{path}"
        );
    }
    let log = client
        .get(format!(
            "{base}/api/session-log?{token}&session_id={}",
            id.0
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(log.status(), 200);
    assert_eq!(
        STANDARD
            .decode(
                log.json::<Value>().await.unwrap()["data_b64"]
                    .as_str()
                    .unwrap()
            )
            .unwrap(),
        b"ordinary loopback log"
    );
    cancel.0.cancel();
    server.await.unwrap().unwrap();
    owner.stop_requests();
    owner.drain_requests().await;
    owner.stop_effects().unwrap();
    owner.drain_effects().await;
}
