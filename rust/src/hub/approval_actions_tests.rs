use super::*;
use crate::{
    approval::token::ONE_TAP_TTL,
    config::{Resource, RuntimePaths},
    proto::{self, core::*},
    storage::SqliteSessionStorage,
    terminal::{
        events::CoreEventBus,
        journal::{JournalOptions, SessionJournal},
        session::{EngineOptions, SessionEngine},
    },
};
use std::{
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::Notify;

fn now() -> Timestamp {
    Timestamp::UNIX_EPOCH + Duration::from_secs(1_700_000_000)
}
type SendHook = Arc<dyn Fn(SessionBinding) + Send + Sync>;
#[derive(Default)]
struct Transport {
    frames: Mutex<Vec<proto::Message>>,
    fail: AtomicBool,
    pause: AtomicBool,
    entered: Notify,
    resume: Notify,
    hook: Mutex<Option<SendHook>>,
}
impl WrapperTransport for Transport {
    fn send<'a>(
        &'a self,
        binding: SessionBinding,
        message: proto::Message,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            if self.pause.swap(false, Ordering::SeqCst) {
                self.entered.notify_one();
                self.resume.notified().await;
            }
            if self.fail.swap(false, Ordering::SeqCst) {
                return Err(SessionError::Transport("synthetic write failure".into()));
            }
            self.frames.lock().unwrap().push(message);
            let hook = self.hook.lock().unwrap().clone();
            if let Some(hook) = hook {
                hook(binding);
            }
            Ok(())
        })
    }
}
struct Sink {
    journal: Arc<SessionJournal>,
    bus: CoreEventBus,
    messages: Mutex<Vec<proto::Message>>,
    consumed: AtomicUsize,
    pause: AtomicBool,
    fail: AtomicBool,
    entered: Notify,
    resume: Notify,
    completed: Notify,
}
impl CoreEffectSink for Sink {
    fn apply<'a>(&'a self, effects: CoreEffects) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
        Box::pin(async move {
            for (index, effect) in effects.0.into_iter().enumerate() {
                let consumed = matches!(
                    &effect,
                    CoreEffect::Persist(PersistenceEffect::ApprovalConsumed { .. })
                        | CoreEffect::PersistBound {
                            effect: PersistenceEffect::ApprovalConsumed { .. },
                            ..
                        }
                );
                if consumed {
                    self.consumed.fetch_add(1, Ordering::SeqCst);
                    if self.pause.swap(false, Ordering::SeqCst) {
                        self.entered.notify_one();
                        self.resume.notified().await;
                    }
                    if self.fail.swap(false, Ordering::SeqCst) {
                        return Err(CoreEffectFailure {
                            index,
                            error: SessionError::Transport("synthetic closure failure".into()),
                        });
                    }
                }
                let result = match effect {
                    CoreEffect::Persist(effect) => self.journal.apply(effect),
                    CoreEffect::PersistBound {
                        binding,
                        scope,
                        effect,
                        order,
                    } => {
                        order.wait().await;
                        let result = self.journal.apply_bound(binding, scope, effect);
                        drop(order);
                        result
                    }
                    CoreEffect::Notify(event) => self.bus.publish(event).map(|_| ()),
                    CoreEffect::SendUi { message, .. }
                    | CoreEffect::SendUiBestEffort { message, .. } => {
                        self.messages.lock().unwrap().push(message);
                        Ok(())
                    }
                    _ => panic!("unexpected effect in approval action fixture"),
                };
                if let Err(error) = result {
                    return Err(CoreEffectFailure { index, error });
                }
            }
            self.completed.notify_one();
            Ok(())
        })
    }
}
#[derive(Default)]
struct TaskOwner {
    tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
}
impl ApprovalEffectOwner for TaskOwner {
    fn start(&self, job: CoreFuture<'static, ()>) -> CoreFuture<'static, Result<(), SessionError>> {
        let (done, waiter) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            job.await;
            let _ = done.send(());
        });
        self.tasks.lock().unwrap().push(task);
        Box::pin(async move {
            waiter
                .await
                .map_err(|_| SessionError::Transport("synthetic effect task failed".into()))
        })
    }
    fn drain(&self) -> CoreFuture<'_, ()> {
        Box::pin(async move {
            let tasks = std::mem::take(&mut *self.tasks.lock().unwrap());
            for task in tasks {
                task.await.unwrap();
            }
        })
    }
}
struct NoSpawn;
impl WrappedSessionSpawner for NoSpawn {
    fn spawn_and_wait<'a>(
        &'a self,
        _: WrappedSpawnSpec,
        _: Duration,
        _: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome> {
        Box::pin(async { SpawnWaitOutcome::Failed("synthetic test has no launcher".into()) })
    }
}
struct Fixture {
    _root: tempfile::TempDir,
    _installed: tempfile::TempDir,
    core: Arc<SessionEngine>,
    sink: Arc<Sink>,
    transport: Arc<Transport>,
    store: Arc<SqliteSessionStorage>,
    manager: Arc<OneTapManager>,
    clock: Arc<Mutex<Timestamp>>,
    warnings: Arc<AtomicUsize>,
    http: Arc<ApprovalActionHttp>,
    tasks: Arc<TaskOwner>,
}
fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49122, installed.path()).unwrap();
    let store = Arc::new(
        SqliteSessionStorage::open(
            &paths,
            StorageOptions::baseline(paths.resource(Resource::Logs)),
        )
        .unwrap(),
    );
    let journal = Arc::new(SessionJournal::new(
        paths,
        Some(store.clone()),
        JournalOptions {
            session_enabled: true,
            max_bytes: 0,
        },
    ));
    let bus = CoreEventBus::new(100).unwrap();
    let sink = Arc::new(Sink {
        journal: journal.clone(),
        bus: bus.clone(),
        messages: Mutex::new(Vec::new()),
        consumed: AtomicUsize::new(0),
        pause: AtomicBool::new(false),
        fail: AtomicBool::new(false),
        entered: Notify::new(),
        resume: Notify::new(),
        completed: Notify::new(),
    });
    let transport = Arc::new(Transport::default());
    let core = Arc::new(SessionEngine::new(
        EngineOptions {
            warning: Arc::new(|_, _| {}),
            ..Default::default()
        },
        journal,
        transport.clone(),
        sink.clone(),
        Arc::new(NoSpawn),
        bus,
    ));
    let manager = Arc::new(OneTapManager::new().unwrap());
    let clock = Arc::new(Mutex::new(now()));
    let warnings = Arc::new(AtomicUsize::new(0));
    let tasks = Arc::new(TaskOwner::default());
    let http = Arc::new(ApprovalActionHttp::new(
        core.clone(),
        sink.clone(),
        tasks.clone(),
        manager.clone(),
        Arc::new({
            let clock = clock.clone();
            move || *clock.lock().unwrap()
        }),
        Arc::new({
            let warnings = warnings.clone();
            move |_| {
                warnings.fetch_add(1, Ordering::SeqCst);
            }
        }),
    ));
    Fixture {
        _root: root,
        _installed: installed,
        core,
        sink,
        transport,
        store,
        manager,
        clock,
        warnings,
        http,
        tasks,
    }
}
fn output(command: &str, focus_yes: bool) -> OutputChunk {
    let bytes = format!(
        "\x1b[2J\x1b[HRun: {command}\r\nDo you want to proceed?\r\n{} 1. Yes\r\n{} 2. No",
        if focus_yes { "❯" } else { " " },
        if focus_yes { " " } else { "❯" }
    )
    .into_bytes();
    OutputChunk {
        total_pty_bytes: bytes.len() as i64,
        bytes,
    }
}
async fn native(f: &Fixture, command: &str) -> SessionBinding {
    let number = f.core.snapshots().len() as u64 + 1;
    let registration = f
        .core
        .register(
            RegisterRequest {
                message: proto::Message {
                    provider: "claude".into(),
                    cwd: "/synthetic/project".into(),
                    pid: 7,
                    cols: 120,
                    rows: 30,
                    ..Default::default()
                },
                spawn_proof: None,
            },
            WrapperConnectionId(number),
            now(),
        )
        .await
        .unwrap();
    f.sink.apply(registration.after_registered).await.unwrap();
    repaint(f, registration.binding, command, true).await;
    let ui = UiBinding {
        connection: UiConnectionId(number),
        auth_epoch: f.core.auth_epoch(),
    };
    f.core.attach_ui(ui, None, None).unwrap();
    f.sink
        .apply(f.core.finish_ui_priming(ui).unwrap())
        .await
        .unwrap();
    registration.binding
}
async fn repaint(f: &Fixture, binding: SessionBinding, command: &str, focus_yes: bool) {
    f.sink
        .apply(
            f.core
                .observe_output(binding, output(command, focus_yes), now())
                .unwrap(),
        )
        .await
        .unwrap();
}
fn bound(f: &Fixture, session: SessionBinding) -> ApprovalActionBinding {
    let record = f
        .core
        .details(session.session)
        .unwrap()
        .approval
        .record
        .unwrap();
    let r = record.data();
    ApprovalActionBinding {
        session,
        candidate_key: r.candidate.key.clone(),
        source_epoch: r.candidate.source_epoch,
        sig: r.sig.clone(),
    }
}
fn issue(f: &Fixture, binding: SessionBinding, action: OneTapAction) -> String {
    let b = bound(f, binding);
    f.manager
        .issue(
            binding.session,
            &b.sig,
            &b.sig,
            b.source_epoch,
            action,
            *f.clock.lock().unwrap(),
        )
        .unwrap()
}
fn pending(f: &Fixture, binding: SessionBinding) -> bool {
    f.core
        .details(binding.session)
        .unwrap()
        .approval
        .record
        .is_some()
}
fn frames(f: &Fixture) -> Vec<Vec<u8>> {
    f.transport
        .frames
        .lock()
        .unwrap()
        .iter()
        .map(|m| m.data.clone())
        .collect()
}
fn body(response: &Response) -> serde_json::Value {
    serde_json::from_slice(&response.body).unwrap()
}
fn error(response: &Response, status: u16, code: &str) {
    assert_eq!(
        response.status,
        status,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    assert_eq!(body(response)["error"], code);
}
async fn call(f: &Fixture, token: &str) -> Response {
    let response = f
        .http
        .one_tap_guarded(token, &TaskCancellation::default())
        .await;
    f.tasks.drain().await;
    response
}

#[test]
fn path_validation_is_available_before_method_and_host_guards() {
    for path in [
        "/api/approval-action/",
        "/api/approval-action/a/b",
        "/other",
    ] {
        error(
            &one_tap_token(path).unwrap_err(),
            401,
            "invalid_action_token",
        );
    }
    assert_eq!(one_tap_token("/api/approval-action/a.b").unwrap(), "a.b");
}

#[tokio::test]
async fn successful_one_tap_commits_shared_core_ledger_ui_and_nonce_once() {
    let f = fixture();
    let session = native(&f, "git status").await;
    let token = issue(&f, session, OneTapAction::Approve);
    let claim = f.manager.verify(&token, now()).unwrap();
    let response = call(&f, &token).await;
    assert_eq!(response.status, 200);
    assert_eq!(
        body(&response),
        json!({"ok":true,"session_id":session.session.0,"action":"approve"})
    );
    assert_eq!(frames(&f), vec![b"\r".to_vec()]);
    assert!(!pending(&f, session));
    assert_eq!(f.manager.consume(&claim, now()), Err(TokenError::Consumed));
    f.tasks.drain().await;
    let rows = f
        .store
        .approvals_by_live_session(session.session, 10, false)
        .unwrap()
        .unwrap();
    assert_eq!(rows[0].selected_text, "\r");
    assert_eq!(rows[0].state, "resolved");
    assert_eq!(f.sink.consumed.load(Ordering::SeqCst), 1);
    assert!(f.sink.messages.lock().unwrap().iter().any(|m| {
        m.approval_state
            .as_ref()
            .is_some_and(|a| a.close.as_ref().is_some_and(|c| c.reason == "answered"))
    }));
    // Go checks the pending record before consuming: ordinary successful replay
    // reports no pending approval, rather than action_already_used.
    error(&call(&f, &token).await, 409, "approval_not_pending");
    assert_eq!(frames(&f).len(), 1);
}

#[tokio::test]
async fn invalid_expired_and_mismatched_tokens_never_send_or_consume() {
    let f = fixture();
    let session = native(&f, "git status").await;
    let token = issue(&f, session, OneTapAction::Approve);
    for invalid in ["".to_string(), "bad.token".into(), format!("{token}x")] {
        error(&call(&f, &invalid).await, 401, "invalid_action_token");
    }
    let b = bound(&f, session);
    for (sid, aid, sig, epoch) in [
        (session.session, "different", b.sig.as_str(), b.source_epoch),
        (session.session, "wrong", "wrong", b.source_epoch),
        (
            LiveSessionId(987),
            b.sig.as_str(),
            b.sig.as_str(),
            b.source_epoch,
        ),
        (
            session.session,
            b.sig.as_str(),
            b.sig.as_str(),
            ApprovalSourceEpoch(b.source_epoch.0 + 1),
        ),
    ] {
        let token = f
            .manager
            .issue(sid, aid, sig, epoch, OneTapAction::Approve, now())
            .unwrap();
        error(&call(&f, &token).await, 409, "approval_not_pending");
        let claim = f.manager.verify(&token, now()).unwrap();
        f.manager.consume(&claim, now()).unwrap();
    }
    *f.clock.lock().unwrap() = now() + ONE_TAP_TTL;
    error(&call(&f, &token).await, 401, "invalid_action_token");
    assert!(frames(&f).is_empty());
    assert!(pending(&f, session));
}

#[tokio::test]
async fn failed_send_leaves_nonce_and_reservation_retryable() {
    let f = fixture();
    let session = native(&f, "git status").await;
    let token = issue(&f, session, OneTapAction::Reject);
    f.transport.fail.store(true, Ordering::SeqCst);
    error(&call(&f, &token).await, 409, "action_not_applied");
    assert!(frames(&f).is_empty());
    assert!(pending(&f, session));
    assert_eq!(call(&f, &token).await.status, 200);
    assert_eq!(frames(&f), vec![b"2\r".to_vec()]);
}

#[tokio::test]
async fn high_risk_approval_requires_hub_but_rejection_is_allowed() {
    let f = fixture();
    let session = native(&f, "rm -rf /synthetic/owned-output").await;
    let approve = issue(&f, session, OneTapAction::Approve);
    error(
        &call(&f, &approve).await,
        403,
        "high_risk_requires_in_app_confirmation",
    );
    assert!(frames(&f).is_empty());
    assert!(pending(&f, session));
    let reject = issue(&f, session, OneTapAction::Reject);
    assert_eq!(call(&f, &reject).await.status, 200);
    assert_eq!(frames(&f), vec![b"2\r".to_vec()]);
}

#[tokio::test]
async fn expiry_is_rechecked_after_successful_send_and_releases_the_lease() {
    for (after, succeeds) in [
        (now() + ONE_TAP_TTL - Duration::from_nanos(1), true),
        (now() + ONE_TAP_TTL, false),
    ] {
        let f = fixture();
        let session = native(&f, "git status").await;
        let token = issue(&f, session, OneTapAction::Approve);
        *f.transport.hook.lock().unwrap() = Some(Arc::new({
            let clock = f.clock.clone();
            move |_| *clock.lock().unwrap() = after
        }));
        let response = call(&f, &token).await;
        assert_eq!(frames(&f), vec![b"\r".to_vec()]);
        if succeeds {
            assert_eq!(response.status, 200);
            assert!(!pending(&f, session));
        } else {
            error(&response, 401, "invalid_action_token");
            assert!(pending(&f, session));
            *f.transport.hook.lock().unwrap() = None;
            // The original claim was not consumed, although input was sent.
            let claim = f.manager.verify(&token, now()).unwrap();
            f.manager.consume(&claim, now()).unwrap();
            let fresh = issue(&f, session, OneTapAction::Reject);
            assert_eq!(call(&f, &fresh).await.status, 200);
        }
    }
}

#[tokio::test]
async fn consumed_nonce_with_still_pending_record_is_checked_only_after_send() {
    let f = fixture();
    let session = native(&f, "git status").await;
    let token = issue(&f, session, OneTapAction::Approve);
    let claim = f.manager.verify(&token, now()).unwrap();
    f.manager.consume(&claim, now()).unwrap();
    error(&call(&f, &token).await, 409, "action_already_used");
    assert_eq!(frames(&f), vec![b"\r".to_vec()]);
    assert!(pending(&f, session));
    let fresh = issue(&f, session, OneTapAction::Reject);
    assert_eq!(call(&f, &fresh).await.status, 200);
}

#[tokio::test]
async fn replacement_during_send_survives_and_the_old_nonce_stays_consumed() {
    let f = fixture();
    let session = native(&f, "git status").await;
    let before = bound(&f, session);
    let token = issue(&f, session, OneTapAction::Approve);
    let claim = f.manager.verify(&token, now()).unwrap();
    *f.transport.hook.lock().unwrap() = Some(Arc::new({
        let core = f.core.clone();
        move |binding| {
            // Drop only the un-applied replacement effects in this focused race;
            // the shared core retains its actual replacement candidate.
            drop(
                core.observe_output(binding, output("git diff", true), now())
                    .unwrap(),
            );
        }
    }));
    error(&call(&f, &token).await, 409, "approval_not_pending");
    assert_ne!(bound(&f, session).candidate_key, before.candidate_key);
    assert!(pending(&f, session));
    assert_eq!(f.manager.consume(&claim, now()), Err(TokenError::Consumed));
    *f.transport.hook.lock().unwrap() = None;
    let fresh = issue(&f, session, OneTapAction::Reject);
    assert_eq!(call(&f, &fresh).await.status, 200);
}

#[tokio::test]
async fn one_tap_resolves_focus_after_the_actual_core_input_fifo() {
    let f = fixture();
    let session = native(&f, "git status").await;
    let before = bound(&f, session);
    let token = issue(&f, session, OneTapAction::Approve);
    let cancel = TaskCancellation::default();
    let predecessor = f.core.submit(
        session,
        InputRequest {
            bytes: b"not sent".to_vec(),
            authority: InputAuthority::Internal,
        },
        now(),
        &cancel,
    );
    let mut action = Box::pin(f.http.one_tap_guarded(&token, &cancel));
    assert!(futures_util::poll!(&mut action).is_pending());
    repaint(&f, session, "git status", false).await;
    assert_eq!(bound(&f, session), before);
    drop(predecessor);
    assert_eq!(action.await.status, 200);
    f.tasks.drain().await;
    assert_eq!(frames(&f), vec![b"1\r".to_vec()]);
}

#[tokio::test]
async fn cancelled_queued_action_can_be_retried_with_the_same_token() {
    let f = fixture();
    let session = native(&f, "git status").await;
    let token = issue(&f, session, OneTapAction::Approve);
    let cancel = TaskCancellation::default();
    let predecessor = f.core.submit(
        session,
        InputRequest {
            bytes: b"not sent".to_vec(),
            authority: InputAuthority::Internal,
        },
        now(),
        &cancel,
    );
    let mut action = Box::pin(f.http.one_tap_guarded(&token, &cancel));
    assert!(futures_util::poll!(&mut action).is_pending());
    cancel.cancel();
    error(&action.await, 409, "action_not_applied");
    drop(predecessor);
    assert!(frames(&f).is_empty());
    assert_eq!(call(&f, &token).await.status, 200);
}

#[tokio::test]
async fn dropped_send_future_releases_actual_core_reservation_without_consuming() {
    let f = fixture();
    let session = native(&f, "git status").await;
    let token = issue(&f, session, OneTapAction::Approve);
    f.transport.pause.store(true, Ordering::SeqCst);
    let cancel = TaskCancellation::default();
    let mut action = Box::pin(f.http.one_tap_guarded(&token, &cancel));
    assert!(futures_util::poll!(&mut action).is_pending());
    f.transport.entered.notified().await;
    drop(action);
    assert!(pending(&f, session));
    assert!(frames(&f).is_empty());
    assert_eq!(call(&f, &token).await.status, 200);
}

#[tokio::test]
async fn post_send_cancellation_and_http_drop_do_not_discard_committed_effects() {
    let f = fixture();
    let session = native(&f, "git status").await;
    let token = issue(&f, session, OneTapAction::Approve);
    f.sink.pause.store(true, Ordering::SeqCst);
    // Clear the notifications left by fixture setup before checking this drain.
    while futures_util::poll!(Box::pin(f.sink.completed.notified())).is_ready() {}
    let cancel = TaskCancellation::default();
    *f.transport.hook.lock().unwrap() = Some(Arc::new({
        let cancel = cancel.clone();
        move |_| cancel.cancel()
    }));
    let mut action = Box::pin(f.http.one_tap_guarded(&token, &cancel));
    assert!(futures_util::poll!(&mut action).is_pending());
    f.sink.entered.notified().await;
    assert!(!pending(&f, session));
    drop(action);
    f.sink.resume.notify_one();
    tokio::time::timeout(Duration::from_secs(5), f.sink.completed.notified())
        .await
        .unwrap();
    f.tasks.drain().await;
    let rows = f
        .store
        .approvals_by_live_session(session.session, 10, false)
        .unwrap()
        .unwrap();
    assert_eq!(rows[0].state, "resolved");
    assert_eq!(f.sink.consumed.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn committed_action_reports_effect_failure_without_retry_or_false_rollback() {
    let f = fixture();
    let session = native(&f, "git status").await;
    let token = issue(&f, session, OneTapAction::Approve);
    f.sink.fail.store(true, Ordering::SeqCst);
    assert_eq!(call(&f, &token).await.status, 200);
    assert!(!pending(&f, session));
    assert_eq!(f.warnings.load(Ordering::SeqCst), 1);
    assert_eq!(f.sink.consumed.load(Ordering::SeqCst), 1);
    assert_eq!(frames(&f).len(), 1);
}

#[tokio::test]
async fn dropping_the_returned_action_lease_releases_it_before_another_action() {
    let f = fixture();
    let session = native(&f, "git status").await;
    let cancel = TaskCancellation::default();
    let request = NativeActionRequest {
        binding: bound(&f, session),
        selection: NativeActionSelection::ApproveOnce,
        origin: NativeActionOrigin::OneTap {
            nonce: "synthetic".into(),
        },
    };
    let action = f
        .core
        .prepare_and_send(request.clone(), &cancel)
        .await
        .unwrap();
    drop(ActionLease::new(f.core.clone(), action));
    let retry = f.core.prepare_and_send(request, &cancel).await.unwrap();
    f.core.release(retry);
    assert!(pending(&f, session));
}

#[derive(Default)]
struct Rules {
    calls: Mutex<Vec<(String, String)>>,
    error: Mutex<Option<String>>,
}
impl ApprovalBatchRules for Rules {
    fn add_and_reload(&self, command: &str, cwd: &str) -> Result<ApprovalAutoRule, String> {
        self.calls
            .lock()
            .unwrap()
            .push((command.into(), cwd.into()));
        if let Some(error) = self.error.lock().unwrap().clone() {
            return Err(error);
        }
        Ok(ApprovalAutoRule {
            id: "synthetic-rule".into(),
            command: command.into(),
            risk: vec!["low".into()],
            working_dir: cwd.into(),
        })
    }
}
fn signature(f: &Fixture, session: SessionBinding) -> String {
    let detail = f.core.details(session.session).unwrap();
    batch_signature(
        &detail.snapshot.provider,
        &detail.snapshot.cwd,
        &detail.approval.record.unwrap().data().summary,
    )
}
async fn batch(f: &Fixture, rules: &Rules, body: Vec<u8>) -> Response {
    let result = f
        .http
        .batch_authenticated(
            &Request {
                method: "POST".into(),
                path: "/api/approval/batch".into(),
                body,
                ..Default::default()
            },
            rules,
            &TaskCancellation::default(),
        )
        .await;
    f.tasks.drain().await;
    result
}

#[tokio::test]
async fn batch_only_approves_matching_low_risk_and_retains_other_risks() {
    let f = fixture();
    let low = native(&f, "git status").await;
    let other_low = native(&f, "git status").await;
    let mid = native(&f, "git commit -m synthetic").await;
    let high = native(&f, "rm -rf /synthetic/owned-output").await;
    let rules = Rules::default();
    let response = batch(
        &f,
        &rules,
        serde_json::to_vec(&json!({"signature":signature(&f, low),"action":"approve"})).unwrap(),
    )
    .await;
    assert_eq!(body(&response), json!({"ok":true,"matched":2,"applied":2}));
    assert!(!pending(&f, low));
    assert!(!pending(&f, other_low));
    for session in [mid, high] {
        let response = batch(
            &f,
            &rules,
            serde_json::to_vec(&json!({"signature":signature(&f, session),"action":"approve"}))
                .unwrap(),
        )
        .await;
        assert_eq!(body(&response), json!({"ok":true,"matched":1,"applied":0}));
        assert!(pending(&f, session));
    }
    assert!(rules.calls.lock().unwrap().is_empty());
    assert_eq!(frames(&f).len(), 2);
}

#[tokio::test]
async fn deny_session_preserves_the_source_or_signature_matching_quirk() {
    let f = fixture();
    let target = native(&f, "git status").await;
    let by_signature = native(&f, "git diff").await;
    let untouched = native(&f, "git log").await;
    let response = batch(&f, &Rules::default(), serde_json::to_vec(&json!({
        "signature": signature(&f, by_signature), "action":"deny_session", "session_id":target.session.0,
    })).unwrap()).await;
    assert_eq!(body(&response), json!({"ok":true,"matched":2,"applied":2}));
    assert!(!pending(&f, target));
    assert!(!pending(&f, by_signature));
    assert!(pending(&f, untouched));
    assert_eq!(frames(&f), vec![b"2\r".to_vec(), b"2\r".to_vec()]);
}

#[tokio::test]
async fn batch_keeps_disconnected_after_matching_count_but_excludes_stale_screens() {
    let f = fixture();
    let first = native(&f, "git status").await;
    let disconnected = native(&f, "git status").await;
    let stale = native(&f, "git status").await;
    let sig = signature(&f, first);
    *f.transport.hook.lock().unwrap() = Some(Arc::new({
        let core = f.core.clone();
        move |_| {
            // The snapshot already counted both live records. Normal disconnect
            // closes the second, but must not retroactively reduce matched.
            drop(core.disconnected(disconnected, now()).unwrap());
        }
    }));
    let bytes = b"\x1b[2J\x1b[Hno prompt here".to_vec();
    f.sink
        .apply(
            f.core
                .observe_output(
                    stale,
                    OutputChunk {
                        total_pty_bytes: bytes.len() as i64,
                        bytes,
                    },
                    now(),
                )
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        pending(&f, stale),
        "one missing redraw does not close the record yet"
    );
    let response = batch(
        &f,
        &Rules::default(),
        serde_json::to_vec(&json!({"signature":sig,"action":"approve"})).unwrap(),
    )
    .await;
    *f.transport.hook.lock().unwrap() = None;
    assert_eq!(body(&response), json!({"ok":true,"matched":2,"applied":1}));
    assert_eq!(frames(&f), vec![b"\r".to_vec()]);
    assert!(!pending(&f, first));
    assert!(!pending(&f, disconnected));
    assert!(pending(&f, stale));
}

#[tokio::test]
async fn batch_auto_rule_requires_live_low_risk_and_propagates_real_adapter_result() {
    let f = fixture();
    let mid = native(&f, "git commit -m synthetic").await;
    let rules = Rules::default();
    let response = batch(
        &f,
        &rules,
        serde_json::to_vec(&json!({"signature":signature(&f, mid),"action":"auto_rule"})).unwrap(),
    )
    .await;
    error(&response, 403, "auto_rule_requires_low_risk");
    assert!(rules.calls.lock().unwrap().is_empty());
    let low = native(&f, "git status").await;
    let request =
        serde_json::to_vec(&json!({"signature":signature(&f, low),"action":"auto_rule"})).unwrap();
    *rules.error.lock().unwrap() = Some("synthetic policy save failed".into());
    let response = batch(&f, &rules, request.clone()).await;
    error(&response, 400, "auto_rule_not_added");
    assert_eq!(body(&response)["detail"], "synthetic policy save failed");
    *rules.error.lock().unwrap() = None;
    let response = batch(&f, &rules, request).await;
    assert_eq!(response.status, 200);
    assert_eq!(
        body(&response),
        json!({"ok":true,"matched":1,"rule":{"id":"synthetic-rule","command":"git status","risk":["low"],"working_dir":"/synthetic/project"}})
    );
    assert_eq!(rules.calls.lock().unwrap().len(), 2);
    assert!(pending(&f, low));
    assert!(frames(&f).is_empty());
}

#[tokio::test]
async fn malformed_or_oversized_batch_has_no_side_effects_and_first_value_is_accepted() {
    let f = fixture();
    let rules = Rules::default();
    for input in [
        b"{".to_vec(),
        b"[]".to_vec(),
        b"{\"session_id\":1.5}".to_vec(),
    ] {
        error(&batch(&f, &rules, input).await, 400, "bad_request");
    }
    for input in [b"null".as_slice(), b"{}", b"{\"action\":\"other\"}"] {
        error(
            &batch(&f, &rules, input.to_vec()).await,
            400,
            "invalid_batch_request",
        );
    }
    let mut large = b"{\"signature\":\"a\",\"action\":\"auto_rule\",\"unknown\":\"".to_vec();
    large.extend(std::iter::repeat_n(
        b'x',
        super::super::http::JSON_BODY_LIMIT,
    ));
    large.extend_from_slice(b"\"}");
    error(&batch(&f, &rules, large).await, 400, "bad_request");
    let response = batch(
        &f,
        &rules,
        b"{\"SIGNATURE\":\"a\",\"ACTION\":\"approve\",\"SESSION_ID\":null} {malformed trailing"
            .to_vec(),
    )
    .await;
    assert_eq!(body(&response), json!({"ok":true,"matched":0,"applied":0}));
    error(
        &batch(
            &f,
            &rules,
            b"{\"action\":\"deny_session\",\"session_id\":0}".to_vec(),
        )
        .await,
        400,
        "invalid_batch_action",
    );
    assert!(rules.calls.lock().unwrap().is_empty());
    assert!(frames(&f).is_empty());
}

fn oracle() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../tests/fixtures/services/approval-actions/go_oracle.json"
    ))
    .unwrap()
}

