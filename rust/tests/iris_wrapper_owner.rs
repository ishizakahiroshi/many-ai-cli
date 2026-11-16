//! Independent owner-seam acceptance with synthetic transports and owned PTYs.
use many_ai_cli::{
    process::{
        Cancellation, ProcessPlan,
        pty::{LocalCessation, PtyExit, PtyFactory, PtySession, PtySize},
    },
    proto::{Message, core::CoreFuture},
    wrapper::{
        runtime::{self, ExecutionOwnerObserver, WrapperOptions},
        transport::{HubConnector, HubSocket},
    },
};
use std::{
    collections::BTreeMap,
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

type Trace = Arc<Mutex<Vec<String>>>;
fn mark(trace: &Trace, text: impl Into<String>) {
    trace.lock().unwrap().push(text.into());
}
fn plan(root: &std::path::Path) -> ProcessPlan {
    ProcessPlan {
        executable: "synthetic-never-executed".into(),
        args: vec![],
        cwd: root.into(),
        env: BTreeMap::new(),
        stdin: vec![],
        timeout: Duration::ZERO,
        output_cap: 1024,
        pipe_drain_timeout: Duration::from_millis(50),
    }
}
struct Connector {
    trace: Trace,
    disconnect: bool,
    ready_marker: Option<std::path::PathBuf>,
}
struct Socket {
    trace: Trace,
    registered: bool,
    disconnect: bool,
    ready_marker: Option<std::path::PathBuf>,
}
impl HubConnector for Connector {
    fn connect<'a>(&'a self, _: Option<&'a str>) -> CoreFuture<'a, io::Result<Box<dyn HubSocket>>> {
        Box::pin(async move {
            Ok(Box::new(Socket {
                trace: self.trace.clone(),
                registered: false,
                disconnect: self.disconnect,
                ready_marker: self.ready_marker.clone(),
            }) as Box<dyn HubSocket>)
        })
    }
    fn probe(&self) -> CoreFuture<'_, bool> {
        Box::pin(async { false })
    }
}
impl HubSocket for Socket {
    fn send<'a>(&'a mut self, frame: &'a Message) -> CoreFuture<'a, io::Result<()>> {
        Box::pin(async move {
            mark(&self.trace, format!("send:{}", frame.r#type));
            Ok(())
        })
    }
    fn receive(&mut self) -> CoreFuture<'_, io::Result<Message>> {
        Box::pin(async move {
            if !self.registered {
                self.registered = true;
                return Ok(Message {
                    r#type: "registered".into(),
                    session_id: 71,
                    cols: 80,
                    rows: 24,
                    ..Default::default()
                });
            }
            if self.disconnect {
                if let Some(marker) = &self.ready_marker {
                    // Native creation/resume is not helper readiness. In
                    // particular, never close ConPTY during DLL initialization
                    // merely because the fake registration already completed.
                    tokio::time::timeout(Duration::from_secs(5), async {
                        loop {
                            if std::fs::read_to_string(marker)
                                .ok()
                                .and_then(|v| serde_json::from_str::<serde_json::Value>(&v).ok())
                                .and_then(|v| v["pid"].as_u64())
                                .is_some_and(|pid| pid > 0)
                            {
                                break;
                            }
                            tokio::time::sleep(Duration::from_millis(5)).await;
                        }
                    })
                    .await
                    .map_err(|_| {
                        io::Error::new(
                            io::ErrorKind::TimedOut,
                            "native helper readiness was not observed",
                        )
                    })?;
                    mark(&self.trace, "native-ready-before-disconnect");
                }
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "synthetic socket loss",
                ));
            }
            std::future::pending().await
        })
    }
}
struct Observer {
    trace: Trace,
    fail: &'static str,
    seen: Mutex<Vec<(bool, bool)>>,
}
impl ExecutionOwnerObserver for Observer {
    fn before_spawn(&self, session_id: i64, plan: &ProcessPlan) -> io::Result<()> {
        assert_eq!(session_id, 71);
        assert_eq!(
            plan.env
                .get(std::ffi::OsStr::new("MANY_AI_CLI_SESSION_ID"))
                .and_then(|v| v.as_ref())
                .unwrap(),
            "71"
        );
        mark(&self.trace, "admit");
        if self.fail == "admit" {
            Err(io::Error::other("synthetic durable admission failure"))
        } else {
            Ok(())
        }
    }
    fn root_started(&self, pid: u32) -> io::Result<()> {
        assert!(pid > 0);
        mark(&self.trace, format!("bound:{pid}"));
        if self.fail == "bind" {
            Err(io::Error::other("synthetic root binding failure"))
        } else {
            Ok(())
        }
    }
    fn finished(&self, exit: Option<&PtyExit>, proof: Option<&LocalCessation>) -> io::Result<()> {
        mark(&self.trace, "finished");
        if let Some(proof) = proof {
            assert_eq!(proof.scope(), "windows-owned-job");
            assert!(proof.root_waited());
            assert_eq!(proof.active_processes(), 0);
            assert!(proof.checked_at_unix_ms() > 0);
        }
        self.seen
            .lock()
            .unwrap()
            .push((exit.is_some(), proof.is_some()));
        if self.fail == "finish" {
            Err(io::Error::other(
                "synthetic final receipt persistence failure",
            ))
        } else {
            Ok(())
        }
    }
}
struct Pty {
    trace: Trace,
    ended: Cancellation,
    wait_error: bool,
}
impl PtySession for Pty {
    fn pid(&self) -> u32 {
        7171
    }
    fn read<'a>(&'a self, _: &'a mut [u8]) -> CoreFuture<'a, io::Result<usize>> {
        Box::pin(async move {
            self.ended.cancelled().await;
            Ok(0)
        })
    }
    fn write<'a>(&'a self, bytes: &'a [u8]) -> CoreFuture<'a, io::Result<usize>> {
        Box::pin(async move { Ok(bytes.len()) })
    }
    fn resize(&self, _: PtySize) -> io::Result<()> {
        Ok(())
    }
    fn close(&self) {
        mark(&self.trace, "close:7171");
        self.ended.cancel();
    }
    fn wait(&self) -> CoreFuture<'_, io::Result<PtyExit>> {
        Box::pin(async move {
            if self.wait_error {
                return Err(io::Error::other("synthetic native wait failure"));
            }
            self.ended.cancelled().await;
            Ok(PtyExit {
                code: 0,
                signal: String::new(),
                forced: false,
            })
        })
    }
    fn terminate_checked(&self, _: Duration) -> CoreFuture<'_, io::Result<LocalCessation>> {
        Box::pin(async move {
            mark(&self.trace, "checked:7171");
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "synthetic PTY has no native proof",
            ))
        })
    }
}
struct Factory {
    trace: Trace,
    pty: Arc<Pty>,
    spawns: AtomicUsize,
}
impl PtyFactory for Factory {
    fn spawn(&self, _: &ProcessPlan, _: PtySize) -> io::Result<Arc<dyn PtySession>> {
        self.spawns.fetch_add(1, Ordering::SeqCst);
        mark(&self.trace, "spawn");
        Ok(self.pty.clone())
    }
}
struct Fixture {
    trace: Trace,
    observer: Arc<Observer>,
    connector: Connector,
    factory: Factory,
    root: tempfile::TempDir,
}
impl Fixture {
    fn new(fail: &'static str, disconnect: bool, already_exited: bool, wait_error: bool) -> Self {
        let trace = Arc::new(Mutex::new(vec![]));
        let ended = Cancellation::default();
        if already_exited {
            ended.cancel();
        }
        Self {
            observer: Arc::new(Observer {
                trace: trace.clone(),
                fail,
                seen: Mutex::new(vec![]),
            }),
            connector: Connector {
                trace: trace.clone(),
                disconnect,
                ready_marker: None,
            },
            factory: Factory {
                trace: trace.clone(),
                pty: Arc::new(Pty {
                    trace: trace.clone(),
                    ended,
                    wait_error,
                }),
                spawns: AtomicUsize::new(0),
            },
            trace,
            root: tempfile::tempdir().unwrap(),
        }
    }
    async fn run(&self, observed: bool) -> io::Result<runtime::WrapperResult> {
        let mut options = WrapperOptions::new(
            Message {
                provider: "synthetic".into(),
                token: "synthetic-token".into(),
                ..Default::default()
            },
            plan(self.root.path()),
        );
        options.reconnect_grace = Duration::ZERO;
        options.drain_timeout = Duration::from_millis(50);
        if observed {
            options.execution_owner = Some(self.observer.clone());
        }
        tokio::time::timeout(
            Duration::from_secs(5),
            runtime::run(
                options,
                &self.connector,
                &self.factory,
                &Cancellation::default(),
                |_, _| {
                    mark(&self.trace, "prepare");
                    Ok(())
                },
            ),
        )
        .await
        .expect("wrapper seam deadline")
    }
    fn before(&self, first: &str, second: &str) {
        let trace = self.trace.lock().unwrap();
        assert!(
            trace.iter().position(|v| v == first).unwrap()
                < trace.iter().position(|v| v == second).unwrap(),
            "{trace:?}"
        );
    }
}

