use super::{auth::*, http::*};
use crate::config::Config;

fn request() -> Request {
    Request {
        method: "POST".into(),
        path: "/api/test".into(),
        host: "127.0.0.1:48888".into(),
        remote_addr: "127.0.0.1:12500".into(),
        query: "token=synthetic".into(),
        ..Default::default()
    }
}
fn config() -> Config {
    Config {
        token: "synthetic".into(),
        ..Default::default()
    }
}

#[test]
fn service_guard_precedence_is_token_method_host_origin() {
    let cfg = config();
    let mut r = request();
    r.query.clear();
    r.host = "evil.invalid".into();
    r.method = "DELETE".into();
    r.headers
        .push(("Origin".into(), "https://evil.invalid".into()));
    assert_eq!(
        guard_base(&r, &cfg, 48888, &["POST"]).unwrap_err().status,
        401
    );
    r.query = "token=synthetic".into();
    assert_eq!(
        guard_base(&r, &cfg, 48888, &["POST"]).unwrap_err().status,
        405
    );
    r.method = "POST".into();
    assert!(
        String::from_utf8(guard_base(&r, &cfg, 48888, &["POST"]).unwrap_err().body)
            .unwrap()
            .contains("host not allowed")
    );
    r.host = "localhost:48888".into();
    assert!(
        String::from_utf8(guard_base(&r, &cfg, 48888, &["POST"]).unwrap_err().body)
            .unwrap()
            .contains("origin not allowed")
    );
    r.headers.clear();
    assert!(guard_base(&r, &cfg, 48888, &["POST"]).is_ok());
}
#[test]
fn service_host_portless_and_ipv6_compatibility() {
    for host in [
        "localhost",
        "LOCALHOST.",
        "127.0.0.1",
        "[::1]",
        "::1",
        "localhost:",
        "localhost:48888",
        "[::1]:48888",
    ] {
        assert!(allowed_host(host, 48888, &[]), "{host}");
    }
    for host in [
        "",
        "localhost:80",
        "127.0.0.2",
        "evil.invalid",
        "localhost:foo",
        "localhost:48888:0",
        "localhost..",
    ] {
        assert!(!allowed_host(host, 48888, &[]), "{host}");
    }
    assert!(allowed_host("", 0, &[]));
    assert!(allowed_host(
        "EXAMPLE.test.",
        48888,
        &["example.test".into()]
    ));
    assert!(!allowed_host(
        "example.test:443",
        48888,
        &["example.test".into()]
    ));
}
#[test]
fn service_origin_http_default_port_and_configured_https() {
    assert!(!allowed_origin("http://localhost", 48888, &[]));
    assert!(allowed_origin("http://localhost", 80, &[]));
    assert!(allowed_origin("http://[::1]:48888", 48888, &[]));
    assert!(!allowed_origin("https://localhost", 48888, &[]));
    assert!(allowed_origin(
        "https://example.test:8443",
        48888,
        &["example.test".into()]
    ));
    assert!(!allowed_origin(
        "https://example.test.evil",
        48888,
        &["example.test".into()]
    ));
    assert!(!allowed_origin("null", 48888, &[]));
    assert!(!allowed_origin("http://127.1:48888", 48888, &[]));
    assert!(!allowed_origin("http://0x7f000001:48888", 48888, &[]));
}
#[test]
fn service_token_precedence_and_bearer_case_are_go_compatible() {
    let mut r = request();
    r.headers = vec![
        ("Authorization".into(), "Bearer header".into()),
        ("Cookie".into(), format!("{TOKEN_COOKIE}=cookie")),
    ];
    assert_eq!(request_token(&r), "synthetic");
    r.query = "token=&token=second".into();
    assert_eq!(request_token(&r), "header");
    r.headers[0].1 = "bearer ignored".into();
    assert_eq!(request_token(&r), "cookie");
    r.headers[0].1 = "Bearer   ".into();
    assert_eq!(request_token(&r), "cookie"); // TrimSpace removes the required trailing prefix space.
    r.headers[0].1 = "Bearer \t x ".into();
    assert_eq!(request_token(&r), "x");
    assert!(!valid_token("", ""));
    assert!(!valid_token("abc", "abd"));
    assert!(valid_token("abc", "abc"));
}
#[test]
fn service_loopback_bypass_never_covers_logical_remote() {
    let mut cfg = config();
    cfg.hub.allow_loopback_without_token = true;
    let mut r = request();
    r.query.clear();
    assert!(token_or_trusted(&r, &cfg));
    r.headers
        .push(("X-Forwarded-For".into(), "203.0.113.10".into()));
    assert!(logically_remote(&r));
    assert!(!token_or_trusted(&r, &cfg));
    r.headers.clear();
    r.host = "machine.example".into();
    assert!(!token_or_trusted(&r, &cfg));
    cfg.hub.trusted_networks = vec!["127.0.0.1/32".into()];
    assert!(token_or_trusted(&r, &cfg));
    cfg.hub.allow_loopback_without_token = false;
    assert!(!token_or_trusted(&r, &cfg));
}
#[test]
fn service_remote_address_mapped_ipv6_and_https_proxy() {
    assert!(is_loopback("[::ffff:127.0.0.1]:100"));
    assert!(is_loopback("[::1%lo]:100"));
    let mut r = request();
    r.headers
        .push(("X-Forwarded-Proto".into(), " HTTPS ".into()));
    assert!(uses_https(&r));
    r.remote_addr = "203.0.113.3:4000".into();
    assert!(!uses_https(&r));
    r.tls = true;
    assert!(uses_https(&r));
}
#[test]
fn service_safe_methods_skip_origin_but_not_host() {
    let cfg = config();
    let mut r = request();
    r.headers
        .push(("Sec-Fetch-Site".into(), "cross-site".into()));
    for method in ["GET", "HEAD", "OPTIONS"] {
        r.method = method.into();
        assert!(guard_base(&r, &cfg, 48888, &[]).is_ok());
    }
    r.method = "POST".into();
    assert_eq!(guard_base(&r, &cfg, 48888, &[]).unwrap_err().status, 403);
    r.headers
        .push(("Origin".into(), "http://localhost:48888".into()));
    assert!(guard_base(&r, &cfg, 48888, &[]).is_ok());
}
#[test]
fn service_websocket_requires_origin_for_proxied_clients() {
    let mut cfg = config();
    cfg.hub.allowed_hosts = vec!["machine.example".into()];
    let mut r = request();
    assert!(websocket_origin(&r, &cfg, 48888).is_ok());
    r.host = "machine.example".into();
    assert!(websocket_origin(&r, &cfg, 48888).is_err());
    r.headers
        .push(("Origin".into(), "https://machine.example".into()));
    assert!(websocket_origin(&r, &cfg, 48888).is_ok());
}
#[test]
fn service_route_coverage_does_not_hide_unresolved_entries() {
    let inventory: serde_json::Value =
        serde_json::from_str(include_str!("../../inventory/services.json")).unwrap();
    let coverage: serde_json::Value =
        serde_json::from_str(include_str!("route_coverage.json")).unwrap();
    let mut source: Vec<_> = inventory["routes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["path"].as_str().unwrap())
        .collect();
    let mut actual: Vec<_> = coverage["routes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["path"].as_str().unwrap())
        .collect();
    source.sort();
    actual.sort();
    assert_eq!(source.len(), 155);
    assert_eq!(actual, source);
    for route in coverage["routes"].as_array().unwrap() {
        assert!(matches!(
            route["status"].as_str(),
            Some("unresolved" | "partial" | "implemented")
        ));
        if route["status"] == "implemented" {
            assert!(route["implementation"].is_string());
            assert!(route["fixture"].is_string());
        }
    }
}

fn router() -> (tempfile::TempDir, super::ServiceRouter) {
    let dir = tempfile::tempdir().unwrap();
    let trial = dir.path().join("trial");
    std::fs::create_dir(&trial).unwrap();
    let paths =
        crate::config::RuntimePaths::trial(&trial, 48888, &dir.path().join("installed")).unwrap();
    let store =
        std::sync::Arc::new(crate::config::ConfigStore::new(paths.clone(), config()).unwrap());
    (dir, super::ServiceRouter::isolated(store, paths))
}
fn json_response(response: &Response) -> serde_json::Value {
    serde_json::from_slice(&response.body).unwrap()
}
#[test]
fn service_settings_first_value_case_fold_null_unknown_and_reload() {
    let (_dir, router) = router();
    let mut r = request();
    r.path = "/api/terminal-color".into();
    r.body = br#"{"TERMINAL_COLOR":"OFF","terminal_color":null,"unknown":{"x":1}} trailing bytes"#
        .to_vec();
    let result = router.handle(&r, 1000).response;
    assert_eq!(result.status, 200);
    assert_eq!(json_response(&result)["terminal_color"], "off");
    let bytes = std::fs::read(router.paths.resource(crate::config::Resource::Config)).unwrap();
    let loaded =
        crate::config::Config::from_yaml(std::str::from_utf8(&bytes).unwrap(), &router.paths)
            .unwrap();
    assert_eq!(loaded.hub.terminal_color, "off");
    r.path = "/api/input-config".into();
    r.body = br#"{"deferred_enter_ms":"bad","deferred_enter_ms":1}"#.to_vec();
    assert_eq!(router.handle(&r, 1000).response.status, 400);
}
#[test]
fn service_settings_clamps_preserves_owned_flags_and_rejects_invalid_values() {
    let (_dir, router) = router();
    let mut snapshot = router.config.snapshot().unwrap();
    snapshot.config.log.legacy_logs_notice_shown = true;
    router
        .config
        .persist(snapshot.revision, snapshot.config)
        .unwrap();
    let mut r = request();
    r.path = "/api/log-config".into();
    r.body=br#"{"max_size_mb":0,"max_backups":900,"session_retention_days":-10,"attachment_max_total_mb":200001}"#.to_vec();
    assert_eq!(router.handle(&r, 1000).response.status, 200);
    let cfg = router.config.snapshot().unwrap().config;
    assert!(cfg.log.legacy_logs_notice_shown);
    assert_eq!(
        (
            cfg.log.max_size_mb,
            cfg.log.max_backups,
            cfg.log.session_retention_days,
            cfg.log.attachment_max_total_mb
        ),
        (1, 100, 0, 100000)
    );
    r.path = "/api/input-config".into();
    r.body = br#"{"deferred_enter_ms":10001}"#.to_vec();
    assert_eq!(router.handle(&r, 1000).response.status, 400);
    assert_eq!(
        router
            .config
            .snapshot()
            .unwrap()
            .config
            .input
            .deferred_enter_ms,
        0
    );
}
#[test]
fn service_settings_failed_save_publishes_all_six_callers_without_changing_files() {
    let cases = [
        (
            "/api/log-config",
            r#"{"enabled":true,"session_enabled":true,"max_size_mb":9,"max_backups":2}"#,
            serde_json::json!([true, true, 9, 2]),
        ),
        (
            "/api/terminal-color",
            r#"{"terminal_color":"off"}"#,
            serde_json::json!("off"),
        ),
        (
            "/api/handoff-intent-mode",
            r#"{"intent_mode":"turn-summary"}"#,
            serde_json::json!("turn-summary"),
        ),
        (
            "/api/reconnect-grace",
            r#"{"wrapper_reconnect_grace_sec":123}"#,
            serde_json::json!(123),
        ),
        (
            "/api/input-config",
            r#"{"deferred_enter_ms":321}"#,
            serde_json::json!(321),
        ),
        (
            "/api/orchestration-config",
            r#"{"board_notify_mode":"interrupt","spawn_confirm_mode":"providers","spawn_confirm_providers":["codex"],"child_timeout_seconds":123,"timeout_respawn":true,"max_children_per_parent":7}"#,
            serde_json::json!(["interrupt", "providers", ["codex"], 123, true, 7]),
        ),
    ];
    for (path, body, expected) in cases {
        let (_dir, router) = router();
        let destination = router.paths.resource(crate::config::Resource::Config);
        // An existing nonempty directory deterministically defeats replacement
        // even for a privileged test UID. Its contents must remain unchanged.
        std::fs::create_dir(&destination).unwrap();
        let retained = destination.join("retained.yaml");
        let prior = router.config.snapshot().unwrap();
        let prior_bytes = prior.config.to_private_yaml().unwrap().into_bytes();
        std::fs::write(&retained, &prior_bytes).unwrap();
        let mut request = request();
        request.path = path.into();
        request.body = body.as_bytes().into();
        let response = router.handle(&request, 1000).response;
        assert_eq!(response.status, 500, "{path}");
        assert_eq!(json_response(&response)["error"], "save_failed", "{path}");
        let after = router.config.snapshot().unwrap();
        assert_eq!(after.revision, prior.revision + 1, "{path}");
        let cfg = after.config;
        let observed = match path {
            "/api/log-config" => serde_json::json!([
                cfg.log.enabled,
                cfg.log.session_enabled,
                cfg.log.max_size_mb,
                cfg.log.max_backups
            ]),
            "/api/terminal-color" => serde_json::json!(cfg.hub.terminal_color),
            "/api/handoff-intent-mode" => serde_json::json!(cfg.handoff.intent_mode),
            "/api/reconnect-grace" => serde_json::json!(cfg.hub.wrapper_reconnect_grace_sec),
            "/api/input-config" => serde_json::json!(cfg.input.deferred_enter_ms),
            "/api/orchestration-config" => serde_json::json!([
                cfg.orchestration.board_notify_mode,
                cfg.orchestration.spawn_confirm_mode,
                cfg.orchestration.spawn_confirm_providers,
                cfg.orchestration.child_timeout_seconds,
                cfg.orchestration.timeout_respawn,
                cfg.orchestration.max_children_per_parent
            ]),
            _ => unreachable!(),
        };
        assert_eq!(observed, expected, "{path}");
        assert_eq!(std::fs::read(&retained).unwrap(), prior_bytes, "{path}");
        assert_eq!(
            std::fs::read_dir(&destination).unwrap().count(),
            1,
            "{path}"
        );
        assert_eq!(
            std::fs::read_dir(router.paths.root()).unwrap().count(),
            1,
            "temporary files must be cleaned on {path}"
        );
    }
}
#[test]
fn service_orchestration_empty_and_null_provider_inputs_both_project_null() {
    let (_dir, router) = router();
    let mut request = request();
    request.path = "/api/orchestration-config".into();
    request.method = "GET".into();
    let response = router.handle(&request, 1000).response;
    assert_eq!(response.status, 200);
    assert_eq!(
        json_response(&response)["spawn_confirm_providers"],
        serde_json::Value::Null
    );
    for providers in [
        serde_json::Value::Null,
        serde_json::json!([]),
        serde_json::json!(["codex"]),
    ] {
        request.method = "POST".into();
        request.body = serde_json::to_vec(&serde_json::json!({"board_notify_mode":"soft-notify","spawn_confirm_mode":"providers","spawn_confirm_providers":providers,"child_timeout_seconds":60})).unwrap();
        assert_eq!(router.handle(&request, 1000).response.status, 200);
        request.method = "GET".into();
        let response = router.handle(&request, 1000).response;
        let expected = if providers.as_array().is_some_and(|items| !items.is_empty()) {
            providers
        } else {
            serde_json::Value::Null
        };
        assert_eq!(
            json_response(&response)["spawn_confirm_providers"],
            expected
        );
    }
}
#[test]
fn service_settings_body_limit_applies_to_first_value_only() {
    let (_dir, router) = router();
    let mut r = request();
    r.path = "/api/terminal-color".into();
    r.body = br#"{"terminal_color":"off"}"#.to_vec();
    r.body.extend(vec![b'x'; JSON_BODY_LIMIT + 100]);
    assert_eq!(router.handle(&r, 1000).response.status, 200);
    r.body = br#"{"unknown":""#.to_vec();
    r.body.extend(vec![b'x'; JSON_BODY_LIMIT]);
    r.body.extend_from_slice(br#""}"#);
    assert_eq!(router.handle(&r, 1000).response.status, 400);
}
#[test]
fn service_pin_sessions_bound_to_nonce_device_and_expiry() {
    let (_dir, router) = router();
    let mut snapshot = router.config.snapshot().unwrap();
    snapshot.config.remote_pin_hash = bcrypt::hash("123456", 4).unwrap();
    snapshot.config.auth_cookie_secret = "synthetic-secret".into();
    router
        .config
        .persist(snapshot.revision, snapshot.config)
        .unwrap();
    let mut r = request();
    r.remote_addr = "203.0.113.1:1234".into();
    r.path = "/api/auth/login".into();
    r.body = br#"{"pin":"123456"}"#.to_vec();
    let result = router.handle(&r, 1000).response;
    assert_eq!(result.status, 200);
    let cookie = result.cookies[0].split(';').next().unwrap().to_string();
    r.headers.push(("Cookie".into(), cookie));
    let cfg = router.config.snapshot().unwrap().config;
    assert!(router.auth.has_valid_pin(&r, &cfg, 1000));
    assert!(!router.auth.has_valid_pin(&r, &cfg, 1000 + 43200));
    r.remote_addr = "203.0.113.2:1234".into();
    assert!(!router.auth.has_valid_pin(&r, &cfg, 1000));
    r.remote_addr = "203.0.113.1:1234".into();
    r.path = "/api/auth/logout".into();
    assert_eq!(router.handle(&r, 1000).response.status, 200);
    assert!(!router.auth.has_valid_pin(&r, &cfg, 1000));
}
#[test]
fn service_pin_lockout_and_global_proxy_limit() {
    let mut limiter = super::pin::Limiter::default();
    for _ in 0..5 {
        assert_eq!(limiter.begin("ip", 1000), 0);
    }
    assert_eq!(limiter.retry_after("ip", 1000), 61);
    assert_eq!(limiter.begin("ip", 1000), 61);
    assert_eq!(limiter.begin("ip", 1060), 0);
    limiter.success("ip");
    assert_eq!(limiter.retry_after("ip", 1060), 0);
    let mut limiter = super::pin::Limiter::default();
    for _ in 0..30 {
        assert_eq!(limiter.begin("", 1000), 0);
    }
    assert_eq!(limiter.retry_after("", 1000), 61);
}
#[test]
fn service_remote_pin_bootstrap_and_gate_do_not_restrict_loopback() {
    let (_dir, router) = router();
    let mut r = request();
    r.path = "/api/auth/set-pin".into();
    r.body = br#"{"pin":"123456"}"#.to_vec();
    r.remote_addr = "203.0.113.1:1234".into();
    assert_eq!(router.handle(&r, 1000).response.status, 403);
    r.remote_addr = "127.0.0.1:1234".into();
    assert_eq!(router.handle(&r, 1000).response.status, 200);
    r.path = "/api/input-config".into();
    r.method = "GET".into();
    assert_eq!(router.handle(&r, 1000).response.status, 200);
    r.remote_addr = "203.0.113.1:1234".into();
    assert_eq!(
        json_response(&router.handle(&r, 1000).response)["error"],
        "pin_required"
    );
    r.path = "/api/auth/status".into();
    let value = json_response(&router.handle(&r, 1000).response);
    assert_eq!(value["authed"], false);
    assert_eq!(value["pin_enabled"], true);
}
#[test]
fn service_reauth_page_cookie_and_public_assets_policy() {
    let (_dir, router) = router();
    let mut r = request();
    r.path = "/".into();
    r.method = "GET".into();
    r.query.clear();
    r.headers.push(("Accept".into(), "text/html".into()));
    let response = router.handle(&r, 1000).response;
    assert_eq!(response.status, 401);
    assert!(response.cookies.is_empty());
    let html = String::from_utf8(response.body).unwrap();
    assert!(html.contains("many-ai-cli-token"));
    assert!(!html.contains("__NONCE__"));
    assert!(response.headers["Content-Security-Policy"].contains("nonce-"));
    r.path = "/app.js".into();
    r.host = "evil.invalid".into();
    let response = router.handle(&r, 1000).response;
    assert_eq!(response.status, 200);
    assert!(response.headers["Content-Type"].contains("javascript"));
    r.path = "/api/notify-generate-topic".into();
    assert_eq!(router.handle(&r, 1000).response.status, 401);
}
#[test]
fn service_unimplemented_operations_never_report_success() {
    let (_dir, router) = router();
    let mut r = request();
    r.path = "/api/routines".into();
    r.method = "GET".into();
    assert_eq!(router.handle(&r, 1000).response.status, 501);
}

#[test]
fn service_pin_login_ignores_set_pin_fields_as_unknown() {
    let mut cfg = config();
    cfg.remote_pin_hash = bcrypt::hash("123456", 4).unwrap();
    cfg.auth_cookie_secret = "synthetic-secret".into();
    let auth = super::pin::AuthService::default();
    let mut r = request();
    r.body = br#"{"pin":"123456","clear":"unknown-to-login"}"#.to_vec();
    assert_eq!(auth.login(&r, &cfg, 1000).status, 200);
}

#[test]
fn service_query_ignores_go_invalid_pairs_before_cookie_fallback() {
    let mut r = request();
    r.headers
        .push(("Cookie".into(), format!("{TOKEN_COOKIE}=synthetic")));
    for query in [
        "token=bad;pair",
        "token=%ZZ",
        "ignored=bad;pair&token=synthetic",
    ] {
        r.query = query.into();
        assert_eq!(request_token(&r), "synthetic", "{query}");
    }
}

#[test]
fn service_production_uses_actual_bound_port_for_host_origin_and_rotation_url() {
    for (configured, bound) in [(48888, 48888), (48888, 48889)] {
        let dir = tempfile::tempdir().unwrap();
        let paths = crate::config::RuntimePaths::production(dir.path()).unwrap();
        assert_eq!(paths.port(), 47777);
        let mut cfg = config();
        cfg.hub.port = configured;
        let store =
            std::sync::Arc::new(crate::config::ConfigStore::new(paths.clone(), cfg).unwrap());
        let router = super::ServiceRouter::isolated_at_port(store, paths, bound).unwrap();
        let mut r = request();
        r.path = "/api/input-config".into();
        r.method = "GET".into();
        r.host = format!("127.0.0.1:{bound}");
        assert_eq!(router.handle(&r, 1000).response.status, 200);
        r.host = "127.0.0.1:47777".into();
        assert_eq!(router.handle(&r, 1000).response.status, 403);
        r.host = format!("127.0.0.1:{bound}");
        r.method = "POST".into();
        r.body = br#"{"deferred_enter_ms":0}"#.to_vec();
        r.headers
            .push(("Origin".into(), format!("http://127.0.0.1:{bound}")));
        assert_eq!(router.handle(&r, 1000).response.status, 200);
        r.headers[0].1 = "http://127.0.0.1:47777".into();
        assert_eq!(router.handle(&r, 1000).response.status, 403);
        r.headers.clear();
        let rotated = router
            .auth
            .rotate(&r, &router.config, i64::from(router.bound_port()), 1000)
            .unwrap();
        assert!(
            json_response(&rotated)["hub_url"]
                .as_str()
                .unwrap()
                .starts_with(&format!("http://127.0.0.1:{bound}/?token="))
        );
    }
}
#[test]
fn service_trial_bound_port_must_preserve_explicit_isolation() {
    let (_dir, router) = router();
    assert!(
        super::ServiceRouter::isolated_at_port(
            router.config.clone(),
            router.paths.clone(),
            router.paths.port() + 1
        )
        .is_err()
    );
    assert!(
        super::ServiceRouter::isolated_at_port(router.config.clone(), router.paths.clone(), 0)
            .is_err()
    );
}

#[test]
fn service_pin_fractional_lockout_does_not_unlock_or_round_early() {
    use std::time::{Duration, UNIX_EPOCH};
    let start = UNIX_EPOCH + Duration::new(1000, 900_000_000);
    let mut limiter = super::pin::Limiter::default();
    for _ in 0..5 {
        assert_eq!(limiter.begin_at("ip", start), 0);
    }
    assert_eq!(limiter.retry_after_at("ip", start), 61);
    assert_eq!(
        limiter.retry_after_at("ip", start + Duration::from_millis(100)),
        60
    );
    assert_eq!(
        limiter.begin_at("ip", start + Duration::from_millis(59_999)),
        1
    );
    assert_eq!(limiter.begin_at("ip", start + Duration::from_secs(60)), 0);
    let auth = super::pin::AuthService::default();
    let mut cfg = config();
    cfg.remote_pin_hash = bcrypt::hash("123456", 4).unwrap();
    cfg.auth_cookie_secret = "synthetic-secret".into();
    let mut r = request();
    r.remote_addr = "203.0.113.1:5000".into();
    r.body = br#"{"pin":"654321"}"#.to_vec();
    for _ in 0..4 {
        assert_eq!(auth.login_at(&r, &cfg, start).status, 401);
    }
    assert_eq!(auth.login_at(&r, &cfg, start).status, 429);
    let status = json_response(&auth.status_at(&r, &cfg, start + Duration::from_millis(100)));
    assert_eq!(status["retry_after"], 60);
    assert_eq!(
        auth.login_at(&r, &cfg, start + Duration::from_millis(59_999))
            .status,
        429
    );
    assert_eq!(
        auth.login_at(&r, &cfg, start + Duration::from_secs(60))
            .status,
        401
    );
}
