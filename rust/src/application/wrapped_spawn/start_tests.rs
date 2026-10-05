//! Ordinary Start observation shares the registration-wait owner and synthetic
//! wrapper. These tests never execute a provider or consult an account/home.
use super::*;

#[cfg(windows)]
async fn windows_native_pid(root: &Path) -> u32 {
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
    .expect("synthetic Windows wrapper did not resume")
}

#[cfg(windows)]
fn windows_registration(pid: u32) -> SpawnRegistrationReceipt {
    SpawnRegistrationReceipt::proof_bound(
        SpawnAttemptId(1),
        SessionBinding {
            session: LiveSessionId(31),
            incarnation: SessionIncarnation(2),
            wrapper: WrapperConnectionId(3),
        },
        pid,
    )
}

#[cfg(windows)]
#[tokio::test]
async fn windows_native_start_resumes_job_registers_and_detaches_before_ack() {
    let t = windows_native_runtime();
    let mut req = ordinary(&t, 1);
    req.spec.initial_prompt = "synthetic native launch prompt".into();
    let outcome = t
        .spawner
        .start_wrapped(req, &HttpWaitCancellation::default())
        .await;
    let SpawnStartOutcome::Started { pid } = outcome else {
        panic!("Windows native process creation failed: {outcome:?}");
    };
    assert_eq!(pid, windows_native_pid(t.root.path()).await);
    assert_eq!(
        std::fs::read(t.root.path().join("job-assigned")).unwrap(),
        b"assigned-before-resume"
    );
    let entry = pending(&t, 1);
    assert!(entry.outcome.borrow().is_none());
    assert_eq!(t.policy.resolves.load(Ordering::SeqCst), 0);
    assert_eq!(t.policy.records.load(Ordering::SeqCst), 1);
    let prompt_path = match &lock(&entry.state).phase {
        Phase::Starting(bundle) => bundle.prompt.as_ref().unwrap().path(),
        _ => panic!("native startup ownership missing"),
    };
    assert_eq!(
        std::fs::read_to_string(&prompt_path).unwrap(),
        "synthetic native launch prompt"
    );
    let ack = t
        .spawner
        .prepare_registration(windows_registration(pid))
        .await
        .unwrap();
    let reap = ack.acknowledged();
    assert_eq!(t.spawner.pending_count(), 0);
    assert!(matches!(
        *entry.outcome.borrow(),
        Some(SpawnWaitOutcome::Registered(_))
    ));
    t.spawner.close();
    assert_eq!(t.spawner.drain().await.unreaped_startups, 0);
    assert!(crate::process::pid_alive(i64::from(pid)));
    assert!(prompt_path.exists());
    std::fs::write(t.root.path().join("stop"), b"stop").unwrap();
    reap.wait().await.unwrap();
    assert_eq!(t.failures.load(Ordering::SeqCst), 0);
    assert_eq!(t.policy.records.load(Ordering::SeqCst), 1);
}

#[cfg(windows)]
#[tokio::test]
async fn windows_cancelled_start_observer_keeps_owned_process_for_registration() {
    let t = windows_native_runtime();
    let waiter = HttpWaitCancellation::default();
    waiter.cancel();
    assert_eq!(
        t.spawner.start_wrapped(ordinary(&t, 1), &waiter).await,
        SpawnStartOutcome::WaiterCancelled
    );
    let pid = windows_native_pid(t.root.path()).await;
    assert_eq!(t.spawner.pending_count(), 1);
    assert!(crate::process::pid_alive(i64::from(pid)));
    let reap = t
        .spawner
        .prepare_registration(windows_registration(pid))
        .await
        .unwrap()
        .acknowledged();
    t.spawner.close();
    assert_eq!(t.spawner.drain().await.unreaped_startups, 0);
    assert!(crate::process::pid_alive(i64::from(pid)));
    std::fs::write(t.root.path().join("stop"), b"stop").unwrap();
    reap.wait().await.unwrap();
    assert_eq!(t.failures.load(Ordering::SeqCst), 0);
}

