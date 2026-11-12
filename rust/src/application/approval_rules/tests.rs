use super::*;
use crate::hub::approval_actions::ApprovalBatchRules;
use crate::{
    config::Config,
    hub::task_owner::HubTaskOwner,
    terminal::{
        events::CoreEventBus,
        journal::{JournalOptions, SessionJournal},
        session::{EngineOptions, PendingShellLookup},
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
    _owner: HubTaskOwner,
    rules: Arc<ApprovalRules>,
    core: Arc<SessionEngine>,
    _effects: Arc<dyn CoreEffectSink>,
    transport: Arc<Transport>,
    audit: Arc<Mutex<Vec<String>>>,
    config: Arc<ConfigStore>,
    // Last: fields drop in this order, and on Windows the folder cannot be
    // removed while a field above still holds a file inside it.
    _root: tempfile::TempDir,
}
impl Fixture {
    fn new(enabled: bool) -> Self {
        Self::with_sink(enabled, Arc::new(Sink))
    }
    /// The transcript holds the one pending call that `candidate` draws.
    fn with_sink(enabled: bool, effects: Arc<dyn CoreEffectSink>) -> Self {
        Self::build(enabled, effects, transcript(&["git status"]))
    }
    fn build(
        enabled: bool,
        effects: Arc<dyn CoreEffectSink>,
        pending_shell: Option<PendingShellLookup>,
    ) -> Self {
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
                pending_shell,
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
    /// Claude's Bash prompt: heading, command lines and description in one indent.
    async fn heading_candidate(&self, lines: &[&str]) -> (LiveSessionId, ImmutableApprovalRecord) {
        let (id, record, effects) = self.observe_candidate(heading_screen(lines)).await;
        drop(effects);
        (id, record)
    }
    async fn candidate_with_effects(
        &self,
        command: &str,
    ) -> (LiveSessionId, ImmutableApprovalRecord, CoreEffects) {
        self.observe_candidate(heading_screen(&[command])).await
    }
    /// A prompt with no heading: the command is only a line above the options.
    async fn inline_candidate(&self, command: &str) -> (LiveSessionId, ImmutableApprovalRecord) {
        let (id, record, effects) = self
            .observe_candidate(format!(
                "\x1b[2J\x1b[HRun: {command}\r\nDo you want to proceed?\r\n❯ 1. Yes\r\n  2. No"
            ))
            .await;
        drop(effects);
        (id, record)
    }
    async fn observe_candidate(
        &self,
        screen: String,
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
        let bytes = screen.into_bytes();
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

fn heading_screen(lines: &[&str]) -> String {
    let block = lines
        .iter()
        .map(|line| format!("   {line}\r\n"))
        .collect::<String>();
    format!(
        "\x1b[2J\x1b[H Bash command\r\n\r\n{block}   Synthetic description\r\n\r\nDo you want to proceed?\r\n❯ 1. Yes\r\n  2. No"
    )
}

#[tokio::test(start_paused = true)]
async fn audit_n08_a_command_like_line_above_a_prompt_without_a_heading_is_never_auto_approved() {
    // Before the fix this screen, a `Run:` line above any approval, was approved
    // by the literal rule: nothing checked that the prompt is a shell approval.
    // The transcript even holds a pending call of the same text.
    let f = Fixture::new(true);
    let (id, record) = f.inline_candidate("git status").await;
    f.rules
        .policy
        .add_and_reload("git status", "/synthetic/project")
        .unwrap();
    assert_eq!(record.data().summary.command, "git status");
    assert_eq!(record.data().summary.risk, "mid");
    assert!(!f.rules.try_apply(id, &record).await.unwrap());
    assert_eq!(f.transport.0.load(Ordering::SeqCst), 0);
    assert!(f.audit.lock().unwrap().is_empty());
    let last = f.rules.history().pop().unwrap();
    assert!(!last.decision.allowed);
    assert!(last.decision.rule_id.is_empty());
}

#[tokio::test(start_paused = true)]
async fn audit_n01_a_command_whose_masked_value_holds_a_second_command_is_never_auto_approved() {
    // Shown as `git status token=[REDACTED]`: a rule that takes one argument
    // used to match the masked text, and the masked text used to be low.
    let full = "git status token=x|./run.sh";
    let f = Fixture::build(true, Arc::new(Sink), transcript(&[full]));
    let (id, record) = f.heading_candidate(&[full]).await;
    let path = f.rules.policy.path();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        path,
        "version: 1\nrules:\n - id: one-argument\n   command: '^git status \\S+$'\n",
    )
    .unwrap();
    f.rules.policy.reload().unwrap();
    let policy = f.rules.policy.snapshot();
    assert_eq!(policy.active_rules(), 1, "{:?}", policy.warnings);
    assert!(
        policy
            .evaluate("git status token=[REDACTED]", "/synthetic/project", "low")
            .allowed,
        "the rule must match the masked text for this test to mean anything"
    );
    assert!(!f.rules.try_apply(id, &record).await.unwrap());
    assert_eq!(f.transport.0.load(Ordering::SeqCst), 0);
    let last = f.rules.history().pop().unwrap();
    assert_eq!(last.summary.command, "git status token=[REDACTED]");
    assert_eq!(last.summary.risk, "mid");
    assert!(!last.decision.allowed);
}

fn transcript(commands: &[&str]) -> Option<PendingShellLookup> {
    let commands: Vec<String> = commands.iter().map(|c| (*c).to_owned()).collect();
    Some(Arc::new(move |_| {
        let commands = commands.clone();
        Box::pin(async move { Some(commands) })
    }))
}

#[tokio::test(start_paused = true)]
async fn heading_prompt_without_transcript_evidence_is_never_auto_approved() {
    // Audit U23: only the first line under the heading was classified, so a rule
    // for `ls -la` approved a second line that removes files.
    let f = Fixture::build(true, Arc::new(Sink), None);
    let (id, record) = f.heading_candidate(&["ls -la", "rm -rf ./x"]).await;
    f.rules
        .policy
        .add_and_reload("ls -la", "/synthetic/project")
        .unwrap();
    assert_ne!(record.data().summary.risk, "low");
    assert!(!f.rules.try_apply(id, &record).await.unwrap());
    assert_eq!(f.transport.0.load(Ordering::SeqCst), 0);
    assert!(f.audit.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn heading_prompt_is_decided_on_the_whole_transcript_command() {
    let f = Fixture::build(true, Arc::new(Sink), transcript(&["ls -la\nrm -rf ./x"]));
    let (id, record) = f.heading_candidate(&["ls -la", "rm -rf ./x"]).await;
    f.rules
        .policy
        .add_and_reload("ls -la", "/synthetic/project")
        .unwrap();
    assert!(!f.rules.try_apply(id, &record).await.unwrap());
    assert_eq!(f.transport.0.load(Ordering::SeqCst), 0);
    let last = f.rules.history().pop().unwrap();
    assert_eq!(last.summary.command, "ls -la\nrm -rf ./x");
    assert_eq!(last.summary.risk, "high");
    assert!(!last.decision.allowed);
}

#[tokio::test(start_paused = true)]
async fn heading_prompt_confirmed_by_transcript_is_still_auto_approved() {
    let f = Fixture::build(true, Arc::new(Sink), transcript(&["ls -la"]));
    let (id, record) = f.heading_candidate(&["ls -la"]).await;
    f.rules
        .policy
        .add_and_reload("ls -la", "/synthetic/project")
        .unwrap();
    assert!(f.rules.try_apply(id, &record).await.unwrap());
    assert!(f.transport.0.load(Ordering::SeqCst) > 0);
}
