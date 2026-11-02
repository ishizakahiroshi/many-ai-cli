//! Actual remote-file adapters with an owned SSH stand-in that never uses a network.
use super::*;

const PAYLOAD: &[u8] = b"synthetic-remote-payload";
type LocalState = (Vec<(Value, Option<Vec<u8>>)>, usize);
struct RemoteFixture {
    fixture: Fixture,
    service: crate::files::FilesService,
    counter: std::path::PathBuf,
    shell: LiveSessionId,
    provider: LiveSessionId,
    disconnected: LiveSessionId,
}
impl RemoteFixture {
    async fn new() -> Self {
        let fixture = fixture(true);
        let shell = register_provider(&fixture, "shell").await;
        let provider = register_provider(&fixture, "claude").await;
        let disconnected = register_provider(&fixture, "shell").await;
        let binding = fixture.core.details(disconnected).unwrap().binding;
        fixture
            .sink
            .apply(fixture.core.disconnected(binding, now()).unwrap())
            .await
            .unwrap();
        assert!(!fixture.core.details(disconnected).unwrap().connected);
        let counter = fixture._root.path().join("ssh-invocations.txt");
        let executable = make_fake_ssh(fixture._root.path(), &counter);
        let mut service =
            crate::files::FilesService::new(fixture._root.path().into(), fixture.paths.clone())
                .with_history(fixture.sink.journal.clone());
        service.ssh_executable = executable;
        Self {
            fixture,
            service,
            counter,
            shell,
            provider,
            disconnected,
        }
    }
    fn invocation_count(&self) -> usize {
        match std::fs::read_to_string(&self.counter) {
            Ok(text) => text.lines().filter(|line| *line == "invocation").count(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => panic!("cannot read synthetic process counter: {error}"),
        }
    }
    fn local_state(&self) -> LocalState {
        let sessions = [self.shell, self.provider, self.disconnected]
            .into_iter()
            .map(|id| {
                let snapshot = self.fixture.core.snapshot(id).unwrap();
                let journal = std::path::PathBuf::from(&snapshot.log_path).with_extension("jsonl");
                let bytes = match std::fs::read(journal) {
                    Ok(bytes) => Some(bytes),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                    Err(error) => panic!("cannot read synthetic journal: {error}"),
                };
                (serde_json::to_value(snapshot).unwrap(), bytes)
            })
            .collect();
        (sessions, self.fixture.sink.messages.lock().unwrap().len())
    }
    async fn send(&self, request: Request) -> Response {
        tokio::time::timeout(
            Duration::from_secs(5),
            self.service.handle(
                &request,
                self.fixture.core.as_ref(),
                None,
                false,
                &AttachmentWorkspace,
            ),
        )
        .await
        .expect("owned fake SSH and validation must finish promptly")
        .unwrap()
    }
    fn assert_no_local_attachment(&self) {
        assert!(!self.fixture.paths.resource(Resource::Attachments).exists());
    }
    async fn assert_no_sqlite_attachment(&self) {
        let store = self.fixture.store.as_ref().unwrap();
        let report = store
            .shutdown(
                ShutdownPolicy::Drain {
                    timeout: Duration::from_secs(5),
                },
                &HubShutdownCancellation::default(),
            )
            .await;
        assert!(!report.timed_out && !report.cancelled, "{report:?}");
        assert!(report.error.is_none(), "{report:?}");
        assert_eq!(report.remaining, 0);
        assert_eq!(report.dropped, 0);
        let reopened = SqliteSessionStorage::open(
            &self.fixture.paths,
            StorageOptions::baseline(self.fixture.paths.resource(Resource::Logs)),
        )
        .unwrap();
        for id in [self.shell, self.provider, self.disconnected] {
            let events = reopened
                .chat_messages_by_live_session(id, 100)
                .unwrap()
                .unwrap_or_default();
            assert!(events.iter().all(|event| event.kind != "attach"));
        }
    }
}

#[cfg(windows)]
fn make_fake_ssh(root: &Path, _counter: &Path) -> std::path::PathBuf {
    let executable = root.join("fake-ssh.cmd");
    // Ignore every SSH argument. Only this literal output and counter can occur;
    // no received remote shell script is ever evaluated by the stand-in.
    std::fs::write(
        &executable,
        "@echo off\r\n>>\"%~dp0ssh-invocations.txt\" echo invocation\r\n<nul set /p \"=many-ai-cli-remote-file:synthetic-remote-payload\"\r\nexit /b 0\r\n",
    )
    .unwrap();
    executable
}
#[cfg(unix)]
fn make_fake_ssh(root: &Path, counter: &Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let executable = root.join("fake-ssh");
    let quoted = crate::launcher::shell_quote(counter.to_str().unwrap());
    std::fs::write(
        &executable,
        format!(
            "#!/bin/sh\nprintf 'invocation\\n' >> {quoted}\nprintf 'many-ai-cli-remote-file:synthetic-remote-payload'\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    executable
}

fn download(id: LiveSessionId, target: &str, path: &str) -> Request {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("session", &id.0.to_string())
        .append_pair("remote_target", target)
        .append_pair("path", path)
        .finish();
    Request {
        method: "GET".into(),
        path: "/api/files-download".into(),
        query,
        ..Default::default()
    }
}
fn upload(id: LiveSessionId, target: &str) -> Request {
    Request {
        method: "POST".into(),
        path: "/api/attach".into(),
        query: url::form_urlencoded::Serializer::new(String::new())
            .append_pair("remote_target", target)
            .finish(),
        headers: vec![(
            "Content-Type".into(),
            "multipart/form-data; boundary=remote-bound".into(),
        )],
        body: format!(
            "--remote-bound\r\nContent-Disposition: form-data; name=\"session_id\"\r\n\r\n{}\r\n--remote-bound\r\nContent-Disposition: form-data; name=\"file\"; filename=\"synthetic.txt\"\r\n\r\nsynthetic upload\r\n--remote-bound--\r\n",
            id.0
        )
        .into_bytes(),
        ..Default::default()
    }
}

#[tokio::test]
async fn remote_download_guards_launch_nothing_and_connected_shell_uses_owned_fake_ssh() {
    let remote = RemoteFixture::new().await;
    let before = remote.local_state();
    for (id, target, path, status) in [
        (LiveSessionId(999), "fixture-host", "/synthetic.txt", 404),
        (remote.provider, "fixture-host", "/synthetic.txt", 400),
        (remote.disconnected, "fixture-host", "/synthetic.txt", 404),
        (
            remote.shell,
            "-oProxyCommand=synthetic",
            "/synthetic.txt",
            400,
        ),
        (remote.shell, "ssh://-fixture", "/synthetic.txt", 400),
        (remote.shell, "fixture;synthetic", "/synthetic.txt", 400),
        (remote.shell, "fixture host", "/synthetic.txt", 400),
        (remote.shell, "fixture-host", "relative.txt", 400),
        (remote.shell, "fixture-host", "/", 400),
        (remote.shell, "fixture-host", "/synthetic\n.txt", 400),
    ] {
        let response = remote.send(download(id, target, path)).await;
        assert_eq!(response.status, status, "{target:?} {path:?}");
        assert_eq!(remote.invocation_count(), 0);
        assert_eq!(remote.local_state(), before);
        remote.assert_no_local_attachment();
    }
    let response = remote
        .send(download(remote.shell, "fixture-host", "/synthetic.txt"))
        .await;
    assert_eq!(
        response.status,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    assert_eq!(response.body, PAYLOAD);
    assert_eq!(
        remote.invocation_count(),
        1,
        "positive control proves the recorder executes"
    );
    assert_eq!(remote.local_state(), before);
    remote.assert_no_local_attachment();
    remote.assert_no_sqlite_attachment().await;
}

#[tokio::test]
async fn remote_upload_guards_launch_nothing_and_failed_fake_protocol_never_saves_attachment() {
    let remote = RemoteFixture::new().await;
    let before = remote.local_state();
    for (id, target, status) in [
        (LiveSessionId(999), "fixture-host", 404),
        (remote.provider, "fixture-host", 400),
        (remote.disconnected, "fixture-host", 404),
        (remote.shell, "-oProxyCommand=synthetic", 400),
        (remote.shell, "ssh://-fixture", 400),
        (remote.shell, "fixture;synthetic", 400),
        (remote.shell, "fixture host", 400),
    ] {
        let response = remote.send(upload(id, target)).await;
        assert_eq!(response.status, status, "{target:?}");
        assert_eq!(remote.invocation_count(), 0);
        assert_eq!(remote.local_state(), before);
        remote.assert_no_local_attachment();
    }
    let response = remote.send(upload(remote.shell, "fixture-host")).await;
    error(
        response,
        502,
        "ssh finished but did not report the saved path",
    );
    assert_eq!(
        remote.invocation_count(),
        1,
        "valid shell reaches the same observed process"
    );
    assert_eq!(remote.local_state(), before);
    remote.assert_no_local_attachment();
    remote.assert_no_sqlite_attachment().await;
}
