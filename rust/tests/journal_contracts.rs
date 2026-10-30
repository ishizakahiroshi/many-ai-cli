use many_ai_cli::proto::time::{Timestamp, UNIX_EPOCH};
use many_ai_cli::{
    config::{Resource, RuntimePaths},
    proto::core::*,
    storage::SqliteSessionStorage,
    terminal::journal::*,
};
use std::{sync::Arc, time::Duration};
fn fixture(
    enabled: bool,
    max_bytes: i64,
) -> (
    tempfile::TempDir,
    tempfile::TempDir,
    Arc<SqliteSessionStorage>,
    SessionJournal,
) {
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49120, installed.path()).unwrap();
    let store = Arc::new(
        SqliteSessionStorage::open(
            &paths,
            StorageOptions::baseline(paths.resource(Resource::Logs)),
        )
        .unwrap(),
    );
    store
        .start_session(SessionStart {
            live_session_id: LiveSessionId(1),
            provider: "copilot".into(),
            started_at: "2026-01-02T03:04:05Z".into(),
            ..Default::default()
        })
        .unwrap();
    let journal = SessionJournal::new(
        paths,
        Some(store.clone()),
        JournalOptions {
            session_enabled: enabled,
            max_bytes,
        },
    );
    (root, installed, store, journal)
}
fn event(kind: &str, text: &str) -> HistoryEvent {
    HistoryEvent(
        serde_json::json!({"type":kind,"text":text,"ts":"2026-01-02T03:04:05Z"})
            .as_object()
            .unwrap()
            .clone(),
    )
}
fn write(j: &SessionJournal, kind: &str, text: &str) {
    bound_apply(
        j,
        PersistenceEffect::Event {
            session: LiveSessionId(1),
            event: event(kind, text),
        },
    )
    .unwrap();
}
fn instant() -> Timestamp {
    UNIX_EPOCH + Duration::from_secs(1767323045)
}
#[test]
fn disabled_body_gate_keeps_registration_metadata_but_neither_body_store() {
    let (_r, _i, store, journal) = fixture(false, 0);
    let paths = journal
        .open(
            LiveSessionId(1),
            "copilot",
            "/synthetic/project",
            instant(),
            false,
        )
        .unwrap();
    write(&journal, "user_input", "synthetic private body");
    bound_apply(
        &journal,
        PersistenceEffect::CardMeta {
            session: LiveSessionId(1),
            meta: SessionCardMeta {
                label: "retained label".into(),
                ..Default::default()
            },
        },
    )
    .unwrap();
    assert!(!paths.jsonl.exists());
    assert_eq!(
        store
            .session_card_meta_by_live_session(LiveSessionId(1))
            .unwrap()
            .label,
        "retained label"
    );
    assert_eq!(store.usage_summary().unwrap().total_messages, 0);
    store.close().unwrap();
}
fn attach(name: &str) -> HistoryEvent {
    HistoryEvent(
        serde_json::json!({"type":"attach","filename":name,"path":"","ts":"2026-01-02T03:04:05Z"})
            .as_object()
            .unwrap()
            .clone(),
    )
}
fn write_attach(j: &SessionJournal, name: &str) {
    bound_apply(
        j,
        PersistenceEffect::Event {
            session: LiveSessionId(1),
            event: attach(name),
        },
    )
    .unwrap();
}
#[test]
fn no_jsonl_file_is_written_and_events_reach_sqlite_once_in_order() {
    // A size limit used to cap the `.jsonl`; with no file to cap it changes nothing.
    let (_r, _i, store, journal) = fixture(true, 1);
    let paths = journal
        .open(
            LiveSessionId(1),
            "copilot",
            "/synthetic/project",
            instant(),
            false,
        )
        .unwrap();
    write_attach(&journal, "first");
    write_attach(&journal, "second");
    write(&journal, "user_input", "synthetic private body");
    journal.close_all();
    store.close().unwrap();
    assert!(!paths.jsonl.exists());
    let connection = rusqlite::Connection::open(store.database_path()).unwrap();
    let names: Vec<String> = connection
        .prepare("SELECT text FROM messages ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(names, ["first", "second"]);
}
#[test]
fn paths_are_sanitized_and_runtime_derived_for_multibyte_and_empty_parts() {
    let (root, _i, _store, j) = fixture(false, 0);
    let paths = j
        .paths(
            LiveSessionId(3),
            " <provider>/ ",
            "/fixture/../../danger:?*",
            instant(),
        )
        .unwrap();
    assert!(
        paths
            .jsonl
            .starts_with(root.path().canonicalize().unwrap().join("logs/sessions"))
    );
    assert!(
        paths
            .jsonl
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .ends_with("_danger____s3.jsonl")
    );
    assert_eq!(sanitize_file_part(".."), "no-project");
    assert_eq!(sanitize_file_part(&"界".repeat(30)).len(), 78);
    assert_eq!(sanitize_file_part("a  b\nc"), "a_b_c");
}
#[cfg(unix)]
#[test]
fn a_symlink_planted_at_the_old_jsonl_path_is_never_written_through() {
    // The `.jsonl` used to be created here (and an existing symlink was refused).
    // Nothing is written there any more, so a planted link must stay exactly as it is.
    use std::{fs, os::unix::fs::symlink};
    let (_r, _i, _s, j) = fixture(true, 0);
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("untouched");
    fs::write(&target, b"original").unwrap();
    let paths = j
        .paths(LiveSessionId(1), "copilot", "/synthetic/project", instant())
        .unwrap();
    fs::create_dir_all(paths.jsonl.parent().unwrap()).unwrap();
    symlink(&target, &paths.jsonl).unwrap();
    j.open(
        LiveSessionId(1),
        "copilot",
        "/synthetic/project",
        instant(),
        false,
    )
    .unwrap();
    write_attach(&j, "first");
    write(&j, "user_input", "synthetic private body");
    j.close_all();
    assert_eq!(fs::read(&target).unwrap(), b"original");
    assert!(
        fs::symlink_metadata(&paths.jsonl)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn reattach_log_names_retain_validated_source_wall_clock_offset() {
    let (_r, _i, _s, j) = fixture(false, 0);
    for (source, expected) in [
        ("2026-01-02T03:04:05+09:00", "2026-01-02_030405"),
        ("2026-01-01T18:04:05Z", "2026-01-01_180405"),
        ("2026-01-02T3:04:05,001-07:00", "2026-01-02_030405"),
    ] {
        let p = j
            .paths_for_timestamp(LiveSessionId(1), "copilot", "/fixture/project", source)
            .unwrap();
        assert!(
            p.jsonl
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .contains(expected)
        );
    }
    assert!(
        j.paths_for_timestamp(
            LiveSessionId(1),
            "copilot",
            "/fixture/project",
            "../../escape"
        )
        .is_err()
    );
}

#[test]
fn byte_masking_matches_actual_go_rune_thresholds_and_preserves_unmatched_invalid_bytes() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "fixtures/core/journal/mask-oracle-21d0bc7.json"
    ))
    .unwrap();
    assert_eq!(cases.len(), 16);
    for (index, case) in cases.iter().enumerate() {
        let input = STANDARD
            .decode(case["input_b64"].as_str().unwrap())
            .unwrap();
        let expected = STANDARD
            .decode(case["masked_b64"].as_str().unwrap())
            .unwrap();
        let masked = many_ai_cli::storage::mask_secret_bytes(&input);
        assert_eq!(masked, expected, "Go byte mask case {index}");
        let text = many_ai_cli::approval::identity::strip_ansi(
            &many_ai_cli::proto::wire::go_utf8_lossy(&masked),
        );
        assert_eq!(
            text,
            case["text"].as_str().unwrap(),
            "Go JSON text case {index}"
        );
    }
}

#[test]
fn source_tolerated_persistence_failure_warns_without_aborting_remaining_effects() {
    let (_r, _i, store, journal) = fixture(true, 0);
    let warnings = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = warnings.clone();
    journal.set_warning_handler(Arc::new(move |id, error| {
        captured.lock().unwrap().push((id, error.clone()))
    }));
    let paths = journal
        .open(
            LiveSessionId(1),
            "copilot",
            "/synthetic/project",
            instant(),
            false,
        )
        .unwrap();
    store.close().unwrap();
    bound_apply(
        &journal,
        PersistenceEffect::ClearSessionHistory(LiveSessionId(1)),
    )
    .unwrap();
    // A failed database write only warns; later effects are still accepted.
    write_attach(&journal, "still accepted");
    assert!(!warnings.lock().unwrap().is_empty());
    assert!(!paths.jsonl.exists());
}

fn bound_apply(journal: &SessionJournal, effect: PersistenceEffect) -> Result<(), SessionError> {
    journal.apply_bound(
        SessionBinding {
            session: LiveSessionId(1),
            incarnation: SessionIncarnation(0),
            wrapper: WrapperConnectionId(0),
        },
        PersistenceBindingScope::Incarnation,
        effect,
    )
}
