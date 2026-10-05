use super::*;

struct CapturedStart {
    outcome: SpawnStartOutcome,
    requests: Mutex<Vec<WrappedStartRequest>>,
}
impl WrappedSessionSpawner for CapturedStart {
    fn start_wrapped<'a>(
        &'a self,
        request: WrappedStartRequest,
        _: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnStartOutcome> {
        Box::pin(async move {
            self.requests.lock().unwrap().push(request);
            self.outcome.clone()
        })
    }
    fn spawn_and_wait<'a>(
        &'a self,
        _: WrappedSpawnSpec,
        _: Duration,
        _: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome> {
        Box::pin(async { panic!("ordinary Start must not wait for registration") })
    }
}
fn request() -> WrappedStartRequest {
    WrappedStartRequest {
        spec: WrappedSpawnSpec {
            registration_metadata: SpawnRegistrationMetadata {
                role: "synthetic-start-owner".into(),
                ..Default::default()
            },
            spawn_attempt: None,
            registration_proof: None,
            provider: "copilot".into(),
            cwd: "/fixture/project".into(),
            model: String::new(),
            model_selection: String::new(),
            risk_confirmed: false,
            label: "synthetic-start".into(),
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
        },
        policy: ResolvedSpawnPolicy {
            base_environment: vec![],
            route_environment: vec![],
            subscription_environment: vec![],
            effective_route: String::new(),
            current_model: String::new(),
            resolved_model: "synthetic-selected-model".into(),
            effort_args: vec![],
            ordinary_model_args: vec![],
        },
        kind: WrappedStartKind::OrdinaryAi { delegation: false },
    }
}
fn registration(spec: &WrappedSpawnSpec) -> RegisterRequest {
    RegisterRequest {
        message: proto::Message {
            provider: spec.provider.clone(),
            cwd: spec.cwd.to_string_lossy().into_owned(),
            label: spec.label.clone(),
            pid: 42,
            cols: 120,
            rows: 30,
            ..Default::default()
        },
        spawn_proof: spec
            .registration_proof
            .as_ref()
            .map(|p| p.as_header_value().to_owned()),
    }
}
#[tokio::test]
async fn native_start_receipt_and_cancelled_observer_keep_admission_until_registration() {
    for outcome in [
        SpawnStartOutcome::Started { pid: 42 },
        SpawnStartOutcome::WaiterCancelled,
    ] {
        let capture = Arc::new(CapturedStart {
            outcome: outcome.clone(),
            requests: Mutex::new(vec![]),
        });
        let f = fixture_with_spawner(capture.clone());
        assert_eq!(
            f.engine
                .start_wrapped(request(), &HttpWaitCancellation::default())
                .await,
            outcome
        );
        assert!(f.engine.begin_provider_update("copilot").is_err());
        let received = capture.requests.lock().unwrap().pop().unwrap();
        assert_eq!(received.policy.resolved_model, "synthetic-selected-model");
        assert_eq!(
            received.kind,
            WrappedStartKind::OrdinaryAi { delegation: false }
        );
        let mut reg = f
            .engine
            .register(registration(&received.spec), WrapperConnectionId(1), now())
            .await
            .unwrap();
        assert_eq!(reg.snapshot.role, "synthetic-start-owner");
        assert_eq!(
            reg.startup_receipt.unwrap().attempt(),
            received.spec.spawn_attempt.unwrap()
        );
        // Complete the accepted registration's ordered persistence effects
        // before submitting another registration, as the real WS caller does.
        f.sink
            .apply(std::mem::take(&mut reg.after_registered))
            .await
            .unwrap();
        assert!(
            f.engine
                .register(registration(&received.spec), WrapperConnectionId(2), now())
                .await
                .is_err()
        );
    }
}
#[tokio::test]
async fn failed_native_start_releases_only_its_own_admission_and_proof() {
    for outcome in [
        SpawnStartOutcome::Failed("synthetic failure".into()),
        SpawnStartOutcome::HubStopped,
    ] {
        let capture = Arc::new(CapturedStart {
            outcome: outcome.clone(),
            requests: Mutex::new(vec![]),
        });
        let f = fixture_with_spawner(capture.clone());
        let unrelated = f.engine.begin_provider_spawn("codex").unwrap();
        assert_eq!(
            f.engine
                .start_wrapped(request(), &HttpWaitCancellation::default())
                .await,
            outcome
        );
        let received = capture.requests.lock().unwrap().pop().unwrap();
        let update = f.engine.begin_provider_update("copilot").unwrap();
        f.engine.end_provider_update(update);
        assert!(f.engine.begin_provider_update("codex").is_err());
        assert!(
            f.engine
                .register(registration(&received.spec), WrapperConnectionId(1), now())
                .await
                .is_err()
        );
        f.engine.end_provider_spawn(unrelated);
    }
}

#[tokio::test]
async fn invalid_native_pid_does_not_claim_start_and_releases_admission() {
    let capture = Arc::new(CapturedStart {
        outcome: SpawnStartOutcome::Started { pid: 0 },
        requests: Mutex::new(vec![]),
    });
    let f = fixture_with_spawner(capture);
    assert_eq!(
        f.engine
            .start_wrapped(request(), &HttpWaitCancellation::default())
            .await,
        SpawnStartOutcome::Failed("launcher reported an invalid native PID".into())
    );
    let update = f.engine.begin_provider_update("copilot").unwrap();
    f.engine.end_provider_update(update);
}

#[tokio::test]
async fn repeated_start_attempt_keeps_original_proof_and_owner_failure_releases_it() {
    let capture = Arc::new(CapturedStart {
        outcome: SpawnStartOutcome::Started { pid: 42 },
        requests: Mutex::new(vec![]),
    });
    let f = fixture_with_spawner(capture.clone());
    let lease = f.engine.begin_provider_spawn("copilot").unwrap();
    let mut first = request();
    first.spec.spawn_attempt = Some(lease.id);
    assert_eq!(
        f.engine
            .start_wrapped(first, &HttpWaitCancellation::default())
            .await,
        SpawnStartOutcome::Started { pid: 42 }
    );
    let mut repeated = request();
    repeated.spec.spawn_attempt = Some(lease.id);
    assert_eq!(
        f.engine
            .start_wrapped(repeated, &HttpWaitCancellation::default())
            .await,
        SpawnStartOutcome::Failed("start attempt is already launched".into())
    );
    assert_eq!(capture.requests.lock().unwrap().len(), 1);
    assert!(f.engine.begin_provider_update("copilot").is_err());
    let received = capture.requests.lock().unwrap().pop().unwrap();
    f.engine.spawn_failed(lease.id, "codex").unwrap();
    assert!(f.engine.begin_provider_update("copilot").is_err());
    f.engine.spawn_failed(lease.id, "copilot").unwrap();
    let update = f.engine.begin_provider_update("copilot").unwrap();
    f.engine.end_provider_update(update);
    assert!(
        f.engine
            .register(registration(&received.spec), WrapperConnectionId(1), now())
            .await
            .is_err()
    );
}
