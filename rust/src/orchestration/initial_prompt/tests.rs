//! Synthetic VT and owned transport fixtures only; no provider process/network.
//! Async tests use a paused Tokio clock so scheduled PTY redraws and submission
//! deadlines keep their ordering independently of the host timer resolution.
use super::*;
use crate::{
    config::RuntimePaths,
    hub::task_owner::HubTaskOwner,
    proto,
    terminal::{
        events::CoreEventBus,
        journal::{JournalOptions, SessionJournal},
        session::EngineOptions,
    },
};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

const PROMPT: &str = "Read the synthetic task and finish it.";
const RULE: &str = "────────────────────────────────────────";
fn lines(text: &str) -> Vec<String> {
    text.lines().map(str::to_owned).collect()
}
fn claude_screen(composer: &str, transcript: bool) -> Vec<u8> {
    format!("\x1b[2J\x1b[HClaude fixture\r\n{}{}\r\n❯ {composer}\r\n{RULE}\r\n⏵⏵ bypass permissions on (shift+tab to cycle)\r\n", if transcript { format!("❯ {PROMPT}\r\nThinking...\r\n") } else { String::new() }, RULE).into_bytes()
}

#[test]
fn fixed_source_screen_predicates() {
    assert_eq!(screen::echo_marker(" a\u{85}b\u{3000}世界 "), "ab世界");
    assert_eq!(
        screen::echo_marker("0123456789abcdefghij"),
        "456789abcdefghij"
    );
    assert!(screen::echo_visible("[Pastedtext#12+33lines]", "absent"));
    assert!(!screen::echo_visible("[Pastedtext#x]", "absent"));
    assert_eq!(
        screen_blocker("codex", &lines("Trust this folder?\nCodex can read")),
        Some("Trustthisfolder?Codexcanread")
    );
    assert_eq!(
        screen_blocker(
            "claude",
            &lines("Is this a project you created or one you trust?")
        ),
        Some("Isthisaprojectyoucreatedoroneyoutrust?")
    );
    assert_eq!(
        screen_blocker("claude", &lines("Allow external CLAUDE.md file imports?")),
        Some("AllowexternalCLAUDE.mdfileimports?")
    );
    assert_eq!(
        screen_blocker("grok", &lines("Trust this folder? Codex can read")),
        None
    );
    let text = lines(&format!(
        "❯ prior turn\n{RULE}\n❯ wrapped task\nlast line\n{RULE}\nfooter"
    ));
    assert_eq!(
        screen::composer_text("claude", &text).as_deref(),
        Some("wrappedtasklastline")
    );
    assert_eq!(screen::composer_text("codex", &text), None);
    assert_eq!(screen::composer_text("claude", &lines("splash only")), None);
    let clear = lines(&format!(
        "❯ prior task\n{RULE}\n❯ \n{RULE}\n⏵⏵ auto mode on (shift+tab to cycle)"
    ));
    assert_eq!(screen::composer_text("claude", &clear).as_deref(), Some(""));
}

#[test]
fn launch_argument_startup_wait_is_screen_state_not_quoted_words() {
    let now = Timestamp::UNIX_EPOCH + Duration::from_secs(100);
    let spawn = now - Duration::from_secs(60);
    let old_output = Some(now - Duration::from_secs(11));
    let trust = lines("Trust this folder? Codex can read");
    assert_eq!(
        launch_arg_startup_screen("codex", &trust, spawn, old_output, now),
        Some("Trustthisfolder?Codexcanread")
    );
    assert_eq!(
        launch_arg_startup_screen("codex", &trust, spawn, Some(now), now),
        None
    );
    assert_eq!(
        launch_arg_startup_screen(
            "codex",
            &lines("Ask Codex to do anything\nTrust this folder? Codex can read"),
            spawn,
            old_output,
            now
        ),
        None
    );
    assert_eq!(
        launch_arg_startup_screen(
            "claude",
            &lines("unknown startup menu"),
            spawn,
            old_output,
            now
        ),
        Some("")
    );
    assert_eq!(
        launch_arg_startup_screen(
            "claude",
            &lines("unknown startup menu"),
            now,
            old_output,
            now
        ),
        None
    );
    assert_eq!(
        launch_arg_startup_screen("grok", &trust, spawn, old_output, now),
        None
    );
    assert_eq!(
        launch_arg_startup_screen("codex", &[], spawn, old_output, now),
        None
    );
}

