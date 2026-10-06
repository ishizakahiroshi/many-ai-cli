use super::*;
use crate::{
    config::{Config, RuntimePaths},
    hub::task_owner::HubTaskOwner,
    proto,
    terminal::{
        events::CoreEventBus,
        journal::{JournalOptions, SessionJournal},
        session::EngineOptions,
    },
};
use std::{
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Default)]
struct Sink {
    messages: Mutex<Vec<GitTurnNotification>>,
}
impl CoreEffectSink for Sink {
    fn apply<'a>(&'a self, effects: CoreEffects) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
        Box::pin(async move {
            for effect in effects.0 {
                if let CoreEffect::SendUiGitTurn { event, .. } = effect {
                    self.messages.lock().unwrap().push(event);
                }
            }
            Ok(())
        })
    }
}
struct Transport;
impl WrapperTransport for Transport {
    fn send<'a>(
        &'a self,
        _: SessionBinding,
        _: proto::Message,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
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
        Box::pin(async { SpawnWaitOutcome::Failed("synthetic test has no provider".into()) })
    }
}
struct Callbacks {
    handoff: HandoffStore,
    sink: Arc<Sink>,
    trace: Mutex<Vec<&'static str>>,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
    block: AtomicBool,
}
impl GitTurnCompletionCallbacks for Callbacks {
    fn routine_completed<'a>(
        &'a self,
        _: SessionBinding,
        _: &'a crate::proto::DoneSummary,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async { panic!("unexpected routine completion in Git fixture") })
    }
    fn handoff_note_written<'a>(
        &'a self,
        _: SessionBinding,
        _: &'a std::path::Path,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async { panic!("unexpected handoff note in Git fixture") })
    }
    fn before_done_publish<'a>(
        &'a self,
        _: SessionBinding,
        _: &'a proto::DoneSummary,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async { panic!("Git-only fixture must not receive DONE") })
    }
    fn inject_turn_summary<'a>(
        &'a self,
        binding: SessionBinding,
        _: i64,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            assert_eq!(
                self.handoff.read_session(binding.session.0).unwrap().len(),
                1
            );
            assert!(self.sink.messages.lock().unwrap().is_empty());
            self.trace.lock().unwrap().push("summary-after-handoff");
            Ok(())
        })
    }
    fn after_git_turn_broadcast<'a>(
        &'a self,
        _: SessionBinding,
        _: &'a GitTurnSnapshot,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            assert_eq!(self.sink.messages.lock().unwrap().len(), 1);
            self.trace.lock().unwrap().push("callback-after-broadcast");
            self.entered.notify_one();
            if self.block.load(Ordering::SeqCst) {
                self.release.notified().await;
            }
            Ok(())
        })
    }
}
struct Fixture {
    _root: tempfile::TempDir,
    cwd: PathBuf,
    files: Arc<FilesService>,
    core: Arc<SessionEngine>,
    observer: Arc<ApplicationEventObserver>,
    callbacks: Arc<Callbacks>,
    warnings: Arc<Mutex<Vec<String>>>,
    owner: HubTaskOwner,
    binding: SessionBinding,
}
async fn git(files: &FilesService, cwd: &std::path::Path, args: &[&str]) {
    let plan = ProcessPlan {
        executable: files.git_executable.clone(),
        args: args.iter().map(|arg| (*arg).into()).collect(),
        cwd: cwd.into(),
        env: files.git_environment.clone(),
        stdin: Vec::new(),
        timeout: Duration::from_secs(5),
        output_cap: 1024 * 1024,
        pipe_drain_timeout: Duration::from_secs(2),
    };
    let result = process::run_capped(&plan, &Cancellation::default())
        .await
        .unwrap();
    assert!(
        matches!(result.outcome, ExitOutcome::Exited { code: Some(0), .. }),
        "synthetic git command failed"
    );
}
async fn fixture(handoff_enabled: bool, summary: bool) -> Fixture {
    fixture_in_with_maintenance_trace(tempfile::tempdir().unwrap(), handoff_enabled, summary, None)
        .await
}
async fn fixture_in_with_maintenance_trace(
    root: tempfile::TempDir,
    handoff_enabled: bool,
    summary: bool,
    trace: Option<PathBuf>,
) -> Fixture {
    let trace_maintenance = trace.is_some();
    let cwd = root.path().join("synthetic-project");
    let runtime = root.path().join("runtime");
    std::fs::create_dir(&cwd).unwrap();
    std::fs::create_dir(&runtime).unwrap();
    let paths = RuntimePaths::trial(&runtime, 49439, &root.path().join("installed")).unwrap();
    let mut config = Config::default();
    config.handoff.enabled = Some(handoff_enabled);
    config.handoff.intent_mode = if summary { "turn-summary" } else { "done-only" }.into();
    let config = Arc::new(ConfigStore::new(paths.clone(), config).unwrap());
    let mut files = FilesService::new(cwd.clone(), paths.clone());
    let git_config = root.path().join("empty-git-config");
    std::fs::write(&git_config, "").unwrap();
    files.git_environment = std::collections::BTreeMap::from([
        (
            "GIT_CONFIG_GLOBAL".into(),
            Some(git_config.into_os_string()),
        ),
        ("GIT_CONFIG_NOSYSTEM".into(), Some("1".into())),
        ("GIT_AUTHOR_NAME".into(), Some("Synthetic Author".into())),
        (
            "GIT_AUTHOR_EMAIL".into(),
            Some("synthetic@example.invalid".into()),
        ),
        ("GIT_COMMITTER_NAME".into(), Some("Synthetic Author".into())),
        (
            "GIT_COMMITTER_EMAIL".into(),
            Some("synthetic@example.invalid".into()),
        ),
        ("GIT_INDEX_FILE".into(), None),
        ("GIT_DIR".into(), None),
        ("GIT_WORK_TREE".into(), None),
    ]);
    if let Some(trace) = trace {
        files.git_environment.insert(
            "GIT_TRACE2_EVENT".into(),
            Some(crate::config::paths::equivalent_root_prefix(&trace).into_os_string()),
        );
    }
    git(&files, &cwd, &["init", "-b", "synthetic"]).await;
    if trace_maintenance {
        // Explicit detachment makes the regression independent of ambient
        // defaults: only the fixture's later override may select foreground work.
        git(&files, &cwd, &["config", "maintenance.autoDetach", "true"]).await;
    }
    // Synchronize only fixture-owned automatic maintenance with the first capture.
    git(&files, &cwd, &["config", "maintenance.autoDetach", "false"]).await;
    std::fs::write(cwd.join("task.txt"), "before\n").unwrap();
    git(&files, &cwd, &["add", "task.txt"]).await;
    git(&files, &cwd, &["commit", "-m", "Synthetic baseline"]).await;
    let files = Arc::new(files);
    let sink = Arc::new(Sink::default());
    let core = Arc::new(SessionEngine::new(
        EngineOptions::default(),
        Arc::new(SessionJournal::new(
            paths.clone(),
            None,
            JournalOptions::default(),
        )),
        Arc::new(Transport),
        sink.clone(),
        Arc::new(NoSpawn),
        CoreEventBus::new(64).unwrap(),
    ));
    let registration = core
        .register(
            RegisterRequest {
                message: proto::Message {
                    provider: "copilot".into(),
                    cwd: cwd.to_string_lossy().into_owned(),
                    pid: 7,
                    cols: 120,
                    rows: 30,
                    ..Default::default()
                },
                spawn_proof: None,
            },
            WrapperConnectionId(1),
            Timestamp::now(),
        )
        .await
        .unwrap();
    sink.apply(registration.after_registered).await.unwrap();
    let ui = UiBinding {
        connection: UiConnectionId(1),
        auth_epoch: core.auth_epoch(),
    };
    core.attach_ui(ui, None, None).unwrap();
    sink.apply(core.finish_ui_priming(ui).unwrap())
        .await
        .unwrap();
    let callbacks = Arc::new(Callbacks {
        handoff: HandoffStore::new(paths.clone()),
        sink: sink.clone(),
        trace: Mutex::new(Vec::new()),
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
        block: AtomicBool::new(false),
    });
    let owner = HubTaskOwner::new(tokio::runtime::Handle::current());
    let warnings = Arc::new(Mutex::new(Vec::new()));
    let warning_sink = warnings.clone();
    let observer = Arc::new(ApplicationEventObserver::new(
        config,
        paths,
        files.clone(),
        callbacks.clone(),
        Arc::new(move |message, error| {
            warning_sink
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(format!("{message}: {error:?}"));
        }),
        owner.handle(),
    ));
    let effects: Arc<dyn CoreEffectSink> = sink;
    observer
        .bind(Arc::downgrade(&core), Arc::downgrade(&effects))
        .unwrap();
    Fixture {
        _root: root,
        cwd,
        files,
        core,
        observer,
        callbacks,
        warnings,
        owner,
        binding: registration.binding,
    }
}
fn event(binding: SessionBinding, end: bool) -> CoreEvent {
    let now = crate::proto::time::format_rfc3339(Timestamp::now()).unwrap();
    CoreEvent::GitTurnCapture {
        binding,
        started_at: now.clone(),
        ended_at: end.then_some(now),
    }
}

