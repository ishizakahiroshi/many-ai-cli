use super::*;
use std::fs;

struct Fixture {
    _root: tempfile::TempDir,
    paths: RuntimePaths,
    outside: PathBuf,
}
impl Fixture {
    fn new(port: u16) -> Self {
        let root = tempfile::tempdir().unwrap();
        let trial = root.path().join("trial space 'quote");
        let outside = root.path().join("outside");
        fs::create_dir(&trial).unwrap();
        fs::create_dir(&outside).unwrap();
        let paths = RuntimePaths::trial(&trial, port, &root.path().join("installed")).unwrap();
        Self {
            _root: root,
            paths,
            outside,
        }
    }
    fn file(&self, directory: &Path, name: &str, body: &[u8]) -> PathBuf {
        let path = directory.join(name);
        fs::write(&path, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        path
    }
}
fn permission_denied(result: io::Result<ResolvedCommand>) {
    assert!(matches!(result, Err(error) if error.kind() == io::ErrorKind::PermissionDenied));
}

#[test]
fn trial_provider_resolution_confines_path_custom_and_cwd_without_production_changes() {
    let fixture = Fixture::new(48891);
    let name = if cfg!(windows) {
        "synthetic.exe"
    } else {
        "synthetic"
    };
    let outside = fixture.file(&fixture.outside, name, b"synthetic only");
    let environment = vec![format!("PATH={}", fixture.outside.display())];
    let resolver = CommandResolver::new(&fixture.paths, &environment, fixture.paths.root());
    permission_denied(resolver.resolve_provider(name, None, &[]));
    let custom = vec![outside.to_string_lossy().into_owned(), "base".into()];
    permission_denied(resolver.resolve_provider("synthetic", Some(&custom), &[]));
    permission_denied(resolver.resolve_provider("missing-synthetic", None, &[]));
    let scoped = ScopedFs(&fixture.paths);
    assert!(!scoped.exists(outside.to_str().unwrap()));
    assert!(!scoped.is_executable(outside.to_str().unwrap(), Platform::native()));
    assert_eq!(
        scoped.read(outside.to_str().unwrap()).unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );

    let inside = fixture.file(fixture.paths.root(), name, b"synthetic only");
    let custom = vec![inside.to_string_lossy().into_owned(), "base".into()];
    let resolved = resolver
        .resolve_provider("synthetic", Some(&custom), &["tail".into()])
        .unwrap();
    assert_eq!(
        Path::new(&resolved.executable),
        inside.canonicalize().unwrap()
    );
    assert_eq!(resolved.args, ["base", "tail"]);
    let wrong_cwd = CommandResolver::new(&fixture.paths, &environment, &fixture.outside);
    permission_denied(wrong_cwd.resolve_provider("synthetic", Some(&custom), &[]));

    let production = RuntimePaths::production(&fixture.outside).unwrap();
    let resolver = CommandResolver::new(&production, &environment, fixture.paths.root());
    let resolved = resolver.resolve_provider(name, None, &[]).unwrap();
    assert_eq!(Path::new(&resolved.executable), outside);
    assert_eq!(
        resolver
            .resolve_provider("missing-synthetic", None, &[])
            .unwrap()
            .executable,
        "missing-synthetic"
    );
}

#[cfg(unix)]
#[test]
fn trial_provider_resolution_rejects_file_directory_and_shell_alias_escapes() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new(48891);
    let outside = fixture.file(&fixture.outside, "synthetic", b"#!/bin/sh\nexit 0\n");
    let alias = fixture.paths.root().join("alias");
    symlink(&outside, &alias).unwrap();
    let directory = fixture.paths.root().join("aliased-directory");
    symlink(&fixture.outside, &directory).unwrap();
    let environment = vec![
        format!("PATH={}", directory.display()),
        format!("SHELL={}", alias.display()),
    ];
    let resolver = CommandResolver::new(&fixture.paths, &environment, fixture.paths.root());
    permission_denied(resolver.resolve_provider(
        "synthetic",
        Some(&[alias.to_string_lossy().into_owned()]),
        &[],
    ));
    permission_denied(resolver.resolve_provider("synthetic", None, &[]));
    permission_denied(resolver.resolve_provider("shell", None, &[]));
    let missing = fixture.paths.root().join("dangling");
    symlink(fixture.outside.join("missing"), &missing).unwrap();
    permission_denied(resolver.resolve_provider(
        "synthetic",
        Some(&[missing.to_string_lossy().into_owned()]),
        &[],
    ));

