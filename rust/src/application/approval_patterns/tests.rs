use super::*;
use crate::{
    application::slash_commands::{SearchContext, SlashCmd, SlashIo},
    config::Config,
    hub::sockets::{EffectDriver, FrameWriter, OrderedEventObserver, SocketRegistry, WireFrame},
    process::Cancellation,
    proto::core::*,
    terminal::{
        events::CoreEventBus,
        journal::{JournalOptions, SessionJournal},
        session::EngineOptions,
    },
};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
struct Source {
    body: Mutex<Option<Vec<u8>>>,
    pending: bool,
}
impl SlashIo for Source {
    fn read<'a>(&'a self, source: &'a str) -> CoreFuture<'a, io::Result<Vec<u8>>> {
        Box::pin(async move {
            if self.pending {
                return std::future::pending().await;
            }
            if source != "synthetic://claude" {
                return Err(io::Error::other("synthetic unavailable source"));
            }
            self.body
                .lock()
                .unwrap()
                .clone()
                .ok_or_else(|| io::Error::other("synthetic unavailable source"))
        })
    }
    fn skills(&self, _: &str, _: &SearchContext) -> Vec<SlashCmd> {
        vec![]
    }
}
#[derive(Clone, Default)]
struct Recording {
    frames: Arc<Mutex<Vec<WireFrame>>>,
    fail: Arc<AtomicBool>,
}
impl FrameWriter for Recording {
    fn write<'a>(&'a mut self, frame: WireFrame) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            if self.fail.load(Ordering::SeqCst) && matches!(frame, WireFrame::Text(_)) {
                return Err(SessionError::Transport("synthetic UI write failure".into()));
            }
            self.frames.lock().unwrap().push(frame);
            Ok(())
        })
    }
}
impl Recording {
    fn messages(&self) -> Vec<crate::proto::Message> {
        self.frames
            .lock()
            .unwrap()
            .iter()
            .filter_map(|frame| match frame {
                WireFrame::Text(text) => Some(serde_json::from_str(text).unwrap()),
                WireFrame::Close => None,
            })
            .collect()
    }
}
struct NoExternalWork;
impl OrderedEventObserver for NoExternalWork {
    fn observe<'a>(&'a self, _: &'a CoreEvent) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async { panic!("pattern synchronization never publishes session events") })
    }
}
impl WrappedSessionSpawner for NoExternalWork {
    fn spawn_and_wait<'a>(
        &'a self,
        _: WrappedSpawnSpec,
        _: Duration,
        _: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome> {
        Box::pin(async { panic!("synthetic test forbids providers") })
    }
}
struct Fixture {
    _root: tempfile::TempDir,
    _installed: tempfile::TempDir,
    owner: Arc<ApprovalPatterns>,
    core: Arc<SessionEngine>,
    effects: Arc<EffectDriver>,
    sockets: Arc<SocketRegistry>,
    recording: Recording,
    warnings: Arc<Mutex<Vec<String>>>,
}
fn fixture_with_config(mut config: Config) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49687, installed.path()).unwrap();
    // Whitespace disables unused sources without selecting default URLs.
    config.approval_pattern_sources = serde_json::from_value(serde_json::json!({
        "claude":" ", "codex":" ", "copilot":" ", "cursor-agent":" ",
        "opencode":" ", "grok":" ", "command-code":" ", "common":" "
    }))
    .unwrap();
    config.approval_pattern_sources.claude = "synthetic://claude".into();
    let config = Arc::new(ConfigStore::new(paths.clone(), config).unwrap());
    let warnings = Arc::new(Mutex::new(Vec::new()));
    let sockets = Arc::new(SocketRegistry::default());
    let journal = Arc::new(SessionJournal::new(
        paths.clone(),
        None,
        JournalOptions::default(),
    ));
    let events = CoreEventBus::new(16).unwrap();
    let effects = Arc::new(
        EffectDriver::new(
            sockets.clone(),
            journal.clone(),
            Arc::new(events.clone()),
            Arc::new(NoExternalWork),
        )
        .with_warning_handler({
            let warnings = warnings.clone();
            Arc::new(move |operation, _| warnings.lock().unwrap().push(operation.into()))
        }),
    );
    let core = Arc::new(SessionEngine::new(
        EngineOptions::default(),
        journal,
        sockets.clone(),
        effects.clone(),
        Arc::new(NoExternalWork),
        events,
    ));
    let trait_core: Arc<dyn SessionCore> = core.clone();
    sockets.bind_core(Arc::downgrade(&trait_core)).unwrap();
    let ui = UiBinding {
        connection: sockets.next_ui().unwrap(),
        auth_epoch: core.auth_epoch(),
    };
    let recording = Recording::default();
    sockets.insert_ui(ui, Box::new(recording.clone())).unwrap();
    core.attach_ui(ui, None, None).unwrap();
    assert!(core.finish_ui_priming(ui).unwrap().0.is_empty());
    let owner = ApprovalPatterns::new(&paths, config, Arc::downgrade(&core), {
        let warnings = warnings.clone();
        Arc::new(move |operation| warnings.lock().unwrap().push(operation.into()))
    })
    .unwrap();
    owner.sync().unwrap();
    Fixture {
        _root: root,
        _installed: installed,
        owner,
        core,
        effects,
        sockets,
        recording,
        warnings,
    }
}
fn fixture() -> Fixture {
    fixture_with_config(Config::default())
}
#[tokio::test]
async fn remote_failure_empty_and_unchanged_preserve_profiles_and_only_changes_publish() {
    let fixture = fixture();
    let owner = &fixture.owner;
    let source = Arc::new(Source {
        body: Mutex::new(None),
        pending: false,
    });
    let sink = fixture.effects.as_ref();
    let cancel = Cancellation::default();
    let initial = owner
        .asset(&format!("{}.{}.json", "claude", "official"))
        .unwrap();
    owner.remote_sync(source.clone(), &cancel, sink).await;
    *source.body.lock().unwrap() = Some(b"ordinary text\n- no phrase\n".to_vec());
    owner.remote_sync(source.clone(), &cancel, sink).await;
    assert_eq!(
        owner
            .asset(&format!("{}.{}.json", "claude", "official"))
            .unwrap(),
        initial
    );
    assert!(fixture.recording.messages().is_empty());
    *source.body.lock().unwrap() =
        Some(b"- `synthetic exact phrase`\n* `synthetic exact phrase`\n".to_vec());
    owner.remote_sync(source.clone(), &cancel, sink).await;
    assert_eq!(
        owner.active().unwrap()["claude"],
        vec!["synthetic exact phrase"]
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&owner.asset("claude.json").unwrap()).unwrap(),
        serde_json::json!(["synthetic exact phrase"])
    );
    assert_eq!(
        fixture.recording.messages()[0].r#type,
        "approval_patterns_updated"
    );
    assert_eq!(fixture.recording.messages()[0].providers, vec!["claude"]);
    owner.remote_sync(source, &cancel, sink).await;
    assert_eq!(fixture.recording.messages().len(), 1);
}
#[tokio::test]
async fn remote_updates_pass_real_ui_priming_and_continue_live_without_warnings() {
    let fixture = fixture();
    let ui = UiBinding {
        connection: fixture.sockets.next_ui().unwrap(),
        auth_epoch: fixture.core.auth_epoch(),
    };
    let recording = Recording::default();
    fixture
        .sockets
        .insert_ui(ui, Box::new(recording.clone()))
        .unwrap();
    let priming = fixture.core.attach_ui(ui, None, None).unwrap();
    let source = Arc::new(Source {
        body: Mutex::new(Some(b"- `synthetic priming phrase`\n".to_vec())),
        pending: false,
    });
    let cancel = Cancellation::default();
    fixture
        .owner
        .remote_sync(source.clone(), &cancel, fixture.effects.as_ref())
        .await;
    assert!(fixture.warnings.lock().unwrap().is_empty());
    assert!(
        recording.frames.lock().unwrap().is_empty(),
        "update must wait behind the initial snapshot"
    );
    assert_eq!(
        fixture.recording.messages().len(),
        1,
        "already primed UI receives live updates"
    );
    assert_eq!(
        fixture.owner.active().unwrap()["claude"],
        vec!["synthetic priming phrase"]
    );

    fixture
        .sockets
        .send_ui_value(
            ui,
            &serde_json::json!({
                "type":"snapshot", "hub_instance":priming.hub_instance, "sessions":priming.sessions
            }),
        )
        .await
        .unwrap();
    for message in priming.ordered_frames {
        fixture.sockets.send_ui(ui, message).await.unwrap();
    }
    let queued = fixture.core.finish_ui_priming(ui).unwrap();
    assert_eq!(queued.0.len(), 1);
    fixture.effects.apply(queued).await.unwrap();
    assert!(fixture.core.finish_ui_priming(ui).unwrap().0.is_empty());
    let messages = recording.messages();
    assert_eq!(
        messages
            .iter()
            .map(|message| message.r#type.as_str())
            .collect::<Vec<_>>(),
        ["snapshot", "approval_snapshot", "approval_patterns_updated"]
    );
    assert_eq!(messages[2].providers, ["claude"]);

    *source.body.lock().unwrap() = Some(b"- `synthetic live phrase`\n".to_vec());
    fixture
        .owner
        .remote_sync(source.clone(), &cancel, fixture.effects.as_ref())
        .await;
    assert_eq!(recording.messages().len(), 4);
    assert_eq!(fixture.recording.messages().len(), 2);
    assert_eq!(recording.messages()[3].providers, ["claude"]);
    fixture
        .owner
        .remote_sync(source.clone(), &cancel, fixture.effects.as_ref())
        .await;
    assert_eq!(
        recording.messages().len(),
        4,
        "unchanged sources do not notify twice"
    );
    assert!(fixture.warnings.lock().unwrap().is_empty());

    // Reusing the real driver also preserves the core's authentication boundary.
    fixture
        .effects
        .apply(fixture.core.invalidate_all_ui())
        .await
        .unwrap();
    *source.body.lock().unwrap() = Some(b"- `synthetic after revocation`\n".to_vec());
    fixture
        .owner
        .remote_sync(source, &cancel, fixture.effects.as_ref())
        .await;
    assert_eq!(recording.messages().len(), 4);
    assert_eq!(fixture.recording.messages().len(), 2);
    assert!(fixture.warnings.lock().unwrap().is_empty());
    assert!(matches!(
        fixture.core.finish_ui_priming(ui),
        Err(SessionError::AuthenticationExpired)
    ));
}
#[tokio::test]
async fn remote_update_real_writer_failure_keeps_transport_warning() {
    let fixture = fixture();
    fixture.recording.fail.store(true, Ordering::SeqCst);
    fixture
        .owner
        .remote_sync(
            Arc::new(Source {
                body: Mutex::new(Some(b"- `synthetic failed delivery`\n".to_vec())),
                pending: false,
            }),
            &Cancellation::default(),
            fixture.effects.as_ref(),
        )
        .await;
    assert_eq!(
        fixture.owner.active().unwrap()["claude"],
        vec!["synthetic failed delivery"]
    );
    assert_eq!(*fixture.warnings.lock().unwrap(), ["broadcast UI delivery"]);
    assert_eq!(
        *fixture.recording.frames.lock().unwrap(),
        [WireFrame::Close]
    );
}
#[tokio::test]
async fn remote_update_after_core_drop_warns_without_retaining_core() {
    let fixture = fixture();
    let weak = Arc::downgrade(&fixture.core);
    drop(fixture.core);
    assert!(weak.upgrade().is_none());
    fixture
        .owner
        .remote_sync(
            Arc::new(Source {
                body: Mutex::new(Some(b"- `synthetic after shutdown`\n".to_vec())),
                pending: false,
            }),
            &Cancellation::default(),
            fixture.effects.as_ref(),
        )
        .await;
    assert_eq!(
        fixture.owner.active().unwrap()["claude"],
        vec!["synthetic after shutdown"]
    );
    assert_eq!(
        *fixture.warnings.lock().unwrap(),
        ["approval pattern update delivery failed"]
    );
    assert!(fixture.recording.messages().is_empty());
}
#[tokio::test]
async fn cancellation_drops_pending_fetches_without_overwriting_private_mirrors() {
    let fixture = fixture();
    let owner = &fixture.owner;
    let before = owner.asset("claude.json").unwrap();
    let sink = fixture.effects.as_ref();
    let cancel = Cancellation::default();
    let source = Arc::new(Source {
        body: Mutex::new(None),
        pending: true,
    });
    let operation = owner.remote_sync(source, &cancel, sink);
    tokio::pin!(operation);
    tokio::select! {_=&mut operation=>panic!("pending source completed"),_=tokio::task::yield_now()=>{}}
    cancel.cancel();
    tokio::time::timeout(std::time::Duration::from_secs(1), operation)
        .await
        .unwrap();
    assert_eq!(owner.asset("claude.json").unwrap(), before);
    assert!(fixture.recording.messages().is_empty());
}
#[test]
fn source_markdown_boundary_crlf_empty_and_duplicate_rules() {
    assert_eq!(
        parse_markdown(
            "- `kept`\n* `kept`\n- `excluded`\r\n-`missing separator`\n- ` `\n- `nested`extra`\n"
        ),
        vec!["kept"]
    );
    assert_eq!(parse_markdown("- `first\nsecond`\n"), vec!["first\nsecond"]);
}
struct DelayedSource;
impl SlashIo for DelayedSource {
    fn read<'a>(&'a self, _: &'a str) -> CoreFuture<'a, io::Result<Vec<u8>>> {
        Box::pin(async move {
            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
            Ok(b"- `synthetic parallel phrase`\n".to_vec())
        })
    }
    fn skills(&self, _: &str, _: &SearchContext) -> Vec<SlashCmd> {
        vec![]
    }
}
#[tokio::test(start_paused = true)]
async fn custom_sources_all_start_together_and_do_not_wait_behind_builtin_deadline() {
    let mut config = Config::default();
    config.custom_providers = (0..9)
        .map(|index| crate::config::CustomProvider {
            id: format!("synthetic-{index}"),
            command: "synthetic.exe".into(),
            approval_pattern_source: format!("synthetic://{index}"),
            ..Default::default()
        })
        .collect();
    let fixture = fixture_with_config(config);
    let owner = &fixture.owner;
    let before = tokio::time::Instant::now();
    let sink = fixture.effects.as_ref();
    owner
        .remote_sync(Arc::new(DelayedSource), &Cancellation::default(), sink)
        .await;
    assert_eq!(before.elapsed(), std::time::Duration::from_secs(15));
    for index in 0..9 {
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(
                &owner.asset(&format!("synthetic-{index}.json")).unwrap()
            )
            .unwrap(),
            serde_json::json!(["synthetic parallel phrase"])
        );
        assert!(
            owner
                .asset(&format!("synthetic-{index}.official.json"))
                .is_err()
        );
    }
    assert!(
        fixture
            .recording
            .messages()
            .iter()
            .any(|frame| frame.providers.len() == 9)
    );
}