#[test]
fn registration_metadata_selects_exactly_one_owner_and_plain_prompt_is_unframed() {
    let binding = SessionBinding {
        session: LiveSessionId(1),
        incarnation: SessionIncarnation(1),
        wrapper: WrapperConnectionId(1),
    };
    let mut metadata = SpawnRegistrationMetadata {
        initial_prompt: "plain instruction".into(),
        ..Default::default()
    };
    let request = InitialPromptRequest::for_registration(binding, &metadata, |_| {
        panic!("plain prompt must not render conductor guide")
    })
    .unwrap()
    .unwrap();
    assert_eq!(request.prompt, "plain instruction");
    assert!(request.notice.parent.is_none());
    metadata.orchestration = OrchestrationId("fixture-orchestration".into());
    let request = InitialPromptRequest::for_registration(binding, &metadata, |id| {
        Ok(format!("guide for {}", id.0))
    })
    .unwrap()
    .unwrap();
    assert_eq!(request.prompt, "guide for fixture-orchestration");
    metadata.auto = true;
    assert!(
        InitialPromptRequest::for_registration(binding, &metadata, |_| panic!(
            "child prompt belongs to child_registered"
        ))
        .unwrap()
        .is_none()
    );
    metadata.auto = false;
    metadata.prompt_at_launch = true;
    assert!(
        InitialPromptRequest::for_registration(binding, &metadata, |_| panic!(
            "launch prompt cannot also be typed"
        ))
        .unwrap()
        .is_none()
    );
    metadata.orchestration.0.clear();
    assert!(
        InitialPromptRequest::for_registration(binding, &metadata, |_| panic!(
            "plain launch prompt cannot also be typed"
        ))
        .unwrap()
        .is_none()
    );
}