#[tokio::test]
async fn owner_admission_precedes_factory_and_final_receipt_precedes_session_end() {
    let f = Fixture::new("", false, true, false);
    f.run(true).await.unwrap();
    f.before("prepare", "admit");
    f.before("admit", "spawn");
    f.before("spawn", "bound:7171");
    f.before("bound:7171", "checked:7171");
    f.before("checked:7171", "finished");
    f.before("finished", "send:session_end");
    assert_eq!(*f.observer.seen.lock().unwrap(), vec![(true, false)]);
}
#[tokio::test]
async fn rejected_durable_admission_never_spawns() {
    let f = Fixture::new("admit", false, true, false);
    assert!(f.run(true).await.is_err());
    assert_eq!(f.factory.spawns.load(Ordering::SeqCst), 0);
    assert!(f.observer.seen.lock().unwrap().is_empty());
}
#[tokio::test]
async fn binding_persistence_failure_stops_exact_owned_pty_without_replay() {
    let f = Fixture::new("bind", false, false, false);
    assert!(f.run(true).await.is_err());
    assert_eq!(f.factory.spawns.load(Ordering::SeqCst), 1);
    f.before("bound:7171", "checked:7171");
    f.before("checked:7171", "close:7171");
    f.before("close:7171", "finished");
    assert_eq!(*f.observer.seen.lock().unwrap(), vec![(true, false)]);
    assert!(
        !f.trace
            .lock()
            .unwrap()
            .iter()
            .any(|v| v == "send:session_end")
    );
}
#[tokio::test]
async fn lost_transport_does_not_skip_local_final_persistence() {
    let f = Fixture::new("", true, false, false);
    f.run(true).await.unwrap();
    f.before("close:7171", "finished");
    assert_eq!(*f.observer.seen.lock().unwrap(), vec![(true, false)]);
}
#[tokio::test]
async fn failed_native_wait_still_checks_cessation_and_persists_unknown() {
    let f = Fixture::new("", false, false, true);
    assert!(f.run(true).await.is_err());
    f.before("checked:7171", "finished");
    assert_eq!(*f.observer.seen.lock().unwrap(), vec![(false, false)]);
}
#[tokio::test]
async fn final_persistence_failure_prevents_session_end() {
    let f = Fixture::new("finish", false, true, false);
    assert!(f.run(true).await.is_err());
    assert_eq!(f.observer.seen.lock().unwrap().len(), 1);
    assert!(
        !f.trace
            .lock()
            .unwrap()
            .iter()
            .any(|v| v == "send:session_end")
    );
}
#[tokio::test]
async fn ordinary_wrapper_without_observer_does_not_invoke_checked_termination() {
    let f = Fixture::new("admit", false, true, false);
    f.run(false).await.unwrap();
    assert!(f.observer.seen.lock().unwrap().is_empty());
    assert!(
        !f.trace
            .lock()
            .unwrap()
            .iter()
            .any(|v| v.starts_with("checked:"))
    );
    assert!(
        f.trace
            .lock()
            .unwrap()
            .iter()
            .any(|v| v == "send:session_end")
    );
}

