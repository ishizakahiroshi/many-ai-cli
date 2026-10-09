use super::*;
use crate::hub::task_owner::HubTaskOwner;
#[tokio::test]
async fn trial_refuses_native_dispatch_without_lookup_or_gui_launch() {
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49298, installed.path()).unwrap();
    let owner = HubTaskOwner::new(tokio::runtime::Handle::current());
    let native =
        NativeHostDispatch::new(paths, root.path().to_owned(), vec![], owner.handle()).unwrap();
    assert_eq!(
        native.pick(PickerKind::Directory).await.unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_eq!(
        native
            .open(OpenKind::Terminal, root.path(), "missing-host-program")
            .unwrap_err()
            .kind(),
        io::ErrorKind::PermissionDenied
    );
}
#[tokio::test]
async fn synthetic_picker_captures_native_output_and_owner_cancellation_reaps() {
    let root = tempfile::tempdir().unwrap();
    let owner = HubTaskOwner::new(tokio::runtime::Handle::current());
    #[cfg(windows)]
    let (executable, args) = (
        // Fixed stdout uses cmd.exe's built-in echo, avoiding PowerShell startup.
        // Production picker scripts still use PowerShell; this does not test them.
        PathBuf::from(std::env::var_os("SystemRoot").expect("Windows system directory"))
            .join("System32")
            .join("cmd.exe"),
        vec!["/D".into(), "/C".into(), "echo synthetic-owned-path".into()],
    );
    #[cfg(unix)]
    let (executable, args) = (
        PathBuf::from("/bin/sh"),
        vec!["-c".into(), "printf 'synthetic-owned-path\\n'".into()],
    );
    let plan = ProcessPlan {
        executable,
        args,
        cwd: root.path().to_owned(),
        env: synthetic_environment(root.path())
            .into_iter()
            .filter_map(|s| {
                s.split_once('=')
                    .map(|(k, v)| (OsString::from(k), Some(OsString::from(v))))
            })
            .collect(),
        stdin: vec![],
        timeout: Duration::ZERO,
        output_cap: 1024,
        pipe_drain_timeout: Duration::from_secs(2),
    };
    let selected = tokio::time::timeout(
        Duration::from_secs(20),
        pick_owned(plan.clone(), &owner.handle()),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(selected, "synthetic-owned-path");
    let started = root.path().join("picker-started.txt");
    let release = root.path().join("picker-release.txt");
    let marker = root.path().join("picker-completed.txt");
    let failure = root.path().join("picker-failure.txt");
    let mut waiting = plan;
    // Exercise the real owned-process path with a child that explicitly
    // acknowledges readiness and stays alive until the owner cancels it.
    waiting.executable = std::env::current_exe().unwrap();
    waiting.args = vec![
        "--exact".into(),
        "application::host_actions::tests::detached_fixture_child".into(),
    ];
    waiting.env.extend([
        ("MANY_AI_HOST_ACTIONS_CHILD_RUN".into(), Some("1".into())),
        (
            "MANY_AI_HOST_ACTIONS_CHILD_STARTED".into(),
            Some(started.as_os_str().to_owned()),
        ),
        (
            "MANY_AI_HOST_ACTIONS_CHILD_RELEASE".into(),
            Some(release.as_os_str().to_owned()),
        ),
        (
            "MANY_AI_HOST_ACTIONS_CHILD_MARKER".into(),
            Some(marker.as_os_str().to_owned()),
        ),
        (
            "MANY_AI_HOST_ACTIONS_CHILD_FAILURE".into(),
            Some(failure.as_os_str().to_owned()),
        ),
    ]);
    let handle = owner.handle();
    let wait = tokio::spawn(async move { pick_owned(waiting, &handle).await });
    // This wait is harness setup, outside the unchanged cancellation deadline.
    wait_for_child_receipt(&started, &failure, "picker readiness").await;
    let pid: i64 = std::fs::read_to_string(&started).unwrap().parse().unwrap();
    assert!(crate::process::pid_alive(pid), "picker child was not alive");
    assert!(
        !wait.is_finished(),
        "picker ended before owner cancellation"
    );
    assert!(
        !failure.exists(),
        "picker fixture failed before cancellation"
    );
    owner.stop_requests();
    owner.stop_effects().unwrap();
    owner.cancel_effects().unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(20), wait)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    assert!(
        !crate::process::pid_alive(pid),
        "owned picker child survived cancellation"
    );
    assert!(!marker.exists(), "picker fixture completed without release");
    assert!(
        !failure.exists(),
        "picker fixture timed out instead of being reaped"
    );
}
#[test]
fn fixed_windows_picker_scripts_preserve_owner_dialog_and_exact_executable_filter() {
    let scripts: serde_json::Value =
        serde_json::from_str(include_str!("picker_scripts.json")).unwrap();
    for key in ["directory", "file", "exe"] {
        let script = scripts[key].as_str().unwrap();
        assert!(script.contains("SetForegroundWindow($owner.Handle)"));
        assert!(script.contains("ShowDialog($owner)"));
        assert!(script.contains("finally"));
    }
    assert!(
        scripts["exe"]
            .as_str()
            .unwrap()
            .contains("Executable (*.exe)|*.exe|All files (*.*)|*.*")
    );
    assert_eq!(
        terminal_description("synthetic-terminal"),
        "synthetic-terminal <dir>"
    );
}