// Keep recurrence context local to the owned synthetic Git trace. Only fixed
// command categories and numeric exit/duration fields may reach a panic; never
// include Trace2 argv, environment, SID, path, or arbitrary event strings.
fn git_trace_summary(path: &std::path::Path) -> String {
    use std::io::Read;

    const MAX_BYTES: u64 = 1024 * 1024;
    const MAX_COMMANDS: usize = 32;
    let Ok(file) = std::fs::File::open(path) else {
        return "trace unavailable".into();
    };
    let mut bytes = Vec::new();
    if file.take(MAX_BYTES + 1).read_to_end(&mut bytes).is_err() {
        return "trace unreadable".into();
    }
    if bytes.len() as u64 > MAX_BYTES {
        return "trace exceeds diagnostic limit".into();
    }
    let trace = String::from_utf8_lossy(&bytes);
    let mut malformed = 0usize;
    let mut recent = Vec::new();
    for line in trace
        .split_inclusive('\n')
        .filter(|line| line.ends_with('\n'))
    {
        let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
            malformed += 1;
            continue;
        };
        let Some(sid) = event["sid"].as_str() else {
            continue;
        };
        match event["event"].as_str() {
            Some("cmd_name") => {
                let category = match event["name"].as_str() {
                    Some("init") => "init",
                    Some("config") => "config",
                    Some("add") => "add",
                    Some("commit") => "commit",
                    Some("maintenance") => "maintenance",
                    Some("rev-parse") => "rev-parse",
                    Some("read-tree") => "read-tree",
                    Some("write-tree") => "write-tree",
                    Some("diff") => "diff",
                    Some("status") => "status",
                    Some("ls-files") => "ls-files",
                    _ => "other",
                };
                recent.push((sid.to_owned(), category, None::<i64>, None::<f64>));
                if recent.len() > MAX_COMMANDS {
                    recent.remove(0);
                }
            }
            Some("exit") => {
                if let Some(row) = recent.iter_mut().rfind(|row| row.0 == sid) {
                    row.2 = event["code"].as_i64();
                    row.3 = event["t_abs"].as_f64();
                }
            }
            _ => {}
        }
    }
    let commands = recent
        .into_iter()
        .map(|(_, category, exit, seconds)| (category, exit, seconds))
        .collect::<Vec<_>>();
    format!(
        "complete={}; malformed={malformed}; recent_commands(category, exit, seconds)={commands:?}",
        trace.ends_with('\n'),
    )
}

