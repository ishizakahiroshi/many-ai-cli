//! Provider maintenance uses synthetic homes and never performs startup writes.
use super::*;
use crate::config::{ConfigBackups, removed};
use std::cell::Cell;

const TOKEN: &str = "synthetic-provider-persistence-token";

struct Fixture {
    _home: tempfile::TempDir,
    paths: RuntimePaths,
}
impl Fixture {
    fn new() -> Self {
        let home = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::production(home.path()).unwrap();
        Self { _home: home, paths }
    }
    fn write(&self, bytes: impl AsRef<[u8]>) {
        std::fs::create_dir_all(self.paths.root()).unwrap();
        std::fs::write(self.paths.resource(Resource::Config), bytes).unwrap();
    }
    fn read(&self) -> Vec<u8> {
        std::fs::read(self.paths.resource(Resource::Config)).unwrap()
    }
    fn backups(&self) -> Vec<crate::config::BackupRecord> {
        ConfigBackups::new(&self.paths).list().unwrap()
    }
    fn load(&self) -> ConfigStore {
        ConfigStore::load_for_provider(self.paths.clone()).unwrap()
    }
}

fn provider() -> CustomProvider {
    CustomProvider {
        id: "synthetic-cli".into(),
        label: "Synthetic CLI".into(),
        command: "synthetic-cli".into(),
        ..CustomProvider::default()
    }
}

fn existing_token() -> Result<String, ConfigError> {
    panic!("an existing token must not be replaced")
}

#[test]
fn read_only_load_keeps_missing_root_and_missing_token_untouched() {
    let fixture = Fixture::new();
    let store = fixture.load();
    assert!(store.snapshot().unwrap().config.token.is_empty());
    assert!(!fixture.paths.root().exists());

    let original = b"# keep this comment\ncustom_providers: []\n";
    fixture.write(original);
    let legacy = fixture.paths.root().join("config.yaml.bak");
    std::fs::write(&legacy, b"synthetic old backup").unwrap();
    let store = fixture.load();
    assert!(store.snapshot().unwrap().config.token.is_empty());
    assert_eq!(fixture.read(), original);
    assert!(legacy.is_file());
    assert!(!fixture.paths.resource(Resource::ConfigBackups).exists());
}

#[test]
fn forced_backup_happens_once_for_lossy_add_and_clean_remove() {
    let fixture = Fixture::new();
    let original = format!("# retain in backup\ntoken: {TOKEN}\ncustom_providers: []\n");
    fixture.write(original.as_bytes());
    let store = fixture.load();
    let observed = Arc::new(Mutex::new(Vec::new()));
    let records = observed.clone();
    store.set_backup_observer(Arc::new(move |record| {
        records.lock().unwrap().push(record.id.clone());
    }));
    let mut before = store.snapshot().unwrap();
    before.config.custom_providers.push(provider());
    let (added, backup, comments) = store
        .persist_with_backup(before.revision, before.config, existing_token)
        .unwrap();
    assert!(comments);
    let backup = backup.unwrap();
    assert_eq!(fixture.backups(), [backup.clone()]);
    assert_eq!(std::fs::read(backup.path).unwrap(), original.as_bytes());
    assert_eq!(added.config.custom_providers, [provider()]);
    assert_eq!(added.config.token, TOKEN);
    assert_eq!(observed.lock().unwrap().len(), 1);

    let added_bytes = fixture.read();
    let mut next = added.config;
    next.custom_providers.clear();
    let (removed, backup, comments) = store
        .persist_with_backup(added.revision, next, existing_token)
        .unwrap();
    assert!(!comments);
    assert_eq!(fixture.backups().len(), 2);
    assert_eq!(std::fs::read(backup.unwrap().path).unwrap(), added_bytes);
    assert_eq!(observed.lock().unwrap().len(), 2);
    assert!(removed.config.custom_providers.is_empty());
}

#[test]
fn missing_config_and_missing_token_are_initialized_only_on_success() {
    for original in [None, Some("# no token yet\ncustom_providers: []\n")] {
        let fixture = Fixture::new();
        if let Some(original) = original {
            fixture.write(original);
        }
        let store = fixture.load();
        let calls = Cell::new(0);
        let mut next = store.snapshot().unwrap();
        next.config.custom_providers.push(provider());
        let (saved, backup, comments) = store
            .persist_with_backup(next.revision, next.config, || {
                calls.set(calls.get() + 1);
                Ok(TOKEN.into())
            })
            .unwrap();
        assert_eq!(calls.get(), 1);
        assert_eq!(saved.config.token, TOKEN);
        assert_eq!(comments, original.is_some());
        assert_eq!(fixture.backups().len(), usize::from(original.is_some()));
        assert_eq!(backup.is_some(), original.is_some());
        if let Some(original) = original {
            assert_eq!(
                std::fs::read(backup.unwrap().path).unwrap(),
                original.as_bytes()
            );
        }
        let loaded = fixture.load().snapshot().unwrap().config;
        assert_eq!(loaded.token, TOKEN);
        assert_eq!(loaded.custom_providers, [provider()]);
    }
}

