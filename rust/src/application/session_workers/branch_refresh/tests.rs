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
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{mpsc, oneshot};

#[derive(Default)]
struct SourceState {
    active: BTreeMap<String, usize>,
    peak: usize,
    per_cwd_peak: BTreeMap<String, usize>,
    calls: Vec<(String, &'static str)>,
}
struct BranchCall {
    cwd: String,
    release: oneshot::Sender<String>,
}
struct Source {
    state: Arc<Mutex<SourceState>>,
    entered: mpsc::UnboundedSender<BranchCall>,
}
struct Active {
    state: Arc<Mutex<SourceState>>,
    cwd: String,
}
impl Drop for Active {
    fn drop(&mut self) {
        let mut state = lock(&self.state);
        *state.active.get_mut(&self.cwd).unwrap() -= 1;
    }
}
impl GitBranchSource for Source {
    fn branch<'a>(&'a self, cwd: &'a str) -> CoreFuture<'a, String> {
        Box::pin(async move {
            {
                let mut state = lock(&self.state);
                let per_cwd = state.active.entry(cwd.into()).or_default();
                *per_cwd += 1;
                let active = *per_cwd;
                let peak = state.per_cwd_peak.entry(cwd.into()).or_default();
                *peak = (*peak).max(active);
                state.peak = state.peak.max(state.active.values().sum());
                state.calls.push((cwd.into(), "branch"));
            }
            let _active = Active {
                state: self.state.clone(),
                cwd: cwd.into(),
            };
            let (release, wait) = oneshot::channel();
            self.entered
                .send(BranchCall {
                    cwd: cwd.into(),
                    release,
                })
                .unwrap_or_else(|_| panic!("branch test receiver dropped"));
            wait.await.expect("test must release or cancel the lookup")
        })
    }
    fn changes<'a>(&'a self, cwd: &'a str) -> CoreFuture<'a, (i64, i64, i64)> {
        Box::pin(async move {
            lock(&self.state).calls.push((cwd.into(), "changes"));
            (1, 2, 3)
        })
    }
    fn project<'a>(&'a self, cwd: &'a str) -> CoreFuture<'a, Option<String>> {
        Box::pin(async move {
            lock(&self.state).calls.push((cwd.into(), "project"));
            Some(format!("root:{cwd}"))
        })
    }
}
struct Publication {
    release: oneshot::Sender<()>,
}
struct Sink {
    messages: Mutex<Vec<(UiBinding, proto::Message)>>,
    block_publication: AtomicBool,
    entered: mpsc::UnboundedSender<Publication>,
}
impl CoreEffectSink for Sink {
    fn apply<'a>(&'a self, effects: CoreEffects) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
        Box::pin(async move {
            let mut observed_branch = false;
            for effect in effects.0 {
                match effect {
                    CoreEffect::SendUi { binding, message }
                    | CoreEffect::SendUiBestEffort { binding, message } => {
                        observed_branch |= message.git_checked;
                        lock(&self.messages).push((binding, message));
                    }
                    CoreEffect::Broadcast(_) | CoreEffect::BroadcastGitTurn(_) => {
                        panic!("controller must publish typed UI effects, never raw broadcasts")
                    }
                    _ => {}
                }
            }
            if observed_branch && self.block_publication.load(Ordering::SeqCst) {
                let (release, wait) = oneshot::channel();
                self.entered
                    .send(Publication { release })
                    .unwrap_or_else(|_| panic!("publication test receiver dropped"));
                wait.await.expect("test must release or cancel publication");
            }
            Ok(())
        })
    }
}
struct NoIo;
impl WrapperTransport for NoIo {
    fn send<'a>(
        &'a self,
        _: SessionBinding,
        _: proto::Message,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async { panic!("branch fixture must not contact a wrapper") })
    }
}
impl WrappedSessionSpawner for NoIo {
    fn spawn_and_wait<'a>(
        &'a self,
        _: WrappedSpawnSpec,
        _: Duration,
        _: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome> {
        Box::pin(async { panic!("branch fixture must not launch a provider") })
    }
}
struct Fixture {
    owner: Arc<Owner>,
    tasks: HubTaskOwner,
    core: Arc<SessionEngine>,
    sink: Arc<Sink>,
    source: Arc<Source>,
    calls: mpsc::UnboundedReceiver<BranchCall>,
    publications: mpsc::UnboundedReceiver<Publication>,
    warnings: Arc<Mutex<Vec<String>>>,
    _workers: Arc<SessionWorkers>,
    _root: tempfile::TempDir,
}
async fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let runtime = root.path().join("runtime");
    std::fs::create_dir(&runtime).unwrap();
    let paths = RuntimePaths::trial(&runtime, 49462, &root.path().join("installed")).unwrap();
    let config = Arc::new(ConfigStore::new(paths.clone(), Config::default()).unwrap());
    let files = Arc::new(FilesService::new(root.path().into(), paths.clone()));
    let (entered, publications) = mpsc::unbounded_channel();
    let sink = Arc::new(Sink {
        messages: Mutex::new(Vec::new()),
        block_publication: AtomicBool::new(false),
        entered,
    });
    let core = Arc::new(SessionEngine::new(
        EngineOptions::default(),
        Arc::new(SessionJournal::new(
            paths.clone(),
            None,
            JournalOptions::default(),
        )),
        Arc::new(NoIo),
        sink.clone(),
        Arc::new(NoIo),
        CoreEventBus::new(64).unwrap(),
    ));
    let ui = UiBinding {
        connection: UiConnectionId(1),
        auth_epoch: core.auth_epoch(),
    };
    core.attach_ui(ui, None, None).unwrap();
    sink.apply(core.finish_ui_priming(ui).unwrap())
        .await
        .unwrap();
    let tasks = HubTaskOwner::new(tokio::runtime::Handle::current());
    let warnings = Arc::new(Mutex::new(Vec::new()));
    let warning_sink = warnings.clone();
    let workers = SessionWorkers::new(
        config,
        paths,
        files,
        tasks.handle(),
        Arc::new(move |operation, error| {
            lock(&warning_sink).push(format!("{operation}: {error:?}"));
        }),
    );
    let effects: Arc<dyn CoreEffectSink> = sink.clone();
    workers
        .bind(Arc::downgrade(&core), Arc::downgrade(&effects))
        .unwrap();
    let (entered, calls) = mpsc::unbounded_channel();
    let source = Arc::new(Source {
        state: Arc::new(Mutex::new(SourceState::default())),
        entered,
    });
    let owner = Owner::new(source.clone(), Arc::downgrade(&workers), tasks.handle());
    Fixture {
        owner,
        tasks,
        core,
        sink,
        source,
        calls,
        publications,
        warnings,
        _workers: workers,
        _root: root,
    }
}
async fn register(f: &Fixture, pid: i64, cwd: &str) -> LiveSessionId {
    let registered = f
        .core
        .register(
            RegisterRequest {
                message: proto::Message {
                    provider: "copilot".into(),
                    cwd: cwd.into(),
                    pid,
                    cols: 120,
                    rows: 30,
                    ..Default::default()
                },
                spawn_proof: None,
            },
            WrapperConnectionId(pid as u64),
            Timestamp::from_unix(1_791_158_400, 0).unwrap(),
        )
        .await
        .unwrap();
    let id = registered.binding.session;
    f.sink.apply(registered.after_registered).await.unwrap();
    lock(&f.sink.messages).clear();
    id
}
impl Fixture {
    async fn next_call(&mut self) -> BranchCall {
        tokio::time::timeout(Duration::from_secs(5), self.calls.recv())
            .await
            .expect("owned refresh did not enter the fake branch source")
            .expect("branch source closed")
    }
    async fn next_publication(&mut self) -> Publication {
        tokio::time::timeout(Duration::from_secs(5), self.publications.recv())
            .await
            .expect("owned refresh did not reach its effect sink")
            .expect("effect sink closed")
    }
    async fn drain(&self) {
        let receipt = tokio::time::timeout(Duration::from_secs(5), self.tasks.drain_effects())
            .await
            .expect("owned refreshes did not drain");
        assert_eq!(receipt.running, 0);
        assert_eq!(receipt.unstarted, 0);
        assert_eq!(receipt.panicked, 0);
        assert!(lock(&self.owner.pending).is_empty());
        assert_eq!(self.owner.slots.available_permits(), 4);
        assert!(lock(&self.warnings).is_empty());
    }
}

