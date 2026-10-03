use many_ai_cli::config::*;
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path};

fn paths() -> RuntimePaths {
    RuntimePaths::production(Path::new(if cfg!(windows) {
        "C:\\synthetic-home"
    } else {
        "/synthetic-home"
    }))
    .unwrap()
}
fn trial() -> (tempfile::TempDir, tempfile::TempDir, RuntimePaths) {
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49199, installed.path()).unwrap();
    (root, installed, paths)
}

#[test]
fn every_source_field_matches_go_public_projection() {
    let config: Config = serde_saphyr::from_str(include_str!(
        "fixtures/foundation/config/all-fields.private.yaml"
    ))
    .unwrap();
    let expected: Value = serde_json::from_str(include_str!(
        "fixtures/foundation/config/all-fields.public.json"
    ))
    .unwrap();
    assert_eq!(config.public_json(), expected);
    let roundtrip: Config = serde_saphyr::from_str(&config.to_private_yaml().unwrap()).unwrap();
    assert_eq!(config, roundtrip);
}

#[test]
fn go_tolerant_optional_sections_keep_credentials_and_valid_neighbors() {
    let config = Config::from_yaml(
        include_str!("fixtures/foundation/config/tolerant.yaml"),
        &paths(),
    )
    .unwrap();
    assert_eq!(config.token, "synthetic-stable-token");
    assert_eq!(config.auth_cookie_secret, "synthetic-cookie-key");
    assert_eq!(config.remote_pin_hash, "synthetic-pin-hash");
    assert!(!config.hub.open_browser);
    assert!(!config.workflow.journal_enabled);
    assert!(config.workflow.task_detail_enabled);
    assert_eq!(config.user_prefs.session_order, [10, 5, 7, -1]);
    assert_eq!(config.user_prefs.quick_cmds.show1, Some(false));
    let profile = &config.subscriptions["claude"][0];
    assert_eq!(profile.id, "work");
    assert!(!profile.is_enabled());
    assert!(!profile.is_settings_sync_enabled());
    assert!(profile.profile_owned_keys.is_empty());
    assert_eq!(profile.default_wins_keys, ["theme", "7"]);
    assert_eq!(
        config.subscriptions["claude"][1].profile_owned_keys,
        ["plugins", "theme"]
    );
    assert_eq!(config.subscriptions["future-provider"][0].id, "future");
    assert!(!config.subscriptions.contains_key("malformed"));
    assert_eq!(config.custom_providers.len(), 3);
    assert_eq!(
        effective_custom_providers(&config.custom_providers).len(),
        1
    );
    assert_eq!(config.user_prefs.spawn.last_model["claude"], "new-model");
    assert_eq!(config.user_prefs.spawn.last_model["codex"], "legacy-model");
    assert!(config.spawn.last_model.is_empty());
    let expected: Value = serde_json::from_str(include_str!(
        "fixtures/foundation/config/tolerant.public.json"
    ))
    .unwrap();
    for section in ["subscriptions", "custom_providers"] {
        assert_eq!(config.public_json()[section], expected[section]);
    }
}

#[test]
fn optional_sections_of_wrong_shape_do_not_reset_other_settings() {
    for body in [
        "subscriptions: nope",
        "custom_providers: {}",
        "user_prefs: {session_order: nope}",
    ] {
        let text = format!("token: stable-synthetic\nhub: {{port: 49999}}\n{body}\n");
        let config = Config::from_yaml(&text, &paths()).unwrap();
        assert_eq!(config.token, "stable-synthetic");
        assert_eq!(config.hub.port, 49999);
    }
}

#[test]
fn all_private_credentials_are_absent_from_public_serialization() {
    let config = Config::from_yaml(
        include_str!("fixtures/foundation/config/tolerant.yaml"),
        &paths(),
    )
    .unwrap();
    let public = config.public_json().to_string();
    let regular = serde_json::to_string(&config).unwrap();
    let debug = format!("{config:?}");
    for value in [
        "synthetic-stable-token",
        "synthetic-cookie-key",
        "synthetic-pin-hash",
    ] {
        assert!(!public.contains(value));
        assert!(!regular.contains(value));
        assert!(!debug.contains(value));
        assert!(config.to_private_yaml().unwrap().contains(value));
    }
}

