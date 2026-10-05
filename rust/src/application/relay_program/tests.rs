use super::*;
use crate::{
    application::{
        orchestration_program::OrchestrationDependencies, session_workers::SessionWorkers,
    },
    config::Config,
    files::FilesService,
    hub::task_owner::HubTaskOwner,
    orchestration::child_launch::ChildPreparer,
    profile::registry::{Registry, default_adapters, embedded_definitions},
    proto::provider::Layers,
    terminal::{
        events::CoreEventBus,
        journal::{JournalOptions, SessionJournal},
        session::EngineOptions,
    },
};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
#[derive(Default)]
struct Io {
    engine: Mutex<Weak<SessionEngine>>,
    starts: AtomicU64,
    fail: AtomicBool,
    block: AtomicBool,
    block_effects: AtomicBool,
    effect_entered: tokio::sync::Notify,
    effect_release: tokio::sync::Notify,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
    specs: Mutex<Vec<WrappedSpawnSpec>>,
    sent: Mutex<Vec<(LiveSessionId, Vec<u8>)>>,
    effects: Mutex<Vec<CoreEffect>>,
}
impl WrapperTransport for Io {
    fn send<'a>(
        &'a self,
        binding: SessionBinding,
        message: proto::Message,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            lock(&self.sent).push((binding.session, message.data));
            Ok(())
        })
    }
}
impl CoreEffectSink for Io {
    fn apply<'a>(&'a self, effects: CoreEffects) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
        Box::pin(async move {
            if self.block_effects.load(Ordering::SeqCst)
                && effects
                    .0
                    .iter()
                    .any(|effect| matches!(effect, CoreEffect::CancelSession { .. }))
            {
                self.effect_entered.notify_one();
                self.effect_release.notified().await;
            }
            lock(&self.effects).extend(effects.0);
            Ok(())
        })
    }
}
impl WrappedSessionSpawner for Io {
    fn spawn_and_wait<'a>(
        &'a self,
        spec: WrappedSpawnSpec,
        _: Duration,
        _: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome> {
        Box::pin(async move {
            lock(&self.specs).push(spec.clone());
            if self.fail.load(Ordering::SeqCst) {
                return SpawnWaitOutcome::Failed("synthetic launch failure".into());
            }
            if self.block.load(Ordering::SeqCst) {
                self.entered.notify_one();
                let token = spec.cancellation.token();
                tokio::select! { _=self.release.notified()=>{}, _=token.cancelled()=>return SpawnWaitOutcome::Failed("synthetic spawn canceled".into()) }
            }
            let id = self.starts.fetch_add(1, Ordering::SeqCst) + 100;
            let core = lock(&self.engine).upgrade().unwrap();
            match core
                .register(
                    RegisterRequest {
                        message: proto::Message {
                            provider: spec.provider,
                            model: spec.model,
                            execution_mode: spec.execution_mode,
                            cwd: spec.cwd.to_string_lossy().into_owned(),
                            label: spec.label,
                            pid: id as i64,
                            cols: 100,
                            rows: 30,
                            ..Default::default()
                        },
                        spawn_proof: spec
                            .registration_proof
                            .map(|p| p.as_header_value().to_owned()),
                    },
                    WrapperConnectionId(id),
                    Timestamp::now(),
                )
                .await
            {
                Ok(registration) => SpawnWaitOutcome::Registered(registration.binding),
                Err(error) => SpawnWaitOutcome::Failed(format!("{error:?}")),
            }
        })
    }
}
struct Fixture {
    _root: tempfile::TempDir,
    _tasks: HubTaskOwner,
    core: Arc<SessionEngine>,
    program: Arc<RelayProgram>,
    _orchestration: Arc<OrchestrationProgram>,
    io: Arc<Io>,
    parent: SessionBinding,
    config: Arc<ConfigStore>,
    paths: RuntimePaths,
}
async fn fixture(limit: i64) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let runtime = root.path().join("trial");
    std::fs::create_dir(&runtime).unwrap();
    let paths = RuntimePaths::trial(&runtime, 49668, &root.path().join("installed")).unwrap();
    let mut cfg = Config::default();
    cfg.orchestration.max_children_per_parent = limit;
    cfg.orchestration.max_total_sessions = limit + 1;
    cfg.orchestration.child_timeout_seconds = 0;
    cfg.orchestration.idle_done_threshold_sec = 0;
    cfg.orchestration.child_startup_fail = Some(false);
    let config = Arc::new(ConfigStore::new(paths.clone(), cfg).unwrap());
    let tasks = HubTaskOwner::new(tokio::runtime::Handle::current());
    let io = Arc::new(Io::default());
    let effects: Arc<dyn CoreEffectSink> = io.clone();
    let core = Arc::new(SessionEngine::new(
        EngineOptions::default(),
        Arc::new(SessionJournal::new(
            paths.clone(),
            None,
            JournalOptions {
                session_enabled: false,
                max_bytes: 0,
            },
        )),
        io.clone(),
        effects.clone(),
        io.clone(),
        CoreEventBus::new(64).unwrap(),
    ));
    *lock(&io.engine) = Arc::downgrade(&core);
    let parent = core
        .register(
            RegisterRequest {
                message: proto::Message {
                    provider: "codex".into(),
                    model: "synthetic".into(),
                    cwd: runtime.to_string_lossy().into_owned(),
                    started_at: "2026-10-05T00:00:00Z".into(),
                    pid: 7,
                    cols: 100,
                    rows: 30,
                    ..Default::default()
                },
                spawn_proof: None,
            },
            WrapperConnectionId(1),
            Timestamp::now(),
        )
        .await
        .unwrap()
        .binding;
    let workers = SessionWorkers::new(
        config.clone(),
        paths.clone(),
        Arc::new(FilesService::new(runtime.clone(), paths.clone())),
        tasks.handle(),
        Arc::new(|_, _| {}),
    );
    workers
        .bind(Arc::downgrade(&core), Arc::downgrade(&effects))
        .unwrap();
    let registry = Registry::build(
        Layers {
            embedded: Some(embedded_definitions().unwrap()),
            ..Default::default()
        },
        &default_adapters(),
    );
    let orchestration = OrchestrationProgram::new(OrchestrationDependencies {
        core: Arc::downgrade(&core),
        effects: Arc::downgrade(&effects),
        config: config.clone(),
        paths: paths.clone(),
        registry: Arc::new(move || Ok(registry.clone())),
        environment: vec!["PATH=".into()],
        hub_cwd: runtime.clone(),
        workers,
        tasks: tasks.handle(),
        warning: Arc::new(|_, _| {}),
    })
    .unwrap();
    // Git intentionally cannot run. same-tree supports a non-Git cwd, and these
    // tests exercise only synthetic registration, files and state transitions.
    let git = WorktreeGit::new(runtime.join("missing-synthetic-git"), BTreeMap::new());
    let executor = Arc::new(ChildLaunchExecutor::new(
        Arc::downgrade(&core),
        config.clone(),
        ChildPreparer::new(
            &paths,
            runtime.clone(),
            Some(runtime.join("home")),
            git.clone(),
        )
        .unwrap()
        .with_board_store(orchestration.board_store()),
        orchestration.clone(),
    ));
    let program = RelayProgram::new(RelayDependencies {
        core: Arc::downgrade(&core),
        effects: Arc::downgrade(&effects),
        config: config.clone(),
        paths: paths.clone(),
        orchestration: orchestration.clone(),
        executor,
        git,
        hub_cwd: runtime,
        tasks: tasks.handle(),
        warning: Arc::new(|_, _| {}),
    })
    .unwrap();
    Fixture {
        _root: root,
        _tasks: tasks,
        core,
        program,
        _orchestration: orchestration,
        io,
        parent,
        config,
        paths,
    }
}
fn request(f: &Fixture, name: &str, strong: bool, headless: bool) -> RelayStartRequest {
    let plan = f.paths.root().join(format!("{name}.md"));
    std::fs::write(&plan, "# Synthetic plan\nC1\nC2\n").unwrap();
    let mut roles = BTreeMap::new();
    for role in [IMPLEMENTATION, REVIEW] {
        roles.insert(
            role.into(),
            Some(Role {
                provider: if headless { "claude" } else { "codex" }.into(),
                model: "synthetic".into(),
                execution_mode: if headless { "headless" } else { "interactive" }.into(),
                ..Default::default()
            }),
        );
    }
    if strong {
        roles.insert(STRONG.into(), roles[IMPLEMENTATION].clone());
    }
    RelayStartRequest {
        plan_path: plan.to_string_lossy().into_owned(),
        mode: "same-tree".into(),
        roles,
        acknowledge_child_full_bypass: true,
        ..Default::default()
    }
}
async fn start(f: &Fixture, name: &str, strong: bool, headless: bool) -> proto::RelayStatus {
    f.program
        .start_relay(
            f.parent.session,
            request(f, name, strong, headless),
            Timestamp::now(),
            TaskCancellation::default(),
        )
        .await
        .unwrap()
        .0
}
fn progress(f: &Fixture, status: &proto::RelayStatus, child: i64, text: &str) {
    let path = f.program.boards.path(&status.orchestration_id);
    std::fs::write(
        path.parent().unwrap().join(format!("child-{child}.md")),
        text,
    )
    .unwrap();
}
async fn advance(f: &Fixture, status: &proto::RelayStatus, child: i64, text: &str) {
    progress(f, status, child, text);
    f.program
        .child_progress(
            &status.orchestration_id,
            LiveSessionId(child),
            text,
            Timestamp::now(),
            TaskCancellation::default(),
        )
        .await
        .unwrap();
}
fn status(f: &Fixture, id: &str) -> proto::RelayStatus {
    f.program
        .list(f.parent.session)
        .unwrap()
        .into_iter()
        .find(|i| i.relay.orchestration_id == id)
        .unwrap()
        .relay
}

