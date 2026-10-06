//! Real Git and the built, normal-mode Hub communicate over authenticated
//! WebSockets. This target runs on native Windows through `--all-targets`, as
//! well as Unix; no mocked Git, provider installation, or account is required.

use futures_util::{FutureExt, SinkExt, StreamExt};
use many_ai_cli::{
    config::{Config, ConfigStore, Resource, RuntimePaths},
    process::{ExitOutcome, ManagedProcess, ProcessEvent, ProcessPlan, SpawnOptions, pid_alive},
    proto,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    net::{Ipv4Addr, TcpListener},
    panic::AssertUnwindSafe,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::{net::TcpStream, time::Instant};
use tokio_tungstenite::{WebSocketStream, client_async, tungstenite::Message};

// Harness deadlines allow cold native Windows Git launches and several real
// refresh cycles. They do not change the product's 250ms Git / 2s refresh limits.
const STEP: Duration = Duration::from_secs(30);

struct Fixture {
    // Retained until all fixture-owned processes have been terminated/reaped.
    _temporary: tempfile::TempDir,
    paths: RuntimePaths,
    home: PathBuf,
    repo: PathBuf,
    nested: PathBuf,
    worktree: PathBuf,
    outside: PathBuf,
    template: PathBuf,
    git: PathBuf,
    environment: BTreeMap<OsString, Option<OsString>>,
    port: u16,
    token: String,
    client: reqwest::Client,
}

impl Fixture {
    fn new(port: u16) -> Self {
        let temporary = tempfile::tempdir().unwrap();
        // Resolve macOS's system-owned /var or /tmp alias before the production
        // instruction-home walk. Keep ordinary drive paths on Windows.
        let root = if cfg!(windows) {
            temporary.path().to_path_buf()
        } else {
            temporary.path().canonicalize().unwrap()
        };
        let home = root.join("synthetic-home");
        let repo = root.join("repository");
        let nested = repo.join("nested");
        let outside = root.join("not-a-repository");
        let template = root.join("empty-git-template");
        let worktree = root.join("linked-worktree");
        let paths = RuntimePaths::production(&home).unwrap();
        for directory in [&home, &nested, &outside, &template, paths.root()] {
            fs::create_dir_all(directory).unwrap();
        }

        let path = std::env::var_os("PATH").expect("native Git must be available on PATH");
        let git_name = if cfg!(windows) { "git.exe" } else { "git" };
        let git = std::env::split_paths(&path)
            .filter(|directory| directory.is_absolute())
            .map(|directory| directory.join(git_name))
            .find(|candidate| candidate.is_file())
            .expect("native Git is required; this regression must not be skipped");
        let mut environment = BTreeMap::new();
        environment.insert("PATH".into(), Some(path));
        // These are OS launch inputs, never account/credential variables.
        for key in ["SystemRoot", "WINDIR", "COMSPEC", "PATHEXT"] {
            if let Some(value) = std::env::var_os(key) {
                environment.insert(key.into(), Some(value));
            }
        }
        for key in [
            "HOME",
            "USERPROFILE",
            "APPDATA",
            "LOCALAPPDATA",
            "XDG_CONFIG_HOME",
            "XDG_CACHE_HOME",
            "XDG_DATA_HOME",
            "TEMP",
            "TMP",
            "TMPDIR",
        ] {
            environment.insert(key.into(), Some(home.as_os_str().to_owned()));
        }
        let git_config = root.join("empty-gitconfig");
        fs::write(&git_config, "").unwrap();
        environment.insert(
            "GIT_CONFIG_GLOBAL".into(),
            Some(git_config.into_os_string()),
        );
        environment.insert("GIT_CONFIG_NOSYSTEM".into(), Some("1".into()));
        environment.insert("GIT_ATTR_NOSYSTEM".into(), Some("1".into()));
        environment.insert("GIT_TERMINAL_PROMPT".into(), Some("0".into()));
        environment.insert(
            "GIT_CEILING_DIRECTORIES".into(),
            Some(root.into_os_string()),
        );
        environment.insert("LC_ALL".into(), Some("C".into()));

        let token = many_ai_cli::process::random_token().unwrap();
        let mut config = Config::defaults(&paths);
        config.token = token.clone();
        config.hub.port = i64::from(port);
        config.hub.open_browser = false;
        config.hub.auto_shutdown = false;
        config.hub.stale_binary_auto_restart = false;
        config.notify.backends = Some(vec![]);
        config.notify.events = Some(vec![]);
        // Normal mode synchronizes approval sources at startup. Point every
        // source to a private empty file so this fixture performs no HTTP fetch.
        let source = paths.root().join("empty-source.md");
        fs::write(&source, "").unwrap();
        let mut sources = serde_json::to_value(&config.approval_pattern_sources).unwrap();
        for value in sources.as_object_mut().unwrap().values_mut() {
            *value = json!(source);
        }
        config.approval_pattern_sources = serde_json::from_value(sources).unwrap();
        let store = ConfigStore::new(paths.clone(), config.clone()).unwrap();
        store.persist(0, config).unwrap();

        Self {
            _temporary: temporary,
            paths,
            home,
            repo,
            nested,
            worktree,
            outside,
            template,
            git,
            environment,
            port,
            token,
            client: reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(3))
                .build()
                .unwrap(),
        }
    }

    fn process(
        &self,
        executable: PathBuf,
        cwd: &Path,
        args: Vec<OsString>,
        timeout: Duration,
    ) -> (
        ManagedProcess,
        tokio::sync::broadcast::Receiver<ProcessEvent>,
    ) {
        ManagedProcess::spawn_owned_with_options(
            ProcessPlan {
                executable,
                args,
                cwd: cwd.into(),
                env: self.environment.clone(),
                stdin: vec![],
                timeout,
                output_cap: 64 * 1024,
                pipe_drain_timeout: Duration::from_secs(2),
            },
            128,
            SpawnOptions {
                env_clear: true,
                stdin_null: true,
                no_window: true,
                ..Default::default()
            },
        )
    }

    async fn git(&self, cwd: &Path, args: &[&str]) {
        let (mut child, _) = self.process(
            self.git.clone(),
            cwd,
            args.iter().map(|arg| OsString::from(*arg)).collect(),
            STEP,
        );
        let result = child.wait().await.expect("fixture Git process failed");
        assert_eq!(
            result.outcome,
            ExitOutcome::Exited {
                code: Some(0),
                signal: None
            },
            "fixture git {args:?}: {}",
            self.redact(&String::from_utf8_lossy(&result.stderr)),
        );
        assert!(
            !result.stdout_truncated && !result.stderr_truncated && !result.pipes_forced_closed
        );
    }

    async fn prepare_git(&self) {
        self.git(
            &self.repo,
            &[
                "init",
                "--initial-branch=main",
                &format!("--template={}", self.template.display()),
            ],
        )
        .await;
        self.git(&self.repo, &["config", "core.autocrlf", "false"])
            .await;
        self.git(&self.repo, &["config", "core.filemode", "false"])
            .await;
        self.git(
            &self.repo,
            &["config", "user.name", "Synthetic Branch Test"],
        )
        .await;
        self.git(
            &self.repo,
            &["config", "user.email", "branch-test@example.invalid"],
        )
        .await;
        fs::write(self.repo.join("tracked.txt"), "before\n").unwrap();
        self.git(&self.repo, &["add", "tracked.txt"]).await;
        self.git(
            &self.repo,
            &[
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "synthetic baseline",
            ],
        )
        .await;
        self.git(
            &self.repo,
            &[
                "worktree",
                "add",
                "-b",
                "linked-branch",
                self.worktree.to_str().unwrap(),
            ],
        )
        .await;
        fs::write(self.repo.join("tracked.txt"), "after\nextra\n").unwrap();
        fs::write(self.repo.join("untracked.txt"), "untracked\n").unwrap();
    }

    fn spawn_hub(
        &self,
    ) -> (
        ManagedProcess,
        tokio::sync::broadcast::Receiver<ProcessEvent>,
    ) {
        self.process(
            env!("CARGO_BIN_EXE_many-ai-cli").into(),
            &self.outside,
            vec!["serve".into(), "--open=false".into()],
            Duration::from_secs(180),
        )
    }

    fn endpoint(&self, path: &str) -> url::Url {
        let mut url = url::Url::parse(&format!("http://127.0.0.1:{}{path}", self.port)).unwrap();
        url.query_pairs_mut().append_pair("token", &self.token);
        url
    }

    async fn ready(&self, pid: u32) {
        tokio::time::timeout(STEP, async {
            loop {
                let ledger = fs::read(self.paths.resource(Resource::Runtime))
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
                if ledger
                    .as_ref()
                    .is_some_and(|record| record["pid"] == pid && record["port"] == self.port)
                    && self
                        .client
                        .get(self.endpoint("/api/info"))
                        .send()
                        .await
                        .is_ok_and(|response| response.status().is_success())
                {
                    return;
                }
                assert!(
                    pid_alive(i64::from(pid)),
                    "owned Hub exited before readiness"
                );
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("owned normal-mode Hub did not become ready on its isolated port");
    }

    async fn connect(&self, hello: Value) -> Socket {
        let stream =
            tokio::time::timeout(STEP, TcpStream::connect((Ipv4Addr::LOCALHOST, self.port)))
                .await
                .expect("TCP connect timed out")
                .unwrap();
        let (socket, _) = tokio::time::timeout(
            STEP,
            client_async(format!("ws://127.0.0.1:{}/ws", self.port), stream),
        )
        .await
        .expect("WebSocket upgrade timed out")
        .unwrap();
        let mut socket = Socket { socket };
        socket.send(hello).await;
        socket
    }

    async fn ui(&self) -> Socket {
        self.connect(json!({"role":"ui", "token":self.token, "cols":120, "rows":30}))
            .await
    }

    async fn register(&self, cwd: &Path) -> (Socket, i64) {
        // A synthetic wrapper uses the real admission/authentication protocol.
        // PID zero represents no spawned provider process and cannot name one.
        let mut wrapper = self
            .connect(json!({
                "type":"register", "provider":"shell", "cwd":cwd,
                "home_dir":self.home, "pid":0, "token":self.token,
            }))
            .await;
        let ack = wrapper.next(Instant::now() + STEP).await;
        assert_eq!(
            ack["type"], "registered",
            "real wrapper registration: {ack}"
        );
        let session = ack["session_id"].as_i64().unwrap();
        assert!(session > 0);
        (wrapper, session)
    }

    fn redact(&self, text: &str) -> String {
        text.replace(&self.token, "<synthetic-token>")
            .replace(&*self.home.parent().unwrap().to_string_lossy(), "<fixture>")
            .replace(&*self._temporary.path().to_string_lossy(), "<fixture>")
    }
}

struct Socket {
    socket: WebSocketStream<TcpStream>,
}
impl Socket {
    async fn send(&mut self, value: Value) {
        tokio::time::timeout(
            STEP,
            self.socket.send(Message::Text(value.to_string().into())),
        )
        .await
        .expect("WebSocket send timed out")
        .unwrap();
    }

    async fn next(&mut self, deadline: Instant) -> Value {
        loop {
            let frame = tokio::time::timeout_at(deadline, self.socket.next())
                .await
                .expect("expected session_update did not arrive before deadline")
                .expect("WebSocket closed unexpectedly")
                .expect("WebSocket receive failed");
            match frame {
                Message::Text(text) => return serde_json::from_str(&text).unwrap(),
                Message::Ping(_) | Message::Pong(_) => {}
                other => panic!("unexpected WebSocket frame: {other:?}"),
            }
        }
    }

    async fn checked_branch(
        &mut self,
        session: i64,
        branch: &str,
        stats: (i64, i64, i64),
        project: Option<&Path>,
    ) -> Value {
        let deadline = Instant::now() + STEP;
        loop {
            let frame = self.next(deadline).await;
            if frame["type"] != "session_update" || frame["session_id"] != session {
                continue;
            }
            // Go omits empty strings and zero counters. Decode those wire
            // defaults, but require positive git_checked=true evidence.
            let decoded: proto::Message = serde_json::from_value(frame.clone()).unwrap();
            let project_matches = match project {
                Some(root) => Path::new(&decoded.project_id)
                    .canonicalize()
                    .ok()
                    .is_some_and(|actual| actual == root.canonicalize().unwrap()),
                None => decoded.project_id.is_empty(),
            };
            if decoded.git_checked
                && decoded.branch == branch
                && project_matches
                && (decoded.git_files, decoded.git_added, decoded.git_deleted) == stats
            {
                assert_eq!(frame["git_checked"], true);
                return frame;
            }
        }
    }

    async fn close(&mut self) {
        tokio::time::timeout(STEP, self.socket.close(None))
            .await
            .expect("WebSocket close timed out")
            .unwrap();
    }
}

fn assert_project(frame: &Value, root: &Path) {
    let actual = frame["project_id"]
        .as_str()
        .expect("session_update must include project_id");
    assert_eq!(
        Path::new(actual).canonicalize().unwrap(),
        root.canonicalize().unwrap()
    );
}

async fn exercise(fixture: &Fixture, hub_pid: u32) {
    fixture.ready(hub_pid).await;
    let mut ui = fixture.ui().await;
    let snapshot = ui.next(Instant::now() + STEP).await;
    assert_eq!(snapshot["type"], "snapshot");
    assert_eq!(snapshot["sessions"], json!([]));

    let (mut main_wrapper, main_id) = fixture.register(&fixture.nested).await;
    let first = ui
        .checked_branch(main_id, "main", (2, 2, 1), Some(&fixture.repo))
        .await;
    assert_project(&first, &fixture.repo);

    let (mut outside_wrapper, outside_id) = fixture.register(&fixture.outside).await;
    let outside = ui.checked_branch(outside_id, "", (0, 0, 0), None).await;
    let decoded: proto::Message = serde_json::from_value(outside).unwrap();
    assert!(
        decoded.project_id.is_empty(),
        "non-Git cwd must not acquire a project"
    );

    let (mut linked_wrapper, linked_id) = fixture.register(&fixture.worktree).await;
    let linked = ui
        .checked_branch(linked_id, "linked-branch", (0, 0, 0), Some(&fixture.repo))
        .await;
    assert_project(&linked, &fixture.repo);

    fixture
        .git(&fixture.repo, &["checkout", "-b", "changed-branch"])
        .await;
    fs::write(fixture.repo.join("tracked.txt"), "before\n").unwrap();
    fs::remove_file(fixture.repo.join("untracked.txt")).unwrap();
    // No registration, PTY output, explicit refresh request, or reconnection:
    // the existing UI must receive the real periodic refresh by itself.
    let changed = ui
        .checked_branch(main_id, "changed-branch", (0, 0, 0), Some(&fixture.repo))
        .await;
    assert_project(&changed, &fixture.repo);

    // A fresh, authenticated snapshot provides positive evidence after a real
    // periodic update that the other sessions retain their correct branches.
    let mut later_ui = fixture.ui().await;
    let snapshot = later_ui.next(Instant::now() + STEP).await;
    assert_eq!(snapshot["type"], "snapshot");
    for (id, branch) in [
        (main_id, "changed-branch"),
        (outside_id, ""),
        (linked_id, "linked-branch"),
    ] {
        let session = snapshot["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|session| session["id"] == id)
            .expect("live wrapper session missing");
        assert_eq!(session["branch"].as_str().unwrap_or_default(), branch);
        if id == outside_id {
            assert!(
                session["project_id"]
                    .as_str()
                    .unwrap_or_default()
                    .is_empty()
            );
        } else {
            assert_project(session, &fixture.repo);
        }
    }
    later_ui.close().await;
    main_wrapper.close().await;
    outside_wrapper.close().await;
    linked_wrapper.close().await;
    ui.close().await;
    let response = fixture
        .client
        .post(fixture.endpoint("/api/shutdown"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_git_branch_refresh_reaches_ui_after_checkout_and_stays_empty_outside_git() {
    let reservation = loop {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        if listener.local_addr().unwrap().port() != 47777 {
            break listener;
        }
    };
    let fixture = Fixture::new(reservation.local_addr().unwrap().port());
    fixture.prepare_git().await;
    drop(reservation);
    let (mut hub, mut events) = fixture.spawn_hub();
    let exercise = AssertUnwindSafe(async {
        let event = tokio::time::timeout(STEP, events.recv())
            .await
            .expect("owned Hub process did not start")
            .unwrap();
        let ProcessEvent::Started { pid } = event else {
            panic!("expected owned Hub process identity: {event:?}");
        };
        exercise(&fixture, pid).await;
    })
    .catch_unwind()
    .await;
    if exercise.is_err() {
        hub.close();
    }
    let mut reaped = tokio::time::timeout(STEP, hub.wait()).await;
    let shutdown_timed_out = reaped.is_err();
    if shutdown_timed_out {
        hub.close();
        reaped = tokio::time::timeout(STEP, hub.wait()).await;
    }
    if exercise.is_err()
        || shutdown_timed_out
        || !matches!(&reaped, Ok(Ok(result)) if result.outcome == ExitOutcome::Exited { code: Some(0), signal: None })
    {
        let log = fs::read_to_string(fixture.paths.resource(Resource::Logs).join("hub.log"))
            .unwrap_or_default();
        let outcome = match &reaped {
            Ok(Ok(output)) => format!(
                "outcome={:?}; stdout_truncated={}; stderr_truncated={}; pipes_forced_closed={}; stderr={}",
                output.outcome,
                output.stdout_truncated,
                output.stderr_truncated,
                output.pipes_forced_closed,
                fixture.redact(&String::from_utf8_lossy(&output.stderr)),
            ),
            Ok(Err(error)) => format!("wait error kind={:?}", error.kind()),
            Err(_) => "wait deadline elapsed".into(),
        };
        eprintln!(
            "native Git branch assertion context:\nHub: {outcome}\n{}",
            fixture.redact(&log)
        );
    }
    if let Err(panic) = exercise {
        std::panic::resume_unwind(panic);
    }
    assert!(
        !shutdown_timed_out,
        "owned Hub did not shut down within deadline"
    );
    assert_eq!(
        reaped.expect("owned Hub reap timed out").unwrap().outcome,
        ExitOutcome::Exited {
            code: Some(0),
            signal: None
        },
        "authenticated shutdown must finish and reap the actual Hub"
    );
}