#[test]
fn git_trace_summary_omits_unlisted_fields_and_bounds_context() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("synthetic-trace.json");
    let private = "synthetic-private-argv-env-path-sid";
    let mut recorded = [
        serde_json::json!({
            "event": "cmd_name", "sid": private, "name": private,
            "argv": [private], "env": {"private": private}, "path": private,
        }),
        serde_json::json!({
            "event": "exit", "sid": private, "code": 23, "t_abs": 1.25,
            "argv": [private], "path": private,
        }),
        serde_json::json!({
            "event": "cmd_name", "sid": "unfinished", "name": "write-tree",
        }),
    ]
    .into_iter()
    .map(|event| format!("{event}\n"))
    .collect::<String>();
    recorded.push_str("malformed complete record\n");
    recorded.push_str(private);
    std::fs::write(&path, recorded).unwrap();
    let summary = git_trace_summary(&path);
    assert!(!summary.contains(private));
    assert_eq!(
        summary,
        "complete=false; malformed=1; recent_commands(category, exit, seconds)=[(\"other\", Some(23), Some(1.25)), (\"write-tree\", None, None)]"
    );

    let mut recorded = String::new();
    for number in 0..40 {
        let sid = format!("synthetic-{number}");
        for event in [
            serde_json::json!({"event": "cmd_name", "sid": sid, "name": "write-tree"}),
            serde_json::json!({"event": "exit", "sid": sid, "code": number, "t_abs": 0.5}),
        ] {
            recorded.push_str(&format!("{event}\n"));
        }
    }
    std::fs::write(&path, recorded).unwrap();
    let summary = git_trace_summary(&path);
    assert_eq!(summary.matches("(\"write-tree\",").count(), 32);
    assert!(summary.contains("(\"write-tree\", Some(8), Some(0.5))"));
    assert!(summary.contains("(\"write-tree\", Some(39), Some(0.5))"));
    assert!(!summary.contains("(\"write-tree\", Some(0),"));
    assert!(!summary.contains("synthetic-"));

    std::fs::write(&path, vec![b' '; 1024 * 1024 + 1]).unwrap();
    assert_eq!(git_trace_summary(&path), "trace exceeds diagnostic limit");
}