#[cfg(windows)]
#[test]
fn native_observer_helper() {
    if let Some(marker) = std::env::var_os("MANY_AI_SYNTHETIC_OWNER_HELPER") {
        use windows_sys::Win32::System::Diagnostics::Debug::GetErrorMode;
        std::fs::write(
            marker,
            serde_json::json!({"pid":std::process::id(),"error_mode":unsafe { GetErrorMode() }})
                .to_string(),
        )
        .unwrap();
        std::thread::sleep(Duration::from_secs(60));
    }
}
#[cfg(windows)]
async fn native_owner_scenario() -> serde_json::Value {
    use many_ai_cli::process::pty::NativePtyFactory;
    let mut f = Fixture::new("", true, false, false);
    let ready_marker = f.root.path().join("native-ready");
    f.connector.ready_marker = Some(ready_marker.clone());
    let mut process = plan(f.root.path());
    process.executable = std::env::current_exe().unwrap();
    process.args = vec![
        "--exact".into(),
        "native_observer_helper".into(),
        "--nocapture".into(),
    ];
    process.env.insert(
        "MANY_AI_SYNTHETIC_OWNER_HELPER".into(),
        Some(ready_marker.clone().into_os_string()),
    );
    let mut options = WrapperOptions::new(
        Message {
            provider: "synthetic".into(),
            token: "synthetic-token".into(),
            ..Default::default()
        },
        process,
    );
    options.execution_owner = Some(f.observer.clone());
    options.reconnect_grace = Duration::ZERO;
    options.drain_timeout = Duration::from_millis(50);
    let result = tokio::time::timeout(
        Duration::from_secs(12),
        runtime::run(
            options,
            &f.connector,
            &NativePtyFactory,
            &Cancellation::default(),
            |_, _| Ok(()),
        ),
    )
    .await
    .expect("native owner deadline")
    .unwrap();
    let marker = std::fs::read_to_string(&ready_marker)
        .expect("native helper must actually enter before cessation acceptance");
    let marker: serde_json::Value = serde_json::from_str(&marker).unwrap();
    let pid = marker["pid"].as_u64().unwrap();
    use windows_sys::Win32::System::Diagnostics::Debug::{
        SEM_FAILCRITICALERRORS, SEM_NOGPFAULTERRORBOX,
    };
    let required = SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX;
    let child_error_mode = marker["error_mode"].as_u64().unwrap() as u32;
    assert_eq!(
        child_error_mode & required,
        required,
        "helper must inherit driver-only error dialog suppression"
    );
    assert!(
        f.trace
            .lock()
            .unwrap()
            .iter()
            .any(|v| v == &format!("bound:{pid}"))
    );
    f.before("native-ready-before-disconnect", "finished");
    assert_ne!(
        result.exit.code, 0xc0000142,
        "Windows application initialization failed"
    );
    assert_eq!(*f.observer.seen.lock().unwrap(), vec![(true, true)]);
    assert_eq!(f.factory.spawns.load(Ordering::SeqCst), 0);
    serde_json::json!({"helper_pid":pid,"helper_error_mode":child_error_mode,"native_cessation_observed":true,"helper_exit_code":result.exit.code})
}

