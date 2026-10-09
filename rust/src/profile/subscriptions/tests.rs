use super::*;
fn fixture() -> (tempfile::TempDir, RuntimePaths, PathBuf, Config) {
    let root = tempfile::tempdir().unwrap();
    let installed = root.path().join("installed-unused");
    std::fs::create_dir(&installed).unwrap();
    let trial = root.path().join("trial");
    std::fs::create_dir(&trial).unwrap();
    let paths = RuntimePaths::trial(&trial, 49179, &installed).unwrap();
    let home = paths.root().join("vendor-home");
    std::fs::create_dir(&home).unwrap();
    let mut cfg = Config::defaults(&paths);
    cfg.subscriptions
        .insert("claude".into(), vec![profile("main"), profile("second")]);
    (root, paths, home, cfg)
}
fn profile(id: &str) -> config::SubscriptionProfile {
    config::SubscriptionProfile {
        id: id.into(),
        ..Default::default()
    }
}
#[test]
fn no_profile_does_not_read_or_modify_vendor_settings_or_environment() {
    let (_root, paths, home, cfg) = fixture();
    let launcher = SubscriptionLauncher::new(paths.clone(), home, paths.root().into());
    for value in ["", "  "] {
        let (env, notice) = launcher
            .launch(
                &cfg,
                "claude",
                value,
                &["CLAUDE_CONFIG_DIR=synthetic-inherited".into()],
            )
            .unwrap();
        assert!(env.is_empty());
        assert!(notice.is_none());
    }
    assert!(!paths.resource(Resource::Subscriptions).exists());
}
#[test]
fn auto_is_ordered_once_and_invalid_profiles_never_fall_back() {
    let (_root, paths, home, mut cfg) = fixture();
    let mut disabled = profile("disabled");
    disabled.enabled = Some(false);
    cfg.subscriptions.get_mut("claude").unwrap().extend([
        profile("main"),
        disabled,
        profile("../bad"),
    ]);
    assert_eq!(selectable(&cfg, "claude"), ["main", "second"]);
    let launcher = SubscriptionLauncher::new(paths.clone(), home, paths.root().into());
    for expected in ["main", "second", "main"] {
        let (env, _) = launcher.launch(&cfg, "claude", " AUTO ", &[]).unwrap();
        assert_eq!(env[1], format!("MANY_AI_CLI_SUBSCRIPTION_ID={expected}"));
    }
    for missing in ["absent", "disabled", "../bad"] {
        assert!(launcher.launch(&cfg, "claude", missing, &[]).is_err());
    }
    cfg.subscriptions.clear();
    assert!(launcher.launch(&cfg, "claude", "auto", &[]).is_err());
    assert!(launcher.launch(&cfg, "copilot", "main", &[]).is_err());
}
#[test]
fn failed_selected_directory_does_not_advance_again_or_use_another_account() {
    let (_root, paths, home, cfg) = fixture();
    let provider = paths.resource(Resource::Subscriptions).join("claude");
    std::fs::create_dir_all(&provider).unwrap();
    std::fs::write(provider.join("main"), "synthetic blocking file").unwrap();
    let launcher = SubscriptionLauncher::new(paths.clone(), home, paths.root().into());
    assert!(launcher.launch(&cfg, "claude", "auto", &[]).is_err());
    assert!(!provider.join("second").exists());
    assert_eq!(
        launcher.launch(&cfg, "claude", "auto", &[]).unwrap().0[1],
        "MANY_AI_CLI_SUBSCRIPTION_ID=second"
    );
}
#[test]
fn profile_seeds_only_named_settings_and_preserves_existing_state() {
    let (_root, paths, home, cfg) = fixture();
    let defaults = home.join(".claude");
    std::fs::create_dir(&defaults).unwrap();
    std::fs::write(
        defaults.join("settings.json"),
        r#"{"theme":"default","permissions":{"allow":["Read"]},"new_setting":true}"#,
    )
    .unwrap();
    std::fs::write(defaults.join("CLAUDE.md"), "synthetic rules").unwrap();
    std::fs::write(defaults.join("auth.json"), "SYNTHETIC_AUTH_DO_NOT_COPY").unwrap();
    std::fs::write(home.join(".claude.json"), r#"{"claudeInChromeDefaultEnabled":true,"hasCompletedClaudeInChromeOnboarding":true,"oauthAccount":"SYNTHETIC_AUTH_DO_NOT_COPY","large":123456789012345678901234567890}"#).unwrap();
    let launcher = SubscriptionLauncher::new(paths.clone(), home, paths.root().into());
    let (_, notice) = launcher.launch(&cfg, "claude", "main", &[]).unwrap();
    assert!(notice.unwrap().applied.contains(&"settings.json".into()));
    let destination = paths.resource(Resource::Subscriptions).join("claude/main");
    assert!(!destination.join("auth.json").exists());
    let state = std::fs::read_to_string(destination.join(".claude.json")).unwrap();
    assert!(!state.contains("SYNTHETIC_AUTH_DO_NOT_COPY"));
    std::fs::write(
        destination.join("settings.json"),
        r#"{"theme":"profile","permissions":{"allow":["Read","Write"]},"profile_only":1}"#,
    )
    .unwrap();
    launcher.launch(&cfg, "claude", "main", &[]).unwrap();
    let settings: serde_json::Value =
        serde_json::from_slice(&std::fs::read(destination.join("settings.json")).unwrap()).unwrap();
    assert_eq!(settings["theme"], "profile");
    assert_eq!(settings["profile_only"], 1);
    assert_eq!(
        settings["permissions"]["allow"],
        serde_json::json!(["Read"])
    );
}
#[test]
fn settings_sync_overrides_and_malformed_source_preserve_destination() {
    let (_root, paths, home, mut cfg) = fixture();
    let defaults = home.join(".claude");
    std::fs::create_dir(&defaults).unwrap();
    std::fs::write(
        defaults.join("settings.json"),
        r#"{"theme":"default","policy":2}"#,
    )
    .unwrap();
    let profile = &mut cfg.subscriptions.get_mut("claude").unwrap()[0];
    profile.profile_owned_keys = vec!["policy".into()];
    profile.default_wins_keys = vec!["theme".into()];
    let launcher = SubscriptionLauncher::new(paths.clone(), home, paths.root().into());
    launcher.launch(&cfg, "claude", "main", &[]).unwrap();
    let destination = paths
        .resource(Resource::Subscriptions)
        .join("claude/main/settings.json");
    std::fs::write(&destination, r#"{"theme":"profile","policy":1}"#).unwrap();
    launcher.launch(&cfg, "claude", "main", &[]).unwrap();
    let before = std::fs::read(&destination).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&before).unwrap();
    assert_eq!(value["theme"], "default");
    assert_eq!(value["policy"], 1);
    std::fs::write(defaults.join("settings.json"), "{malformed synthetic").unwrap();
    let (_, notice) = launcher.launch(&cfg, "claude", "main", &[]).unwrap();
    assert_eq!(notice.unwrap().failed, ["settings.json"]);
    assert_eq!(std::fs::read(destination).unwrap(), before);
}
#[test]
fn codex_first_seed_keeps_user_hooks_and_never_copies_auth() {
    let (_root, paths, home, mut cfg) = fixture();
    cfg.subscriptions
        .insert("codex".into(), vec![profile("main")]);
    let defaults = home.join(".codex");
    std::fs::create_dir(&defaults).unwrap();
    std::fs::write(
        defaults.join("config.toml"),
        "model = 'synthetic-model'\n[[hooks.Stop]]\ncommand = 'synthetic-user-hook'\n",
    )
    .unwrap();
    std::fs::write(defaults.join("auth.json"), "SYNTHETIC_AUTH_DO_NOT_COPY").unwrap();
    let launcher = SubscriptionLauncher::new(paths.clone(), home, paths.root().into());
    launcher.launch(&cfg, "codex", "main", &[]).unwrap();
    let destination = paths.resource(Resource::Subscriptions).join("codex/main");
    assert!(
        std::fs::read_to_string(destination.join("config.toml"))
            .unwrap()
            .contains("synthetic-user-hook")
    );
    assert!(!destination.join("auth.json").exists());
}
#[test]
fn open_code_only_changes_data_home_and_grok_seeds_policy() {
    let (_root, paths, home, mut cfg) = fixture();
    for provider in ["opencode", "grok"] {
        cfg.subscriptions
            .insert(provider.into(), vec![profile("main")]);
    }
    let defaults = home.join(".grok");
    std::fs::create_dir(&defaults).unwrap();
    std::fs::write(
        defaults.join("config.toml"),
        "model = 'synthetic-default'\n[ui]\ncolor = 'blue'\n",
    )
    .unwrap();
    std::fs::write(defaults.join("trusted_folders.toml"), "folders = []\n").unwrap();
    let launcher = SubscriptionLauncher::new(paths.clone(), home, paths.root().into());
    let (env, notice) = launcher.launch(&cfg, "opencode", "main", &[]).unwrap();
    assert!(env[0].starts_with("XDG_DATA_HOME="));
    assert!(notice.unwrap().applied.is_empty());
    launcher.launch(&cfg, "grok", "main", &[]).unwrap();
    let destination = paths.resource(Resource::Subscriptions).join("grok/main");
    assert!(destination.join("trusted_folders.toml").exists());
    std::fs::write(
        destination.join("config.toml"),
        "model = 'synthetic-old'\n[ui]\ncolor = 'red'\n",
    )
    .unwrap();
    launcher.launch(&cfg, "grok", "main", &[]).unwrap();
    let value = std::fs::read_to_string(destination.join("config.toml")).unwrap();
    assert!(value.contains("synthetic-default"));
    assert!(value.contains("red"));
}
#[cfg(unix)]
#[test]
fn synthetic_default_rule_links_track_changes_and_trial_escape_is_rejected() {
    let (_root, paths, home, cfg) = fixture();
    let defaults = home.join(".claude");
    std::fs::create_dir(&defaults).unwrap();
    let actual = home.join("synthetic-rules");
    std::fs::write(&actual, "first").unwrap();
    std::os::unix::fs::symlink(&actual, defaults.join("CLAUDE.md")).unwrap();
    std::fs::create_dir(defaults.join("skills")).unwrap();
    let launcher = SubscriptionLauncher::new(paths.clone(), home, paths.root().into());
    launcher.launch(&cfg, "claude", "main", &[]).unwrap();
    let destination = paths.resource(Resource::Subscriptions).join("claude/main");
    assert!(
        std::fs::symlink_metadata(destination.join("CLAUDE.md"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    std::fs::write(&actual, "second").unwrap();
    assert_eq!(
        std::fs::read_to_string(destination.join("CLAUDE.md")).unwrap(),
        "second"
    );
    let outside = tempfile::tempdir().unwrap();
    std::fs::remove_file(defaults.join("CLAUDE.md")).unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("do-not-read"),
        defaults.join("CLAUDE.md"),
    )
    .unwrap();
    assert!(launcher.launch(&cfg, "claude", "second", &[]).is_err());
}
#[test]
fn trial_paths_reject_parent_components_before_opening_original_spelling() {
    let (_root, paths, home, _) = fixture();
    std::fs::create_dir(home.join("inner")).unwrap();
    // PathBuf::join normalizes parent segments for Windows verbatim roots.
    // Build the original native spelling so check_path receives the traversal.
    let mut original = home.as_os_str().to_os_string();
    original.push(std::path::MAIN_SEPARATOR_STR);
    original.push(["inner", "..", "settings.json"].join(std::path::MAIN_SEPARATOR_STR));
    let traversal = PathBuf::from(original);
    assert!(
        traversal
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    );
    assert!(check_path(&paths, &traversal).is_err());
    assert!(check_path(&paths, &home.join("missing/new.json")).is_ok());
    let outside = tempfile::tempdir().unwrap();
    assert!(check_path(&paths, &outside.path().join("missing/new.json")).is_err());
}
#[cfg(unix)]
#[test]
fn trial_path_alias_to_owned_root_is_allowed() {
    let (root, paths, _home, _) = fixture();
    let alias = root.path().join("owned-alias");
    std::os::unix::fs::symlink(paths.root(), &alias).unwrap();
    assert!(check_path(&paths, &alias.join("vendor-home/new.json")).is_ok());
}
#[test]
fn relative_subscription_tree_override_keeps_go_classification_before_absolutizing() {
    let (_root, paths, home, cfg) = fixture();
    let source = paths
        .resource(Resource::Subscriptions)
        .join("claude/source");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(
        source.join("settings.json"),
        r#"{"policy":"synthetic-relative-profile-source"}"#,
    )
    .unwrap();
    std::fs::create_dir(home.join(".claude")).unwrap();
    std::fs::write(
        home.join(".claude/settings.json"),
        r#"{"policy":"synthetic-default-source"}"#,
    )
    .unwrap();
    let launcher = SubscriptionLauncher::new(paths.clone(), home, paths.root().into());
    let env = vec!["CLAUDE_CONFIG_DIR=subscriptions/claude/source".into()];
    launcher.launch(&cfg, "claude", "main", &env).unwrap();
    assert!(
        std::fs::read_to_string(
            paths
                .resource(Resource::Subscriptions)
                .join("claude/main/settings.json")
        )
        .unwrap()
        .contains("synthetic-relative-profile-source")
    );
}
#[test]
fn relative_filesystem_inputs_are_resolved_before_trial_root_validation() {
    let (_root, paths, home, _) = fixture();
    let hub = home.join("hub-cwd");
    std::fs::create_dir(&hub).unwrap();
    assert_eq!(
        actor_path(&paths, &hub, Path::new("../settings.json")).unwrap(),
        home.join("settings.json")
    );
    assert!(actor_path(&paths, &hub, Path::new("../../../outside.json")).is_err());
    assert_eq!(
        clean(Path::new("../../profile/config.toml")),
        Path::new("../../profile/config.toml")
    );
}
