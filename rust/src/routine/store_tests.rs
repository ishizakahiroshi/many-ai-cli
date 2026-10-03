use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::UNIX_EPOCH;
fn now() -> SystemTime {
    UNIX_EPOCH + Duration::new(1_700_000_000, 123_456_789)
}
fn setup() -> (tempfile::TempDir, Arc<RoutineStore>, Arc<AtomicBool>) {
    let root = tempfile::tempdir().unwrap();
    let fail = Arc::new(AtomicBool::new(false));
    let flag = fail.clone();
    let store = RoutineStore::with_writer(
        Arc::new(Dir::open(root.path()).unwrap()),
        Arc::new(move |dir, bytes| {
            if flag.load(Ordering::SeqCst) {
                Err(io::Error::other("synthetic disk failure"))
            } else {
                dir.replace("routines.json", bytes, 0o600)
            }
        }),
    );
    (root, Arc::new(store), fail)
}
fn definition(root: &Path) -> Definition {
    let project = root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    Definition {
        name: "  Review changes  ".into(),
        cwd: project.to_string_lossy().into(),
        provider: "codex".into(),
        prompt: "  Summarize changes.  ".into(),
        ..Default::default()
    }
}
fn saved(root: &Path, store: &RoutineStore) -> Definition {
    store.save(None, definition(root), root, now()).unwrap()
}
fn summary(id: i64, text: &str) -> DoneSummary {
    DoneSummary {
        session_id: id,
        text: text.into(),
        at: "2023-11-14T22:14:00Z".into(),
        ..Default::default()
    }
}
fn obs(run: &Run, state: &str, last: Option<SystemTime>) -> Observation {
    Observation {
        id: LiveSessionId(42),
        db_id: Some(DbSessionId(700)),
        launch_label: run.session_label.clone(),
        state: state.into(),
        waiting: false,
        last_output: last,
    }
}
#[test]
fn persisted_concurrent_admission_and_alias_retry_are_single_run() {
    let (root, store, _) = setup();
    let definition = saved(root.path(), &store);
    let threads: Vec<_> = (0..20)
        .map(|_| {
            let store = store.clone();
            let id = definition.id.clone();
            std::thread::spawn(move || store.admit(&id, "same-click", "manual", now()).unwrap())
        })
        .collect();
    let results: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|a| !a.existing).count(), 1);
    let run = &results[0].run;
    assert!(results.iter().all(|a| a.run.id == run.id));
    let disk: RoutineFile =
        crate::proto::decode_wire(&std::fs::read(root.path().join("routines.json")).unwrap())
            .unwrap();
    assert_eq!(disk.runs.len(), 1);
    assert_eq!(disk.runs[0].status, "starting");
    assert_eq!(
        disk.requests[&format!("{}|same-click", definition.id)],
        run.id
    );
    assert!(
        store
            .admit(&definition.id, "second-click", "manual", now())
            .unwrap()
            .existing
    );
    store
        .record_done(
            &run.session_label,
            Some(DbSessionId(700)),
            &summary(42, "Reviewed. Tests were not run."),
            "synthetic-hub",
        )
        .unwrap();
    let reopened = RoutineStore::open(Arc::new(Dir::open(root.path()).unwrap()));
    for key in ["same-click", "second-click"] {
        let result = reopened
            .admit(&definition.id, key, "manual", now())
            .unwrap();
        assert!(result.existing);
        assert_eq!(result.run.id, run.id);
        assert_eq!(result.run.session_id, 42);
        assert_eq!(result.run.session_db_id, 700);
    }
}
#[test]
fn failed_saves_do_not_publish_definitions_runs_or_aliases() {
    let (root, store, fail) = setup();
    let definition = saved(root.path(), &store);
    let before = std::fs::read(root.path().join("routines.json")).unwrap();
    fail.store(true, Ordering::SeqCst);
    let mut changed = definition.clone();
    changed.name = "changed".into();
    assert_eq!(
        store
            .save(Some(&definition.id), changed, root.path(), now())
            .unwrap_err(),
        Error::Operation
    );
    assert_eq!(
        store
            .admit(&definition.id, "first", "manual", now())
            .unwrap_err(),
        Error::Operation
    );
    assert!(store.runs("").unwrap().is_empty());
    assert_eq!(store.definitions().unwrap()[0].name, "Review changes");
    assert_eq!(
        std::fs::read(root.path().join("routines.json")).unwrap(),
        before
    );
    fail.store(false, Ordering::SeqCst);
    let first = store
        .admit(&definition.id, "first", "manual", now())
        .unwrap();
    fail.store(true, Ordering::SeqCst);
    assert_eq!(
        store
            .admit(&definition.id, "alias", "manual", now())
            .unwrap_err(),
        Error::Operation
    );
    assert!(
        !store
            .state
            .lock()
            .unwrap()
            .data
            .requests
            .contains_key(&format!("{}|alias", definition.id))
    );
    assert_eq!(
        store.run(&first.run.id).unwrap().unwrap().status,
        "starting"
    );
}
#[test]
fn corrupt_and_historical_stores_are_not_destructively_validated() {
    for bytes in [
        b"{broken".as_slice(),
        b"{\"version\":2}",
        b"{\"version\":1,\"runs\":false}",
    ] {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("routines.json"), bytes).unwrap();
        let store = RoutineStore::open(Arc::new(Dir::open(root.path()).unwrap()));
        assert!(!store.ready());
        assert!(matches!(store.definitions(), Err(Error::Unavailable)));
        assert_eq!(
            std::fs::read(root.path().join("routines.json")).unwrap(),
            bytes
        );
    }
    let root = tempfile::tempdir().unwrap();
    let bytes=br#"{"version":1,"routines":[{"id":"old","name":"","cwd":"relative","provider":"old-provider","schedule":{"kind":"old"}}],"runs":null,"requests":null}"#;
    std::fs::write(root.path().join("routines.json"), bytes).unwrap();
    let store = RoutineStore::open(Arc::new(Dir::open(root.path()).unwrap()));
    assert!(store.ready());
    assert_eq!(store.definitions().unwrap()[0].provider, "old-provider");
    assert!(store.runs("").unwrap().is_empty());
    assert_eq!(
        std::fs::read(root.path().join("routines.json")).unwrap(),
        bytes
    );
}
#[test]
fn crud_validates_new_input_detects_conflicts_and_retains_history() {
    let (root, store, _) = setup();
    let definition = saved(root.path(), &store);
    assert_eq!(definition.completion_mode, "native_or_marker");
    assert_eq!(definition.schedule.kind, "manual");
    assert_eq!(definition.prompt, "Summarize changes.");
    let mut edit = definition.clone();
    edit.updated_at = "stale".into();
    assert_eq!(
        store
            .save(Some(&definition.id), edit, root.path(), now())
            .unwrap_err(),
        Error::Changed
    );
    let admitted = store
        .admit(&definition.id, "once", "manual", now())
        .unwrap();
    assert_eq!(store.delete(&definition.id), Err(Error::Running));
    store
        .record_done(
            &admitted.run.session_label,
            None,
            &summary(42, "complete"),
            "hub",
        )
        .unwrap();
    store.delete(&definition.id).unwrap();
    assert!(store.definitions().unwrap().is_empty());
    assert_eq!(store.runs("").unwrap().len(), 1);
    for (field, value) in [
        ("prompt", "bad\rtext"),
        ("prompt", "bad\u{7f}"),
        ("model", "--flag"),
        ("model", "model arg"),
        ("cwd", root.path().to_str().unwrap()),
        ("provider", "shell"),
    ] {
        let mut item = definition.clone();
        match field {
            "prompt" => item.prompt = value.into(),
            "model" => item.model = value.into(),
            "cwd" => item.cwd = value.into(),
            "provider" => item.provider = value.into(),
            _ => unreachable!(),
        };
        assert!(
            matches!(
                store.save(None, item, root.path(), now()),
                Err(Error::Invalid(_))
            ),
            "{field}"
        );
    }
}
#[test]
fn pending_result_replacement_and_committed_result_immutability_match_go() {
    let (root, store, fail) = setup();
    let definition = saved(root.path(), &store);
    let run = store
        .admit(&definition.id, "once", "manual", now())
        .unwrap()
        .run;
    fail.store(true, Ordering::SeqCst);
    let go: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../tests/fixtures/services/routines-results-go.json"
    ))
    .unwrap();
    for (index, text) in ["first pending", "second pending"].into_iter().enumerate() {
        assert_eq!(
            store.record_done(&run.session_label, None, &summary(42, text), "hub"),
            Err(Error::Operation)
        );
        let state = store.state.lock().unwrap();
        assert_eq!(state.data.runs[0].status, go[index]["run"]["status"]);
        assert_eq!(state.pending_results[&run.id].result, go[index]["pending"]);
    }
    assert_eq!(store.run(&run.id).unwrap().unwrap().status, "starting");
    fail.store(false, Ordering::SeqCst);
    store.refresh(&[], "hub", now()).unwrap();
    assert_eq!(
        store.run(&run.id).unwrap().unwrap().result,
        "second pending"
    );
    store
        .record_done(
            &run.session_label,
            None,
            &summary(42, "must not replace committed"),
            "hub",
        )
        .unwrap();
    store.launched(&run.id, Err(()), "hub", now()).unwrap();
    assert_eq!(
        store.run(&run.id).unwrap().unwrap().result,
        "second pending"
    );
    assert_eq!(store.run(&run.id).unwrap().unwrap().status, "finished");
}
#[test]
fn observation_uses_launch_label_exact_clocks_and_separate_ids() {
    let (root, store, _) = setup();
    let definition = saved(root.path(), &store);
    let run = store
        .admit(&definition.id, "once", "manual", now())
        .unwrap()
        .run;
    store
        .refresh(
            &[obs(&run, "standby", Some(now()))],
            "new-hub",
            now() + Duration::from_secs(30),
        )
        .unwrap();
    assert_eq!(store.run(&run.id).unwrap().unwrap().status, "running");
    store
        .refresh(
            &[obs(&run, "standby", Some(now()))],
            "new-hub",
            now() + Duration::new(30, 1),
        )
        .unwrap();
    let got = store.run(&run.id).unwrap().unwrap();
    assert_eq!(got.status, "waiting");
    assert_eq!((got.session_id, got.session_db_id), (42, 700));
    assert_eq!(got.hub_instance_id, "new-hub");
    assert!(!got.result_available);
    let mut waiting = obs(&run, "running", Some(now()));
    waiting.waiting = true;
    store.refresh(&[waiting], "new-hub", now()).unwrap();
    assert!(store.run(&run.id).unwrap().unwrap().error.is_empty());
    store.refresh(&[], "hub", now()).unwrap();
    store
        .refresh(&[], "hub", now() + Duration::from_secs(90))
        .unwrap();
    assert_eq!(store.run(&run.id).unwrap().unwrap().status, "waiting");
    store
        .refresh(&[], "hub", now() + Duration::new(90, 1))
        .unwrap();
    assert_eq!(store.run(&run.id).unwrap().unwrap().status, "interrupted");
    let reopened = RoutineStore::open(Arc::new(Dir::open(root.path()).unwrap()));
    assert_eq!(reopened.runs("").unwrap()[0].status, "interrupted");
}
#[test]
fn done_ignores_fallback_masks_and_bounds_utf8_without_inventing_results() {
    let (root, store, _) = setup();
    let definition = saved(root.path(), &store);
    let run = store
        .admit(&definition.id, "once", "manual", now())
        .unwrap()
        .run;
    let mut fallback = summary(42, "inferred");
    fallback.fallback = true;
    store
        .record_done(&run.session_label, None, &fallback, "hub")
        .unwrap();
    assert_eq!(store.run(&run.id).unwrap().unwrap().status, "starting");
    // Construct a synthetic masking marker at runtime, never a credential.
    let marker = ["API", "_KEY="].concat();
    let text = format!(
        "  test\nresult {marker}{} {}",
        "x".repeat(24),
        "日".repeat(90_000)
    );
    store
        .record_done(&run.session_label, None, &summary(42, &text), "hub")
        .unwrap();
    let done = store.run(&run.id).unwrap().unwrap();
    assert!(done.result_available && done.result_truncated);
    assert!(done.result.contains(&format!("{marker}***")));
    assert!(!done.result.contains(&"x".repeat(24)));
    assert!(done.result.len() <= 256 * 1024);
    assert!(done.summary.ends_with('…'));
    assert_eq!(done.summary.chars().count(), 321);
    assert!(!store.active_session(&run.session_label, now()));
    assert_eq!(
        store.run_url(&run.session_label),
        Some(format!("/?routine_run={}", run.id))
    );
}
#[test]
fn changed_schedule_cannot_admit_a_stale_tick() {
    let (root, store, _) = setup();
    let mut definition = definition(root.path());
    definition.enabled = true;
    definition.schedule = super::super::schedule::Schedule {
        kind: "daily".into(),
        time: "10:00".into(),
        timezone: "UTC".into(),
    };
    let definition = store.save(None, definition, root.path(), now()).unwrap();
    let due = definition.next_run_at.clone();
    let mut changed = definition.clone();
    changed.enabled = false;
    store
        .save(Some(&definition.id), changed, root.path(), now())
        .unwrap();
    assert_eq!(
        store
            .admit(
                &definition.id,
                &format!("schedule:{due}"),
                "schedule",
                now()
            )
            .unwrap_err(),
        Error::ScheduleChanged
    );
    assert!(store.runs("").unwrap().is_empty());
}
#[cfg(unix)]
#[test]
fn held_store_directory_survives_path_replacement() {
    let root = tempfile::tempdir().unwrap();
    let held = root.path().join("held");
    std::fs::create_dir(&held).unwrap();
    let store = RoutineStore::open(Arc::new(Dir::open(&held).unwrap()));
    let moved = root.path().join("moved");
    std::fs::rename(&held, &moved).unwrap();
    std::fs::create_dir(&held).unwrap();
    let def = saved(root.path(), &store);
    store.admit(&def.id, "once", "manual", now()).unwrap();
    assert!(!held.join("routines.json").exists());
    assert!(moved.join("routines.json").exists());
}

