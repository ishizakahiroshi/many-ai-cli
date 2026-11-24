use super::seed::*;
use super::{registry, validation};
const DEFAULT: &str = r#"model = "gpt-5.1-codex" # synthetic-inline-note
# synthetic-leading-note
approval_policy = "on-request"
[projects."/synthetic/project"]
trust_level = "trusted"
[mcp_servers.demo]
command = "custom-tool"
http_headers = { Authorization = "synthetic-private" }
args = ["--old", "日本語"]
[[hooks.Stop]]
command = "echo user-hook"
[[hooks.Stop]]
command = "MANY_AI_CLI_HUB_TOKEN=synthetic '/synthetic/many ai cli' usage-relay --provider codex --hub http://127.0.0.1:48888 --session 1"
[[hooks.SessionStart]]
command = "echo user-start"
"#;
#[test]
fn profile_absent_codex_seeds_only_validated_scalars_and_discards_source_comments() {
    let result = merge_codex(DEFAULT, None, &SyncOptions::default()).unwrap();
    assert!(result.created);
    assert_eq!(result.removed_temporary_hooks, 1);
    assert_eq!(
        result.text,
        "model = \"gpt-5.1-codex\"\napproval_policy = \"on-request\"\n"
    );
}
#[test]
fn profile_existing_codex_owned_state_hooks_and_profile_only_keys_survive() {
    let existing = "model = 'profile-model'\nprofile_only = 'keep'\napproval_policy = 'never'\n[[hooks.Stop]]\ncommand = 'echo profile-hook'\n";
    let result = merge_codex(DEFAULT, Some(existing), &SyncOptions::default()).unwrap();
    assert!(!result.created);
    assert!(result.text.contains("profile-model"));
    assert!(result.text.contains("profile-hook"));
    assert!(!result.text.contains("user-hook"));
    assert!(!result.text.contains("gpt-5.1-codex"));
    assert!(result.text.contains("on-request"));
    assert!(result.text.contains("profile_only"));
}
#[test]
fn profile_default_wins_cannot_authorize_freeform_commands() {
    let opts = SyncOptions {
        default_wins_keys: vec!["hooks".into()],
        ..Default::default()
    };
    let result = merge_codex(
        DEFAULT,
        Some("[[hooks.Stop]]\ncommand = 'echo profile-hook'\n"),
        &opts,
    )
    .unwrap();
    assert!(!result.text.contains("user-hook"));
    assert!(result.text.contains("profile-hook"));
    assert!(!result.text.contains("MANY_AI_CLI_HUB_TOKEN"));
}
#[test]
fn profile_keeps_its_own_mcp_servers() {
    let result = merge_codex(
        "[mcp_servers.demo]\ncommand='new'\n",
        Some("[mcp_servers.demo]\ncommand='old'\nobsolete='remove'\n[profile_only]\nkeep=true\n"),
        &SyncOptions::default(),
    )
    .unwrap();
    assert!(!result.text.contains("new"));
    assert!(result.text.contains("obsolete"));
    assert!(result.text.contains("profile_only"));
}
#[test]
fn profile_cannot_silently_drop_sandbox_constraints() {
    let options = SyncOptions::default();
    let policy =
        "sandbox_mode='workspace-write'\n[sandbox_workspace_write]\nnetwork_access=false\n";
    assert!(matches!(
        merge_codex(policy, None, &options),
        Err(SeedError::LocalPolicyRequired)
    ));
    assert!(matches!(
        merge_codex(policy, Some("sandbox_mode='workspace-write'"), &options),
        Err(SeedError::LocalPolicyRequired)
    ));
    let existing = "[sandbox_workspace_write]\nnetwork_access = false # profile-local\n";
    let result = merge_codex(policy, Some(existing), &options).unwrap();
    assert!(result.text.contains("workspace-write"));
    assert!(result.text.contains("# profile-local"));
    let normalized = merge_codex(
        "[windows]\nsandbox='elevated' # private-comment\n",
        None,
        &options,
    )
    .unwrap();
    assert!(normalized.text.contains("elevated"));
    assert!(!normalized.text.contains("private-comment"));
    let existing = "windows = {sandbox='unelevated', local_only='synthetic-local'}";
    let default_windows = SyncOptions {
        default_wins_keys: vec!["windows".into()],
        ..Default::default()
    };
    let merged = merge_codex(&normalized.text, Some(existing), &default_windows).unwrap();
    assert!(merged.text.contains("synthetic-local"));
    assert!(!merged.text.contains("unelevated"));
    let sync_off = SyncOptions {
        enabled: false,
        ..options
    };
    let unchanged = merge_codex(&normalized.text, Some(&merged.text), &sync_off).unwrap();
    assert_eq!(unchanged.text, merged.text);
}
#[test]
fn profile_comments_and_key_order_do_not_cause_rewrite() {
    let existing =
        "# profile comments\nhide_agent_reasoning = false\napproval_policy = 'on-request'\n";
    let result = merge_codex(
        "approval_policy = 'on-request'\nhide_agent_reasoning=false\n",
        Some(existing),
        &SyncOptions::default(),
    )
    .unwrap();
    assert!(result.changed_keys.is_empty());
    assert_eq!(result.text, existing);
}
#[test]
fn profile_invalid_documents_fail_without_echoing_contents() {
    let secret = "password = 'SYNTHETIC_SECRET\n";
    let error = merge_codex(secret, None, &SyncOptions::default())
        .err()
        .unwrap();
    assert_eq!(error, SeedError::MalformedDefault);
    assert!(!error.to_string().contains("SYNTHETIC_SECRET"));
    assert!(matches!(
        merge_codex(DEFAULT, Some(secret), &SyncOptions::default()),
        Err(SeedError::MalformedProfile)
    ));
}
#[test]
fn profile_sync_off_existing_is_unchanged_and_absent_is_safe() {
    let options = SyncOptions {
        enabled: false,
        ..Default::default()
    };
    let existing = "model = 'owned'\napproval_policy = 'on-request'\n";
    assert_eq!(
        merge_codex(DEFAULT, Some(existing), &options).unwrap().text,
        existing
    );
    let new = merge_codex(DEFAULT, None, &options).unwrap();
    assert!(new.text.contains("gpt-5.1-codex"));
    assert!(!new.text.contains("user-hook"));
    assert!(!new.text.contains("MANY_AI_CLI_HUB_TOKEN"));
}
#[test]
fn profile_omits_the_owned_hook_in_its_current_form_without_a_token() {
    // The hook no longer carries the token (U03); it is still the Hub's own
    // temporary hook and is not copied into a profile.
    let text = "[[hooks.Stop]]\ncommand = \"echo user-hook\"\n[[hooks.Stop]]\ncommand = \"'/synthetic/many ai cli' usage-relay --provider codex --hub http://127.0.0.1:48888 --session 1\"\n";
    let result = merge_codex(text, None, &SyncOptions::default()).unwrap();
    assert_eq!(result.removed_temporary_hooks, 1);
    assert!(!result.text.contains("user-hook"));
    assert!(!result.text.contains("usage-relay"));
    // A user's own command is not counted as a generated hook.
    let user = "[[hooks.Stop]]\ncommand = \"/usr/bin/many-ai-cli usage-relay --provider codex --hub http://127.0.0.1:48888 --session 1\"\n";
    assert_eq!(
        merge_codex(user, None, &SyncOptions::default())
            .unwrap()
            .removed_temporary_hooks,
        0
    );
}
#[test]
fn profile_similar_user_commands_are_neither_copied_nor_counted_as_generated() {
    let text = "[[hooks.Stop]]\ncommand = \"echo MANY_AI_CLI_HUB_TOKEN=user usage-relay --provider codex\"\n";
    let result = merge_codex(text, None, &SyncOptions::default()).unwrap();
    assert_eq!(result.removed_temporary_hooks, 0);
    assert!(result.text.is_empty());
}