#[tokio::test]
async fn relay_runs_final_implementation_review_completion_and_persists() {
    let f = fixture(6).await;
    let initial = start(&f, "one", false, false).await;
    assert_eq!(initial.state, "implementing");
    assert!(initial.implementation_session_id > 0);
    assert_eq!(initial.review_session_id, 0);
    advance(
        &f,
        &initial,
        initial.implementation_session_id,
        "## DONE implementation final=true\n",
    )
    .await;
    let reviewing = status(&f, &initial.orchestration_id);
    assert_eq!(reviewing.state, "reviewing");
    assert_eq!(reviewing.round, 1);
    advance(
        &f,
        &reviewing,
        reviewing.review_session_id,
        "verdict: pass\n## DONE review\n",
    )
    .await;
    let done = status(&f, &initial.orchestration_id);
    assert_eq!(done.state, "completed");
    assert_eq!(done.completed_cs, 1);
    let (files, warnings) = f.program.store.load().unwrap();
    assert!(warnings.is_empty());
    assert_eq!(files[0].state, "completed");
    assert!(files[0].events.iter().any(|e| e.kind == "completed"));
    assert!(lock(&f.io.effects).iter().any(|effect| matches!(effect, CoreEffect::Notify(CoreEvent::Completed{summary,..}) if summary.kind=="relay")));
    assert!(
        !lock(&f.io.effects)
            .iter()
            .any(|effect| matches!(effect, CoreEffect::Notify(CoreEvent::GitTurnCapture { .. })))
    );
    assert_eq!(
        f.core
            .details(f.parent.session)
            .unwrap()
            .snapshot
            .relays
            .len(),
        1
    );
}
#[tokio::test]
async fn second_relay_does_not_steal_first_board_or_orphan_its_reviewer() {
    let f = fixture(8).await;
    let first = start(&f, "first", false, false).await;
    let second = start(&f, "second", false, false).await;
    assert_ne!(first.orchestration_id, second.orchestration_id);
    advance(
        &f,
        &first,
        first.implementation_session_id,
        "## DONE implementation\n",
    )
    .await;
    let first = status(&f, &first.orchestration_id);
    assert_eq!(first.state, "reviewing");
    let reviewer = f
        .core
        .details(LiveSessionId(first.review_session_id))
        .unwrap();
    assert_eq!(reviewer.snapshot.orchestration_id.0, first.orchestration_id);
    assert_eq!(
        f.core
            .details(f.parent.session)
            .unwrap()
            .snapshot
            .orchestration_id
            .0,
        second.orchestration_id
    );
    let error = f
        .program
        .stop_relay(f.parent.session, "", Timestamp::now())
        .await
        .err()
        .unwrap();
    assert_eq!(error.code, "relay_ambiguous");
}
#[tokio::test]
async fn reserved_review_slot_cannot_be_consumed_by_optional_strong() {
    let f = fixture(2).await;
    let initial = start(&f, "hint", true, false).await;
    advance(
        &f,
        &initial,
        initial.implementation_session_id,
        "## DONE implementation escalate=true\n",
    )
    .await;
    let current = status(&f, &initial.orchestration_id);
    assert_eq!(current.state, "implementing");
    assert_eq!(current.strong_session_id, 0);
    assert_eq!(current.active_implementer, IMPLEMENTATION);
    advance(
        &f,
        &initial,
        initial.implementation_session_id,
        "## DONE implementation escalate=true\n## DONE implementation final=true\n",
    )
    .await;
    let reviewing = status(&f, &initial.orchestration_id);
    assert_eq!(reviewing.state, "reviewing");
    assert!(reviewing.review_session_id > 0);
}
#[tokio::test]
async fn findings_fix_rounds_stop_without_reusing_stale_done() {
    let f = fixture(5).await;
    let mut req = request(&f, "fix", false, false);
    req.max_rounds = 2;
    let initial = f
        .program
        .start_relay(
            f.parent.session,
            req,
            Timestamp::now(),
            TaskCancellation::default(),
        )
        .await
        .unwrap()
        .0;
    advance(
        &f,
        &initial,
        initial.implementation_session_id,
        "## DONE implementation\n",
    )
    .await;
    let mut current = status(&f, &initial.orchestration_id);
    std::fs::write(&current.review_path, "must fix").unwrap();
    advance(
        &f,
        &current,
        current.review_session_id,
        "verdict: findings must=1\n## DONE review\n",
    )
    .await;
    current = status(&f, &initial.orchestration_id);
    assert_eq!(current.state, "fixing");
    advance(
        &f,
        &initial,
        initial.implementation_session_id,
        "## DONE implementation\n",
    )
    .await;
    assert_eq!(status(&f, &initial.orchestration_id).state, "fixing");
    advance(
        &f,
        &initial,
        initial.implementation_session_id,
        "## DONE implementation\n## DONE implementation\n",
    )
    .await;
    current = status(&f, &initial.orchestration_id);
    assert_eq!(current.round, 2);
    std::fs::write(&current.review_path, "still must fix").unwrap();
    advance(
        &f,
        &current,
        current.review_session_id,
        "verdict: findings must=1\n## DONE review\nverdict: findings must=1\n## DONE review\n",
    )
    .await;
    current = status(&f, &initial.orchestration_id);
    assert_eq!(current.state, "stopped");
    assert_eq!(current.reason, "max_rounds");
}
#[tokio::test]
async fn source_verdict_failure_classes_are_retained() {
    for (name, body, expected) in [
        ("missing", "## DONE review\n", "verdict_missing"),
        (
            "file",
            "verdict: findings must=1\n## DONE review\n",
            "review_file_missing",
        ),
        (
            "blocked",
            "verdict: blocked reason=needs account\n## DONE review\n",
            "blocked: needs account",
        ),
    ] {
        let f = fixture(4).await;
        let initial = start(&f, name, false, false).await;
        advance(
            &f,
            &initial,
            initial.implementation_session_id,
            "## DONE implementation\n",
        )
        .await;
        let current = status(&f, &initial.orchestration_id);
        advance(&f, &current, current.review_session_id, body).await;
        assert_eq!(status(&f, &initial.orchestration_id).reason, expected);
    }
}
#[tokio::test]
async fn start_failure_persists_stopped_run_and_releases_two_slot_reservation() {
    let f = fixture(2).await;
    f.io.fail.store(true, Ordering::SeqCst);
    let error = f
        .program
        .start_relay(
            f.parent.session,
            request(&f, "failed", false, false),
            Timestamp::now(),
            TaskCancellation::default(),
        )
        .await
        .err()
        .unwrap();
    assert_eq!(error.code, "spawn_error");
    assert_eq!(
        f.program.list(f.parent.session).unwrap()[0].relay.reason,
        "spawn_error"
    );
    f.io.fail.store(false, Ordering::SeqCst);
    let next = start(&f, "next", false, false).await;
    assert_eq!(next.state, "implementing");
}
#[tokio::test]
async fn explicit_stop_keeps_children_alive_and_cannot_resume_user_stop() {
    let f = fixture(4).await;
    let initial = start(&f, "stop", false, false).await;
    let stopped = f
        .program
        .stop_relay(
            f.parent.session,
            &initial.orchestration_id,
            Timestamp::now(),
        )
        .await
        .unwrap();
    assert_eq!(stopped.reason, "user_stop");
    assert!(
        f.core
            .details(LiveSessionId(initial.implementation_session_id))
            .unwrap()
            .connected
    );
    assert_eq!(
        f.program
            .stop_relay(
                f.parent.session,
                &initial.orchestration_id,
                Timestamp::now()
            )
            .await
            .err()
            .unwrap()
            .code,
        "relay_not_running"
    );
    assert_eq!(
        f.program
            .resume_relay(
                f.parent.session,
                &initial.orchestration_id,
                Timestamp::now(),
                TaskCancellation::default()
            )
            .await
            .err()
            .unwrap()
            .code,
        "relay_not_resumable"
    );
}
#[tokio::test]
async fn headless_done_waits_for_exit_and_times_out_without_launching_reviewer() {
    let f = fixture(6).await;
    let mut cfg = f.config.snapshot().unwrap();
    cfg.config.orchestration.child_timeout_seconds = 5;
    f.config
        .publish_then_persist_legacy(cfg.revision, cfg.config)
        .unwrap();
    let initial = start(&f, "headless", false, true).await;
    let now = Timestamp::now();
    let text = "## DONE implementation final=true\n";
    progress(&f, &initial, initial.implementation_session_id, text);
    f.program
        .child_progress(
            &initial.orchestration_id,
            LiveSessionId(initial.implementation_session_id),
            text,
            now,
            TaskCancellation::default(),
        )
        .await
        .unwrap();
    assert_eq!(status(&f, &initial.orchestration_id).state, "implementing");
    assert_eq!(f.io.starts.load(Ordering::SeqCst), 1);
    f.program
        .poll(now + Duration::from_secs(6), TaskCancellation::default())
        .await
        .unwrap();
    let done = status(&f, &initial.orchestration_id);
    assert_eq!(done.state, "stopped");
    assert_eq!(done.reason, "timeout");
    assert!(
        f.core
            .details(LiveSessionId(initial.implementation_session_id))
            .unwrap()
            .connected
    );
}
#[tokio::test]
async fn independent_http_branches_preserve_errors_empty_list_and_optional_control_body() {
    let f = fixture(4).await;
    let http = crate::hub::relay_routes::RelayHttp::new(f.program.clone());
    let path = format!("/api/sessions/{}/relay", f.parent.session.0);
    let response = http
        .handle_authenticated(
            &crate::hub::http::Request {
                path: path.clone(),
                method: "GET".into(),
                ..Default::default()
            },
            &HttpWaitCancellation::default(),
            Timestamp::now(),
        )
        .await
        .unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&response.body).unwrap()["relays"],
        serde_json::json!([])
    );
    for (method, suffix, body, status_code, code) in [
        ("DELETE", "relay", "", 405, "method_not_allowed"),
        ("POST", "relay", "{", 400, "bad_request"),
        ("POST", "relay-stop", "", 404, "relay_not_found"),
        ("POST", "relay-resume", "", 404, "relay_not_found"),
        ("POST", "relay-cleanup", "", 400, "bad_request"),
    ] {
        let response = http
            .handle_authenticated(
                &crate::hub::http::Request {
                    path: format!("/api/sessions/{}/{suffix}", f.parent.session.0),
                    method: method.into(),
                    body: body.as_bytes().to_vec(),
                    ..Default::default()
                },
                &HttpWaitCancellation::default(),
                Timestamp::now(),
            )
            .await
            .unwrap();
        assert_eq!(response.status, status_code, "{suffix}");
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&response.body).unwrap()["error"],
            code
        );
    }
}
#[tokio::test]
async fn invalid_plan_permissions_and_roles_never_launch() {
    let f = fixture(4).await;
    let mut body = request(&f, "validate", false, false);
    body.acknowledge_child_full_bypass = false;
    assert_eq!(
        f.program
            .start_relay(
                f.parent.session,
                body,
                Timestamp::now(),
                TaskCancellation::default()
            )
            .await
            .err()
            .unwrap()
            .code,
        "full_bypass_acknowledgment_required"
    );
    let mut body = request(&f, "roles", false, false);
    body.roles.remove(REVIEW);
    assert_eq!(
        f.program
            .start_relay(
                f.parent.session,
                body,
                Timestamp::now(),
                TaskCancellation::default()
            )
            .await
            .err()
            .unwrap()
            .code,
        "relay_roles_missing"
    );
    let mut body = request(&f, "range", false, false);
    body.max_rounds = 10;
    assert_eq!(
        f.program
            .start_relay(
                f.parent.session,
                body,
                Timestamp::now(),
                TaskCancellation::default()
            )
            .await
            .err()
            .unwrap()
            .code,
        "bad_request"
    );
    assert_eq!(f.io.starts.load(Ordering::SeqCst), 0);
}

