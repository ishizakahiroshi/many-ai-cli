//! Real registration/child worker callers with synthetic core and transport.
//! No process, provider, account, notification or network is used.
use super::*;
use crate::{
    config::Config,
    hub::task_owner::HubTaskOwner,
    orchestration::{
        child_launch::{ChildPreparation, RegisteredChild},
        initial_prompt::InitialPromptNotice,
    },
    terminal::{
        events::CoreEventBus,
        journal::{JournalOptions, SessionJournal},
        session::EngineOptions,
    },
};
use std::sync::Mutex;

#[derive(Default)]
struct SyntheticIo {
    frames: Mutex<Vec<Vec<u8>>>,
}
impl WrapperTransport for SyntheticIo {
    fn send<'a>(
        &'a self,
        _: SessionBinding,
        message: proto::Message,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            self.frames.lock().unwrap().push(message.data);
            Ok(())
        })
    }
}
impl CoreEffectSink for SyntheticIo {
    fn apply<'a>(&'a self, effects: CoreEffects) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
        Box::pin(async move {
            // Persistence is disabled. Drop ordered effects rather than retaining
            // their tickets and preventing the next real core registration.
            drop(effects);
            Ok(())
        })
    }
}
impl WrappedSessionSpawner for SyntheticIo {
    fn spawn_and_wait<'a>(
        &'a self,
        _: WrappedSpawnSpec,
        _: Duration,
        _: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome> {
        Box::pin(async { SpawnWaitOutcome::Failed("synthetic fixture forbids providers".into()) })
    }
}
struct Fixture {
    _root: tempfile::TempDir,
    cwd: PathBuf,
    core: Arc<SessionEngine>,
    workers: Arc<SessionWorkers>,
    owner: HubTaskOwner,
    io: Arc<SyntheticIo>,
    binding: SessionBinding,
}
impl Fixture {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let cwd = root.path().join("trial");
        std::fs::create_dir(&cwd).unwrap();
        let paths = RuntimePaths::trial(&cwd, 49679, &root.path().join("installed")).unwrap();
        let owner = HubTaskOwner::new(tokio::runtime::Handle::current());
        let io = Arc::new(SyntheticIo::default());
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
        let registered = core
            .register(
                RegisterRequest {
                    message: proto::Message {
                        provider: "claude".into(),
                        cwd: cwd.to_string_lossy().into_owned(),
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
            .unwrap();
        let binding = registered.binding;
        io.apply(registered.after_registered).await.unwrap();
        let workers = SessionWorkers::new(
            Arc::new(ConfigStore::new(paths.clone(), Config::default()).unwrap()),
            paths.clone(),
            Arc::new(FilesService::new(cwd.clone(), paths)),
            owner.handle(),
            Arc::new(|_, _| {}),
        );
        workers
            .bind(Arc::downgrade(&core), Arc::downgrade(&effects))
            .unwrap();
        Self {
            _root: root,
            cwd,
            core,
            workers,
            owner,
            io,
            binding,
        }
    }
    fn request(&self, binding: SessionBinding) -> InitialPromptRequest {
        InitialPromptRequest {
            binding,
            prompt: "synthetic initial task".into(),
            notice: InitialPromptNotice::default(),
        }
    }
    fn child(&self, binding: SessionBinding) -> RegisteredChild {
        RegisteredChild {
            binding,
            parent: binding,
            role: "review".into(),
            preparation: ChildPreparation {
                orchestration: OrchestrationId("synthetic".into()),
                board_path: self.cwd.join("board.md"),
                absolute_cwd: self.cwd.clone(),
                child_cwd: self.cwd.clone(),
                branch: String::new(),
            },
            restart_spec: WrappedSpawnSpec {
                registration_metadata: SpawnRegistrationMetadata::default(),
                spawn_attempt: None,
                registration_proof: None,
                provider: "claude".into(),
                cwd: self.cwd.clone(),
                model: String::new(),
                model_selection: String::new(),
                risk_confirmed: false,
                label: String::new(),
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
                grants: InternalSpawnGrants::from_config(Vec::new()),
                cancellation: TaskCancellation::default(),
            },
            initial_prompt: "synthetic initial task".into(),
            prompt_via_launch_arg: false,
            inject_after_registration: Some("synthetic rendered task".into()),
            spawned_at: Timestamp::now(),
        }
    }
    async fn reattach(&self, old: SessionBinding) -> SessionBinding {
        let attached = self
            .core
            .reattach(
                ReattachRequest {
                    restored_metadata: None,
                    message: proto::Message {
                        session_id: old.session.0,
                        provider: "claude".into(),
                        cwd: self.cwd.to_string_lossy().into_owned(),
                        pid: 12,
                        cols: 80,
                        rows: 24,
                        ..Default::default()
                    },
                },
                WrapperConnectionId(old.wrapper.0 + 1),
                Timestamp::now(),
            )
            .await
            .unwrap();
        let current = attached.binding;
        self.io.apply(attached.after_reattached).await.unwrap();
        current
    }
    async fn input(&self, binding: SessionBinding) -> InputDisposition {
        self.core
            .submit(
                binding,
                InputRequest {
                    bytes: b"synthetic manual input".to_vec(),
                    authority: InputAuthority::Internal,
                },
                Timestamp::now(),
                &TaskCancellation::default(),
            )
            .await
            .disposition
    }
}

#[tokio::test]
async fn retired_prompt_callers_return_failure_and_preserve_replacement_gate() {
    let f = Fixture::new().await;
    drop(f.core.dismiss(f.binding.session, Timestamp::now()).unwrap());
    let current = f.reattach(f.binding).await;
    assert_ne!(current.incarnation, f.binding.incarnation);
    f.core.set_initial_gate(current, Timestamp::now()).unwrap();
    for result in [
        f.workers.enqueue_initial_request(f.request(f.binding)),
        f.workers.enqueue_child_prompt(&f.child(f.binding)),
    ] {
        assert!(matches!(result, Err(SessionError::StaleBinding)));
    }
    assert!(matches!(
        f.input(current).await,
        InputDisposition::Deferred {
            reason: DeferredReason::InitialPrompt,
            ..
        }
    ));
    assert!(f.io.frames.lock().unwrap().is_empty());
    assert_eq!(f.owner.snapshot().effects.running, 0);
}

#[tokio::test]
async fn stale_wrapper_start_is_propagated_and_releases_only_its_incarnation_gate() {
    let f = Fixture::new().await;
    let current = f.reattach(f.binding).await;
    assert_eq!(current.incarnation, f.binding.incarnation);
    f.core.set_initial_gate(current, Timestamp::now()).unwrap();
    assert!(matches!(
        f.workers.enqueue_initial_request(f.request(f.binding)),
        Err(SessionError::StaleBinding)
    ));
    assert!(matches!(
        f.input(current).await,
        InputDisposition::TransportWritten { .. }
    ));
    assert_eq!(f.io.frames.lock().unwrap().len(), 1);
    assert_eq!(f.owner.snapshot().effects.running, 0);
}

#[tokio::test]
async fn registration_hook_propagates_binding_retired_during_prompt_selection() {
    let f = Fixture::new().await;
    let core = f.core.clone();
    let binding = f.binding;
    f.workers
        .set_conductor_renderer(Arc::new(move |_| {
            // Registration validation already passed; reproduce a disconnect
            // before the actual InitialPromptDriver owns delivery.
            drop(core.disconnected(binding, Timestamp::now())?);
            Ok("synthetic conductor task".into())
        }))
        .unwrap();
    f.core.set_initial_gate(binding, Timestamp::now()).unwrap();
    let result = f.workers.registered(
        binding,
        &SpawnRegistrationMetadata {
            orchestration: OrchestrationId("synthetic".into()),
            ..Default::default()
        },
    );
    assert!(matches!(result, Err(SessionError::StaleBinding)));
    assert!(f.io.frames.lock().unwrap().is_empty());
    assert_eq!(f.owner.snapshot().effects.running, 0);
    let current = f.reattach(binding).await;
    assert!(matches!(
        f.input(current).await,
        InputDisposition::TransportWritten { .. }
    ));
}

#[tokio::test]
async fn all_prompt_callers_clear_owned_gate_when_effect_admission_is_closed() {
    let f = Fixture::new().await;
    f.owner.stop_requests();
    f.owner.drain_requests().await;
    f.owner.stop_effects().unwrap();
    for caller in 0..3 {
        f.core
            .set_initial_gate(f.binding, Timestamp::now())
            .unwrap();
        let result = match caller {
            0 => f.workers.enqueue_initial_request(f.request(f.binding)),
            1 => f.workers.enqueue_child_prompt(&f.child(f.binding)),
            _ => f.workers.registered(
                f.binding,
                &SpawnRegistrationMetadata {
                    initial_prompt: "synthetic initial task".into(),
                    ..Default::default()
                },
            ),
        };
        assert!(matches!(result, Err(SessionError::Shutdown)));
        // No future was admitted, but the synchronous guard must still release
        // the gate. A manual input must not sit behind undeliverable work.
        assert!(matches!(
            f.input(f.binding).await,
            InputDisposition::TransportWritten { .. }
        ));
    }
    assert_eq!(f.io.frames.lock().unwrap().len(), 3);
    let effects = f.owner.snapshot().effects;
    assert_eq!(effects.running, 0);
    assert_eq!(effects.unstarted, 0);
    assert_eq!(effects.completed, 0);
}