#[tokio::test]
async fn fixture_git_keeps_auto_maintenance_enabled_and_waits_for_completion() {
    let root = tempfile::tempdir().unwrap();
    let trace = root.path().join("maintenance-trace.json");
    let f = fixture_in_with_maintenance_trace(root, false, false, Some(trace.clone())).await;
    let recorded = std::fs::read_to_string(&trace).expect("read synthetic Git Trace2 events");
    let mut detach_flags = Vec::new();
    // The RED control may still have a detached writer. Read complete records:
    // commit's required child_start precedes its return. Only booleans are shown.
    for line in recorded
        .split_inclusive('\n')
        .filter(|line| line.ends_with('\n'))
    {
        let event: serde_json::Value =
            serde_json::from_str(line).expect("synthetic Git Trace2 JSON");
        if event["event"].as_str() != Some("child_start") {
            continue;
        }
        let Some(argv) = event["argv"].as_array() else {
            continue;
        };
        if argv.windows(3).any(|args| {
            args[0].as_str() == Some("maintenance")
                && args[1].as_str() == Some("run")
                && args[2].as_str() == Some("--auto")
        }) {
            detach_flags.push((
                argv.iter().any(|arg| arg.as_str() == Some("--detach")),
                argv.iter().any(|arg| arg.as_str() == Some("--no-detach")),
            ));
        }
    }
    assert_eq!(
        detach_flags,
        vec![(false, true)],
        "actual auto-maintenance must remain enabled and run without detaching"
    );
    assert!(
        recorded.ends_with('\n'),
        "completed foreground trace must be complete"
    );
    let lock = f.cwd.join(".git").join("objects").join("maintenance.lock");
    assert!(
        matches!(std::fs::symlink_metadata(lock), Err(error) if error.kind() == std::io::ErrorKind::NotFound),
        "owned automatic maintenance must release its lock before fixture commit returns"
    );
    // Exercise the actual snapshot owner immediately, with its unchanged
    // five-second resolution/materialization budgets and no retry or mock Git.
    f.observer.observe(&event(f.binding, false)).await.unwrap();
    let reservation = f.files.reserve_git_turn_end(f.binding);
    assert!(
        reservation.is_some(),
        "initial real snapshot did not establish a reservation; git_trace={}",
        git_trace_summary(&trace),
    );
    assert!(f.warnings.lock().unwrap().is_empty());
    drop(reservation);
}

