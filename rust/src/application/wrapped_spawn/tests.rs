use super::*;
use crate::{
    config::{Resource, RuntimePaths},
    proto::core::*,
};
use std::{
    path::Path,
    sync::atomic::{AtomicUsize, Ordering},
};

struct Policy {
    resolves: AtomicUsize,
    records: AtomicUsize,
    fail: bool,
}
impl SpawnLaunchPolicy for Policy {
    fn resolve(&self, spec: &WrappedSpawnSpec, base: &[String]) -> io::Result<ResolvedSpawnPolicy> {
        self.resolves.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(io::Error::other("synthetic policy failure"));
        }
        Ok(ResolvedSpawnPolicy {
            base_environment: base.to_vec(),
            route_environment: vec!["SYNTHETIC_PRECEDENCE=route".into()],
            subscription_environment: vec!["SYNTHETIC_PRECEDENCE=subscription".into()],
            effective_route: spec.route.clone(),
            ordinary_model_args: vec![],
            current_model: spec.model.clone(),
            resolved_model: spec.model.trim().to_owned(),
            effort_args: if spec.effort.is_empty() {
                vec![]
            } else {
                vec!["--effort".into(), spec.effort.clone()]
            },
        })
    }
    fn folder_trust(&self, _: &WrappedSpawnSpec, _: &[String]) -> io::Result<()> {
        Ok(())
    }
    fn registered_model(&self, _: &str, _: &str) -> io::Result<()> {
        self.records.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
fn spec(root: &Path, attempt: u64) -> WrappedSpawnSpec {
    WrappedSpawnSpec {
        spawn_attempt: Some(SpawnAttemptId(attempt)),
        registration_proof: Some(SpawnRegistrationProof::issue().unwrap()),
        registration_metadata: SpawnRegistrationMetadata::default(),
        provider: "codex".into(),
        cwd: root.into(),
        model: "synthetic-model".into(),
        model_selection: String::new(),
        risk_confirmed: true,
        label: "same-label".into(),
        permission_mode: String::new(),
        sandbox: String::new(),
        ask_for_approval: String::new(),
        route: String::new(),
        utf8_session: false,
        effort: String::new(),
        execution_mode: String::new(),
        permission_preset: String::new(),
        initial_prompt: String::new(),
        subscription_profile_id: String::new(),
        subscription_login: false,
        usage_probe: false,
        grants: InternalSpawnGrants::default(),
        cancellation: TaskCancellation::default(),
    }
}
struct TestRuntime {
    root: tempfile::TempDir,
    _installed: tempfile::TempDir,
    options: WrapperSpawnOptions,
    policy: Arc<Policy>,
    failures: Arc<AtomicUsize>,
    spawner: Arc<ProcessWrappedSpawner>,
}
fn runtime(capacity: usize, fail_policy: bool) -> TestRuntime {
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let executable = root.path().join(if cfg!(windows) {
        "missing-synthetic.exe"
    } else {
        "synthetic-wrapper"
    });
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(
            &executable,
            include_bytes!("../../../tests/fixtures/application/wrapped_spawn/owned-wrapper.sh"),
        )
        .unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let options = WrapperSpawnOptions {
        paths: RuntimePaths::trial(root.path(), 49171, installed.path()).unwrap(),
        executable,
        application_home: installed.path().to_path_buf(),
        environment: vec![
            format!("SYNTHETIC_WRAPPER_ROOT={}", root.path().display()),
            "PATH=/usr/bin:/bin".into(),
            "SYNTHETIC_PRECEDENCE=base".into(),
            "MANY_AI_CLI_USAGE_PROBE=1".into(),
            "MANY_AI_CLI_SUBSCRIPTION_LOGIN=1".into(),
            "MANY_AI_CLI_INTERNAL_SPAWN_PROOF=stale-synthetic".into(),
            "MANY_AI_CLI_INTERNAL_STARTUP_JOB=stale-synthetic".into(),
        ],
        hub_port: 49171,
        parent_shell: "synthetic-shell".into(),
        pending_capacity: capacity,
        reap_timeout: Duration::from_secs(2),
    };
    let policy = Arc::new(Policy {
        resolves: AtomicUsize::new(0),
        records: AtomicUsize::new(0),
        fail: fail_policy,
    });
    let failures = Arc::new(AtomicUsize::new(0));
    let failed = failures.clone();
    let spawner = Arc::new(
        ProcessWrappedSpawner::new(
            options.clone(),
            policy.clone(),
            Arc::new(move |_, _| {
                failed.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
            Arc::new(|_| {}),
        )
        .unwrap(),
    );
    TestRuntime {
        root,
        _installed: installed,
        options,
        policy,
        failures,
        spawner,
    }
}
#[cfg(windows)]
fn windows_native_runtime() -> TestRuntime {
    let mut test = runtime(8, false);
    let source = std::path::PathBuf::from(
        std::env::var_os("MANY_TEST_NATIVE_WRAPPER_EXE")
            .or_else(|| option_env!("MANY_AI_NATIVE_TEST_WRAPPER").map(Into::into))
            .expect("native Windows build must prepare the synthetic wrapper test fixture"),
    );
    assert!(source.is_absolute() && source.is_file());
    let executable = test.root.path().join("synthetic-native-wrapper.exe");
    std::fs::copy(source, &executable).unwrap();
    test.options.executable = executable;
    let failed = test.failures.clone();
    test.spawner = Arc::new(
        ProcessWrappedSpawner::new(
            test.options.clone(),
            test.policy.clone(),
            Arc::new(move |_, _| {
                failed.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
            Arc::new(|_| {}),
        )
        .unwrap(),
    );
    test
}
#[cfg(unix)]
fn binding() -> SessionBinding {
    SessionBinding {
        session: LiveSessionId(31),
        incarnation: SessionIncarnation(2),
        wrapper: WrapperConnectionId(3),
    }
}
#[cfg(unix)]
fn registration(attempt: u64, pid: u32) -> SpawnRegistrationReceipt {
    SpawnRegistrationReceipt::proof_bound(SpawnAttemptId(attempt), binding(), pid)
}
#[cfg(unix)]
async fn pid(root: &Path) -> u32 {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Ok(text) = std::fs::read_to_string(root.join("pid"))
                && let Ok(pid) = text.trim().parse()
            {
                return pid;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}
async fn spawn(
    test: &TestRuntime,
    attempt: u64,
    wait: Duration,
) -> tokio::task::JoinHandle<SpawnWaitOutcome> {
    let spawner = test.spawner.clone();
    let spec = spec(test.root.path(), attempt);
    tokio::spawn(async move {
        spawner
            .spawn_and_wait(spec, wait, &HttpWaitCancellation::default())
            .await
    })
}
#[test]
fn argv_environment_prompt_and_trial_root_follow_go_launch_order() {
    let t = runtime(8, false);
    let mut request = spec(t.root.path(), 1);
    request.sandbox = "workspace-write".into();
    request.ask_for_approval = "on-request".into();
    request.effort = "high".into();
    request.execution_mode = "headless".into();
    request.route = "ollama".into();
    request.utf8_session = true;
    request.initial_prompt = "synthetic secret instruction\nwith two lines".into();
    request.grants = InternalSpawnGrants::from_config(vec![
        " Read ".into(),
        "bad;tool".into(),
        "Bash(cargo test:*)".into(),
    ]);
    let prepared = launch::prepare(&t.options, t.policy.as_ref(), &request, &|_| {}).unwrap();
    let args = prepared
        .process
        .args
        .iter()
        .map(|s| s.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let prompt = prepared.prompt.as_ref().unwrap();
    let expected = vec![
        "--trial-root".into(),
        t.options.paths.root().to_string_lossy().into_owned(),
        "--trial-port".into(),
        "49171".into(),
        "wrap".into(),
        "codex".into(),
        "--label=same-label".into(),
        "--model".into(),
        "synthetic-model".into(),
        "--sandbox".into(),
        "workspace-write".into(),
        "--ask-for-approval".into(),
        "on-request".into(),
        "--effort".into(),
        "high".into(),
        "--allowed-tools".into(),
        "Read,Bash(cargo test:*)".into(),
        "--headless".into(),
        "--prompt-file".into(),
        prompt.path().to_string_lossy().into_owned(),
        "--codex-oss".into(),
        "--utf8".into(),
    ];
    assert_eq!(args, expected);
    assert!(!args.iter().any(|v| v.contains("synthetic secret")));
    assert_eq!(
        std::fs::read_to_string(prompt.path()).unwrap(),
        request.initial_prompt
    );
    assert_eq!(
        prepared
            .process
            .env
            .get(std::ffi::OsStr::new("SYNTHETIC_PRECEDENCE"))
            .unwrap()
            .as_deref(),
        Some(std::ffi::OsStr::new("subscription"))
    );
    assert_eq!(
        prepared
            .process
            .env
            .get(std::ffi::OsStr::new("MANY_AI_CLI_USAGE_PROBE"))
            .unwrap()
            .as_deref(),
        Some(std::ffi::OsStr::new("0"))
    );
    assert_eq!(
        prepared
            .process
            .env
            .get(std::ffi::OsStr::new("MANY_AI_CLI_INTERNAL_STARTUP_JOB")),
        Some(&None)
    );
    assert!(
        prepared
            .process
            .env
            .get(std::ffi::OsStr::new(SPAWN_PROOF_ENV))
            .and_then(|value| value.as_deref())
            == Some(std::ffi::OsStr::new(
                request
                    .registration_proof
                    .as_ref()
                    .unwrap()
                    .as_header_value()
            ))
    );
    let path = prompt.path();
    drop(prepared);
    assert!(!path.exists());
    assert_eq!(t.policy.resolves.load(Ordering::SeqCst), 1);
}
#[test]
fn login_skips_provider_flags_prompt_and_risk() {
    let t = runtime(8, false);
    let mut request = spec(t.root.path(), 1);
    request.subscription_login = true;
    request.risk_confirmed = false;
    request.permission_mode = "bypassPermissions".into();
    request.execution_mode = "headless".into();
    request.initial_prompt = "unused".into();
    request.effort = "high".into();
    let prepared = launch::prepare(&t.options, t.policy.as_ref(), &request, &|_| {}).unwrap();
    assert_eq!(
        &prepared.process.args[4..],
        &[
            "wrap",
            "codex",
            "--label=same-label",
            "--subscription-login"
        ]
    );
    assert!(prepared.prompt.is_none());
    assert!(prepared.record_model.is_none());
}
#[tokio::test]
async fn policy_failure_does_not_spawn_and_releases_once() {
    let t = runtime(8, true);
    assert!(matches!(
        spawn(&t, 1, Duration::from_secs(1)).await.await.unwrap(),
        SpawnWaitOutcome::Failed(_)
    ));
    assert_eq!(t.failures.load(Ordering::SeqCst), 1);
    assert_eq!(t.spawner.pending_count(), 0);
    assert!(!t.root.path().join("pid").exists());
    t.spawner.close();
    assert_eq!(t.spawner.drain().await.unreaped_startups, 0);
}
#[cfg(unix)]
#[tokio::test]
async fn matching_registration_detaches_before_ack_and_survives_shutdown() {
    let t = runtime(8, false);
    let waiter = spawn(&t, 1, Duration::from_secs(3)).await;
    let pid = pid(t.root.path()).await;
    let ack = t
        .spawner
        .prepare_registration(registration(1, pid))
        .await
        .unwrap();
    assert_eq!(ack.binding(), binding());
    assert_eq!(t.spawner.pending_count(), 0);
    t.spawner.close();
    assert_eq!(t.spawner.drain().await.unreaped_startups, 0);
    assert!(crate::process::pid_alive(i64::from(pid)));
    let reap = ack.acknowledged();
    assert_eq!(
        waiter.await.unwrap(),
        SpawnWaitOutcome::Registered(binding())
    );
    std::fs::write(t.root.path().join("stop"), b"stop").unwrap();
    assert_eq!(reap.wait().await.unwrap().code, Some(7));
    assert_eq!(t.failures.load(Ordering::SeqCst), 0);
    assert_eq!(t.policy.records.load(Ordering::SeqCst), 1);
    let logs = std::fs::read_dir(t.options.paths.resource(Resource::Logs).join("spawn")).unwrap();
    let log = logs.into_iter().next().unwrap().unwrap().path();
    let text = std::fs::read_to_string(&log).unwrap();
    assert!(text.contains("proof-present"));
    assert!(text.contains("stdout-append"));
    assert!(text.contains("stderr-append"));
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(log).unwrap().permissions().mode() & 0o777,
        0o600
    );
}
#[cfg(unix)]
#[tokio::test]
async fn mismatched_pid_and_attempt_cannot_steal_owner() {
    let t = runtime(8, false);
    let waiter = spawn(&t, 1, Duration::from_secs(3)).await;
    let pid = pid(t.root.path()).await;
    assert!(
        t.spawner
            .prepare_registration(registration(2, pid))
            .await
            .is_err()
    );
    assert!(
        t.spawner
            .prepare_registration(registration(1, pid + 1))
            .await
            .is_err()
    );
    assert_eq!(t.spawner.pending_count(), 1);
    assert!(crate::process::pid_alive(i64::from(pid)));
    let reap = t
        .spawner
        .prepare_registration(registration(1, pid))
        .await
        .unwrap()
        .acknowledged();
    assert_eq!(
        waiter.await.unwrap(),
        SpawnWaitOutcome::Registered(binding())
    );
    std::fs::write(t.root.path().join("stop"), b"stop").unwrap();
    reap.wait().await.unwrap();
}
#[cfg(unix)]
#[tokio::test]
async fn uncertain_ack_and_dropped_guard_never_kill_accepted_wrapper() {
    for explicit in [false, true] {
        let t = runtime(8, false);
        let waiter = spawn(&t, 1, Duration::from_secs(3)).await;
        let pid = pid(t.root.path()).await;
        let ack = t
            .spawner
            .prepare_registration(registration(1, pid))
            .await
            .unwrap();
        let reap = if explicit {
            Some(ack.delivery_uncertain())
        } else {
            drop(ack);
            None
        };
        assert_eq!(
            waiter.await.unwrap(),
            SpawnWaitOutcome::Registered(binding())
        );
        t.spawner.close();
        t.spawner.drain().await;
        assert!(crate::process::pid_alive(i64::from(pid)));
        std::fs::write(t.root.path().join("stop"), b"stop").unwrap();
        if let Some(reap) = reap {
            reap.wait().await.unwrap();
        } else {
            tokio::time::timeout(Duration::from_secs(3), async {
                while crate::process::pid_alive(i64::from(pid)) {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
        }
        assert_eq!(t.failures.load(Ordering::SeqCst), 0);
    }
}
#[cfg(unix)]
#[tokio::test]
async fn known_pre_ack_rejection_terminates_exact_startup() {
    let t = runtime(8, false);
    let waiter = spawn(&t, 1, Duration::from_secs(3)).await;
    let pid = pid(t.root.path()).await;
    let reap = t
        .spawner
        .prepare_registration(registration(1, pid))
        .await
        .unwrap()
        .reject_before_ack();
    assert!(matches!(waiter.await.unwrap(), SpawnWaitOutcome::Failed(_)));
    assert!(reap.wait().await.unwrap().signal.is_some());
    assert_eq!(t.failures.load(Ordering::SeqCst), 0);
}
#[cfg(unix)]
#[tokio::test]
async fn http_cancellation_does_not_cancel_startup_registration() {
    let t = runtime(8, false);
    let spec = spec(t.root.path(), 1);
    let cancellation = HttpWaitCancellation::default();
    cancellation.cancel();
    assert_eq!(
        t.spawner
            .spawn_and_wait(spec, Duration::from_secs(3), &cancellation)
            .await,
        SpawnWaitOutcome::WaiterCancelled
    );
    let pid = pid(t.root.path()).await;
    let reap = t
        .spawner
        .prepare_registration(registration(1, pid))
        .await
        .unwrap()
        .acknowledged();
    t.spawner.close();
    t.spawner.drain().await;
    assert!(crate::process::pid_alive(i64::from(pid)));
    std::fs::write(t.root.path().join("stop"), b"stop").unwrap();
    reap.wait().await.unwrap();
    assert_eq!(t.failures.load(Ordering::SeqCst), 0);
}
#[cfg(unix)]
#[tokio::test]
async fn timeout_shutdown_and_task_cancel_abort_only_still_owned_startups() {
    for cause in [0, 1, 2] {
        let t = runtime(8, false);
        let request = spec(t.root.path(), 1);
        let cancellation = request.cancellation.clone();
        let spawner = t.spawner.clone();
        let waiter = tokio::spawn(async move {
            spawner
                .spawn_and_wait(
                    request,
                    Duration::from_secs(1),
                    &HttpWaitCancellation::default(),
                )
                .await
        });
        let pid = pid(t.root.path()).await;
        if cause == 1 {
            t.spawner.close();
        } else if cause == 2 {
            cancellation.cancel();
        }
        let outcome = waiter.await.unwrap();
        assert!(matches!(
            outcome,
            SpawnWaitOutcome::TimedOut | SpawnWaitOutcome::HubStopped | SpawnWaitOutcome::Failed(_)
        ));
        t.spawner.close();
        assert_eq!(t.spawner.drain().await.unreaped_startups, 0);
        assert!(!crate::process::pid_alive(i64::from(pid)));
        assert_eq!(t.failures.load(Ordering::SeqCst), 1);
        assert_eq!(t.spawner.pending_count(), 0);
    }
}
#[cfg(unix)]
#[tokio::test]
async fn bounded_capacity_is_attempt_based_not_label_based() {
    let t = runtime(1, false);
    let first = spawn(&t, 1, Duration::from_secs(3)).await;
    pid(t.root.path()).await;
    assert!(matches!(
        spawn(&t, 2, Duration::from_secs(3)).await.await.unwrap(),
        SpawnWaitOutcome::Failed(_)
    ));
    assert!(matches!(
        spawn(&t, 1, Duration::from_secs(3)).await.await.unwrap(),
        SpawnWaitOutcome::Failed(_)
    ));
    assert_eq!(t.policy.resolves.load(Ordering::SeqCst), 1);
    t.spawner.close();
    first.await.unwrap();
    t.spawner.drain().await;
    assert_eq!(t.spawner.pending_count(), 0);
}

#[cfg(unix)]
#[tokio::test]
async fn fast_registration_waits_for_exact_installation_and_failure_wakes_waiter() {
    for fail_install in [false, true] {
        let t = runtime(8, false);
        let request = spec(t.root.path(), 1);
        let prepared = launch::prepare(&t.options, t.policy.as_ref(), &request, &|_| {}).unwrap();
        let owner = WrapperStartup::spawn(
            SpawnAttemptId(1),
            &prepared.process,
            prepared.stdout,
            prepared.stderr,
        )
        .unwrap();
        let actual_pid = owner.pid();
        let (changed, _) = watch::channel(0);
        let (outcome, _) = watch::channel(None);
        let entry = Arc::new(Entry {
            attempt: SpawnAttemptId(1),
            provider: request.provider,
            state: Mutex::new(EntryState {
                phase: Phase::Installing,
                reap: None,
                native_start: None,
            }),
            changed,
            outcome,
            start: watch::channel(None).0,
        });
        lock(&t.spawner.inner.registry)
            .entries
            .insert(SpawnAttemptId(1), entry.clone());
        let spawner = t.spawner.clone();
        let registration = tokio::spawn(async move {
            spawner
                .prepare_registration(registration(1, actual_pid))
                .await
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!registration.is_finished());
        if fail_install {
            let reap = owner.abort();
            t.spawner.inner.fail(
                &entry,
                SpawnWaitOutcome::Failed("synthetic installation failure".into()),
            );
            assert!(registration.await.unwrap().is_err());
            reap.wait().await.unwrap();
            assert_eq!(t.failures.load(Ordering::SeqCst), 1);
        } else {
            lock(&entry.state).phase = Phase::Starting(StartupBundle {
                owner,
                prompt: prepared.prompt,
                record_model: None,
            });
            entry.notify();
            let reap = registration.await.unwrap().unwrap().acknowledged();
            assert_eq!(t.spawner.pending_count(), 0);
            std::fs::write(t.root.path().join("stop"), b"stop").unwrap();
            reap.wait().await.unwrap();
        }
    }
}
#[tokio::test]
async fn missing_executable_cleans_prompt_and_never_reports_success() {
    let mut t = runtime(8, false);
    t.options.executable = t.root.path().join("missing-executable");
    let failed = t.failures.clone();
    let spawner = ProcessWrappedSpawner::new(
        t.options.clone(),
        t.policy.clone(),
        Arc::new(move |_, _| {
            failed.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }),
        Arc::new(|_| {}),
    )
    .unwrap();
    let mut request = spec(t.root.path(), 1);
    request.initial_prompt = "synthetic instruction".into();
    assert!(matches!(
        spawner
            .spawn_and_wait(
                request,
                Duration::from_secs(3),
                &HttpWaitCancellation::default()
            )
            .await,
        SpawnWaitOutcome::Failed(_)
    ));
    assert_eq!(spawner.pending_count(), 0);
    assert_eq!(t.failures.load(Ordering::SeqCst), 1);
    assert_eq!(
        std::fs::read_dir(t.options.paths.resource(Resource::Temporary))
            .unwrap()
            .count(),
        0
    );
    spawner.close();
    assert_eq!(spawner.drain().await.unreaped_startups, 0);
}
#[cfg(unix)]
#[tokio::test]
async fn early_exit_is_terminal_and_prompt_is_reclaimed() {
    let mut t = runtime(8, false);
    t.options.environment.push("SYNTHETIC_EXIT_EARLY=1".into());
    let failed = t.failures.clone();
    let spawner = ProcessWrappedSpawner::new(
        t.options.clone(),
        t.policy.clone(),
        Arc::new(move |_, _| {
            failed.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }),
        Arc::new(|_| {}),
    )
    .unwrap();
    let mut request = spec(t.root.path(), 1);
    request.initial_prompt = "synthetic instruction".into();
    assert!(matches!(
        spawner
            .spawn_and_wait(
                request,
                Duration::from_secs(3),
                &HttpWaitCancellation::default()
            )
            .await,
        SpawnWaitOutcome::Failed(_)
    ));
    spawner.close();
    assert_eq!(spawner.drain().await.unreaped_startups, 0);
    assert_eq!(t.failures.load(Ordering::SeqCst), 1);
    assert_eq!(
        std::fs::read_dir(t.options.paths.resource(Resource::Temporary))
            .unwrap()
            .count(),
        0
    );
}
#[test]
fn risk_gate_requires_confirmation_except_usage_probe() {
    let t = runtime(8, false);
    let mut request = spec(t.root.path(), 1);
    request.risk_confirmed = false;
    request.model_selection = "required".into();
    assert!(launch::prepare(&t.options, t.policy.as_ref(), &request, &|_| {}).is_err());
    request.usage_probe = true;
    assert!(launch::prepare(&t.options, t.policy.as_ref(), &request, &|_| {}).is_ok());
}

#[cfg(unix)]
#[tokio::test]
async fn dropping_http_future_preserves_owner_and_same_label_can_register_twice() {
    let t = runtime(1, false);
    let waiter = spawn(&t, 1, Duration::from_secs(3)).await;
    let first_pid = pid(t.root.path()).await;
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    let first = t
        .spawner
        .prepare_registration(registration(1, first_pid))
        .await
        .unwrap()
        .acknowledged();
    assert_eq!(t.spawner.pending_count(), 0);
    std::fs::remove_file(t.root.path().join("pid")).unwrap();
    let waiter = spawn(&t, 2, Duration::from_secs(3)).await;
    let second_pid = pid(t.root.path()).await;
    assert_ne!(first_pid, second_pid);
    let second = t
        .spawner
        .prepare_registration(registration(2, second_pid))
        .await
        .unwrap()
        .acknowledged();
    assert_eq!(
        waiter.await.unwrap(),
        SpawnWaitOutcome::Registered(binding())
    );
    t.spawner.close();
    assert_eq!(t.spawner.drain().await.unreaped_startups, 0);
    assert!(crate::process::pid_alive(i64::from(first_pid)));
    assert!(crate::process::pid_alive(i64::from(second_pid)));
    std::fs::write(t.root.path().join("stop"), b"stop").unwrap();
    first.wait().await.unwrap();
    second.wait().await.unwrap();
    assert_eq!(t.policy.resolves.load(Ordering::SeqCst), 2);
    assert_eq!(t.failures.load(Ordering::SeqCst), 0);
}
#[cfg(unix)]
#[tokio::test]
async fn http_timeout_after_transfer_does_not_reclaim_provisional_owner() {
    let t = runtime(8, false);
    let waiter = spawn(&t, 1, Duration::from_secs(1)).await;
    let actual_pid = pid(t.root.path()).await;
    let ack = t
        .spawner
        .prepare_registration(registration(1, actual_pid))
        .await
        .unwrap();
    assert_eq!(waiter.await.unwrap(), SpawnWaitOutcome::TimedOut);
    t.spawner.close();
    t.spawner.drain().await;
    assert!(crate::process::pid_alive(i64::from(actual_pid)));
    let reap = ack.delivery_uncertain();
    std::fs::write(t.root.path().join("stop"), b"stop").unwrap();
    reap.wait().await.unwrap();
    assert_eq!(t.failures.load(Ordering::SeqCst), 0);
}

#[test]
fn inherited_path_empty_entries_are_removed_without_global_mutation() {
    let mut t = runtime(8, false);
    let (key, path, expected) = if cfg!(windows) {
        (
            "Path",
            ";C:\\synthetic-bin;; ;D:\\synthetic-node;",
            "C:\\synthetic-bin;D:\\synthetic-node",
        )
    } else {
        (
            "PATH",
            ":/synthetic/bin:: :/synthetic/node:",
            "/synthetic/bin:/synthetic/node",
        )
    };
    t.options.environment.retain(|entry| {
        !entry
            .split_once('=')
            .is_some_and(|(key, _)| key.eq_ignore_ascii_case("PATH"))
    });
    t.options.environment.push(format!("{key}={path}"));
    let request = spec(t.root.path(), 1);
    let prepared = launch::prepare(&t.options, t.policy.as_ref(), &request, &|_| {}).unwrap();
    assert_eq!(
        prepared
            .process
            .env
            .get(std::ffi::OsStr::new(key))
            .unwrap()
            .as_deref(),
        Some(std::ffi::OsStr::new(expected))
    );
    assert!(t.options.environment.contains(&format!("{key}={path}")));
}

struct PanickingPolicy {
    delegate: Arc<Policy>,
    on_resolve: bool,
    on_record: bool,
}
impl SpawnLaunchPolicy for PanickingPolicy {
    fn resolve(
        &self,
        request: &WrappedSpawnSpec,
        base: &[String],
    ) -> io::Result<ResolvedSpawnPolicy> {
        assert!(!self.on_resolve, "synthetic launch-policy panic");
        self.delegate.resolve(request, base)
    }
    fn folder_trust(&self, request: &WrappedSpawnSpec, environment: &[String]) -> io::Result<()> {
        self.delegate.folder_trust(request, environment)
    }
    fn registered_model(&self, provider: &str, model: &str) -> io::Result<()> {
        assert!(!self.on_record, "synthetic model-policy panic");
        self.delegate.registered_model(provider, model)
    }
}
#[tokio::test]
async fn launch_policy_panic_releases_pending_entry_and_wakes_observer() {
    let mut t = runtime(8, false);
    let failure_count = t.failures.clone();
    t.spawner = Arc::new(
        ProcessWrappedSpawner::new(
            t.options.clone(),
            Arc::new(PanickingPolicy {
                delegate: t.policy.clone(),
                on_resolve: true,
                on_record: false,
            }),
            Arc::new(move |_, _| {
                failure_count.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
            Arc::new(|_| {}),
        )
        .unwrap(),
    );
    let outcome = tokio::time::timeout(
        Duration::from_secs(3),
        t.spawner.spawn_and_wait(
            spec(t.root.path(), 1),
            Duration::from_secs(30),
            &HttpWaitCancellation::default(),
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        outcome,
        SpawnWaitOutcome::Failed("wrapper lifecycle was abandoned".into())
    );
    assert_eq!(t.spawner.pending_count(), 0);
    assert_eq!(t.failures.load(Ordering::SeqCst), 1);
    assert_eq!(t.spawner.drain().await.unreaped_startups, 0);
    assert!(!t.root.path().join("pid").exists());
}
#[test]
fn runtime_drop_before_first_poll_cleans_reservation_despite_panicking_callbacks() {
    let mut t = runtime(8, false);
    let failure_count = t.failures.clone();
    let warnings = Arc::new(AtomicUsize::new(0));
    let warning_count = warnings.clone();
    t.spawner = Arc::new(
        ProcessWrappedSpawner::new(
            t.options.clone(),
            t.policy.clone(),
            Arc::new(move |_, _| {
                failure_count.fetch_add(1, Ordering::SeqCst);
                panic!("synthetic failure callback panic");
            }),
            Arc::new(move |_| {
                warning_count.fetch_add(1, Ordering::SeqCst);
                panic!("synthetic warning callback panic");
            }),
        )
        .unwrap(),
    );
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let cancellation = HttpWaitCancellation::default();
    let mut observer = t.spawner.spawn_and_wait(
        spec(t.root.path(), 1),
        Duration::from_secs(30),
        &cancellation,
    );
    {
        // Merely enter the single-thread runtime and poll the observer once.
        // The queued lifecycle is guaranteed not to receive its first poll.
        let _entered = executor.enter();
        let waker = futures_util::task::noop_waker();
        let mut context = std::task::Context::from_waker(&waker);
        assert!(observer.as_mut().poll(&mut context).is_pending());
    }
    let entry = lock(&t.spawner.inner.registry)
        .entries
        .get(&SpawnAttemptId(1))
        .cloned()
        .unwrap();
    assert_eq!(t.spawner.pending_count(), 1);
    assert_eq!(t.policy.resolves.load(Ordering::SeqCst), 0);
    drop(observer);
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(executor))).is_ok());
    assert_eq!(t.spawner.pending_count(), 0);
    assert_eq!(*t.spawner.inner.active.borrow(), 0);
    assert_eq!(
        entry.outcome.borrow().clone(),
        Some(SpawnWaitOutcome::Failed(
            "wrapper lifecycle was abandoned".into()
        ))
    );
    assert_eq!(t.failures.load(Ordering::SeqCst), 1);
    assert_eq!(warnings.load(Ordering::SeqCst), 1);
    assert_eq!(*lock(&t.spawner.inner.cleanup_failures), 0);
    assert!(!t.root.path().join("pid").exists());
}
#[cfg(unix)]
#[test]
fn runtime_drop_during_owned_startup_aborts_before_releasing_active_count() {
    let t = runtime(8, false);
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let actual_pid = executor.block_on(async {
        let _observer = spawn(&t, 1, Duration::from_secs(30)).await;
        pid(t.root.path()).await
    });
    let entry = lock(&t.spawner.inner.registry)
        .entries
        .get(&SpawnAttemptId(1))
        .cloned()
        .unwrap();
    let reap = match &lock(&entry.state).phase {
        Phase::Starting(bundle) => bundle.owner.receipt(),
        _ => panic!("synthetic wrapper did not enter startup ownership"),
    };
    drop(executor);
    assert_eq!(t.spawner.pending_count(), 0);
    assert_eq!(*t.spawner.inner.active.borrow(), 0);
    assert_eq!(t.failures.load(Ordering::SeqCst), 1);
    assert!(matches!(
        entry.outcome.borrow().as_ref(),
        Some(SpawnWaitOutcome::Failed(_))
    ));
    let observer = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    observer.block_on(async {
        let exit = tokio::time::timeout(Duration::from_secs(3), reap.wait())
            .await
            .unwrap()
            .unwrap();
        assert!(exit.signal.is_some());
        assert!(!crate::process::pid_alive(i64::from(actual_pid)));
        // The abandoned lifecycle did not observe the passive reap itself.
        assert_eq!(t.spawner.drain().await.unreaped_startups, 1);
    });
}
#[cfg(unix)]
#[tokio::test]
async fn registration_drop_during_unwind_publishes_acceptance_before_callback_panics() {
    let mut t = runtime(8, false);
    let failures = t.failures.clone();
    let warnings = Arc::new(AtomicUsize::new(0));
    let warning_count = warnings.clone();
    t.spawner = Arc::new(
        ProcessWrappedSpawner::new(
            t.options.clone(),
            Arc::new(PanickingPolicy {
                delegate: t.policy.clone(),
                on_resolve: false,
                on_record: true,
            }),
            Arc::new(move |_, _| {
                failures.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
            Arc::new(move |_| {
                warning_count.fetch_add(1, Ordering::SeqCst);
                panic!("synthetic acceptance warning panic");
            }),
        )
        .unwrap(),
    );
    let waiter = spawn(&t, 1, Duration::from_secs(3)).await;
    let actual_pid = pid(t.root.path()).await;
    let reap = {
        let entry = lock(&t.spawner.inner.registry)
            .entries
            .get(&SpawnAttemptId(1))
            .cloned()
            .unwrap();
        let state = lock(&entry.state);
        match &state.phase {
            Phase::Starting(bundle) => bundle.owner.receipt(),
            _ => panic!("startup missing"),
        }
    };
    let ack = t
        .spawner
        .prepare_registration(registration(1, actual_pid))
        .await
        .unwrap();
    let outer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ack = ack;
        panic!("synthetic transport unwind");
    }));
    assert!(outer.is_err());
    assert_eq!(
        waiter.await.unwrap(),
        SpawnWaitOutcome::Registered(binding())
    );
    assert_eq!(warnings.load(Ordering::SeqCst), 1);
    t.spawner.close();
    assert_eq!(t.spawner.drain().await.unreaped_startups, 0);
    assert!(crate::process::pid_alive(i64::from(actual_pid)));
    std::fs::write(t.root.path().join("stop"), b"stop").unwrap();
    reap.wait().await.unwrap();
    assert_eq!(t.failures.load(Ordering::SeqCst), 0);
}

#[test]
fn resolved_model_suppression_and_registry_effort_argv_are_not_reinterpreted() {
    struct RegistryPolicy<'a>(&'a Policy);
    impl SpawnLaunchPolicy for RegistryPolicy<'_> {
        fn resolve(
            &self,
            spec: &WrappedSpawnSpec,
            env: &[String],
        ) -> io::Result<ResolvedSpawnPolicy> {
            let mut resolved = self.0.resolve(spec, env)?;
            resolved.resolved_model.clear();
            resolved.effort_args = vec!["-c".into(), "model_reasoning_effort=high".into()];
            Ok(resolved)
        }
        fn folder_trust(&self, spec: &WrappedSpawnSpec, env: &[String]) -> io::Result<()> {
            self.0.folder_trust(spec, env)
        }
        fn registered_model(&self, provider: &str, model: &str) -> io::Result<()> {
            self.0.registered_model(provider, model)
        }
    }
    let t = runtime(8, false);
    let mut request = spec(t.root.path(), 1);
    request.provider = "synthetic-custom".into();
    request.effort = "high".into();
    let prepared = launch::prepare(
        &t.options,
        &RegistryPolicy(t.policy.as_ref()),
        &request,
        &|_| {},
    )
    .unwrap();
    let args: Vec<_> = prepared
        .process
        .args
        .iter()
        .map(|v| v.to_string_lossy().into_owned())
        .collect();
    assert!(!args.iter().any(|v| v == "--model" || v == "--effort"));
    assert!(
        args.windows(2)
            .any(|pair| pair == ["-c", "model_reasoning_effort=high"])
    );
    assert!(prepared.record_model.is_none());
}

#[test]
fn invalid_model_and_label_still_observe_the_once_only_profile_policy_boundary() {
    for model in [true, false] {
        let t = runtime(8, false);
        let mut request = spec(t.root.path(), 1);
        if model {
            request.model = "invalid model".into();
        } else {
            request.label = "invalid label".into();
        }
        assert!(launch::prepare(&t.options, t.policy.as_ref(), &request, &|_| {}).is_err());
        assert_eq!(t.policy.resolves.load(Ordering::SeqCst), 1);
        assert!(!t.root.path().join("pid").exists());
    }
}

#[path = "start_tests.rs"]
mod start_tests;
