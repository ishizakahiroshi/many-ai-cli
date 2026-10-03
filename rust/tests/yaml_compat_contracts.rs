//! Source-generated yaml.v3 comparisons. Only synthetic config files are used.
use many_ai_cli::config::{
    Config, ConfigError, ConfigStore, DEFAULT_WHISPER_HALLUCINATION_PHRASES, Resource, RuntimePaths,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path};

fn synthetic_paths() -> RuntimePaths {
    RuntimePaths::production(Path::new(if cfg!(windows) {
        r"C:\synthetic-home"
    } else {
        "/synthetic-home"
    }))
    .unwrap()
}

#[derive(Deserialize)]
struct Case {
    name: String,
    yaml: String,
    paths: Vec<String>,
    expected: Option<BTreeMap<String, Value>>,
    #[serde(default)]
    rejected: bool,
    saved_has_hallucination_phrases: bool,
}

#[test]
fn schema_directed_yaml_matches_frozen_go_scalar_and_collection_cases() {
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/foundation/yaml/compat.json")).unwrap();
    assert_eq!(cases.len(), 42);
    let paths = synthetic_paths();
    for case in cases {
        let result = Config::from_yaml(&case.yaml, &paths);
        if case.rejected {
            assert!(result.is_err(), "Go rejected {}", case.name);
            continue;
        }
        let config = result.unwrap_or_else(|error| panic!("{}: {error}", case.name));
        let mut projection = config.public_json();
        projection["token"] = json!(config.token);
        projection["auth_cookie_secret"] = json!(config.auth_cookie_secret);
        projection["remote_pin_hash"] = json!(config.remote_pin_hash);
        let expected = case.expected.unwrap();
        for path in &case.paths {
            assert_eq!(
                projection.pointer(path).unwrap_or(&Value::Null),
                &expected[path],
                "{} {path}",
                case.name
            );
        }
        let saved = config.to_private_yaml().unwrap();
        let reloaded = Config::from_yaml(&saved, &paths)
            .unwrap_or_else(|error| panic!("{} persisted: {error}", case.name));
        assert_eq!(
            config.token, reloaded.token,
            "{} persisted token",
            case.name
        );
        assert_eq!(
            config.auth_cookie_secret, reloaded.auth_cookie_secret,
            "{} persisted cookie",
            case.name
        );
        assert_eq!(
            config.remote_pin_hash, reloaded.remote_pin_hash,
            "{} persisted PIN hash",
            case.name
        );
        let mut expected_reload = config.clone();
        if config
            .voice
            .whisper
            .hallucination_phrases
            .as_ref()
            .is_some_and(Vec::is_empty)
        {
            // Go yaml.Marshal omitempty drops the empty slice. Its next config
            // load restores the default list; this is not a lossless-roundtrip
            // contract. The oracle records the actual Go save omission.
            assert!(
                !case.saved_has_hallucination_phrases,
                "{} Go omission",
                case.name
            );
            assert!(!saved.contains("hallucination_phrases:"));
            expected_reload.voice.whisper.hallucination_phrases = Some(
                DEFAULT_WHISPER_HALLUCINATION_PHRASES
                    .iter()
                    .map(|value| (*value).to_owned())
                    .collect(),
            );
        }
        assert_eq!(
            expected_reload.public_json(),
            reloaded.public_json(),
            "{} persisted settings",
            case.name
        );
    }
}

#[test]
fn ignored_yaml_only_values_never_backup_valid_config_or_rotate_token() {
    let directory = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::production(directory.path()).unwrap();
    std::fs::create_dir_all(paths.root()).unwrap();
    let config_file = paths.resource(Resource::Config);
    let original = "token: 00123\nauth_cookie_secret: 0xFF\nremote_pin_hash: TRUE\nhub: {port: 0123}\nfuture: {[a,b]: .nan}\n";
    std::fs::write(&config_file, original).unwrap();
    let store = ConfigStore::load_or_create(paths.clone(), || {
        panic!("valid historical YAML must never regenerate its token")
    })
    .unwrap();
    assert!(!store.recovered_from_invalid_yaml());
    let loaded = store.snapshot().unwrap().config;
    assert_eq!(loaded.token, "00123");
    assert_eq!(loaded.auth_cookie_secret, "0xFF");
    assert_eq!(loaded.remote_pin_hash, "TRUE");
    assert_eq!(loaded.hub.port, 83);
    assert_eq!(std::fs::read_to_string(config_file).unwrap(), original);
    assert!(!paths.root().join("config.yaml.bak").exists());
}

#[test]
fn decoding_limits_are_non_destructive_and_errors_contain_no_input() {
    let directory = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::production(directory.path()).unwrap();
    std::fs::create_dir_all(paths.root()).unwrap();
    let config_file = paths.resource(Resource::Config);
    let synthetic_secret = "synthetic-private-spelling-00123";
    let oversized = format!(
        "token: {synthetic_secret}\nfuture: {}\n",
        "x".repeat(8 * 1024 * 1024)
    );
    std::fs::write(&config_file, &oversized).unwrap();
    let error = match ConfigStore::load_or_create(paths.clone(), || {
        panic!("resource limits must not regenerate tokens")
    }) {
        Ok(_) => panic!("oversized YAML must be refused without recovery"),
        Err(error) => error,
    };
    assert!(!matches!(error, ConfigError::Parse(_)));
    assert!(!error.to_string().contains(synthetic_secret));
    assert_eq!(std::fs::read_to_string(config_file).unwrap(), oversized);
    assert!(!paths.root().join("config.yaml.bak").exists());
    let nested = format!(
        "token: {synthetic_secret}\nfuture: {}0{}\n",
        "[".repeat(140),
        "]".repeat(140)
    );
    let error = Config::from_yaml(&nested, &paths).unwrap_err();
    assert!(!matches!(error, ConfigError::Parse(_)));
    assert!(!error.to_string().contains(synthetic_secret));
    let bad = format!("token: {synthetic_secret}\nhub: {{port: [}}\n");
    let error = Config::from_yaml(&bad, &paths).unwrap_err();
    assert!(matches!(error, ConfigError::Parse(_)));
    assert!(!error.to_string().contains(synthetic_secret));
}

#[test]
fn aliases_and_merge_keys_preserve_lexemes_without_exponential_expansion() {
    let paths = synthetic_paths();
    let mut input = "token: synthetic-alias-token\na: &a [x]\n".to_owned();
    let mut previous = "a".to_owned();
    for index in 0..20 {
        let current = format!("n{index}");
        input.push_str(&format!(
            "{current}: &{current} [*{previous}, *{previous}]\n"
        ));
        previous = current;
    }
    // Unused YAML values remain unexpanded, so a harmless ignored tree is safe.
    let config = Config::from_yaml(&input, &paths).unwrap();
    assert_eq!(config.token, "synthetic-alias-token");
    let cyclic = "token: synthetic-token\nhub: &hub {<<: *hub}\n";
    let error = Config::from_yaml(cyclic, &paths).unwrap_err();
    assert!(!matches!(error, ConfigError::Parse(_)));
}
