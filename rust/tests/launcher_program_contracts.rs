use many_ai_cli::{
    application::launcher_program,
    cli,
    config::RuntimePaths,
    launcher::{ConnectorConfig, OutputSink},
    process::{Cancellation, OutputStream},
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

type RecordedOutput = Arc<Mutex<Vec<(OutputStream, Vec<u8>)>>>;
fn recording() -> (OutputSink, RecordedOutput) {
    let log = Arc::new(Mutex::new(Vec::new()));
    let copy = log.clone();
    (
        Arc::new(move |stream, bytes| copy.lock().unwrap().push((stream, bytes.to_vec()))),
        log,
    )
}

#[test]
fn launcher_help_prints_banner_before_usage_without_home_or_store_access() {
    let (sink, log) = recording();
    let result = launcher_program::prepare_launcher(&["--help".into()], "1.2.3", &sink).unwrap();
    assert!(result.is_none());
    let log = log.lock().unwrap();
    assert!(matches!(log[0].0, OutputStream::Stdout));
    assert!(String::from_utf8_lossy(&log[0].1).contains("1.2.3"));
    assert!(matches!(log[1].0, OutputStream::Stderr));
    assert_eq!(log[1].1, launcher_program::LAUNCHER_USAGE.as_bytes());
}

#[test]
fn main_connect_rejects_launcher_only_ui_flag_but_profile_value_is_opaque() {
    assert_eq!(
        cli::parse_connect(&["--ui".into()]).unwrap_err(),
        "flag provided but not defined: -ui"
    );
    assert_eq!(
        cli::parse_connect(&["--profile".into(), "--ui".into()])
            .unwrap()
            .profile,
        "--ui"
    );
    assert!(
        !cli::parse_connect(&["positional".into(), "--ui".into()])
            .unwrap()
            .open_ui
    );
}

#[tokio::test]
async fn trial_launcher_default_serves_local_ui_without_automatic_browser_and_joins_shutdown() {
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49001, installed.path()).unwrap();
    let (sink, log) = recording();
    let mut connector = ConnectorConfig::new(paths, root.path().into());
    connector.output = Some(sink);
    let cancel = Cancellation::default();
    let stop = cancel.clone();
    let opened = Arc::new(AtomicUsize::new(0));
    let copy = opened.clone();
    let task = tokio::spawn(async move {
        launcher_program::run_launcher(&[], connector, "synthetic", &stop, &move |_| {
            copy.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .await
    });
    let url = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let line = log.lock().unwrap().iter().find_map(|(_, bytes)| {
                String::from_utf8_lossy(bytes)
                    .strip_prefix("Opening connection selection page: ")
                    .map(|value| value.trim().to_owned())
            });
            if let Some(url) = line {
                break url;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let response = client.get(url).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        response.bytes().await.unwrap().as_ref(),
        many_ai_cli::assets::LAUNCHER_UI
    );
    assert_eq!(opened.load(Ordering::SeqCst), 0);
    cancel.cancel();
    tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        !root.path().join("config.yaml").exists(),
        "standalone launcher must not create main config"
    );
}

#[tokio::test]
async fn connect_missing_selection_fails_before_opening_profile_store() {
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49001, installed.path()).unwrap();
    let connector = ConnectorConfig::new(paths, root.path().into());
    let error = launcher_program::run_connect(&[], connector, &Cancellation::default(), &|_| {
        panic!("no browser action")
    })
    .await
    .unwrap_err();
    assert_eq!(error, "connect requires --profile <name> or --last");
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

async fn executable(
    binary: &str,
    args: Vec<std::ffi::OsString>,
    cwd: &std::path::Path,
    home: Option<&std::path::Path>,
) -> many_ai_cli::process::ProcessOutput {
    use many_ai_cli::process::{ProcessPlan, run_capped};
    let env = ["HOME", "USERPROFILE"]
        .into_iter()
        .map(|name| (name.into(), home.map(|path| path.as_os_str().to_owned())))
        .collect();
    run_capped(
        &ProcessPlan {
            executable: binary.into(),
            args,
            cwd: cwd.into(),
            env,
            stdin: vec![],
            timeout: Duration::from_secs(5),
            output_cap: 64 * 1024,
            pipe_drain_timeout: Duration::from_secs(1),
        },
        &Cancellation::default(),
    )
    .await
    .unwrap()
}
#[tokio::test]
async fn actual_launcher_help_exits_without_home_resolution_or_files() {
    let root = tempfile::tempdir().unwrap();
    let result = executable(
        env!("CARGO_BIN_EXE_many-ai-cli-launcher"),
        vec!["--help".into()],
        root.path(),
        None,
    )
    .await;
    assert_eq!(
        result.outcome,
        many_ai_cli::process::ExitOutcome::Exited {
            code: Some(0),
            signal: None
        }
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("many-ai-cli"));
    assert_eq!(result.stderr, launcher_program::LAUNCHER_USAGE.as_bytes());
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}
#[tokio::test]
async fn actual_main_shell_init_uses_only_explicit_owned_trial_root() {
    let root = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let result = executable(
        env!("CARGO_BIN_EXE_many-ai-cli"),
        vec![
            "--trial-root".into(),
            root.path().into(),
            "--trial-port".into(),
            "49389".into(),
            "shell-init".into(),
        ],
        root.path(),
        Some(home.path()),
    )
    .await;
    assert_eq!(
        result.outcome,
        many_ai_cli::process::ExitOutcome::Exited {
            code: Some(0),
            signal: None
        }
    );
    assert_eq!(
        String::from_utf8(result.stdout).unwrap(),
        many_ai_cli::wrapper::shell::init_script()
    );
    assert!(result.stderr.is_empty());
    assert!(root.path().join("config.yaml").is_file());
    assert_eq!(std::fs::read_dir(home.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn actual_trial_launcher_binary_retains_and_serves_the_embedded_ui() {
    use many_ai_cli::process::{ExitOutcome, ManagedProcess, ProcessEvent, ProcessPlan};
    let root = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let (mut process, mut events) = ManagedProcess::spawn_owned(
        ProcessPlan {
            executable: env!("CARGO_BIN_EXE_many-ai-cli-launcher").into(),
            args: vec![
                "--trial-root".into(),
                root.path().into(),
                "--trial-port".into(),
                "49388".into(),
            ],
            cwd: root.path().into(),
            env: ["HOME", "USERPROFILE"]
                .into_iter()
                .map(|key| (key.into(), Some(home.path().as_os_str().to_owned())))
                .collect(),
            stdin: vec![],
            timeout: Duration::from_secs(10),
            output_cap: 64 * 1024,
            pipe_drain_timeout: Duration::from_secs(1),
        },
        32,
    );
    let url = tokio::time::timeout(Duration::from_secs(5), async {
        let mut output = vec![];
        loop {
            if let ProcessEvent::Output {
                stream: OutputStream::Stdout,
                bytes,
            } = events.recv().await.unwrap()
            {
                output.extend(bytes);
                let text = String::from_utf8_lossy(&output);
                if let Some(line) = text
                    .split_inclusive('\n')
                    .filter(|line| line.ends_with('\n'))
                    .find_map(|line| line.strip_prefix("Opening connection selection page: "))
                {
                    break line.trim().to_owned();
                }
            }
        }
    })
    .await
    .unwrap();
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let response = client.get(url).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        response.bytes().await.unwrap().as_ref(),
        many_ai_cli::assets::LAUNCHER_UI
    );
    // End only this owned fixture process. Graceful UI/task draining is covered
    // separately by the library shutdown test; this test proves actual linkage.
    process.close();
    let result = tokio::time::timeout(Duration::from_secs(3), process.wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.outcome, ExitOutcome::Cancelled);
    assert_eq!(std::fs::read_dir(home.path()).unwrap().count(), 0);
}