fn end(f: &Fixture, id: i64, state: &str) {
    let binding = f.core.details(LiveSessionId(id)).unwrap().binding;
    f.core
        .observe_end(
            binding,
            SessionEnd {
                declared_state: state.into(),
                exit_code: if state == "completed" { 0 } else { 1 },
                reason: String::new(),
            },
            Timestamp::now(),
        )
        .unwrap();
    f.core.disconnected(binding, Timestamp::now()).unwrap();
}
#[tokio::test]
async fn headless_nonzero_exit_overrides_early_final_done() {
    let f = fixture(4).await;
    let initial = start(&f, "headless-fail", false, true).await;
    progress(
        &f,
        &initial,
        initial.implementation_session_id,
        "## DONE implementation final=true\n",
    );
    end(&f, initial.implementation_session_id, "error");
    f.program
        .poll(Timestamp::now(), TaskCancellation::default())
        .await
        .unwrap();
    let stopped = status(&f, &initial.orchestration_id);
    assert_eq!(stopped.reason, "child_exited");
    assert_eq!(stopped.review_session_id, 0);
}
#[tokio::test]
async fn headless_each_instruction_gets_new_process_without_spending_retained_card_budget() {
    let f = fixture(2).await;
    let initial = start(&f, "headless-units", false, true).await;
    progress(
        &f,
        &initial,
        initial.implementation_session_id,
        "## DONE implementation\n",
    );
    end(&f, initial.implementation_session_id, "completed");
    f.program
        .poll(Timestamp::now(), TaskCancellation::default())
        .await
        .unwrap();
    let reviewing = status(&f, &initial.orchestration_id);
    assert_eq!(reviewing.state, "reviewing");
    progress(
        &f,
        &reviewing,
        reviewing.review_session_id,
        "verdict: pass\n## DONE review\n",
    );
    end(&f, reviewing.review_session_id, "completed");
    f.program
        .poll(Timestamp::now(), TaskCancellation::default())
        .await
        .unwrap();
    let next = status(&f, &initial.orchestration_id);
    assert_eq!(next.state, "implementing");
    assert_eq!(next.completed_cs, 1);
    assert_ne!(
        next.implementation_session_id,
        initial.implementation_session_id
    );
    assert!(
        lock(&f.io.specs)
            .last()
            .unwrap()
            .initial_prompt
            .contains("no memory of earlier rounds")
    );
    // Late EOF/card disappearance from a completed previous unit cannot stop it.
    f.core
        .dismiss(
            LiveSessionId(initial.implementation_session_id),
            Timestamp::now(),
        )
        .unwrap();
    f.program
        .poll(Timestamp::now(), TaskCancellation::default())
        .await
        .unwrap();
    assert_eq!(status(&f, &initial.orchestration_id).state, "implementing");
}
#[tokio::test]
async fn strong_escalation_resets_round_and_next_c_returns_to_cheap_implementer() {
    let f = fixture(5).await;
    let initial = start(&f, "strong", true, false).await;
    advance(
        &f,
        &initial,
        initial.implementation_session_id,
        "## DONE implementation escalate=true\n",
    )
    .await;
    let strong = status(&f, &initial.orchestration_id);
    assert_eq!(strong.active_implementer, STRONG);
    assert!(strong.strong_session_id > 0);
    assert_eq!(strong.round, 0);
    advance(
        &f,
        &strong,
        strong.strong_session_id,
        "## DONE implementation-strong\n",
    )
    .await;
    let reviewing = status(&f, &initial.orchestration_id);
    assert!(reviewing.review_path.contains("strong-r1"));
    advance(
        &f,
        &reviewing,
        reviewing.review_session_id,
        "verdict: findings should=2\n## DONE review\n",
    )
    .await;
    let next = status(&f, &initial.orchestration_id);
    assert_eq!(next.active_implementer, IMPLEMENTATION);
    assert_eq!(next.completed_cs, 1);
    assert_eq!(next.state, "implementing");
}
async fn cold_registration(f: &Fixture, label: &str, started: &str) -> SessionBinding {
    let message = proto::Message {
        session_id: 500 + f.core.snapshots().len() as i64,
        provider: "codex".into(),
        cwd: f.paths.root().to_string_lossy().into_owned(),
        label: label.into(),
        started_at: started.into(),
        pid: 800 + f.core.snapshots().len() as i64,
        cols: 100,
        rows: 30,
        ..Default::default()
    };
    let restored_metadata = f.program.resolve_reattach_metadata(&message);
    f.core
        .reattach(
            ReattachRequest {
                message,
                restored_metadata,
            },
            WrapperConnectionId(800 + f.core.snapshots().len() as u64),
            Timestamp::now(),
        )
        .await
        .unwrap()
        .binding
}

