use super::*;
use crate::{
    config::Config,
    terminal::{
        events::CoreEventBus,
        journal::{JournalOptions, SessionJournal},
        session::{EngineOptions, SessionEngine},
    },
};
use std::sync::atomic::{AtomicUsize, Ordering};
struct Boundary;
impl WrapperTransport for Boundary {
    fn send<'a>(
        &'a self,
        _: SessionBinding,
        _: crate::proto::Message,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async { panic!("read-only handoff never sends terminal input") })
    }
}
impl CoreEffectSink for Boundary {
    fn apply<'a>(&'a self, _: CoreEffects) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
        Box::pin(async { panic!("read-only handoff never emits session effects") })
    }
}
impl WrappedSessionSpawner for Boundary {
    fn spawn_and_wait<'a>(
        &'a self,
        _: WrappedSpawnSpec,
        _: std::time::Duration,
        _: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome> {
        Box::pin(async { panic!("handoff HTTP never starts provider") })
    }
}
#[derive(Default)]
struct Hooks {
    ensures: AtomicUsize,
    notes: AtomicUsize,
}
impl HandoffHttpHooks for Hooks {
    fn ensure_transcript(&self, _: LiveSessionId, _: Timestamp) -> Result<(), SessionError> {
        self.ensures.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn request_note<'a>(
        &'a self,
        _: LiveSessionId,
        _: Timestamp,
    ) -> CoreFuture<'a, Result<PathBuf, &'static str>> {
        Box::pin(async move {
            self.notes.fetch_add(1, Ordering::SeqCst);
            Err("session_not_writable")
        })
    }
    fn warning(&self, _: &'static str) {
        panic!("unexpected synthetic storage failure")
    }
}
struct Fixture {
    http: HandoffHttp,
    hooks: Arc<Hooks>,
    // Last: fields drop in this order, and on Windows the folder cannot be
    // removed while a field above still holds a file inside it.
    _root: tempfile::TempDir,
    _installed: tempfile::TempDir,
}
fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49277, installed.path()).unwrap();
    let config = Arc::new(ConfigStore::new(paths.clone(), Config::defaults(&paths)).unwrap());
    let hooks = Arc::new(Hooks::default());
    let engine = Arc::new(SessionEngine::new(
        EngineOptions::default(),
        Arc::new(SessionJournal::new(
            paths.clone(),
            None,
            JournalOptions::default(),
        )),
        Arc::new(Boundary),
        Arc::new(Boundary),
        Arc::new(Boundary),
        CoreEventBus::new(32).unwrap(),
    ));
    Fixture {
        http: HandoffHttp::new(paths, config, engine, hooks.clone()),
        hooks,
        _root: root,
        _installed: installed,
    }
}
fn request(method: &str, path: &str, body: &str) -> Request {
    Request {
        method: method.into(),
        path: path.into(),
        body: body.as_bytes().to_owned(),
        ..Default::default()
    }
}
fn now() -> Timestamp {
    Timestamp::from_unix(1791158400, 0).unwrap()
}
fn value(response: Response) -> serde_json::Value {
    serde_json::from_slice(&response.body).unwrap()
}
#[tokio::test]
async fn absent_board_preview_and_list_do_not_create_records_or_rendered_files() {
    let f = fixture();
    let list = value(
        f.http
            .handle_authenticated(&request("GET", PATH, ""), now())
            .await
            .unwrap(),
    );
    assert!(list["entries"].is_null());
    let preview = value(
        f.http
            .handle_authenticated(&request("GET", "/api/handoff/999/", ""), now())
            .await
            .unwrap(),
    );
    assert_eq!(preview["ok"], true);
    assert_eq!(preview["exists"], false);
    assert_eq!(preview["live"], false);
    assert_eq!(preview["candidate_providers"].as_array().unwrap().len(), 7);
    assert!(!f.http.store.directory().exists());
    assert_eq!(f.hooks.ensures.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn source_path_actions_validate_method_before_id_and_note_ignores_body() {
    let f = fixture();
    for (method, path, status) in [
        ("POST", "/api/handoff/not-id", 405),
        ("GET", "/api/handoff/1/note", 405),
        ("POST", "/api/handoff/0/note", 400),
        ("GET", "/api/handoff/1/unknown", 404),
        ("POST", "/api/handoff/1/note", 404),
    ] {
        let response = f
            .http
            .handle_authenticated(&request(method, path, "invalid body"), now())
            .await
            .unwrap();
        assert_eq!(response.status, status, "{path}");
    }
    assert_eq!(f.hooks.notes.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn manual_note_masks_body_records_path_and_list_chooses_newest_successor() {
    let f = fixture();
    for (id, from) in [(1, 0), (2, 1), (3, 1)] {
        f.http
            .store
            .append(
                id,
                Record {
                    kind: KIND_SESSION_START.into(),
                    provider: "codex".into(),
                    handoff_from: from,
                    ..Default::default()
                },
                now(),
            )
            .unwrap();
    }
    let response = f
        .http
        .handle_authenticated(
            &request(
                "POST",
                "/api/handoff/1/manual-note",
                r#"{"text":"synthetic handoff Bearer abcdefghijklmnop"} trailing"#,
            ),
            now(),
        )
        .await
        .unwrap();
    assert_eq!(response.status, 200);
    let note = f.http.store.manual_note_path_for(1).unwrap();
    assert_eq!(
        std::fs::read_to_string(&note).unwrap(),
        "synthetic handoff Bearer ***"
    );
    let preview = value(
        f.http
            .handle_authenticated(&request("GET", "/api/handoff/1", ""), now())
            .await
            .unwrap(),
    );
    assert_eq!(preview["note_path"], note.to_string_lossy().as_ref());
    assert!(preview["markdown"].as_str().unwrap().contains("codex"));
    assert_eq!(preview["note_paths"].as_array().unwrap().len(), 1);
    let list = value(
        f.http
            .handle_authenticated(&request("GET", PATH, ""), now())
            .await
            .unwrap(),
    );
    let entries = list["entries"].as_array().unwrap();
    assert_eq!(
        entries
            .iter()
            .map(|e| e["session_id"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        vec![3, 2, 1]
    );
    assert_eq!(entries[2]["handoff_to"], 3);
    assert!(entries[2].get("note_paths").is_none());
    assert_eq!(f.http.store.read_session(1).unwrap().len(), 2);
}

#[tokio::test]
async fn handoff_get_dispatches_a_recorded_transcript_effect_and_replaces_rendered_output() {
    use crate::orchestration::handoff::KIND_TRANSCRIPT;
    use std::sync::Mutex;
    struct RecordedTranscript {
        store: HandoffStore,
        calls: Mutex<Vec<LiveSessionId>>,
        pending: Mutex<Option<(LiveSessionId, PathBuf)>>,
        appends: AtomicUsize,
    }
    impl HandoffHttpHooks for RecordedTranscript {
        fn ensure_transcript(&self, id: LiveSessionId, at: Timestamp) -> Result<(), SessionError> {
            self.calls.lock().unwrap().push(id);
            let mut pending = self.pending.lock().unwrap();
            if pending.as_ref().is_some_and(|(target, _)| *target == id) {
                let (_, path) = pending.take().unwrap();
                // This is an explicitly staged dependency effect, not the real
                // SessionWorkers callback or a duplicate implementation of its
                // eligibility policy. The real handoff store performs the write.
                self.store
                    .append(
                        id.0,
                        Record {
                            kind: KIND_TRANSCRIPT.into(),
                            transcript: path.to_string_lossy().into_owned(),
                            ..Default::default()
                        },
                        at,
                    )
                    .unwrap();
                self.appends.fetch_add(1, Ordering::SeqCst);
            }
            Ok(())
        }
        fn request_note<'a>(
            &'a self,
            _: LiveSessionId,
            _: Timestamp,
        ) -> CoreFuture<'a, Result<PathBuf, &'static str>> {
            panic!("GET does not request a provider note")
        }
        fn warning(&self, _: &'static str) {
            panic!("synthetic storage must succeed")
        }
    }
    let mut f = fixture();
    let transcript = f._root.path().join("synthetic-transcript.jsonl");
    std::fs::write(&transcript, b"synthetic transcript content\n").unwrap();
    f.http
        .store
        .append(
            1,
            Record {
                kind: KIND_SESSION_START.into(),
                provider: "codex".into(),
                ..Default::default()
            },
            now(),
        )
        .unwrap();
    let hook = Arc::new(RecordedTranscript {
        store: f.http.store.clone(),
        calls: Mutex::new(Vec::new()),
        pending: Mutex::new(Some((LiveSessionId(1), transcript.clone()))),
        appends: AtomicUsize::new(0),
    });
    f.http.hooks = hook.clone();
    let rendered = f.http.store.rendered_path_for(1).unwrap();
    assert!(!rendered.exists());
    let listed = f
        .http
        .handle_authenticated(&request("GET", PATH, ""), now())
        .await
        .unwrap();
    assert_eq!(listed.status, 200);
    assert!(hook.calls.lock().unwrap().is_empty());
    assert_eq!(hook.appends.load(Ordering::SeqCst), 0);
    assert!(!rendered.exists());

    let absent = value(
        f.http
            .handle_authenticated(&request("GET", "/api/handoff/999", ""), now())
            .await
            .unwrap(),
    );
    assert_eq!(absent["exists"], false);
    assert_eq!(*hook.calls.lock().unwrap(), vec![LiveSessionId(999)]);
    assert_eq!(hook.appends.load(Ordering::SeqCst), 0);
    assert!(!f.http.store.path_for(999).unwrap().exists());
    assert!(!f.http.store.rendered_path_for(999).unwrap().exists());
    assert!(hook.pending.lock().unwrap().is_some());

    std::fs::write(&rendered, b"stale rendered sentinel").unwrap();
    for expected_calls in [2, 3] {
        let preview = value(
            f.http
                .handle_authenticated(&request("GET", "/api/handoff/1", ""), now())
                .await
                .unwrap(),
        );
        assert_eq!(hook.calls.lock().unwrap().len(), expected_calls);
        assert_eq!(hook.calls.lock().unwrap().last(), Some(&LiveSessionId(1)));
        assert_eq!(hook.appends.load(Ordering::SeqCst), 1);
        assert!(hook.pending.lock().unwrap().is_none());
        let records = f.http.store.read_session(1).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[1].kind, KIND_TRANSCRIPT);
        assert_eq!(records[1].transcript, transcript.to_string_lossy().as_ref());
        assert_eq!(
            preview["transcript_path"],
            transcript.to_string_lossy().as_ref()
        );
        let actual_rendered = std::fs::read_to_string(&rendered).unwrap();
        assert_eq!(preview["markdown"], actual_rendered);
        assert!(actual_rendered.contains("codex"));
        assert!(!actual_rendered.contains("stale rendered sentinel"));
        assert_eq!(
            std::fs::read(&transcript).unwrap(),
            b"synthetic transcript content\n"
        );
        // The second GET must write the rendered file again without appending
        // a second staged transcript record.
        if expected_calls == 2 {
            std::fs::write(&rendered, b"stale rendered sentinel").unwrap();
        }
    }
}

#[tokio::test]
async fn preview_and_list_show_only_the_latest_launch_of_a_reused_number() {
    // Issue #46: the preview and list read the same launch as the rendered
    // handoff. The older launch stays in the file but must not leak its end,
    // transcript, note or successor link.
    let f = fixture();
    let old_transcript = f._root.path().join("old-transcript.jsonl");
    let old_note = f._root.path().join("old.note.md");
    std::fs::write(&old_transcript, b"synthetic\n").unwrap();
    std::fs::write(&old_note, b"synthetic\n").unwrap();
    let old_transcript = old_transcript.to_string_lossy().into_owned();
    let old_note = old_note.to_string_lossy().into_owned();
    f.http
        .store
        .append(
            2,
            Record {
                kind: KIND_SESSION_START.into(),
                provider: "codex".into(),
                ..Default::default()
            },
            now(),
        )
        .unwrap();
    let lines = [
        serde_json::json!({"version":1,"ts":"2026-10-01T09:00:00Z","session_id":6,"kind":"session_start","provider":"codex","cwd":"/old/project","model":"old-model","handoff_from":2}),
        serde_json::json!({"version":1,"ts":"2026-10-01T10:00:00Z","session_id":6,"kind":"transcript","transcript":old_transcript}),
        serde_json::json!({"version":1,"ts":"2026-10-01T10:01:00Z","session_id":6,"kind":"note","note":old_note}),
        serde_json::json!({"version":1,"ts":"2026-10-30T12:00:00Z","session_id":6,"kind":"session_end","model":"old-end-model","text":"exit"}),
        serde_json::json!({"version":1,"ts":"2026-11-02T08:00:00Z","session_id":6,"kind":"session_start","provider":"claude","cwd":"/new/project","model":"new-model"}),
        serde_json::json!({"version":1,"ts":"2026-11-01T08:00:00Z","session_id":6,"kind":"session_end","text":"backwards"}),
    ];
    let path = f.http.store.path_for(6).unwrap();
    let body: String = lines.iter().map(|line| format!("{line}\n")).collect();
    std::fs::write(&path, &body).unwrap();

    let preview = value(
        f.http
            .handle_authenticated(&request("GET", "/api/handoff/6", ""), now())
            .await
            .unwrap(),
    );
    assert_eq!(preview["exists"], true);
    assert_eq!(preview["provider"], "claude");
    assert_eq!(preview["cwd"], "/new/project");
    assert_eq!(preview["model"], "new-model");
    assert_eq!(preview["started_at"], "2026-11-02T08:00:00Z");
    for absent in [
        "ended_at",
        "transcript_path",
        "note_path",
        "note_paths",
        "handoff_from",
    ] {
        assert!(preview.get(absent).is_none(), "{absent}: {preview}");
    }
    let markdown = preview["markdown"].as_str().unwrap();
    for old in [
        old_transcript.as_str(),
        old_note.as_str(),
        "/old/project",
        "old-model",
        "2026-10-30T12:00:00Z",
        "backwards",
    ] {
        assert!(!markdown.contains(old), "{old} leaked into:\n{markdown}");
    }

    let list = value(
        f.http
            .handle_authenticated(&request("GET", PATH, ""), now())
            .await
            .unwrap(),
    );
    let entries = list["entries"].as_array().unwrap();
    let entry = |id: i64| {
        entries
            .iter()
            .find(|e| e["session_id"] == id)
            .unwrap_or_else(|| panic!("#{id} missing: {list}"))
    };
    assert_eq!(entry(6)["provider"], "claude");
    assert!(entry(6).get("ended_at").is_none(), "{list}");
    assert!(entry(6).get("transcript_path").is_none(), "{list}");
    assert!(entry(2).get("handoff_to").is_none(), "{list}");
    // Reading never rewrites the record file.
    assert_eq!(std::fs::read_to_string(&path).unwrap(), body);
}

#[tokio::test]
async fn preview_keeps_what_was_recorded_before_a_reattach() {
    // A Hub restart that reattaches the running CLI under the same number
    // writes session_reattach, not session_start: the launch goes on (#46).
    let f = fixture();
    let note = f._root.path().join("before-restart.note.md");
    std::fs::write(&note, b"synthetic\n").unwrap();
    let note = note.to_string_lossy().into_owned();
    let lines = [
        serde_json::json!({"version":1,"ts":"2026-11-02T08:00:00Z","session_id":8,"kind":"session_start","provider":"claude","cwd":"/project"}),
        serde_json::json!({"version":1,"ts":"2026-11-02T08:10:00Z","session_id":8,"kind":"note","note":note}),
        serde_json::json!({"version":1,"ts":"2026-11-02T09:00:00Z","session_id":8,"kind":"session_reattach","provider":"claude","cwd":"/project","model":"reattach-model"}),
    ];
    let path = f.http.store.path_for(8).unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let body: String = lines.iter().map(|line| format!("{line}\n")).collect();
    std::fs::write(&path, &body).unwrap();

    let preview = value(
        f.http
            .handle_authenticated(&request("GET", "/api/handoff/8", ""), now())
            .await
            .unwrap(),
    );
    assert_eq!(preview["started_at"], "2026-11-02T08:00:00Z");
    assert_eq!(preview["model"], "reattach-model");
    assert_eq!(preview["note_path"], note.as_str());
    assert!(preview["markdown"].as_str().unwrap().contains(&note));
}
