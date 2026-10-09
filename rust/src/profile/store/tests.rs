use super::*;
use crate::config::{Config, CustomProvider};
use serde_json::{Value, json};

fn paths(temp: &tempfile::TempDir) -> RuntimePaths {
    RuntimePaths::trial(
        temp.path(),
        49817,
        &temp.path().join("../installed-provider-store-fixture"),
    )
    .unwrap()
}
fn definition(id: &str) -> Definition {
    Definition {
        schema_version: 1,
        id: id.into(),
        display_name: format!("Fixture {id}"),
        launch: Some(LaunchDefinition {
            executable: "fixture-agent".into(),
            ..Default::default()
        }),
        ..Default::default()
    }
}
fn config(paths: &RuntimePaths) -> Arc<ConfigStore> {
    Arc::new(ConfigStore::new(paths.clone(), Config::default()).unwrap())
}
fn write_accepted(paths: &RuntimePaths, definitions: Vec<Definition>) -> String {
    let digests = definitions
        .iter()
        .map(|def| (def.id.clone(), definition_digest(def).unwrap()))
        .collect();
    let bundle = DistributionBundle {
        payload: DistributionPayload {
            schema_version: 1,
            catalog_version: "fixture-v1".into(),
            created_at: "fixture".into(),
            definitions: Some(definitions),
            digests: Some(digests),
            ..Default::default()
        },
        key_id: "already-accepted-fixture".into(),
        signature: "accepted-by-source-workflow".into(),
    };
    let raw = to_go_json(&bundle).unwrap();
    let digest = content_digest(&raw);
    let root =
        Dir::open_or_create_private(&paths.resource(Resource::ProviderDistributions)).unwrap();
    root.child_dir("accepted", true)
        .unwrap()
        .create_new(&format!("{digest}.json"), &raw, 0o600)
        .unwrap();
    write_json(
        &root,
        "accepted.json",
        &DistributionStatus {
            catalog_version: "fixture-v1".into(),
            digest: digest.clone(),
            state: "accepted".into(),
            ..Default::default()
        },
    )
    .unwrap();
    digest
}

