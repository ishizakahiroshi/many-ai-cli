use super::*;
use crate::{
    config::{Config, Resource, SubscriptionProfile},
    profile::registry::{default_adapters, embedded_definitions},
    proto::{core::*, provider::Layers},
};
use std::{collections::BTreeMap, path::Path, sync::Mutex};
fn visible_native_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        PathBuf::from(
            path.to_string_lossy()
                .strip_prefix(r"\\?\")
                .unwrap_or(&path.to_string_lossy()),
        )
    }
    #[cfg(not(windows))]
    {
        path.to_path_buf()
    }
}
struct IsolatedEnvironment;
impl PathEnvironment for IsolatedEnvironment {
    fn sanitize(&self, environment: &[String]) -> io::Result<Vec<String>> {
        Ok(environment.to_vec())
    }
}
struct Fixture {
    _root: tempfile::TempDir,
    paths: RuntimePaths,
    home: PathBuf,
    config: Arc<ConfigStore>,
    policy: ConfigSpawnLaunchPolicy,
}
fn fixture(edit: impl FnOnce(&mut Config), models: LocalModelSnapshot) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let installed = root.path().join("installed");
    std::fs::create_dir(&installed).unwrap();
    let trial = root.path().join("trial");
    std::fs::create_dir(&trial).unwrap();
    let paths = RuntimePaths::trial(&trial, 49181, &installed).unwrap();
    let home = visible_native_path(paths.root()).join("vendor-home");
    std::fs::create_dir(&home).unwrap();
    let mut cfg = Config::defaults(&paths);
    edit(&mut cfg);
    let config = Arc::new(ConfigStore::new(paths.clone(), cfg).unwrap());
    let registry = Registry::build(
        Layers {
            embedded: Some(embedded_definitions().unwrap()),
            ..Default::default()
        },
        &default_adapters(),
    );
    let policy = ConfigSpawnLaunchPolicy::new(
        paths.clone(),
        config.clone(),
        SpawnPolicyDependencies {
            registry: Arc::new(move || Ok(registry.clone())),
            local_models: Arc::new(move || Ok(models.clone())),
            path_environment: Arc::new(IsolatedEnvironment),
            vendor_home: home.clone(),
            hub_cwd: visible_native_path(paths.root()),
            seeded: Arc::new(|_| {}),
            trusted: Arc::new(|_, _| {}),
        },
    )
    .unwrap();
    Fixture {
        _root: root,
        paths,
        home,
        config,
        policy,
    }
}
fn spec(root: &Path, provider: &str) -> WrappedSpawnSpec {
    WrappedSpawnSpec {
        registration_metadata: SpawnRegistrationMetadata::default(),
        spawn_attempt: None,
        registration_proof: None,
        provider: provider.into(),
        cwd: root.into(),
        model: "synthetic-model".into(),
        model_selection: String::new(),
        risk_confirmed: true,
        label: String::new(),
        permission_mode: String::new(),
        sandbox: String::new(),
        ask_for_approval: String::new(),
        route: String::new(),
        utf8_session: false,
        effort: String::new(),
        execution_mode: String::new(),
        permission_preset: String::new(),
        initial_prompt: String::new(),
        subscription_profile_id: String::new(),
        subscription_login: false,
        usage_probe: false,
        grants: InternalSpawnGrants::default(),
        cancellation: TaskCancellation::default(),
    }
}
#[test]
fn route_inference_precedence_and_official_inherited_environment() {
    let models = LocalModelSnapshot {
        ollama: ["shared".into(), "cached".into()].into(),
        lm_studio: ["shared".into()].into(),
    };
    let f = fixture(
        |cfg| {
            cfg.local_models.push(config::LocalModel {
                id: "configured".into(),
                label: String::new(),
            });
            cfg.user_prefs
                .spawn
                .last_model
                .insert("claude".into(), " previous ".into());
        },
        models,
    );
    let base = vec![
        "ANTHROPIC_API_KEY=SYNTHETIC_INHERITED".into(),
        "OTHER=exact-value".into(),
    ];
    let mut request = spec(f.paths.root(), "claude");
    for (model, expected) in [
        ("shared", "lm-studio"),
        ("cached", "ollama"),
        ("configured", "ollama"),
        ("synthetic:cloud", "ollama"),
        ("synthetic:120b-cloud", "anthropic"),
        ("ordinary", "anthropic"),
        ("", ""),
    ] {
        request.model = model.into();
        let resolved = f.policy.resolve(&request, &base).unwrap();
        assert_eq!(resolved.effective_route, expected);
        assert_eq!(resolved.current_model, "previous");
        assert_eq!(resolved.base_environment, base);
        if matches!(expected, "" | "anthropic") {
            assert!(resolved.route_environment.is_empty());
        } else {
            assert!(
                resolved
                    .route_environment
                    .contains(&"ANTHROPIC_API_KEY=".into())
            );
        }
    }
}
#[test]
fn login_ignores_inferred_and_explicit_route_configuration_after_profile_resolution() {
    let f = fixture(
        |cfg| {
            cfg.subscriptions.insert(
                "opencode".into(),
                vec![SubscriptionProfile {
                    id: "main".into(),
                    ..Default::default()
                }],
            );
        },
        LocalModelSnapshot::default(),
    );
    let mut request = spec(f.paths.root(), "opencode");
    request.subscription_login = true;
    request.subscription_profile_id = "main".into();
    request.model = "nvidia/invalid-model".into();
    request.route = "nvidia-nim".into();
    request.effort = "otherwise-invalid".into();
    let resolved = f.policy.resolve(&request, &[]).unwrap();
    assert_eq!(resolved.effective_route, "");
    assert!(resolved.route_environment.is_empty());
    assert_eq!(
        resolved.subscription_environment[1],
        "MANY_AI_CLI_SUBSCRIPTION_ID=main"
    );
    request.subscription_profile_id = "missing".into();
    assert!(f.policy.resolve(&request, &[]).is_err());
}
#[test]
fn nim_key_prefers_environment_and_runtime_config_merges_without_exposing_keys() {
    let f = fixture(
        |cfg| cfg.nvidia_nim.enabled = true,
        LocalModelSnapshot::default(),
    );
    let secrets = f.paths.root().join("secrets");
    std::fs::create_dir(&secrets).unwrap();
    std::fs::write(secrets.join("nvidia_api_key"), "SYNTHETIC_FILE_PLACEHOLDER").unwrap();
    let mut request = spec(f.paths.root(), "opencode");
    request.model = "nvidia/synthetic/model".into();
    let base = vec!["NVIDIA_API_KEY= SYNTHETIC_ENV_PLACEHOLDER ".into(), r#"OPENCODE_CONFIG_CONTENT={"provider":{"other":{"keep":true},"nvidia":{"options":{"extra":1},"models":{"old":{}}}},"keep":true}"#.into()];
    let resolved = f.policy.resolve(&request, &base).unwrap();
    assert_eq!(resolved.effective_route, "nvidia-nim");
    assert_eq!(
        resolved.route_environment[0],
        "NVIDIA_API_KEY=SYNTHETIC_ENV_PLACEHOLDER"
    );
    let content = resolved.route_environment[1]
        .strip_prefix("OPENCODE_CONFIG_CONTENT=")
        .unwrap();
    assert!(!content.contains("SYNTHETIC_ENV_PLACEHOLDER"));
    let value: serde_json::Value = serde_json::from_str(content).unwrap();
    assert_eq!(value["provider"]["other"]["keep"], true);
    assert_eq!(value["provider"]["nvidia"]["options"]["extra"], 1);
    assert!(value["provider"]["nvidia"]["models"]["old"].is_object());
    let resolved = f.policy.resolve(&request, &[]).unwrap();
    assert_eq!(
        resolved.route_environment[0],
        "NVIDIA_API_KEY=SYNTHETIC_FILE_PLACEHOLDER"
    );
    let invalid = ["NVIDIA_API_KEY=SYNTHETIC\nINVALID".into()];
    let error = f
        .policy
        .resolve(&request, &invalid)
        .err()
        .unwrap()
        .to_string();
    assert!(!error.contains("SYNTHETIC"));
    let invalid = ["OPENCODE_CONFIG_CONTENT=SYNTHETIC_INVALID_PRIVATE_TEXT".into()];
    let error = f
        .policy
        .resolve(&request, &invalid)
        .err()
        .unwrap()
        .to_string();
    assert!(!error.contains("SYNTHETIC_INVALID"));
}
#[test]
fn rejects_absent_provider_invalid_route_and_invalid_profile_without_fallback() {
    let f = fixture(
        |cfg| {
            cfg.subscriptions.insert(
                "claude".into(),
                vec![SubscriptionProfile {
                    id: "disabled".into(),
                    enabled: Some(false),
                    ..Default::default()
                }],
            );
        },
        LocalModelSnapshot::default(),
    );
    assert!(
        f.policy
            .resolve(&spec(f.paths.root(), "absent"), &[])
            .is_err()
    );
    let mut request = spec(f.paths.root(), "claude");
    request.route = "nvidia-nim".into();
    assert!(f.policy.resolve(&request, &[]).is_err());
    request.route = "unknown".into();
    assert!(f.policy.resolve(&request, &[]).is_err());
    request.route.clear();
    for id in ["disabled", "absent", "auto"] {
        request.subscription_profile_id = id.into();
        assert!(f.policy.resolve(&request, &[]).is_err());
    }
    assert!(!f.paths.resource(Resource::Subscriptions).exists());
}
#[test]
fn model_publication_uses_shared_store_and_preserves_memory_on_write_failure() {
    let f = fixture(|_| {}, LocalModelSnapshot::default());
    f.policy
        .registered_model("codex", "synthetic-selected")
        .unwrap();
    assert_eq!(
        f.config
            .snapshot()
            .unwrap()
            .config
            .user_prefs
            .spawn
            .last_model["codex"],
        "synthetic-selected"
    );
    let saved = std::fs::read_to_string(f.paths.resource(Resource::Config)).unwrap();
    assert!(saved.contains("synthetic-selected"));
    std::fs::remove_file(f.paths.resource(Resource::Config)).unwrap();
    std::fs::create_dir(f.paths.resource(Resource::Config)).unwrap();
    assert!(
        f.policy
            .registered_model("codex", "synthetic-memory-only")
            .is_err()
    );
    assert_eq!(
        f.config
            .snapshot()
            .unwrap()
            .config
            .user_prefs
            .spawn
            .last_model["codex"],
        "synthetic-memory-only"
    );
}
struct Sources {
    user: BTreeMap<String, String>,
    machine: BTreeMap<String, String>,
    dirs: BTreeSet<String>,
    reads: Mutex<Vec<String>>,
}
impl WindowsEnvironmentSources for Sources {
    fn user_value(&self, name: &str) -> Option<String> {
        self.reads.lock().unwrap().push(name.into());
        self.user.get(&name.to_ascii_uppercase()).cloned()
    }
    fn machine_value(&self, name: &str) -> Option<String> {
        self.machine.get(&name.to_ascii_uppercase()).cloned()
    }
    fn is_dir(&self, path: &str) -> bool {
        self.dirs.contains(path)
    }
}
#[test]
fn synthetic_windows_path_refresh_preserves_unknown_refs_and_recovers_missing_entries() {
    let source = Sources {
        user: [
            (
                "PATH".into(),
                r"C:\Tools\;%PNPM_HOME%\bin;%UNRESOLVED%\bin; C:\Recovered ".into(),
            ),
            ("PNPM_HOME".into(), r"C:\Pnpm".into()),
        ]
        .into(),
        machine: [("PATH".into(), r"C:\System;C:\Tools".into())].into(),
        dirs: [r"C:\Home\.local\bin".into(), r"C:\Local\pnpm".into()].into(),
        reads: Mutex::new(vec![]),
    };
    let adapter = WindowsPathEnvironment::new(source);
    let env = vec![
        r"Path=;C:\Tools;;%PNPM_HOME%\bin;%MISSING%\literal;".into(),
        r"LOCALAPPDATA=C:\Local".into(),
        r"USERPROFILE=C:\Home".into(),
        "OTHER=SYNTHETIC_EXACT".into(),
        "NO_EQUALS".into(),
    ];
    let result = adapter.sanitize(&env).unwrap();
    assert_eq!(
        result[0],
        r"Path=C:\Tools;C:\Pnpm\bin;%MISSING%\literal;C:\Recovered;C:\System;C:\Local\pnpm;C:\Home\.local\bin"
    );
    assert_eq!(result[1..], env[1..]);
}
#[test]
fn native_unix_sanitizer_only_removes_empty_path_entries() {
    #[cfg(not(windows))]
    {
        let env = vec![
            "PATH=:alpha: :beta:".into(),
            "NO_EQUALS".into(),
            "=bad".into(),
            "OTHER=SYNTHETIC_EXACT".into(),
        ];
        let result = NativePathEnvironment.sanitize(&env).unwrap();
        assert_eq!(result[0], "PATH=alpha:beta");
        assert_eq!(result[1..], env[1..]);
    }
}
#[test]
fn raw_registry_effort_mapping_and_invalid_declared_level_match_go() {
    let f = fixture(|_| {}, LocalModelSnapshot::default());
    let mut request = spec(f.paths.root(), "codex");
    request.effort = "high".into();
    let resolved = f.policy.resolve(&request, &[]).unwrap();
    assert_eq!(
        resolved.effort_args,
        ["-c", "model_reasoning_effort=\"high\""]
    );
    request.provider = "opencode".into();
    request.effort = "synthetic-freeform".into();
    assert!(
        f.policy
            .resolve(&request, &[])
            .unwrap()
            .effort_args
            .is_empty()
    );
    request.effort = "high".into();
    assert_eq!(
        f.policy.resolve(&request, &[]).unwrap().effort_args,
        ["--variant", "high"]
    );
}
#[test]
fn custom_model_suppression_and_explicit_registry_mapping() {
    let mut f = fixture(|_| {}, LocalModelSnapshot::default());
    let definition = |model_args: Vec<String>| crate::proto::provider::Definition {
        schema_version: 1,
        id: "synthetic-custom".into(),
        display_name: "Synthetic".into(),
        launch: Some(crate::proto::provider::LaunchDefinition {
            executable: "synthetic-cli".into(),
            model_args,
            effort_args: vec!["--synthetic-effort".into(), "{effort}".into()],
            effort_levels: vec!["deep".into()],
            ..Default::default()
        }),
        ..Default::default()
    };
    let registry = |model_args| {
        Registry::build(
            Layers {
                user: Some(vec![definition(model_args)]),
                ..Default::default()
            },
            &default_adapters(),
        )
    };
    let no_model = registry(vec![]);
    assert!(no_model.lookup("synthetic-custom").is_some());
    f.policy.dependencies.registry = Arc::new(move || Ok(no_model.clone()));
    let mut request = spec(f.paths.root(), "synthetic-custom");
    request.model = "synthetic:cloud".into();
    request.effort = "deep".into();
    let resolved = f.policy.resolve(&request, &[]).unwrap();
    assert!(resolved.resolved_model.is_empty());
    assert!(resolved.ordinary_model_args.is_empty());
    assert!(resolved.effective_route.is_empty());
    assert_eq!(resolved.effort_args, ["--synthetic-effort", "deep"]);
    let with_model = registry(vec!["--synthetic-model".into(), "{model}".into()]);
    f.policy.dependencies.registry = Arc::new(move || Ok(with_model.clone()));
    let resolved = f.policy.resolve(&request, &[]).unwrap();
    assert_eq!(resolved.resolved_model, "synthetic:cloud");
    assert_eq!(
        resolved.ordinary_model_args,
        ["--synthetic-model", "synthetic:cloud"]
    );
    assert!(resolved.effective_route.is_empty());
    request.subscription_profile_id = "main".into();
    assert!(f.policy.resolve(&request, &[]).is_err());
}

#[test]
fn ordinary_policy_failure_stage_distinguishes_subscription_from_route() {
    let f = fixture(|_| {}, LocalModelSnapshot::default());
    let mut request = spec(f.paths.root(), "claude");
    request.subscription_profile_id = "absent-profile".into();
    let subscription = f.policy.resolve(&request, &[]).err().unwrap();
    assert_eq!(
        failure_stage(&subscription),
        Some(PolicyFailureStage::Subscription)
    );
    request.subscription_profile_id.clear();
    request.provider = "opencode".into();
    request.route = "nvidia-nim".into();
    request.model = "nvidia/synthetic".into();
    let route = f.policy.resolve(&request, &[]).err().unwrap();
    assert_eq!(failure_stage(&route), Some(PolicyFailureStage::Route));
    let public = crate::application::ordinary_spawn::policy_error(route);
    assert_eq!(public.code, "route_error");
    assert_eq!(public.detail, "route error");
    let public = crate::application::ordinary_spawn::policy_error(subscription);
    assert_eq!(public.code, "invalid_subscription");
}
#[test]
fn folder_trust_uses_selected_profile_environment_without_advancing_auto_again() {
    let f = fixture(
        |cfg| {
            cfg.subscriptions.insert(
                "claude".into(),
                ["first", "second"]
                    .map(|id| SubscriptionProfile {
                        id: id.into(),
                        ..Default::default()
                    })
                    .to_vec(),
            );
        },
        LocalModelSnapshot::default(),
    );
    let cwd = visible_native_path(f.paths.root()).join("project");
    std::fs::create_dir(&cwd).unwrap();
    let mut request = spec(&cwd, "claude");
    request.subscription_profile_id = "auto".into();
    let first = f.policy.resolve(&request, &[]).unwrap();
    f.policy
        .folder_trust(&request, &first.subscription_environment)
        .unwrap();
    let first_config = f
        .paths
        .resource(Resource::Subscriptions)
        .join("claude/first/.claude.json");
    assert!(first_config.exists());
    assert!(
        !f.paths
            .resource(Resource::Subscriptions)
            .join("claude/second")
            .exists()
    );
    assert!(!f.home.join(".claude.json").exists());
    let second = f.policy.resolve(&request, &[]).unwrap();
    assert_eq!(
        second.subscription_environment[1],
        "MANY_AI_CLI_SUBSCRIPTION_ID=second"
    );
}
#[test]
fn claude_trust_preserves_raw_siblings_and_unknown_entries_updates_unanswered_false() {
    let f = fixture(|_| {}, LocalModelSnapshot::default());
    let cwd = visible_native_path(f.paths.root()).join("project");
    std::fs::create_dir(&cwd).unwrap();
    let key = cwd.to_string_lossy().replace('\\', "/");
    let path = f.home.join(".claude.json");
    let bytes = format!(
        r#"{{"large":123456789012345678901234567890,"synthetic_identity":"SYNTHETIC_PLACEHOLDER","projects":{{{}:{{"hasTrustDialogAccepted":false,"large":123456789012345678901234567890,"other":"<keep>&"}}}}}}"#,
        serde_json::to_string(&key).unwrap()
    );
    std::fs::write(&path, bytes).unwrap();
    let request = spec(&cwd, "claude");
    f.policy.folder_trust(&request, &[]).unwrap();
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(after.contains("123456789012345678901234567890"));
    assert!(after.contains("<keep>&"));
    assert!(after.contains("SYNTHETIC_PLACEHOLDER"));
    let first = after.clone();
    f.policy.folder_trust(&request, &[]).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), first);
    let bytes = format!(
        r#"{{"projects":{{{}:{{"unknown":true}}}}}}"#,
        serde_json::to_string(&key).unwrap()
    );
    std::fs::write(&path, &bytes).unwrap();
    f.policy.folder_trust(&request, &[]).unwrap();
    assert_eq!(std::fs::read_to_string(path).unwrap(), bytes);
}
#[test]
fn codex_trust_appends_preserving_bytes_and_prior_refusal_prevents_broader_trust() {
    let f = fixture(|_| {}, LocalModelSnapshot::default());
    let cwd = visible_native_path(f.paths.root()).join("project");
    std::fs::create_dir(&cwd).unwrap();
    let home = f.home.join(".codex");
    std::fs::create_dir(&home).unwrap();
    let path = home.join("config.toml");
    let original = "# synthetic comment\nmodel = 'synthetic-model'\n";
    std::fs::write(&path, original).unwrap();
    let request = spec(&cwd, "codex");
    f.policy.folder_trust(&request, &[]).unwrap();
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(after.starts_with(original));
    assert!(after.contains("trust_level = \"trusted\""));
    f.policy.folder_trust(&request, &[]).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), after);
    let refusal = format!(
        "[projects.'{}']\ntrust_level = 'untrusted'\n",
        f.paths.root().display()
    );
    std::fs::write(&path, &refusal).unwrap();
    f.policy.folder_trust(&request, &[]).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), refusal);
    std::fs::write(&path, "[malformed synthetic").unwrap();
    assert!(f.policy.folder_trust(&request, &[]).is_err());
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        "[malformed synthetic"
    );
}
#[test]
fn repository_and_linked_worktree_trust_share_main_repository_key() {
    let f = fixture(|_| {}, LocalModelSnapshot::default());
    let repo = visible_native_path(f.paths.root()).join("main-repo");
    let git = repo.join(".git");
    let worktree = visible_native_path(f.paths.root()).join("linked-worktree");
    let metadata = git.join("worktrees/synthetic");
    std::fs::create_dir_all(&metadata).unwrap();
    std::fs::create_dir(&worktree).unwrap();
    std::fs::write(git.join("HEAD"), "ref: refs/heads/synthetic\n").unwrap();
    std::fs::write(
        worktree.join(".git"),
        format!("gitdir: {}\n", metadata.display()),
    )
    .unwrap();
    std::fs::write(metadata.join("commondir"), "../..\n").unwrap();
    std::fs::write(
        metadata.join("gitdir"),
        format!("{}\n", worktree.join(".git").display()),
    )
    .unwrap();
    for provider in ["claude", "codex"] {
        let result = f.policy.trust.grant(provider, &[], &worktree).unwrap();
        let expected = if cfg!(windows) && provider == "claude" {
            repo.to_string_lossy().replace('\\', "/")
        } else if cfg!(windows) {
            repo.to_string_lossy().to_ascii_lowercase()
        } else {
            repo.to_string_lossy().into_owned()
        };
        assert_eq!(result.key, expected);
        assert!(result.written);
    }
}
#[test]
fn broad_home_and_unsupported_provider_trust_are_rejected_without_writes() {
    let f = fixture(|_| {}, LocalModelSnapshot::default());
    assert!(f.policy.trust.grant("claude", &[], &f.home).is_err());
    assert!(f.policy.trust.grant("grok", &[], f.paths.root()).is_err());
    assert!(!f.home.join(".claude.json").exists());
}
#[cfg(unix)]
#[test]
fn codex_trust_append_keeps_existing_file_mode_and_creates_private_file() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    for existing in [false, true] {
        let f = fixture(|_| {}, LocalModelSnapshot::default());
        let cwd = visible_native_path(f.paths.root()).join("project");
        std::fs::create_dir(&cwd).unwrap();
        let home = f.home.join(".codex");
        std::fs::create_dir(&home).unwrap();
        let path = home.join("config.toml");
        let original = "# synthetic settings\nmodel = 'synthetic-model'\n";
        let prior = if existing {
            std::fs::write(&path, original).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            Some(std::fs::metadata(&path).unwrap())
        } else {
            None
        };
        let result = f.policy.trust.grant("codex", &[], &cwd).unwrap();
        assert!(result.written);
        let after = std::fs::read_to_string(&path).unwrap();
        if existing {
            assert!(after.starts_with(original));
        }
        assert!(after.contains("trust_level = \"trusted\""));
        let metadata = std::fs::metadata(&path).unwrap();
        assert_eq!(
            metadata.permissions().mode() & 0o777,
            if existing { 0o644 } else { 0o600 },
            "Codex append changed the existing config file mode"
        );
        if let Some(prior) = prior {
            assert_eq!((metadata.dev(), metadata.ino()), (prior.dev(), prior.ino()));
        }
        assert!(!f.policy.trust.grant("codex", &[], &cwd).unwrap().written);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), after);
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            if existing { 0o644 } else { 0o600 }
        );
    }
}
#[cfg(unix)]
#[test]
fn codex_trust_preserves_existing_parent_modes_and_creates_missing_parent_privately() {
    use std::os::unix::fs::PermissionsExt;
    for existing_config in [false, true] {
        let f = fixture(|_| {}, LocalModelSnapshot::default());
        let cwd = f.paths.root().join("project");
        std::fs::create_dir(&cwd).unwrap();
        let home = f.home.join(".codex");
        std::fs::create_dir(&home).unwrap();
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o750)).unwrap();
        if existing_config {
            std::fs::write(home.join("config.toml"), "# synthetic settings\n").unwrap();
        }
        assert!(f.policy.trust.grant("codex", &[], &cwd).unwrap().written);
        assert_eq!(
            std::fs::metadata(&home).unwrap().permissions().mode() & 0o777,
            0o750
        );
        assert!(
            std::fs::read_to_string(home.join("config.toml"))
                .unwrap()
                .contains("trust_level = \"trusted\"")
        );
    }
    let f = fixture(|_| {}, LocalModelSnapshot::default());
    let cwd = f.paths.root().join("project");
    std::fs::create_dir(&cwd).unwrap();
    assert!(f.policy.trust.grant("codex", &[], &cwd).unwrap().written);
    assert_eq!(
        std::fs::metadata(f.home.join(".codex"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
}
#[cfg(unix)]
#[test]
fn claude_trust_preserves_config_symlink_and_existing_mode_within_owned_trial() {
    use std::os::unix::fs::PermissionsExt;
    let f = fixture(|_| {}, LocalModelSnapshot::default());
    let cwd = visible_native_path(f.paths.root()).join("project");
    std::fs::create_dir(&cwd).unwrap();
    let target = f.home.join("synthetic-linked-state.json");
    std::fs::write(&target, "{}").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o640)).unwrap();
    let link = f.home.join(".claude.json");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    f.policy.trust.grant("claude", &[], &cwd).unwrap();
    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        std::fs::metadata(target).unwrap().permissions().mode() & 0o777,
        0o640
    );
}
#[test]
fn invalid_model_consumes_only_one_auto_choice_before_route_key_work() {
    let f = fixture(
        |cfg| {
            cfg.subscriptions.insert(
                "claude".into(),
                ["first", "second"]
                    .map(|id| SubscriptionProfile {
                        id: id.into(),
                        ..Default::default()
                    })
                    .to_vec(),
            );
        },
        LocalModelSnapshot::default(),
    );
    let mut request = spec(f.paths.root(), "claude");
    request.subscription_profile_id = "auto".into();
    request.model = "invalid model".into();
    assert!(f.policy.resolve(&request, &[]).is_err());
    assert!(
        f.paths
            .resource(Resource::Subscriptions)
            .join("claude/first")
            .is_dir()
    );
    assert!(
        !f.paths
            .resource(Resource::Subscriptions)
            .join("claude/second")
            .exists()
    );
    request.model = "synthetic-valid".into();
    assert_eq!(
        f.policy
            .resolve(&request, &[])
            .unwrap()
            .subscription_environment[1],
        "MANY_AI_CLI_SUBSCRIPTION_ID=second"
    );
}
#[test]
fn disabled_provider_registry_is_not_overridden_by_builtin_defaults() {
    let mut f = fixture(|_| {}, LocalModelSnapshot::default());
    let registry = Registry::build(
        Layers {
            embedded: Some(embedded_definitions().unwrap()),
            overrides: Some(vec![crate::proto::provider::Definition {
                id: "claude".into(),
                enabled: Some(false),
                ..Default::default()
            }]),
            ..Default::default()
        },
        &default_adapters(),
    );
    f.policy.dependencies.registry = Arc::new(move || Ok(registry.clone()));
    assert!(
        f.policy
            .resolve(&spec(f.paths.root(), "claude"), &[])
            .is_err()
    );
}
#[test]
fn claude_stale_temp_recovery_removes_only_aged_matching_regular_files() {
    use std::time::{Duration, SystemTime};
    let f = fixture(|_| {}, LocalModelSnapshot::default());
    let stale = f.home.join(".claude.json.many-ai-cli-123-456.tmp");
    let fresh = f.home.join(".claude.json.many-ai-cli-123-789.tmp");
    let unrelated = f.home.join("synthetic-unrelated.tmp");
    for path in [&stale, &fresh, &unrelated] {
        std::fs::write(path, "SYNTHETIC_TEMP_PAYLOAD").unwrap();
    }
    std::fs::OpenOptions::new()
        .write(true)
        .open(&stale)
        .unwrap()
        .set_modified(SystemTime::now() - Duration::from_secs(120))
        .unwrap();
    assert_eq!(f.policy.trust.reclaim(&[vec![], vec![]]).unwrap(), (1, 0));
    assert!(!stale.exists());
    assert!(fresh.exists());
    assert!(unrelated.exists());
    assert_eq!(f.policy.trust.reclaim(&[vec![]]).unwrap(), (0, 0));
}
#[test]
fn model_and_route_config_are_reread_after_profile_preparation() {
    let mut f = fixture(
        |cfg| {
            cfg.subscriptions.insert(
                "claude".into(),
                vec![SubscriptionProfile {
                    id: "main".into(),
                    ..Default::default()
                }],
            );
            cfg.user_prefs
                .spawn
                .last_model
                .insert("claude".into(), "synthetic-old".into());
        },
        LocalModelSnapshot::default(),
    );
    let config = f.config.clone();
    f.policy.dependencies.seeded = Arc::new(move |_| {
        let snapshot = config.snapshot().unwrap();
        let mut next = snapshot.config;
        next.user_prefs
            .spawn
            .last_model
            .insert("claude".into(), "synthetic-new".into());
        next.ollama.base_url = "http://127.0.0.1:11435".into();
        config.persist(snapshot.revision, next).unwrap();
    });
    let mut request = spec(f.paths.root(), "claude");
    request.subscription_profile_id = "main".into();
    request.route = "ollama".into();
    let resolved = f.policy.resolve(&request, &[]).unwrap();
    assert_eq!(resolved.current_model, "synthetic-new");
    assert!(
        resolved
            .route_environment
            .contains(&"ANTHROPIC_BASE_URL=http://127.0.0.1:11435".into())
    );
}
#[test]
fn relative_vendor_environment_keeps_child_spelling_and_trust_uses_hub_actor_cwd() {
    let mut f = fixture(|_| {}, LocalModelSnapshot::default());
    let hub = visible_native_path(f.paths.root()).join("hub-cwd");
    let child = visible_native_path(f.paths.root()).join("child-cwd");
    for root in [&hub, &child] {
        std::fs::create_dir_all(root.join("relative-vendor")).unwrap();
    }
    let dependencies = SpawnPolicyDependencies {
        hub_cwd: hub.clone(),
        ..f.policy.dependencies.clone()
    };
    f.policy =
        ConfigSpawnLaunchPolicy::new(f.paths.clone(), f.config.clone(), dependencies).unwrap();
    for (provider, key, filename) in [
        ("claude", "CLAUDE_CONFIG_DIR", ".claude.json"),
        ("codex", "CODEX_HOME", "config.toml"),
    ] {
        let child_file = child.join("relative-vendor").join(filename);
        std::fs::write(&child_file, "SYNTHETIC_CHILD_FILE_UNTOUCHED").unwrap();
        let env = vec![format!("{key}=relative-vendor")];
        let request = spec(&child, provider);
        let resolved = f.policy.resolve(&request, &env).unwrap();
        assert_eq!(resolved.base_environment, env);
        assert!(resolved.subscription_environment.is_empty());
        let result = f.policy.trust.grant(provider, &env, &child).unwrap();
        assert!(result.written);
        assert_eq!(
            result.config_path,
            Path::new("relative-vendor").join(filename)
        );
        assert!(hub.join("relative-vendor").join(filename).exists());
        assert_eq!(
            std::fs::read_to_string(&child_file).unwrap(),
            "SYNTHETIC_CHILD_FILE_UNTOUCHED"
        );
    }
}
#[test]
fn relative_vendor_seed_reads_hub_cwd_and_retains_environment_text() {
    let mut f = fixture(
        |cfg| {
            cfg.subscriptions.insert(
                "claude".into(),
                vec![SubscriptionProfile {
                    id: "main".into(),
                    ..Default::default()
                }],
            );
        },
        LocalModelSnapshot::default(),
    );
    let hub = visible_native_path(f.paths.root()).join("hub-cwd");
    let child = visible_native_path(f.paths.root()).join("child-cwd");
    for (root, value) in [
        (&hub, "synthetic-hub-policy"),
        (&child, "synthetic-child-policy"),
    ] {
        std::fs::create_dir_all(root.join("relative-vendor")).unwrap();
        std::fs::write(
            root.join("relative-vendor/settings.json"),
            format!("{{\"policy\":\"{value}\"}}"),
        )
        .unwrap();
    }
    let dependencies = SpawnPolicyDependencies {
        hub_cwd: hub,
        ..f.policy.dependencies.clone()
    };
    f.policy =
        ConfigSpawnLaunchPolicy::new(f.paths.clone(), f.config.clone(), dependencies).unwrap();
    let env = vec!["CLAUDE_CONFIG_DIR=relative-vendor".into()];
    let mut request = spec(&child, "claude");
    request.subscription_profile_id = "main".into();
    let resolved = f.policy.resolve(&request, &env).unwrap();
    assert_eq!(resolved.base_environment, env);
    let destination = f
        .paths
        .resource(Resource::Subscriptions)
        .join("claude/main/settings.json");
    assert!(
        std::fs::read_to_string(destination)
            .unwrap()
            .contains("synthetic-hub-policy")
    );
    assert!(
        std::fs::read_to_string(child.join("relative-vendor/settings.json"))
            .unwrap()
            .contains("synthetic-child-policy")
    );
}
