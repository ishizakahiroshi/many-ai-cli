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
    _root: tempfile::TempDir,
    _installed: tempfile::TempDir,
    http: HandoffHttp,
    hooks: Arc<Hooks>,
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
