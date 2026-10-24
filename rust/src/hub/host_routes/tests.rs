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

#[tokio::test]
async fn terminal_app_failed_persist_keeps_memory_and_destination_then_recovers() {
    let f = fixture();
    let destination = f.http.paths.resource(Resource::Config);
    std::fs::create_dir(&destination).unwrap();
    let sentinel = destination.join("preserved");
    std::fs::write(&sentinel, b"synthetic existing destination").unwrap();
    let before = f.http.config.snapshot().unwrap();
    let response = f
        .http
        .handle_authenticated(&request(
            "/api/terminal-app",
            serde_json::json!({"terminal_app":"  synthetic app.exe  "}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status, 500);
    let after = f.http.config.snapshot().unwrap();
    assert_ne!(after.revision, before.revision);
    assert_eq!(after.config.terminal_app, "synthetic app.exe");
    assert!(destination.is_dir());
    assert_eq!(
        std::fs::read(&sentinel).unwrap(),
        b"synthetic existing destination"
    );
    assert_eq!(std::fs::read_dir(&destination).unwrap().count(), 1);
    assert!(f.dispatch.opened.lock().unwrap().is_empty());
    assert!(f.dispatch.picked.lock().unwrap().is_empty());

    // Only the two synthetic fixture objects are removed to let the same
    // owner's normal persistence path run after its earlier save failure.
    std::fs::remove_file(sentinel).unwrap();
    std::fs::remove_dir(&destination).unwrap();
    let response = f
        .http
        .handle_authenticated(&request(
            "/api/terminal-app",
            serde_json::json!({"terminal_app":"second synthetic app.exe"}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(
        f.http.config.snapshot().unwrap().config.terminal_app,
        "second synthetic app.exe"
    );
    assert!(
        std::fs::read_to_string(destination)
            .unwrap()
            .contains("second synthetic app.exe")
    );
    assert!(f.dispatch.opened.lock().unwrap().is_empty());
    assert!(f.dispatch.picked.lock().unwrap().is_empty());
}

#[tokio::test]
async fn logical_remote_pin_scope_and_physical_open_guards_stay_distinct() {
    use crate::application::agent_history::AgentHistory;
    async fn owned_response(router: &crate::hub::ServiceRouter, request: &Request) -> Response {
        let at = crate::proto::time::Timestamp::from_unix(1001, 0).unwrap();
        // Follow the transport's real dispatch selection. Synchronous routes
        // do not pass through the owner reserved for native/accepted work.
        if !crate::hub::router::needs_owned_request(&request.path) {
            return router.handle_async_at(request, at).await.response;
        }
        router
            .handle_owned_async_at(
                request,
                at,
                &TaskCancellation::default(),
                &HttpWaitCancellation::default(),
            )
            .await
            .response
    }
    let f = fixture();
    let paths = f.http.paths.clone();
    let config = f.http.config.clone();
    let dispatch = f.dispatch.clone();
    let mut initial = config.snapshot().unwrap();
    initial.config.token = "synthetic".into();
    initial.config.remote_pin_hash = bcrypt::hash("123456", 4).unwrap();
    initial.config.auth_cookie_secret = "synthetic-secret".into();
    initial.config.hub.allowed_hosts = vec!["synthetic.test".into()];
    config.persist(initial.revision, initial.config).unwrap();
    let before = std::fs::read(paths.resource(Resource::Config)).unwrap();
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
    let transcript = paths.root().join("synthetic-rollout.jsonl");
    std::fs::write(&transcript, b"synthetic transcript").unwrap();
    let at = crate::proto::time::Timestamp::from_unix(1000, 0).unwrap();
    let binding = core
        .register(
            RegisterRequest {
                message: crate::proto::Message {
                    provider: "codex".into(),
                    pid: 6011,
                    cwd: paths.root().to_string_lossy().into_owned(),
                    home_dir: paths.root().join("actor").to_string_lossy().into_owned(),
                    cols: 120,
                    rows: 30,
                    ..Default::default()
                },
                spawn_proof: None,
            },
            WrapperConnectionId(6011),
            at,
        )
        .await
        .unwrap()
        .binding;
    core.apply_observation(
        binding,
        SessionObservation::Transcript {
            path: transcript,
            agent_session_id: "synthetic-thread".into(),
            safe_offset: 0,
            grew_at: String::new(),
        },
        at,
    )
    .unwrap();
    // Normal history path policy still uses only this explicit temporary root;
    // the OS opener is the recording fake, so no actual GUI is opened.
    let history_paths = RuntimePaths::production(paths.root()).unwrap();
    let history = AgentHistory::new(
        core,
        history_paths,
        paths.root().join("actor"),
        dispatch.clone(),
        Arc::new(|_| panic!("no artifact reads in guard matrix")),
    );
    let hints = Arc::new(crate::hub::runtime_routes::NetHints::default());
    let router = crate::hub::ServiceRouter::isolated(config, paths.clone())
        .with_host(Arc::new(f.http))
        .with_agent_history(Arc::new(
            crate::hub::agent_history_routes::AgentHistoryHttp::new(history),
        ))
        .with_runtime_observations(hints.clone(), "synthetic-shell".into());
    let mut forwarded = Request {
        method: "POST".into(),
        path: "/api/open-folder".into(),
        host: "127.0.0.1:49297".into(),
        remote_addr: "127.0.0.1:1234".into(),
        query: "token=synthetic".into(),
        headers: vec![
            ("X-Forwarded-For".into(), "203.0.113.10".into()),
            ("Origin".into(), "http://127.0.0.1:49297".into()),
        ],
        body: serde_json::to_vec(&serde_json::json!({"path":paths.root()})).unwrap(),
        ..Default::default()
    };
    assert!(crate::hub::auth::logically_remote(&forwarded));
    assert_eq!(owned_response(&router, &forwarded).await.status, 401);
    assert!(dispatch.opened.lock().unwrap().is_empty());
    let login_cookie = |peer: &str| {
        let mut login = forwarded.clone();
        login.remote_addr = peer.into();
        login.path = "/api/auth/login".into();
        login.body = br#"{"pin":"123456"}"#.to_vec();
        let response = router.handle(&login, 1000).response;
        assert_eq!(response.status, 200);
        response.cookies[0].split(';').next().unwrap().to_owned()
    };
    let local_cookie = login_cookie("127.0.0.1:1234");
    let remote_cookie = login_cookie("203.0.113.1:1234");
    forwarded.headers.push(("Cookie".into(), local_cookie));
    assert_eq!(owned_response(&router, &forwarded).await.status, 200);
    assert_eq!(dispatch.opened.lock().unwrap().len(), 1);
    forwarded.body = serde_json::to_vec(&serde_json::json!({"path":f._installed.path()})).unwrap();
    assert_eq!(owned_response(&router, &forwarded).await.status, 403);
    assert_eq!(dispatch.opened.lock().unwrap().len(), 1);
    forwarded.path = "/api/agent-log/open".into();
    forwarded.query.push_str("&session_id=bad");
    assert_eq!(owned_response(&router, &forwarded).await.status, 400);
    forwarded.query = format!("token=synthetic&session_id={}", binding.session.0);
    assert_eq!(owned_response(&router, &forwarded).await.status, 200);
    assert_eq!(
        dispatch.opened.lock().unwrap()[1],
        (
            OpenKind::Directory,
            paths.root().to_path_buf(),
            String::new()
        )
    );
    let mut remote = forwarded.clone();
    remote.remote_addr = "203.0.113.1:1234".into();
    remote.headers.last_mut().unwrap().1 = remote_cookie;
    assert_eq!(owned_response(&router, &remote).await.status, 403);
    remote.path = "/api/open-folder".into();
    remote.body = serde_json::to_vec(&serde_json::json!({"path":paths.root()})).unwrap();
    assert_eq!(owned_response(&router, &remote).await.status, 403);
    assert_eq!(dispatch.opened.lock().unwrap().len(), 2);
    forwarded.path = "/api/net-hint".into();
    forwarded.body = br#"{"ssh":true,"host_label":"synthetic","env_kind":"remote"}"#.to_vec();
    assert_eq!(owned_response(&router, &forwarded).await.status, 403);
    assert!(!hints.snapshot().ssh);
    forwarded
        .headers
        .retain(|(name, _)| name != "X-Forwarded-For");
    assert!(!crate::hub::auth::logically_remote(&forwarded));
    assert_eq!(owned_response(&router, &forwarded).await.status, 200);
    assert!(hints.snapshot().ssh);
    forwarded.host = "synthetic.test:49297".into();
    forwarded.body = br#"{"ssh":false}"#.to_vec();
    assert!(crate::hub::auth::logically_remote(&forwarded));
    assert_eq!(owned_response(&router, &forwarded).await.status, 403);
    assert!(hints.snapshot().ssh);
    forwarded.path = "/api/open-folder".into();
    forwarded.body = serde_json::to_vec(&serde_json::json!({"path":paths.root()})).unwrap();
    assert_eq!(owned_response(&router, &forwarded).await.status, 200);
    assert_eq!(dispatch.opened.lock().unwrap().len(), 3);
    assert_eq!(
        std::fs::read(paths.resource(Resource::Config)).unwrap(),
        before
    );
    assert!(dispatch.picked.lock().unwrap().is_empty());
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