fn persisted(f: &Fixture, id: &str, parent_started: &str, child: i64, label: &str) -> RelayFile {
    let parent = f.core.details(f.parent.session).unwrap().snapshot;
    let board = f
        .program
        .boards
        .ensure(id, &parent, "restored synthetic", Timestamp::now())
        .unwrap();
    let body = request(f, "restored-plan", false, false);
    RelayFile {
        version: 1,
        orchestration_id: id.into(),
        board_path: board.to_string_lossy().into_owned(),
        parent_session_id: f.parent.session.0,
        parent_started_at: parent_started.into(),
        parent_provider: "codex".into(),
        parent_cwd: parent.cwd.clone(),
        plan_path: body.plan_path,
        mode: "same-tree".into(),
        max_rounds: 3,
        escalate_after: 2,
        state: "implementing".into(),
        active_implementer: IMPLEMENTATION.into(),
        roles: body
            .roles
            .into_iter()
            .filter_map(|(key, value)| value.map(|value| (key, value)))
            .collect(),
        child_cwd: parent.cwd,
        implementation_session_id: child,
        implementation_label: label.into(),
        ..Default::default()
    }
}
#[tokio::test]
async fn restored_children_do_not_adopt_recycled_parent_ids_and_preack_preserves_progress_identity()
{
    let f = fixture(8).await;
    let unrelated = cold_registration(&f, "unrelated", "2026-10-05T01:00:00Z").await;
    let file = persisted(
        &f,
        "r1-restored",
        "2026-10-04T00:00:00Z",
        unrelated.session.0,
        "old-worker",
    );
    f.program.store.save(&file).unwrap();
    let at = Timestamp::now();
    f.program.restore(at).await.unwrap();
    let child = cold_registration(&f, "old-worker", "2026-10-04T01:00:00Z").await;
    f.program.prepare_reattach(child, at).await.unwrap();
    let details = f.core.details(child.session).unwrap();
    assert_eq!(details.snapshot.orchestration_id.0, file.orchestration_id);
    assert_eq!(details.snapshot.parent_session_id.0, 0);
    let handle = lock(&f.program.state).runs[&file.orchestration_id].clone();
    {
        let run = handle.lock().await;
        assert_eq!(run.file.implementation_session_id, child.session.0);
        assert_eq!(run.file.implementation_progress_id, unrelated.session.0);
        assert!(!run.parent_attached);
    }
    std::fs::write(
        Path::new(&file.board_path)
            .parent()
            .unwrap()
            .join(format!("child-{}.md", unrelated.session.0)),
        "## DONE implementation\n",
    )
    .unwrap();
    f.program
        .poll(at + Duration::from_secs(2), TaskCancellation::default())
        .await
        .unwrap();
    assert_eq!(f.io.starts.load(Ordering::SeqCst), 0);
    assert!(f.core.details(unrelated.session).is_some());
    assert!(f.program.list(f.parent.session).unwrap().is_empty());
    let parent = cold_registration(&f, "restored-parent", "2026-10-04T00:00:00Z").await;
    f.program.prepare_reattach(parent, at).await.unwrap();
    assert_eq!(
        f.core
            .details(child.session)
            .unwrap()
            .snapshot
            .parent_session_id,
        parent.session
    );
    let run = handle.lock().await;
    assert!(run.parent_attached);
    assert_eq!(run.file.parent_session_id, parent.session.0);
}
#[tokio::test]
async fn restored_unattached_timeout_remains_invisible_and_explicit_resume_adopts_it() {
    let f = fixture(8).await;
    let file = persisted(
        &f,
        "r1-timeout",
        "2026-10-04T00:00:00Z",
        44,
        "missing-old-worker",
    );
    f.program.store.save(&file).unwrap();
    let at = Timestamp::now();
    f.program.restore(at).await.unwrap();
    f.program
        .poll(at + Duration::from_secs(121), TaskCancellation::default())
        .await
        .unwrap();
    assert!(f.program.list(f.parent.session).unwrap().is_empty());
    let adopted = f
        .program
        .resume_relay(
            f.parent.session,
            &file.orchestration_id,
            at + Duration::from_secs(122),
            TaskCancellation::default(),
        )
        .await
        .unwrap();
    assert_eq!(adopted.state, "implementing");
    assert!(adopted.implementation_session_id > 0);
    assert_eq!(f.program.list(f.parent.session).unwrap().len(), 1);
}
#[tokio::test]
async fn cold_preack_ignores_active_locked_relay_and_cannot_deadlock_registration() {
    let f = fixture(4).await;
    let active = start(&f, "active", false, false).await;
    let handle = lock(&f.program.state).runs[&active.orchestration_id].clone();
    let _guard = handle.lock().await;
    let child = f
        .core
        .details(LiveSessionId(active.implementation_session_id))
        .unwrap()
        .binding;
    tokio::time::timeout(
        Duration::from_millis(100),
        f.program.prepare_reattach(child, Timestamp::now()),
    )
    .await
    .expect("active relay must be excluded before its run lock")
    .unwrap();
}
#[cfg(unix)]
#[tokio::test]
async fn plan_symlink_escape_and_cleanup_alias_are_rejected_before_child_effects() {
    use std::os::unix::fs::symlink;
    let f = fixture(4).await;
    let outside = f._root.path().join("outside.md");
    std::fs::write(&outside, "private synthetic").unwrap();
    symlink(&outside, f.paths.root().join("escape.md")).unwrap();
    let mut body = request(&f, "safe", false, false);
    body.plan_path = "escape.md".into();
    assert!(
        f.program
            .start_relay(
                f.parent.session,
                body,
                Timestamp::now(),
                TaskCancellation::default()
            )
            .await
            .is_err()
    );
    assert_eq!(f.io.starts.load(Ordering::SeqCst), 0);
    let root = f.paths.root().join("trees");
    std::fs::create_dir(&root).unwrap();
    symlink(f._root.path(), root.join("escaped")).unwrap();
    let cfg = config::OrchestrationConfig {
        worktree_dir_root: root.to_string_lossy().into_owned(),
        ..Default::default()
    };
    assert!(
        f.program
            .git
            .validate_cleanup(f.paths.root(), &root.join("escaped"), &cfg)
            .is_err()
    );
}