#[tokio::test]
async fn actual_capture_handoff_summary_broadcast_callback_order_and_next_input_barrier() {
    let f = fixture(true, true).await;
    f.observer.observe(&event(f.binding, false)).await.unwrap();
    std::fs::write(f.cwd.join("task.txt"), "after\n").unwrap();
    f.callbacks.block.store(true, Ordering::SeqCst);
    f.observer.observe(&event(f.binding, true)).await.unwrap();
    // The return did not wait for completion, but its reservation already owns
    // the Files gate. A second end cannot enqueue duplicate completion work.
    assert!(f.files.reserve_git_turn_end(f.binding).is_none());
    tokio::time::timeout(Duration::from_secs(10), f.callbacks.entered.notified())
        .await
        .unwrap();
    let observer = f.observer.clone();
    let next = event(f.binding, false);
    let start = tokio::spawn(async move { observer.observe(&next).await });
    tokio::task::yield_now().await;
    assert!(!start.is_finished());
    assert_eq!(
        *f.callbacks.trace.lock().unwrap(),
        vec!["summary-after-handoff", "callback-after-broadcast"]
    );
    let records = f
        .callbacks
        .handoff
        .read_session(f.binding.session.0)
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].kind, KIND_GIT_TURN);
    assert_eq!(records[0].files, vec!["task.txt"]);
    assert!(!records[0].commit.is_empty());
    f.callbacks.release.notify_one();
    start.await.unwrap().unwrap();
    f.owner.drain_effects().await;
    assert!(f.core.is_current(f.binding));
}

#[tokio::test]
async fn handoff_gate_is_independent_of_capture_and_summary_requires_both_gates() {
    let f = fixture(false, true).await;
    f.observer.observe(&event(f.binding, false)).await.unwrap();
    std::fs::write(f.cwd.join("task.txt"), "after\n").unwrap();
    f.observer.observe(&event(f.binding, true)).await.unwrap();
    f.owner.drain_effects().await;
    assert!(
        f.callbacks
            .handoff
            .read_session(f.binding.session.0)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        *f.callbacks.trace.lock().unwrap(),
        vec!["callback-after-broadcast"]
    );
}

