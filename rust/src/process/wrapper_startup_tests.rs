use super::*;
use crate::{
    config::private_io,
    proto::core::{LiveSessionId, SessionIncarnation, WrapperConnectionId},
    wrapper::startup::StartupLifetime,
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Instant,
};

const FIXTURE: &str = "process::wrapper_startup::tests::startup_fixture";
const MODE: &str = "MANY_AI_SYNTHETIC_WRAPPER_MODE";
const ROOT: &str = "MANY_AI_SYNTHETIC_WRAPPER_ROOT";
fn plan(root: &Path, mode: &str) -> ProcessPlan {
    ProcessPlan {
        executable: std::env::current_exe().unwrap(),
        args: vec!["--exact".into(), FIXTURE.into(), "--nocapture".into()],
        cwd: root.into(),
        env: BTreeMap::from([
            (MODE.into(), Some(mode.into())),
            (ROOT.into(), Some(root.as_os_str().into())),
            ("HOME".into(), Some(root.as_os_str().into())),
            ("USERPROFILE".into(), Some(root.as_os_str().into())),
        ]),
        stdin: vec![],
        timeout: Duration::ZERO,
        output_cap: 0,
        pipe_drain_timeout: Duration::from_millis(100),
    }
}
fn launch(root: &Path, mode: &str) -> WrapperStartup {
    let log = private_io::open_append(&root.join("spawn.log")).unwrap();
    WrapperStartup::spawn(
        SpawnAttemptId(7),
        &plan(root, mode),
        log.try_clone().unwrap(),
        log,
    )
    .unwrap()
}
fn receipt(_owner: &WrapperStartup, attempt: u64, pid: u32) -> SpawnRegistrationReceipt {
    SpawnRegistrationReceipt::proof_bound(
        SpawnAttemptId(attempt),
        SessionBinding {
            session: LiveSessionId(19),
            incarnation: SessionIncarnation(23),
            wrapper: WrapperConnectionId(29),
        },
        pid,
    )
}
async fn file(root: &Path, name: &str) -> String {
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(value) = std::fs::read_to_string(root.join(name))
            && !value.is_empty()
        {
            return value;
        }
        assert!(
            Instant::now() < end,
            "synthetic wrapper did not reach {name}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
async fn exit(receipt: &ReapReceipt) -> WrapperExit {
    tokio::time::timeout(Duration::from_secs(5), receipt.wait())
        .await
        .unwrap()
        .unwrap()
}
fn alive(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        && stat
            .rsplit_once(") ")
            .is_some_and(|(_, rest)| rest.starts_with('Z'))
    {
        return false;
    }
    crate::process::pid_alive(i64::from(pid))
}
async fn gone(pid: u32) {
    let end = Instant::now() + Duration::from_secs(5);
    while alive(pid) {
        assert!(Instant::now() < end, "owned synthetic child remains alive");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

// This helper is inert in an ordinary test run. Child runs use only their own
// temporary root, current test executable and synthetic environment.
#[test]
fn startup_fixture() {
    let Ok(mode) = std::env::var(MODE) else {
        return;
    };
    let root = PathBuf::from(std::env::var_os(ROOT).expect("synthetic root"));
    if mode == "descendant" {
        assert!(std::env::var_os(STARTUP_JOB_ENV).is_none());
        #[cfg(windows)]
        {
            use windows_sys::Win32::{Foundation::HANDLE, System::JobObjects::*};
            let old = std::env::var("MANY_AI_SYNTHETIC_JOB_NUMBER")
                .unwrap()
                .parse::<usize>()
                .unwrap();
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
            assert_eq!(
                unsafe {
                    QueryInformationJobObject(
                        old as HANDLE,
                        JobObjectExtendedLimitInformation,
                        (&mut limits as *mut JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                        std::mem::size_of_val(&limits) as u32,
                        std::ptr::null_mut(),
                    )
                },
                0
            );
        }
        std::fs::write(root.join("provider-clean"), b"bootstrap-not-inherited").unwrap();
        std::thread::sleep(Duration::from_secs(20));
        std::process::exit(0);
    }
    let lifetime = StartupLifetime::adopt_from_environment().unwrap();
    #[cfg(unix)]
    unsafe {
        assert_eq!(libc::getsid(0), std::process::id() as i32);
        assert_eq!(libc::getpgrp(), std::process::id() as i32);
    }
    #[cfg(windows)]
    assert!(StartupLifetime::adopt_from_environment().is_err());
    {
        use std::io::Read;
        let mut byte = [0u8; 1];
        assert_eq!(std::io::stdin().read(&mut byte).unwrap(), 0);
    }
    println!("synthetic-wrapper-stdout");
    eprintln!("synthetic-wrapper-stderr");
    std::fs::write(root.join("ready"), std::process::id().to_string()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    if mode == "await_ack" {
        while !root.join("ack").exists() {
            if Instant::now() >= deadline {
                lifetime.exit(91);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    if mode == "early_exit" {
        lifetime.exit(37);
    }
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", FIXTURE, "--nocapture"])
        .env(MODE, "descendant")
        .env_remove(STARTUP_JOB_ENV)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    command.env(
        "MANY_AI_SYNTHETIC_JOB_NUMBER",
        std::env::var_os(STARTUP_JOB_ENV).unwrap(),
    );
    let mut child = command.spawn().unwrap();
    std::fs::write(root.join("descendant"), child.id().to_string()).unwrap();
    if mode == "exit_with_descendant" {
        lifetime.exit(41);
    }
    while !root.join("stop").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    let _ = child.kill();
    let _ = child.wait();
    lifetime.exit(23);
}

#[tokio::test]
async fn failed_exec_and_non_null_stdin_create_no_wrapper() {
    let root = tempfile::tempdir().unwrap();
    let mut launch = plan(root.path(), "early_exit");
    launch.executable = root.path().join("missing-wrapper");
    let log = private_io::open_append(&root.path().join("spawn.log")).unwrap();
    assert!(
        WrapperStartup::spawn(SpawnAttemptId(7), &launch, log.try_clone().unwrap(), log).is_err()
    );
    let mut launch = plan(root.path(), "early_exit");
    launch.stdin = vec![1];
    let log = private_io::open_append(&root.path().join("spawn.log")).unwrap();
    assert!(
        WrapperStartup::spawn(SpawnAttemptId(7), &launch, log.try_clone().unwrap(), log).is_err()
    );
    assert!(!root.path().join("ready").exists());
}
#[tokio::test]
async fn unacknowledged_owner_future_drop_reaps_wrapper_and_owned_descendant() {
    let root = tempfile::tempdir().unwrap();
    let owner = launch(root.path(), "startup_child");
    let observed = owner.receipt();
    let child = file(root.path(), "descendant").await.parse().unwrap();
    let owned_future = async move {
        let _owner = owner;
        std::future::pending::<()>().await;
    };
    assert!(
        tokio::time::timeout(Duration::from_millis(20), owned_future)
            .await
            .is_err()
    );
    exit(&observed).await;
    gone(child).await;
}
#[tokio::test]
async fn failed_or_lost_initial_ack_does_not_start_provider() {
    let root = tempfile::tempdir().unwrap();
    let owner = launch(root.path(), "await_ack");
    file(root.path(), "ready").await;
    let observed = owner.abort();
    exit(&observed).await;
    assert!(!root.path().join("descendant").exists());
}
#[tokio::test]
async fn early_exit_cleans_startup_descendant_and_preserves_status() {
    let root = tempfile::tempdir().unwrap();
    let owner = launch(root.path(), "exit_with_descendant");
    let child = file(root.path(), "descendant").await.parse().unwrap();
    assert_eq!(exit(&owner.receipt()).await.code, Some(41));
    gone(child).await;
}
#[tokio::test]
async fn mismatched_attempt_or_owned_pid_cannot_transfer() {
    for wrong_attempt in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let owner = launch(root.path(), "await_ack");
        file(root.path(), "ready").await;
        let observed = owner.receipt();
        let proof = receipt(
            &owner,
            if wrong_attempt { 8 } else { 7 },
            if wrong_attempt { owner.pid() } else { 1 },
        );
        assert!(owner.begin_registration(proof).is_err());
        exit(&observed).await;
        assert!(!root.path().join("descendant").exists());
    }
}
#[tokio::test]
async fn acknowledged_transfer_survives_owner_shutdown_and_exits_explicitly() {
    let root = tempfile::tempdir().unwrap();
    let unrelated_root = tempfile::tempdir().unwrap();
    let unrelated = launch(unrelated_root.path(), "await_ack");
    file(unrelated_root.path(), "ready").await;
    let owner = launch(root.path(), "await_ack");
    file(root.path(), "ready").await;
    let proof = receipt(&owner, 7, owner.pid());
    let provisional = owner.begin_registration(proof).unwrap();
    assert_eq!(provisional.binding().session, LiveSessionId(19));
    std::fs::write(root.path().join("ack"), b"registered").unwrap();
    let observed = provisional.acknowledged();
    let child = file(root.path(), "descendant").await.parse().unwrap();
    file(root.path(), "provider-clean").await;
    assert!(alive(observed.pid()) && alive(child));
    assert!(alive(unrelated.pid()));
    std::fs::write(root.path().join("stop"), b"explicit-wrapper-close").unwrap();
    assert_eq!(exit(&observed).await.code, Some(23));
    assert_eq!(exit(&observed).await.code, Some(23));
    gone(child).await;
    assert!(alive(unrelated.pid()));
    exit(&unrelated.abort()).await;
    let log = std::fs::read_to_string(root.path().join("spawn.log")).unwrap();
    assert!(log.contains("synthetic-wrapper-stdout") && log.contains("synthetic-wrapper-stderr"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(root.path().join("spawn.log"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}
#[tokio::test]
async fn delivery_uncertain_provisional_drop_detaches_but_known_pre_ack_failure_aborts() {
    for uncertain in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let owner = launch(root.path(), "await_ack");
        file(root.path(), "ready").await;
        let observed = owner.receipt();
        let proof = receipt(&owner, 7, owner.pid());
        let provisional = owner.begin_registration(proof).unwrap();
        if uncertain {
            std::fs::write(root.path().join("ack"), b"registered").unwrap();
            drop(provisional);
            file(root.path(), "provider-clean").await;
            assert!(alive(observed.pid()));
            std::fs::write(root.path().join("stop"), b"explicit-close").unwrap();
            assert_eq!(exit(&observed).await.code, Some(23));
        } else {
            provisional.abort();
            exit(&observed).await;
            assert!(!root.path().join("descendant").exists());
        }
    }
}

#[test]
fn detached_child_reaping_survives_hub_runtime_shutdown() {
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let observed = runtime.block_on(async {
        let owner = launch(root.path(), "await_ack");
        file(root.path(), "ready").await;
        let proof = receipt(&owner, 7, owner.pid());
        let provisional = owner.begin_registration(proof).unwrap();
        std::fs::write(root.path().join("ack"), b"registered").unwrap();
        let observed = provisional.acknowledged();
        file(root.path(), "provider-clean").await;
        observed
    });
    drop(runtime);
    assert!(alive(observed.pid()));
    std::fs::write(root.path().join("stop"), b"explicit-wrapper-close").unwrap();
    let next_runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    assert_eq!(next_runtime.block_on(exit(&observed)).code, Some(23));
}