#[tokio::test]
async fn synthetic_detached_child_survives_hub_effect_shutdown() {
    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("dispatch-receipt.txt");
    let started = root.path().join("dispatch-started.txt");
    let release = root.path().join("dispatch-release.txt");
    let failure = root.path().join("dispatch-failure.txt");
    let owner = HubTaskOwner::new(tokio::runtime::Handle::current());
    // Use this test binary as the synthetic command on every OS. This avoids
    // making the process-lifetime contract depend on a shell or interpreter.
    let mut environment = synthetic_environment(root.path());
    environment.extend([
        "MANY_AI_HOST_ACTIONS_CHILD_RUN=1".into(),
        format!("MANY_AI_HOST_ACTIONS_CHILD_STARTED={}", started.display()),
        format!("MANY_AI_HOST_ACTIONS_CHILD_RELEASE={}", release.display()),
        format!("MANY_AI_HOST_ACTIONS_CHILD_MARKER={}", marker.display()),
        format!("MANY_AI_HOST_ACTIONS_CHILD_FAILURE={}", failure.display()),
    ]);
    let executable = std::env::current_exe().unwrap();
    let native = NativeHostDispatch::new(
        RuntimePaths::production(root.path()).unwrap(),
        root.path().to_owned(),
        environment,
        owner.handle(),
    )
    .unwrap();
    native
        .detached(
            executable.to_str().unwrap(),
            vec![
                "--exact".into(),
                "application::host_actions::tests::detached_fixture_child".into(),
            ],
        )
        .unwrap();
    wait_for_child_receipt(&started, &failure, "start").await;
    owner.stop_requests();
    owner.stop_effects().unwrap();
    owner.cancel_effects().unwrap();
    drop(owner);
    write_child_receipt(&release, b"release").unwrap();
    wait_for_child_receipt(&marker, &failure, "completion").await;
    assert_eq!(std::fs::read_to_string(marker).unwrap(), "completed");
}

#[test]
fn detached_fixture_child() {
    if std::env::var_os("MANY_AI_HOST_ACTIONS_CHILD_RUN").is_none() {
        return;
    }
    let started = std::env::var_os("MANY_AI_HOST_ACTIONS_CHILD_STARTED").unwrap();
    let release = std::env::var_os("MANY_AI_HOST_ACTIONS_CHILD_RELEASE").unwrap();
    let marker = std::env::var_os("MANY_AI_HOST_ACTIONS_CHILD_MARKER").unwrap();
    let failure = std::env::var_os("MANY_AI_HOST_ACTIONS_CHILD_FAILURE").unwrap();
    let result = (|| -> io::Result<()> {
        write_child_receipt(
            Path::new(&started),
            std::process::id().to_string().as_bytes(),
        )?;
        let release = Path::new(&release);
        // Harness fail-safe only: allow the parent's readiness wait and its
        // unchanged cancellation deadline to finish before self-reporting failure.
        let deadline = std::time::Instant::now() + Duration::from_secs(120);
        while !release.exists() {
            if std::time::Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "parent did not release the detached child after shutdown",
                ));
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        write_child_receipt(Path::new(&marker), b"completed")
    })();
    if let Err(error) = result {
        let _ = write_child_receipt(
            Path::new(&failure),
            format!("{:?}", error.kind()).as_bytes(),
        );
        panic!("detached fixture child could not write its receipt");
    }
}

fn write_child_receipt(path: &Path, contents: &[u8]) -> io::Result<()> {
    let temporary = path.with_extension("pending");
    std::fs::write(&temporary, contents)?;
    std::fs::rename(temporary, path)
}

async fn wait_for_child_receipt(marker: &Path, failure: &Path, phase: &str) {
    // Receipt waits are harness scheduling bounds, not product timing budgets.
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            if marker.exists() {
                break;
            }
            if failure.exists() {
                panic!(
                    "detached fixture child failed during {phase}: {}",
                    std::fs::read_to_string(failure)
                        .unwrap_or_else(|_| "unreadable error receipt".into())
                );
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("detached fixture child timed out during {phase}"));
}

fn synthetic_environment(root: &Path) -> Vec<String> {
    let mut values: Vec<String> = ["HOME", "USERPROFILE", "TEMP", "TMP"]
        .into_iter()
        .map(|key| format!("{key}={}", root.display()))
        .collect();
    #[cfg(windows)]
    values.push(format!(
        "SystemRoot={}",
        std::env::var("SystemRoot").expect("Windows system directory")
    ));
    #[cfg(unix)]
    values.push("PATH=/usr/bin:/bin".into());
    values
}
