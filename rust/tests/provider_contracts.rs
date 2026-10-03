//! Offline Go oracle contracts. All commands, identifiers, paths and content are
//! synthetic. These tests never run a provider or contact an external service.
use many_ai_cli::{
    config::{Config, CustomProvider, HeadlessDef, RuntimePaths, legacy_provider_definitions},
    proto::provider::*,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

#[derive(Deserialize)]
struct WireCase {
    name: String,
    r#type: String,
    input: Value,
    expected: Value,
    expected_json: String,
    #[serde(default)]
    rejected: bool,
}

fn check_wire<T: DeserializeOwned + Serialize + Default>(case: &WireCase) {
    let parsed = serde_json::from_value::<T>(case.input.clone());
    if case.rejected {
        assert!(parsed.is_err(), "{}: Go rejected this value", case.name);
        return;
    }
    let parsed = parsed.unwrap_or_else(|error| panic!("{}: {error}", case.name));
    assert_eq!(
        to_go_json(&parsed).unwrap(),
        case.expected_json.as_bytes(),
        "{} exact Go bytes",
        case.name
    );
    let actual = serde_json::to_value(&parsed).unwrap();
    assert_eq!(actual, case.expected, "{}", case.name);
    let second: T = serde_json::from_value(actual.clone()).unwrap();
    assert_eq!(
        serde_json::to_value(second).unwrap(),
        actual,
        "{}",
        case.name
    );
    if case.name.ends_with("/zero") {
        assert_eq!(serde_json::to_value(T::default()).unwrap(), case.expected);
    }
}

#[test]
fn all_provider_dtos_match_go_zero_filled_and_historical_wire_cases() {
    let cases: Vec<WireCase> =
        serde_json::from_str(include_str!("fixtures/foundation/provider/wire.json")).unwrap();
    assert_eq!(cases.len(), 73);
    for case in &cases {
        macro_rules! dispatch {
            ($($typ:ident),+ $(,)?) => {
                match case.r#type.as_str() {
                    $(stringify!($typ) => check_wire::<$typ>(case),)+
                    unknown => panic!("unmapped Go provider DTO: {unknown}"),
                }
            };
        }
        dispatch!(
            SourceRef,
            Definition,
            LaunchDefinition,
            ModelsDefinition,
            ModelDefinition,
            HeadlessDefinition,
            PresentationDefinition,
            UpdateDefinition,
            AdapterRefs,
            Layers,
            AdapterCatalog,
            CapabilitySummary,
            EffectiveDefinition,
            Summary,
            Diagnostic,
            LaunchRequest,
            ResolvedLaunch,
            AdapterDescriptor,
            DistributionPayload,
            DistributionBundle,
            DistributionStatus,
            DistributionFieldDiff,
            DistributionProviderDiff,
            RevisionRecord,
            QuarantineRecord,
        );
    }
}

#[derive(Deserialize)]
struct LegacyCase {
    name: String,
    yaml: String,
    raw: Value,
    definitions: Value,
    diagnostics: Value,
}

#[test]
fn legacy_adapter_matches_go_and_preserves_historical_config_without_reset() {
    let cases: Vec<LegacyCase> =
        serde_json::from_str(include_str!("fixtures/foundation/provider/legacy.json")).unwrap();
    assert_eq!(cases.len(), 9);
    let paths = RuntimePaths::production(std::path::Path::new(if cfg!(windows) {
        r"C:\synthetic-home"
    } else {
        "/synthetic-home"
    }))
    .unwrap();
    for case in cases {
        let config = Config::from_yaml(&case.yaml, &paths)
            .unwrap_or_else(|error| panic!("{}: {error}", case.name));
        assert_eq!(config.token, "synthetic-stable-token", "{}", case.name);
        let public = config.public_json();
        let raw = public.get("custom_providers").unwrap_or(&Value::Null);
        assert_eq!(raw, &case.raw, "{} raw historical entries", case.name);
        let unchanged = config.custom_providers.clone();
        let (definitions, diagnostics) = legacy_provider_definitions(&config.custom_providers);
        assert_eq!(
            serde_json::to_value(&definitions).unwrap(),
            case.definitions,
            "{} definitions",
            case.name
        );
        // The Go function initializes definitions to [] and leaves diagnostics
        // nil until a warning is appended. Vec is the internal Rust API only.
        let wire_diagnostics = if diagnostics.is_empty() {
            Value::Null
        } else {
            serde_json::to_value(diagnostics).unwrap()
        };
        assert_eq!(wire_diagnostics, case.diagnostics, "{} warnings", case.name);
        assert!(
            unchanged == config.custom_providers,
            "{} mutation",
            case.name
        );
        let encoded = config.to_private_yaml().unwrap();
        let reloaded = Config::from_yaml(&encoded, &paths).unwrap();
        assert!(
            unchanged == reloaded.custom_providers,
            "{} persisted",
            case.name
        );
        assert_eq!(reloaded.token, "synthetic-stable-token", "{}", case.name);
    }
}

#[test]
fn disabled_overlays_and_false_capability_entries_survive_roundtrip() {
    let overlay: Definition = serde_json::from_value(json!({
        "id": "fake-provider",
        "enabled": false,
        "update": {"enabled": false},
        "capabilities": {"headless": false},
        "models": {"allow_custom": false},
        "source": {"origin": "override"}
    }))
    .unwrap();
    assert_eq!(overlay.enabled, Some(false));
    assert_eq!(overlay.update.as_ref().unwrap().enabled, Some(false));
    assert_eq!(overlay.capabilities.get("headless"), Some(&false));
    assert_eq!(
        serde_json::to_value(overlay).unwrap(),
        json!({
            "id": "fake-provider", "enabled": false,
            "update": {"enabled": false}, "capabilities": {"headless": false},
            "models": {}, "adapters": {}, "source": {"origin": "override"}
        })
    );
}

