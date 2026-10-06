//! Exercise the built Hub, its real wrapper, and a real cmd.exe under ConPTY.
//! Cargo discovers this target automatically; the native candidate CI runs it
//! through `cargo test --all-targets`. Cross-compilation is not runtime evidence.
#![cfg(windows)]

use futures_util::{FutureExt, SinkExt, StreamExt};
use many_ai_cli::{
    approval::identity::strip_ansi,
    config::{Config, ConfigStore, CustomProvider, Resource, RuntimePaths},
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

const STEP: Duration = Duration::from_secs(15);
const PROVIDER: &str = "synth-echo";
const LABEL: &str = "native-conpty-roundtrip";

struct Fixture {
    // Kept alive until the Hub and its entire owned process tree are reaped.
    _temporary: tempfile::TempDir,
    paths: RuntimePaths,
    home: PathBuf,
    work: PathBuf,
    command: PathBuf,
    system_root: OsString,
    token: String,
    client: reqwest::Client,
}

impl Fixture {
    fn new(port: u16) -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let trial = temporary.path().join("trial");
        let home = temporary.path().join("synthetic-home");
        let work = trial.join("work");
        let bin = trial.join("bin");
        for directory in [&home, &work, &bin] {
            fs::create_dir_all(directory).unwrap();
        }
        let paths = RuntimePaths::trial(&trial, port, &home.join(".many-ai-cli")).unwrap();
        let system_root = std::env::var_os("SystemRoot").expect("native Windows SystemRoot");
        let command = bin.join("cmd.exe");
        fs::copy(Path::new(&system_root).join("System32/cmd.exe"), &command).unwrap();
        // No external commands or real provider/account paths. A broken stdin
        // exits 3 immediately; only an actual `quit` received through set /p
        // exits 0. `/d` below disables cmd.exe AutoRun registry commands.
        fs::write(
            work.join("provider.cmd"),
            concat!(
                "@echo off\r\n",
                "setlocal EnableExtensions DisableDelayedExpansion\r\n",
                "echo SYNTH_READY\r\n",
                "echo SYNTH_STDERR 1>&2\r\n",
                ":read\r\n",
                "set \"SYNTH_LINE=\"\r\n",
                "set /p \"SYNTH_LINE=\"\r\n",
                "if errorlevel 1 exit /b 3\r\n",
                "if \"%SYNTH_LINE%\"==\"quit\" exit /b 0\r\n",
                "echo SYNTH_ECHO:%SYNTH_LINE%\r\n",
                "goto read\r\n",
            ),
        )
        .unwrap();
        let token = many_ai_cli::process::random_token().unwrap();
        let mut config = Config::defaults(&paths);
        config.token = token.clone();
        config.hub.open_browser = false;
        config.hub.auto_shutdown = false;
        config.log.session_enabled = true;
        config.custom_providers = vec![CustomProvider {
            id: PROVIDER.into(),
            command: format!("\"{}\" /d /q /c provider.cmd", command.display()),
            ..Default::default()
        }];
        let store = ConfigStore::new(paths.clone(), config.clone()).unwrap();
        store.persist(0, config).unwrap();
        Self {
            _temporary: temporary,
            paths,
            home,
            work,
            command,
            system_root,
            token,
            client: reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(3))
                .build()
                .unwrap(),
        }
    }

    fn spawn(
        &self,
    ) -> (
        ManagedProcess,
        tokio::sync::broadcast::Receiver<ProcessEvent>,
    ) {
        let mut environment = BTreeMap::new();
        for key in ["HOME", "USERPROFILE", "APPDATA", "LOCALAPPDATA"] {
            environment.insert(key.into(), Some(self.home.as_os_str().to_owned()));
        }
        for key in ["TEMP", "TMP", "TMPDIR"] {
            environment.insert(key.into(), Some(self.paths.root().as_os_str().to_owned()));
        }
        environment.insert("SystemRoot".into(), Some(self.system_root.clone()));
        environment.insert("WINDIR".into(), Some(self.system_root.clone()));
        environment.insert("COMSPEC".into(), Some(self.command.as_os_str().to_owned()));
        environment.insert(
            "PATH".into(),
            Some(self.command.parent().unwrap().as_os_str().to_owned()),
        );
        environment.insert("PATHEXT".into(), Some(".EXE;.COM;.BAT;.CMD".into()));
        ManagedProcess::spawn_owned_with_options(
            ProcessPlan {
                executable: env!("CARGO_BIN_EXE_many-ai-cli").into(),
                args: vec![
                    "--trial-root".into(),
                    self.paths.root().into(),
                    "--trial-port".into(),
                    self.paths.port().to_string().into(),
                    "serve".into(),
                ],
                cwd: self.work.clone(),
                env: environment,
                stdin: vec![],
                timeout: Duration::from_secs(90),
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

    fn endpoint(&self, path: &str) -> url::Url {
        let mut url =
            url::Url::parse(&format!("http://127.0.0.1:{}{path}", self.paths.port())).unwrap();
        url.query_pairs_mut().append_pair("token", &self.token);
        url
    }

    async fn ready(&self, pid: u32) {
        tokio::time::timeout(STEP, async {
            loop {
                let ledger = fs::read(self.paths.resource(Resource::Runtime))
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
                if ledger.as_ref().is_some_and(|record| {
                    record["pid"] == pid && record["port"] == self.paths.port()
                }) && self
                    .client
                    .get(self.endpoint("/api/info"))
                    .send()
                    .await
                    .is_ok_and(|response| response.status().is_success())
                {
                    break;
                }
                assert!(
                    pid_alive(i64::from(pid)),
                    "owned Hub exited before readiness"
                );
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("owned Hub did not become ready on its isolated port");
    }

    async fn connect_ui(&self) -> Ui {
        let port = self.paths.port();
        let stream = tokio::time::timeout(STEP, TcpStream::connect((Ipv4Addr::LOCALHOST, port)))
            .await
            .expect("UI TCP connect timed out")
            .unwrap();
        let (socket, _) = tokio::time::timeout(
            STEP,
            client_async(format!("ws://127.0.0.1:{port}/ws"), stream),
        )
        .await
        .expect("UI WebSocket upgrade timed out")
        .unwrap();
        let mut ui = Ui { socket };
        ui.send(json!({"role":"ui", "token":self.token, "cols":120, "rows":30}))
            .await;
        ui
    }

    async fn snapshot(&self) -> Value {
        let mut ui = self.connect_ui().await;
        let snapshot = ui.next(Instant::now() + STEP).await;
        assert_eq!(snapshot["type"], "snapshot");
        ui.close().await;
        snapshot
    }

    fn diagnostics(&self) -> String {
        let mut output = String::new();
        for directory in ["spawn", "sessions"] {
            let Ok(entries) = fs::read_dir(self.paths.resource(Resource::Logs).join(directory))
            else {
                continue;
            };
            let mut entries: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
            entries.sort();
            for path in entries {
                if let Ok(bytes) = fs::read(&path) {
                    output.push_str(&format!(
                        "\n{directory}/{} ({} bytes):\n{}\n",
                        path.file_name().unwrap().to_string_lossy(),
                        bytes.len(),
                        String::from_utf8_lossy(&bytes[..bytes.len().min(16 * 1024)]),
                    ));
                }
            }
        }
        self.redact(&output)
    }

    fn redact(&self, text: &str) -> String {
        text.replace(&self.token, "<synthetic-token>")
            .replace(&*self._temporary.path().to_string_lossy(), "<fixture>")
    }
}

struct Ui {
    socket: WebSocketStream<TcpStream>,
}
impl Ui {
    async fn send(&mut self, value: Value) {
        tokio::time::timeout(
            STEP,
            self.socket.send(Message::Text(value.to_string().into())),
        )
        .await
        .expect("UI send timed out")
        .unwrap();
    }

    async fn next(&mut self, deadline: Instant) -> Value {
        loop {
            let frame = tokio::time::timeout_at(deadline, self.socket.next())
                .await
                .expect("expected UI event did not arrive")
                .expect("UI WebSocket closed unexpectedly")
                .expect("UI WebSocket receive failed");
            match frame {
                Message::Text(text) => return serde_json::from_str(&text).unwrap(),
                Message::Ping(_) | Message::Pong(_) => {}
                other => panic!("unexpected UI WebSocket frame: {other:?}"),
            }
        }
    }

    async fn close(&mut self) {
        tokio::time::timeout(STEP, self.socket.close(None))
            .await
            .expect("UI WebSocket close timed out")
            .unwrap();
    }
}

async fn output_until(ui: &mut Ui, session: i64, output: &mut Vec<u8>, markers: &[&str]) {
    let deadline = Instant::now() + STEP;
    loop {
        let text = strip_ansi(&String::from_utf8_lossy(output));
        if markers.iter().all(|marker| text.contains(marker)) {
            return;
        }
        let value = ui.next(deadline).await;
        if value["session_id"].as_i64() != Some(session) {
            continue;
        }
        assert_ne!(
            value["type"], "session_end",
            "provider ended before {markers:?}: {value}"
        );
        if value["type"] == "pty_data" {
            let frame: proto::Message = serde_json::from_value(value).unwrap();
            output.extend_from_slice(&frame.data);
            assert!(
                output.len() <= 64 * 1024,
                "unexpected synthetic PTY output flood"
            );
        }
    }
}

async fn roundtrip(fixture: &Fixture, hub_pid: u32, output: &mut Vec<u8>) {
    fixture.ready(hub_pid).await;
    let mut ui = fixture.connect_ui().await;
    let snapshot = ui.next(Instant::now() + STEP).await;
    assert_eq!(snapshot["type"], "snapshot");
    assert_eq!(snapshot["sessions"], json!([]));

    let response = fixture
        .client
        .post(fixture.endpoint("/api/spawn"))
        .json(&json!({
            "provider":PROVIDER, "cwd":fixture.work, "label":LABEL,
            "execution_mode":"interactive", "isolate_worktree":false, "delegation":false
        }))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let body = response.text().await.unwrap();
    assert_eq!(status, reqwest::StatusCode::OK, "spawn response: {body}");
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["ok"], true);

    let deadline = Instant::now() + STEP;
    let registered = loop {
        let frame = ui.next(deadline).await;
        if frame["type"] == "session_update" && frame["provider"] == PROVIDER {
            assert_eq!(frame["label"], LABEL);
            assert_eq!(frame["state"], "running");
            break frame;
        }
    };
    let session = registered["session_id"].as_i64().unwrap();
    assert!(session > 0);
    output_until(&mut ui, session, output, &["SYNTH_READY", "SYNTH_STDERR"]).await;

    let journal = PathBuf::from(registered["jsonl_path"].as_str().unwrap());
    let journal = journal.canonicalize().unwrap();
    let sessions_root = fixture
        .paths
        .resource(Resource::Logs)
        .join("sessions")
        .canonicalize()
        .unwrap();
    assert!(journal.starts_with(sessions_root));
    // Public session cards omit PID. Registration's real wrapper PID is instead
    // recorded in its ordered session_start event before any PTY output.
    let entries = journal_entries(&journal);
    let start = entries
        .iter()
        .find(|entry| entry["type"] == "session_start" && entry["session_id"] == session)
        .expect("actual wrapper registration must be journaled");
    let wrapper_pid = start["pid"].as_i64().unwrap();
    assert!(wrapper_pid > 0 && wrapper_pid != i64::from(hub_pid));

    // Require positive liveness/state evidence after a real no-input interval.
    // A missing event or an elapsed receive deadline never counts as success.
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(
        pid_alive(wrapper_pid),
        "wrapper exited instead of waiting for input"
    );
    let waiting = fixture.snapshot().await;
    let waiting = waiting["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == session)
        .expect("waiting provider session missing from actual Hub snapshot");
    assert_eq!(waiting["state"], "running");

    ui.send(json!({"type":"pty_input", "session_id":session, "text":"hello\r"}))
        .await;
    output_until(&mut ui, session, output, &["SYNTH_ECHO:hello"]).await;
    ui.send(json!({"type":"pty_input", "session_id":session, "text":"quit\r"}))
        .await;
    let deadline = Instant::now() + STEP;
    let ended = loop {
        let frame = ui.next(deadline).await;
        if frame["type"] == "session_end" && frame["session_id"] == session {
            break frame;
        }
    };
    assert_eq!(
        ended["state"], "completed",
        "actual UI session_end: {ended}"
    );
    // Fixed Go wrapper_loop.go broadcasts state/reason on disconnect, omitting
    // exit_code. Do not mistake the DTO's default zero for provider-exit proof.
    assert!(ended.get("exit_code").is_none(), "actual UI wire: {ended}");
    let decoded: proto::Message = serde_json::from_value(ended).unwrap();
    assert_eq!(decoded.exit_code, 0, "omitted Go-compatible wire default");

    let entries = journal_entries(&journal);
    let ends: Vec<_> = entries
        .iter()
        .filter(|entry| entry["type"] == "session_end" && entry["session_id"] == session)
        .collect();
    assert_eq!(
        ends.len(),
        1,
        "actual wrapper end must be persisted exactly once"
    );
    assert_eq!(ends[0]["state"], "completed");
    assert_eq!(
        ends[0].get("exit_code"),
        Some(&json!(0)),
        "actual provider exit"
    );

    let final_snapshot = fixture.snapshot().await;
    let completed = final_snapshot["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == session)
        .expect("completed provider session missing from actual Hub snapshot");
    assert_eq!(completed["state"], "completed");
    tokio::time::timeout(STEP, async {
        while pid_alive(wrapper_pid) {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("actual wrapper did not exit after provider quit");
    ui.close().await;
    let response = fixture
        .client
        .post(fixture.endpoint("/api/shutdown"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
}

fn journal_entries(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_windows_spawn_waits_for_ui_input_and_delivers_pty_output_and_clean_end() {
    // Hold a dynamic loopback reservation until immediately before launching.
    // Trial mode refuses fallback ports; never probe the real/default Hub.
    let reservation = loop {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        if listener.local_addr().unwrap().port() != 47777 {
            break listener;
        }
    };
    let fixture = Fixture::new(reservation.local_addr().unwrap().port());
    drop(reservation);
    let (mut hub, mut events) = fixture.spawn();
    let mut output = Vec::new();
    let exercise = AssertUnwindSafe(async {
        let event = tokio::time::timeout(STEP, events.recv())
            .await
            .expect("owned Hub process did not start")
            .unwrap();
        let ProcessEvent::Started { pid } = event else {
            panic!("expected owned Hub process identity: {event:?}");
        };
        roundtrip(&fixture, pid, &mut output).await;
    })
    .catch_unwind()
    .await;
    if exercise.is_err() {
        // The retained ManagedProcess owns a kill-on-close Windows Job. Always
        // terminate/reap only this fixture tree before propagating assertions.
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
        eprintln!(
            "native synthetic spawn diagnostics:\nPTY: {}\nHub: {}\n{}",
            fixture.redact(&String::from_utf8_lossy(&output)),
            fixture.redact(&format!("{reaped:?}")),
            fixture.diagnostics(),
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
        "authenticated shutdown must finish and reap the actual Hub",
    );
    assert_eq!(fs::read_dir(&fixture.home).unwrap().count(), 0);
}