#[test]
fn defaults_false_values_and_go_empty_filter_omission_are_preserved() {
    let defaults = Config::defaults(&paths());
    assert_eq!(defaults.hub.port, 47777);
    assert!(defaults.hub.open_browser);
    assert!(!defaults.log.session_enabled);
    assert_eq!(defaults.orchestration.max_children_per_parent, 10);
    assert_eq!(defaults.orchestration.max_total_sessions, 257);
    assert!(defaults.orchestration.worktree_enabled());
    assert!(defaults.orchestration.child_full_bypass_enabled());
    assert_eq!(defaults.handoff.retention_days, 14);
    assert_eq!(defaults.voice.whisper.model, "small");
    assert_eq!(
        defaults
            .voice
            .whisper
            .hallucination_phrases
            .as_ref()
            .unwrap()
            .len(),
        10
    );
    let config = Config::from_yaml("voice: {whisper: {hallucination_phrases: []}}\norchestration: {worktree_auto: false, child_full_bypass: false, max_children_per_parent: 999}\nhandoff: {enabled: false}\n", &paths()).unwrap();
    assert!(!config.orchestration.worktree_enabled());
    assert!(!config.orchestration.child_full_bypass_enabled());
    assert!(!config.handoff.enabled_or_default());
    assert_eq!(config.orchestration.max_children_per_parent, 256);
    let restored = Config::from_yaml(&config.to_private_yaml().unwrap(), &paths()).unwrap();
    assert_eq!(
        restored
            .voice
            .whisper
            .hallucination_phrases
            .as_ref()
            .unwrap()
            .len(),
        10
    );
    assert_eq!(restored.orchestration.worktree_auto, Some(false));
    assert_eq!(restored.handoff.enabled, Some(false));
}

#[test]
fn source_normalization_keeps_optional_invalid_values_for_warnings() {
    let config = Config::from_yaml("hub: {terminal_color: junk}\nhandoff: {intent_mode: unknown, note_on_threshold: typo}\nuser_prefs: {usage_probe_model: 'bad;model'}\nslash_cmd_sources: {claude: 'https://code.claude.com/docs/en/commands.md'}\norchestration: {child_permission_default: typo, child_execution_mode: bad}\n", &paths()).unwrap();
    assert_eq!(config.hub.terminal_color, "force");
    assert_eq!(config.handoff.intent_mode, "done-only");
    assert_eq!(config.handoff.note_on_threshold, "typo");
    assert_eq!(config.handoff.note_on_threshold_or_default(), "ask");
    assert_eq!(
        config.user_prefs.usage_probe_model,
        DEFAULT_USAGE_PROBE_MODEL
    );
    assert_eq!(
        config.slash_cmd_sources.claude,
        SlashCmdSources::defaults().claude
    );
    assert_eq!(config.warnings().len(), 3);
    assert!(config.validate().is_ok());
}

#[test]
fn validation_separates_optional_warnings_and_fatal_network_settings() {
    for raw in [
        "0.0.0.0/0",
        "::/0",
        "not-a-cidr",
        "203.0.113.0/8",
        "fd00::/48",
        "203.0.113.7/33",
    ] {
        let mut config = Config::defaults(&paths());
        config.hub.trusted_networks = vec![raw.into()];
        assert!(config.validate().is_err(), "{raw}");
    }
    for raw in [
        "198.51.100.0/24",
        "203.0.113.7/32",
        "fd00::/64",
        "fe80::1/128",
    ] {
        let mut config = Config::defaults(&paths());
        config.hub.trusted_networks = vec![raw.into()];
        assert!(config.validate().is_ok(), "{raw}");
    }
    for raw in [
        "",
        "*",
        "192.0.2.1:47801",
        "http://192.0.2.1",
        "bad/host",
        "[::1]:80",
    ] {
        let mut config = Config::defaults(&paths());
        config.hub.allowed_hosts = vec![raw.into()];
        assert!(config.validate().is_err(), "{raw}");
    }
    for raw in [
        "localhost",
        "synthetic.example.",
        "[::1]",
        "::1",
        "192.0.2.1",
    ] {
        let mut config = Config::defaults(&paths());
        config.hub.allowed_hosts = vec![raw.into()];
        assert!(config.validate().is_ok(), "{raw}");
    }
    for raw in [
        "ftp://127.0.0.1:11434",
        "http://127.0.0.1:11434/v1",
        "http://user:pass@127.0.0.1:11434",
        "http://127.0.0.1:11434?x=1",
    ] {
        let mut config = Config::defaults(&paths());
        config.ollama.base_url = raw.into();
        assert!(config.validate().is_err(), "{raw}");
    }
    let mut config = Config::defaults(&paths());
    config.ollama.base_url = "http://localhost:11434".into();
    assert!(config.validate().is_ok());
    assert_eq!(config.warnings().len(), 1);
    config.ollama.allow_private_hosts = true;
    assert!(config.warnings().is_empty());
    config.voice.whisper.server_url = "https://synthetic.example".into();
    assert!(config.validate().is_err());
    config.voice.whisper.server_url = "http://127.0.0.1:8178".into();
    assert!(config.validate().is_ok());
}