#[test]
fn effective_definition_keeps_both_capability_fields_and_flattened_base() {
    let effective = EffectiveDefinition {
        definition: Definition {
            id: "fake-provider".into(),
            capabilities: [("headless".into(), false)].into(),
            ..Default::default()
        },
        capabilities: CapabilitySummary {
            launch: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let encoded = serde_json::to_value(effective).unwrap();
    assert_eq!(encoded["id"], "fake-provider");
    assert!(encoded.get("definition").is_none());
    assert_eq!(encoded["capabilities"]["headless"], false);
    assert_eq!(encoded["capabilities_summary"]["launch"], true);
    assert_eq!(encoded["capabilities_summary"]["headless"], false);
    assert_eq!(
        encoded["capabilities_summary"].as_object().unwrap().len(),
        10
    );
}

#[test]
fn schema_reads_are_not_new_write_validation_and_enums_are_open() {
    // Struct decoding intentionally accepts an old version and an executable
    // shape that the new-manifest validator may reject at the C3 write boundary.
    let historical: Definition = serde_json::from_value(json!({
        "schema_version": -1,
        "id": "historical/provider",
        "launch": {"executable": "fake old executable", "unknown": "ignored"},
        "source": {"origin": "future-layer"},
        "unknown_optional": true
    }))
    .unwrap();
    assert_eq!(historical.schema_version, -1);
    assert_eq!(historical.source.origin.as_str(), "future-layer");
    assert!(
        !serde_json::to_value(historical)
            .unwrap()
            .as_object()
            .unwrap()
            .contains_key("unknown_optional")
    );
    let diagnostic: Diagnostic =
        serde_json::from_value(json!({"severity": "future-severity"})).unwrap();
    assert!(!diagnostic.is_error());
    let diagnostic = Diagnostic {
        severity: SEVERITY_ERROR.into(),
        ..Default::default()
    };
    assert!(diagnostic.is_error());
    let descriptor: AdapterDescriptor =
        serde_json::from_value(json!({"kind": "future-adapter"})).unwrap();
    assert_eq!(descriptor.kind.as_str(), "future-adapter");
}

#[test]
fn required_go_collections_distinguish_nil_and_empty() {
    let nil = DistributionPayload::default();
    let empty = DistributionPayload {
        definitions: Some(vec![]),
        digests: Some(Default::default()),
        ..Default::default()
    };
    assert_eq!(
        serde_json::to_value(nil).unwrap()["definitions"],
        Value::Null
    );
    assert_eq!(
        serde_json::to_value(empty).unwrap()["definitions"],
        json!([])
    );
    let nil = Layers::default();
    let empty = Layers {
        overrides: Some(vec![]),
        ..Default::default()
    };
    assert_eq!(serde_json::to_value(nil).unwrap()["Overrides"], Value::Null);
    assert_eq!(serde_json::to_value(empty).unwrap()["Overrides"], json!([]));
}

#[test]
fn adapter_catalog_and_stable_provider_order_match_the_frozen_source() {
    let expected: Value =
        serde_json::from_str(include_str!("fixtures/foundation/provider/catalog.json")).unwrap();
    let catalog = default_adapter_catalog();
    assert_eq!(serde_json::to_value(&catalog).unwrap(), expected);
    for key in [
        "",
        "none",
        "unsupported",
        "launch:generic-v1",
        "subagent:grok-v1",
    ] {
        assert!(catalog.has(key));
    }
    for key in [
        "launch:future-v1",
        "NONE",
        "subagents:grok-v1",
        "fake-executable",
    ] {
        assert!(!catalog.has(key));
    }
    let small = AdapterCatalog::new(["", "fake:key", "fake:key"]);
    assert_eq!(small.keys.as_ref().unwrap().len(), 1);
    assert!(small.has("fake:key"));
    assert!(!AdapterCatalog::default().has("fake:key"));
    let mut ids: Vec<String> = [
        "z-custom",
        "shell",
        "codex",
        "claude",
        "a-custom",
        "command-code",
        "grok",
        "opencode",
        "cursor-agent",
        "copilot",
        "claude",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    sort_ids(&mut ids);
    assert_eq!(
        ids,
        [
            "claude",
            "claude",
            "codex",
            "copilot",
            "cursor-agent",
            "opencode",
            "grok",
            "command-code",
            "a-custom",
            "shell",
            "z-custom"
        ]
    );
    assert_eq!(builtin_order("shell"), BUILTIN_PROVIDER_IDS.len());
}

#[test]
fn legacy_output_owns_its_data_and_does_not_default_the_prompt_transport() {
    let mut raw = vec![CustomProvider {
        id: "fake".into(),
        command: "fake-executable --synthetic".into(),
        headless: Some(HeadlessDef {
            args: vec!["--print".into()],
            format: " text ".into(),
            prompt_via: " ".into(),
        }),
        ..Default::default()
    }];
    let (definitions, diagnostics) = legacy_provider_definitions(&raw);
    assert!(diagnostics.is_empty());
    raw[0].command.clear();
    raw[0].headless.as_mut().unwrap().args.clear();
    let launch = definitions[0].launch.as_ref().unwrap();
    assert_eq!(launch.executable, "fake-executable");
    assert_eq!(launch.args, ["--synthetic"]);
    assert_eq!(launch.headless.as_ref().unwrap().args, ["--print"]);
    assert_eq!(launch.headless.as_ref().unwrap().format, "text");
    assert_eq!(launch.headless.as_ref().unwrap().prompt_via, "");
    assert_eq!(definitions[0].source.origin.as_str(), ORIGIN_LEGACY);
    assert_eq!(definitions[0].enabled, None);
}
