use super::*;
use crate::{
    config::ConfigStore,
    hub::{ServiceRouter, transport},
    process::Cancellation,
};

#[tokio::test]
async fn child_credential_is_local_and_bound_to_its_live_session_operations() {
    use crate::hub::http::Request;
    let mut f = fixture_at_port(true, 49327);
    f.config.token = "synthetic-hub-key".into();
    f.config.remote_pin_hash = "synthetic-configured-pin".into();
    f.config.hub.allow_loopback_without_token = true;
    let binding = f.core.details(register(&f).await).unwrap().binding;
    let router = ServiceRouter::new(
        Arc::new(ConfigStore::new(f.paths.clone(), f.config.clone()).unwrap()),
        f.paths.clone(),
        f.core.clone(),
        49327,
    )
    .unwrap();
    let child = router.child_tokens.issue(&f.config.token, binding).unwrap();
    let request = |method: &str, path: &str| Request {
        method: method.into(),
        path: path.into(),
        host: "127.0.0.1:49327".into(),
        remote_addr: "127.0.0.1:49328".into(),
        headers: vec![("Authorization".into(), format!("Bearer {child}"))],
        ..Default::default()
    };
    let parent = format!("/api/sessions/{}", binding.session.0);
    let usage = request("POST", crate::hub::usage_routes::PATH);
    assert!(router.preflight(&usage, 1).is_ok());
    let mut lower_scheme = request("GET", "/api/session-history");
    lower_scheme.headers[0].1 = format!("bearer {child}");
    assert_eq!(router.preflight(&lower_scheme, 1).unwrap_err().status, 401);
    lower_scheme
        .headers
        .insert(0, ("Authorization".into(), "Bearer other".into()));
    assert_eq!(router.preflight(&lower_scheme, 1).unwrap_err().status, 401);
    lower_scheme.headers.clear();
    lower_scheme.query = format!("token=&token={child}");
    assert_eq!(router.preflight(&lower_scheme, 1).unwrap_err().status, 401);
    for (method, action) in [
        ("POST", "spawn-child"),
        ("POST", "send-child"),
        ("POST", "close-child"),
        ("GET", "relay"),
        ("POST", "relay"),
        ("POST", "relay-stop"),
    ] {
        assert!(
            router
                .preflight(&request(method, &format!("{parent}/{action}")), 1)
                .is_ok()
        );
    }
    for (method, path) in [
        ("GET", "/api/session-history".to_owned()),
        ("POST", "/api/auth/launch-code".to_owned()),
        ("POST", format!("{parent}/spawn-confirm")),
        ("POST", format!("{parent}/inject")),
        ("POST", format!("{parent}/relay-resume")),
        (
            "POST",
            format!("/api/sessions/{}/send-child", binding.session.0 + 1),
        ),
    ] {
        assert_eq!(
            router
                .preflight(&request(method, &path), 1)
                .unwrap_err()
                .status,
            401
        );
    }
    let mut remote = usage.clone();
    remote
        .headers
        .push(("X-Forwarded-For".into(), "192.0.2.9".into()));
    assert_eq!(router.preflight(&remote, 1).unwrap_err().status, 401);
    let mut wrong_origin = usage.clone();
    wrong_origin
        .headers
        .push(("Origin".into(), "https://example.invalid".into()));
    assert_eq!(router.preflight(&wrong_origin, 1).unwrap_err().status, 403);
    let mut query = usage.clone();
    query.headers.clear();
    query.query = format!("token={child}");
    assert_eq!(router.preflight(&query, 1).unwrap_err().status, 401);
    let mut bad = usage.clone();
    bad.headers[0].1.push('x');
    assert_eq!(router.preflight(&bad, 1).unwrap_err().status, 401);
    assert_eq!(
        router
            .preflight(&request("GET", crate::hub::usage_routes::PATH), 1)
            .unwrap_err()
            .status,
        405
    );
    router.child_tokens.disconnect(binding);
    assert_eq!(router.preflight(&usage, 1).unwrap_err().status, 401);
}
struct CancelOnDrop(Cancellation);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
#[tokio::test]
async fn unconfirmed_empty_app_exit_rejects_a_new_session_before_any_stop_effect() {
    for connected in [false, true] {
        let mut f = fixture_at_port(true, 49327);
        f.config.token = "synthetic-exit-token".into();
        let id = if connected {
            Some(register(&f).await)
        } else {
            None
        };
        let tasks = HubTaskOwner::new(tokio::runtime::Handle::current());
        let config = Arc::new(ConfigStore::new(f.paths.clone(), f.config.clone()).unwrap());
        let router = ServiceRouter::new(config, f.paths.clone(), f.core.clone(), 49327)
            .unwrap()
            .with_task_owner(tasks.handle())
            .with_session_routes(
                f.http.clone(),
                Arc::new(|http: &SessionHttp, config: &Config| {
                    Ok(http.handle_info_authenticated(config, &info_context()))
                }),
                // The fixture panics if a CancelSession effect reaches it.
                f.sink.clone(),
            );
        let response = router
            .handle_owned_async_at(
                &crate::hub::http::Request {
                    method: "POST".into(),
                    path: "/api/kill-all".into(),
                    query: "token=synthetic-exit-token".into(),
                    host: "127.0.0.1:49327".into(),
                    remote_addr: "127.0.0.1:49328".into(),
                    headers: vec![("x-many-ai-cli-require-empty".into(), "1".into())],
                    ..Default::default()
                },
                crate::proto::time::UNIX_EPOCH,
                &crate::proto::core::TaskCancellation::default(),
                &crate::proto::core::HttpWaitCancellation::default(),
            )
            .await;
        assert_eq!(response.response.status, if connected { 409 } else { 200 });
        let shutdown = router
            .handle_owned_async_at(
                &crate::hub::http::Request {
                    method: "POST".into(),
                    path: "/api/shutdown".into(),
                    query: "token=synthetic-exit-token".into(),
                    host: "127.0.0.1:49327".into(),
                    remote_addr: "127.0.0.1:49328".into(),
                    headers: vec![("x-many-ai-cli-require-empty".into(), "1".into())],
                    ..Default::default()
                },
                crate::proto::time::UNIX_EPOCH,
                &crate::proto::core::TaskCancellation::default(),
                &crate::proto::core::HttpWaitCancellation::default(),
            )
            .await;
        // No ShutdownOwner: only the empty case can reach its unavailable response.
        assert_eq!(shutdown.response.status, if connected { 409 } else { 503 });
        if let Some(id) = id {
            assert!(f.core.details(id).unwrap().connected);
        }
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
    std::fs::create_dir_all(std::path::Path::new(&snapshot.log_path).parent().unwrap()).unwrap();
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
    let info = info.json::<Value>().await.unwrap();
    assert_eq!(info["active_sessions"], 1);
    assert_eq!(info["connected_sessions"], 1);

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