#[test]
fn provider_registry_matches_fixed_go_layers_hashes_and_diagnostics() {
    let corpus: serde_json::Value =
        serde_json::from_str(include_str!("../../tests/fixtures/services/registry.json")).unwrap();
    for case in corpus["registry"].as_array().unwrap() {
        let layers: crate::proto::provider::Layers =
            serde_json::from_value(case["layers"].clone()).unwrap();
        let registry = registry::Registry::build(layers, &registry::default_adapters());
        assert_eq!(
            registry.revision(),
            case["revision"].as_str().unwrap(),
            "{}",
            case["name"]
        );
        assert_eq!(
            serde_json::to_value(registry.list()).unwrap(),
            case["list"],
            "{} list",
            case["name"]
        );
        let definitions: Vec<_> = registry
            .list()
            .iter()
            .map(|s| registry.lookup(&s.id).unwrap())
            .collect();
        assert_eq!(
            serde_json::to_value(definitions).unwrap(),
            case["definitions"],
            "{} definitions",
            case["name"]
        );
        let diagnostics = registry.diagnostics();
        let actual = if diagnostics.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::to_value(diagnostics).unwrap()
        };
        assert_eq!(actual, case["diagnostics"], "{} diagnostics", case["name"]);
    }
}
#[test]
fn provider_new_input_validation_matches_go_including_ignored_extreme_numbers() {
    let corpus: serde_json::Value =
        serde_json::from_str(include_str!("../../tests/fixtures/services/registry.json")).unwrap();
    for case in corpus["validation"].as_array().unwrap() {
        let result = validation::validate_definition(
            case["raw"].as_str().unwrap().as_bytes(),
            &registry::default_adapters(),
        );
        assert_eq!(
            result.is_err(),
            case["rejected"].as_bool().unwrap(),
            "{}",
            case["name"]
        );
        if let Ok((_, diagnostics)) = result {
            let actual = if diagnostics.is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::to_value(diagnostics).unwrap()
            };
            assert_eq!(actual, case["diagnostics"], "{}", case["name"]);
        }
    }
}
#[test]
fn provider_launch_expands_arguments_without_shell_or_losing_candidates() {
    use crate::proto::provider::*;
    let registry = registry::Registry::build(
        Layers {
            embedded: Some(registry::embedded_definitions().unwrap()),
            ..Default::default()
        },
        &registry::default_adapters(),
    );
    let codex = registry.lookup("codex").unwrap();
    let request = LaunchRequest {
        model: "synthetic model".into(),
        effort: "high".into(),
        ..Default::default()
    };
    let plan = registry::resolve_launch(&codex, &request).unwrap();
    assert_eq!(plan.executable_candidates.unwrap(), vec!["codex"]);
    let args = plan.args.unwrap();
    assert!(args.windows(2).any(|v| v == ["--model", "synthetic model"]));
    assert!(args.iter().any(|v| v == "model_reasoning_effort=\"high\""));
    assert!(registry::effort_args(&codex, "invalid").is_err());
}