/// Only a separately spawned --exact driver with an explicit fixture receipt
/// path mutates its own process error mode. The shared test parent never does.
#[cfg(windows)]
#[test]
fn native_owner_driver() {
    let Some(receipt_path) = std::env::var_os("MANY_AI_SYNTHETIC_OWNER_DRIVER_RECEIPT") else {
        return;
    };
    use windows_sys::Win32::System::Diagnostics::Debug::{
        GetErrorMode, SEM_FAILCRITICALERRORS, SEM_NOGPFAULTERRORBOX, SetErrorMode,
    };
    let required = SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX;
    // This process runs exactly this one test. Windows children inherit this
    // process setting; there is no shared-parent/global OS mutation to restore.
    unsafe {
        SetErrorMode(GetErrorMode() | required);
    }
    let driver_mode = unsafe { GetErrorMode() };
    assert_eq!(driver_mode & required, required);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut receipt = if std::env::var_os("MANY_AI_SYNTHETIC_OWNER_DIAGNOSTIC").as_deref()
        == Some(std::ffi::OsStr::new("1"))
    {
        runtime.block_on(async {
            let mut trials = Vec::new();
            // One positive readiness control, followed by two legacy trials.
            // This explicit opt-in diagnostic never changes default acceptance.
            for wait_marker in [true, false, false] {
                trials.push(native_owner_diagnostic(wait_marker).await);
            }
            serde_json::json!({"trials":trials})
        })
    } else {
        runtime.block_on(native_owner_scenario())
    };
    receipt["driver_pid"] = serde_json::json!(std::process::id());
    receipt["driver_error_mode"] = serde_json::json!(driver_mode);
    std::fs::write(receipt_path, receipt.to_string()).unwrap();
}