#[tokio::test]
async fn unbound_observer_fails_closed_and_snapshot_failure_is_best_effort() {
    let f = fixture(true, false).await;
    let unbound = ApplicationEventObserver::new(
        f.observer.config.clone(),
        f.files.paths.clone(),
        f.files.clone(),
        f.callbacks.clone(),
        Arc::new(|_, _| {}),
        f.owner.handle(),
    );
    assert!(matches!(
        unbound.observe(&event(f.binding, false)).await,
        Err(SessionError::Shutdown)
    ));
    let missing = SessionBinding {
        session: LiveSessionId(999_999),
        ..f.binding
    };
    f.observer.observe(&event(missing, false)).await.unwrap();
    assert!(f.callbacks.trace.lock().unwrap().is_empty());
    assert!(
        f.callbacks
            .handoff
            .read_session(f.binding.session.0)
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn start_capture_materialization_has_an_independent_resolution_budget() {
    let f = fixture(false, false).await;
    f.files
        .expire_next_git_resolution
        .store(true, Ordering::SeqCst);
    f.observer.observe(&event(f.binding, false)).await.unwrap();
    assert_eq!(f.files.expired_git_resolutions.load(Ordering::SeqCst), 1);
    assert!(!f.files.expire_next_git_resolution.load(Ordering::SeqCst));
    let reservation = f.files.reserve_git_turn_end(f.binding);
    assert!(
        reservation.is_some(),
        "actual start snapshot reused the completed resolver's expired budget"
    );
    assert!(f.warnings.lock().unwrap().is_empty());
    drop(reservation);
    assert_eq!(
        std::fs::read_dir(f.files.paths.root().join("tmp"))
            .unwrap()
            .count(),
        0
    );
    assert!(f.core.is_current(f.binding));
}

#[tokio::test]
async fn end_capture_materialization_has_an_independent_resolution_budget() {
    let f = fixture(false, false).await;
    f.observer.observe(&event(f.binding, false)).await.unwrap();
    let reservation = f.files.reserve_git_turn_end(f.binding);
    assert!(reservation.is_some(), "initial actual Git baseline missing");
    drop(reservation);
    std::fs::write(f.cwd.join("task.txt"), "after\n").unwrap();
    f.files
        .expire_next_git_resolution
        .store(true, Ordering::SeqCst);
    f.observer.observe(&event(f.binding, true)).await.unwrap();
    f.owner.drain_effects().await;
    assert_eq!(f.files.expired_git_resolutions.load(Ordering::SeqCst), 1);
    assert!(!f.files.expire_next_git_resolution.load(Ordering::SeqCst));
    let messages = f.callbacks.sink.messages.lock().unwrap();
    assert_eq!(
        messages.len(),
        1,
        "actual end snapshot reused the completed resolver's expired budget"
    );
    assert_eq!(messages[0].turn, 1);
    assert_eq!(messages[0].files_changed, 1);
    assert_eq!(messages[0].added, 1);
    assert_eq!(messages[0].removed, 1);
    drop(messages);
    assert_eq!(
        *f.callbacks.trace.lock().unwrap(),
        vec!["callback-after-broadcast"]
    );
    assert!(f.warnings.lock().unwrap().is_empty());
    assert!(f.files.reserve_git_turn_end(f.binding).is_none());
    assert_eq!(
        std::fs::read_dir(f.files.paths.root().join("tmp"))
            .unwrap()
            .count(),
        0
    );
    assert!(f.core.is_current(f.binding));
}

#[tokio::test]
async fn end_reservation_release_and_warm_reconnect_preserve_the_capture_incarnation() {
    let root = tempfile::tempdir().unwrap();
    let trace = root.path().join("reconnect-git-trace.json");
    let f = fixture_in_with_maintenance_trace(root, false, false, Some(trace.clone())).await;
    f.observer.observe(&event(f.binding, false)).await.unwrap();
    let reservation = f.files.reserve_git_turn_end(f.binding).unwrap();
    assert!(f.files.reserve_git_turn_end(f.binding).is_none());
    drop(reservation);
    let reservation = f.files.reserve_git_turn_end(f.binding).unwrap();
    let wrong = SessionBinding {
        incarnation: SessionIncarnation(f.binding.incarnation.0 + 1),
        ..f.binding
    };
    assert!(f.files.reserve_git_turn_end(wrong).is_none());
    let warm = f
        .core
        .reattach(
            ReattachRequest {
                restored_metadata: None,
                message: proto::Message {
                    session_id: f.binding.session.0,
                    provider: "copilot".into(),
                    cwd: f.cwd.to_string_lossy().into_owned(),
                    pid: 7,
                    cols: 120,
                    rows: 30,
                    ..Default::default()
                },
            },
            WrapperConnectionId(2),
            Timestamp::now(),
        )
        .await
        .unwrap();
    assert_eq!(warm.binding.incarnation, f.binding.incarnation);
    assert_ne!(warm.binding.wrapper, f.binding.wrapper);
    std::fs::write(f.cwd.join("task.txt"), "after reconnect\n").unwrap();
    let ended = crate::proto::time::format_rfc3339(Timestamp::now()).unwrap();
    let completed = f
        .files
        .observe_reserved_git_turn_end(f.core.as_ref(), reservation, &ended)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(completed.snapshot.turn, 1);
    assert_eq!(completed.snapshot.files, 1);
    assert!(f.core.is_current(warm.binding));
    drop(completed);
    let warnings_before = f
        .warnings
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .len();
    f.observer
        .observe(&event(warm.binding, false))
        .await
        .unwrap();
    let reservation = f.files.reserve_git_turn_end(warm.binding);
    let warnings = f
        .warnings
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .skip(warnings_before)
        .cloned()
        .collect::<Vec<_>>();
    assert!(
        reservation.is_some(),
        "post-reconnect start did not establish a Git-turn reservation; warnings={warnings:?}; git_trace={}",
        git_trace_summary(&trace),
    );
}
