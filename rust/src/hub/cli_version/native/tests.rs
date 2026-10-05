use super::*;
use std::{
    fs,
    io::{Read, Write},
};

const CHILD_TEST: &str = "hub::cli_version::native::tests::owned_cli_version_helper";
const MODE: &str = "MANY_SYNTHETIC_VERSION_HELPER";

#[test]
fn owned_cli_version_helper() {
    let Ok(mode) = std::env::var(MODE) else {
        return;
    };
    match mode.as_str() {
        "mixed" => {
            let mut byte = [0; 1];
            assert_eq!(std::io::stdin().read(&mut byte).unwrap(), 0);
            assert!(std::env::var_os("MANY_AI_CLI").is_none());
            assert_eq!(std::env::var("MANY_SYNTHETIC_EXACT").unwrap(), "last");
            assert!(
                Path::new(&std::env::var("HOME").unwrap())
                    .starts_with(fs::canonicalize(std::env::current_dir().unwrap()).unwrap())
            );
            std::io::stderr().write_all(b"stderr-first\n").unwrap();
            std::io::stderr().flush().unwrap();
            std::io::stdout().write_all(b"stdout-second\n").unwrap();
            std::io::stdout().flush().unwrap();
        }
        "chatty" => {
            std::io::stderr().write_all(&vec![b'E'; 8192]).unwrap();
            std::io::stderr().flush().unwrap();
            std::io::stdout().write_all(&vec![b'O'; 32768]).unwrap();
            std::io::stdout().flush().unwrap();
        }
        "exit" => {
            std::io::stderr().write_all(b"synthetic exit\n").unwrap();
            std::process::exit(7);
        }
        "hang" | "held" => {
            let mut child = std::process::Command::new(std::env::current_exe().unwrap());
            child.args(["--exact", CHILD_TEST, "--nocapture", "--quiet"]);
            child.env(MODE, "descendant");
            let mut child = child.spawn().unwrap();
            fs::write("parent-ready", b"synthetic").unwrap();
            if mode == "held" {
                std::process::exit(0);
            }
            let _ = child.wait();
        }
        "descendant" => {
            fs::write("descendant-ready", b"synthetic").unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(30);
            while !Path::new("release-survival").exists() {
                if std::time::Instant::now() >= deadline {
                    std::process::exit(8);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            fs::write("unexpected-survival", b"synthetic").unwrap();
        }
        _ => std::process::exit(9),
    }
    std::process::exit(0);
}

struct Fixture {
    _directory: tempfile::TempDir,
    paths: RuntimePaths,
    command: CliVersionCommand,
}
fn fixture(mode: &str) -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("trial");
    fs::create_dir(&root).unwrap();
    let paths = RuntimePaths::trial(&root, 48777, &directory.path().join("installed")).unwrap();
    let executable = root.join(if cfg!(windows) {
        "owned-test-helper.exe"
    } else {
        "owned-test-helper"
    });
    fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
    let mut environment = vec![
        format!("{MODE}={mode}"),
        "MANY_SYNTHETIC_EXACT=first".into(),
        "MANY_SYNTHETIC_EXACT=last".into(),
    ];
    #[cfg(windows)]
    for key in ["SystemRoot", "WINDIR"] {
        if let Some(value) = std::env::var_os(key) {
            environment.push(format!("{key}={}", value.to_string_lossy()));
        }
    }
    #[cfg(unix)]
    let _ = &mut environment;
    let command = CliVersionCommand {
        executable: executable.to_string_lossy().into_owned(),
        args: vec![
            "--exact".into(),
            CHILD_TEST.into(),
            "--nocapture".into(),
            "--quiet".into(),
        ],
        cwd: root,
        environment,
        platform: Platform::native(),
        timeout: Duration::from_secs(10),
        output_cap: super::super::OUTPUT_CAP,
    };
    Fixture {
        _directory: directory,
        paths,
        command,
    }
}
fn executor(paths: RuntimePaths) -> NativeCliVersionExecutor {
    NativeCliVersionExecutor::new(paths, TrialVersionPolicy::OwnedSynthetic)
}

#[tokio::test]
async fn owned_native_combined_output_null_stdin_exact_env_exit_and_cap() {
    for (mode, suffix, exit) in [
        ("mixed", "stderr-first\nstdout-second\n", 0),
        ("exit", "synthetic exit\n", 7),
    ] {
        let f = fixture(mode);
        let output = executor(f.paths).execute(f.command).await.unwrap();
        assert!(output.output.ends_with(suffix.as_bytes()));
        assert_eq!(output.exit_code, exit);
        assert!(!output.start_failed && !output.timed_out);
    }
    let f = fixture("chatty");
    let output = executor(f.paths).execute(f.command).await.unwrap();
    assert_eq!(output.output.len(), 8192);
    assert!(output.output.ends_with(&[b'E'; 64]));
    assert!(!output.output.windows(64).any(|bytes| bytes == [b'O'; 64]));
    assert_eq!(output.exit_code, 0);
}

#[tokio::test]
async fn missing_start_is_distinct_from_poststart_incomplete_capture() {
    let mut f = fixture("mixed");
    f.command.executable = f
        .command
        .cwd
        .join("missing-owned-helper")
        .to_string_lossy()
        .into_owned();
    let output = executor(f.paths).execute(f.command).await.unwrap();
    assert!(output.start_failed);
    assert!(output.output.is_empty());
    let f = fixture("held");
    let result = executor(f.paths).execute(f.command).await;
    assert!(matches!(result, Err(CliVersionFailure::ProcessUnavailable)));
}

#[tokio::test]
async fn timeout_kills_owned_descendant() {
    let f = fixture("hang");
    let root = f.command.cwd.clone();
    let output = executor(f.paths).execute(f.command).await.unwrap();
    assert!(output.timed_out && !output.start_failed);
    assert!(root.join("parent-ready").is_file());
    assert_eq!(output.exit_code, if cfg!(windows) { 1 } else { -1 });
    fs::write(root.join("release-survival"), b"synthetic").unwrap();
    tokio::time::sleep(Duration::from_millis(2000)).await;
    assert!(!root.join("unexpected-survival").exists());
}

#[tokio::test]
async fn dropped_executor_future_kills_owned_descendant() {
    let mut f = fixture("hang");
    // This test aborts a live owner after both processes reach their helpers.
    // Keep its command deadline beyond the bounded readiness phase so a
    // command timeout cannot accidentally satisfy the cancellation assertion.
    f.command.timeout = Duration::from_secs(60);
    let root = f.command.cwd.clone();
    let runner = executor(f.paths);
    let task = tokio::spawn(async move { runner.execute(f.command).await });
    let readiness = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if root.join("parent-ready").is_file() && root.join("descendant-ready").is_file() {
                break Ok(());
            }
            if task.is_finished() {
                break Err("executor completed before both owned helpers were ready");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    // Readiness failure must also drop the owned executor future, rather than
    // detaching it by panicking while a JoinHandle is still live.
    task.abort();
    let termination = task.await;
    assert!(
        matches!(readiness, Ok(Ok(()))),
        "owned parent/descendant startup failed: {readiness:?}"
    );
    assert!(matches!(termination, Err(error) if error.is_cancelled()));
    fs::write(root.join("release-survival"), b"synthetic").unwrap();
    tokio::time::sleep(Duration::from_millis(2000)).await;
    assert!(!root.join("unexpected-survival").exists());
}

#[tokio::test]
async fn trial_default_external_path_and_vendor_home_alias_are_blocked() {
    let f = fixture("mixed");
    assert!(matches!(
        NativeCliVersionExecutor::new(f.paths.clone(), TrialVersionPolicy::Disabled)
            .execute(f.command)
            .await,
        Err(CliVersionFailure::TrialDisabled)
    ));
    let mut f = fixture("mixed");
    f.command.executable = std::env::current_exe()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(matches!(
        executor(f.paths).execute(f.command).await,
        Err(CliVersionFailure::TrialBoundary)
    ));
    let mut f = fixture("mixed");
    f.command.cwd = f._directory.path().to_owned();
    assert!(matches!(
        executor(f.paths).execute(f.command).await,
        Err(CliVersionFailure::TrialBoundary)
    ));
    #[cfg(unix)]
    {
        let f = fixture("mixed");
        std::os::unix::fs::symlink(f._directory.path(), f.command.cwd.join("home")).unwrap();
        assert!(matches!(
            executor(f.paths).execute(f.command).await,
            Err(CliVersionFailure::TrialBoundary)
        ));
    }
}

#[test]
fn environment_deduplicates_case_insensitively_only_on_windows() {
    let entries = vec!["Path=first".into(), "PATH=last".into()];
    let windows = exact_environment(&entries, Platform::Windows).unwrap();
    assert_eq!(windows.len(), 1);
    assert_eq!(
        windows.get(&OsString::from("PATH")),
        Some(&Some(OsString::from("last")))
    );
    assert_eq!(
        exact_environment(&entries, Platform::Unix).unwrap().len(),
        2
    );
    assert!(matches!(
        exact_environment(&["bad\0=value".into()], Platform::native()),
        Err(CliVersionFailure::InvalidEnvironment)
    ));
}