#[tokio::test]
async fn busy_cwd_keeps_unique_pending_ids_and_immediately_drains_new_sessions() {
    let mut f = fixture().await;
    let first = register(&f, 1, "project").await;
    f.owner.queue(Vec::new()).unwrap();
    f.owner.queue(vec![(first, " \t\n".into())]).unwrap();
    assert!(lock(&f.owner.pending).is_empty());
    assert_eq!(f.tasks.snapshot().effects.running, 0);
    f.owner
        .queue(vec![(first, " project ".into()), (first, "project".into())])
        .unwrap();
    let running = f.next_call().await;
    assert_eq!(running.cwd, "project");
    let second = register(&f, 2, "project").await;
    f.owner
        .queue(vec![
            (first, "project".into()),
            (second, "project".into()),
            (first, " project ".into()),
        ])
        .unwrap();
    f.owner.queue(vec![(second, "project".into())]).unwrap();
    assert_eq!(
        lock(&f.owner.pending).get("project"),
        Some(&vec![first, second])
    );
    assert_eq!(f.tasks.snapshot().effects.running, 1);
    running.release.send("main".into()).unwrap();
    let pending = f.next_call().await;
    assert_eq!(pending.cwd, "project");
    // No timer or new queue call is needed to start this second pass.
    assert_eq!(f.core.snapshot(first).unwrap().branch, "main");
    assert!(f.core.snapshot(second).unwrap().branch.is_empty());
    pending.release.send("feature".into()).unwrap();
    f.drain().await;
    assert_eq!(f.core.snapshot(first).unwrap().branch, "feature");
    assert_eq!(f.core.snapshot(second).unwrap().branch, "feature");
    assert_eq!(f.core.snapshot(second).unwrap().project_id, "root:project");
    let messages = lock(&f.sink.messages);
    assert_eq!(
        messages
            .iter()
            .filter(|(_, m)| m.session_id == first.0 && m.git_checked)
            .count(),
        2
    );
    assert_eq!(
        messages
            .iter()
            .filter(|(_, m)| m.session_id == second.0 && m.git_checked)
            .count(),
        1
    );
    let source = lock(&f.source.state);
    assert_eq!(source.per_cwd_peak["project"], 1);
    assert_eq!(
        source.calls,
        vec![
            ("project".into(), "branch"),
            ("project".into(), "changes"),
            ("project".into(), "project"),
            ("project".into(), "branch"),
            ("project".into(), "changes"),
            ("project".into(), "project"),
        ]
    );
}

