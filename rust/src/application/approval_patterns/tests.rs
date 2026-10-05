use super::*;
use crate::{
    application::slash_commands::{SearchContext, SlashCmd, SlashIo},
    config::Config,
    process::Cancellation,
    proto::core::*,
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
#[derive(Default)]
struct Sink(Mutex<Vec<crate::proto::Message>>);
impl CoreEffectSink for Sink {
    fn apply<'a>(&'a self, effects: CoreEffects) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
        Box::pin(async move {
            for effect in effects.0 {
                match effect {
                    CoreEffect::Broadcast(message) => self.0.lock().unwrap().push(message),
                    _ => panic!("unexpected effect"),
                }
            }
            Ok(())
        })
    }
}
fn fixture() -> (tempfile::TempDir, tempfile::TempDir, Arc<ApprovalPatterns>) {
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49687, installed.path()).unwrap();
    let mut config = Config::default();
    config.approval_pattern_sources.claude = "synthetic://claude".into();
    let config = Arc::new(ConfigStore::new(paths.clone(), config).unwrap());
    let owner = ApprovalPatterns::new(&paths, config, Weak::new(), Arc::new(|_| {})).unwrap();
    owner.sync().unwrap();
    (root, installed, owner)
}
#[tokio::test]
async fn remote_failure_empty_and_unchanged_preserve_profiles_and_only_changes_publish() {
    let (_root, _installed, owner) = fixture();
    let source = Arc::new(Source {
        body: Mutex::new(None),
        pending: false,
    });
    let sink = Sink::default();
    let cancel = Cancellation::default();
    let initial = owner
        .asset(&format!("{}.{}.json", "claude", "official"))
        .unwrap();
    owner.remote_sync(source.clone(), &cancel, &sink).await;
    *source.body.lock().unwrap() = Some(b"ordinary text\n- no phrase\n".to_vec());
    owner.remote_sync(source.clone(), &cancel, &sink).await;
    assert_eq!(
        owner
            .asset(&format!("{}.{}.json", "claude", "official"))
            .unwrap(),
        initial
    );
    assert!(sink.0.lock().unwrap().is_empty());
    *source.body.lock().unwrap() =
        Some(b"- `synthetic exact phrase`\n* `synthetic exact phrase`\n".to_vec());
    owner.remote_sync(source.clone(), &cancel, &sink).await;
    assert_eq!(
        owner.active().unwrap()["claude"],
        vec!["synthetic exact phrase"]
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&owner.asset("claude.json").unwrap()).unwrap(),
        serde_json::json!(["synthetic exact phrase"])
    );
    assert_eq!(
        sink.0.lock().unwrap()[0].r#type,
        "approval_patterns_updated"
    );
    assert_eq!(sink.0.lock().unwrap()[0].providers, vec!["claude"]);
    owner.remote_sync(source, &cancel, &sink).await;
    assert_eq!(sink.0.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn cancellation_drops_pending_fetches_without_overwriting_private_mirrors() {
    let (_root, _installed, owner) = fixture();
    let before = owner.asset("claude.json").unwrap();
    let sink = Sink::default();
    let cancel = Cancellation::default();
    let source = Arc::new(Source {
        body: Mutex::new(None),
        pending: true,
    });
    let operation = owner.remote_sync(source, &cancel, &sink);
    tokio::pin!(operation);
    tokio::select! {_=&mut operation=>panic!("pending source completed"),_=tokio::task::yield_now()=>{}}
    cancel.cancel();
    tokio::time::timeout(std::time::Duration::from_secs(1), operation)
        .await
        .unwrap();
    assert_eq!(owner.asset("claude.json").unwrap(), before);
    assert!(sink.0.lock().unwrap().is_empty());
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
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49689, installed.path()).unwrap();
    let mut config = Config::default();
    config.custom_providers = (0..9)
        .map(|index| crate::config::CustomProvider {
            id: format!("synthetic-{index}"),
            command: "synthetic.exe".into(),
            approval_pattern_source: format!("synthetic://{index}"),
            ..Default::default()
        })
        .collect();
    let config = Arc::new(ConfigStore::new(paths.clone(), config).unwrap());
    let owner = ApprovalPatterns::new(&paths, config, Weak::new(), Arc::new(|_| {})).unwrap();
    owner.sync().unwrap();
    let before = tokio::time::Instant::now();
    let sink = Sink::default();
    owner
        .remote_sync(Arc::new(DelayedSource), &Cancellation::default(), &sink)
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
        sink.0
            .lock()
            .unwrap()
            .iter()
            .any(|frame| frame.providers.len() == 9)
    );
}

#[test]
fn oversized_legacy_migration_preserves_whole_file_and_rejects_profile_assets() {
    let (_root, _installed, owner) = fixture();
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