#[cfg(windows)]
#[tokio::test]
async fn windows_unregistered_start_task_cancel_reaps_job_without_overwriting_start() {
    let t = windows_native_runtime();
    let req = ordinary(&t, 1);
    let cancel = req.spec.cancellation.clone();
    let outcome = t
        .spawner
        .start_wrapped(req, &HttpWaitCancellation::default())
        .await;
    let SpawnStartOutcome::Started { pid } = outcome else {
        panic!("Windows native process creation failed: {outcome:?}");
    };
    assert_eq!(pid, windows_native_pid(t.root.path()).await);
    let entry = pending(&t, 1);
    cancel.cancel();
    assert_eq!(t.spawner.drain().await.unreaped_startups, 0);
    assert_eq!(t.spawner.pending_count(), 0);
    assert!(!crate::process::pid_alive(i64::from(pid)));
    assert_eq!(
        *entry.start.borrow(),
        Some(SpawnStartOutcome::Started { pid })
    );
    assert_eq!(t.failures.load(Ordering::SeqCst), 1);
    assert!(
        t.spawner
            .prepare_registration(windows_registration(pid))
            .await
            .is_err()
    );
}

fn request(t: &TestRuntime, attempt: u64, kind: WrappedStartKind) -> WrappedStartRequest {
    WrappedStartRequest {
        spec: spec(t.root.path(), attempt),
        policy: ResolvedSpawnPolicy {
            base_environment: t.options.environment.clone(),
            route_environment: vec!["SYNTHETIC_PRECEDENCE=prepared-route".into()],
            subscription_environment: vec!["SYNTHETIC_PRECEDENCE=prepared-profile".into()],
            effective_route: String::new(),
            current_model: "synthetic-model".into(),
            resolved_model: "prepared-model".into(),
            effort_args: vec!["-c".into(), "model_reasoning_effort=high".into()],
            ordinary_model_args: vec![],
        },
        kind,
    }
}
fn ordinary(t: &TestRuntime, attempt: u64) -> WrappedStartRequest {
    request(
        t,
        attempt,
        WrappedStartKind::OrdinaryAi { delegation: false },
    )
}

#[test]
fn ordinary_permission_risk_and_custom_model_arguments_match_provider_switch() {
    let t = runtime(8, false);
    for (provider, permission, risk) in [
        ("claude", true, true),
        ("codex", false, true),
        ("copilot", false, false),
        ("cursor-agent", false, false),
        ("opencode", true, true),
        ("grok", false, false),
        ("command-code", true, true),
        ("synthetic-custom", false, false),
    ] {
        let mut req = ordinary(&t, 1);
        req.spec.provider = provider.into();
        req.spec.permission_mode = "bypassPermissions".into();
        req.spec.sandbox = "danger-full-access".into();
        req.spec.ask_for_approval = "never".into();
        req.spec.risk_confirmed = false;
        req.policy.ordinary_model_args =
            vec!["--synthetic-model=日本語 a".into(), "literal;$()".into()];
        let unconfirmed = launch::prepare_start(
            &t.options,
            t.policy.as_ref(),
            &req.spec,
            &req.policy,
            req.kind,
            &|_| {},
        );
        assert_eq!(unconfirmed.is_err(), risk, "{provider}");
        req.spec.risk_confirmed = true;
        let prepared = launch::prepare_start(
            &t.options,
            t.policy.as_ref(),
            &req.spec,
            &req.policy,
            req.kind,
            &|_| {},
        )
        .unwrap();
        let argv = prepared.process.args;
        assert_eq!(
            argv.iter().any(|v| v == "--permission-mode"),
            permission,
            "{provider}"
        );
        assert_eq!(
            argv.iter().any(|v| v == "--sandbox"),
            provider == "codex",
            "{provider}"
        );
        assert_eq!(
            argv.iter().any(|v| v == "--ask-for-approval"),
            provider == "codex",
            "{provider}"
        );
        if provider == "synthetic-custom" {
            assert!(!argv.iter().any(|v| v == "--model"));
            assert!(
                argv.windows(2)
                    .any(|v| v == ["--synthetic-model=日本語 a", "literal;$()"])
            );
        } else {
            assert!(argv.windows(2).any(|v| v == ["--model", "prepared-model"]));
        }
    }
}
fn pending(t: &TestRuntime, attempt: u64) -> Arc<Entry> {
    lock(&t.spawner.inner.registry)
        .entries
        .get(&SpawnAttemptId(attempt))
        .cloned()
        .unwrap()
}
fn poll_once<T>(future: &mut CoreFuture<'_, T>) -> std::task::Poll<T> {
    let waker = futures_util::task::noop_waker();
    future
        .as_mut()
        .poll(&mut std::task::Context::from_waker(&waker))
}
fn trust_grant() -> InternalSpawnGrants {
    InternalSpawnGrants::from_config(vec!["Read".into()]).with_human_folder_trust(
        &SpawnConfirmationResponse {
            approved: true,
            grant_folder_trust: Some(true),
            ..Default::default()
        },
        &VerifiedConfirmationRequest::after_server_authentication(AuthEpoch(1)),
    )
}

