use super::*;
#[derive(Default)]
struct CapturedSpawn(Mutex<Vec<WrappedSpawnSpec>>);
impl WrappedSessionSpawner for CapturedSpawn {
    fn spawn_and_wait<'a>(
        &'a self,
        spec: WrappedSpawnSpec,
        _: Duration,
        _: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome> {
        Box::pin(async move {
            self.0.lock().unwrap().push(spec);
            // This observer has timed out; the simulated startup remains pending.
            SpawnWaitOutcome::TimedOut
        })
    }
}
fn spec(metadata: SpawnRegistrationMetadata) -> WrappedSpawnSpec {
    WrappedSpawnSpec {
        registration_metadata: metadata,
        spawn_attempt: None,
        registration_proof: None,
        provider: "copilot".into(),
        cwd: "/fixture/project".into(),
        model: String::new(),
        model_selection: String::new(),
        risk_confirmed: false,
        label: "same-visible-label".into(),
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
async fn pending(
    f: &Fixture,
    capture: &CapturedSpawn,
    metadata: SpawnRegistrationMetadata,
) -> WrappedSpawnSpec {
    assert_eq!(
        f.engine
            .spawn_and_wait(
                spec(metadata),
                Duration::from_secs(1),
                &HttpWaitCancellation::default()
            )
            .await,
        SpawnWaitOutcome::TimedOut
    );
    capture.0.lock().unwrap().pop().unwrap()
}
fn request(spec: &WrappedSpawnSpec, pid: i64) -> RegisterRequest {
    RegisterRequest {
        message: proto::Message {
            provider: spec.provider.clone(),
            cwd: spec.cwd.to_string_lossy().into_owned(),
            label: spec.label.clone(),
            pid,
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
async fn proof_binds_server_metadata_before_persistence_snapshot_and_ack() {
    let capture = Arc::new(CapturedSpawn::default());
    let f = fixture_with_spawner(capture.clone());
    let parent = register(&f).await.binding.session;
    let launch = pending(
        &f,
        &capture,
        SpawnRegistrationMetadata {
            parent,
            role: "worker".into(),
            auto: true,
            depth: 1,
            orchestration: OrchestrationId("synthetic-board".into()),
            board_path: "/fixture/board.md".into(),
            worktree_branch: "synthetic/worker".into(),
            handoff_from: LiveSessionId(19),
            ..Default::default()
        },
    )
    .await;
    let mut reg = f
        .engine
        .register(request(&launch, 41), WrapperConnectionId(2), now())
        .await
        .unwrap();
    let receipt = reg.startup_receipt.as_ref().unwrap();
    assert_eq!(receipt.attempt(), launch.spawn_attempt.unwrap());
    assert_eq!(receipt.binding(), reg.binding);
    assert_eq!(receipt.pid(), 41);
    assert_eq!(reg.snapshot.parent_session_id, parent);
    assert_eq!(reg.snapshot.role, "worker");
    assert!(reg.snapshot.auto);
    assert_eq!(reg.snapshot.handoff_from, LiveSessionId(19));
    assert_eq!(reg.registered.orchestration_id, "synthetic-board");
    assert!(reg.registered.auto);
    assert_eq!(reg.registered.board_path, "/fixture/board.md");
    let row = f
        .store
        .session_overview_by_live_session(reg.binding.session)
        .unwrap();
    assert_eq!(row.parent_session_id, parent);
    assert_eq!(row.role, "worker");
    assert_eq!(
        row.orchestration_id,
        OrchestrationId("synthetic-board".into())
    );
    f.sink
        .apply(std::mem::take(&mut reg.after_registered))
        .await
        .unwrap();
    assert!(
        f.engine
            .register(request(&launch, 41), WrapperConnectionId(3), now())
            .await
            .is_err()
    );
}
#[tokio::test]
async fn malformed_startup_identity_does_not_consume_the_reserved_attempt() {
    let capture = Arc::new(CapturedSpawn::default());
    let f = fixture_with_spawner(capture.clone());
    let launch = pending(&f, &capture, SpawnRegistrationMetadata::default()).await;
    for pid in [0, -1, i64::MAX] {
        assert!(
            f.engine
                .register(request(&launch, pid), WrapperConnectionId(1), now())
                .await
                .is_err()
        );
    }
    let reg = f
        .engine
        .register(request(&launch, 42), WrapperConnectionId(2), now())
        .await
        .unwrap();
    assert_eq!(reg.startup_receipt.unwrap().pid(), 42);
}
#[tokio::test]
async fn identical_labels_cannot_exchange_attempt_metadata() {
    let capture = Arc::new(CapturedSpawn::default());
    let f = fixture_with_spawner(capture.clone());
    let one = pending(
        &f,
        &capture,
        SpawnRegistrationMetadata {
            role: "one".into(),
            ..Default::default()
        },
    )
    .await;
    let two = pending(
        &f,
        &capture,
        SpawnRegistrationMetadata {
            role: "two".into(),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(one.label, two.label);
    let mut two_reg = f
        .engine
        .register(request(&two, 52), WrapperConnectionId(2), now())
        .await
        .unwrap();
    f.sink
        .apply(std::mem::take(&mut two_reg.after_registered))
        .await
        .unwrap();
    let one_reg = f
        .engine
        .register(request(&one, 51), WrapperConnectionId(1), now())
        .await
        .unwrap();
    assert_eq!(two_reg.snapshot.role, "two");
    assert_eq!(one_reg.snapshot.role, "one");
    assert_eq!(
        two_reg.startup_receipt.unwrap().attempt(),
        two.spawn_attempt.unwrap()
    );
    assert_eq!(
        one_reg.startup_receipt.unwrap().attempt(),
        one.spawn_attempt.unwrap()
    );
}
#[tokio::test]
async fn manual_registration_has_no_startup_right_or_client_owned_board_metadata() {
    let f = fixture();
    let mut req = RegisterRequest {
        message: proto::Message {
            provider: "copilot".into(),
            cwd: "/fixture/project".into(),
            pid: -7,
            auto: true,
            orchestration_id: "wire-claim".into(),
            board_path: "/fixture/claimed".into(),
            ..Default::default()
        },
        spawn_proof: None,
    };
    req.message.label = "same-visible-label".into();
    let reg = f
        .engine
        .register(req, WrapperConnectionId(1), now())
        .await
        .unwrap();
    assert!(reg.startup_receipt.is_none());
    assert!(!reg.snapshot.auto);
    assert!(reg.registered.orchestration_id.is_empty());
    assert!(reg.snapshot.board_path.is_empty());
}

#[tokio::test]
async fn conductor_mark_is_idempotent_and_keeps_the_same_parent_on_warm_reconnect() {
    let f = fixture();
    let first = register(&f).await.binding;
    ui(&f, 7);
    let effects = f
        .engine
        .mark_conductor(
            first,
            OrchestrationId("owned-board".into()),
            "/fixture/board.md".into(),
        )
        .unwrap();
    assert_eq!(effects.0.len(), 1);
    assert!(matches!(effects.0[0], CoreEffect::SendUiBestEffort { .. }));
    assert!(
        f.engine
            .mark_conductor(
                first,
                OrchestrationId("owned-board".into()),
                "/fixture/board.md".into()
            )
            .unwrap()
            .0
            .is_empty()
    );
    let second = reattach(&f, first, b"", 0, 2, 0).await.binding;
    assert_eq!(first.incarnation, second.incarnation);
    f.engine
        .mark_conductor(
            first,
            OrchestrationId("next-board".into()),
            "/fixture/next.md".into(),
        )
        .unwrap();
    assert_eq!(
        f.engine.snapshot(second.session).unwrap().orchestration_id,
        OrchestrationId("next-board".into())
    );
    let stale = SessionBinding {
        incarnation: SessionIncarnation(first.incarnation.0 + 1),
        ..first
    };
    assert!(matches!(
        f.engine
            .mark_conductor(stale, OrchestrationId("stale".into()), String::new()),
        Err(SessionError::StaleBinding)
    ));
}
