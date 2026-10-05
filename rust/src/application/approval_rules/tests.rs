use super::*;
use crate::hub::approval_actions::ApprovalBatchRules;
use crate::{
    config::Config,
    hub::task_owner::HubTaskOwner,
    terminal::{
        events::CoreEventBus,
        journal::{JournalOptions, SessionJournal},
        session::EngineOptions,
    },
};
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
#[derive(Default)]
struct Transport(AtomicUsize);
impl WrapperTransport for Transport {
    fn send<'a>(
        &'a self,
        _: SessionBinding,
        _: proto::Message,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }
}
struct Sink;
struct OrderedSink;
impl CoreEffectSink for OrderedSink {
    fn apply<'a>(&'a self, effects: CoreEffects) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
        Box::pin(async move {
            for effect in effects.0 {
                if let CoreEffect::PersistBound { order, .. } = effect {
                    order.wait().await;
                }
            }
            Ok(())
        })
    }
}
impl CoreEffectSink for Sink {
    fn apply<'a>(&'a self, _: CoreEffects) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
        Box::pin(async { Ok(()) })
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
        Box::pin(async { panic!("approval regression must not spawn a provider") })
    }
}
struct Fixture {
    _root: tempfile::TempDir,
    _owner: HubTaskOwner,
    rules: Arc<ApprovalRules>,
    core: Arc<SessionEngine>,
    _effects: Arc<dyn CoreEffectSink>,
    transport: Arc<Transport>,
    audit: Arc<Mutex<Vec<String>>>,
    config: Arc<ConfigStore>,
}
impl Fixture {
    fn new(enabled: bool) -> Self {
        Self::with_sink(enabled, Arc::new(Sink))
    }
    fn with_sink(enabled: bool, effects: Arc<dyn CoreEffectSink>) -> Self {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("runtime")).unwrap();
        std::fs::create_dir(root.path().join("installed")).unwrap();
        let paths = RuntimePaths::trial(
            &root.path().join("runtime"),
            49125,
            &root.path().join("installed"),
        )
        .unwrap();
        let mut config = Config::default();
        config.user_prefs.approval.auto_approval_enabled = enabled;
        let config = Arc::new(ConfigStore::new(paths.clone(), config).unwrap());
        let owner = HubTaskOwner::new(tokio::runtime::Handle::current());
        let audit = Arc::new(Mutex::new(Vec::new()));
        let output = audit.clone();
        let rules = ApprovalRules::new(
            config.clone(),
            paths.clone(),
            owner.handle(),
            Arc::new(|_, _| {}),
            Arc::new(move |record| {
                output
                    .lock()
                    .unwrap()
                    .push(serde_json::to_string(record).unwrap());
                Ok(())
            }),
        );
        let transport = Arc::new(Transport::default());
        let core = Arc::new(SessionEngine::new(
            EngineOptions {
                approval_policy: Some(rules.policy.live_callback()),
                ..Default::default()
            },
            Arc::new(SessionJournal::new(paths, None, JournalOptions::default())),
            transport.clone(),
            effects.clone(),
            Arc::new(NoSpawn),
            CoreEventBus::new(64).unwrap(),
        ));
        rules
            .bind(Arc::downgrade(&core), Arc::downgrade(&effects))
            .unwrap();
        Self {
            _root: root,
            _owner: owner,
            rules,
            core,
            _effects: effects,
            transport,
            audit,
            config,
        }
    }
    async fn candidate(&self, command: &str) -> (LiveSessionId, ImmutableApprovalRecord) {
        let (id, record, effects) = self.candidate_with_effects(command).await;
        drop(effects);
        (id, record)
    }
    async fn candidate_with_effects(
        &self,
        command: &str,
    ) -> (LiveSessionId, ImmutableApprovalRecord, CoreEffects) {
        let registered = self
            .core
            .register(
                RegisterRequest {
                    message: proto::Message {
                        provider: "claude".into(),
                        cwd: "/synthetic/project".into(),
                        cols: 120,
                        rows: 30,
                        pid: 7,
                        ..Default::default()
                    },
                    spawn_proof: None,
                },
                WrapperConnectionId(self.core.snapshots().len() as u64 + 1),
                Timestamp::now(),
            )
            .await
            .unwrap();
        let bytes = format!(
            "\x1b[2J\x1b[HRun: {command}\r\nDo you want to proceed?\r\n❯ 1. Yes\r\n  2. No"
        )
        .into_bytes();
        let effects = self
            .core
            .observe_output(
                registered.binding,
                OutputChunk {
                    total_pty_bytes: bytes.len() as i64,
                    bytes,
                },
                Timestamp::now(),
            )
            .unwrap();
        let record = self
            .core
            .details(registered.binding.session)
            .unwrap()
            .approval
            .record
            .unwrap();
        (registered.binding.session, record, effects)
    }
}
#[tokio::test(start_paused = true)]
async fn disabled_setting_records_would_approve_history_without_sending_or_auditing() {
    let f = Fixture::new(false);
    let (id, record) = f.candidate("git status").await;
    f.rules
        .policy
        .add_and_reload("git status", "/synthetic/project")
        .unwrap();
    for _ in 0..101 {
        assert!(!f.rules.try_apply(id, &record).await.unwrap());
    }
    assert_eq!(f.rules.history().len(), 100);
    assert!(f.rules.history().iter().all(|c| c.decision.allowed));
    assert_eq!(f.transport.0.load(Ordering::SeqCst), 0);
    assert!(f.audit.lock().unwrap().is_empty());
}
#[tokio::test(start_paused = true)]
async fn enabled_literal_rule_commits_once_and_writes_real_audit_then_stale_candidate_cannot_resend()
 {
    let f = Fixture::new(true);
    let (id, record) = f.candidate("git status").await;
    f.rules
        .policy
        .add_and_reload("git status", "/synthetic/project")
        .unwrap();
    assert!(f.rules.try_apply(id, &record).await.unwrap());
    let sent = f.transport.0.load(Ordering::SeqCst);
    assert!(sent > 0);
    assert!(!f.rules.try_apply(id, &record).await.unwrap());
    assert_eq!(f.transport.0.load(Ordering::SeqCst), sent);
    assert!(f.core.details(id).unwrap().approval.record.is_none());
    f._owner.drain_effects().await;
    let audit = f.audit.lock().unwrap();
    assert_eq!(audit.len(), 1);
    let parsed: serde_json::Value = serde_json::from_str(&audit[0]).unwrap();
    assert_eq!(parsed["command"], "git status");
    assert_eq!(parsed["risk"], "low");
    assert!(parsed["rule_id"].as_str().unwrap().starts_with("batch-"));
}
#[tokio::test(start_paused = true)]
async fn consumed_automatic_approval_returns_while_detection_ticket_is_held_and_removes_priming_open()
 {
    let f = Fixture::with_sink(true, Arc::new(OrderedSink));
    let ui = UiBinding {
        connection: UiConnectionId(42),
        auth_epoch: f.core.auth_epoch(),
    };
    f.core.attach_ui(ui, None, None).unwrap();
    let (id, record, detection) = f.candidate_with_effects("git status").await;
    assert!(
        detection
            .0
            .iter()
            .any(|effect| matches!(effect, CoreEffect::PersistBound { .. }))
    );
    f.rules
        .policy
        .add_and_reload("git status", "/synthetic/project")
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(1), f.rules.try_apply(id, &record))
            .await
            .expect("automatic observer must not wait behind its enclosing detection ticket")
            .unwrap()
    );
    assert!(f.audit.lock().unwrap().is_empty());
    let priming = f.core.finish_ui_priming(ui).unwrap();
    assert!(!priming.0.iter().any(
        |effect| matches!(effect,CoreEffect::SendUi{message,..} if message.session_id==id.0 && message.approval_state.as_ref().and_then(|state|state.open.as_ref()).is_some())
    ));
    drop(detection);
    f._owner.drain_effects().await;
    assert_eq!(f.audit.lock().unwrap().len(), 1);
    assert!(f.core.details(id).unwrap().approval.record.is_none());
}
#[tokio::test(start_paused = true)]
async fn live_setting_disable_after_detection_prevents_runtime_action() {
    let f = Fixture::new(true);
    let (id, record) = f.candidate("git status").await;
    f.rules
        .policy
        .add_and_reload("git status", "/synthetic/project")
        .unwrap();
    let mut snapshot = f.config.snapshot().unwrap();
    snapshot.config.user_prefs.approval.auto_approval_enabled = false;
    f.config
        .publish_then_persist_legacy(snapshot.revision, snapshot.config)
        .unwrap();
    assert!(!f.rules.try_apply(id, &record).await.unwrap());
    assert_eq!(f.transport.0.load(Ordering::SeqCst), 0);
    assert!(f.audit.lock().unwrap().is_empty());
}