#[test]
fn invalid_config_and_stale_revision_do_not_create_files_or_call_token_factory() {
    for original in [None, Some("# untouched\ncustom_providers: []\n")] {
        let fixture = Fixture::new();
        if let Some(original) = original {
            fixture.write(original);
        }
        let store = fixture.load();
        let initial = store.snapshot().unwrap();
        let mut invalid = initial.config.clone();
        invalid.hub.trusted_networks = vec!["invalid CIDR".into()];
        assert!(
            store
                .persist_with_backup(0, invalid, existing_token)
                .is_err()
        );
        assert!(matches!(
            store.persist_with_backup(1, initial.config, existing_token),
            Err(ConfigError::Conflict { .. })
        ));
        assert_eq!(store.snapshot().unwrap().revision, 0);
        assert!(!fixture.paths.resource(Resource::ConfigBackups).exists());
        if let Some(original) = original {
            assert_eq!(fixture.read(), original.as_bytes());
        } else {
            assert!(!fixture.paths.root().exists());
        }
    }
}

#[test]
fn token_factory_failure_keeps_original_bytes_and_creates_no_backup() {
    for fail_empty in [false, true] {
        let fixture = Fixture::new();
        let original = b"# untouched\ncustom_providers: []\n";
        fixture.write(original);
        let store = fixture.load();
        let snapshot = store.snapshot().unwrap();
        assert!(
            store
                .persist_with_backup(snapshot.revision, snapshot.config, || {
                    if fail_empty {
                        Ok(String::new())
                    } else {
                        Err(ConfigError::invalid("synthetic token generation failure"))
                    }
                })
                .is_err()
        );
        assert_eq!(fixture.read(), original);
        assert_eq!(store.snapshot().unwrap().revision, 0);
        assert!(fixture.backups().is_empty());
    }
}

#[test]
fn unreadable_load_and_concurrent_broken_edit_are_never_overwritten() {
    let fixture = Fixture::new();
    let broken = format!("token: {TOKEN}\nhub: [unterminated");
    fixture.write(broken.as_bytes());
    let error = match ConfigStore::load_for_provider(fixture.paths.clone()) {
        Ok(_) => panic!("broken YAML must not load"),
        Err(error) => error,
    };
    assert!(matches!(error, ConfigError::Unreadable(_)));
    assert!(!error.to_string().contains(TOKEN));
    assert_eq!(fixture.read(), broken.as_bytes());
    assert!(fixture.backups().is_empty());

    fixture.write(format!("token: {TOKEN}\n"));
    let store = fixture.load();
    let mut next = store.snapshot().unwrap();
    next.config.custom_providers.push(provider());
    fixture.write(broken.as_bytes());
    let error = store
        .persist_with_backup(next.revision, next.config, existing_token)
        .unwrap_err();
    assert!(!error.to_string().contains(TOKEN));
    assert_eq!(fixture.read(), broken.as_bytes());
    assert_eq!(store.snapshot().unwrap().revision, 0);
    assert!(fixture.backups().is_empty());
}

#[test]
fn failed_backup_keeps_bytes_and_memory_unchanged() {
    let fixture = Fixture::new();
    let original = format!("token: {TOKEN}\n");
    fixture.write(original.as_bytes());
    std::fs::write(fixture.paths.root().join("backups"), b"blocked").unwrap();
    let store = fixture.load();
    let mut next = store.snapshot().unwrap();
    next.config.custom_providers.push(provider());
    assert!(
        store
            .persist_with_backup(next.revision, next.config, existing_token)
            .is_err()
    );
    assert_eq!(fixture.read(), original.as_bytes());
    assert_eq!(store.snapshot().unwrap().revision, 0);
    assert!(store.snapshot().unwrap().config.custom_providers.is_empty());
}

#[test]
fn rejected_raw_entries_keep_their_startup_positions_and_c2_notices() {
    let fixture = Fixture::new();
    let original = format!(
        "token: {TOKEN}\ncustom_providers:\n  - not-a-mapping\n  - id: synthetic-cli\n    command: synthetic-cli\n"
    );
    fixture.write(original.as_bytes());
    let store = fixture.load();
    assert_eq!(store.startup_problems().len(), 1);
    assert_eq!(store.startup_problems()[0].index, Some(1));
    assert_eq!(store.held_problems(), store.startup_problems().to_vec());
    let snapshot = store.snapshot().unwrap();
    let (_, backup, _) = store
        .persist_with_backup(snapshot.revision, snapshot.config, existing_token)
        .unwrap();
    assert_eq!(fixture.backups().len(), 1);
    let backup = backup.unwrap();
    assert_eq!(std::fs::read(backup.path).unwrap(), original.as_bytes());
    let notices = removed::list(&fixture.paths).unwrap();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].backup_id, backup.id);
    assert_eq!(notices[0].index, Some(1));
    assert!(store.held_problems().is_empty());
}