#[test]
fn clone_is_detached_at_every_nested_container_and_optional_boolean() {
    let original = Config::from_yaml(
        include_str!("fixtures/foundation/config/tolerant.yaml"),
        &paths(),
    )
    .unwrap();
    let mut copy = original.clone();
    copy.subscriptions.get_mut("claude").unwrap()[0].enabled = Some(true);
    copy.subscriptions.get_mut("claude").unwrap()[0].default_wins_keys[0] = "changed".into();
    copy.user_prefs
        .spawn
        .last_model
        .insert("claude".into(), "changed".into());
    copy.user_prefs.quick_cmds.show1 = Some(true);
    copy.custom_providers[0].headless.as_mut().unwrap().args[0] = "changed".into();
    assert!(!original.subscriptions["claude"][0].is_enabled());
    assert_eq!(
        original.subscriptions["claude"][0].default_wins_keys[0],
        "theme"
    );
    assert_eq!(original.user_prefs.spawn.last_model["claude"], "new-model");
    assert_eq!(original.user_prefs.quick_cmds.show1, Some(false));
    assert_eq!(
        original.custom_providers[0].headless.as_ref().unwrap().args[0],
        "--print"
    );
}

#[test]
fn guarded_store_publishes_only_successful_writes_and_rejects_stale_snapshots() {
    let (_root, _installed, paths) = trial();
    let path = paths.resource(Resource::Config);
    let store = ConfigStore::load_or_create(paths, || Ok("synthetic-token".into())).unwrap();
    let a = store.snapshot().unwrap();
    let b = store.snapshot().unwrap();
    let mut next = a.config;
    next.user_prefs.display_name = "saved".into();
    let saved = store.persist(a.revision, next).unwrap();
    assert_eq!(saved.revision, 1);
    assert!(matches!(
        store.persist(b.revision, b.config),
        Err(ConfigError::Conflict { .. })
    ));
    assert_eq!(
        store.snapshot().unwrap().config.user_prefs.display_name,
        "saved"
    );
    assert!(std::fs::read_to_string(&path).unwrap().contains("saved"));
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    let mut failed = saved.config;
    failed.user_prefs.display_name = "must-not-publish".into();
    assert!(store.persist(1, failed).is_err());
    assert_eq!(store.snapshot().unwrap().revision, 1);
    assert_eq!(
        store.snapshot().unwrap().config.user_prefs.display_name,
        "saved"
    );
}

#[test]
fn invalid_yaml_is_privately_backed_up_and_recovered_but_errors_are_redacted() {
    let (_root, _installed, paths) = trial();
    let path = paths.resource(Resource::Config);
    let backup = paths.root().join("config.yaml.bak");
    let original = "token: synthetic-private-marker\nhub: [unterminated";
    std::fs::write(&path, original).unwrap();
    let error = Config::from_yaml(original, &paths).unwrap_err();
    assert!(!error.to_string().contains("synthetic-private-marker"));
    let store = ConfigStore::load_or_create(paths, || Ok("synthetic-replacement".into())).unwrap();
    assert!(store.recovered_from_invalid_yaml());
    assert_eq!(
        store.snapshot().unwrap().config.token,
        "synthetic-replacement"
    );
    assert_eq!(std::fs::read_to_string(backup).unwrap(), original);
    assert!(
        std::fs::read_to_string(path)
            .unwrap()
            .contains("synthetic-replacement")
    );
}