    // Legitimate native canonical spelling aliases remain usable, and only the
    // canonical absolute executable is handed to the process owner.
    let inside = fixture.file(fixture.paths.root(), "owned", b"#!/bin/sh\nexit 0\n");
    let inside_alias = fixture.paths.root().join("inside-alias");
    symlink(&inside, &inside_alias).unwrap();
    let resolved = resolver
        .resolve_provider(
            "synthetic",
            Some(&[inside_alias.to_string_lossy().into_owned()]),
            &[],
        )
        .unwrap();
    assert_eq!(
        Path::new(&resolved.executable),
        inside.canonicalize().unwrap()
    );
}

#[cfg(windows)]
#[test]
fn trial_windows_shim_cannot_resolve_or_read_external_executable() {
    let fixture = Fixture::new(48891);
    let outside = fixture.file(&fixture.outside, "outside.exe", b"synthetic only");
    let shim = fixture.file(
        fixture.paths.root(),
        "synthetic.cmd",
        format!("\"{}\" %*\r\n", outside.display()).as_bytes(),
    );
    let environment = vec![format!("COMSPEC={}", outside.display())];
    let resolver = CommandResolver::new(&fixture.paths, &environment, fixture.paths.root());
    let custom = vec![shim.to_string_lossy().into_owned()];
    permission_denied(resolver.resolve_provider("synthetic", Some(&custom), &["tail".into()]));
    assert!(
        resolver
            .launch_shell_shim("synthetic", Some(&custom))
            .is_err()
    );
    let inside = fixture.file(fixture.paths.root(), "inside.exe", b"synthetic only");
    fs::write(&shim, format!("\"{}\" %*\r\n", inside.display())).unwrap();
    let resolved = resolver
        .resolve_provider("synthetic", Some(&custom), &["tail".into()])
        .unwrap();
    assert_eq!(
        Path::new(&resolved.executable),
        inside.canonicalize().unwrap()
    );
    assert_eq!(resolved.args, ["tail"]);
}

#[cfg(unix)]
mod native {
    use super::*;
    use crate::{
        config::{Config, CustomProvider, HeadlessDef},
        process::Cancellation,
        proto::{Message, core::TerminalSize},
        wrapper::{
            entry::{WrapperContext, run_cli},
            runtime::WrapperResult,
        },
    };
    use futures_util::{SinkExt, StreamExt};
    use std::{os::unix::fs::symlink, time::Duration};
    use tokio::{net::TcpListener, task::JoinHandle};
    use tokio_tungstenite::{accept_async, tungstenite::Message as WsMessage};

