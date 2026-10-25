//! A real core replaces the wrapper after attachment creation but before its
//! current-owner check. The attachment callback only schedules that real race.
use super::*;
use crate::{
    proto::{self, core::*},
    terminal::{
        events::CoreEventBus,
        journal::{JournalOptions, SessionJournal},
        session::{EngineOptions, SessionEngine},
    },
};
use std::sync::{Arc, Mutex};

struct NoProcesses;
impl WrapperTransport for NoProcesses {
    fn send<'a>(
        &'a self,
        _: SessionBinding,
        _: proto::Message,
    ) -> CoreFuture<'a, std::result::Result<(), SessionError>> {
        panic!("attachment replacement tests never send provider input")
    }
}
impl WrappedSessionSpawner for NoProcesses {
    fn spawn_and_wait<'a>(
        &'a self,
        _: WrappedSpawnSpec,
        _: Duration,
        _: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome> {
        panic!("attachment replacement tests never start a provider")
    }
}
struct JournalEffects(Arc<SessionJournal>);
impl CoreEffectSink for JournalEffects {
    fn apply<'a>(
        &'a self,
        effects: CoreEffects,
    ) -> CoreFuture<'a, std::result::Result<(), CoreEffectFailure>> {
        Box::pin(async move {
            for (index, effect) in effects.0.into_iter().enumerate() {
                let result = match effect {
                    CoreEffect::Persist(effect) => self.0.apply(effect),
                    CoreEffect::PersistBound {
                        binding,
                        scope,
                        effect,
                        order,
                    } => {
                        order.wait().await;
                        let result = self.0.apply_bound(binding, scope, effect);
                        drop(order);
                        result
                    }
                    // No socket, OS process or notification backend exists.
                    _ => Ok(()),
                };
                result.map_err(|error| CoreEffectFailure { index, error })?;
            }
            Ok(())
        })
    }
}
struct AttachmentHistory {
    journal: Arc<SessionJournal>,
    calls: Mutex<Vec<(SessionBinding, PersistenceBindingScope)>>,
}
impl PersistenceEffectSink for AttachmentHistory {
    fn apply(&self, _: PersistenceEffect) -> std::result::Result<(), SessionError> {
        panic!("attachment history must use exact-wrapper persistence")
    }
    fn apply_bound(
        &self,
        binding: SessionBinding,
        scope: PersistenceBindingScope,
        effect: PersistenceEffect,
    ) -> std::result::Result<(), SessionError> {
        assert!(matches!(&effect, PersistenceEffect::Event { event, .. }
            if event.0["type"] == "attach"));
        self.calls.lock().unwrap().push((binding, scope));
        self.journal.apply_bound(binding, scope, effect)
    }
}
struct Fixture {
    _temp: tempfile::TempDir,
    service: FilesService,
    journal: Arc<SessionJournal>,
    core: Arc<SessionEngine>,
    sink: Arc<JournalEffects>,
    history: Arc<AttachmentHistory>,
    initial: SessionBinding,
    existing: PathBuf,
}
impl Fixture {
    async fn new() -> Self {
        let (temp, service, _) = setup();
        let journal = Arc::new(SessionJournal::new(
            service.paths.clone(),
            None,
            JournalOptions {
                session_enabled: true,
                max_bytes: 0,
            },
        ));
        let sink = Arc::new(JournalEffects(journal.clone()));
        let core = Arc::new(SessionEngine::new(
            EngineOptions::default(),
            journal.clone(),
            Arc::new(NoProcesses),
            sink.clone(),
            Arc::new(NoProcesses),
            CoreEventBus::new(32).unwrap(),
        ));
        let registered = tokio::time::timeout(
            Duration::from_secs(5),
            core.register(
                RegisterRequest {
                    message: proto::Message {
                        provider: "shell".into(),
                        cwd: service.hub_cwd.to_string_lossy().into_owned(),
                        pid: 7,
                        cols: 120,
                        rows: 30,
                        ..Default::default()
                    },
                    spawn_proof: None,
                },
                WrapperConnectionId(1),
                crate::proto::time::Timestamp::now(),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        let initial = registered.binding;
        tokio::time::timeout(
            Duration::from_secs(5),
            sink.apply(registered.after_registered),
        )
        .await
        .unwrap()
        .unwrap();
        let history = Arc::new(AttachmentHistory {
            journal: journal.clone(),
            calls: Mutex::new(Vec::new()),
        });
        let service = service.with_history(history.clone());
        let directory = service
            .paths
            .resource(crate::config::Resource::Attachments)
            .join(initial.session.0.to_string());
        fs::create_dir_all(&directory).unwrap();
        let existing = directory.join("existing.txt");
        fs::write(&existing, b"preserve preexisting attachment").unwrap();
        Self {
            _temp: temp,
            service,
            journal,
            core,
            sink,
            history,
            initial,
            existing,
        }
    }
    fn created_file(&self) -> PathBuf {
        let created = fs::read_dir(self.existing.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path != &self.existing)
            .collect::<Vec<_>>();
        assert_eq!(created.len(), 1, "the save created exactly one new file");
        created.into_iter().next().unwrap()
    }
    fn replace_inside_save(&self) -> SessionBinding {
        assert!(self.core.is_current(self.initial));
        let started = self.core.snapshot(self.initial.session).unwrap().started_at;
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let replacement = tokio::time::timeout(
                    Duration::from_secs(5),
                    self.core.reattach(
                        ReattachRequest {
                            restored_metadata: None,
                            message: proto::Message {
                                session_id: self.initial.session.0,
                                provider: "shell".into(),
                                cwd: self.service.hub_cwd.to_string_lossy().into_owned(),
                                pid: 7,
                                started_at: started,
                                cols: 120,
                                rows: 30,
                                ..Default::default()
                            },
                        },
                        WrapperConnectionId(2),
                        crate::proto::time::Timestamp::now(),
                    ),
                )
                .await
                .unwrap()
                .unwrap();
                assert_eq!(replacement.binding.session, self.initial.session);
                assert_eq!(replacement.binding.incarnation, self.initial.incarnation);
                assert_ne!(replacement.binding.wrapper, self.initial.wrapper);
                tokio::time::timeout(
                    Duration::from_secs(5),
                    self.sink.apply(replacement.after_reattached),
                )
                .await
                .unwrap()
                .unwrap();
                replacement.binding
            })
        })
    }
    fn attachment_records(&self) -> Vec<Value> {
        let path = self.core.snapshot(self.initial.session).unwrap().jsonl_path;
        fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .filter(|record| record["type"] == "attach")
            .collect()
    }
    fn assert_replacement_untouched(&self, replacement: SessionBinding) {
        assert!(!self.core.is_current(self.initial));
        assert!(self.core.is_current(replacement));
        let current = self.core.details(replacement.session).unwrap();
        assert_eq!(current.binding, replacement);
        assert!(current.connected);
        assert_eq!(
            fs::read(&self.existing).unwrap(),
            b"preserve preexisting attachment"
        );
        assert!(self.history.calls.lock().unwrap().is_empty());
        assert!(self.attachment_records().is_empty());
    }
    fn positive_save(&self, replacement: SessionBinding) -> Value {
        let saved = self
            .service
            .save_attachment(
                replacement,
                "shell",
                "fresh.txt",
                b"fresh attachment",
                |binding| self.core.is_current(binding),
            )
            .unwrap();
        assert_eq!(
            fs::read(saved["saved_path"].as_str().unwrap()).unwrap(),
            b"fresh attachment"
        );
        let calls = self.history.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, replacement);
        assert!(matches!(calls[0].1, PersistenceBindingScope::ExactWrapper));
        let records = self.attachment_records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["path"], saved["saved_path"]);
        saved
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.journal.close_all();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_core_replacement_during_attachment_save_rolls_back_only_new_local_file() {
    let fixture = Fixture::new().await;
    let mut created = None;
    let mut replacement = None;
    let error = fixture
        .service
        .save_attachment(
            fixture.initial,
            "shell",
            "stale.txt",
            b"new stale bytes",
            |binding| {
                let path = fixture.created_file();
                assert_eq!(fs::read(&path).unwrap(), b"new stale bytes");
                created = Some(path);
                replacement = Some(fixture.replace_inside_save());
                fixture.core.is_current(binding)
            },
        )
        .unwrap_err();
    assert_eq!(error.status, 409);
    assert_eq!(value(&error)["error"], "stale_session");
    assert!(!created.unwrap().exists());
    let replacement = replacement.unwrap();
    fixture.assert_replacement_untouched(replacement);
    fixture.positive_save(replacement);
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_core_replacement_keeps_residual_when_owned_windows_handle_blocks_rollback() {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};
    let fixture = Fixture::new().await;
    let mut held = None;
    let mut created = None;
    let mut replacement = None;
    let error = fixture
        .service
        .save_attachment(
            fixture.initial,
            "shell",
            "residual.txt",
            b"residual owned bytes",
            |binding| {
                let path = fixture.created_file();
                // This handle permits reads/writes but omits FILE_SHARE_DELETE.
                // The actual rollback remove_file therefore receives an OS error.
                held = Some(
                    fs::OpenOptions::new()
                        .read(true)
                        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                        .open(&path)
                        .unwrap(),
                );
                created = Some(path);
                replacement = Some(fixture.replace_inside_save());
                fixture.core.is_current(binding)
            },
        )
        .unwrap_err();
    assert_eq!(error.status, 409);
    assert_eq!(value(&error)["error"], "stale_session");
    let residual = created.unwrap();
    assert_eq!(fs::read(&residual).unwrap(), b"residual owned bytes");
    let replacement = replacement.unwrap();
    fixture.assert_replacement_untouched(replacement);
    fixture.positive_save(replacement);
    assert_eq!(fs::read(&residual).unwrap(), b"residual owned bytes");
    drop(held);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn already_uploaded_synthetic_remote_copy_survives_real_replacement_and_local_rollback() {
    let fixture = Fixture::new().await;
    // An owned file models the copy already returned by remote upload. The
    // test starts at RemoteCopy; no SSH executable or network is involved.
    let remote_directory = fixture._temp.path().join("synthetic-remote-upload");
    fs::create_dir(&remote_directory).unwrap();
    let remote_file = remote_directory.join("already-uploaded.txt");
    fs::write(&remote_file, b"already uploaded synthetic bytes").unwrap();
    let remote = attachments::RemoteCopy {
        target: "synthetic-host".into(),
        path: remote_file.to_string_lossy().into_owned(),
    };
    let mut created = None;
    let mut replacement = None;
    let error = fixture
        .service
        .save_attachment_with(
            fixture.initial,
            "shell",
            "remote.txt",
            b"already uploaded synthetic bytes",
            Some(&remote),
            |binding| {
                created = Some(fixture.created_file());
                replacement = Some(fixture.replace_inside_save());
                fixture.core.is_current(binding)
            },
        )
        .unwrap_err();
    assert_eq!(error.status, 409);
    assert_eq!(value(&error)["error"], "stale_session");
    assert!(!created.unwrap().exists());
    assert_eq!(
        fs::read(&remote_file).unwrap(),
        b"already uploaded synthetic bytes"
    );
    let replacement = replacement.unwrap();
    fixture.assert_replacement_untouched(replacement);
    let saved = fixture
        .service
        .save_attachment_with(
            replacement,
            "shell",
            "fresh-remote.txt",
            b"already uploaded synthetic bytes",
            Some(&remote),
            |binding| fixture.core.is_current(binding),
        )
        .unwrap();
    assert_eq!(saved["remote_path"], remote.path);
    assert_eq!(saved["remote_target"], remote.target);
    assert_eq!(saved["inject"], format!("{} ", remote.path));
    assert_eq!(
        fs::read(saved["saved_path"].as_str().unwrap()).unwrap(),
        b"already uploaded synthetic bytes"
    );
    assert_eq!(
        fs::read(&remote_file).unwrap(),
        b"already uploaded synthetic bytes"
    );
    let calls = fixture.history.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, replacement);
    assert!(matches!(calls[0].1, PersistenceBindingScope::ExactWrapper));
    let records = fixture.attachment_records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["remote_path"], remote.path);
    assert_eq!(records[0]["path"], saved["saved_path"]);
}