#[test]
fn ordinary_persist_still_avoids_redundant_backups() {
    let fixture = Fixture::new();
    let mut config = Config::defaults(&fixture.paths);
    config.token = TOKEN.into();
    fixture.write(config.to_private_yaml().unwrap());
    let store = fixture.load();
    let snapshot = store.snapshot().unwrap();
    store.persist(snapshot.revision, snapshot.config).unwrap();
    assert!(fixture.backups().is_empty());
    let snapshot = store.snapshot().unwrap();
    let (_, backup, comments) = store
        .persist_with_backup(snapshot.revision, snapshot.config, existing_token)
        .unwrap();
    assert!(backup.is_some());
    assert!(!comments);
    assert_eq!(fixture.backups().len(), 1);
}

#[test]
fn comment_notice_uses_yaml_comments_instead_of_hashes_inside_values() {
    assert!(has_yaml_comments(b"# top-level comment\ntoken: synthetic\n"));
    assert!(has_yaml_comments(b"token: synthetic # inline comment\n"));
    assert!(!has_yaml_comments(b"token: 'synthetic # quoted text'\n"));
    assert!(!has_yaml_comments(
        b"unknown: |\n  # block scalar text\ntoken: synthetic\n"
    ));
    assert!(!has_yaml_comments(b"token: synthetic\n"));
}

#[test]
fn listing_preserves_malformed_rows_and_maps_duplicate_reasons_by_raw_index() {
    let fixture = Fixture::new();
    let original = format!(
        "token: {TOKEN}\ncustom_providers:\n\
         \x20 - id: shared\n\
         \x20   label: [wrong-type]\n\
         \x20   command: malformed-label-cli\n\
         \x20   headless: {{args: [exec], format: text, prompt_via: arg}}\n\
         \x20 - id: shared\n\
         \x20   label: Usable CLI\n\
         \x20   command: usable-cli\n\
         \x20 - id: shared\n\
         \x20   label: Duplicate CLI\n\
         \x20   command: duplicate-cli\n"
    );
    fixture.write(original.as_bytes());
    let store = fixture.load();
    let rows = store.provider_listing().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0]["id"], "shared");
    assert!(rows[0]["label"].is_null());
    assert_eq!(rows[0]["command"], "malformed-label-cli");
    assert_eq!(rows[0]["headless"], true);
    assert_eq!(rows[0]["status"], "dropped");
    assert_eq!(rows[1]["label"], "Usable CLI");
    assert_eq!(rows[1]["status"], "in-use");
    assert_eq!(rows[1]["reasons"], serde_json::json!([]));
    assert_eq!(rows[2]["label"], "Duplicate CLI");
    assert_eq!(rows[2]["status"], "dropped");
    for index in [0, 2] {
        let reasons: Vec<_> = store
            .startup_problems()
            .iter()
            .filter(|problem| problem.index == Some(index + 1))
            .map(|problem| problem.text_en().reason)
            .collect();
        assert_eq!(rows[index]["reasons"], serde_json::json!(reasons));
    }
    let keys = [
        "builtin", "command", "headless", "id", "label", "reasons", "status",
    ];
    for row in rows {
        assert_eq!(row.as_object().unwrap().len(), keys.len());
        assert!(keys.iter().all(|key| row.get(*key).is_some()));
        assert_eq!(row["builtin"], false);
    }
    assert_eq!(fixture.read(), original.as_bytes());
    assert!(fixture.backups().is_empty());
}

#[test]
fn listing_keeps_non_mapping_and_bad_headless_rows_without_other_config_fields() {
    let fixture = Fixture::new();
    fixture.write(format!(
        "token: {TOKEN}\ncustom_providers:\n\
         \x20 - not-a-mapping\n\
         \x20 - id: bad-headless\n\
         \x20   label: Readable label\n\
         \x20   command: readable-command\n\
         \x20   headless: [wrong-type]\n"
    ));
    let rows = fixture.load().provider_listing().unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows[0]["id"].is_null());
    assert_eq!(rows[0]["status"], "dropped");
    assert_eq!(rows[1]["id"], "bad-headless");
    assert_eq!(rows[1]["label"], "Readable label");
    assert_eq!(rows[1]["command"], "readable-command");
    assert_eq!(rows[1]["headless"], true);
    assert_eq!(rows[1]["status"], "dropped");
    assert!(!serde_json::to_string(&rows).unwrap().contains(TOKEN));
}