#[test]
fn trial_rebinds_runtime_paths_and_refuses_external_profile_locations() {
    let (_root, _installed, paths) = trial();
    let config = Config::from_yaml("hub: {port: 47777, log_dir: /synthetic-production/logs}\norchestration: {worktree_dir_root: /synthetic-production/worktrees}\n", &paths).unwrap();
    assert_eq!(config.hub.port, 49199);
    assert!(Path::new(&config.hub.log_dir).starts_with(paths.root()));
    assert!(Path::new(&config.orchestration.worktree_dir_root).starts_with(paths.root()));
    assert!(
        Config::from_yaml(
            "subscriptions: {claude: [{id: work, profile_dir: '~/private'}]}",
            &paths
        )
        .is_err()
    );
    let profile = SubscriptionProfile {
        id: "work".into(),
        dir: "short".into(),
        ..Default::default()
    };
    assert_eq!(
        resolve_subscription_profile_dir(&paths, "claude", &profile, None).unwrap(),
        paths
            .resource(Resource::Subscriptions)
            .join("claude")
            .join("short")
    );
    assert!(resolve_subscription_profile_dir(&paths, "../escape", &profile, None).is_err());
}

#[test]
fn direct_json_session_order_keeps_legacy_numeric_strings() {
    let prefs: UserPrefs =
        serde_json::from_value(json!({"session_order": [11, 2, "7", "x", {}, 1.5]})).unwrap();
    assert_eq!(prefs.session_order, [11, 2, 7]);
}

#[test]
fn subscription_and_custom_identifier_contracts_remain_distinct() {
    for bad in ["auto", "a..b", "../bad", "Bad", "", "a/b"] {
        assert!(validate_subscription_id(bad).is_err());
    }
    assert!(validate_subscription_dir_name("auto").is_ok());
    assert!(validate_custom_provider_id("a..b").is_ok()); // source does not use custom IDs as directory names
    for reserved in BUILTIN_PROVIDER_IDS.iter().copied().chain(["shell"]) {
        assert!(validate_custom_provider_id(reserved).is_err());
    }
    let profile = SubscriptionProfile {
        id: " WORK ".into(),
        ..Default::default()
    };
    let profiles = BTreeMap::from([("claude".into(), vec![profile])]);
    assert!(find_subscription(&profiles, "claude", "work").is_some());
}

#[test]
fn loading_tolerated_sections_never_regenerates_or_rewrites_the_file() {
    let (_root, _installed, paths) = trial();
    let path = paths.resource(Resource::Config);
    let original = include_str!("fixtures/foundation/config/tolerant.yaml");
    std::fs::write(&path, original).unwrap();
    let store =
        ConfigStore::load_or_create(paths, || panic!("existing token must not be regenerated"))
            .unwrap();
    assert!(!store.recovered_from_invalid_yaml());
    assert_eq!(
        store.snapshot().unwrap().config.token,
        "synthetic-stable-token"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    assert!(!path.with_extension("yaml.bak").exists());
}

#[test]
fn concurrent_writers_cannot_publish_a_stale_snapshot() {
    let (_root, _installed, paths) = trial();
    let store = ConfigStore::load_or_create(paths, || Ok("synthetic-token".into())).unwrap();
    let snapshots = [store.snapshot().unwrap(), store.snapshot().unwrap()];
    let gate = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let jobs: Vec<_> = snapshots
            .into_iter()
            .enumerate()
            .map(|(i, mut s)| {
                let store = &store;
                let gate = &gate;
                scope.spawn(move || {
                    s.config.user_prefs.display_name = format!("writer-{i}");
                    gate.wait();
                    store.persist(s.revision, s.config)
                })
            })
            .collect();
        let results: Vec<_> = jobs.into_iter().map(|j| j.join().unwrap()).collect();
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|r| matches!(r, Err(ConfigError::Conflict { .. })))
                .count(),
            1
        );
    });
    assert_eq!(store.snapshot().unwrap().revision, 1);
}