#[cfg(windows)]
async fn native_owner_diagnostic(wait_marker: bool) -> serde_json::Value {
    use many_ai_cli::process::pty::NativePtyFactory;
    let mut f = Fixture::new("", true, false, false);
    let marker = f.root.path().join("native-ready");
    if wait_marker {
        f.connector.ready_marker = Some(marker.clone());
    }
    let mut process = plan(f.root.path());
    process.executable = std::env::current_exe().unwrap();
    process.args = vec![
        "--exact".into(),
        "native_observer_helper".into(),
        "--nocapture".into(),
    ];
    process.env.insert(
        "MANY_AI_SYNTHETIC_OWNER_HELPER".into(),
        Some(marker.clone().into_os_string()),
    );
    let mut options = WrapperOptions::new(
        Message {
            provider: "synthetic".into(),
            token: "synthetic-token".into(),
            ..Default::default()
        },
        process,
    );
    options.execution_owner = Some(f.observer.clone());
    options.reconnect_grace = Duration::ZERO;
    options.drain_timeout = Duration::from_millis(50);
    let started = std::time::Instant::now();
    let result = tokio::time::timeout(
        Duration::from_secs(7),
        runtime::run(
            options,
            &f.connector,
            &NativePtyFactory,
            &Cancellation::default(),
            |_, _| Ok(()),
        ),
    )
    .await;
    let marker_value = std::fs::read_to_string(&marker)
        .ok()
        .and_then(|v| serde_json::from_str::<serde_json::Value>(&v).ok());
    let seen = f.observer.seen.lock().unwrap().clone();
    let (status, code, signal, forced) = match result {
        Ok(Ok(exit)) => (
            "returned",
            Some(exit.exit.code),
            Some(exit.exit.signal),
            Some(exit.exit.forced),
        ),
        Ok(Err(_)) => ("io-error", None, None, None),
        Err(_) => ("deadline", None, None, None),
    };
    let helper_ready = marker_value
        .as_ref()
        .is_some_and(|v| v["pid"].as_u64().is_some_and(|pid| pid > 0));
    let local_cessation = seen.iter().any(|(_, proof)| *proof);
    if wait_marker {
        assert!(
            helper_ready && local_cessation && status == "returned",
            "positive native readiness control failed"
        );
    }
    serde_json::json!({"mode":if wait_marker{"wait-marker-control"}else{"legacy-immediate-eof"},"helper_ready":helper_ready,"helper_error_mode":marker_value.as_ref().and_then(|v|v["error_mode"].as_u64()),"status":status,"pty_exit_code":code,"pty_signal":signal,"pty_forced":forced,"local_cessation":local_cessation,"elapsed_ms":started.elapsed().as_millis()})
}