#[test]
fn listing_and_existence_follow_written_state_instead_of_old_startup_indices() {
    let fixture = Fixture::new();
    fixture.write(format!(
        "token: {TOKEN}\ncustom_providers:\n\
         \x20 - id: old-broken\n\
         \x20   label: [wrong-type]\n\
         \x20   command: old-cli\n"
    ));
    let store = fixture.load();
    assert!(store.contains_custom_provider_id("old-broken").unwrap());
    let mut next = store.snapshot().unwrap();
    next.config.custom_providers.push(provider());
    store
        .persist_with_backup(next.revision, next.config, existing_token)
        .unwrap();
    let rows = store.provider_listing().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], "synthetic-cli");
    assert_eq!(rows[0]["status"], "in-use");
    assert_eq!(rows[0]["reasons"], serde_json::json!([]));
    assert!(!store.contains_custom_provider_id("old-broken").unwrap());
    assert!(store.contains_custom_provider_id("synthetic-cli").unwrap());
}

#[test]
fn dropped_id_lookup_matches_full_ids_and_explicit_empty_ids() {
    let fixture = Fixture::new();
    let long_id = "x".repeat(80);
    fixture.write(format!(
        "token: {TOKEN}\ncustom_providers:\n\
         \x20 - id: {long_id}\n\
         \x20   label: [wrong-type]\n\
         \x20   command: synthetic-cli\n\
         \x20 - id: ''\n\
         \x20   command: empty-id-cli\n"
    ));
    let store = fixture.load();
    assert!(
        store
            .snapshot()
            .unwrap()
            .config
            .custom_providers
            .iter()
            .all(|p| p.id != long_id)
    );
    assert_ne!(
        store.startup_problems()[0].id.as_deref(),
        Some(long_id.as_str())
    );
    assert!(store.contains_custom_provider_id(&long_id).unwrap());
    assert!(store.contains_custom_provider_id("").unwrap());
    assert!(!store.contains_custom_provider_id("missing").unwrap());
    assert_eq!(store.provider_listing().unwrap()[0]["id"], long_id);

    let absent = Fixture::new();
    absent.write(format!(
        "token: {TOKEN}\ncustom_providers:\n  - command: no-id-cli\n"
    ));
    assert!(!absent.load().contains_custom_provider_id("").unwrap());
}

#[test]
fn in_memory_listing_falls_back_to_config_and_redacts_before_json_encoding() {
    let fixture = Fixture::new();
    let token = "synthetic\"private\nmarker";
    let mut config = Config::defaults(&fixture.paths);
    config.token = token.into();
    config.custom_providers.push(CustomProvider {
        id: token.into(),
        label: format!("label {token}"),
        command: format!("program {token}"),
        ..CustomProvider::default()
    });
    let store = ConfigStore::new(fixture.paths.clone(), config).unwrap();
    let rows = store.provider_listing().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], "[redacted]");
    assert_eq!(rows[0]["label"], "label [redacted]");
    assert_eq!(rows[0]["command"], "program [redacted]");
    assert!(!serde_json::to_string(&rows).unwrap().contains("synthetic"));
    assert!(store.contains_custom_provider_id(token).unwrap());
    assert!(!fixture.paths.root().exists());
}

#[test]
fn malformed_provider_section_is_a_dropped_row() {
    let fixture = Fixture::new();
    fixture.write(format!("token: {TOKEN}\ncustom_providers: wrong-type\n"));
    let store = fixture.load();
    let rows = store.provider_listing().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["status"], "dropped");
    assert_eq!(
        rows[0]["reasons"],
        serde_json::json!([store.startup_problems()[0].text_en().reason])
    );
    assert!(!store.contains_custom_provider_id("wrong-type").unwrap());
}

#[cfg(unix)]
#[test]
fn read_only_load_preserves_permissions_and_valid_save_makes_private_files() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    fixture.write(format!("token: {TOKEN}\n"));
    let file = fixture.paths.resource(Resource::Config);
    std::fs::set_permissions(fixture.paths.root(), std::fs::Permissions::from_mode(0o755))
        .unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
    let store = fixture.load();
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(fixture.paths.root()), 0o755);
    assert_eq!(mode(&file), 0o644);
    let snapshot = store.snapshot().unwrap();
    let (_, backup, _) = store
        .persist_with_backup(snapshot.revision, snapshot.config, existing_token)
        .unwrap();
    assert_eq!(mode(fixture.paths.root()), 0o700);
    assert_eq!(mode(&file), 0o600);
    assert_eq!(mode(&backup.unwrap().path), 0o600);
}