#[test]
fn fixed_go_record_decoder_corpus_preserves_historical_values() {
    #[derive(serde::Deserialize)]
    struct Case {
        input: String,
        ok: bool,
        output: Option<serde_json::Value>,
    }
    let cases: Vec<Case> = serde_json::from_slice(include_bytes!(
        "../../tests/fixtures/services/routines-store-go.json"
    ))
    .unwrap();
    assert_eq!(cases.len(), 12);
    for case in cases {
        let decoded = RoutineFile::decode(case.input.as_bytes());
        assert_eq!(decoded.is_ok(), case.ok, "{}", case.input);
        if let Ok(decoded) = decoded {
            let mut expected = case.output.unwrap();
            // Go copyLocked allocates [] before the next store mutation and HTTP
            // list handlers allocate []; the historical bytes themselves remain.
            for field in ["routines", "runs"] {
                if expected[field].is_null() {
                    expected[field] = serde_json::json!([]);
                }
            }
            assert_eq!(
                serde_json::to_value(decoded).unwrap(),
                expected,
                "{}",
                case.input
            );
        }
    }
}

#[test]
fn definition_limit_and_history_window_are_independent() {
    let (root, store, _) = setup();
    let original = saved(root.path(), &store);
    {
        let mut state = store.state.lock().unwrap();
        for n in 1..200 {
            let mut def = original.clone();
            def.id = format!("historical-{n}");
            state.data.routines.push(def);
        }
        for n in 0..205 {
            state.data.runs.push(Run {
                id: format!("run-{n}"),
                routine_id: original.id.clone(),
                status: "finished".into(),
                ..Default::default()
            });
        }
    }
    assert_eq!(
        store
            .save(None, definition(root.path()), root.path(), now())
            .unwrap_err(),
        Error::Limit
    );
    let runs = store.runs(&original.id).unwrap();
    assert_eq!(runs.len(), 200);
    assert_eq!(runs[0].id, "run-204");
    assert_eq!(runs[199].id, "run-5");
    let mut item = original.clone();
    item.name = "日".repeat(67);
    assert!(matches!(
        store.save(Some(&original.id), item, root.path(), now()),
        Err(Error::Invalid(_))
    ));
    let mut item = original.clone();
    item.prompt = "日".repeat(2731);
    assert!(matches!(
        store.save(Some(&original.id), item, root.path(), now()),
        Err(Error::Invalid(_))
    ));
}
