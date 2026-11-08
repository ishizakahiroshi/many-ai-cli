use super::*;
#[test]
fn pinned_go_status_corpus_preserves_secret_free_shapes_and_typed_json_failures() {
    let cases: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/subscription-status/cases.json"
    ))
    .unwrap();
    let oracle: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/subscription-status/go-observations.json"
    ))
    .unwrap();
    assert_eq!(cases.as_array().unwrap().len(), 30);
    for (case, expected) in cases
        .as_array()
        .unwrap()
        .iter()
        .zip(oracle.as_array().unwrap())
    {
        let result = parse_status(
            case["provider"].as_str().unwrap(),
            case["output"].as_str().unwrap(),
            case["code"].as_i64().unwrap() as i32,
        );
        let observed = match result {
            Ok(status) => serde_json::json!({"status":status,"error":""}),
            Err(error) => serde_json::json!({"status":Status::default(),"error":error.to_string()}),
        };
        assert_eq!(&observed, expected, "provider {}", case["provider"]);
        let encoded = observed.to_string();
        for private in [
            "synthetic@example.com",
            "synthetic-secret-value",
            "synthetic-only",
        ] {
            assert!(!encoded.contains(private));
        }
    }
}
#[test]
fn claude_account_fingerprint_tells_same_account_apart_without_exposing_it() {
    let output = |email: &str, org: &str| {
        format!(
            r#"{{"loggedIn":true,"authMethod":"claude.ai","subscriptionType":"max","email":"{email}","orgId":"{org}"}}"#
        )
    };
    let a = parse_status("claude", &output("Synthetic@Example.com", "org-1"), 0).unwrap();
    let b = parse_status("claude", &output("synthetic@example.com", "org-1"), 0).unwrap();
    let c = parse_status("claude", &output("synthetic@example.com", "org-2"), 0).unwrap();
    assert!(a.account.is_some());
    assert_eq!(a.account, b.account, "case differences are one account");
    assert_ne!(
        a.account, c.account,
        "another organization is another account"
    );
    let encoded = serde_json::to_string(&a).unwrap();
    assert!(!encoded.contains("synthetic") && !encoded.contains("account"));
}
#[test]
fn claude_status_without_identity_fields_has_no_fingerprint() {
    let status =
        parse_status("claude", r#"{"loggedIn":true,"authMethod":"claude.ai"}"#, 0).unwrap();
    assert!(status.logged_in);
    assert!(status.account.is_none());
}
#[test]
fn vendor_status_command_has_fixed_read_args_and_profile_environment() {
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49299, installed.path()).unwrap();
    let executable = root
        .path()
        .join(if cfg!(windows) { "codex.exe" } else { "codex" });
    std::fs::write(&executable, b"synthetic-only-not-an-executable").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let native = NativeSubscriptionCli::new(
        paths,
        root.path().into(),
        vec![
            format!("PATH={}", root.path().display()),
            "CODEX_HOME=must-be-replaced".into(),
        ],
    );
    let profile = root.path().join("profile");
    std::fs::create_dir(&profile).unwrap();
    let plan = native
        .command("codex", &profile, Duration::from_secs(20))
        .unwrap();
    assert_eq!(
        plan.args,
        vec![OsString::from("login"), OsString::from("status")]
    );
    assert_eq!(
        plan.env[&OsString::from("CODEX_HOME")],
        Some(profile.as_os_str().to_owned())
    );
    assert_eq!(plan.timeout, Duration::from_secs(20));
    assert!(
        !plan
            .env
            .values()
            .any(|v| v.as_ref().is_some_and(|v| v == "must-be-replaced"))
    );
}
