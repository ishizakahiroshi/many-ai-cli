//! Source → Rust: internal/handoff/{handoff_test,render_test}.go → orchestration/handoff.rs.
use many_ai_cli::proto::time::{Timestamp, UNIX_EPOCH};
use many_ai_cli::{config::RuntimePaths, orchestration::handoff::*};
use serde::Deserialize;
use std::{fs, sync::Arc, time::Duration};
#[derive(Deserialize)]
struct Golden {
    id: i64,
    records: Option<Vec<Record>>,
    rendered: String,
    sanitized: Option<Vec<Record>>,
}
fn fixture() -> (tempfile::TempDir, tempfile::TempDir, HandoffStore) {
    let trial = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(trial.path(), 49321, installed.path()).unwrap();
    (trial, installed, HandoffStore::new(paths))
}
#[test]
fn actual_go_render_and_sanitize_corpus() {
    let cases: Vec<Golden> =
        serde_json::from_str(include_str!("fixtures/core/handoff/go-golden.json")).unwrap();
    assert_eq!(cases.len(), 7);
    for c in cases {
        let records = c.records.unwrap_or_default();
        assert_eq!(render_markdown(c.id, &records), c.rendered);
        assert_eq!(
            records.into_iter().map(sanitize).collect::<Vec<_>>(),
            c.sanitized.unwrap_or_default()
        );
    }
}
#[test]
fn typed_allowlist_is_exact_and_paths_are_never_opened() {
    let mut record = Record {
        version: 1,
        ts: "T".into(),
        session_id: 4,
        provider: "p".into(),
        cwd: "c".into(),
        branch: "b".into(),
        model: "m".into(),
        subscription_id: "s".into(),
        kind: "future".into(),
        files: vec!["f".into()],
        commit: "x".into(),
        commit_subject: "s".into(),
        added: 1,
        removed: 1,
        files_changed: 1,
        turn: 1,
        work_doc: "unread-workdoc".into(),
        text: "t".into(),
        handoff_from: 1,
        transcript: "unread-transcript".into(),
        note: "unread-note".into(),
    };
    let value = serde_json::to_value(&record).unwrap();
    let keys = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        keys,
        [
            "version",
            "ts",
            "session_id",
            "provider",
            "cwd",
            "branch",
            "model",
            "subscription_id",
            "kind",
            "files",
            "commit",
            "commit_subject",
            "added",
            "removed",
            "files_changed",
            "turn",
            "work_doc",
            "text",
            "handoff_from",
            "transcript",
            "note"
        ]
        .into_iter()
        .collect()
    );
    record.kind = "session_start".into();
    let rendered = render_markdown(4, &[record]);
    assert!(rendered.contains("unread-transcript") && rendered.contains("unread-note"));
}
#[test]
fn append_stamps_version_and_preserves_explicit_fields() {
    let (_t, _i, store) = fixture();
    store
        .append(
            42,
            Record {
                kind: "unknown_future".into(),
                version: 99,
                text: "done".into(),
                ..Default::default()
            },
            UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        )
        .unwrap();
    store
        .append(
            42,
            Record {
                kind: "note".into(),
                session_id: 9,
                ts: "literal time".into(),
                note: "/synthetic/path".into(),
                ..Default::default()
            },
            Timestamp::now(),
        )
        .unwrap();
    let records = store.read_session(42).unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].version, 1);
    assert_eq!(records[0].session_id, 42);
    assert_eq!(records[0].kind, "unknown_future");
    // Source uses local RFC3339 time; the same instant can land on November 15
    // east of UTC. Decode with chrono independently of the shared formatter.
    let stamped = chrono::DateTime::parse_from_rfc3339(&records[0].ts).unwrap();
    assert_eq!(stamped.timestamp(), 1_700_000_000);
    assert_eq!(stamped.timestamp_subsec_nanos(), 0);
    assert_eq!(records[1].session_id, 9);
    assert_eq!(records[1].ts, "literal time");
    assert!(store.read_session(999).unwrap().is_empty());
    let (path, md) = store.write_rendered(42).unwrap();
    assert_eq!(fs::read_to_string(path).unwrap(), md);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(store.directory())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(store.path_for(42).unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}
