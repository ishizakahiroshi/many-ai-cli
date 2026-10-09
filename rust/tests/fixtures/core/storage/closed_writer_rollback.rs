//! Staged integration test: closed-writer rollback, not live-data/cutover acceptance.
//! Root can include this file as an integration test after the active snapshot.
use many_ai_cli::{
    config::{Resource, RuntimePaths},
    proto::{core::*, time::UNIX_EPOCH},
    storage::SqliteSessionStorage,
};
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

fn open(paths: &RuntimePaths) -> SqliteSessionStorage {
    SqliteSessionStorage::open(
        paths,
        StorageOptions::baseline(paths.resource(Resource::Logs)),
    )
    .unwrap()
}
fn event(kind: &str, text: &str) -> HistoryEvent {
    HistoryEvent(
        json!({"type":kind,"text":text,"ts":"2025-01-01T01:00:00Z"})
            .as_object()
            .unwrap()
            .clone(),
    )
}
fn recovery_files(paths: &RuntimePaths) -> Vec<PathBuf> {
    let db = paths.resource(Resource::Database);
    vec![
        db.clone(),
        PathBuf::from(format!("{}-wal", db.display())),
        PathBuf::from(format!("{}-shm", db.display())),
        paths.resource(Resource::Config),
    ]
}

#[test]
fn closed_writer_copy_restore_go_continuation_and_rust_reread() {
    let root = tempfile::Builder::new()
        .prefix("many-ai-storage-oracle-")
        .tempdir()
        .unwrap();
    let installed = tempfile::tempdir().unwrap();
    let backup = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49129, installed.path()).unwrap();
    let original_config = b"hub:\n  port: 49129\nui:\n  theme: dark\n";
    std::fs::write(paths.resource(Resource::Config), original_config).unwrap();
    {
        let store = open(&paths);
        store
            .start_session(SessionStart {
                live_session_id: LiveSessionId(1),
                provider: "copilot".into(),
                display: "copilot".into(),
                cwd: "/synthetic/project".into(),
                started_at: "2025-01-01T00:00:00Z".into(),
                ..Default::default()
            })
            .unwrap();
        store
            .update_session_card_meta(
                LiveSessionId(1),
                SessionCardMeta {
                    label: "edited card".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        store
            .store_event(
                LiveSessionId(1),
                event("user_input", "synthetic rollback query"),
            )
            .unwrap();
        store
            .store_event(LiveSessionId(1), event("pty_output", "synthetic answer"))
            .unwrap();
        store.store_approval_detected(ApprovalDetected {
            live_session_id: LiveSessionId(1),
            sig: "rollback-approval".into(),
            source: "transcript".into(),
            provider: String::new(),
            options: vec![],
            kind: "marker".into(),
            question: "Continue?".into(),
            context: "synthetic".into(),
            block: "Q1 Continue?\n1. yes\n2. no".into(),
            candidate_key: "key-rollback-approval".into(),
            source_epoch: ApprovalSourceEpoch(3),
            detected_at: Some(UNIX_EPOCH + Duration::from_secs(100)),
        });
        store.store_approval_consumed(
            LiveSessionId(1),
            "rollback-approval",
            "yes",
            UNIX_EPOCH + Duration::from_secs(110),
        );
        store.close().unwrap();
    }
    // Every writer and SQLite handle is closed before copying the coherent set.
    // Preserve sidecar presence as well as bytes; never copy a bare live WAL DB.
    let files = recovery_files(&paths);
    let mut saved = Vec::new();
    for (index, path) in files.iter().enumerate() {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => panic!("closed recovery read failed: {error}"),
        };
        if let Some(bytes) = &bytes {
            std::fs::write(backup.path().join(index.to_string()), bytes).unwrap();
        }
        saved.push(bytes);
    }
    assert!(saved[0].as_ref().is_some_and(|bytes| !bytes.is_empty()));
    assert_eq!(saved[3].as_deref(), Some(original_config.as_slice()));
    {
        let store = open(&paths);
        store
            .update_session_card_meta(
                LiveSessionId(1),
                SessionCardMeta {
                    label: "candidate changed card".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        store
            .store_event(
                LiveSessionId(1),
                event("user_input", "candidate-only mutation must disappear"),
            )
            .unwrap();
        assert_eq!(
            store
                .chat_messages_by_live_session(LiveSessionId(1), 100)
                .unwrap()
                .unwrap()
                .len(),
            3
        );
        assert_eq!(
            store
                .session_card_meta_by_live_session(LiveSessionId(1))
                .unwrap()
                .label,
            "candidate changed card"
        );
        store.close().unwrap();
    }
    std::fs::write(
        paths.resource(Resource::Config),
        b"hub:\n  port: 49130\nui:\n  theme: light\n",
    )
    .unwrap();
    assert_ne!(
        std::fs::read(&files[0]).unwrap(),
        saved[0].as_ref().unwrap().as_slice()
    );
    assert_ne!(std::fs::read(&files[3]).unwrap(), original_config);
    // Restore only explicit fixture-owned paths, after candidate writer close.
    for (index, path) in files.iter().enumerate() {
        match &saved[index] {
            Some(expected) => {
                std::fs::copy(backup.path().join(index.to_string()), path).unwrap();
                assert_eq!(&std::fs::read(path).unwrap(), expected);
            }
            None => {
                match std::fs::remove_file(path) {
                    Ok(()) => (),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                    Err(error) => panic!("fixture sidecar removal failed: {error}"),
                };
                assert!(!path.exists());
            }
        }
    }
    std::fs::write(
        root.path().join("synthetic-storage-fixture"),
        b"Go/Rust storage compatibility only\n",
    )
    .unwrap();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let oracle_source = root.path().join("oracle-source");
    copy_verified_go_sources(repo, &oracle_source).unwrap();
    std::fs::write(
        oracle_source.join("rollback-oracle.go"),
        include_bytes!("rollback-oracle.go"),
    )
    .unwrap();
    let go = std::env::var_os("MANY_AI_GO_BINARY").unwrap_or_else(|| "go".into());
    let mut command = std::process::Command::new(go);
    command
        .current_dir(&oracle_source)
        .args([
            "run",
            "-mod=readonly",
            "-buildvcs=false",
            "rollback-oracle.go",
        ])
        .arg(root.path())
        .env("GOTOOLCHAIN", "local")
        .env("GOWORK", "off")
        .env("GOPROXY", "off")
        .env("GOSUMDB", "off")
        .env_remove("GOFLAGS");
    for variable in ["GOCACHE", "GOMODCACHE", "GOPATH", "XDG_CACHE_HOME"] {
        command.env(
            variable,
            std::env::var_os(variable).unwrap_or_else(|| {
                root.path()
                    .join(variable.to_ascii_lowercase())
                    .into_os_string()
            }),
        );
    }
    let output = command
        .output()
        .expect("pinned Go toolchain required via MANY_AI_GO_BINARY");
    assert!(
        output.status.success(),
        "isolated Go read/write failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let receipt: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        receipt,
        json!({"go_baseline":"21d0bc7935a2c4696fb89ccff2e324157a528c2d","sessions":1,"messages":2,"approvals":1,"search":1,"go_write":true})
    );
    let reopened = open(&paths);
    let messages = reopened
        .chat_messages_by_live_session(LiveSessionId(1), 100)
        .unwrap()
        .unwrap();
    assert_eq!(
        messages
            .iter()
            .map(|message| message.raw_text.as_str())
            .collect::<Vec<_>>(),
        vec![
            "synthetic rollback query",
            "synthetic answer",
            "synthetic Go rollback continuation"
        ]
    );
    assert_eq!(
        reopened
            .session_card_meta_by_live_session(LiveSessionId(1))
            .unwrap()
            .label,
        "edited card"
    );
    assert!(
        reopened
            .search_messages("mutation", 10)
            .unwrap()
            .unwrap_or_default()
            .is_empty()
    );
    assert_eq!(
        reopened
            .search_messages("continuation", 10)
            .unwrap()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        std::fs::read(paths.resource(Resource::Config)).unwrap(),
        original_config
    );
    reopened.close().unwrap();
}

fn copy_verified_go_sources(
    source: &std::path::Path,
    destination: &std::path::Path,
) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    #[derive(serde::Deserialize)]
    struct Source {
        path: String,
        sha256: String,
    }
    #[derive(serde::Deserialize)]
    struct Manifest {
        baseline: String,
        files: Vec<Source>,
    }
    let manifest: Manifest =
        serde_json::from_str(include_str!("go-source-manifest.json")).map_err(|e| e.to_string())?;
    if manifest.baseline != "21d0bc7935a2c4696fb89ccff2e324157a528c2d" {
        return Err("unexpected Go baseline manifest".into());
    }
    for file in manifest.files {
        let relative = std::path::Path::new(&file.path);
        if relative
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
        {
            return Err("invalid source manifest path".into());
        }
        let raw = std::fs::read(source.join(relative))
            .map_err(|_| format!("missing Go oracle source {}", file.path))?;
        let canonical = raw
            .iter()
            .enumerate()
            .filter_map(|(i, b)| {
                if *b == b'\r' && raw.get(i + 1) == Some(&b'\n') {
                    None
                } else {
                    Some(*b)
                }
            })
            .collect::<Vec<_>>();
        let digest = Sha256::digest(&canonical)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        if digest != file.sha256 {
            return Err(format!(
                "Go oracle source differs from the fixed manifest: {}",
                file.path
            ));
        }
        let target = destination.join(relative);
        std::fs::create_dir_all(target.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(target, canonical).map_err(|e| e.to_string())?;
    }
    Ok(())
}
