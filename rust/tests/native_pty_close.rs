#![cfg(windows)]

use many_ai_cli::process::{
    Cancellation, ExitOutcome, ProcessPlan, SpawnOptions, pid_alive,
    pty::{NativePtyFactory, PtyFactory, PtySize},
    run_capped_with_options,
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path, time::Duration};
use windows_sys::Win32::System::Diagnostics::Debug::{
    GetErrorMode, SEM_FAILCRITICALERRORS, SEM_NOGPFAULTERRORBOX, SetErrorMode,
};

fn plan(root: &Path) -> ProcessPlan {
    ProcessPlan {
        executable: std::env::current_exe().unwrap(),
        args: vec![
            "--exact".into(),
            "terminal_child".into(),
            "--nocapture".into(),
        ],
        cwd: root.to_path_buf(),
        env: BTreeMap::new(),
        stdin: Vec::new(),
        pipe_drain_timeout: Duration::from_secs(1),
        output_cap: 8192,
        timeout: Duration::from_secs(20),
    }
}

#[test]
fn terminal_child() {
    let Some(marker) = std::env::var_os("MANY_AI_TEST_TERMINAL_READY") else {
        return;
    };
    std::fs::write(
        marker,
        json!({"pid":std::process::id(),"error_mode":unsafe { GetErrorMode() }}).to_string(),
    )
    .unwrap();
    if std::env::var_os("MANY_AI_TEST_TERMINAL_NATURAL").is_none() {
        std::thread::sleep(Duration::from_secs(60));
    }
}

async fn wait_ready(path: &Path, pid: u32) -> Value {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(bytes) = std::fs::read(path)
                && let Ok(value) = serde_json::from_slice::<Value>(&bytes)
            {
                assert_eq!(value["pid"], pid);
                assert_eq!(value["error_mode"].as_u64().unwrap() & 3, 3);
                return value;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("helper must reach its own entry point")
}

async fn scenario(root: &Path, name: &str, ready: bool, natural: bool) -> Value {
    let marker = root.join(format!("{name}.json"));
    let mut child = plan(root);
    child.env.insert(
        "MANY_AI_TEST_TERMINAL_READY".into(),
        Some(marker.clone().into_os_string()),
    );
    if natural {
        child
            .env
            .insert("MANY_AI_TEST_TERMINAL_NATURAL".into(), Some("1".into()));
    }
    let pty = NativePtyFactory.spawn(&child, PtySize::default()).unwrap();
    if ready {
        wait_ready(&marker, pty.pid()).await;
    }
    let exit = if natural {
        let exit = tokio::time::timeout(Duration::from_secs(7), pty.wait())
            .await
            .unwrap()
            .unwrap();
        pty.close();
        pty.close();
        assert_eq!(exit.code, 0);
        assert!(!exit.forced);
        exit
    } else {
        pty.close();
        pty.close();
        let exit = tokio::time::timeout(Duration::from_secs(7), pty.wait())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            exit.code, 1,
            "{name}: shutdown must not cause DLL initialization failure"
        );
        assert!(
            exit.forced,
            "{name}: explicit disposal must record its hard stop"
        );
        exit
    };
    assert_eq!(pty.wait().await.unwrap(), exit);
    assert!(!pid_alive(i64::from(pty.pid())));
    let proof = pty.terminate_checked(Duration::from_secs(2)).await.unwrap();
    assert!(proof.root_waited());
    assert_eq!(proof.active_processes(), 0);
    assert_eq!(proof.root_pid(), pty.pid());
    json!({"scenario":name,"code":exit.code,"forced":exit.forced,"local_tree_ceased":true,"helper_ready":marker.exists()})
}

#[test]
fn terminal_close_driver() {
    let Some(receipt) = std::env::var_os("MANY_AI_TEST_TERMINAL_DRIVER") else {
        return;
    };
    // This is a dedicated --exact child, never the shared test process or Hub.
    unsafe {
        SetErrorMode(GetErrorMode() | SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX);
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    let trials = runtime.block_on(async {
        let unrelated_marker = root.path().join("unrelated.json");
        let mut unrelated_plan = plan(root.path());
        unrelated_plan.env.insert(
            "MANY_AI_TEST_TERMINAL_READY".into(),
            Some(unrelated_marker.clone().into_os_string()),
        );
        let unrelated = NativePtyFactory
            .spawn(&unrelated_plan, PtySize::default())
            .unwrap();
        wait_ready(&unrelated_marker, unrelated.pid()).await;
        let mut trials = Vec::new();
        for (name, ready, natural) in [
            ("immediate-1", false, false),
            ("ready-close", true, false),
            ("immediate-2", false, false),
            ("natural-exit", true, true),
        ] {
            trials.push(scenario(root.path(), name, ready, natural).await);
            assert!(
                pid_alive(i64::from(unrelated.pid())),
                "disposal must preserve unrelated process"
            );
        }
        unrelated.close();
        tokio::time::timeout(Duration::from_secs(7), unrelated.wait())
            .await
            .unwrap()
            .unwrap();
        trials
    });
    std::fs::write(receipt, json!({"driver_pid":std::process::id(),"driver_error_mode":unsafe { GetErrorMode() },"trials":trials,"unrelated_process_preserved":true}).to_string()).unwrap();
}

#[tokio::test]
async fn normal_terminal_disposal_waits_for_tree_before_closing_conpty() {
    let parent_mode = unsafe { GetErrorMode() };
    let root = tempfile::tempdir().unwrap();
    let receipt = root.path().join("receipt.json");
    let mut driver = plan(root.path());
    driver.args = vec![
        "--exact".into(),
        "terminal_close_driver".into(),
        "--nocapture".into(),
    ];
    driver.env.insert(
        "MANY_AI_TEST_TERMINAL_DRIVER".into(),
        Some(receipt.clone().into_os_string()),
    );
    let result = run_capped_with_options(
        &driver,
        &Cancellation::default(),
        SpawnOptions {
            no_window: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        unsafe { GetErrorMode() },
        parent_mode,
        "shared parent error mode must not change"
    );
    assert_eq!(
        result.outcome,
        ExitOutcome::Exited {
            code: Some(0),
            signal: None
        },
        "{result:?}"
    );
    let receipt: Value = serde_json::from_slice(&std::fs::read(receipt).unwrap()).unwrap();
    assert_eq!(receipt["trials"].as_array().unwrap().len(), 4);
    assert_eq!(receipt["driver_error_mode"].as_u64().unwrap() & 3, 3);
    assert_eq!(receipt["unrelated_process_preserved"], true);
    println!("{receipt}");
}