fn start_http_request(f: &Fixture, name: &str) -> crate::hub::http::Request {
    let body = request(f, name, false, false);
    crate::hub::http::Request{path:format!("/api/sessions/{}/relay",f.parent.session.0),method:"POST".into(),body:serde_json::to_vec(&serde_json::json!({"plan_path":body.plan_path,"mode":body.mode,"roles":body.roles,"acknowledge_child_full_bypass":true})).unwrap(),..Default::default()}
}
#[tokio::test]
async fn http_disconnect_does_not_cancel_owned_launch_or_duplicate_registration() {
    let f = fixture(2).await;
    f.io.block.store(true, Ordering::SeqCst);
    let request = start_http_request(&f, "detached-http");
    let waiter = HttpWaitCancellation::default();
    let own_waiter = waiter.clone();
    let http = crate::hub::relay_routes::RelayHttp::new(f.program.clone());
    let task = tokio::spawn(async move {
        http.handle_authenticated(&request, &own_waiter, Timestamp::now())
            .await
            .unwrap()
    });
    tokio::time::timeout(Duration::from_secs(2), f.io.entered.notified())
        .await
        .unwrap();
    waiter.token().cancel();
    assert_eq!(task.await.unwrap().status, 503);
    assert_eq!(f.io.starts.load(Ordering::SeqCst), 0);
    f.io.release.notify_one();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if f.program
                .list(f.parent.session)
                .unwrap()
                .first()
                .is_some_and(|item| item.relay.state == "implementing")
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(f.io.starts.load(Ordering::SeqCst), 1);
    let (records, warnings) = f.program.store.load().unwrap();
    assert!(warnings.is_empty());
    assert_eq!(records[0].state, "implementing");
    assert!(records[0].implementation_session_id > 0);
}
#[tokio::test]
async fn hub_cancellation_leaves_recoverable_state_and_releases_reserved_capacity() {
    let f = fixture(2).await;
    f.io.block.store(true, Ordering::SeqCst);
    let request = start_http_request(&f, "cancel-owned");
    let http = crate::hub::relay_routes::RelayHttp::new(f.program.clone());
    let task = tokio::spawn(async move {
        http.handle_authenticated(&request, &HttpWaitCancellation::default(), Timestamp::now())
            .await
            .unwrap()
    });
    tokio::time::timeout(Duration::from_secs(2), f.io.entered.notified())
        .await
        .unwrap();
    f._tasks.stop_requests();
    f._tasks.drain_requests().await;
    f._tasks.stop_effects().unwrap();
    f._tasks.cancel_effects().unwrap();
    f._tasks.drain_effects().await;
    assert_eq!(task.await.unwrap().status, 503);
    let reservation = f
        .core
        .reserve_children(
            AdmissionRequest {
                parent: f.parent.session,
                slots: 2,
                origin: VerifiedSpawnOrigin::Autonomous,
                replace: None,
            },
            AdmissionLimits {
                max_children_per_parent: 2,
                max_total_sessions: 3,
            },
        )
        .unwrap();
    assert!(f.core.release_children(&reservation.id));
    let (records, warnings) = f.program.store.load().unwrap();
    assert!(warnings.is_empty());
    assert_eq!(records[0].state, "stopped");
    assert_eq!(records[0].reason, "hub_restart");
    assert_eq!(f.io.starts.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn authenticated_router_enforces_token_method_host_origin_before_relay_body() {
    let f = fixture(4).await;
    let mut config = f.config.snapshot().unwrap();
    config.config.token = "synthetic-relay-token".into();
    f.config
        .publish_then_persist_legacy(config.revision, config.config)
        .unwrap();
    let router = crate::hub::router::ServiceRouter::new(
        f.config.clone(),
        f.paths.clone(),
        f.core.clone(),
        49668,
    )
    .unwrap()
    .with_task_owner(f._tasks.handle())
    .with_relay(Arc::new(crate::hub::relay_routes::RelayHttp::new(
        f.program.clone(),
    )));
    let mut request = crate::hub::http::Request {
        path: format!("/api/sessions/{}/relay", f.parent.session.0),
        method: "DELETE".into(),
        host: "evil.invalid".into(),
        remote_addr: "127.0.0.1:55000".into(),
        body: b"{".to_vec(),
        headers: vec![("Origin".into(), "https://evil.invalid".into())],
        ..Default::default()
    };
    let call = |request: crate::hub::http::Request| {
        let router = &router;
        async move {
            router
                .handle_owned_async_at(
                    &request,
                    Timestamp::now(),
                    &TaskCancellation::default(),
                    &HttpWaitCancellation::default(),
                )
                .await
                .response
        }
    };
    assert_eq!(call(request.clone()).await.status, 401);
    request.query = "token=synthetic-relay-token".into();
    assert_eq!(call(request.clone()).await.status, 405);
    request.method = "POST".into();
    let badhost = call(request.clone()).await;
    assert!(
        String::from_utf8(badhost.body)
            .unwrap()
            .contains("host not allowed")
    );
    request.host = "127.0.0.1:49668".into();
    let badorigin = call(request.clone()).await;
    assert!(
        String::from_utf8(badorigin.body)
            .unwrap()
            .contains("origin not allowed")
    );
    request.method = "GET".into();
    let get = call(request.clone()).await;
    assert_eq!(get.status, 200);
    request.method = "POST".into();
    request.headers.clear();
    assert_eq!(call(request).await.status, 400);
    assert_eq!(f.io.starts.load(Ordering::SeqCst), 0);
}

fn delivered(f: &Fixture, id: i64, at: Timestamp) {
    let binding = f.core.details(LiveSessionId(id)).unwrap().binding;
    f.core
        .record_initial_prompt_outcome(
            binding,
            &crate::orchestration::initial_prompt::InitialPromptOutcome::Delivered {
                evidence: crate::orchestration::initial_prompt::DeliveryEvidence::EchoObserved,
                attempts: 1,
                composer_enter_retried: false,
            },
            at,
        )
        .unwrap();
}
#[tokio::test]
async fn relay_timeout_keeps_live_child_state_and_never_uses_ordinary_timeout_respawn() {
    let f = fixture(4).await;
    let mut cfg = f.config.snapshot().unwrap();
    cfg.config.orchestration.child_timeout_seconds = 2;
    cfg.config.orchestration.timeout_respawn = true;
    cfg.config.orchestration.max_timeout_respawns = 3;
    f.config
        .publish_then_persist_legacy(cfg.revision, cfg.config)
        .unwrap();
    let initial = start(&f, "timeout-state", false, false).await;
    let now = Timestamp::now();
    delivered(&f, initial.implementation_session_id, now);
    let before = f
        .core
        .details(LiveSessionId(initial.implementation_session_id))
        .unwrap()
        .snapshot
        .state;
    f.program
        .poll(now + Duration::from_secs(3), TaskCancellation::default())
        .await
        .unwrap();
    assert_eq!(status(&f, &initial.orchestration_id).reason, "timeout");
    let child = f
        .core
        .details(LiveSessionId(initial.implementation_session_id))
        .unwrap();
    assert_eq!(child.snapshot.state, before);
    assert!(child.connected);
    assert_eq!(f.io.starts.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn relay_startup_failure_retains_evidence_without_marking_live_child_error() {
    let f = fixture(4).await;
    let mut cfg = f.config.snapshot().unwrap();
    cfg.config.orchestration.child_startup_fail = Some(true);
    cfg.config.orchestration.child_startup_grace_seconds = 1;
    cfg.config.orchestration.child_startup_kill = Some(false);
    f.config
        .publish_then_persist_legacy(cfg.revision, cfg.config)
        .unwrap();
    let initial = start(&f, "startup-evidence", false, false).await;
    let now = Timestamp::now();
    delivered(&f, initial.implementation_session_id, now);
    let binding = f
        .core
        .details(LiveSessionId(initial.implementation_session_id))
        .unwrap()
        .binding;
    f.core
        .mark_orchestration_child_state(binding, "standby", now)
        .unwrap();
    f.program
        .poll(now, TaskCancellation::default())
        .await
        .unwrap();
    f.program
        .poll(now + Duration::from_secs(2), TaskCancellation::default())
        .await
        .unwrap();
    assert_eq!(
        status(&f, &initial.orchestration_id).reason,
        "startup_failed"
    );
    assert_eq!(
        f.core.details(binding.session).unwrap().snapshot.state,
        "standby"
    );
    assert!(
        std::fs::read_to_string(f.program.boards.path(&initial.orchestration_id))
            .unwrap()
            .contains("screen tail:")
    );
    assert!(!lock(&f.io.effects).iter().any(
        |effect| matches!(effect,CoreEffect::CancelSession{binding:target,..} if *target==binding)
    ));
}

#[tokio::test]
async fn relay_parent_notice_obeys_soft_default_and_explicit_interrupt() {
    for mode in ["soft-notify", "interrupt"] {
        let f = fixture(4).await;
        let mut cfg = f.config.snapshot().unwrap();
        cfg.config.orchestration.board_notify_mode = mode.into();
        f.config
            .publish_then_persist_legacy(cfg.revision, cfg.config)
            .unwrap();
        let initial = start(&f, mode, false, false).await;
        f.program
            .stop_relay(
                f.parent.session,
                &initial.orchestration_id,
                Timestamp::now(),
            )
            .await
            .unwrap();
        let sent = lock(&f.io.sent)
            .iter()
            .any(|(id, _)| *id == f.parent.session);
        let snapshot = f.core.details(f.parent.session).unwrap().snapshot;
        if mode == "soft-notify" {
            assert!(!sent);
            assert!(snapshot.board_notify_pending);
        } else {
            assert!(sent);
            assert!(!snapshot.board_notify_pending);
        }
        assert!(
            !std::fs::read_to_string(f.program.boards.path(&initial.orchestration_id))
                .unwrap()
                .contains("event pending:")
        );
    }
}
#[tokio::test]
async fn ordinary_error_notifier_cannot_inject_into_relay_conductor() {
    let f = fixture(4).await;
    let _initial = start(&f, "relay-error", false, false).await;
    f._orchestration
        .notify_orchestration_error(
            f.parent,
            "depth",
            "synthetic limit",
            TaskCancellation::default(),
        )
        .await
        .unwrap();
    assert!(
        !f.core
            .details(f.parent.session)
            .unwrap()
            .snapshot
            .board_notify_pending
    );
    assert!(
        !lock(&f.io.sent)
            .iter()
            .any(|(id, _)| *id == f.parent.session)
    );
}

#[tokio::test]
async fn cleanup_claim_blocks_resume_and_duplicate_cleanup_and_releases_on_abort() {
    let f = fixture(8).await;
    let initial = start(&f, "cleanup-race", false, false).await;
    f.program
        .stop_relay(
            f.parent.session,
            &initial.orchestration_id,
            Timestamp::now(),
        )
        .await
        .unwrap();
    let handle = lock(&f.program.state).runs[&initial.orchestration_id].clone();
    let tree = RelayGit::root(
        f.paths.root(),
        &f.config.snapshot().unwrap().config.orchestration,
    )
    .join("cleanup-race/relay");
    std::fs::create_dir_all(&tree).unwrap();
    let sentinel = tree.join("synthetic-user-work.txt");
    std::fs::write(&sentinel, "preserved synthetic work").unwrap();
    {
        let mut run = handle.lock().await;
        run.file.reason = "timeout".into();
        run.file.mode = "worktree".into();
        run.file.worktree_path = tree.to_string_lossy().into_owned();
        run.file.branch = "synthetic-branch".into();
    }
    f.io.block_effects.store(true, Ordering::SeqCst);
    let program = f.program.clone();
    let id = initial.orchestration_id.clone();
    let parent = f.parent.session;
    let cleanup = tokio::spawn(async move {
        program
            .cleanup_relay(parent, &id, Timestamp::now(), TaskCancellation::default())
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), f.io.effect_entered.notified())
        .await
        .unwrap();
    assert!(
        f.core
            .details(LiveSessionId(initial.implementation_session_id))
            .is_none()
    );
    let label = lock(&f.io.specs)[0].label.clone();
    let late = f
        .core
        .reattach(
            ReattachRequest {
                message: proto::Message {
                    session_id: 777,
                    provider: "codex".into(),
                    cwd: f.paths.root().to_string_lossy().into_owned(),
                    label: label.clone(),
                    pid: 777,
                    ..Default::default()
                },
                restored_metadata: f.program.resolve_reattach_metadata(&proto::Message {
                    label: label.clone(),
                    ..Default::default()
                }),
            },
            WrapperConnectionId(777),
            Timestamp::now(),
        )
        .await;
    assert!(
        matches!(late, Err(SessionError::InvalidRequest(ref detail)) if detail == "session dismissed")
    );
    assert!(f.core.details(LiveSessionId(777)).is_none());
    let (saved, _) = f.program.store.load().unwrap();
    assert!(
        saved
            .iter()
            .find(|file| file.orchestration_id == initial.orchestration_id)
            .unwrap()
            .revoked_child_labels
            .contains(&label)
    );

    let resumed = f
        .program
        .resume_relay(
            parent,
            &initial.orchestration_id,
            Timestamp::now(),
            TaskCancellation::default(),
        )
        .await
        .err()
        .unwrap();
    assert_eq!(resumed.code, "relay_cleanup_in_progress");
    let duplicate = f
        .program
        .cleanup_relay(
            parent,
            &initial.orchestration_id,
            Timestamp::now(),
            TaskCancellation::default(),
        )
        .await
        .err()
        .unwrap();
    assert_eq!(duplicate.code, "relay_cleanup_in_progress");
    assert_eq!(f.io.starts.load(Ordering::SeqCst), 1);
    assert_eq!(
        std::fs::read_to_string(&sentinel).unwrap(),
        "preserved synthetic work"
    );
    let fresh = fixture(8).await;
    let mut completed = persisted(&fresh, "completed-revoked", "old-parent", 0, "");
    completed.state = "completed".into();
    completed.revoked_child_labels = saved
        .iter()
        .find(|file| file.orchestration_id == initial.orchestration_id)
        .unwrap()
        .revoked_child_labels
        .clone();
    fresh.program.store.save(&completed).unwrap();
    fresh.program.restore(Timestamp::now()).await.unwrap();
    assert!(!fresh.program.owns(&completed.orchestration_id));
    let message = proto::Message {
        session_id: 778,
        label: label.clone(),
        provider: "codex".into(),
        cwd: fresh.paths.root().to_string_lossy().into_owned(),
        pid: 778,
        ..Default::default()
    };
    let metadata = fresh.program.resolve_reattach_metadata(&message);
    assert!(metadata.is_none());
    assert!(
        matches!(fresh.core.reattach(ReattachRequest { message, restored_metadata: metadata }, WrapperConnectionId(778), Timestamp::now()).await,
        Err(SessionError::InvalidRequest(ref detail)) if detail == "session dismissed")
    );
    cleanup.abort();
    assert!(cleanup.await.err().unwrap().is_cancelled());
    f.io.block_effects.store(false, Ordering::SeqCst);
    let retry = f
        .program
        .resume_relay(
            parent,
            &initial.orchestration_id,
            Timestamp::now(),
            TaskCancellation::default(),
        )
        .await
        .err()
        .unwrap();
    // This fixture intentionally has no Git executable. It reaches validation
    // after the cleanup claim is released; no process/provider ever runs.
    assert_eq!(retry.code, "spawn_error");
    assert_eq!(
        std::fs::read_to_string(&sentinel).unwrap(),
        "preserved synthetic work"
    );
}

#[tokio::test]
async fn interrupted_resume_persists_recoverable_state_before_child_launch() {
    let f = fixture(4).await;
    let mut file = persisted(&f, "resume-interrupted", "2026-10-05T00:00:00Z", 0, "");
    file.state = "stopped".into();
    file.reason = "timeout".into();
    f.program.store.save(&file).unwrap();
    f.program.restore(Timestamp::now()).await.unwrap();
    f.io.block.store(true, Ordering::SeqCst);
    let program = f.program.clone();
    let id = file.orchestration_id.clone();
    let parent = f.parent.session;
    let resume = tokio::spawn(async move {
        program
            .resume_relay(parent, &id, Timestamp::now(), TaskCancellation::default())
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), f.io.entered.notified())
        .await
        .unwrap();
    let (records, warnings) = f.program.store.load().unwrap();
    assert!(warnings.is_empty());
    let during = records
        .iter()
        .find(|r| r.orchestration_id == file.orchestration_id)
        .unwrap();
    assert_eq!(during.state, "stopped");
    assert_eq!(during.reason, "hub_restart");
    assert!(during.resumable());
    resume.abort();
    assert!(resume.await.err().unwrap().is_cancelled());
    let (records, _) = f.program.store.load().unwrap();
    let stopped = records
        .iter()
        .find(|r| r.orchestration_id == file.orchestration_id)
        .unwrap();
    assert_eq!(stopped.state, "stopped");
    assert_eq!(stopped.reason, "hub_restart");
    assert!(stopped.resumable());
    let reservation = f
        .core
        .reserve_children(
            AdmissionRequest {
                parent,
                slots: 4,
                origin: VerifiedSpawnOrigin::Autonomous,
                replace: None,
            },
            AdmissionLimits {
                max_children_per_parent: 4,
                max_total_sessions: 5,
            },
        )
        .unwrap();
    assert!(f.core.release_children(&reservation.id));
    // A fresh runtime owner must load the saved interrupted record rather than
    // silently dropping it as a stopped record with an empty reason.
    lock(&f.program.state).runs.clear();
    lock(&f.program.state).meta.clear();
    f.program.restore(Timestamp::now()).await.unwrap();
    assert!(f.program.owns(&file.orchestration_id));
}

#[tokio::test]
async fn resume_cannot_forget_prior_children_or_spawn_a_second_worktree_user() {
    let f = fixture(8).await;
    let initial = start(&f, "retained-child", false, false).await;
    f.program
        .stop_relay(
            f.parent.session,
            &initial.orchestration_id,
            Timestamp::now(),
        )
        .await
        .unwrap();
    let handle = lock(&f.program.state).runs[&initial.orchestration_id].clone();
    {
        let mut run = handle.lock().await;
        run.file.reason = "timeout".into();
    }
    let error = f
        .program
        .resume_relay(
            f.parent.session,
            &initial.orchestration_id,
            Timestamp::now(),
            TaskCancellation::default(),
        )
        .await
        .err()
        .expect("prior live child must block resume");
    assert_eq!(error.code, "relay_children_active");
    assert_eq!(f.io.starts.load(Ordering::SeqCst), 1);
    assert!(
        f.core
            .details(LiveSessionId(initial.implementation_session_id))
            .unwrap()
            .connected
    );
    assert_eq!(
        handle.lock().await.file.implementation_session_id,
        initial.implementation_session_id
    );
}

#[tokio::test]
async fn restored_numeric_child_ids_never_authorize_unrelated_dismissal_or_block_resume() {
    let f = fixture(8).await;
    let mut file = persisted(
        &f,
        "recycled-cleanup",
        "2026-10-05T00:00:00Z",
        f.parent.session.0,
        "missing-old-child",
    );
    let tree = RelayGit::root(
        f.paths.root(),
        &f.config.snapshot().unwrap().config.orchestration,
    )
    .join("recycled/relay");
    std::fs::create_dir_all(&tree).unwrap();
    file.parent_started_at = f
        .core
        .details(f.parent.session)
        .unwrap()
        .snapshot
        .started_at;
    file.mode = "worktree".into();
    file.worktree_path = tree.to_string_lossy().into_owned();
    file.branch = "synthetic".into();
    file.state = "stopped".into();
    file.reason = "timeout".into();
    f.program.store.save(&file).unwrap();
    f.program.restore(Timestamp::now()).await.unwrap();
    f.program
        .prepare_reattach(f.parent, Timestamp::now())
        .await
        .unwrap();
    let error = f
        .program
        .cleanup_relay(
            f.parent.session,
            &file.orchestration_id,
            Timestamp::now(),
            TaskCancellation::default(),
        )
        .await
        .err()
        .unwrap();
    assert_eq!(error.code, "relay_cleanup_error"); // fixture has no Git executable
    assert!(f.core.details(f.parent.session).is_some());
    assert!(!lock(&f.io.effects).iter().any(|effect| matches!(effect, CoreEffect::CancelSession { binding, .. } if binding.session == f.parent.session)));
    let handle = lock(&f.program.state).runs[&file.orchestration_id].clone();
    {
        let mut run = handle.lock().await;
        run.file.mode = "same-tree".into();
        run.file.worktree_path.clear();
    }
    let resumed = f
        .program
        .resume_relay(
            f.parent.session,
            &file.orchestration_id,
            Timestamp::now(),
            TaskCancellation::default(),
        )
        .await
        .unwrap();
    assert_eq!(resumed.state, "implementing");
    assert_ne!(resumed.implementation_session_id, f.parent.session.0);
    assert!(f.core.details(f.parent.session).is_some());
}

#[tokio::test]
async fn resume_rejects_old_launch_identity_even_without_restored_relay_metadata() {
    let f = fixture(8).await;
    let initial = start(&f, "metadata-free-old-child", false, false).await;
    let label = lock(&f.io.specs)[0].label.clone();
    f.program
        .stop_relay(
            f.parent.session,
            &initial.orchestration_id,
            Timestamp::now(),
        )
        .await
        .unwrap();
    let handle = lock(&f.program.state).runs[&initial.orchestration_id].clone();
    handle.lock().await.file.reason = "timeout".into();
    // This synthetic fixture has no persistence sink; release its ordered
    // dismissal ticket before admitting the replacement connection.
    drop(
        f.core
            .dismiss(
                LiveSessionId(initial.implementation_session_id),
                Timestamp::now(),
            )
            .unwrap(),
    );
    let message = proto::Message {
        session_id: 777,
        label,
        provider: "codex".into(),
        cwd: f.paths.root().to_string_lossy().into_owned(),
        pid: 777,
        ..Default::default()
    };
    let metadata = f.program.resolve_reattach_metadata(&message);
    assert!(metadata.is_none());
    let old = f
        .core
        .reattach(
            ReattachRequest {
                message,
                restored_metadata: metadata,
            },
            WrapperConnectionId(777),
            Timestamp::now(),
        )
        .await
        .unwrap();
    drop(old.after_registered);
    assert!(
        f.core
            .details(old.binding.session)
            .unwrap()
            .snapshot
            .orchestration_id
            .0
            .is_empty()
    );
    let error = f
        .program
        .resume_relay(
            f.parent.session,
            &initial.orchestration_id,
            Timestamp::now(),
            TaskCancellation::default(),
        )
        .await
        .err()
        .unwrap();
    assert_eq!(error.code, "relay_children_active");
    assert_eq!(f.io.starts.load(Ordering::SeqCst), 1);
    assert!(f.core.details(old.binding.session).unwrap().connected);
}
