use super::*;
use crate::{
    config::Config,
    proto::core::*,
    terminal::{
        events::CoreEventBus,
        journal::{JournalOptions, SessionJournal},
        session::{EngineOptions, SessionEngine},
    },
};
use std::sync::Mutex;
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
struct Dispatch {
    opened: Mutex<Vec<(OpenKind, PathBuf, String)>>,
    picked: Mutex<Vec<PickerKind>>,
}
impl HostDispatch for Dispatch {
    fn pick<'a>(&'a self, kind: PickerKind) -> CoreFuture<'a, std::io::Result<String>> {
        Box::pin(async move {
            self.picked.lock().unwrap().push(kind);
            Ok("synthetic-selection".into())
        })
    }
    fn open(&self, kind: OpenKind, path: &Path, app: &str) -> std::io::Result<()> {
        self.opened
            .lock()
            .unwrap()
            .push((kind, path.into(), app.into()));
        Ok(())
    }
}
struct Fixture {
    _root: tempfile::TempDir,
    _installed: tempfile::TempDir,
    http: HostHttp,
    dispatch: Arc<Dispatch>,
}
fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49297, installed.path()).unwrap();
    std::fs::create_dir_all(paths.root()).unwrap();
    let config = Arc::new(ConfigStore::new(paths.clone(), Config::defaults(&paths)).unwrap());
    let core = Arc::new(SessionEngine::new(
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
    let files = Arc::new(FilesService::new(paths.root().to_owned(), paths.clone()));
    let dispatch = Arc::new(Dispatch::default());
    Fixture {
        http: HostHttp::new(paths, config, core, files, dispatch.clone(), None),
        dispatch,
        _root: root,
        _installed: installed,
    }
}
fn request(path: &str, body: serde_json::Value) -> Request {
    Request {
        method: "POST".into(),
        path: path.into(),
        remote_addr: "127.0.0.1:49997".into(),
        body: serde_json::to_vec(&body).unwrap(),
        ..Default::default()
    }
}
fn value(r: Response) -> serde_json::Value {
    serde_json::from_slice(&r.body).unwrap()
}
#[tokio::test]
async fn completion_preserves_raw_exists_keys_sorted_hidden_filter_and_remote_boundaries() {
    let f = fixture();
    let root = f.http.paths.root();
    for name in ["zeta", "alpha", ".hidden"] {
        std::fs::create_dir(root.join(name)).unwrap();
    }
    std::fs::write(root.join("file"), b"synthetic").unwrap();
    let raw = root.to_string_lossy().into_owned();
    let response = f
        .http
        .handle_authenticated(&request(
            "/api/path-exists",
            serde_json::json!({"paths":["",raw,"alpha","file","missing"]}),
        ))
        .await
        .unwrap();
    let body = value(response);
    assert_eq!(body["results"][&raw], true);
    assert_eq!(body["results"]["alpha"], true);
    assert_eq!(body["results"]["file"], false);
    assert!(body["results"].get("").is_none());
    let result = value(
        f.http
            .handle_authenticated(&request(
                "/api/list-subdirs",
                serde_json::json!({"path":format!("  {raw}  ")}),
            ))
            .await
            .unwrap(),
    );
    assert_eq!(result["subdirs"], serde_json::json!(["alpha", "zeta"]));
    let empty = value(
        f.http
            .handle_authenticated(&request(
                "/api/list-subdirs",
                serde_json::json!({"path":" "}),
            ))
            .await
            .unwrap(),
    );
    assert!(empty.get("path").is_none());
    assert_eq!(empty["subdirs"], serde_json::json!([]));
    let mut remote = request(
        "/api/list-subdirs",
        serde_json::json!({"path":f._installed.path()}),
    );
    remote.remote_addr = "192.0.2.1:49997".into();
    assert_eq!(
        f.http.handle_authenticated(&remote).await.unwrap().status,
        403
    );
}
#[tokio::test]
async fn host_calls_validate_peer_before_body_and_exact_scopes_before_dispatch() {
    let f = fixture();
    let root = f.http.paths.root();
    let document = root.join("synthetic & document.txt");
    std::fs::write(&document, b"fixture").unwrap();
    let mut remote = request(
        "/api/open-default-file",
        serde_json::json!({"path":document}),
    );
    remote.remote_addr = "192.0.2.1:4".into();
    remote.body = b"invalid".to_vec();
    assert_eq!(
        f.http.handle_authenticated(&remote).await.unwrap().status,
        403
    );
    assert!(f.dispatch.opened.lock().unwrap().is_empty());
    let denied = request(
        "/api/open-default-file",
        serde_json::json!({"path":root.join("payload.Ps1")}),
    );
    assert_eq!(
        f.http.handle_authenticated(&denied).await.unwrap().status,
        403
    );
    let outside = request(
        "/api/open-terminal",
        serde_json::json!({"path":f._installed.path()}),
    );
    assert_eq!(
        f.http.handle_authenticated(&outside).await.unwrap().status,
        403
    );
    assert_eq!(
        f.http
            .handle_authenticated(&request(
                "/api/open-folder",
                serde_json::json!({"path":root})
            ))
            .await
            .unwrap()
            .status,
        200
    );
    assert_eq!(f.dispatch.opened.lock().unwrap()[0].1, root);
    assert_eq!(
        f.http
            .handle_authenticated(&request(
                "/api/open-default-file",
                serde_json::json!({"path":document,"paths":false})
            ))
            .await
            .unwrap()
            .status,
        200
    );
    assert_eq!(
        f.dispatch.opened.lock().unwrap()[1],
        (OpenKind::File, document, String::new())
    );
    let mut malformed = request(
        "/api/open-dir",
        serde_json::json!({"kind":"path","path":root}),
    );
    malformed.remote_addr = "127.0.0.1".into();
    assert_eq!(
        value(f.http.handle_authenticated(&malformed).await.unwrap())["detail"],
        "loopback remote address required"
    );
}
#[tokio::test]
async fn picker_ignores_body_terminal_setting_publishes_before_failed_save() {
    let f = fixture();
    let mut picker = request("/api/pick-file", serde_json::json!(false));
    picker.query = "filter=exe".into();
    picker.body = b"invalid".to_vec();
    assert_eq!(
        value(f.http.handle_authenticated(&picker).await.unwrap())["path"],
        "synthetic-selection"
    );
    assert_eq!(
        *f.dispatch.picked.lock().unwrap(),
        vec![PickerKind::File { executable: true }]
    );
    std::fs::create_dir(f.http.paths.resource(Resource::Config)).unwrap();
    let response = f
        .http
        .handle_authenticated(&request(
            "/api/terminal-app",
            serde_json::json!({"terminal_app":"  synthetic app.exe  "}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status, 500);
    assert_eq!(
        f.http.config.snapshot().unwrap().config.terminal_app,
        "synthetic app.exe"
    );
    let mut get = Request {
        method: "GET".into(),
        path: "/api/terminal-app".into(),
        remote_addr: "192.0.2.1:4".into(),
        ..Default::default()
    };
    assert_eq!(
        value(f.http.handle_authenticated(&get).await.unwrap())["effective_terminal_app"],
        "synthetic app.exe <dir>"
    );
    get.method = "DELETE".into();
    assert_eq!(f.http.handle_authenticated(&get).await.unwrap().status, 405);
}
#[test]
fn source_path_casefold_uses_unicode_classes_and_component_boundary() {
    assert!(ancestor(
        Path::new("/synthetic/root"),
        Path::new("/synthetic/root/child")
    ));
    assert!(!ancestor(
        Path::new("/synthetic/root"),
        Path::new("/synthetic/root2")
    ));
    #[cfg(windows)]
    {
        assert!(ancestor(Path::new(r"C:\Σ"), Path::new(r"c:\ς\child")));
        assert!(!ancestor(Path::new(r"C:\İ"), Path::new(r"c:\i\child")));
    }
}