struct Hooks {
    trust: Box<dyn Fn() -> io::Result<()> + Send + Sync>,
    record: Box<dyn Fn() -> io::Result<()> + Send + Sync>,
}
impl SpawnLaunchPolicy for Hooks {
    fn resolve(&self, _: &WrappedSpawnSpec, _: &[String]) -> io::Result<ResolvedSpawnPolicy> {
        panic!("ordinary prepared launch must not resolve a second time")
    }
    fn folder_trust(&self, _: &WrappedSpawnSpec, _: &[String]) -> io::Result<()> {
        (self.trust)()
    }
    fn registered_model(&self, _: &str, _: &str) -> io::Result<()> {
        (self.record)()
    }
}

#[test]
fn ordinary_prepared_policy_is_used_once_and_delegation_overrides_both_values() {
    let t = runtime(8, true);
    for delegation in [false, true] {
        let mut req = request(&t, 1, WrappedStartKind::OrdinaryAi { delegation });
        req.policy
            .base_environment
            .push(format!("MANY_AI_CLI_DELEGATION={}", u8::from(!delegation)));
        let prepared = launch::prepare_start(
            &t.options,
            t.policy.as_ref(),
            &req.spec,
            &req.policy,
            req.kind,
            &|_| {},
        )
        .unwrap();
        let args = prepared.process.args;
        assert!(args.windows(2).any(|v| v == ["--model", "prepared-model"]));
        assert!(
            args.windows(2)
                .any(|v| v == ["-c", "model_reasoning_effort=high"])
        );
        assert_eq!(prepared.record_model.as_deref(), Some("prepared-model"));
        assert_eq!(
            prepared.process.env[std::ffi::OsStr::new("MANY_AI_CLI_DELEGATION")],
            Some(u8::from(delegation).to_string().into())
        );
        assert_eq!(
            prepared.process.env[std::ffi::OsStr::new("SYNTHETIC_PRECEDENCE")],
            Some("prepared-profile".into())
        );
    }
    assert_eq!(t.policy.resolves.load(Ordering::SeqCst), 0);
}

#[test]
fn bare_keeps_only_shell_utf8_label_self_trial_proof_and_resolved_environment() {
    let t = runtime(8, true);
    let hooks = Hooks {
        trust: Box::new(|| panic!("bare must not write trust")),
        record: Box::new(|| panic!("bare must not save a model")),
    };
    for provider in ["shell", "codex"] {
        let mut req = request(&t, 1, WrappedStartKind::Bare);
        req.spec.provider = provider.into();
        req.spec.utf8_session = true;
        req.spec.grants = trust_grant();
        req.spec.subscription_login = true;
        req.spec.usage_probe = true;
        req.spec.risk_confirmed = false;
        req.spec.model_selection = "required".into();
        req.spec.permission_mode = "bypassPermissions".into();
        req.spec.sandbox = "danger-full-access".into();
        req.spec.ask_for_approval = "never".into();
        req.spec.execution_mode = "headless".into();
        req.spec.initial_prompt = "must not become a file".into();
        req.policy.base_environment.retain(|v| {
            !v.starts_with("MANY_AI_CLI_USAGE_PROBE=")
                && !v.starts_with("MANY_AI_CLI_SUBSCRIPTION_LOGIN=")
        });
        req.policy
            .base_environment
            .push("MANY_AI_CLI_DELEGATION=kept".into());
        let prepared = launch::prepare_start(
            &t.options,
            &hooks,
            &req.spec,
            &req.policy,
            req.kind,
            &|_| {},
        )
        .unwrap();
        let args = &prepared.process.args[4..];
        let mut expected = vec!["wrap", provider, "--label=same-label"];
        if provider == "shell" {
            expected.push("--utf8");
        }
        assert_eq!(args, expected);
        assert!(prepared.prompt.is_none());
        assert!(prepared.record_model.is_none());
        assert!(!t.options.paths.resource(Resource::Temporary).exists());
        assert_eq!(
            prepared.process.env[std::ffi::OsStr::new("MANY_AI_CLI_DELEGATION")],
            Some("kept".into())
        );
        for absent in ["MANY_AI_CLI_USAGE_PROBE", "MANY_AI_CLI_SUBSCRIPTION_LOGIN"] {
            assert_eq!(prepared.process.env[std::ffi::OsStr::new(absent)], None);
        }
        assert_eq!(
            prepared.process.env[std::ffi::OsStr::new("SYNTHETIC_PRECEDENCE")],
            Some("prepared-profile".into())
        );
        assert_eq!(
            prepared.process.env[std::ffi::OsStr::new("MANY_AI_CLI_BIN")],
            Some(t.options.executable.as_os_str().into())
        );
        assert_eq!(
            prepared.process.env[std::ffi::OsStr::new(SPAWN_PROOF_ENV)],
            Some(
                req.spec
                    .registration_proof
                    .as_ref()
                    .unwrap()
                    .as_header_value()
                    .into()
            )
        );
    }
}