#[tokio::test]
async fn a_cwd_waiting_for_a_slot_is_already_owned_and_coalesces_without_overlap() {
    let mut f = fixture().await;
    let first = register(&f, 1, "project").await;
    let second = register(&f, 2, "project").await;
    let occupied = f.owner.slots.clone().acquire_many_owned(4).await.unwrap();
    f.owner.queue(vec![(first, "project".into())]).unwrap();
    f.owner
        .queue(vec![
            (second, " project ".into()),
            (second, "project".into()),
            (first, "project".into()),
        ])
        .unwrap();
    assert_eq!(f.tasks.snapshot().effects.running, 1);
    assert_eq!(
        lock(&f.owner.pending).get("project"),
        Some(&vec![second, first])
    );
    assert!(f.calls.try_recv().is_err());
    drop(occupied);
    let first_pass = f.next_call().await;
    first_pass.release.send("main".into()).unwrap();
    let second_pass = f.next_call().await;
    assert_eq!(second_pass.cwd, "project");
    assert!(f.core.snapshot(second).unwrap().branch.is_empty());
    second_pass.release.send("main".into()).unwrap();
    f.drain().await;
    assert_eq!(f.core.snapshot(second).unwrap().branch, "main");
    let source = lock(&f.source.state);
    assert_eq!(source.peak, 1);
    assert_eq!(source.per_cwd_peak["project"], 1);
    assert_eq!(
        source
            .calls
            .iter()
            .filter(|(_, stage)| *stage == "branch")
            .count(),
        2
    );
}