    async fn listener() -> TcpListener {
        loop {
            let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
                .await
                .unwrap();
            if listener.local_addr().unwrap().port() != 47777 {
                return listener;
            }
        }
    }
    fn hub(listener: TcpListener, replace: Option<(PathBuf, PathBuf)>) -> JoinHandle<()> {
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let frame = socket.next().await.unwrap().unwrap();
            let registration: Message = serde_json::from_slice(&frame.into_data()).unwrap();
            assert_eq!(registration.r#type, "register");
            if let Some((alias, outside)) = replace {
                fs::remove_file(&alias).unwrap();
                symlink(outside, alias).unwrap();
            }
            socket
                .send(WsMessage::Text(
                    serde_json::to_string(&Message {
                        r#type: "registered".into(),
                        session_id: 17,
                        ..Default::default()
                    })
                    .unwrap()
                    .into(),
                ))
                .await
                .unwrap();
            while let Some(Ok(frame)) = socket.next().await {
                if let Ok(frame) = serde_json::from_slice::<Message>(&frame.into_data())
                    && frame.r#type == "session_end"
                {
                    break;
                }
            }
        })
    }
    async fn run(
        fixture: &Fixture,
        command: &str,
        headless: bool,
        path: &Path,
    ) -> io::Result<WrapperResult> {
        let config = Config {
            custom_providers: vec![CustomProvider {
                id: "synthetic".into(),
                command: format!("\"{}\"", command.replace('"', "\"\"")),
                headless: Some(HeadlessDef {
                    format: "text".into(),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        };
        // The process owners inherit ambient keys. Override every visible key
        // with an empty synthetic value before setting this fixture's paths, so
        // the harmless script receives no real provider/account environment.
        let mut environment: Vec<String> = std::env::vars_os()
            .filter_map(|(key, _)| key.into_string().ok().map(|key| format!("{key}=")))
            .collect();
        environment.extend([
            format!("PATH={}", path.display()),
            format!("HOME={}", fixture.paths.root().display()),
            format!("TMPDIR={}", fixture.paths.root().display()),
            format!("TEMP={}", fixture.paths.root().display()),
            format!("TMP={}", fixture.paths.root().display()),
        ]);
        let args = if headless {
            vec!["--headless".into()]
        } else {
            vec![]
        };
        tokio::time::timeout(
            Duration::from_secs(5),
            run_cli(
                WrapperContext {
                    config: &config,
                    paths: &fixture.paths,
                    cwd: fixture.paths.root(),
                    executable: &fixture.paths.root().join("synthetic-wrapper"),
                    environment: &environment,
                    shell: "synthetic-shell",
                    terminal_size: TerminalSize { cols: 80, rows: 24 },
                    home_dir: fixture.paths.root(),
                },
                "synthetic",
                &args,
                &Cancellation::default(),
            ),
        )
        .await
        .expect("bounded synthetic wrapper")
    }
    fn script(fixture: &Fixture, directory: &Path, name: &str, marker: &Path) -> PathBuf {
        fixture.file(
            directory,
            name,
            format!(
                "#!/bin/sh\nprintf 'synthetic only' > '{}'\n",
                marker.to_string_lossy().replace('\'', "'\\''")
            )
            .as_bytes(),
        )
    }

    #[tokio::test]
    async fn trial_native_entry_rejects_external_path_and_custom_alias_before_launch() {
        for headless in [false, true] {
            for kind in ["absolute", "path", "alias"] {
                let listener = listener().await;
                let fixture = Fixture::new(listener.local_addr().unwrap().port());
                let marker = fixture.outside.join("must-not-run");
                let outside = script(&fixture, &fixture.outside, "synthetic", &marker);
                let command = match kind {
                    "absolute" => outside.to_string_lossy().into_owned(),
                    "path" => "synthetic".into(),
                    _ => {
                        let alias = fixture.paths.root().join("alias");
                        symlink(&outside, &alias).unwrap();
                        alias.to_string_lossy().into_owned()
                    }
                };
                let server = hub(listener, None);
                let result = run(&fixture, &command, headless, &fixture.outside).await;
                server.abort();
                let _ = server.await;
                assert!(
                    !marker.exists(),
                    "outside synthetic script ran: {kind}, headless={headless}"
                );
                assert!(
                    matches!(result, Err(error) if error.kind() == io::ErrorKind::PermissionDenied)
                );
            }
        }
    }

    #[tokio::test]
    async fn trial_native_entry_rechecks_executable_after_registration_for_pty_and_headless() {
        for headless in [false, true] {
            let listener = listener().await;
            let fixture = Fixture::new(listener.local_addr().unwrap().port());
            let marker = fixture.outside.join("must-not-run");
            let outside = script(&fixture, &fixture.outside, "synthetic", &marker);
            let owned = script(&fixture, fixture.paths.root(), "owned", &marker);
            let server = hub(listener, Some((owned.clone(), outside)));
            let result = run(
                &fixture,
                owned.to_str().unwrap(),
                headless,
                fixture.paths.root(),
            )
            .await;
            server.abort();
            let _ = server.await;
            assert!(
                !marker.exists(),
                "outside script ran after registration, headless={headless}"
            );
            assert!(
                matches!(result, Err(error) if error.kind() == io::ErrorKind::PermissionDenied)
            );
        }
    }

    #[tokio::test]
    async fn trial_native_entry_runs_owned_synthetic_executable_for_pty_and_headless() {
        for headless in [false, true] {
            let listener = listener().await;
            let fixture = Fixture::new(listener.local_addr().unwrap().port());
            let marker = fixture.paths.root().join("did-run");
            let owned = script(&fixture, fixture.paths.root(), "synthetic", &marker);
            let server = hub(listener, None);
            let result = run(
                &fixture,
                owned.to_str().unwrap(),
                headless,
                fixture.paths.root(),
            )
            .await;
            server.abort();
            let _ = server.await;
            assert_eq!(result.unwrap().exit.code, 0);
            assert_eq!(fs::read(&marker).unwrap(), b"synthetic only");
        }
    }
}