#[test]
fn child_request_uses_selected_registered_prompt_and_retained_role_only() {
    use crate::orchestration::child_launch::{ChildPreparation, RegisteredChild};
    let binding = SessionBinding {
        session: LiveSessionId(2),
        incarnation: SessionIncarnation(2),
        wrapper: WrapperConnectionId(2),
    };
    let parent = SessionBinding {
        session: LiveSessionId(1),
        incarnation: SessionIncarnation(1),
        wrapper: WrapperConnectionId(1),
    };
    let mut child = RegisteredChild {
        binding,
        parent,
        role: "review".into(),
        preparation: ChildPreparation {
            orchestration: OrchestrationId("synthetic-orchestration".into()),
            board_path: PathBuf::from("synthetic-board.md"),
            absolute_cwd: PathBuf::from("synthetic-project"),
            child_cwd: PathBuf::from("synthetic-child"),
            branch: "synthetic-branch".into(),
        },
        restart_spec: WrappedSpawnSpec {
            registration_metadata: SpawnRegistrationMetadata::default(),
            spawn_attempt: None,
            registration_proof: None,
            provider: "claude".into(),
            cwd: PathBuf::from("synthetic-child"),
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
        initial_prompt: "unframed base instructions".into(),
        prompt_via_launch_arg: false,
        inject_after_registration: Some("exact already-rendered instruction for session 2".into()),
        spawned_at: Timestamp::UNIX_EPOCH,
    };
    let request = InitialPromptRequest::for_child(&child).unwrap();
    assert_eq!(request.binding, binding);
    assert_eq!(request.notice.parent, Some(parent));
    assert_eq!(request.notice.role, "review");
    assert_eq!(
        request.prompt,
        "exact already-rendered instruction for session 2"
    );
    child.inject_after_registration = None;
    child.prompt_via_launch_arg = true;
    assert!(InitialPromptRequest::for_child(&child).is_none());
    child.prompt_via_launch_arg = false;
    child.restart_spec.execution_mode = "headless".into();
    assert!(InitialPromptRequest::for_child(&child).is_none());
}

#[derive(Default)]
struct Sink {
    count: AtomicUsize,
}
impl CoreEffectSink for Sink {
    fn apply<'a>(&'a self, effects: CoreEffects) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
        Box::pin(async move {
            self.count.fetch_add(effects.0.len(), Ordering::SeqCst);
            // This disabled-persistence fixture consumes effects immediately.
            // Keeping lease-bearing CoreEffects would stall later registration.
            drop(effects);
            Ok(())
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
        Box::pin(async { SpawnWaitOutcome::Failed("fixture has no provider launcher".into()) })
    }
}
struct Transport {
    engine: Mutex<Weak<SessionEngine>>,
    frames: Mutex<Vec<Vec<u8>>>,
    tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    enters: AtomicUsize,
    bodies: AtomicUsize,
    calls: AtomicUsize,
    fail_on: AtomicUsize,
    submit_on: usize,
    echo: AtomicBool,
    redraw: AtomicBool,
}
impl Transport {
    fn draw(&self, binding: SessionBinding, data: Vec<u8>) {
        if let Some(engine) = self.engine.lock().unwrap().upgrade() {
            engine
                .observe_output(
                    binding,
                    OutputChunk {
                        bytes: data,
                        total_pty_bytes: 0,
                    },
                    Timestamp::now(),
                )
                .unwrap();
        }
    }
    async fn drain(&self) {
        let tasks = std::mem::take(&mut *self.tasks.lock().unwrap());
        for task in tasks {
            task.await.unwrap();
        }
    }
}
impl Drop for Transport {
    fn drop(&mut self) {
        for task in self.tasks.get_mut().unwrap().drain(..) {
            task.abort();
        }
    }
}
impl WrapperTransport for Transport {
    fn send<'a>(
        &'a self,
        binding: SessionBinding,
        message: proto::Message,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            if message.r#type != "pty_input" {
                return Err(SessionError::InvalidRequest(
                    "fixture expects input only".into(),
                ));
            }
            let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if call == self.fail_on.load(Ordering::SeqCst) {
                return Err(SessionError::Transport(
                    "synthetic transport failure".into(),
                ));
            }
            let data = message.data;
            self.frames.lock().unwrap().push(data.clone());
            if data.starts_with(b"\x1b[200~") {
                self.bodies.fetch_add(1, Ordering::SeqCst);
                if self.echo.load(Ordering::SeqCst) {
                    self.draw(binding, claude_screen(PROMPT, false));
                }
            } else if data == b"\r" {
                let enter = self.enters.fetch_add(1, Ordering::SeqCst) + 1;
                if self.redraw.load(Ordering::SeqCst) {
                    let engine = self.engine.lock().unwrap().clone();
                    let submitted = self.submit_on > 0 && enter >= self.submit_on;
                    let echoed = self.echo.load(Ordering::SeqCst);
                    // Like the fixed Go fakeClaudeTUI, redraw after send returns
                    // so the generic output-confirmation sees the new output.
                    self.tasks.lock().unwrap().push(tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_millis(2)).await;
                        if let Some(engine) = engine.upgrade() {
                            engine
                                .observe_output(
                                    binding,
                                    OutputChunk {
                                        bytes: claude_screen(
                                            if submitted || !echoed { "" } else { PROMPT },
                                            submitted && echoed,
                                        ),
                                        total_pty_bytes: 0,
                                    },
                                    Timestamp::now(),
                                )
                                .unwrap();
                        }
                    }));
                }
            }
            Ok(())
        })
    }
}