#[test]
fn error_status_and_body_match_extracted_fixed_go_writer() {
    let oracle = oracle();
    for case in oracle["errors"].as_array().unwrap() {
        let response = match case["name"].as_str().unwrap() {
            "invalid" => token_error(TokenError::Invalid),
            "expired" => token_error(TokenError::Expired),
            "consumed" => token_error(TokenError::Consumed),
            "high_risk" => action_error(ApprovalActionError::HighRisk),
            "not_pending" => action_error(ApprovalActionError::StaleCandidate),
            "other" => action_error(ApprovalActionError::Reserved),
            _ => panic!("unexpected oracle error case"),
        };
        assert_eq!(response.status, case["status"].as_u64().unwrap() as u16);
        assert_eq!(body(&response), case["body"]);
    }
}

#[test]
fn batch_decode_and_signatures_match_extracted_fixed_go_helpers() {
    let oracle = oracle();
    for case in oracle["decode"].as_array().unwrap() {
        let result = decode_json::<BatchRequest>(&Request {
            body: case["input"].as_str().unwrap().as_bytes().to_vec(),
            ..Default::default()
        });
        assert_eq!(result.is_ok(), case["ok"].as_bool().unwrap(), "{case}");
        if let Ok(request) = result {
            assert_eq!(request.signature, case["signature"]);
            assert_eq!(request.action, case["action"]);
            assert_eq!(request.session_id, case["session_id"]);
        }
    }
    for case in oracle["signatures"].as_array().unwrap() {
        let summary = proto::ApprovalSummary {
            command: case["command"].as_str().unwrap().into(),
            risk: case["risk"].as_str().unwrap().into(),
            ..Default::default()
        };
        assert_eq!(
            batch_signature(
                case["provider"].as_str().unwrap(),
                case["cwd"].as_str().unwrap(),
                &summary
            ),
            case["signature"]
        );
    }
}

#[test]
fn nanosecond_expiry_boundary_matches_extracted_fixed_go_consumption() {
    let oracle = oracle();
    for case in oracle["expiry"].as_array().unwrap() {
        let manager = OneTapManager::new().unwrap();
        let token = manager
            .issue(
                LiveSessionId(1),
                "sig",
                "sig",
                ApprovalSourceEpoch(1),
                OneTapAction::Approve,
                now(),
            )
            .unwrap();
        let claim = manager.verify(&token, now()).unwrap();
        let offset = case["offset_nanos"].as_i64().unwrap();
        let expires = now() + ONE_TAP_TTL;
        let after = if offset < 0 {
            expires - Duration::from_nanos(offset.unsigned_abs())
        } else {
            expires + Duration::from_nanos(offset as u64)
        };
        let result = manager.consume(&claim, after);
        assert_eq!(result.is_ok(), case["result"] == "ok");
        assert_eq!(manager.consume(&claim, now()).is_err(), case["used"] == 1);
    }
}