#[test]
fn grid_optional_log_failure_still_prepares_raw_provider_start() {
    use std::io::Write;
    let t = runtime(8, true);
    let hooks = Hooks {
        trust: Box::new(|| panic!("grid does not write folder trust")),
        record: Box::new(|| panic!("grid does not save model preferences")),
    };
    let log_dir = t.options.paths.resource(Resource::Logs);
    std::fs::create_dir_all(log_dir.parent().unwrap()).unwrap();
    std::fs::write(&log_dir, b"synthetic path blocks directory creation").unwrap();
    let mut req = request(&t, 1, WrappedStartKind::Grid);
    req.spec.provider = "codex".into();
    req.spec.utf8_session = true;
    req.spec.grants = trust_grant();
    req.spec.permission_mode = "bypassPermissions".into();
    req.spec.initial_prompt = "must not become a grid prompt file".into();
    let mut prepared = launch::prepare_start(
        &t.options,
        &hooks,
        &req.spec,
        &req.policy,
        req.kind,
        &|_| {},
    )
    .unwrap();
    assert_eq!(
        prepared.process.args[4..],
        ["wrap", "codex", "--label=same-label"]
    );
    prepared
        .stdout
        .write_all(b"discarded synthetic stdout")
        .unwrap();
    prepared
        .stderr
        .write_all(b"discarded synthetic stderr")
        .unwrap();
    assert!(prepared.prompt.is_none());
    assert!(prepared.record_model.is_none());
    req.kind = WrappedStartKind::Bare;
    assert!(
        launch::prepare_start(
            &t.options,
            &hooks,
            &req.spec,
            &req.policy,
            req.kind,
            &|_| {},
        )
        .is_err()
    );
    assert_eq!(
        std::fs::read(&log_dir).unwrap(),
        b"synthetic path blocks directory creation"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn ordinary_start_precedes_registration_records_once_and_transfers_prompt() {
    let t = runtime(8, true);
    let mut req = ordinary(&t, 1);
    req.spec.initial_prompt = "one launch prompt".into();
    let outcome = t
        .spawner
        .start_wrapped(req, &HttpWaitCancellation::default())
        .await;
    let SpawnStartOutcome::Started { pid: actual_pid } = outcome else {
        panic!("native creation failed")
    };
    assert_eq!(actual_pid, pid(t.root.path()).await);
    let entry = pending(&t, 1);
    assert!(entry.outcome.borrow().is_none());
    assert_eq!(t.policy.resolves.load(Ordering::SeqCst), 0);
    assert_eq!(t.policy.records.load(Ordering::SeqCst), 1);
    let prompt_path = match &lock(&entry.state).phase {
        Phase::Starting(bundle) => {
            assert!(bundle.record_model.is_none());
            bundle.prompt.as_ref().unwrap().path()
        }
        _ => panic!("native Start did not retain startup ownership"),
    };
    assert_eq!(
        std::fs::read_to_string(&prompt_path).unwrap(),
        "one launch prompt"
    );
    let ack = t
        .spawner
        .prepare_registration(registration(1, actual_pid))
        .await
        .unwrap();
    assert!(entry.outcome.borrow().is_none());
    let reap = ack.acknowledged();
    assert_eq!(
        *entry.outcome.borrow(),
        Some(SpawnWaitOutcome::Registered(binding()))
    );
    assert_eq!(
        *entry.start.borrow(),
        Some(SpawnStartOutcome::Started { pid: actual_pid })
    );
    assert_eq!(t.policy.records.load(Ordering::SeqCst), 1);
    assert!(prompt_path.exists());
    t.spawner.close();
    assert_eq!(t.spawner.drain().await.unreaped_startups, 0);
    assert!(crate::process::pid_alive(i64::from(actual_pid)));
    std::fs::write(t.root.path().join("stop"), b"stop").unwrap();
    reap.wait().await.unwrap();
    assert_eq!(t.failures.load(Ordering::SeqCst), 0);
}

#[cfg(unix)]
#[tokio::test]
async fn fast_registration_retains_start_until_observer_polls_again() {
    let t = runtime(8, false);
    let waiter = HttpWaitCancellation::default();
    let mut observer = t.spawner.start_wrapped(ordinary(&t, 1), &waiter);
    assert!(poll_once(&mut observer).is_pending());
    let entry = pending(&t, 1);
    let actual_pid = pid(t.root.path()).await;
    let reap = t
        .spawner
        .prepare_registration(registration(1, actual_pid))
        .await
        .unwrap()
        .delivery_uncertain();
    assert_eq!(t.spawner.pending_count(), 0);
    // Both results survive even though the HTTP observer has not polled since
    // before native creation and the entry is already outside the registry.
    assert_eq!(
        observer.await,
        SpawnStartOutcome::Started { pid: actual_pid }
    );
    assert_eq!(
        *entry.outcome.borrow(),
        Some(SpawnWaitOutcome::Registered(binding()))
    );
    assert_eq!(t.policy.records.load(Ordering::SeqCst), 1);
    t.spawner.close();
    assert_eq!(t.spawner.drain().await.unreaped_startups, 0);
    std::fs::write(t.root.path().join("stop"), b"stop").unwrap();
    reap.wait().await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn cancelled_and_dropped_start_observers_leave_owned_startup_running() {
    for cancelled in [false, true] {
        let t = runtime(8, false);
        let waiter = HttpWaitCancellation::default();
        if cancelled {
            waiter.cancel();
            assert_eq!(
                t.spawner.start_wrapped(ordinary(&t, 1), &waiter).await,
                SpawnStartOutcome::WaiterCancelled
            );
        } else {
            let mut observer = t.spawner.start_wrapped(ordinary(&t, 1), &waiter);
            assert!(poll_once(&mut observer).is_pending());
            drop(observer);
        }
        let actual_pid = pid(t.root.path()).await;
        assert_eq!(t.spawner.pending_count(), 1);
        let reap = t
            .spawner
            .prepare_registration(registration(1, actual_pid))
            .await
            .unwrap()
            .acknowledged();
        t.spawner.close();
        assert_eq!(t.spawner.drain().await.unreaped_startups, 0);
        assert!(crate::process::pid_alive(i64::from(actual_pid)));
        std::fs::write(t.root.path().join("stop"), b"stop").unwrap();
        reap.wait().await.unwrap();
        assert_eq!(t.failures.load(Ordering::SeqCst), 0);
        assert_eq!(t.policy.records.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn native_failure_never_records_start_or_model_and_reclaims_prompt() {
    let mut t = runtime(8, false);
    t.options.executable = t.root.path().join("missing-native-start-executable");
    let failures = t.failures.clone();
    t.spawner = Arc::new(
        ProcessWrappedSpawner::new(
            t.options.clone(),
            t.policy.clone(),
            Arc::new(move |_, _| {
                failures.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
            Arc::new(|_| {}),
        )
        .unwrap(),
    );
    let mut req = ordinary(&t, 1);
    req.spec.initial_prompt = "synthetic prompt".into();
    let waiter = HttpWaitCancellation::default();
    let mut observer = t.spawner.start_wrapped(req, &waiter);
    assert!(poll_once(&mut observer).is_pending());
    let entry = pending(&t, 1);
    assert!(matches!(observer.await, SpawnStartOutcome::Failed(_)));
    assert!(lock(&entry.state).native_start.is_none());
    assert!(matches!(
        *entry.start.borrow(),
        Some(SpawnStartOutcome::Failed(_))
    ));
    assert_eq!(t.policy.records.load(Ordering::SeqCst), 0);
    assert_eq!(t.failures.load(Ordering::SeqCst), 1);
    assert_eq!(t.spawner.pending_count(), 0);
    assert_eq!(
        std::fs::read_dir(t.options.paths.resource(Resource::Temporary))
            .unwrap()
            .count(),
        0
    );
    t.spawner.close();
    assert_eq!(t.spawner.drain().await.unreaped_startups, 0);
}

#[cfg(unix)]
#[tokio::test]
async fn later_exit_task_cancel_and_hub_stop_cannot_overwrite_successful_start() {
    for cause in [0, 1, 2] {
        let t = runtime(8, false);
        let req = ordinary(&t, 1);
        let task = req.spec.cancellation.clone();
        let outcome = t
            .spawner
            .start_wrapped(req, &HttpWaitCancellation::default())
            .await;
        let SpawnStartOutcome::Started { pid: actual_pid } = outcome else {
            panic!("native creation failed")
        };
        let entry = pending(&t, 1);
        if cause == 0 {
            std::fs::write(t.root.path().join("stop"), b"stop").unwrap();
        } else if cause == 1 {
            task.cancel();
        } else {
            t.spawner.close();
        }
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(3), t.spawner.drain())
                .await
                .unwrap()
                .unreaped_startups,
            0
        );
        assert_eq!(t.spawner.pending_count(), 0);
        assert_eq!(t.failures.load(Ordering::SeqCst), 1);
        assert_eq!(
            *entry.start.borrow(),
            Some(SpawnStartOutcome::Started { pid: actual_pid })
        );
        assert!(!crate::process::pid_alive(i64::from(actual_pid)));
    }
}

#[cfg(unix)]
#[tokio::test]
async fn ordinary_ownership_has_no_registration_timeout_and_shares_capacity() {
    let t = runtime(1, false);
    let outcome = t
        .spawner
        .start_wrapped(ordinary(&t, 1), &HttpWaitCancellation::default())
        .await;
    let SpawnStartOutcome::Started { pid: actual_pid } = outcome else {
        panic!("native creation failed")
    };
    pid(t.root.path()).await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(120)).await;
    tokio::task::yield_now().await;
    assert_eq!(t.spawner.pending_count(), 1);
    assert!(pending(&t, 1).outcome.borrow().is_none());
    assert!(crate::process::pid_alive(i64::from(actual_pid)));
    assert_eq!(
        t.spawner
            .start_wrapped(ordinary(&t, 2), &HttpWaitCancellation::default())
            .await,
        SpawnStartOutcome::Failed("wrapper startup capacity reached".into())
    );
    assert_eq!(
        t.spawner
            .spawn_and_wait(
                spec(t.root.path(), 2),
                Duration::from_secs(1),
                &HttpWaitCancellation::default()
            )
            .await,
        SpawnWaitOutcome::Failed("wrapper startup capacity reached".into())
    );
    tokio::time::resume();
    t.spawner.close();
    assert_eq!(t.spawner.drain().await.unreaped_startups, 0);
}

#[cfg(unix)]
#[tokio::test]
async fn model_failure_or_panic_warns_once_without_undoing_start_or_recording_at_ack() {
    for panic in [false, true] {
        let mut t = runtime(8, false);
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let warnings = Arc::new(AtomicUsize::new(0));
        let warning_count = warnings.clone();
        t.spawner = Arc::new(
            ProcessWrappedSpawner::new(
                t.options.clone(),
                Arc::new(Hooks {
                    trust: Box::new(|| Ok(())),
                    record: Box::new(move || {
                        count.fetch_add(1, Ordering::SeqCst);
                        assert!(!panic, "synthetic Start model panic");
                        Err(io::Error::other("synthetic Start model failure"))
                    }),
                }),
                Arc::new(|_, _| Ok(())),
                Arc::new(move |_| {
                    warning_count.fetch_add(1, Ordering::SeqCst);
                    panic!("synthetic warning panic");
                }),
            )
            .unwrap(),
        );
        let outcome = t
            .spawner
            .start_wrapped(ordinary(&t, 1), &HttpWaitCancellation::default())
            .await;
        let SpawnStartOutcome::Started { pid: actual_pid } = outcome else {
            panic!("native creation failed")
        };
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(warnings.load(Ordering::SeqCst), 1);
        let ack = t
            .spawner
            .prepare_registration(registration(1, actual_pid))
            .await
            .unwrap();
        let reap = ack.acknowledged();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        t.spawner.close();
        assert_eq!(t.spawner.drain().await.unreaped_startups, 0);
        std::fs::write(t.root.path().join("stop"), b"stop").unwrap();
        reap.wait().await.unwrap();
    }
}

#[tokio::test]
async fn close_before_creation_wakes_start_observer_without_fake_success() {
    let t = runtime(8, false);
    let waiter = HttpWaitCancellation::default();
    let mut observer = t.spawner.start_wrapped(ordinary(&t, 1), &waiter);
    assert!(poll_once(&mut observer).is_pending());
    let entry = pending(&t, 1);
    t.spawner.close();
    assert_eq!(observer.await, SpawnStartOutcome::HubStopped);
    assert_eq!(t.spawner.drain().await.unreaped_startups, 0);
    assert!(lock(&entry.state).native_start.is_none());
    assert_eq!(t.policy.records.load(Ordering::SeqCst), 0);
    assert_eq!(t.failures.load(Ordering::SeqCst), 1);
    assert!(!t.root.path().join("pid").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn reentrant_close_and_model_panic_keep_real_start_and_reap_the_owner() {
    let mut t = runtime(8, false);
    let owner = Arc::new(Mutex::new(std::sync::Weak::<ProcessWrappedSpawner>::new()));
    let on_record = owner.clone();
    let records = Arc::new(AtomicUsize::new(0));
    let count = records.clone();
    let failures = t.failures.clone();
    t.spawner = Arc::new(
        ProcessWrappedSpawner::new(
            t.options.clone(),
            Arc::new(Hooks {
                trust: Box::new(|| Ok(())),
                record: Box::new(move || {
                    count.fetch_add(1, Ordering::SeqCst);
                    lock(&on_record).upgrade().unwrap().close();
                    panic!("synthetic close/record race");
                }),
            }),
            Arc::new(move |_, _| {
                failures.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
            Arc::new(|_| {}),
        )
        .unwrap(),
    );
    *lock(&owner) = Arc::downgrade(&t.spawner);
    let outcome = tokio::time::timeout(
        Duration::from_secs(3),
        t.spawner
            .start_wrapped(ordinary(&t, 1), &HttpWaitCancellation::default()),
    )
    .await
    .unwrap();
    let SpawnStartOutcome::Started { pid: actual_pid } = outcome else {
        panic!("real native creation was lost")
    };
    assert_eq!(t.spawner.drain().await.unreaped_startups, 0);
    assert_eq!(t.spawner.pending_count(), 0);
    assert!(!crate::process::pid_alive(i64::from(actual_pid)));
    assert_eq!(records.load(Ordering::SeqCst), 1);
    assert_eq!(t.failures.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn preparation_panic_wakes_start_observer_and_releases_once() {
    let mut t = runtime(8, false);
    let failures = t.failures.clone();
    t.spawner = Arc::new(
        ProcessWrappedSpawner::new(
            t.options.clone(),
            Arc::new(Hooks {
                trust: Box::new(|| panic!("synthetic prepared trust panic")),
                record: Box::new(|| panic!("must not reach model record")),
            }),
            Arc::new(move |_, _| {
                failures.fetch_add(1, Ordering::SeqCst);
                panic!("synthetic failure callback panic")
            }),
            Arc::new(|_| panic!("synthetic warning callback panic")),
        )
        .unwrap(),
    );
    let mut req = ordinary(&t, 1);
    req.spec.grants = trust_grant();
    let outcome = tokio::time::timeout(
        Duration::from_secs(3),
        t.spawner
            .start_wrapped(req, &HttpWaitCancellation::default()),
    )
    .await
    .unwrap();
    assert_eq!(
        outcome,
        SpawnStartOutcome::Failed("wrapper lifecycle was abandoned".into())
    );
    assert_eq!(t.spawner.pending_count(), 0);
    assert_eq!(t.spawner.drain().await.unreaped_startups, 0);
    assert_eq!(t.failures.load(Ordering::SeqCst), 1);
    assert!(!t.root.path().join("pid").exists());
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn registration_can_ack_while_start_model_callback_is_inflight() {
    let mut t = runtime(8, false);
    let (entered, waiting) = tokio::sync::oneshot::channel();
    let entered = Mutex::new(Some(entered));
    let gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
    let callback_gate = gate.clone();
    let records = Arc::new(AtomicUsize::new(0));
    let count = records.clone();
    t.spawner = Arc::new(
        ProcessWrappedSpawner::new(
            t.options.clone(),
            Arc::new(Hooks {
                trust: Box::new(|| Ok(())),
                record: Box::new(move || {
                    count.fetch_add(1, Ordering::SeqCst);
                    lock(&entered).take().unwrap().send(()).unwrap();
                    let (gate, changed) = callback_gate.as_ref();
                    let (_guard, result) = changed
                        .wait_timeout_while(lock(gate), Duration::from_secs(10), |open| !*open)
                        .unwrap();
                    assert!(!result.timed_out(), "synthetic callback gate timed out");
                    Ok(())
                }),
            }),
            Arc::new(|_, _| Ok(())),
            Arc::new(|_| {}),
        )
        .unwrap(),
    );
    // Always release the blocking callback if an assertion unwinds the test.
    struct OpenOnDrop(Arc<(Mutex<bool>, std::sync::Condvar)>);
    impl Drop for OpenOnDrop {
        fn drop(&mut self) {
            *lock(&self.0.0) = true;
            self.0.1.notify_all();
        }
    }
    let open = OpenOnDrop(gate);
    let waiter = HttpWaitCancellation::default();
    let mut observer = t.spawner.start_wrapped(ordinary(&t, 1), &waiter);
    assert!(poll_once(&mut observer).is_pending());
    waiting.await.unwrap();
    let entry = pending(&t, 1);
    let actual_pid = lock(&entry.state).native_start.unwrap();
    assert!(entry.start.borrow().is_none());
    let reap = t
        .spawner
        .prepare_registration(registration(1, actual_pid))
        .await
        .unwrap()
        .acknowledged();
    assert_eq!(
        *entry.outcome.borrow(),
        Some(SpawnWaitOutcome::Registered(binding()))
    );
    assert_eq!(records.load(Ordering::SeqCst), 1);
    assert!(entry.start.borrow().is_none());
    drop(open);
    assert_eq!(
        observer.await,
        SpawnStartOutcome::Started { pid: actual_pid }
    );
    assert_eq!(records.load(Ordering::SeqCst), 1);
    t.spawner.close();
    assert_eq!(t.spawner.drain().await.unreaped_startups, 0);
    std::fs::write(t.root.path().join("stop"), b"stop").unwrap();
    reap.wait().await.unwrap();
}

#[test]
fn abandoned_runtime_before_native_creation_wakes_start_observer_without_success() {
    let t = runtime(8, false);
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let waiter = HttpWaitCancellation::default();
    let mut observer = t.spawner.start_wrapped(ordinary(&t, 1), &waiter);
    {
        let _entered = executor.enter();
        assert!(poll_once(&mut observer).is_pending());
    }
    let entry = pending(&t, 1);
    drop(executor);
    assert_eq!(
        poll_once(&mut observer),
        std::task::Poll::Ready(SpawnStartOutcome::Failed(
            "wrapper lifecycle was abandoned".into()
        ))
    );
    assert!(lock(&entry.state).native_start.is_none());
    assert_eq!(t.spawner.pending_count(), 0);
    assert_eq!(*t.spawner.inner.active.borrow(), 0);
    assert_eq!(t.failures.load(Ordering::SeqCst), 1);
    assert_eq!(t.policy.records.load(Ordering::SeqCst), 0);
    assert!(!t.root.path().join("pid").exists());
}