struct Callbacks {
    records: Mutex<Vec<InitialPromptOutcome>>,
    parents: Mutex<Vec<(SessionBinding, String, String)>>,
    warnings: Mutex<Vec<&'static str>>,
    sink: Arc<Sink>,
    board_fail: AtomicBool,
}
impl InitialPromptCallbacks for Callbacks {
    fn record_outcome(
        &self,
        _: SessionBinding,
        outcome: &InitialPromptOutcome,
        _: Timestamp,
    ) -> Result<(), SessionError> {
        self.records.lock().unwrap().push(outcome.clone());
        Ok(())
    }
    fn append_board_failure<'a>(
        &'a self,
        path: &'a std::path::Path,
        text: String,
        _: &'a TaskCancellation,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            if self.board_fail.load(Ordering::SeqCst) {
                return Err(SessionError::Transport("synthetic board failure".into()));
            }
            std::fs::write(path, text).map_err(|error| SessionError::Transport(error.to_string()))
        })
    }
    fn notify_parent_failure<'a>(
        &'a self,
        parent: SessionBinding,
        limit: &'static str,
        detail: String,
        _: &'a TaskCancellation,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            self.parents
                .lock()
                .unwrap()
                .push((parent, limit.into(), detail));
            Ok(())
        })
    }
    fn apply_effects<'a>(
        &'a self,
        effects: CoreEffects,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            self.sink
                .apply(effects)
                .await
                .map_err(|failure| failure.error)
        })
    }
    fn warning(&self, operation: &'static str, _: &SessionError) {
        self.warnings.lock().unwrap().push(operation);
    }
}
struct Fixture {
    root: tempfile::TempDir,
    engine: Arc<SessionEngine>,
    transport: Arc<Transport>,
    callbacks: Arc<Callbacks>,
    driver: Arc<InitialPromptDriver>,
    owner: HubTaskOwner,
    binding: SessionBinding,
}
impl Fixture {
    async fn new(provider: &str, submit_on: usize) -> Self {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("runtime")).unwrap();
        let paths = RuntimePaths::trial(
            &root.path().join("runtime"),
            49127,
            &root.path().join("installed"),
        )
        .unwrap();
        let journal = Arc::new(SessionJournal::new(paths, None, JournalOptions::default()));
        let transport = Arc::new(Transport {
            engine: Mutex::new(Weak::new()),
            frames: Mutex::new(Vec::new()),
            tasks: Mutex::new(Vec::new()),
            enters: AtomicUsize::new(0),
            bodies: AtomicUsize::new(0),
            calls: AtomicUsize::new(0),
            fail_on: AtomicUsize::new(0),
            submit_on,
            echo: AtomicBool::new(true),
            redraw: AtomicBool::new(true),
        });
        let sink = Arc::new(Sink::default());
        let engine = Arc::new(SessionEngine::new(
            EngineOptions {
                submit_timing: SubmitTiming {
                    idle_settle: Duration::from_millis(1),
                    minimum: Duration::from_millis(1),
                    slow_minimum: Duration::from_millis(1),
                    maximum: Duration::from_millis(10),
                    poll: Duration::from_millis(1),
                    confirm_window: Duration::from_millis(15),
                },
                ..Default::default()
            },
            journal,
            transport.clone(),
            sink.clone(),
            Arc::new(NoSpawn),
            CoreEventBus::new(100).unwrap(),
        ));
        *transport.engine.lock().unwrap() = Arc::downgrade(&engine);
        let registration = engine
            .register(
                RegisterRequest {
                    message: proto::Message {
                        provider: provider.into(),
                        pid: 7,
                        cols: 130,
                        rows: 35,
                        cwd: root.path().to_string_lossy().into_owned(),
                        ..Default::default()
                    },
                    spawn_proof: None,
                },
                WrapperConnectionId(1),
                Timestamp::now(),
            )
            .await
            .unwrap();
        let binding = registration.binding;
        sink.apply(registration.after_registered).await.unwrap();
        engine.set_initial_gate(binding, Timestamp::now()).unwrap();
        let callbacks = Arc::new(Callbacks {
            records: Mutex::new(Vec::new()),
            parents: Mutex::new(Vec::new()),
            warnings: Mutex::new(Vec::new()),
            sink,
            board_fail: AtomicBool::new(false),
        });
        let mut driver = InitialPromptDriver::new(Arc::downgrade(&engine), callbacks.clone());
        driver.timing = Timing {
            composer_wait: Duration::from_millis(20),
            composer_stable: Duration::from_millis(4),
            quiet: Duration::from_millis(1),
            quiet_wait: Duration::from_millis(5),
            echo_wait: Duration::from_millis(15),
            confirm_wait: Duration::from_millis(10),
            clear_stable: Duration::from_millis(4),
            poll: Duration::from_millis(1),
        };
        let fixture = Self {
            root,
            engine,
            transport,
            callbacks,
            driver: Arc::new(driver),
            owner: HubTaskOwner::new(tokio::runtime::Handle::current()),
            binding,
        };
        fixture.transport.draw(binding, claude_screen("", false));
        fixture
    }
    fn request(&self) -> InitialPromptRequest {
        InitialPromptRequest {
            binding: self.binding,
            prompt: PROMPT.into(),
            notice: InitialPromptNotice {
                parent: Some(self.binding),
                board_path: Some(self.root.path().join("board.md")),
                role: "implementation".into(),
            },
        }
    }
    async fn run(&self) -> InitialPromptCompletion {
        let result = self
            .driver
            .start(self.owner.effect_permit().unwrap(), self.request())
            .unwrap()
            .wait()
            .await
            .unwrap()
            .unwrap();
        self.transport.drain().await;
        result
    }
    async fn assert_gate_cleared(&self) {
        let result = self
            .engine
            .submit(
                self.binding,
                InputRequest {
                    bytes: b"manual".to_vec(),
                    authority: InputAuthority::Internal,
                },
                Timestamp::now(),
                &TaskCancellation::default(),
            )
            .await;
        assert!(
            matches!(
                result.disposition,
                InputDisposition::TransportWritten { .. }
            ),
            "{:?}",
            result.disposition
        );
    }
}