#[test]
fn profile_real_seed_creates_private_file_and_never_copies_credentials() {
    use super::persistence::seed_codex;
    use crate::files::safe_fs::Dir;
    let dir = tempfile::tempdir().unwrap();
    let default = dir.path().join("default");
    let profile = dir.path().join("profile");
    std::fs::create_dir(&default).unwrap();
    std::fs::create_dir(&profile).unwrap();
    std::fs::write(default.join("config.toml"), DEFAULT).unwrap();
    std::fs::write(
        default.join("auth.json"),
        "SYNTHETIC_CREDENTIAL_DO_NOT_COPY",
    )
    .unwrap();
    let cap = Dir::open(&profile).unwrap();
    let report = seed_codex(&default.join("config.toml"), &cap, &SyncOptions::default()).unwrap();
    assert!(report.created);
    assert_eq!(report.removed_temporary_hooks, 1);
    let text = std::fs::read_to_string(profile.join("config.toml")).unwrap();
    assert!(text.contains("gpt-5.1-codex"));
    assert!(!text.contains("user-hook"));
    assert!(!text.contains("MANY_AI_CLI_HUB_TOKEN"));
    assert!(!profile.join("auth.json").exists());
    assert_eq!(
        std::fs::read_to_string(default.join("config.toml")).unwrap(),
        DEFAULT
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(profile.join("config.toml"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}
#[cfg(unix)]
#[test]
fn profile_default_symlink_is_read_only_destination_symlink_rejected() {
    use super::persistence::seed_codex;
    use crate::files::safe_fs::Dir;
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.toml");
    std::fs::write(&source, DEFAULT).unwrap();
    let link = dir.path().join("default.toml");
    symlink(&source, &link).unwrap();
    let profile = dir.path().join("profile");
    std::fs::create_dir(&profile).unwrap();
    let cap = Dir::open(&profile).unwrap();
    seed_codex(&link, &cap, &SyncOptions::default()).unwrap();
    std::fs::remove_file(profile.join("config.toml")).unwrap();
    symlink(&source, profile.join("config.toml")).unwrap();
    assert!(seed_codex(&link, &cap, &SyncOptions::default()).is_err());
    assert_eq!(std::fs::read_to_string(source).unwrap(), DEFAULT);
}
#[test]
fn profile_malformed_existing_never_truncates_or_overwrites() {
    use super::persistence::seed_codex;
    use crate::files::safe_fs::Dir;
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("default.toml");
    std::fs::write(&source, DEFAULT).unwrap();
    let profile = dir.path().join("profile");
    std::fs::create_dir(&profile).unwrap();
    let original = "broken = ['unterminated\n";
    std::fs::write(profile.join("config.toml"), original).unwrap();
    assert!(
        seed_codex(
            &source,
            &Dir::open(&profile).unwrap(),
            &SyncOptions::default()
        )
        .is_err()
    );
    assert_eq!(
        std::fs::read_to_string(profile.join("config.toml")).unwrap(),
        original
    );
}

#[test]
fn profile_sync_off_validates_present_default_and_accepts_an_absent_one() {
    let options = SyncOptions {
        enabled: false,
        ..Default::default()
    };
    assert!(merge_codex("also invalid", Some("malformed = ["), &options).is_err());
    assert!(matches!(
        merge_codex("sandbox_mode='read-only'", Some(""), &options),
        Err(SeedError::LocalPolicyRequired)
    ));
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.toml"), "malformed = [").unwrap();
    let report = super::persistence::seed_codex(
        &dir.path().join("absent-default.toml"),
        &crate::files::safe_fs::Dir::open(dir.path()).unwrap(),
        &options,
    )
    .unwrap();
    assert!(!report.created);
    assert!(report.changed_keys.is_empty());
}
#[test]
fn provider_raw_unknown_unicode_and_extreme_number_use_shared_decoder() {
    let raw=b"{\"schema_version\":1,\"id\":\"synthetic\",\"display_name\":\"Synthetic\",\"launch\":{\"executable\":\"fake\"},\"\xff\":1e999}";
    let (_, diagnostics) =
        validation::validate_definition(raw, &registry::default_adapters()).unwrap();
    assert!(
        diagnostics
            .iter()
            .any(|d| d.code == "unknown_field" && d.field == "\u{fffd}")
    );
}
/// pending devin-child (screen): a registered AI CLI declares its screen in
/// its definition. The field survives the shared decoder (which drops fields
/// its schema does not name), and whitespace-only or control-character
/// markers, which would match every screen, are refused.
#[test]
fn a_definition_screen_declaration_is_decoded_and_its_markers_are_checked() {
    let raw = r#"{"schema_version":1,"id":"synthetic","display_name":"Synthetic","launch":{"executable":"fake"},
        "screen":{"ready_markers":["Ask me anything"],"startup_blockers":["Trust this folder?"],
        "composer_prefix":">","composer_end_markers":["esc to cancel"]}}"#;
    let (definition, diagnostics) =
        validation::validate_definition(raw.as_bytes(), &registry::default_adapters()).unwrap();
    assert!(
        diagnostics.is_empty(),
        "{:?}",
        diagnostics.iter().map(|d| &d.field).collect::<Vec<_>>()
    );
    let screen = definition.screen.unwrap();
    assert_eq!(screen.ready_markers, ["Ask me anything"]);
    assert_eq!(screen.startup_blockers, ["Trust this folder?"]);
    assert_eq!(screen.composer_prefix, ">");
    assert_eq!(screen.composer_end_markers, ["esc to cancel"]);
    let raw = r#"{"schema_version":1,"id":"synthetic","display_name":"Synthetic","launch":{"executable":"fake"},
        "screen":{"ready_markers":["  "],"startup_blockers":["a\u0007"],"composer_prefix":" > "}}"#;
    let (_, diagnostics) =
        validation::validate_definition(raw.as_bytes(), &registry::default_adapters()).unwrap();
    let fields = diagnostics
        .iter()
        .filter(|d| d.code == "invalid_screen")
        .map(|d| d.field.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        fields,
        [
            "screen.composer_prefix",
            "screen.ready_markers[0]",
            "screen.startup_blockers[0]"
        ]
    );
}
