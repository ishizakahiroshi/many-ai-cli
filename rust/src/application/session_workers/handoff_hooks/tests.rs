use super::*;
use crate::{
    config::Config,
    hub::task_owner::HubTaskOwner,
    terminal::{
        events::CoreEventBus,
        journal::{JournalOptions, SessionJournal},
        session::EngineOptions,
    },
};
use std::sync::Mutex;
#[derive(Default)]
struct Io {
    effects: Mutex<Vec<CoreEffect>>,
}
impl WrapperTransport for Io {
    fn send<'a>(
        &'a self,
        _: SessionBinding,
        _: proto::Message,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async { Ok(()) })
    }
}
impl CoreEffectSink for Io {
    fn apply<'a>(&'a self, effects: CoreEffects) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
        Box::pin(async move {
            self.effects.lock().unwrap().extend(effects.0);
            Ok(())
        })
    }
}
impl WrappedSessionSpawner for Io {
    fn spawn_and_wait<'a>(
        &'a self,
        _: WrappedSpawnSpec,
        _: Duration,
        _: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome> {
        Box::pin(async { SpawnWaitOutcome::Failed("synthetic test forbids providers".into()) })
    }
}
#[tokio::test]
async fn memo_receipt_checks_file_then_records_only_path_without_reading_body() {
    let root = tempfile::tempdir().unwrap();
    let runtime = root.path().join("trial");
    std::fs::create_dir(&runtime).unwrap();
    let paths = RuntimePaths::trial(&runtime, 49678, &root.path().join("installed")).unwrap();
    let tasks = HubTaskOwner::new(tokio::runtime::Handle::current());
    let io = Arc::new(Io::default());
    let effects: Arc<dyn CoreEffectSink> = io.clone();
    let core = Arc::new(SessionEngine::new(
        EngineOptions::default(),
        Arc::new(SessionJournal::new(
            paths.clone(),
            None,
            JournalOptions::default(),
        )),
        io.clone(),
        effects.clone(),
        io.clone(),
        CoreEventBus::new(16).unwrap(),
    ));
    let binding = core
        .register(
            RegisterRequest {
                message: proto::Message {
                    provider: "claude".into(),
                    cwd: runtime.to_string_lossy().into_owned(),
                    pid: 12,
                    cols: 80,
                    rows: 24,
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
        Arc::new(ConfigStore::new(paths.clone(), Config::default()).unwrap()),
        paths.clone(),
        Arc::new(FilesService::new(runtime, paths.clone())),
        tasks.handle(),
        Arc::new(|_, _| {}),
    );
    workers
        .bind(Arc::downgrade(&core), Arc::downgrade(&effects))
        .unwrap();
    let store = HandoffStore::new(paths);
    let path = store.note_path_for(binding.session.0).unwrap();
    workers.handoff_note_written(binding, &path).await.unwrap();
    assert!(store.read_session(binding.session.0).unwrap().is_empty());
    assert!(
        matches!(io.effects.lock().unwrap().last(),Some(CoreEffect::Broadcast(message)) if message.r#type=="handoff_note" && !message.note_ok && message.note_path.is_empty())
    );
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    // Invalid UTF-8 body proves this route records filesystem identity only.
    std::fs::write(&path, b"\xff\xfePRIVATE-MEMO-BODY-SENTINEL").unwrap();
    workers.handoff_note_written(binding, &path).await.unwrap();
    let records = store.read_session(binding.session.0).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].kind, crate::orchestration::handoff::KIND_NOTE);
    assert_eq!(records[0].note, path.to_string_lossy());
    assert!(
        !serde_json::to_string(&records)
            .unwrap()
            .contains("PRIVATE-MEMO-BODY-SENTINEL")
    );
    assert!(
        matches!(io.effects.lock().unwrap().last(),Some(CoreEffect::Broadcast(message)) if message.r#type=="handoff_note" && message.note_ok && message.note_path==path.to_string_lossy())
    );
}
