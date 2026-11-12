use super::{auth::*, http::*};
use crate::config::Config;
use crate::proto::time::UNIX_EPOCH;

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
    let local = "127.0.0.1:48888";
    assert!(!allowed_origin("http://localhost", 48888, &[], local));
    assert!(allowed_origin("http://localhost", 80, &[], local));
    assert!(allowed_origin("http://[::1]:48888", 48888, &[], local));
    assert!(!allowed_origin("https://localhost", 48888, &[], local));
    let hosts = ["example.test".to_owned()];
    // An https origin must be the authority the request addressed, port
    // included: another HTTPS service on the same name is a different origin
    // that still receives the Hub's cookies (U06).
    assert!(!allowed_origin(
        "https://example.test:8443",
        48888,
        &hosts,
        "example.test"
    ));
    assert!(allowed_origin(
        "https://example.test:8443",
        48888,
        &hosts,
        "example.test:8443"
    ));
    assert!(allowed_origin(
        "https://example.test",
        48888,
        &hosts,
        "Example.Test."
    ));
    assert!(allowed_origin(
        "https://example.test:443",
        48888,
        &hosts,
        "example.test"
    ));
    assert!(!allowed_origin(
        "https://example.test",
        48888,
        &hosts,
        "example.test:8443"
    ));
    assert!(!allowed_origin(
        "https://example.test",
        48888,
        &hosts,
        local
    ));
    assert!(!allowed_origin(
        "https://example.test.evil",
        48888,
        &hosts,
        "example.test.evil"
    ));
    assert!(!allowed_origin("null", 48888, &[], local));
    assert!(!allowed_origin("http://127.1:48888", 48888, &[], local));
    assert!(!allowed_origin(
        "http://0x7f000001:48888",
        48888,
        &[],
        local
    ));
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
    // A cookie no longer carries the token (U05).
    r.headers[0].1 = "bearer ignored".into();
    assert_eq!(request_token(&r), "");
    r.headers[0].1 = "Bearer   ".into();
    assert_eq!(request_token(&r), ""); // TrimSpace removes the required trailing prefix space.
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
    // A page served by another HTTPS port of the same machine name (U06).
    r.headers.last_mut().unwrap().1 = "https://machine.example:8443".into();
    assert_eq!(websocket_origin(&r, &cfg, 48888).unwrap_err().status, 403);
    // A loopback proxy's X-Forwarded-Host is the authority the browser used.
    r.host = "127.0.0.1:48888".into();
    r.headers
        .push(("X-Forwarded-Host".into(), "machine.example:8443".into()));
    assert!(websocket_origin(&r, &cfg, 48888).is_ok());
    r.headers.last_mut().unwrap().1 = "machine.example".into();
    assert_eq!(websocket_origin(&r, &cfg, 48888).unwrap_err().status, 403);
    // POST requests use the same origin check.
    r.method = "POST".into();
    assert_eq!(require_origin(&r, &cfg, 48888).unwrap_err().status, 403);
    r.headers.last_mut().unwrap().1 = "machine.example:8443".into();
    assert!(require_origin(&r, &cfg, 48888).is_ok());
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
#[derive(Default)]
struct SyntheticLaunchShellFs(std::collections::BTreeSet<String>);
impl SyntheticLaunchShellFs {
    fn put(&mut self, path: &str) {
        self.0.insert(path.to_ascii_lowercase());
    }
}
impl crate::process::execpath::ExecutableFs for SyntheticLaunchShellFs {
    fn exists(&self, path: &str) -> bool {
        self.0.contains(&path.to_ascii_lowercase())
    }
    fn is_executable(&self, path: &str, _: crate::process::execpath::Platform) -> bool {
        self.exists(path)
    }
    fn read(&self, path: &str) -> std::io::Result<Vec<u8>> {
        if self.exists(path) {
            Ok(Vec::new())
        } else {
            Err(std::io::Error::from(std::io::ErrorKind::NotFound))
        }
    }
}
#[test]
fn approval_pattern_item_guards_auth_host_and_origin_before_endpoint_methods() {
    let (_root, router) = router();
    for path in [
        "/api/approval-patterns/profile",
        "/api/approval-patterns/copy-official",
        "/api/approval-patterns/claude",
    ] {
        let mut r = request();
        r.path = path.into();
        r.method = "DELETE".into();
        r.query.clear();
        r.host = "evil.invalid".into();
        assert_eq!(router.preflight(&r, 1000).unwrap_err().status, 401);
        r.query = "token=synthetic".into();
        let denied = router.preflight(&r, 1000).unwrap_err();
        assert_eq!(denied.status, 403);
        assert!(
            String::from_utf8(denied.body)
                .unwrap()
                .contains("host not allowed")
        );
        r.host = "127.0.0.1:48888".into();
        r.headers
            .push(("Origin".into(), "https://evil.invalid".into()));
        assert_eq!(router.preflight(&r, 1000).unwrap_err().status, 403);
        r.headers.clear();
        assert!(router.preflight(&r, 1000).is_ok());
    }
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
fn ai_launch_shell_defaults_missing_values_and_roundtrips_explicit_invalid_values() {
    let (_dir, router) = router();
    let old = crate::config::Config::from_yaml("hub:\n  terminal_color: inherit\n", &router.paths)
        .unwrap();
    assert_eq!(old.hub.ai_launch_shell, "auto");
    let saved = old.to_private_yaml().unwrap();
    let reloaded = crate::config::Config::from_yaml(&saved, &router.paths).unwrap();
    assert_eq!(reloaded.hub.ai_launch_shell, "auto");

    let invalid = crate::config::Config::from_yaml(
        "hub:\n  ai_launch_shell: imaginary-shell\n",
        &router.paths,
    )
    .unwrap();
    assert_eq!(invalid.hub.ai_launch_shell, "imaginary-shell");
    let invalid_saved = invalid.to_private_yaml().unwrap();
    let invalid_reloaded = crate::config::Config::from_yaml(&invalid_saved, &router.paths).unwrap();
    assert_eq!(invalid_reloaded.hub.ai_launch_shell, "imaginary-shell");
}
#[test]
fn ai_launch_shell_api_saves_reloads_and_reports_current_availability() {
    let (_dir, router) = router();
    let mut initial = router.config.snapshot().unwrap();
    initial.config.hub.terminal_color = "off".into();
    initial.config.hub.idle_timeout_min = 23;
    router
        .config
        .persist(initial.revision, initial.config)
        .unwrap();

    let mut r = request();
    r.path = "/api/ai-launch-shell".into();
    r.body = br#"{"ai_launch_shell":"pwsh"}"#.to_vec();
    let response = router.handle(&r, 1000).response;
    assert_eq!(response.status, 200);
    assert_eq!(json_response(&response)["ai_launch_shell"], "pwsh");

    let saved = router.config.snapshot().unwrap();
    assert_eq!(saved.config.hub.ai_launch_shell, "pwsh");
    assert_eq!(saved.config.hub.terminal_color, "off");
    assert_eq!(saved.config.hub.idle_timeout_min, 23);
    let bytes = std::fs::read(router.paths.resource(crate::config::Resource::Config)).unwrap();
    let reloaded =
        crate::config::Config::from_yaml(std::str::from_utf8(&bytes).unwrap(), &router.paths)
            .unwrap();
    assert_eq!(reloaded.hub.ai_launch_shell, "pwsh");
    assert_eq!(reloaded.hub.terminal_color, "off");
    assert_eq!(reloaded.hub.idle_timeout_min, 23);

    r.method = "GET".into();
    r.body.clear();
    let response = router.handle(&r, 1000).response;
    assert_eq!(response.status, 200);
    let body = json_response(&response);
    assert_eq!(body["ai_launch_shell"], "pwsh");
    for mode in ["auto", "pwsh", "cmd"] {
        assert!(body["availability"][mode]["available"].is_boolean());
    }
    assert!(body["selected_availability"]["available"].is_boolean());
}
#[test]
fn ai_launch_shell_api_auth_and_invalid_value_leave_state_and_file_unchanged() {
    let (_dir, router) = router();
    let initial = router.config.snapshot().unwrap();
    router
        .config
        .persist(initial.revision, initial.config)
        .unwrap();
    let before = router.config.snapshot().unwrap();
    let destination = router.paths.resource(crate::config::Resource::Config);
    let before_bytes = std::fs::read(&destination).unwrap();

    for method in ["GET", "POST"] {
        let mut unauthenticated = request();
        unauthenticated.path = "/api/ai-launch-shell".into();
        unauthenticated.method = method.into();
        unauthenticated.query.clear();
        unauthenticated.body = br#"{"ai_launch_shell":"pwsh"}"#.to_vec();
        assert_eq!(router.handle(&unauthenticated, 1000).response.status, 401);
    }

    let mut invalid = request();
    invalid.path = "/api/ai-launch-shell".into();
    invalid.body = br#"{"ai_launch_shell":"powershell"}"#.to_vec();
    let response = router.handle(&invalid, 1000).response;
    assert_eq!(response.status, 400);
    assert_eq!(json_response(&response)["error"], "invalid_ai_launch_shell");
    let after = router.config.snapshot().unwrap();
    assert_eq!(after.revision, before.revision);
    assert_eq!(
        after.config.hub.ai_launch_shell,
        before.config.hub.ai_launch_shell
    );
    assert_eq!(std::fs::read(destination).unwrap(), before_bytes);
}
#[test]
fn protected_route_guard_failures_preserve_config_before_a_live_success() {
    let (_dir, router) = router();
    let initial = router.config.snapshot().unwrap();
    router
        .config
        .persist(initial.revision, initial.config)
        .unwrap();
    let before = router.config.snapshot().unwrap();
    let destination = router.paths.resource(crate::config::Resource::Config);
    let before_bytes = std::fs::read(&destination).unwrap();

    // Exercise the composed router and real temporary persistence destination,
    // rather than interpreting guard return values as proof of no mutation.
    for (label, token, method, host, origin, expected) in [
        (
            "query-over-bearer",
            "wrong",
            "POST",
            "localhost:48888",
            "http://localhost:48888",
            401,
        ),
        (
            "method-before-host",
            "synthetic",
            "DELETE",
            "foreign.invalid",
            "https://foreign.invalid",
            405,
        ),
        (
            "host-before-origin",
            "synthetic",
            "POST",
            "foreign.invalid",
            "https://foreign.invalid",
            403,
        ),
        (
            "origin-before-owner",
            "synthetic",
            "POST",
            "localhost:48888",
            "https://foreign.invalid",
            403,
        ),
    ] {
        let mut r = request();
        r.path = "/api/ai-launch-shell".into();
        r.query = format!("token={token}");
        r.method = method.into();
        r.host = host.into();
        r.headers = vec![
            ("Authorization".into(), "Bearer synthetic".into()),
            ("Cookie".into(), format!("{TOKEN_COOKIE}=synthetic")),
            ("Origin".into(), origin.into()),
        ];
        r.body = br#"{"ai_launch_shell":"pwsh"}"#.to_vec();
        assert_eq!(router.handle(&r, 1000).response.status, expected, "{label}");
        let after = router.config.snapshot().unwrap();
        assert_eq!(after.revision, before.revision, "{label}");
        assert_eq!(
            after.config.hub.ai_launch_shell, before.config.hub.ai_launch_shell,
            "{label}"
        );
        assert_eq!(
            std::fs::read(&destination).unwrap(),
            before_bytes,
            "{label}"
        );
    }

    let mut accepted = request();
    accepted.path = "/api/ai-launch-shell".into();
    accepted.body = br#"{"ai_launch_shell":"pwsh"}"#.to_vec();
    assert_eq!(router.handle(&accepted, 1000).response.status, 200);
    let after = router.config.snapshot().unwrap();
    assert_eq!(after.config.hub.ai_launch_shell, "pwsh");
    assert_ne!(after.revision, before.revision);
    assert_ne!(std::fs::read(destination).unwrap(), before_bytes);
}

#[test]
fn protected_safe_get_skips_origin_but_keeps_host_guard_without_mutation() {
    let (_dir, router) = router();
    let initial = router.config.snapshot().unwrap();
    router
        .config
        .persist(initial.revision, initial.config)
        .unwrap();
    let before = router.config.snapshot().unwrap();
    let destination = router.paths.resource(crate::config::Resource::Config);
    let before_bytes = std::fs::read(&destination).unwrap();
    let mut r = request();
    r.path = "/api/input-config".into();
    r.method = "GET".into();
    r.headers = vec![
        ("Origin".into(), "https://foreign.invalid".into()),
        ("Sec-Fetch-Site".into(), "cross-site".into()),
    ];
    assert_eq!(router.handle(&r, 1000).response.status, 200);
    r.host = "foreign.invalid:48888".into();
    assert_eq!(router.handle(&r, 1000).response.status, 403);
    let after = router.config.snapshot().unwrap();
    assert_eq!(after.revision, before.revision);
    assert_eq!(std::fs::read(destination).unwrap(), before_bytes);
}

#[test]
fn ai_launch_shell_api_failed_save_does_not_publish_memory_or_change_existing_files() {
    let (_dir, router) = router();
    let destination = router.paths.resource(crate::config::Resource::Config);
    std::fs::create_dir(&destination).unwrap();
    let retained = destination.join("retained.yaml");
    let before = router.config.snapshot().unwrap();
    let before_bytes = before.config.to_private_yaml().unwrap().into_bytes();
    std::fs::write(&retained, &before_bytes).unwrap();

    let mut r = request();
    r.path = "/api/ai-launch-shell".into();
    r.body = br#"{"ai_launch_shell":"pwsh"}"#.to_vec();
    let response = router.handle(&r, 1000).response;
    assert_eq!(response.status, 500);
    assert_eq!(json_response(&response)["error"], "save_failed");
    let after = router.config.snapshot().unwrap();
    assert_eq!(after.revision, before.revision);
    assert_eq!(after.config.hub.ai_launch_shell, "auto");
    assert_eq!(std::fs::read(&retained).unwrap(), before_bytes);
    assert_eq!(std::fs::read_dir(&destination).unwrap().count(), 1);
}
#[test]
fn ai_launch_shell_availability_uses_the_fake_filesystem_and_preserves_invalid_mode() {
    use crate::process::execpath::Platform;

    let env = vec![
        r"PATH=C:\PowerShell;C:\Windows\System32".into(),
        r"COMSPEC=C:\Windows\System32\cmd.exe".into(),
    ];
    let mut fs = SyntheticLaunchShellFs::default();
    fs.put(r"C:\PowerShell\pwsh.exe");
    fs.put(r"C:\Windows\System32\cmd.exe");
    let status = super::settings::ai_launch_shell_availability(
        "pwsh",
        Platform::Windows,
        &env,
        std::path::Path::new(r"C:\work"),
        &fs,
    );
    assert_eq!(status["ai_launch_shell"], "pwsh");
    assert_eq!(status["availability"]["pwsh"]["available"], true);
    assert_eq!(status["availability"]["cmd"]["available"], true);
    assert_eq!(
        status["availability"]["cmd"]["executable"],
        r"C:\Windows\System32\cmd.exe"
    );
    assert_eq!(status["selected_availability"]["available"], true);

    let mut missing = SyntheticLaunchShellFs::default();
    missing.put(r"C:\Windows\System32\cmd.exe");
    let status = super::settings::ai_launch_shell_availability(
        "pwsh",
        Platform::Windows,
        &env,
        std::path::Path::new(r"C:\work"),
        &missing,
    );
    assert_eq!(status["ai_launch_shell"], "pwsh");
    assert_eq!(status["availability"]["pwsh"]["available"], false);
    assert!(status["availability"]["pwsh"]["reason"].is_string());
    assert_eq!(status["selected_availability"]["available"], false);

    let invalid = super::settings::ai_launch_shell_availability(
        "imaginary-shell",
        Platform::Windows,
        &env,
        std::path::Path::new(r"C:\work"),
        &fs,
    );
    assert_eq!(invalid["ai_launch_shell"], "imaginary-shell");
    assert_eq!(invalid["selected_availability"]["available"], false);
    assert!(invalid["selected_availability"]["reason"].is_string());
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

fn pin_guard_router() -> (tempfile::TempDir, super::ServiceRouter) {
    let (root, router) = router();
    let mut snapshot = router.config.snapshot().unwrap();
    snapshot.config.remote_pin_hash = bcrypt::hash("123456", 4).unwrap();
    snapshot.config.auth_cookie_secret = "synthetic-secret".into();
    router
        .config
        .persist(snapshot.revision, snapshot.config)
        .unwrap();
    (root, router)
}

fn pin_guard_login(router: &super::ServiceRouter, now: i64) -> String {
    let mut login = pin_guard_write_request("");
    login.path = "/api/auth/login".into();
    login.body = br#"{"pin":"123456"}"#.to_vec();
    let response = router.handle(&login, now).response;
    assert_eq!(response.status, 200);
    response.cookies[0].split(';').next().unwrap().to_owned()
}

fn pin_guard_write_request(cookie: &str) -> Request {
    let mut r = request();
    r.remote_addr = "203.0.113.1:1234".into();
    r.path = "/api/ai-launch-shell".into();
    r.headers = vec![
        ("User-Agent".into(), "Chrome/100 Windows".into()),
        ("Origin".into(), "http://127.0.0.1:48888".into()),
    ];
    if !cookie.is_empty() {
        r.headers.push(("Cookie".into(), cookie.into()));
    }
    r.body = br#"{"ai_launch_shell":"pwsh"}"#.to_vec();
    r
}

fn assert_guarded_setting_write(
    router: &super::ServiceRouter,
    r: &Request,
    now: i64,
    expected: u16,
) {
    let before = router.config.snapshot().unwrap();
    let destination = router.paths.resource(crate::config::Resource::Config);
    let before_bytes = std::fs::read(&destination).unwrap();
    let admission = router.preflight(r, now);
    if expected == 200 {
        assert!(admission.is_ok());
    } else {
        assert_eq!(admission.unwrap_err().status, expected);
    }
    let dispatch = router.handle(r, now);
    assert_eq!(dispatch.response.status, expected);
    let after = router.config.snapshot().unwrap();
    if expected == 200 {
        let body: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
        assert_eq!(after.config.hub.ai_launch_shell, body["ai_launch_shell"]);
        assert_ne!(after.revision, before.revision);
        assert_ne!(std::fs::read(destination).unwrap(), before_bytes);
    } else {
        assert!(dispatch.effects.0.is_empty());
        assert_eq!(after.revision, before.revision);
        assert_eq!(
            after.config.hub.ai_launch_shell,
            before.config.hub.ai_launch_shell
        );
        assert_eq!(std::fs::read(destination).unwrap(), before_bytes);
        if expected == 401 && json_response(&dispatch.response)["error"] == "pin_required" {
            assert!(dispatch.response.headers["Cache-Control"].contains("no-store"));
        }
    }
}

#[test]
fn remote_pin_expiry_guards_actual_write_at_the_exact_boundary() {
    let (_root, router) = pin_guard_router();
    let issued = 1000;
    let expires = issued + 12 * 60 * 60;
    let cookie = pin_guard_login(&router, issued);
    let mut r = pin_guard_write_request(&cookie);
    assert_guarded_setting_write(&router, &r, issued, 200);
    r.body = br#"{"ai_launch_shell":"cmd"}"#.to_vec();
    assert_guarded_setting_write(&router, &r, expires - 1, 200);
    r.body = br#"{"ai_launch_shell":"pwsh"}"#.to_vec();
    assert_guarded_setting_write(&router, &r, expires, 401);
    assert_guarded_setting_write(&router, &r, expires + 1, 401);

    // These base-guard routes deliberately remain usable without a live PIN;
    // their successful responses must not be mistaken for protected API access.
    let before = router.config.snapshot().unwrap();
    let destination = router.paths.resource(crate::config::Resource::Config);
    let before_bytes = std::fs::read(&destination).unwrap();
    r.method = "GET".into();
    r.path = "/api/auth/status".into();
    let status = router.handle(&r, expires).response;
    assert_eq!(status.status, 200);
    assert_eq!(json_response(&status)["authed"], false);
    r.method = "POST".into();
    r.path = "/api/auth/logout".into();
    assert_eq!(router.handle(&r, expires).response.status, 200);
    assert_eq!(router.config.snapshot().unwrap().revision, before.revision);
    assert_eq!(std::fs::read(destination).unwrap(), before_bytes);
}

#[test]
fn remote_pin_device_nonce_and_logout_gate_actual_persisted_writes() {
    let (_root, router) = pin_guard_router();
    let cookie = pin_guard_login(&router, 1000);
    let mut r = pin_guard_write_request("");
    assert_guarded_setting_write(&router, &r, 1001, 401);
    r = pin_guard_write_request(&cookie);
    r.remote_addr = "203.0.113.2:1234".into();
    assert_guarded_setting_write(&router, &r, 1001, 401);
    r = pin_guard_write_request(&cookie);
    r.headers[0].1 = "Firefox/100 Windows".into();
    assert_guarded_setting_write(&router, &r, 1001, 401);
    r.headers[0].1 = "Chrome/100 Android".into();
    assert_guarded_setting_write(&router, &r, 1001, 401);
    r = pin_guard_write_request(&cookie);
    assert_guarded_setting_write(&router, &r, 1001, 200);
    r.body = br#"{"ai_launch_shell":"cmd"}"#.to_vec();

    // A new auth owner has the same persisted signing configuration but none
    // of the old owner's live nonces. A signed cookie alone is insufficient.
    let restarted = super::ServiceRouter::isolated(router.config.clone(), router.paths.clone());
    assert_guarded_setting_write(&restarted, &r, 1002, 401);
    assert_guarded_setting_write(&router, &r, 1002, 200);
    let mut logout = r.clone();
    logout.path = "/api/auth/logout".into();
    let response = router.handle(&logout, 1003).response;
    assert_eq!(response.status, 200);
    assert!(response.cookies.iter().any(
        |cookie| cookie.starts_with(&format!("{PIN_COOKIE}=")) && cookie.contains("Max-Age=0")
    ));
    r.body = br#"{"ai_launch_shell":"pwsh"}"#.to_vec();
    assert_guarded_setting_write(&router, &r, 1003, 401);
    r.headers.last_mut().unwrap().1 = pin_guard_login(&router, 1004);
    assert_guarded_setting_write(&router, &r, 1004, 200);
}

#[test]
fn protected_missing_base_credentials_cannot_mutate_config_even_with_a_valid_pin() {
    let (_root, router) = pin_guard_router();
    let cookie = pin_guard_login(&router, 1000);
    let mut r = pin_guard_write_request(&cookie);
    r.query.clear();
    assert_guarded_setting_write(&router, &r, 1001, 401);
    r.method = "DELETE".into();
    r.host = "foreign.invalid".into();
    r.headers[1].1 = "https://foreign.invalid".into();
    assert_guarded_setting_write(&router, &r, 1001, 401);
    r = pin_guard_write_request(&cookie);
    assert_guarded_setting_write(&router, &r, 1001, 200);
}

#[test]
fn remote_pin_revocation_between_admission_and_dispatch_rechecks_before_write() {
    let (_root, router) = pin_guard_router();
    let cookie = pin_guard_login(&router, 1000);
    let mut r = pin_guard_write_request(&cookie);
    router.preflight(&r, 1001).unwrap();
    let mut logout = r.clone();
    logout.path = "/api/auth/logout".into();
    assert_eq!(router.handle(&logout, 1002).response.status, 200);
    assert_guarded_setting_write(&router, &r, 1003, 401);

    let issued = 1004;
    r.headers.last_mut().unwrap().1 = pin_guard_login(&router, issued);
    router.preflight(&r, issued + 12 * 60 * 60 - 1).unwrap();
    assert_guarded_setting_write(&router, &r, issued + 12 * 60 * 60, 401);
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
    // With a PIN set, a loopback request must also prove it is on this
    // machine: Host and proxy headers pass through a relay unchanged (U09).
    assert_eq!(
        json_response(&router.handle(&r, 1000).response)["error"],
        "pin_required"
    );
    let config = router.config.snapshot().unwrap().config;
    let proof = super::pin::cli_local_proof(&config).unwrap();
    r.headers
        .push((super::pin::LOCAL_PROOF_HEADER.into(), proof.clone()));
    assert_eq!(router.handle(&r, 1000).response.status, 200);
    // A wrong proof proves nothing.
    r.headers.last_mut().unwrap().1 = "0".repeat(proof.len());
    assert_eq!(router.handle(&r, 1000).response.status, 401);
    r.headers.last_mut().unwrap().1 = proof;
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
    // The page holds nothing secret: it is served without the token, and an
    // old token cookie is deleted, never set (U05).
    r.headers
        .push(("Cookie".into(), format!("{TOKEN_COOKIE}=synthetic")));
    let response = router.handle(&r, 1000).response;
    assert_eq!(response.status, 200);
    let cleared: Vec<_> = response
        .cookies
        .iter()
        .filter(|cookie| cookie.starts_with(&format!("{TOKEN_COOKIE}=")))
        .collect();
    assert_eq!(cleared.len(), 1);
    assert!(
        cleared[0].starts_with(&format!("{TOKEN_COOKIE}=;")) && cleared[0].contains("Max-Age=0")
    );
    assert!(!String::from_utf8_lossy(&response.body).contains("synthetic"));
    // With a launch code the sign-in page answers, to trade it.
    r.query = "launch=0123".into();
    let response = router.handle(&r, 1000).response;
    assert_eq!(response.status, 401);
    assert!(
        !response
            .cookies
            .iter()
            .any(|cookie| cookie.contains("synthetic"))
    );
    let html = String::from_utf8(response.body).unwrap();
    assert!(html.contains("many-ai-cli-token"));
    assert!(!html.contains("__NONCE__"));
    // 戻り先が //evil や /\evil のとき、別サイトへ飛ばさず / に戻すガード（実行はできないので文面で固定する）。
    assert!(html.contains("path.charAt(1) === '/'") && html.contains("path.charAt(1) === '\\\\'"));
    // The launch code is traded once and taken out of the address bar first.
    assert!(html.contains("'/api/auth/launch'") && html.contains("params.delete('launch')"));
    // Audit N24: on this machine the page keeps the token in the tab only and
    // deletes what an earlier version left (web/tests/token-store.test.ts runs
    // this script; here the text that reaches the browser is pinned).
    assert!(
        html.contains("sessionStorage.setItem('many-ai-cli-token', token)")
            && html.contains("localStorage.removeItem('many-ai-cli-token')")
            && html.contains("id=\"failed-here\"")
    );
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
fn service_query_ignores_go_invalid_pairs_before_header_fallback() {
    let mut r = request();
    r.headers
        .push(("Authorization".into(), "Bearer synthetic".into()));
    for query in [
        "token=bad;pair",
        "token=%ZZ",
        "ignored=bad;pair&token=synthetic",
    ] {
        r.query = query.into();
        assert_eq!(request_token(&r), "synthetic", "{query}");
    }
    // The token cookie of an earlier version is not read (U05).
    r.headers = vec![("Cookie".into(), format!("{TOKEN_COOKIE}=synthetic"))];
    r.query = "token=bad;pair".into();
    assert_eq!(request_token(&r), "");
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
    use std::time::Duration;
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

#[test]
fn audit_u08_unidentified_remote_pin_attempts_share_a_staged_lock() {
    use std::time::Duration;
    let auth = super::pin::AuthService::default();
    let mut cfg = config();
    cfg.remote_pin_hash = bcrypt::hash("123456", 4).unwrap();
    cfg.auth_cookie_secret = "synthetic-secret".into();
    // A proxy that forwards for the client but names no Tailscale user.
    let mut r = request();
    r.headers
        .push(("X-Forwarded-For".into(), "198.51.100.7".into()));
    r.body = br#"{"pin":"654321"}"#.to_vec();
    let mut other = r.clone();
    other.headers = vec![("X-Forwarded-For".into(), "198.51.100.8".into())];
    let mut at = UNIX_EPOCH + Duration::from_secs(1_000);
    for lock in [60, 300, 1800] {
        for _ in 0..4 {
            assert_eq!(auth.login_at(&r, &cfg, at).status, 401);
        }
        assert_eq!(auth.login_at(&r, &cfg, at).status, 429);
        let status = json_response(&auth.status_at(&r, &cfg, at + Duration::from_secs(1)));
        assert_eq!(status["retry_after"], lock);
        // Another unidentified client is behind the same lock.
        assert_eq!(auth.login_at(&other, &cfg, at).status, 429);
        at = at + Duration::from_secs(lock);
    }
    // A Tailscale user keeps a lock of their own.
    let mut named = r.clone();
    named.headers.push((
        "Tailscale-User-Login".into(),
        "synthetic-tailnet-user".into(),
    ));
    assert_eq!(auth.login_at(&named, &cfg, at).status, 401);
}

#[test]
fn audit_u08_a_pin_lock_that_starts_is_reported_once() {
    use std::time::Duration;
    let auth = super::pin::AuthService::default();
    let mut cfg = config();
    cfg.remote_pin_hash = bcrypt::hash("123456", 4).unwrap();
    cfg.auth_cookie_secret = "synthetic-secret".into();
    let mut r = request();
    r.remote_addr = "203.0.113.9:5000".into();
    r.headers.push((
        "User-Agent".into(),
        "Mozilla/5.0 (Windows NT 10.0) Chrome/1".into(),
    ));
    r.body = br#"{"pin":"654321"}"#.to_vec();
    let at = UNIX_EPOCH + Duration::from_secs(1_000);
    for _ in 0..4 {
        assert_eq!(auth.login_at(&r, &cfg, at).status, 401);
        assert!(auth.take_lockout_alerts().is_empty());
    }
    assert_eq!(auth.login_at(&r, &cfg, at).status, 429);
    assert_eq!(
        auth.take_lockout_alerts(),
        vec![super::pin::LockoutAlert {
            source: "203.0.113.9".into(),
            device: "chrome/windows".into(),
            seconds: 60,
            global: false,
        }]
    );
    // Attempts refused while locked do not report it again.
    assert_eq!(auth.login_at(&r, &cfg, at).status, 429);
    assert!(auth.take_lockout_alerts().is_empty());

    // The right PIN on the attempt that reaches the limit leaves no lock.
    let fresh = super::pin::AuthService::default();
    for _ in 0..4 {
        assert_eq!(fresh.login_at(&r, &cfg, at).status, 401);
    }
    let mut right = r.clone();
    right.body = br#"{"pin":"123456"}"#.to_vec();
    assert_eq!(fresh.login_at(&right, &cfg, at).status, 200);
    assert!(fresh.take_lockout_alerts().is_empty());
}

#[test]
fn audit_u04_a_launch_code_opens_the_dashboard_once_for_this_user_only() {
    let (_dir, router) = router();
    let mut cfg = config();
    cfg.token = "synthetic-launch-token".into();
    let launch = |code: &str, remote: &str, own: bool, now: i64| {
        let mut r = request();
        r.path = "/api/auth/launch".into();
        r.remote_addr = remote.into();
        r.body = format!(r#"{{"code":"{code}"}}"#).into_bytes();
        router.auth.launch_for_test(&r, &cfg, now, own)
    };
    let code = router.auth.mint_launch_code(1000).unwrap();
    assert_eq!(code.len(), 32);
    // Another OS user's connection spends the code and gets nothing.
    assert_eq!(launch(&code, "127.0.0.1:50000", false, 1000).status, 401);
    assert_eq!(launch(&code, "127.0.0.1:50000", true, 1000).status, 401);
    // This user's connection gets the token, once.
    let code = router.auth.mint_launch_code(1000).unwrap();
    let response = launch(&code, "127.0.0.1:50001", true, 1010);
    assert_eq!(response.status, 200);
    assert_eq!(json_response(&response)["token"], "synthetic-launch-token");
    assert!(response.cookies.is_empty());
    assert_eq!(launch(&code, "127.0.0.1:50001", true, 1010).status, 401);
    // A minute later the code is gone.
    let code = router.auth.mint_launch_code(1000).unwrap();
    assert_eq!(launch(&code, "127.0.0.1:50002", true, 1060).status, 401);
    // From another machine, or addressed by another name, it is refused.
    let code = router.auth.mint_launch_code(1000).unwrap();
    assert_eq!(launch(&code, "203.0.113.5:50003", true, 1000).status, 401);
    let code = router.auth.mint_launch_code(1000).unwrap();
    assert_eq!(launch("wrong", "127.0.0.1:50004", true, 1000).status, 401);
    assert_eq!(launch(&code, "127.0.0.1:50004", true, 1000).status, 200);

    // Codes are issued only to a local client that holds the token.
    let mut r = request();
    r.path = "/api/auth/launch-code".into();
    let response = router.handle(&r, 1000).response;
    assert_eq!(response.status, 200);
    assert_eq!(json_response(&response)["expires_in"], 60);
    r.remote_addr = "203.0.113.5:4000".into();
    assert_ne!(router.handle(&r, 1000).response.status, 200);
    let mut r = request();
    r.path = "/api/auth/launch-code".into();
    r.query.clear();
    assert_eq!(router.handle(&r, 1000).response.status, 401);
}

#[test]
fn audit_u09_a_browser_the_hub_opened_counts_as_local_for_the_pin() {
    let mut cfg = config();
    cfg.remote_pin_hash = bcrypt::hash("123456", 4).unwrap();
    cfg.auth_cookie_secret = "synthetic-secret".into();
    let auth = super::pin::AuthService::default();
    let mut page = request();
    page.path = "/api/input-config".into();
    page.method = "GET".into();
    // A loopback request that proves nothing needs the PIN.
    assert!(auth.treated_remote(&page, &cfg, 1000));
    assert_eq!(
        auth.guard(&page, &cfg, 48888, &["GET"], 1000)
            .unwrap_err()
            .status,
        401
    );
    // The browser that redeems a launch code gets the local cookie.
    let code = auth.mint_launch_code(1000).unwrap();
    let mut launch = request();
    launch.path = "/api/auth/launch".into();
    launch.body = format!(r#"{{"code":"{code}"}}"#).into_bytes();
    let response = auth.launch_for_test(&launch, &cfg, 1000, true);
    assert_eq!(response.status, 200);
    let cookie = response
        .cookies
        .iter()
        .find(|cookie| cookie.starts_with(&format!("{}=", super::pin::LOCAL_COOKIE)))
        .unwrap();
    assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"));
    let pair = cookie.split(';').next().unwrap().to_owned();
    page.headers.push(("Cookie".into(), pair.clone()));
    assert!(!auth.treated_remote(&page, &cfg, 1000));
    assert!(auth.guard(&page, &cfg, 48888, &["GET"], 1000).is_ok());
    // It does not make a proxied request local, nor outlive its 7 days,
    // nor survive a new cookie secret (revoking all access).
    let mut proxied = page.clone();
    proxied
        .headers
        .push(("X-Forwarded-For".into(), "203.0.113.9".into()));
    assert!(auth.treated_remote(&proxied, &cfg, 1000));
    assert!(!auth.treated_remote(&page, &cfg, 1000 + 7 * 24 * 60 * 60 - 1));
    assert!(auth.treated_remote(&page, &cfg, 1000 + 7 * 24 * 60 * 60));
    let rotated = Config {
        auth_cookie_secret: "rotated-secret".into(),
        ..cfg.clone()
    };
    assert!(auth.treated_remote(&page, &rotated, 1000));
    // Without a PIN nothing changes for loopback.
    let open = config();
    let mut plain = request();
    plain.path = "/api/input-config".into();
    assert!(!auth.treated_remote(&plain, &open, 1000));
}

/// A browser the Hub opened: the request it sends, with its local cookie.
fn opened_browser(auth: &super::pin::AuthService, cfg: &Config, now: i64) -> Request {
    let code = auth.mint_launch_code(now).unwrap();
    let mut launch = request();
    launch.path = "/api/auth/launch".into();
    launch.body = format!(r#"{{"code":"{code}"}}"#).into_bytes();
    let response = auth.launch_for_test(&launch, cfg, now, true);
    assert_eq!(response.status, 200);
    let pair = response
        .cookies
        .iter()
        .find(|cookie| cookie.starts_with(&format!("{}=", super::pin::LOCAL_COOKIE)))
        .and_then(|cookie| cookie.split(';').next())
        .unwrap()
        .to_owned();
    let mut page = request();
    page.path = "/api/input-config".into();
    page.method = "GET".into();
    page.headers.push(("Cookie".into(), pair));
    page
}

#[test]
fn audit_n25_the_local_cookie_counts_only_while_the_hub_remembers_handing_it_out() {
    let mut cfg = config();
    cfg.remote_pin_hash = bcrypt::hash("123456", 4).unwrap();
    cfg.auth_cookie_secret = "synthetic-secret".into();
    let auth = super::pin::AuthService::default();
    let page = opened_browser(&auth, &cfg, 1000);
    assert!(!auth.treated_remote(&page, &cfg, 1000));
    // The same cookie from a browser that looks different is not the one the
    // Hub opened.
    let mut other = page.clone();
    other.headers.push((
        "User-Agent".into(),
        "Mozilla/5.0 (X11; Linux x86_64) Firefox/130.0".into(),
    ));
    assert!(auth.treated_remote(&other, &cfg, 1000));
    // A Hub that was restarted remembers no cookie, however well signed.
    let restarted = super::pin::AuthService::default();
    assert!(restarted.treated_remote(&page, &cfg, 1000));
    // Logging out ends it, and tells the browser to drop the cookie.
    let response = auth.logout(&page, &cfg);
    assert!(response.cookies.iter().any(|cookie| {
        cookie.starts_with(&format!("{}=;", super::pin::LOCAL_COOKIE))
            && cookie.contains("Max-Age=0")
    }));
    assert!(auth.treated_remote(&page, &cfg, 1000));
    assert_eq!(
        auth.guard(&page, &cfg, 48888, &["GET"], 1000)
            .unwrap_err()
            .status,
        401
    );
    // Another browser's cookie is not touched by that logout.
    let second = opened_browser(&auth, &cfg, 1000);
    let third = opened_browser(&auth, &cfg, 1000);
    auth.logout(&second, &cfg);
    assert!(auth.treated_remote(&second, &cfg, 1000));
    assert!(!auth.treated_remote(&third, &cfg, 1000));
}

#[test]
fn audit_n25_changing_or_clearing_the_pin_ends_every_local_cookie() {
    for body in [r#"{"pin":"654321"}"#, r#"{"clear":true}"#] {
        let dir = tempfile::tempdir().unwrap();
        let trial = dir.path().join("trial");
        std::fs::create_dir(&trial).unwrap();
        let paths =
            crate::config::RuntimePaths::trial(&trial, 48888, &dir.path().join("installed"))
                .unwrap();
        let mut cfg = config();
        cfg.remote_pin_hash = bcrypt::hash("123456", 4).unwrap();
        cfg.auth_cookie_secret = "synthetic-secret".into();
        let store = crate::config::ConfigStore::new(paths, cfg.clone()).unwrap();
        let auth = super::pin::AuthService::default();
        let page = opened_browser(&auth, &cfg, 1000);
        let bystander = opened_browser(&auth, &cfg, 1000);
        let mut change = page.clone();
        change.method = "POST".into();
        change.path = "/api/auth/set-pin".into();
        change.body = body.as_bytes().to_vec();
        assert_eq!(auth.set_pin(&change, &store, 1000).status, 200, "{body}");
        // Judged against a config that still asks for a PIN: neither cookie
        // counts any more (the secret did not change, only the Hub's memory).
        assert!(auth.treated_remote(&page, &cfg, 1000), "{body}");
        assert!(auth.treated_remote(&bystander, &cfg, 1000), "{body}");
    }
}
#[test]
fn audit_u05_an_asset_ticket_reads_media_only_and_ends_with_the_token() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let cfg = config();
    let ticket = asset_ticket(&cfg.token, now);
    let mut r = request();
    r.query = format!("ticket={ticket}&path=x");
    r.method = "GET".into();
    for path in [
        "/api/files-asset",
        "/api/files-download",
        "/api/avatar",
        "/api/user-prefs/notify-sound-custom",
        "/api/memo-images/a.png",
        "/api/provider-icons/claude",
    ] {
        r.path = path.into();
        assert!(token_or_trusted(&r, &cfg), "{path}");
    }
    // It opens no other route, and does not write.
    r.path = "/api/files-content".into();
    assert!(!token_or_trusted(&r, &cfg));
    r.path = "/api/asset-ticket".into();
    assert!(!token_or_trusted(&r, &cfg));
    r.path = "/api/files-asset".into();
    r.method = "POST".into();
    assert!(!token_or_trusted(&r, &cfg));
    r.method = "GET".into();
    // A rotated token, an expired or a forged ticket is refused.
    let rotated = Config {
        token: "rotated".into(),
        ..config()
    };
    assert!(!token_or_trusted(&r, &rotated));
    assert!(!valid_asset_ticket(
        &ticket,
        &cfg.token,
        now + ASSET_TICKET_TTL
    ));
    let forged = format!("{}.{}", now + 60, "0".repeat(64));
    assert!(!valid_asset_ticket(&forged, &cfg.token, now));
    let far = asset_ticket(&cfg.token, now + ASSET_TICKET_TTL);
    assert!(!valid_asset_ticket(&far, &cfg.token, now));
    // Only a token holder is given a ticket.
    let (_dir, router) = router();
    let mut r = request();
    r.path = "/api/asset-ticket".into();
    r.method = "GET".into();
    let response = router.handle(&r, 1000).response;
    assert_eq!(response.status, 200);
    let given = json_response(&response)["ticket"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(valid_asset_ticket(&given, "synthetic", 1000));
    r.query = format!("ticket={given}");
    assert_eq!(router.handle(&r, 1000).response.status, 401);
}

#[test]
fn audit_u04_the_dashboard_trades_a_launch_code_on_the_sign_in_page() {
    let (_dir, router) = router();
    let mut r = request();
    r.path = "/".into();
    r.method = "GET".into();
    r.headers.push(("Accept".into(), "text/html".into()));
    // Even a signed-in browser trades the code, so the token it keeps is
    // the one the code stands for.
    r.query = "token=synthetic&launch=0123".into();
    let response = router.handle(&r, 1000).response;
    assert_eq!(response.status, 401);
    assert!(
        String::from_utf8(response.body)
            .unwrap()
            .contains("/api/auth/launch")
    );
    // The launch route itself needs no token, but still the Host and Origin.
    let mut r = request();
    r.path = "/api/auth/launch".into();
    r.query.clear();
    r.body = br#"{"code":"0123"}"#.to_vec();
    assert_eq!(router.handle(&r, 1000).response.status, 401);
    r.host = "evil.invalid".into();
    assert_eq!(router.handle(&r, 1000).response.status, 403);
    let mut r = request();
    r.path = "/api/auth/launch".into();
    r.headers
        .push(("Origin".into(), "http://evil.invalid".into()));
    assert_eq!(router.handle(&r, 1000).response.status, 403);
}

#[test]
fn audit_u08_the_global_pin_lock_is_reported_as_global() {
    let mut limiter = super::pin::Limiter::default();
    for i in 0..30 {
        assert_eq!(limiter.begin(&format!("key-{i}"), 1000), 0);
    }
    assert_eq!(
        limiter.take_started(),
        vec![super::pin::Lockout {
            seconds: 60,
            global: true,
        }]
    );
    assert!(limiter.take_started().is_empty());
}

#[test]
fn audit_u08_pin_attempts_per_day_stay_low_for_one_key_and_for_new_keys() {
    // One unidentified key, tried whenever it is not locked for 24 hours.
    let mut limiter = super::pin::Limiter::default();
    let mut tries = 0;
    for now in 0..86_400 {
        if limiter.retry_after("remote:anon", now) == 0 && limiter.begin("remote:anon", now) == 0 {
            tries += 1;
        }
    }
    assert!(tries <= 330, "{tries} attempts in a day");
    // A new key for every attempt only meets the global counter, which is
    // staged as well: 30 attempts, then 60, 300 and 1800 seconds.
    let mut limiter = super::pin::Limiter::default();
    let mut lock_lengths = Vec::new();
    let mut now = 0;
    let mut key = 0;
    while lock_lengths.len() < 4 {
        key += 1;
        assert_eq!(limiter.begin(&format!("key-{key}"), now), 0);
        let retry = limiter.retry_after("", now);
        if retry > 0 {
            lock_lengths.push(retry - 1);
            now += retry - 1;
        }
    }
    assert_eq!(lock_lengths, [60, 300, 1800, 1800]);
    // An hour without attempts starts the global steps again.
    for _ in 0..29 {
        key += 1;
        assert_eq!(limiter.begin(&format!("key-{key}"), now + 3_601), 0);
    }
    assert_eq!(limiter.begin("key-last", now + 3_601), 0);
    assert_eq!(limiter.retry_after("", now + 3_601), 61);
}
