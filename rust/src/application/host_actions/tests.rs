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
        PathBuf::from(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"),
        vec![
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-Command".into(),
            "[Console]::WriteLine('synthetic-owned-path')".into(),
        ],
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
    let mut waiting = plan;
    #[cfg(windows)]
    {
        waiting.args = vec![
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-Command".into(),
            "Start-Sleep -Seconds 60".into(),
        ];
    }
    #[cfg(unix)]
    {
        waiting.args = vec!["-c".into(), "sleep 60".into()];
    }
    let handle = owner.handle();
    let wait = tokio::spawn(async move { pick_owned(waiting, &handle).await });
    tokio::time::sleep(Duration::from_millis(100)).await;
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
    let owner = HubTaskOwner::new(tokio::runtime::Handle::current());
    // Exercise the dispatch Start receipt using a synthetic non-GUI command.
    // The native GUI API remains uninvoked in this acceptance.
    #[cfg(windows)]
    let (program, args) = {
        let script = root.path().join("detached-fixture.ps1");
        std::fs::write(
            &script,
            "Start-Sleep -Milliseconds 400; [IO.File]::WriteAllText($args[0], 'completed')",
        )
        .unwrap();
        (
            r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe",
            vec![
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-ExecutionPolicy".into(),
                "Bypass".into(),
                "-File".into(),
                script.as_os_str().to_owned(),
                marker.as_os_str().to_owned(),
            ],
        )
    };
    #[cfg(unix)]
    let (program, args) = (
        "/bin/sh",
        vec![
            "-c".into(),
            "sleep 0.4; printf completed > \"$1\"".into(),
            "host-fixture".into(),
            marker.as_os_str().to_owned(),
        ],
    );
    let native = NativeHostDispatch::new(
        RuntimePaths::production(root.path()).unwrap(),
        root.path().to_owned(),
        synthetic_environment(root.path()),
        owner.handle(),
    )
    .unwrap();
    native.detached(program, args).unwrap();
    owner.stop_requests();
    owner.stop_effects().unwrap();
    owner.cancel_effects().unwrap();
    drop(owner);
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if marker.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(std::fs::read_to_string(marker).unwrap(), "completed");
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