#[test]
fn validation_and_backup_failures_preserve_existing_bytes() {
    let (_root, _installed, paths) = trial();
    let path = paths.resource(Resource::Config);
    let original = "token: synthetic-unchanged\nhub: {trusted_networks: ['0.0.0.0/0']}\n";
    std::fs::write(&path, original).unwrap();
    assert!(
        ConfigStore::load_or_create(paths.clone(), || panic!(
            "must not regenerate a validation failure"
        ))
        .is_err()
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    let broken = "token: synthetic-unchanged\nhub: [broken";
    std::fs::write(&path, broken).unwrap();
    std::fs::create_dir(paths.root().join("config.yaml.bak")).unwrap();
    assert!(
        ConfigStore::load_or_create(paths, || panic!(
            "failed backup must stop before generating a token"
        ))
        .is_err()
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), broken);
}

#[test]
fn production_legacy_migration_uses_only_the_supplied_synthetic_home() {
    let home = tempfile::tempdir().unwrap();
    let legacy = home.path().join(".any-ai-cli");
    std::fs::create_dir(&legacy).unwrap();
    std::fs::write(
        legacy.join("config.yaml"),
        "token: synthetic-legacy-token\n",
    )
    .unwrap();
    let paths = RuntimePaths::production(home.path()).unwrap();
    let store =
        ConfigStore::load_or_create(paths.clone(), || panic!("legacy token must survive")).unwrap();
    assert_eq!(
        store.snapshot().unwrap().config.token,
        "synthetic-legacy-token"
    );
    assert!(!legacy.exists());
    assert!(paths.resource(Resource::Config).exists());
}

#[test]
fn terminal_color_and_raw_url_validation_match_go_normalization() {
    assert_eq!(normalize_terminal_color("  OFF  "), "off");
    assert_eq!(normalize_terminal_color("  Inherit  "), "inherit");
    for raw in [
        "http:localhost",
        "http://localhost/..",
        "http://localhost/.",
        "http://local\nhost",
    ] {
        let mut config = Config::defaults(&paths());
        config.ollama.base_url = raw.into();
        assert!(config.validate().is_err(), "{raw}");
    }
    for raw in ["http://127.1", "http://0x7f000001"] {
        let mut config = Config::defaults(&paths());
        config.voice.whisper.server_url = raw.into();
        assert!(config.validate().is_err(), "{raw}");
    }
    let mut config = Config::defaults(&paths());
    config.hub.allowed_hosts = vec!["synthetic.example..".into()];
    assert!(config.validate().is_err());
}

#[test]
fn historical_reads_and_new_write_validation_are_separate() {
    let (_root, _installed, paths) = trial();
    let file = paths.resource(Resource::Config);
    let historical = "token: synthetic-historical-token\nuser_prefs:\n  session_order: [7, '11', legacy-bad]\nfuture_optional:\n  version: 17\n  setting: synthetic\n";
    std::fs::write(&file, historical).unwrap();
    let store =
        ConfigStore::load_or_create(paths.clone(), || panic!("historical token must survive"))
            .unwrap();
    let snapshot = store.snapshot().unwrap();
    assert_eq!(snapshot.config.user_prefs.session_order, [7, 11]);
    assert_eq!(snapshot.config.token, "synthetic-historical-token");
    assert!(!store.recovered_from_invalid_yaml());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), historical);
    let mut invalid = snapshot.config.clone();
    invalid.hub.allowed_hosts = vec!["-invalid.example".into()];
    assert!(store.persist(snapshot.revision, invalid).is_err());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), historical);
    let mut next = snapshot.config;
    next.user_prefs.display_name = "synthetic update".into();
    store.persist(snapshot.revision, next).unwrap();
    let reloaded = Config::from_yaml(&std::fs::read_to_string(file).unwrap(), &paths).unwrap();
    assert_eq!(reloaded.user_prefs.session_order, [7, 11]);
    assert_eq!(reloaded.token, "synthetic-historical-token");
    // Unknown YAML keys are ignored by Go's typed Config; preserving source bytes
    // until an explicit write is required, inventing persisted fields is not.
}