/// Bounded regression: the old owner-close order returns an initialization
/// failure (or close-event status); the explicit owned stop must return code 1.
#[cfg(windows)]
#[tokio::test]
async fn owned_stop_precedes_conpty_close_during_initialization() {
    use many_ai_cli::process::{ExitOutcome, SpawnOptions, run_capped_with_options};
    use windows_sys::Win32::System::Diagnostics::Debug::GetErrorMode;
    let parent_mode_before = unsafe { GetErrorMode() };
    let root = tempfile::tempdir().unwrap();
    let receipt_path = root.path().join("diagnostic.json");
    let mut driver = plan(root.path());
    driver.executable = std::env::current_exe().unwrap();
    driver.args = vec![
        "--exact".into(),
        "native_owner_driver".into(),
        "--nocapture".into(),
    ];
    driver.timeout = Duration::from_secs(28);
    driver.output_cap = 8192;
    driver.env.insert(
        "MANY_AI_SYNTHETIC_OWNER_DRIVER_RECEIPT".into(),
        Some(receipt_path.clone().into_os_string()),
    );
    driver.env.insert(
        "MANY_AI_SYNTHETIC_OWNER_DIAGNOSTIC".into(),
        Some("1".into()),
    );
    let output = run_capped_with_options(
        &driver,
        &Cancellation::default(),
        SpawnOptions {
            no_window: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let parent_mode_after = unsafe { GetErrorMode() };
    assert_eq!(
        parent_mode_before, parent_mode_after,
        "driver must not alter shared test parent error mode"
    );
    assert_eq!(
        output.outcome,
        ExitOutcome::Exited {
            code: Some(0),
            signal: None
        },
        "bounded isolated diagnostic failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut receipt: serde_json::Value =
        serde_json::from_slice(&std::fs::read(receipt_path).unwrap()).unwrap();
    receipt["parent_error_mode_before"] = serde_json::json!(parent_mode_before);
    receipt["parent_error_mode_after"] = serde_json::json!(parent_mode_after);
    let trials = receipt["trials"].as_array().unwrap();
    assert_eq!(trials.len(), 3);
    assert_eq!(receipt["driver_error_mode"].as_u64().unwrap() & 3, 3);
    assert_eq!(trials[0]["helper_ready"], true);
    assert_eq!(trials[0]["helper_error_mode"].as_u64().unwrap() & 3, 3);
    for trial in trials {
        assert_eq!(trial["status"], "returned", "{receipt}");
        assert_eq!(trial["pty_exit_code"], 1, "{receipt}");
        assert_eq!(trial["pty_forced"], true, "{receipt}");
        assert_eq!(trial["local_cessation"], true, "{receipt}");
        assert!(
            trial["helper_ready"].is_boolean(),
            "scheduler-dependent readiness must still be recorded"
        );
    }
    // Preserve the evidence beyond this fixture TempDir's lifetime.
    std::fs::write(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("owner-stop-regression.json"),
        receipt.to_string(),
    )
    .unwrap();
    println!("{}", receipt);
}

#[cfg(windows)]
#[tokio::test]
async fn lost_transport_persists_actual_native_cessation_without_forged_receipt() {
    use many_ai_cli::process::{ExitOutcome, SpawnOptions, run_capped_with_options};
    use windows_sys::Win32::System::Diagnostics::Debug::GetErrorMode;
    let parent_mode_before = unsafe { GetErrorMode() };
    let root = tempfile::tempdir().unwrap();
    let receipt_path = root.path().join("driver-receipt.json");
    let mut driver = plan(root.path());
    driver.executable = std::env::current_exe().unwrap();
    driver.args = vec![
        "--exact".into(),
        "native_owner_driver".into(),
        "--nocapture".into(),
    ];
    driver.timeout = Duration::from_secs(12);
    driver.output_cap = 8192;
    driver.env.insert(
        "MANY_AI_SYNTHETIC_OWNER_DRIVER_RECEIPT".into(),
        Some(receipt_path.clone().into_os_string()),
    );
    // The exact owned Job also contains the driver's ConPTY child. Only this
    // fixture tree is stopped on timeout; no normal Hub/user process is touched.
    let output = run_capped_with_options(
        &driver,
        &Cancellation::default(),
        SpawnOptions {
            no_window: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let parent_mode_after = unsafe { GetErrorMode() };
    assert_eq!(
        parent_mode_before, parent_mode_after,
        "driver must not alter shared test parent error mode"
    );
    eprintln!(
        "native-owner parent_error_mode_before={parent_mode_before} parent_error_mode_after={parent_mode_after}"
    );
    assert_eq!(
        output.outcome,
        ExitOutcome::Exited {
            code: Some(0),
            signal: None
        },
        "isolated native driver failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let receipt: serde_json::Value = serde_json::from_slice(
        &std::fs::read(receipt_path)
            .expect("successful native driver must leave acceptance receipt"),
    )
    .unwrap();
    use windows_sys::Win32::System::Diagnostics::Debug::{
        SEM_FAILCRITICALERRORS, SEM_NOGPFAULTERRORBOX,
    };
    let required = (SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX) as u64;
    assert_eq!(
        receipt["driver_error_mode"].as_u64().unwrap() & required,
        required
    );
    assert_eq!(
        receipt["helper_error_mode"].as_u64().unwrap() & required,
        required
    );
    assert!(receipt["driver_pid"].as_u64().unwrap() > 0);
    assert!(receipt["helper_pid"].as_u64().unwrap() > 0);
    assert_ne!(receipt["driver_pid"], receipt["helper_pid"]);
    assert_eq!(receipt["native_cessation_observed"], true);
}

#[tokio::test]
async fn unsupported_owned_stop_never_claims_cessation_or_closes_the_fixture() {
    let f = Fixture::new("", false, false, false);
    assert_eq!(
        f.factory.pty.close_owned().unwrap_err().kind(),
        io::ErrorKind::Unsupported
    );
    assert!(!f.factory.pty.ended.is_cancelled());
    assert!(f.observer.seen.lock().unwrap().is_empty());
    assert_eq!(
        f.factory
            .pty
            .terminate_checked(Duration::from_millis(10))
            .await
            .unwrap_err()
            .kind(),
        io::ErrorKind::Unsupported
    );
}