#[test]
fn concurrent_appends_keep_every_line_once() {
    let (_t, _i, store) = fixture();
    let store = Arc::new(store);
    let threads = (0..12)
        .map(|n| {
            let store = store.clone();
            std::thread::spawn(move || {
                store
                    .append(
                        1,
                        Record {
                            kind: "done".into(),
                            text: format!("entry-{n}"),
                            ..Default::default()
                        },
                        UNIX_EPOCH,
                    )
                    .unwrap();
            })
        })
        .collect::<Vec<_>>();
    for t in threads {
        t.join().unwrap();
    }
    let records = store.read_session(1).unwrap();
    assert_eq!(records.len(), 12);
    assert_eq!(
        records
            .iter()
            .map(|r| r.text.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        12
    );
    assert_eq!(fs::read_dir(store.directory()).unwrap().count(), 1);
}
#[test]
fn append_preserves_historical_bytes_and_existing_file_handle() {
    use std::io::{Read, Seek, SeekFrom, Write};
    let (_trial, _installed, store) = fixture();
    fs::create_dir_all(store.directory()).unwrap();
    let path = store.path_for(1).unwrap();
    let mut existing = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    existing.write_all(b"historical-invalid-record\n").unwrap();
    let prior_len = 8 * 1024 * 1024;
    existing.set_len(prior_len).unwrap();
    store
        .append(
            1,
            Record {
                kind: KIND_DONE.into(),
                text: "new completion".into(),
                ..Default::default()
            },
            UNIX_EPOCH,
        )
        .unwrap();
    // A still-open handle observes the append. Atomic replacement of an old
    // body would leave this handle at its old length and fail this assertion.
    assert!(existing.metadata().unwrap().len() > prior_len);
    existing.seek(SeekFrom::Start(prior_len)).unwrap();
    let mut appended = String::new();
    existing.read_to_string(&mut appended).unwrap();
    let record: Record = many_ai_cli::proto::decode_wire(appended.as_bytes()).unwrap();
    assert_eq!(record.text, "new completion");
    existing.rewind().unwrap();
    let mut prefix = [0; 26];
    existing.read_exact(&mut prefix).unwrap();
    assert_eq!(&prefix, b"historical-invalid-record\n");
}
#[test]
fn decode_skips_corruption_handles_go_case_nulls_and_unknown_fields() {
    let (t, _i, store) = fixture();
    fs::create_dir_all(store.directory()).unwrap();
    fs::write(store.path_for(1).unwrap(),concat!("{\"VERSION\":1,\"Kind\":\"done\",\"TEXT\":\"hello\",\"files\":null,\"ignored\":1e1000}\n","{bad\n","{\"kind\":\"bad earlier field\",\"session_id\":\"wrong\",\"session_id\":4}\n","{\"kind\":\"future\",\"transcript\":\"/synthetic/log\",\"content\":\"NOT_STORED\"}\n")).unwrap();
    let records = store.read_session(1).unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].text, "hello");
    assert_eq!(records[1].kind, "future");
    assert!(
        !serde_json::to_string(&records)
            .unwrap()
            .contains("NOT_STORED")
    );
    let (path, _) = store.write_rendered(1).unwrap();
    assert!(path.starts_with(t.path().canonicalize().unwrap()));
}
#[test]
fn oversized_record_reports_error_without_unbounded_allocation() {
    let (_t, _i, store) = fixture();
    fs::create_dir_all(store.directory()).unwrap();
    fs::write(
        store.path_for(1).unwrap(),
        vec![b'x'; MAX_RECORD_BYTES as usize + 1],
    )
    .unwrap();
    assert_eq!(
        store.read_session(1).unwrap_err().kind(),
        std::io::ErrorKind::InvalidData
    );
}
#[test]
fn retention_owns_only_handoff_suffixes_and_stats_count_jsonl() {
    let (_t, _i, store) = fixture();
    assert!(!store.stat_dir(Timestamp::now()).unwrap().exists);
    store
        .append(
            1,
            Record {
                kind: "done".into(),
                ..Default::default()
            },
            UNIX_EPOCH,
        )
        .unwrap();
    fs::write(
        store.note_path_for(1).unwrap(),
        b"secret memo body not read",
    )
    .unwrap();
    fs::write(store.manual_note_path_for(1).unwrap(), b"manual").unwrap();
    store.write_rendered(1).unwrap();
    fs::write(store.directory().join("unrelated.txt"), b"keep").unwrap();
    fs::create_dir(store.directory().join("directory.jsonl")).unwrap();
    assert_eq!(store.stat_dir(Timestamp::now()).unwrap().files, 1);
    store
        .prune_older_than(Timestamp::now() + Duration::from_secs(2))
        .unwrap();
    assert!(!store.path_for(1).unwrap().exists());
    assert!(!store.note_path_for(1).unwrap().exists());
    assert!(!store.manual_note_path_for(1).unwrap().exists());
    assert!(!store.rendered_path_for(1).unwrap().exists());
    assert!(store.directory().join("unrelated.txt").exists());
    assert!(store.directory().join("directory.jsonl").is_dir());
}
#[cfg(unix)]
#[test]
fn trial_handoff_symlink_cannot_write_installed_root() {
    let trial = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(trial.path(), 49321, installed.path()).unwrap();
    std::os::unix::fs::symlink(
        installed.path(),
        paths.resource(many_ai_cli::config::Resource::Handoff),
    )
    .unwrap();
    let store = HandoffStore::new(paths);
    assert!(
        store
            .append(1, Record::default(), Timestamp::now())
            .is_err()
    );
    assert!(store.read_session(1).is_err());
    assert!(store.write_rendered(1).is_err());
    assert!(store.stat_dir(Timestamp::now()).is_err());
    assert!(store.prune_older_than(Timestamp::now()).is_err());
    assert_eq!(fs::read_dir(installed.path()).unwrap().count(), 0);
}
#[cfg(unix)]
#[test]
fn leaf_symlinks_never_expose_or_prune_outside_content() {
    let (_trial, installed, store) = fixture();
    fs::create_dir_all(store.directory()).unwrap();
    let outside = installed.path().join("outside.jsonl");
    let secret = b"{\"kind\":\"done\",\"text\":\"outside body\"}\n";
    fs::write(&outside, secret).unwrap();
    let link = store.path_for(1).unwrap();
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    assert!(store.read_session(1).is_err());
    assert!(read_all(&link).is_err());
    assert!(store.write_rendered(1).is_err());
    assert!(store.append(1, Record::default(), UNIX_EPOCH).is_err());
    assert_eq!(store.stat_dir(Timestamp::now()).unwrap().files, 0);
    store
        .prune_older_than(Timestamp::now() + Duration::from_secs(1))
        .unwrap();
    assert_eq!(fs::read(&outside).unwrap(), secret);
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}
#[cfg(target_os = "linux")]
#[test]
fn concurrent_directory_exchange_cannot_redirect_handoff_io() {
    use std::{ffi::CString, os::unix::ffi::OsStrExt, sync::Barrier};
    let (trial, installed, store) = fixture();
    store
        .append(
            1,
            Record {
                kind: KIND_DONE.into(),
                text: "inside".into(),
                ..Default::default()
            },
            UNIX_EPOCH,
        )
        .unwrap();
    let outside = installed.path().join("s1.jsonl");
    let secret = b"{\"kind\":\"done\",\"text\":\"outside body\"}\n";
    fs::write(&outside, secret).unwrap();
    let directory = store.directory();
    let alternate = trial.path().join("swap-link");
    std::os::unix::fs::symlink(installed.path(), &alternate).unwrap();
    let root_name = CString::new(directory.as_os_str().as_bytes()).unwrap();
    let alternate_name = CString::new(alternate.as_os_str().as_bytes()).unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let gate = barrier.clone();
    let swapping = std::thread::spawn(move || {
        gate.wait();
        // Exchange keeps both names present, so append cannot recreate a gap.
        // Even count restores the original directory before fixture cleanup.
        for _ in 0..512 {
            // SAFETY: both NUL-terminated paths name this test's synthetic entries.
            let result = unsafe {
                libc::renameat2(
                    libc::AT_FDCWD,
                    root_name.as_ptr(),
                    libc::AT_FDCWD,
                    alternate_name.as_ptr(),
                    libc::RENAME_EXCHANGE,
                )
            };
            assert_eq!(result, 0, "{}", std::io::Error::last_os_error());
            std::thread::yield_now();
        }
    });
    barrier.wait();
    for _ in 0..128 {
        let _ = store.append(
            1,
            Record {
                kind: KIND_DONE.into(),
                text: "inside".into(),
                ..Default::default()
            },
            UNIX_EPOCH,
        );
        if let Ok(records) = read_all(&directory.join("s1.jsonl")) {
            assert!(records.iter().all(|record| record.text != "outside body"));
        }
        if let Ok((_, rendered)) = store.write_rendered(1) {
            assert!(!rendered.contains("outside body"));
        }
        let _ = store.stat_dir(Timestamp::now());
        let _ = store.prune_older_than(Timestamp::now() + Duration::from_secs(1));
    }
    swapping.join().unwrap();
    assert_eq!(fs::read(&outside).unwrap(), secret);
    assert_eq!(fs::read_dir(installed.path()).unwrap().count(), 1);
}
#[test]
fn rendered_replacement_failure_keeps_source_log() {
    let (_t, _i, store) = fixture();
    store
        .append(
            1,
            Record {
                kind: "done".into(),
                text: "preserve".into(),
                ..Default::default()
            },
            UNIX_EPOCH,
        )
        .unwrap();
    fs::create_dir(store.rendered_path_for(1).unwrap()).unwrap();
    assert!(store.write_rendered(1).is_err());
    assert_eq!(store.read_session(1).unwrap()[0].text, "preserve");
    assert_eq!(fs::read_dir(store.directory()).unwrap().count(), 2);
}
