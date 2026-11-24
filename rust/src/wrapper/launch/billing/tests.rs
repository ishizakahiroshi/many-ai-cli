//! Synthetic values only: no real key, key command, Claude Code login or paid
//! run. The fake `--version` probe prints a fixed string.
use super::*;
use crate::{
    config::{Config, Resource, private_io},
    wrapper::{
        hooks::SessionHooks,
        launch::{self, WrapperArgs},
    },
};

const RECIPE: &str = "synthetic-resolver-must-never-run";

fn trial() -> (tempfile::TempDir, tempfile::TempDir, RuntimePaths) {
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49001, installed.path()).unwrap();
    (root, installed, paths)
}
fn save_recipe(paths: &RuntimePaths) {
    private_io::write_atomic(
        &paths.resource(Resource::HeadlessBilling),
        format!(r#"{{"version":1,"profiles":[{{"id":"synthetic","display_name":"Synthetic","provider":"claude","command":"{RECIPE}"}}]}}"#).as_bytes(),
    )
    .unwrap();
}
fn selected(paths: &RuntimePaths) -> ResolvedBilling {
    headless_billing::resolve(
        paths,
        headless_billing::validate_selection(
            "claude",
            "headless",
            headless_billing::API_KEY,
            "synthetic",
            Some(0.25),
            "",
        )
        .unwrap(),
    )
    .unwrap()
}
fn api_launch(billing: &ResolvedBilling) -> LaunchPlan {
    launch::prepare_launch(
        &Config::default(),
        "claude",
        WrapperArgs {
            headless: true,
            billing_mode: billing.mode.clone(),
            billing_profile: billing.profile_id.clone(),
            billing_revision: billing.revision.clone(),
            max_budget_usd: billing.max_budget_usd.unwrap().to_string(),
            ..Default::default()
        },
    )
    .unwrap()
}
fn registered_with(billing: Option<&ResolvedBilling>) -> Message {
    let mut data = serde_json::json!({"headless_run_id": "9:1"});
    if let Some(billing) = billing {
        data["headless_billing"] = serde_json::json!({
            "mode": billing.mode,
            "profile_id": billing.profile_id,
            "revision": billing.revision,
            "max_budget_usd": billing.max_budget_usd,
        });
    }
    Message {
        session_id: 9,
        data: serde_json::to_vec(&data).unwrap(),
        ..Default::default()
    }
}
/// A process that prints `output` and exits 0, writing `marker` if given.
fn fake_probe(cwd: &Path, output: &str) -> ProcessPlan {
    #[cfg(windows)]
    let (executable, args) = (
        PathBuf::from(std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".into())),
        vec!["/d".into(), "/c".into(), format!("echo {output}").into()],
    );
    #[cfg(unix)]
    let (executable, args) = (
        PathBuf::from("/bin/sh"),
        vec![
            "-c".into(),
            "printf '%s\\n' \"$1\"; printf '%s' synthetic-stderr >&2".into(),
            "synthetic".into(),
            output.into(),
        ],
    );
    ProcessPlan {
        executable,
        args,
        cwd: cwd.into(),
        env: Default::default(),
        stdin: Vec::new(),
        timeout: Duration::from_secs(10),
        output_cap: 100,
        pipe_drain_timeout: Duration::from_secs(1),
    }
}

#[test]
fn conflicting_flags_fail_closed_for_api_billing_only() {
    for flag in [
        "--settings",
        "--setting-sources",
        "--managed-settings",
        "--project-config-root",
        "--client-data-url",
        "--worktree",
        "--desktop",
        "--sdk-url",
        "--teleport",
        "--cloud",
        "--environment",
        "--remote",
        "--remote-control",
        "--bg",
        "--background",
        "--tmux",
        "--resume",
        "--continue",
        "--fork-session",
        "--from-pr",
        "--debug",
        "--debug-file",
        "--debug-to-stderr",
        "--bare",
        "--max-budget-usd",
        "--session-id",
        "--add-dir",
        "--mcp-config",
        "--plugin-dir",
        "--plugin-url",
    ] {
        for args in [
            vec![flag.to_owned(), "synthetic".into()],
            vec![format!("{flag}=synthetic")],
        ] {
            let error = validate_provider_args(&args, true).unwrap_err();
            assert_eq!(
                error.to_string(),
                BillingError::AuthConflict.to_string(),
                "{flag}"
            );
            // The plan login launches as before: these checks do not apply.
            assert!(validate_provider_args(&args, false).is_ok(), "{flag}");
        }
    }
    for flag in [
        "-d", "-dapi", "-d=api", "-d2e", "-pd", "-pdapi", "-w", "-wother", "-pwother", "-r",
        "-rprior", "-prprior", "-c", "-cp", "-pc",
    ] {
        assert!(
            validate_provider_args(&[flag.into()], true).is_err(),
            "{flag}"
        );
    }
    for args in [
        vec!["--model", "sonnet", "--effort", "high"],
        vec!["--permission-mode", "dontAsk"],
        vec!["-p"],
        vec!["-nwork-debug"],
        vec!["--name=work-debug"],
    ] {
        assert!(
            validate_provider_args(
                &args.into_iter().map(str::to_owned).collect::<Vec<_>>(),
                true
            )
            .is_ok()
        );
    }
}

#[test]
fn hosted_auth_and_debug_environment_cannot_bypass_flag_guard() {
    for name in [
        "CLAUDE_CODE_CLIENT_DATA_URL",
        "DEBUG",
        "DEBUG_SDK",
        "DEBUG_AUTH",
        "CLAUDE_DEBUG",
        "NODE_DEBUG",
        "CLAUDE_CODE_DEBUG_LOGS_DIR",
        "CLAUDE_CODE_DEBUG_LOG_LEVEL",
    ] {
        assert!(conflicting_launch_environment(&[format!(
            "{name}=synthetic"
        )]));
        assert!(conflicting_launch_environment(&[format!(
            "{}=",
            name.to_ascii_lowercase()
        )]));
    }
    assert!(!conflicting_launch_environment(&[
        "CLAUDE_CONFIG_DIR=/synthetic/known".into(),
        "TERM=xterm".into()
    ]));
}

#[test]
fn settings_inspection_rejects_helpers_env_and_managed_overrides() {
    let (root, _installed, paths) = trial();
    let home = root.path().join("home");
    let cwd = root.path().join("project");
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    std::fs::create_dir(&cwd).unwrap();
    assert!(check_auth_files(&paths, &home, &cwd, &[]).is_ok());
    std::fs::write(
        home.join(".claude/settings.json"),
        r#"{"apiKeyHelper":"synthetic-do-not-execute"}"#,
    )
    .unwrap();
    assert!(check_auth_files(&paths, &home, &cwd, &[]).is_err());
    std::fs::write(home.join(".claude/settings.json"), "{}").unwrap();
    for key in [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_BASE_URL",
        "CLAUDE_CODE_USE_BEDROCK",
        "CLAUDE_CODE_CLIENT_DATA_URL",
        "DEBUG",
        "CLAUDE_CODE_DEBUG_LOGS_DIR",
    ] {
        assert!(check_auth_files(&paths, &home, &cwd, &[format!("{key}=synthetic")]).is_err());
        std::fs::write(
            home.join(".claude/settings.json"),
            serde_json::to_vec(&serde_json::json!({"env": {key: "synthetic"}})).unwrap(),
        )
        .unwrap();
        assert!(check_auth_files(&paths, &home, &cwd, &[]).is_err(), "{key}");
    }
    std::fs::write(home.join(".claude/settings.json"), "{}").unwrap();
    // A project settings file above cwd counts too.
    std::fs::create_dir_all(root.path().join(".claude")).unwrap();
    std::fs::write(
        root.path().join(".claude/settings.local.json"),
        r#"{"forceLoginMethod":"claudeai"}"#,
    )
    .unwrap();
    assert!(check_auth_files(&paths, &home, &cwd, &[]).is_err());
    std::fs::remove_file(root.path().join(".claude/settings.local.json")).unwrap();
    assert!(check_auth_files(&paths, &home, &cwd, &[]).is_ok());
    // Any managed (policy) file is refused, whatever it holds.
    std::fs::write(home.join(".claude/managed-settings.json"), "{}").unwrap();
    assert!(check_auth_files(&paths, &home, &cwd, &[]).is_err());
}

#[test]
fn initial_config_root_is_supported_but_a_nested_root_override_is_refused() {
    let (root, _installed, paths) = trial();
    let home = root.path().join("home");
    let cwd = root.path().join("project");
    let selected = root.path().join("selected-profile");
    for directory in [&home, &cwd, &selected] {
        std::fs::create_dir_all(directory).unwrap();
    }
    let environment = vec![format!("CLAUDE_CONFIG_DIR={}", selected.display())];
    assert!(check_auth_files(&paths, &home, &cwd, &environment).is_ok());
    for key in ["CLAUDE_CONFIG_DIR", "claude_config_dir"] {
        std::fs::write(
            selected.join("settings.json"),
            serde_json::to_vec(&serde_json::json!({
                "env": {key: root.path().join("uninspected-profile").to_string_lossy()},
            }))
            .unwrap(),
        )
        .unwrap();
        let error = check_auth_files(&paths, &home, &cwd, &environment).unwrap_err();
        assert_eq!(error.to_string(), BillingError::AuthConflict.to_string());
    }
    // A relative root is refused before anything is read.
    assert!(check_auth_files(&paths, &home, &cwd, &["CLAUDE_CONFIG_DIR=relative".into()]).is_err());
}

/// Plan login is the default: a key the environment or settings provide would
/// bill the run to it, so the run stops instead. Synthetic names and values.
#[test]
fn a_plan_login_run_stops_when_a_key_would_bill_it_instead() {
    let (root, _installed, paths) = trial();
    let home = root.path().join("home");
    let cwd = root.path().join("project");
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    std::fs::create_dir(&cwd).unwrap();
    let plan = |environment: &[String]| check_plan_login(&paths, &home, &cwd, environment);
    let refused = |result: io::Result<()>| {
        result.unwrap_err().to_string() == BillingError::PlanLoginWouldUseKey.to_string()
    };
    // Nothing set: the plan run starts as before. The plan's own login token,
    // a debug switch or another config root are not a key.
    assert!(plan(&[]).is_ok());
    assert!(
        plan(&[
            "CLAUDE_CODE_OAUTH_TOKEN=synthetic".into(),
            "DEBUG=1".into(),
            "ANTHROPIC_API_KEY=".into(),
        ])
        .is_ok()
    );
    // A key in the environment.
    let error = plan(&["ANTHROPIC_API_KEY=synthetic-not-a-key".into()]).unwrap_err();
    assert!(!error.to_string().contains("synthetic-not-a-key"));
    assert!(refused(Err(error)));
    // A local model route clears the key and sets its own endpoint.
    assert!(
        plan(&[
            "ANTHROPIC_AUTH_TOKEN=synthetic".into(),
            "ANTHROPIC_API_KEY=".into(),
            "ANTHROPIC_BASE_URL=http://127.0.0.1:9".into(),
        ])
        .is_ok()
    );
    // A key helper or key in the user settings.
    for settings in [
        serde_json::json!({"apiKeyHelper": "synthetic-do-not-execute"}),
        serde_json::json!({"env": {"ANTHROPIC_API_KEY": "synthetic"}}),
    ] {
        std::fs::write(
            home.join(".claude/settings.json"),
            serde_json::to_vec(&settings).unwrap(),
        )
        .unwrap();
        assert!(refused(plan(&[])), "{settings}");
    }
    // Other settings, and a file that does not parse, are left as before.
    for settings in [r#"{"model":"synthetic"}"#, "{not json"] {
        std::fs::write(home.join(".claude/settings.json"), settings).unwrap();
        assert!(plan(&[]).is_ok(), "{settings}");
    }
    // A project's settings above cwd count too.
    std::fs::create_dir_all(root.path().join(".claude")).unwrap();
    std::fs::write(
        root.path().join(".claude/settings.local.json"),
        r#"{"apiKeyHelper":"synthetic-do-not-execute"}"#,
    )
    .unwrap();
    assert!(refused(plan(&[])));
    std::fs::remove_file(root.path().join(".claude/settings.local.json")).unwrap();
    assert!(plan(&[]).is_ok());
}

#[test]
fn only_reviewed_claude_code_versions_are_accepted() {
    for ok in ["2.1.295 (Claude Code)", "2.1.330 (Claude Code)\r\n"] {
        assert!(verified_version(ok.as_bytes()), "{ok}");
    }
    for refused in [
        "2.1.294 (Claude Code)",
        // Not reviewed yet: a newer patch is not assumed to read the same
        // settings and credential sources.
        "2.1.296 (Claude Code)",
        "2.1.331 (Claude Code)",
        "2.1.1000 (Claude Code)",
        "2.1.0295 (Claude Code)",
        "2.2.0 (Claude Code)",
        "3.0.0 (Claude Code)",
        "2.1.330",
        "2.1.330 (Other)",
        "2.1.x (Claude Code)",
        "2.1.+330 (Claude Code)",
        "",
    ] {
        assert!(!verified_version(refused.as_bytes()), "{refused}");
    }
}

#[test]
fn only_the_hub_confirmed_selection_counts() {
    let (_root, _installed, paths) = trial();
    save_recipe(&paths);
    let billing = selected(&paths);
    assert!(hub_confirmed(&registered_with(Some(&billing)), &billing));
    // A manual `wrap` (or an older Hub) never carries the confirmation.
    assert!(!hub_confirmed(&registered_with(None), &billing));
    assert!(!hub_confirmed(&Message::default(), &billing));
    for change in 0..4 {
        let mut other = billing.clone();
        match change {
            0 => other.profile_id = "another".into(),
            1 => other.revision = "0".repeat(64),
            2 => other.max_budget_usd = Some(100.0),
            _ => other.mode = headless_billing::PLAN_LOGIN.into(),
        }
        assert!(!hub_confirmed(&registered_with(Some(&other)), &billing));
    }
    let mut extra = registered_with(Some(&billing));
    let mut value: serde_json::Value = serde_json::from_slice(&extra.data).unwrap();
    value["headless_billing"]["command"] = RECIPE.into();
    extra.data = serde_json::to_vec(&value).unwrap();
    assert!(!hub_confirmed(&extra, &billing));
}

#[test]
fn wrapper_flags_shape_is_checked_before_registration() {
    let (_root, _installed, paths) = trial();
    save_recipe(&paths);
    let billing = selected(&paths);
    let plan = api_launch(&billing);
    assert!(plan.billing.is_api());
    assert_eq!(plan.billing.revision, billing.revision);
    let base = WrapperArgs {
        headless: true,
        billing_mode: billing.mode.clone(),
        billing_profile: billing.profile_id.clone(),
        billing_revision: billing.revision.clone(),
        max_budget_usd: "0.25".into(),
        ..Default::default()
    };
    let refused = |provider: &str, change: &dyn Fn(&mut WrapperArgs)| {
        let mut args = base.clone();
        change(&mut args);
        launch::prepare_launch(&Config::default(), provider, args).is_err()
    };
    assert!(refused("codex", &|_| {}));
    assert!(refused("claude", &|a| a.headless = false));
    assert!(refused("claude", &|a| a.billing_revision.clear()));
    assert!(refused("claude", &|a| a.max_budget_usd = "0".into()));
    assert!(refused("claude", &|a| a.max_budget_usd = "NaN".into()));
    assert!(refused("claude", &|a| a.billing_profile.clear()));
    assert!(refused("claude", &|a| a.provider_args =
        vec!["--settings".into(), "x".into()]));
    assert!(refused("claude", &|a| {
        a.billing_mode = headless_billing::PLAN_LOGIN.into();
        a.billing_profile.clear();
        a.max_budget_usd.clear();
    }));
    // Plan login (no flags) launches as before.
    let plan = launch::prepare_launch(
        &Config::default(),
        "claude",
        WrapperArgs {
            headless: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(!plan.billing.is_api());
}

#[tokio::test]
async fn api_prepare_merges_one_owned_settings_file_without_running_the_recipe() {
    let (root, _installed, paths) = trial();
    let home = root.path().join("home");
    let cwd = root.path().join("project");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir(&cwd).unwrap();
    save_recipe(&paths);
    let billing = selected(&paths);
    let plan = api_launch(&billing);
    let registered = registered_with(Some(&billing));
    let version = fake_probe(&cwd, "2.1.330 (Claude Code)");
    let recipe = prepare(
        BillingContext {
            paths: &paths,
            home_dir: &home,
            cwd: &cwd,
            environment: &[],
            launch: &plan,
            registered: &registered,
        },
        &version,
        &Cancellation::default(),
    )
    .await
    .unwrap();
    let mut hooks = SessionHooks::default();
    let mut args = plan.provider_args.clone();
    let extra = hooks
        .claude_settings_with_recipe(
            &paths,
            &registered,
            Path::new("/synthetic/many-ai-cli"),
            49001,
            cfg!(windows),
            true,
            false,
            Some(&recipe),
        )
        .unwrap();
    args.extend(extra.args);
    finish_args(&mut args, &plan.billing).unwrap();
    assert_eq!(args.iter().filter(|v| *v == "--settings").count(), 1);
    assert_eq!(args.iter().filter(|v| *v == "--bare").count(), 1);
    let budget = args.iter().position(|v| v == "--max-budget-usd").unwrap();
    assert_eq!(args[budget + 1], "0.25");
    assert!(!args.iter().any(|arg| arg.contains(RECIPE)));
    let index = args.iter().position(|v| v == "--settings").unwrap();
    let file = PathBuf::from(&args[index + 1]);
    let settings: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(settings["apiKeyHelper"], RECIPE);
    // A second settings source would make the helper ambiguous.
    let mut doubled = args.clone();
    doubled.extend(["--settings".into(), "other".into()]);
    assert!(finish_args(&mut doubled, &plan.billing).is_err());
    hooks.finish().unwrap();
    assert!(!file.exists());
}

#[tokio::test]
async fn unconfirmed_changed_or_conflicting_api_runs_stop_before_any_probe() {
    let (root, _installed, paths) = trial();
    let home = root.path().join("home");
    let cwd = root.path().join("project");
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    std::fs::create_dir(&cwd).unwrap();
    save_recipe(&paths);
    let billing = selected(&paths);
    let plan = api_launch(&billing);
    let version = fake_probe(&cwd, "2.1.330 (Claude Code)");
    let run = |registered: Message, environment: Vec<String>| {
        let paths = paths.clone();
        let home = home.clone();
        let cwd = cwd.clone();
        let version = version.clone();
        let plan = &plan;
        async move {
            prepare(
                BillingContext {
                    paths: &paths,
                    home_dir: &home,
                    cwd: &cwd,
                    environment: &environment,
                    launch: plan,
                    registered: &registered,
                },
                &version,
                &Cancellation::default(),
            )
            .await
            .err()
            .map(|error| error.to_string())
        }
    };
    // Not confirmed by the Hub (a manual `wrap` with the same flags).
    assert_eq!(
        run(registered_with(None), vec![]).await,
        Some(BillingError::HumanConfirmationRequired.to_string())
    );
    // A competing credential in the environment.
    assert_eq!(
        run(
            registered_with(Some(&billing)),
            vec!["ANTHROPIC_API_KEY=synthetic-not-a-key".into()]
        )
        .await,
        Some(BillingError::AuthConflict.to_string())
    );
    // An unreviewed CLI version.
    let old = fake_probe(&cwd, "2.0.1 (Claude Code)");
    let registered = registered_with(Some(&billing));
    let error = prepare(
        BillingContext {
            paths: &paths,
            home_dir: &home,
            cwd: &cwd,
            environment: &[],
            launch: &plan,
            registered: &registered,
        },
        &old,
        &Cancellation::default(),
    )
    .await
    .err()
    .map(|error| error.to_string());
    assert_eq!(error, Some(BillingError::UnsupportedVersion.to_string()));
    // The private file changed after the person confirmed it.
    private_io::write_atomic(
        &paths.resource(Resource::HeadlessBilling),
        br#"{"version":1,"profiles":[{"id":"synthetic","display_name":"Synthetic","provider":"claude","command":"another-synthetic-resolver"}]}"#,
    )
    .unwrap();
    assert_eq!(
        run(registered_with(Some(&billing)), vec![]).await,
        Some(BillingError::RevisionChanged.to_string())
    );
    // None of the messages carries the recipe.
    for error in [
        BillingError::HumanConfirmationRequired,
        BillingError::AuthConflict,
        BillingError::UnsupportedVersion,
        BillingError::RevisionChanged,
    ] {
        assert!(!error.to_string().contains(RECIPE));
    }
}

#[tokio::test]
async fn probe_keeps_only_bounded_stdout_and_uses_a_fixed_failure() {
    let root = tempfile::tempdir().unwrap();
    let plan = fake_probe(root.path(), "synthetic-stdout");
    let result = probe(&plan, &Cancellation::default()).await.unwrap();
    assert_eq!(String::from_utf8_lossy(&result).trim(), "synthetic-stdout");
    let mut failed = plan;
    #[cfg(windows)]
    {
        failed.args = vec![
            "/d".into(),
            "/c".into(),
            "echo synthetic-secret 1>&2 & exit 1".into(),
        ];
    }
    #[cfg(unix)]
    {
        failed.args = vec!["-c".into(), "printf synthetic-secret >&2; exit 1".into()];
    }
    let error = probe(&failed, &Cancellation::default()).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        BillingError::UnsupportedVersion.to_string()
    );
    assert!(!error.to_string().contains("synthetic-secret"));
}