#[tokio::test(start_paused = true)]
async fn quiet_elapsed_output_returns_without_using_max_wait() {
    let fixture = Fixture::new("claude", 0).await;
    // A fixed historical output instant is already quiet on the real wall clock.
    // Tokio's paused clock separately proves that no poll/timeout was required.
    let output_at = Timestamp::UNIX_EPOCH + Duration::from_secs(100);
    fixture
        .engine
        .observe_output(
            fixture.binding,
            OutputChunk {
                bytes: b"synthetic historical output".to_vec(),
                total_pty_bytes: 0,
            },
            output_at,
        )
        .unwrap();
    assert_eq!(
        fixture
            .engine
            .initial_prompt_observation(fixture.binding)
            .unwrap()
            .0
            .last_output_at,
        Some(output_at)
    );
    let started = Instant::now();
    fixture
        .driver
        .wait_quiet(
            &fixture.engine,
            fixture.binding,
            &TaskCancellation::default(),
        )
        .await
        .unwrap();
    assert_eq!(started.elapsed(), Duration::ZERO);
    assert!(fixture.transport.frames.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn quiet_max_wait_fallback_runs_when_wall_clock_quiet_cannot_win() {
    let fixture = Fixture::new("claude", 0).await;
    // Synthetic future output makes duration_since fail, deterministically
    // excluding the elapsed-output branch while Tokio advances each poll.
    let output_at = Timestamp::from_unix(4_102_444_800, 0).unwrap();
    fixture
        .engine
        .observe_output(
            fixture.binding,
            OutputChunk {
                bytes: b"synthetic future output".to_vec(),
                total_pty_bytes: 0,
            },
            output_at,
        )
        .unwrap();
    assert_eq!(
        fixture
            .engine
            .initial_prompt_observation(fixture.binding)
            .unwrap()
            .0
            .last_output_at,
        Some(output_at)
    );
    let started = Instant::now();
    fixture
        .driver
        .wait_quiet(
            &fixture.engine,
            fixture.binding,
            &TaskCancellation::default(),
        )
        .await
        .unwrap();
    assert_eq!(started.elapsed(), fixture.driver.timing.quiet_wait);
    assert!(fixture.transport.frames.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn first_enter_and_composer_retry_match_source() {
    for (submit_on, expected_enters) in [(1, 1), (2, 2)] {
        let fixture = Fixture::new("claude", submit_on).await;
        let result = fixture.run().await;
        assert!(
            matches!(
                result.outcome,
                InitialPromptOutcome::Delivered {
                    evidence: DeliveryEvidence::ComposerCleared,
                    ..
                }
            ),
            "{:?}",
            result
        );
        assert_eq!(fixture.transport.bodies.load(Ordering::SeqCst), 1);
        assert_eq!(
            fixture.transport.enters.load(Ordering::SeqCst),
            expected_enters
        );
        assert_eq!(fixture.callbacks.records.lock().unwrap().len(), 1);
        assert!(!fixture.root.path().join("board.md").exists());
        fixture.assert_gate_cleared().await;
    }
}

#[tokio::test(start_paused = true)]
async fn stuck_composer_records_failure_and_both_notices() {
    let fixture = Fixture::new("claude", 0).await;
    let result = fixture.run().await;
    assert!(matches!(
        result.outcome,
        InitialPromptOutcome::Failed { .. }
    ));
    assert_eq!(fixture.transport.enters.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.transport.bodies.load(Ordering::SeqCst), 1);
    let board = std::fs::read_to_string(fixture.root.path().join("board.md")).unwrap();
    assert!(board.contains("initial prompt NOT delivered"));
    assert!(board.contains("still sitting in the child's composer"));
    let parents = fixture.callbacks.parents.lock().unwrap();
    assert_eq!(parents.len(), 1);
    assert_eq!(parents[0].1, "child_input_blocked");
}

#[tokio::test(start_paused = true)]
async fn stable_folder_trust_blocks_without_a_single_input_and_failure_is_not_silent() {
    for (provider, screen) in [
        ("claude", "Is this a project you created or one you trust?"),
        (
            "codex",
            "Trust this folder? Codex can read, edit, and run files",
        ),
    ] {
        let mut fixture = Fixture::new(provider, 1).await;
        Arc::get_mut(&mut fixture.driver)
            .unwrap()
            .timing
            .composer_wait = Duration::from_millis(200);
        fixture.transport.draw(
            fixture.binding,
            format!("\x1b[2J\x1b[H{screen}").into_bytes(),
        );
        let started = Instant::now();
        let result = fixture.run().await;
        assert!(matches!(
            result.outcome,
            InitialPromptOutcome::Failed { .. }
        ));
        assert!(started.elapsed() < Duration::from_millis(100));
        assert!(fixture.transport.frames.lock().unwrap().is_empty());
        let board = std::fs::read_to_string(fixture.root.path().join("board.md")).unwrap();
        assert!(board.contains("trust its working folder"));
        assert!(board.contains("orchestrate send"));
        assert_eq!(fixture.callbacks.parents.lock().unwrap().len(), 1);
        fixture.assert_gate_cleared().await;
    }
}

#[tokio::test(start_paused = true)]
async fn board_notice_failure_does_not_skip_parent_or_outcome() {
    let fixture = Fixture::new("claude", 1).await;
    fixture.callbacks.board_fail.store(true, Ordering::SeqCst);
    fixture.transport.draw(
        fixture.binding,
        b"\x1b[2J\x1b[HAllow external CLAUDE.md file imports?".to_vec(),
    );
    let result = fixture.run().await;
    assert_eq!(result.notice_errors.len(), 1);
    assert_eq!(fixture.callbacks.parents.lock().unwrap().len(), 1);
    assert_eq!(fixture.callbacks.records.lock().unwrap().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn gated_user_input_flushes_only_after_initial_body_and_submit() {
    let fixture = Fixture::new("claude", 1).await;
    for bytes in [b"user first".to_vec(), b"user second".to_vec()] {
        let result = fixture
            .engine
            .submit(
                fixture.binding,
                InputRequest {
                    bytes,
                    authority: InputAuthority::Internal,
                },
                Timestamp::now(),
                &TaskCancellation::default(),
            )
            .await;
        assert!(matches!(
            result.disposition,
            InputDisposition::Deferred {
                reason: DeferredReason::InitialPrompt,
                ..
            }
        ));
    }
    let result = fixture.run().await;
    assert_eq!(result.cleanup, PendingInputCleanup::FlushAttempted);
    let frames = fixture.transport.frames.lock().unwrap();
    assert_eq!(frames.len(), 4);
    assert!(frames[0].starts_with(b"\x1b[200~"));
    assert_eq!(frames[1], b"\r");
    assert_eq!(frames[2], b"user first");
    assert_eq!(frames[3], b"user second");
}

#[tokio::test(start_paused = true)]
async fn cancellation_releases_gate_without_claiming_async_flush() {
    let fixture = Fixture::new("claude", 1).await;
    let permit = fixture.owner.effect_permit().unwrap();
    permit.cancellation().cancel();
    let result = fixture
        .driver
        .start(permit, fixture.request())
        .unwrap()
        .wait()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.outcome, InitialPromptOutcome::Cancelled);
    assert_eq!(result.cleanup, PendingInputCleanup::CancelledBeforeFlush);
    assert!(fixture.transport.frames.lock().unwrap().is_empty());
    fixture.assert_gate_cleared().await;
}

#[tokio::test(start_paused = true)]
async fn dropped_waiter_does_not_cancel_owned_delivery() {
    let fixture = Fixture::new("claude", 1).await;
    drop(
        fixture
            .driver
            .start(fixture.owner.effect_permit().unwrap(), fixture.request())
            .unwrap(),
    );
    fixture.owner.drain_effects().await;
    assert!(matches!(
        fixture.callbacks.records.lock().unwrap().as_slice(),
        [InitialPromptOutcome::Delivered { .. }]
    ));
}

#[tokio::test(start_paused = true)]
async fn forced_abort_drops_guard_before_lane_drain_and_does_not_spawn_cleanup() {
    let fixture = Fixture::new("claude", 1).await;
    fixture
        .transport
        .draw(fixture.binding, b"\x1b[2J\x1b[Hstarting".to_vec());
    let waiter = fixture
        .driver
        .start(fixture.owner.effect_permit().unwrap(), fixture.request())
        .unwrap();
    fixture.owner.stop_requests();
    fixture.owner.drain_requests().await;
    fixture.owner.stop_effects().unwrap();
    fixture.owner.abort_effects().unwrap();
    assert!(waiter.wait().await.is_err());
    fixture.owner.drain_effects().await;
    assert_eq!(
        *fixture.callbacks.records.lock().unwrap(),
        vec![InitialPromptOutcome::Interrupted]
    );
    assert!(
        fixture
            .callbacks
            .warnings
            .lock()
            .unwrap()
            .iter()
            .any(|warning| warning.contains("flush/async notices incomplete"))
    );
    assert!(fixture.transport.frames.lock().unwrap().is_empty());
    fixture.assert_gate_cleared().await;
}

#[tokio::test(start_paused = true)]
async fn observation_rejects_old_wrapper_but_cleanup_follows_same_incarnation() {
    let fixture = Fixture::new("claude", 1).await;
    let old = fixture.binding;
    let attachment = fixture
        .engine
        .reattach(
            ReattachRequest {
                restored_metadata: None,
                message: proto::Message {
                    session_id: old.session.0,
                    provider: "claude".into(),
                    pid: 7,
                    cwd: fixture.root.path().to_string_lossy().into_owned(),
                    cols: 130,
                    rows: 35,
                    ..Default::default()
                },
            },
            WrapperConnectionId(2),
            Timestamp::now(),
        )
        .await
        .unwrap();
    let current = attachment.binding;
    assert_eq!(current.session, old.session);
    assert_eq!(current.incarnation, old.incarnation);
    assert_ne!(current.wrapper, old.wrapper);
    assert!(matches!(
        fixture.engine.initial_prompt_observation(old),
        Err(SessionError::StaleBinding)
    ));
    assert!(fixture.engine.initial_prompt_observation(current).is_ok());
    fixture
        .engine
        .set_initial_gate(current, Timestamp::now())
        .unwrap();
    assert!(fixture.engine.clear_initial_gate_scoped(old));
    let receipt = fixture
        .engine
        .submit(
            current,
            InputRequest {
                bytes: b"after reconnect".to_vec(),
                authority: InputAuthority::Internal,
            },
            Timestamp::now(),
            &TaskCancellation::default(),
        )
        .await;
    assert!(matches!(
        receipt.disposition,
        InputDisposition::TransportWritten { .. }
    ));
}

#[tokio::test(start_paused = true)]
async fn retired_incarnation_cannot_clear_a_replacement_sessions_gate() {
    let fixture = Fixture::new("claude", 1).await;
    let old = fixture.binding;
    fixture
        .engine
        .dismiss(old.session, Timestamp::now())
        .unwrap();
    assert!(!fixture.engine.clear_initial_gate_scoped(old));
    let replacement = fixture
        .engine
        .reattach(
            ReattachRequest {
                restored_metadata: None,
                message: proto::Message {
                    session_id: old.session.0,
                    provider: "claude".into(),
                    pid: 8,
                    cols: 130,
                    rows: 35,
                    ..Default::default()
                },
            },
            WrapperConnectionId(2),
            Timestamp::now(),
        )
        .await
        .unwrap()
        .binding;
    assert_eq!(replacement.session, old.session);
    assert_ne!(replacement.incarnation, old.incarnation);
    fixture
        .engine
        .set_initial_gate(replacement, Timestamp::now())
        .unwrap();
    assert!(!fixture.engine.clear_initial_gate_scoped(old));
    assert!(matches!(
        fixture.engine.initial_prompt_observation(old),
        Err(SessionError::StaleBinding)
    ));
    let receipt = fixture
        .engine
        .submit(
            replacement,
            InputRequest {
                bytes: b"replacement user".to_vec(),
                authority: InputAuthority::Internal,
            },
            Timestamp::now(),
            &TaskCancellation::default(),
        )
        .await;
    assert!(matches!(
        receipt.disposition,
        InputDisposition::Deferred {
            reason: DeferredReason::InitialPrompt,
            ..
        }
    ));
}

#[tokio::test(start_paused = true)]
async fn no_echo_retries_body_exactly_twice_then_reports_failure() {
    let fixture = Fixture::new("claude", 0).await;
    fixture.transport.echo.store(false, Ordering::SeqCst);
    let result = fixture.run().await;
    assert!(
        matches!(result.outcome, InitialPromptOutcome::Failed { ref detail } if detail.contains("after 2 attempts"))
    );
    assert_eq!(fixture.transport.bodies.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.callbacks.parents.lock().unwrap().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn transport_failure_does_not_reinject_body_before_core_pending_flush() {
    let fixture = Fixture::new("claude", 1).await;
    fixture.transport.fail_on.store(1, Ordering::SeqCst);
    let result = fixture.run().await;
    assert!(matches!(
        result.outcome,
        InitialPromptOutcome::TransportUnconfirmed { .. }
    ));
    assert_eq!(fixture.transport.bodies.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.transport.calls.load(Ordering::SeqCst), 3);
    assert!(fixture.callbacks.parents.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn generic_no_output_retry_and_composer_retry_remain_distinct_source_paths() {
    let fixture = Fixture::new("claude", 0).await;
    fixture.transport.redraw.store(false, Ordering::SeqCst);
    let result = fixture.run().await;
    assert!(matches!(
        result.outcome,
        InitialPromptOutcome::Failed { .. }
    ));
    assert_eq!(fixture.transport.bodies.load(Ordering::SeqCst), 1);
    // The fixed Go caller has a generic no-new-output retry, then a separate
    // composer-stuck retry. A migration must not silently collapse the two.
    assert_eq!(fixture.transport.enters.load(Ordering::SeqCst), 3);
}

#[tokio::test(start_paused = true)]
async fn provider_without_composer_extractor_keeps_source_echo_evidence() {
    let fixture = Fixture::new("grok", 0).await;
    let result = fixture.run().await;
    assert!(matches!(
        result.outcome,
        InitialPromptOutcome::Delivered {
            evidence: DeliveryEvidence::EchoObserved,
            composer_enter_retried: false,
            ..
        }
    ));
    assert_eq!(fixture.transport.bodies.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.transport.enters.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn transient_ready_or_blocked_screen_does_not_end_stability_wait() {
    let fixture = Fixture::new("codex", 1).await;
    fixture.transport.draw(
        fixture.binding,
        b"\x1b[2J\x1b[HAsk Codex to do anything".to_vec(),
    );
    let cancel = TaskCancellation::default();
    let ready = fixture
        .driver
        .wait_composer(&fixture.engine, fixture.binding, &cancel);
    let redraw = async {
        tokio::time::sleep(Duration::from_millis(1)).await;
        fixture.transport.draw(
            fixture.binding,
            b"\x1b[2J\x1b[HTrust this folder? Codex can read".to_vec(),
        );
    };
    let (ready, ()) = tokio::join!(ready, redraw);
    assert!(matches!(
        ready.unwrap(),
        ComposerReady::Blocked("Trustthisfolder?Codexcanread")
    ));
}

#[tokio::test(start_paused = true)]
async fn late_echo_during_second_readiness_wait_prevents_body_reinjection() {
    let mut fixture = Fixture::new("claude", 1).await;
    let driver = Arc::get_mut(&mut fixture.driver).unwrap();
    driver.timing.composer_stable = Duration::from_millis(30);
    driver.timing.composer_wait = Duration::from_millis(100);
    driver.timing.echo_wait = Duration::from_millis(5);
    fixture.transport.echo.store(false, Ordering::SeqCst);
    let late_echo = async {
        while fixture.transport.bodies.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        fixture
            .transport
            .draw(fixture.binding, claude_screen("", true));
    };
    let (result, ()) = tokio::join!(fixture.run(), late_echo);
    assert!(matches!(
        result.outcome,
        InitialPromptOutcome::Delivered { attempts: 1, .. }
    ));
    assert_eq!(fixture.transport.bodies.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn post_echo_approval_prevents_orchestration_composer_enter_retry() {
    let fixture = Fixture::new("claude", 0).await;
    let approval = format!(
        "\x1b[2J\x1b[HRun: echo fixture\r\nDo you want to proceed?\r\n❯ 1. Yes\r\n  2. No\r\n{RULE}\r\n❯ {PROMPT}\r\n{RULE}\r\n⏵⏵ bypass permissions on (shift+tab to cycle)"
    );
    fixture
        .transport
        .draw(fixture.binding, approval.into_bytes());
    assert!(
        fixture
            .engine
            .details(fixture.binding.session)
            .unwrap()
            .approval
            .record
            .is_some()
    );
    let result = fixture
        .driver
        .confirm(
            &fixture.engine,
            fixture.binding,
            &screen::echo_marker(PROMPT),
            1,
            &TaskCancellation::default(),
        )
        .await
        .unwrap();
    assert!(matches!(
        result,
        InitialPromptOutcome::Delivered {
            evidence: DeliveryEvidence::ApprovalObservedAfterEcho,
            composer_enter_retried: false,
            ..
        }
    ));
    assert!(fixture.transport.frames.lock().unwrap().is_empty());
}