#[tokio::test]
async fn four_slots_cover_lookup_application_and_publication_before_a_fifth_cwd_can_enter() {
    let mut f = fixture().await;
    let mut requests = Vec::new();
    for index in 0..5 {
        let cwd = format!("project-{index}");
        let id = register(&f, index + 1, &cwd).await;
        requests.push((id, cwd));
    }
    let fifth = requests.pop().unwrap();
    f.sink.block_publication.store(true, Ordering::SeqCst);
    f.owner.queue(requests).unwrap();
    let mut first_four = Vec::new();
    for _ in 0..4 {
        first_four.push(f.next_call().await);
    }
    assert_eq!(lock(&f.source.state).active.values().sum::<usize>(), 4);
    assert_eq!(f.owner.slots.available_permits(), 0);
    for call in first_four {
        call.release.send("main".into()).unwrap();
    }
    let mut publishing = Vec::new();
    for _ in 0..4 {
        publishing.push(f.next_publication().await);
    }
    assert_eq!(lock(&f.source.state).active.values().sum::<usize>(), 0);
    assert_eq!(
        f.owner.slots.available_permits(),
        0,
        "a completed lookup must retain its slot during publication"
    );
    f.owner.queue(vec![fifth.clone()]).unwrap();
    tokio::task::yield_now().await;
    assert!(
        f.calls.try_recv().is_err(),
        "the fifth cwd entered while four publications were blocked"
    );
    assert_eq!(f.tasks.snapshot().effects.running, 5);
    assert_eq!(lock(&f.owner.pending).len(), 5);
    publishing.pop().unwrap().release.send(()).unwrap();
    let fifth_call = f.next_call().await;
    assert_eq!(fifth_call.cwd, fifth.1);
    fifth_call.release.send("main".into()).unwrap();
    publishing.push(f.next_publication().await);
    for publication in publishing {
        publication.release.send(()).unwrap();
    }
    f.drain().await;
    let source = lock(&f.source.state);
    assert_eq!(source.peak, 4);
    assert!(source.per_cwd_peak.values().all(|peak| *peak == 1));
    assert_eq!(
        source
            .calls
            .iter()
            .filter(|(_, stage)| *stage == "branch")
            .count(),
        5
    );
    assert_eq!(
        lock(&f.sink.messages)
            .iter()
            .filter(|(_, message)| message.git_checked)
            .count(),
        5
    );
}

#[tokio::test]
async fn deleted_ids_still_finish_branch_and_stats_but_do_not_lookup_project_or_publish() {
    let mut f = fixture().await;
    let id = register(&f, 1, "project").await;
    f.owner.queue(vec![(id, "project".into())]).unwrap();
    let call = f.next_call().await;
    drop(f.core.dismiss(id, Timestamp::now()).unwrap());
    call.release.send("late".into()).unwrap();
    f.drain().await;
    assert!(f.core.snapshot(id).is_none());
    assert!(lock(&f.sink.messages).is_empty());
    assert_eq!(
        lock(&f.source.state).calls,
        vec![("project".into(), "branch"), ("project".into(), "changes")]
    );
}

#[tokio::test]
async fn shutdown_cancels_running_and_waiting_cwds_and_releases_pending_ownership() {
    let mut f = fixture().await;
    let mut requests = Vec::new();
    for index in 0..5 {
        let cwd = format!("project-{index}");
        let id = register(&f, index + 1, &cwd).await;
        requests.push((id, cwd));
    }
    f.owner.queue(requests.clone()).unwrap();
    let mut running = Vec::new();
    for _ in 0..4 {
        running.push(f.next_call().await);
    }
    f.owner.queue(requests.clone()).unwrap();
    assert_eq!(lock(&f.owner.pending).len(), 5);
    assert!(lock(&f.owner.pending).values().all(|ids| ids.len() == 1));
    f.tasks.stop_requests();
    f.tasks.stop_effects().unwrap();
    f.tasks.cancel_effects().unwrap();
    f.drain().await;
    assert!(running.iter().all(|call| call.release.is_closed()));
    assert_eq!(lock(&f.source.state).active.values().sum::<usize>(), 0);
    assert!(f.calls.try_recv().is_err());
    assert!(lock(&f.sink.messages).is_empty());
    assert!(matches!(
        f.owner.queue(requests),
        Err(SessionError::Shutdown)
    ));
    assert!(lock(&f.owner.pending).is_empty());
}

#[tokio::test]
async fn aborting_an_unpolled_refresh_releases_its_precreated_cwd_guard() {
    let mut f = fixture().await;
    let id = register(&f, 1, "project").await;
    f.owner.queue(vec![(id, "project".into())]).unwrap();
    assert!(lock(&f.owner.pending).contains_key("project"));
    // The current-thread runtime cannot poll the admitted task before this
    // synchronous stop/abort sequence. Cleanup must already belong to the task.
    f.tasks.stop_requests();
    f.tasks.stop_effects().unwrap();
    f.tasks.abort_effects().unwrap();
    f.drain().await;
    assert!(f.calls.try_recv().is_err());
    assert!(lock(&f.source.state).calls.is_empty());
    assert_eq!(f.tasks.snapshot().effects.cancelled, 1);
}