#[test]
fn go_oracle_canonical_hash_revision_and_registry() {
    let fixture: Value = serde_json::from_slice(include_bytes!(
        "../../../fixtures/provider-store-oracle/expected.json"
    ))
    .unwrap();
    let mut definitions = vec![];
    for row in fixture["rows"].as_array().unwrap() {
        let definition: Definition = serde_json::from_value(row["definition"].clone()).unwrap();
        assert_eq!(
            String::from_utf8(to_go_json(&definition).unwrap()).unwrap(),
            row["canonical"].as_str().unwrap()
        );
        let digest = definition_digest(&definition).unwrap();
        assert_eq!(digest, row["digest"]);
        assert_eq!(
            revision_id(
                &definition.id,
                row["parent"].as_str().unwrap(),
                &digest,
                row["created_at"].as_str().unwrap()
            ),
            row["revision"]
        );
        definitions.push(definition);
    }
    let registry = Registry::build(
        Layers {
            embedded: Some(embedded_definitions().unwrap()),
            user: Some(vec![definitions.remove(0)]),
            ..Default::default()
        },
        &default_adapters(),
    );
    assert_eq!(registry.revision(), fixture["registry_revision"]);
}
#[test]
fn user_create_save_delete_and_historical_load_diagnostics() {
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(&temp);
    let store = FileStore::new(&paths);
    assert!(store.load().unwrap().definitions.is_empty());
    assert!(!paths.resource(Resource::ProviderDefinitions).exists());
    let def = definition("fixture-cli");
    store.create_new(&def).unwrap();
    assert!(matches!(
        store.create_new(&def),
        Err(StoreError::AlreadyExists)
    ));
    let mut updated = def.clone();
    updated.display_name = "Updated".into();
    store.save(&updated).unwrap();
    let loaded = store.load().unwrap();
    assert_eq!(loaded.definitions[0].display_name, "Updated");
    assert_eq!(loaded.definitions[0].source.origin.as_str(), "user");
    let dir = Dir::open(&paths.resource(Resource::ProviderDefinitions)).unwrap();
    let mut historical = serde_json::to_value(definition("old-cli")).unwrap();
    historical["future_optional"] = json!({"unsupported":"retained on disk"});
    dir.create_new(
        "old-cli.json",
        &serde_json::to_vec(&historical).unwrap(),
        0o600,
    )
    .unwrap();
    let before = dir.read("old-cli.json", usize::MAX).unwrap();
    let mut older = definition("older-cli");
    older.schema_version = 0;
    dir.create_new("older-cli.json", &to_go_json(&older).unwrap(), 0o600)
        .unwrap();
    dir.create_new("broken.json", b"{", 0o600).unwrap();
    let loaded = store.load().unwrap();
    assert_eq!(loaded.definitions.len(), 2);
    assert!(
        loaded
            .diagnostics
            .iter()
            .any(|d| d.code == "unknown_field" && d.field == "old-cli.json.future_optional")
    );
    assert!(
        loaded
            .diagnostics
            .iter()
            .any(|d| d.code == "unsupported_schema_version")
    );
    assert!(
        loaded
            .diagnostics
            .iter()
            .any(|d| d.code == "invalid_user_definition")
    );
    assert_eq!(dir.read("old-cli.json", usize::MAX).unwrap(), before);
    assert!(store.save(&older).is_err());
    assert!(store.save(&definition("claude")).is_err());
    store.delete("fixture-cli").unwrap();
    assert!(matches!(
        store.delete("fixture-cli"),
        Err(StoreError::Invalid(_))
    ));
}
#[test]
fn sparse_history_override_preserves_prior_edits_and_backup_order() {
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(&temp);
    let store = HistoryStore::new(&paths);
    let baseline = embedded_definitions().unwrap().remove(0);
    let first = store
        .save_override(
            "claude",
            Definition {
                id: "claude".into(),
                display_name: "Customized".into(),
                ..Default::default()
            },
            &baseline,
            "",
            "edit",
        )
        .unwrap();
    assert_eq!(store.current("claude").unwrap().revision, first.revision);
    assert!(first.payload.launch.is_none());
    let second = store
        .save_override(
            "claude",
            Definition {
                id: "claude".into(),
                enabled: Some(false),
                ..Default::default()
            },
            &baseline,
            &first.revision,
            "delete",
        )
        .unwrap();
    assert_eq!(second.payload.display_name, "Customized");
    assert_eq!(second.payload.enabled, Some(false));
    assert!(second.payload.launch.is_none());
    assert!(
        paths
            .resource(Resource::ProviderBackups)
            .join("claude")
            .join(format!("{}.json", first.revision))
            .is_file()
    );
    assert!(matches!(
        store.save_override(
            "claude",
            Definition {
                id: "claude".into(),
                enabled: Some(true),
                ..Default::default()
            },
            &baseline,
            "",
            "edit"
        ),
        Err(StoreError::RevisionConflict { .. })
    ));
    assert_eq!(store.current("claude").unwrap().revision, second.revision);
    let loaded = store.load_overrides().unwrap();
    assert_eq!(loaded.definitions[0].source.revision, second.revision);
    let snapshot = store
        .backup_snapshot("snapshot-cli", &definition("snapshot-cli"), "create")
        .unwrap();
    assert!(store.current("snapshot-cli").is_err());
    assert!(
        paths
            .resource(Resource::ProviderBackups)
            .join("snapshot-cli")
            .join(format!("{}.json", snapshot.revision))
            .is_file()
    );
}
#[test]
fn effective_override_sparse_delta_and_clear_rejection() {
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(&temp);
    let store = HistoryStore::new(&paths);
    let baseline = embedded_definitions().unwrap().remove(0);
    let mut desired = baseline.clone();
    desired.display_name = "Edited".into();
    let saved = store
        .save_effective_override("claude", desired.clone(), &baseline, "", "edit")
        .unwrap();
    assert_eq!(saved.payload.display_name, "Edited");
    assert!(saved.payload.launch.is_none());
    desired.launch.as_mut().unwrap().model_args.clear();
    assert!(
        matches!(store.save_effective_override("claude",desired,&baseline,&saved.revision,"edit"),Err(StoreError::OverrideClearsValue(fields)) if fields.contains(&"launch.model_args".into()))
    );
    assert_eq!(store.current("claude").unwrap().revision, saved.revision);
}
#[test]
fn registry_loads_all_five_layers_and_survives_restart() {
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(&temp);
    let cfg = Config {
        custom_providers: vec![CustomProvider {
            id: "legacy-cli".into(),
            command: "legacy-cli --agent".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let config = Arc::new(ConfigStore::new(paths.clone(), cfg).unwrap());
    FileStore::new(&paths)
        .create_new(&definition("user-cli"))
        .unwrap();
    write_accepted(&paths, vec![definition("dist-cli")]);
    let baseline = embedded_definitions().unwrap().remove(0);
    HistoryStore::new(&paths)
        .save_override(
            "claude",
            Definition {
                id: "claude".into(),
                enabled: Some(false),
                ..Default::default()
            },
            &baseline,
            "",
            "delete",
        )
        .unwrap();
    let first = ProviderRegistryStore::new(&paths, config.clone()).unwrap();
    let frozen = first.snapshot().unwrap();
    for (id, origin) in [
        ("codex", "embedded"),
        ("dist-cli", "distribution"),
        ("legacy-cli", "legacy"),
        ("user-cli", "user"),
        ("claude", "override"),
    ] {
        assert_eq!(
            frozen
                .registry
                .lookup(id)
                .unwrap()
                .effective_source
                .origin
                .as_str(),
            origin
        );
    }
    assert_eq!(
        frozen.registry.lookup("claude").unwrap().definition.enabled,
        Some(false)
    );
    let restarted = ProviderRegistryStore::new(&paths, config).unwrap();
    assert_eq!(
        restarted.snapshot().unwrap().registry.revision(),
        frozen.registry.revision()
    );
    first
        .definitions
        .create_new(&definition("later-cli"))
        .unwrap();
    let newer = first.reload().unwrap();
    assert!(frozen.registry.lookup("later-cli").is_none());
    assert!(newer.registry.lookup("later-cli").is_some());
    assert_eq!(first.baseline("claude").unwrap().enabled, Some(true));
}
#[test]
fn accepted_absent_corrupt_and_unknown_optional_are_distinct() {
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(&temp);
    let store = AcceptedDistributionStore::new(&paths);
    assert!(store.load_accepted().unwrap().is_none());
    let digest = write_accepted(&paths, vec![definition("dist-cli")]);
    assert!(store.load_accepted().unwrap().is_some());
    let root = Dir::open(&paths.resource(Resource::ProviderDistributions)).unwrap();
    root.child_dir("accepted", false)
        .unwrap()
        .replace(&format!("{digest}.json"), b"{}", 0o600)
        .unwrap();
    let loaded = store.load_definitions();
    assert!(loaded.definitions.is_empty());
    assert_eq!(
        loaded.diagnostics[0].code,
        "distribution_accepted_load_failed"
    );
    assert!(
        loaded.diagnostics[0]
            .message
            .contains("content digest mismatch")
    );
    let live = ProviderRegistryStore::new(&paths, config(&paths)).unwrap();
    assert!(live.snapshot().unwrap().registry.lookup("claude").is_some());
    assert_eq!(
        live.snapshot().unwrap().diagnostics.last().unwrap().code,
        "distribution_accepted_load_failed"
    );
}
#[test]
fn source_startup_degrades_reload_keeps_previous_and_projection_drops_load_diagnostics() {
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(&temp);
    write_accepted(&paths, vec![definition("retained-distribution")]);
    std::fs::write(
        paths.resource(Resource::ProviderDefinitions),
        b"ordinary invalid root file",
    )
    .unwrap();
    let store = ProviderRegistryStore::new(&paths, config(&paths)).unwrap();
    let before = store.snapshot().unwrap();
    assert!(before.registry.lookup("claude").is_some());
    assert!(before.registry.lookup("retained-distribution").is_some());
    assert!(
        before
            .diagnostics
            .iter()
            .any(|d| d.code == "provider_store_load")
    );
    assert!(store.reload().is_err());
    assert_eq!(
        before.registry.revision(),
        store.snapshot().unwrap().registry.revision()
    );
    let temp2 = tempfile::tempdir().unwrap();
    let paths2 = self::paths(&temp2);
    let dir = Dir::open_or_create_private(&paths2.resource(Resource::ProviderDefinitions)).unwrap();
    dir.create_new("bad.json", b"{", 0o600).unwrap();
    let store2 = ProviderRegistryStore::new(&paths2, config(&paths2)).unwrap();
    assert!(
        store2
            .snapshot()
            .unwrap()
            .diagnostics
            .iter()
            .any(|d| d.code == "invalid_user_definition")
    );
    assert!(
        store2
            .reload()
            .unwrap()
            .diagnostics
            .iter()
            .all(|d| d.code != "invalid_user_definition")
    );
    assert!(
        store2
            .definitions
            .load()
            .unwrap()
            .diagnostics
            .iter()
            .any(|d| d.code == "invalid_user_definition")
    );
}
#[test]
fn source_shadow_recovery_and_bad_head_quarantine_are_persisted() {
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(&temp);
    let store = HistoryStore::new(&paths);
    let baseline = embedded_definitions().unwrap().remove(0);
    let record = store
        .save_override(
            "claude",
            Definition {
                id: "claude".into(),
                enabled: Some(false),
                ..Default::default()
            },
            &baseline,
            "",
            "delete",
        )
        .unwrap();
    let dir = Dir::open(&paths.resource(Resource::ProviderOverrides))
        .unwrap()
        .child_dir("claude", false)
        .unwrap();
    dir.rename_to("HEAD", &dir, "HEAD.atomic-replace-shadow")
        .unwrap();
    assert_eq!(store.current("claude").unwrap().revision, record.revision);
    dir.replace("HEAD", b"missing-revision\n", 0o600).unwrap();
    let loaded = store.load_overrides().unwrap();
    assert_eq!(loaded.diagnostics[0].code, "invalid_override");
    assert!(loaded.definitions.is_empty());
    assert!(dir.read("HEAD", 1024).is_ok());
    let quarantine = Dir::open(&paths.resource(Resource::ProviderBackups))
        .unwrap()
        .child_dir("quarantine", false)
        .unwrap();
    assert_eq!(quarantine.entries().unwrap().len(), 1);
}

#[test]
fn persisted_go_wire_duplicate_casefold_and_null_members() {
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(&temp);
    let history = HistoryStore::new(&paths);
    let record = history
        .backup_snapshot("wire-cli", &definition("wire-cli"), "fixture")
        .unwrap();
    let raw = format!(
        r#"{{"SCHEMA_VERSION":1,"PROVIDER_ID":"wire-cli","REVISION":"{}","CONTENT_DIGEST":"{}","PAYLOAD":{{"schema_version":1,"id":"wire-cli","display_name":"Fixture wire-cli"}},"payload":{{"launch":{{"executable":"fixture-agent"}}}},"payload":null}}"#,
        record.revision, record.content_digest
    );
    let decoded = crate::proto::decode_wire::<RevisionRecord>(raw.as_bytes()).unwrap();
    assert_eq!(
        definition_digest(&decoded.payload).unwrap(),
        record.content_digest
    );
    let root = Dir::open_or_create_private(&paths.resource(Resource::ProviderOverrides))
        .unwrap()
        .child_dir("wire-cli", true)
        .unwrap();
    root.child_dir("revisions", true)
        .unwrap()
        .create_new(&format!("{}.json", record.revision), raw.as_bytes(), 0o600)
        .unwrap();
    root.create_new("HEAD", record.revision.as_bytes(), 0o600)
        .unwrap();
    assert_eq!(
        history.current("wire-cli").unwrap().content_digest,
        record.content_digest
    );
    let digest = write_accepted(&paths, vec![definition("dist-cli")]);
    let distribution = Dir::open(&paths.resource(Resource::ProviderDistributions)).unwrap();
    distribution.replace("accepted.json",format!(r#"{{"CATALOG_VERSION":"fixture","DIGEST":"{digest}","digest":null,"STATE":"accepted"}}"#).as_bytes(),0o600).unwrap();
    assert!(
        AcceptedDistributionStore::new(&paths)
            .load_accepted()
            .unwrap()
            .is_some()
    );
}

#[test]
fn source_signed_accepted_bundle_bytes_load_without_network() {
    let fixture: Value = serde_json::from_slice(include_bytes!(
        "../../../fixtures/provider-store-oracle/expected.json"
    ))
    .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(&temp);
    let root =
        Dir::open_or_create_private(&paths.resource(Resource::ProviderDistributions)).unwrap();
    let digest = fixture["accepted_digest"].as_str().unwrap();
    root.child_dir("accepted", true)
        .unwrap()
        .create_new(
            &format!("{digest}.json"),
            fixture["accepted_bundle"].as_str().unwrap().as_bytes(),
            0o600,
        )
        .unwrap();
    write_json(
        &root,
        "accepted.json",
        &DistributionStatus {
            catalog_version: "fixture-v1".into(),
            digest: digest.into(),
            state: "accepted".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let store = ProviderRegistryStore::new(&paths, config(&paths)).unwrap();
    assert_eq!(
        store
            .snapshot()
            .unwrap()
            .registry
            .lookup("distributed-fixture")
            .unwrap()
            .definition
            .display_name,
        "Distribution <fixture>"
    );
}
#[test]
fn source_exclusive_create_has_one_winner() {
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(&temp);
    let store = Arc::new(FileStore::new(&paths));
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let store = store.clone();
            std::thread::spawn(move || store.create_new(&definition("racer")))
        })
        .collect();
    let mut winners = 0;
    for thread in threads {
        match thread.join().unwrap() {
            Ok(()) => winners += 1,
            Err(StoreError::AlreadyExists) => {}
            Err(error) => panic!("unexpected create error: {error}"),
        }
    }
    assert_eq!(winners, 1);
    assert_eq!(store.load().unwrap().definitions.len(), 1);
}

#[test]
fn every_builtin_unchanged_effective_save_remains_sparse_and_readable() {
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(&temp);
    let store = ProviderRegistryStore::new(&paths, config(&paths)).unwrap();
    for id in BUILTIN_PROVIDER_IDS {
        let desired = store
            .snapshot()
            .unwrap()
            .registry
            .lookup(id)
            .unwrap()
            .definition;
        let baseline = store.baseline(id).unwrap();
        let saved = store
            .history
            .save_effective_override(id, desired, &baseline, "", "edit")
            .unwrap();
        assert!(saved.payload.launch.is_none());
        assert!(saved.payload.models.is_none());
        assert!(saved.payload.update.is_none());
        assert_eq!(store.history.current(id).unwrap().revision, saved.revision);
    }
    assert_eq!(
        store.history.load_overrides().unwrap().definitions.len(),
        BUILTIN_PROVIDER_IDS.len()
    );
}