#[test]
fn oversized_legacy_migration_preserves_whole_file_and_rejects_profile_assets() {
    let fixture = fixture();
    let owner = &fixture.owner;
    let mut bytes = b"[]".to_vec();
    bytes.resize(2 * 1024 * 1024, b' ');
    bytes.extend_from_slice(b"SYNTHETIC TAIL SENTINEL");
    owner
        .dir
        .remove_file(&format!("{}.{}.json", "claude", "custom"))
        .ok();
    owner.dir.replace("claude.json", &bytes, 0o600).unwrap();
    assert!(owner.sync().is_err());
    assert_eq!(
        owner.dir.read("claude.json", bytes.len() + 1).unwrap(),
        bytes
    );
    assert!(
        owner
            .dir
            .metadata(&format!("{}.{}.json", "claude", "custom"))
            .is_err()
    );
    assert!(owner.asset("claude.json").is_err());
    owner
        .dir
        .replace(&format!("{}.{}.json", "claude", "official"), &bytes, 0o600)
        .unwrap();
    assert!(owner.read("claude", "official").is_err());
    assert!(owner.copy_official("claude").is_err());
    assert_eq!(
        owner
            .dir
            .read(
                &format!("{}.{}.json", "claude", "official"),
                bytes.len() + 1
            )
            .unwrap(),
        bytes
    );
}
