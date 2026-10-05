//! Initial instruction delivery through the one SessionEngine input owner.
//!
//! Source: orchestration.go injectInitialPromptNotify/readiness/echo/confirmation.
//! The launch caller chooses exactly one prompt route. This driver is only for
//! the typed route; launch-argument/headless acceptance is observed elsewhere.
//! A transport write is never recorded as provider acceptance.
use crate::{
    hub::task_owner::{OwnedTaskPermit, TaskWaiter},
    proto::{core::*, time::Timestamp},
    terminal::session::SessionEngine,
};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    path::PathBuf,
    sync::{Arc, Weak},
    time::Duration,
};
use tokio::time::Instant;

mod screen;
pub use screen::launch_arg_startup_screen;
#[cfg(test)]
mod tests;

/// Source provider startup blocker classifier. Pure, with no session ownership.
pub(crate) fn screen_blocker(provider: &str, lines: &[String]) -> Option<&'static str> {
    screen::blocker(provider, &screen::collapse_whitespace(&lines.concat()))
}

#[derive(Clone, Debug, Default)]
pub struct InitialPromptNotice {
    pub parent: Option<SessionBinding>,
    pub board_path: Option<PathBuf>,
    pub role: String,
}
pub struct InitialPromptRequest {
    pub binding: SessionBinding,
    pub prompt: String,
    pub notice: InitialPromptNotice,
}
impl InitialPromptRequest {
    /// ChildLaunchExecutor already selected the sole delivery route and rendered
    /// the registered id into its prompt. Never rebuild it from restart inputs.
    pub fn for_child(child: &super::child_launch::RegisteredChild) -> Option<Self> {
        child.inject_after_registration.as_ref().map(|prompt| Self {
            binding: child.binding,
            prompt: prompt.clone(),
            notice: InitialPromptNotice {
                parent: Some(child.parent),
                board_path: Some(child.preparation.board_path.clone()),
                role: child.role.clone(),
            },
        })
    }

    /// Source wrapper_loop.go registrationInjectPrompt. The conductor renderer
    /// belongs to the launch/role owner and is evaluated only for that route.
    /// Auto children are exclusively started by child_registered.
    pub fn for_registration(
        binding: SessionBinding,
        metadata: &SpawnRegistrationMetadata,
        conductor_prompt: impl FnOnce(&OrchestrationId) -> Result<String, SessionError>,
    ) -> Result<Option<Self>, SessionError> {
        let prompt = if metadata.prompt_at_launch {
            return Ok(None);
        } else if !metadata.orchestration.0.is_empty() && !metadata.auto {
            conductor_prompt(&metadata.orchestration)?
        } else if metadata.orchestration.0.is_empty() && !metadata.initial_prompt.is_empty() {
            metadata.initial_prompt.clone()
        } else {
            return Ok(None);
        };
        Ok((!prompt.is_empty()).then_some(Self {
            binding,
            prompt,
            notice: InitialPromptNotice::default(),
        }))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeliveryEvidence {
    /// A launch argument has no terminal echo; provider transcript user records
    /// are the fixed source's acceptance evidence.
    TranscriptUserObserved,
    /// Source compatibility for providers without a composer extractor.
    EchoObserved,
    ComposerCleared,
    /// The source treats a post-echo approval as a started turn; never answer it.
    ApprovalObservedAfterEcho,
    /// Source marks post-echo delivery when the retry Enter is deferred.
    EchoObservedSubmitUnconfirmed,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InitialPromptOutcome {
    /// Fixed-source delivery classification. The evidence is retained explicitly;
    /// it is not a claim that socket I/O proved provider/model acceptance.
    Delivered {
        evidence: DeliveryEvidence,
        attempts: usize,
        /// This is the orchestration composer retry. The existing input owner
        /// independently retains Go's generic no-new-output submit retry.
        composer_enter_retried: bool,
    },
    /// The input owner retained an unsent frame. Do not send the body again.
    Deferred,
    /// A failed receipt does not prove the input owner queued its remainder.
    TransportUnconfirmed {
        detail: String,
    },
    Failed {
        detail: String,
    },
    Cancelled,
    /// Forced abort/runtime abandonment: only synchronous cleanup ran.
    Interrupted,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendingInputCleanup {
    FlushAttempted,
    CancelledBeforeFlush,
    SessionRetired,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InitialPromptCompletion {
    pub outcome: InitialPromptOutcome,
    pub cleanup: PendingInputCleanup,
    /// Both board and parent are attempted even when one notice fails.
    pub notice_errors: Vec<SessionError>,
}

/// Required production integration, with no success/no-op defaults. Board and
/// orchestration state remain with their existing owner, never a second session
/// map. `record_outcome` must be synchronous and safe during forced-abort Drop.
pub trait InitialPromptCallbacks: Send + Sync {
    fn record_outcome(
        &self,
        binding: SessionBinding,
        outcome: &InitialPromptOutcome,
        at: Timestamp,
    ) -> Result<(), SessionError>;
    fn append_board_failure<'a>(
        &'a self,
        path: &'a std::path::Path,
        text: String,
        cancel: &'a TaskCancellation,
    ) -> CoreFuture<'a, Result<(), SessionError>>;
    fn notify_parent_failure<'a>(
        &'a self,
        parent: SessionBinding,
        limit: &'static str,
        detail: String,
        cancel: &'a TaskCancellation,
    ) -> CoreFuture<'a, Result<(), SessionError>>;
    fn apply_effects<'a>(
        &'a self,
        effects: CoreEffects,
    ) -> CoreFuture<'a, Result<(), SessionError>>;
    fn warning(&self, operation: &'static str, error: &SessionError);
}

#[derive(Clone, Copy)]
struct Timing {
    composer_wait: Duration,
    composer_stable: Duration,
    quiet: Duration,
    quiet_wait: Duration,
    echo_wait: Duration,
    confirm_wait: Duration,
    clear_stable: Duration,
    poll: Duration,
}
impl Default for Timing {
    fn default() -> Self {
        Self {
            composer_wait: Duration::from_secs(45),
            composer_stable: Duration::from_millis(1500),
            quiet: Duration::from_millis(300),
            quiet_wait: Duration::from_secs(5),
            echo_wait: Duration::from_secs(20),
            confirm_wait: Duration::from_secs(5),
            clear_stable: Duration::from_millis(300),
            poll: Duration::from_millis(100),
        }
    }
}

pub struct InitialPromptDriver {
    engine: Weak<SessionEngine>,
    callbacks: Arc<dyn InitialPromptCallbacks>,
    timing: Timing,
}
impl InitialPromptDriver {
    pub fn new(engine: Weak<SessionEngine>, callbacks: Arc<dyn InitialPromptCallbacks>) -> Self {
        Self {
            engine,
            callbacks,
            timing: Timing::default(),
        }
    }

    /// Registration has already installed the source initial gate. The caller
    /// registers board/restart context before this handoff and keeps
    /// the Hub lifecycle owner alive through its normal drain. The mandatory
    /// permit owns the work independently of the HTTP response waiter.
    pub fn start(
        self: &Arc<Self>,
        permit: OwnedTaskPermit,
        request: InitialPromptRequest,
    ) -> Result<TaskWaiter<Result<InitialPromptCompletion, SessionError>>, SessionError> {
        let engine = self.engine.upgrade().ok_or(SessionError::Shutdown)?;
        // Validate before transferring the task. Construct the guard now so an
        // unpolled future also clears its own gate on runtime/owner abandonment.
        let guard = DeliveryGuard {
            engine: engine.clone(),
            callbacks: self.callbacks.clone(),
            binding: request.binding,
            recorded: false,
            cleared: false,
            finished: false,
        };
        engine.initial_prompt_observation(request.binding)?;
        let cancel = permit.cancellation();
        let driver = self.clone();
        Ok(permit.start(async move { driver.run(engine, request, cancel, guard).await }))
    }

    async fn run(
        &self,
        engine: Arc<SessionEngine>,
        request: InitialPromptRequest,
        cancel: TaskCancellation,
        mut guard: DeliveryGuard,
    ) -> Result<InitialPromptCompletion, SessionError> {
        let result = self.deliver(&engine, &request, &cancel).await;
        let outcome = match result {
            Ok(outcome) => outcome,
            Err(SessionError::Cancelled) => InitialPromptOutcome::Cancelled,
            Err(error) => {
                // Missing/stale sessions never masquerade as accepted delivery.
                self.callbacks.warning("initial prompt delivery", &error);
                InitialPromptOutcome::Failed {
                    detail: format!("initial prompt delivery stopped: {error:?}"),
                }
            }
        };
        let mut errors = Vec::new();
        if let InitialPromptOutcome::Failed { detail } = &outcome {
            self.report_failure(&request, detail, &cancel, &mut errors)
                .await;
        }
        // Once a synchronous callback is attempted, a callback panic must not
        // trigger a second, contradictory record from Drop.
        guard.recorded = true;
        if let Err(error) =
            self.callbacks
                .record_outcome(request.binding, &outcome, Timestamp::now())
        {
            self.callbacks
                .warning("record initial prompt outcome", &error);
            errors.push(error);
        }
        // An attempted record must never be followed by a contradictory Drop
        // outcome. Its error is retained rather than reported as success.
        let cleared = engine.clear_initial_gate_scoped(request.binding);
        guard.cleared = true;
        let cleanup = if !cleared {
            PendingInputCleanup::SessionRetired
        } else if cancel.token().is_cancelled() {
            self.callbacks.warning(
                "initial prompt pending input not flushed",
                &SessionError::Cancelled,
            );
            PendingInputCleanup::CancelledBeforeFlush
        } else {
            let effects = engine.flush(request.binding, &cancel).await;
            if let Err(error) = self.callbacks.apply_effects(effects).await {
                self.callbacks
                    .warning("initial prompt pending input effects", &error);
                errors.push(error);
            }
            PendingInputCleanup::FlushAttempted
        };
        guard.finished = true;
        Ok(InitialPromptCompletion {
            outcome,
            cleanup,
            notice_errors: errors,
        })
    }

    async fn report_failure(
        &self,
        request: &InitialPromptRequest,
        detail: &str,
        cancel: &TaskCancellation,
        errors: &mut Vec<SessionError>,
    ) {
        let notice = &request.notice;
        if let Some(path) = notice
            .board_path
            .as_ref()
            .filter(|path| !path.as_os_str().is_empty())
        {
            let text = format!(
                "initial prompt NOT delivered: role={} session={} {detail}\n",
                notice.role, request.binding.session.0
            );
            if let Err(error) = self
                .callbacks
                .append_board_failure(path, text, cancel)
                .await
            {
                self.callbacks
                    .warning("initial prompt board failure notice", &error);
                errors.push(error);
            }
        }
        if let Some(parent) = notice.parent.filter(|parent| parent.session.0 > 0) {
            let text = format!(
                "role={} id={}: {detail}",
                notice.role, request.binding.session.0
            );
            if let Err(error) = self
                .callbacks
                .notify_parent_failure(parent, "child_input_blocked", text, cancel)
                .await
            {
                self.callbacks
                    .warning("initial prompt parent failure notice", &error);
                errors.push(error);
            }
        }
    }

    async fn deliver(
        &self,
        engine: &SessionEngine,
        request: &InitialPromptRequest,
        cancel: &TaskCancellation,
    ) -> Result<InitialPromptOutcome, SessionError> {
        const MAX_ATTEMPTS: usize = 2;
        let binding = request.binding;
        let marker = screen::echo_marker(&request.prompt);
        let bytes = format!(
            "\x1b[200~{}\x1b[201~\r",
            request.prompt.trim_end_matches(['\r', '\n'])
        )
        .into_bytes();
        for attempt in 1..=MAX_ATTEMPTS {
            match self.wait_composer(engine, binding, cancel).await? {
                ComposerReady::Blocked(signal) => {
                    return Ok(InitialPromptOutcome::Failed {
                        detail: screen::blocked_detail(signal),
                    });
                }
                ComposerReady::Ready => {}
                ComposerReady::Unknown | ComposerReady::NoSignal => {
                    self.wait_quiet(engine, binding, cancel).await?
                }
            }
            if attempt > 1
                && self
                    .wait_echo(engine, binding, &marker, Duration::ZERO, cancel)
                    .await?
            {
                return self
                    .confirm(engine, binding, &marker, attempt - 1, cancel)
                    .await;
            }
            let receipt = engine
                .submit(
                    binding,
                    InputRequest {
                        bytes: bytes.clone(),
                        authority: InputAuthority::InitialPrompt,
                    },
                    Timestamp::now(),
                    cancel,
                )
                .await;
            match input_result(receipt, cancel)? {
                WriteResult::Deferred => return Ok(InitialPromptOutcome::Deferred),
                WriteResult::Unconfirmed(detail) => {
                    return Ok(InitialPromptOutcome::TransportUnconfirmed { detail });
                }
                WriteResult::Written => {}
            }
            if self
                .wait_echo(engine, binding, &marker, self.timing.echo_wait, cancel)
                .await?
            {
                return self
                    .confirm(engine, binding, &marker, attempt, cancel)
                    .await;
            }
        }
        Ok(InitialPromptOutcome::Failed {
            detail: format!(
                "the child never echoed the initial prompt after {MAX_ATTEMPTS} attempts; it was NOT delivered"
            ),
        })
    }

    async fn wait_composer(
        &self,
        engine: &SessionEngine,
        binding: SessionBinding,
        cancel: &TaskCancellation,
    ) -> Result<ComposerReady, SessionError> {
        let (details, _) = observation(engine, binding, cancel)?;
        if screen::composer_signals(&details.snapshot.provider).is_empty() {
            return Ok(ComposerReady::NoSignal);
        }
        let start = Instant::now();
        let mut ready_since = None;
        let mut blocked_since = None;
        let mut last_blocker = None;
        loop {
            let (details, lines) = observation(engine, binding, cancel)?;
            let collapsed = screen::collapse_whitespace(&lines.concat());
            let blocked = screen_blocker(&details.snapshot.provider, &lines);
            if let Some(signal) = blocked {
                last_blocker = Some(signal);
                ready_since = None;
                if blocked_since.get_or_insert_with(Instant::now).elapsed()
                    >= self.timing.composer_stable
                {
                    return Ok(ComposerReady::Blocked(signal));
                }
            } else if screen::contains_composer(&details.snapshot.provider, &collapsed) {
                blocked_since = None;
                if ready_since.get_or_insert_with(Instant::now).elapsed()
                    >= self.timing.composer_stable
                {
                    return Ok(ComposerReady::Ready);
                }
            } else {
                ready_since = None;
                blocked_since = None;
            }
            if start.elapsed() >= self.timing.composer_wait {
                return Ok(last_blocker.map_or(ComposerReady::Unknown, ComposerReady::Blocked));
            }
            pause(self.timing.poll, cancel).await?;
        }
    }

    async fn wait_quiet(
        &self,
        engine: &SessionEngine,
        binding: SessionBinding,
        cancel: &TaskCancellation,
    ) -> Result<(), SessionError> {
        let start = Instant::now();
        loop {
            let (details, _) = observation(engine, binding, cancel)?;
            // Unlike submit-settle, source waitForInputReady requires at least
            // one output before quiet can win over the timeout.
            if details.last_output_at.is_some_and(|at| {
                Timestamp::now().duration_since(at).unwrap_or_default() >= self.timing.quiet
            }) || start.elapsed() >= self.timing.quiet_wait
            {
                return Ok(());
            }
            pause(Duration::from_millis(50).min(self.timing.poll), cancel).await?;
        }
    }

    async fn wait_echo(
        &self,
        engine: &SessionEngine,
        binding: SessionBinding,
        marker: &str,
        max_wait: Duration,
        cancel: &TaskCancellation,
    ) -> Result<bool, SessionError> {
        let start = Instant::now();
        loop {
            let (_, lines) = observation(engine, binding, cancel)?;
            if screen::echo_visible(&screen::collapse_whitespace(&lines.concat()), marker) {
                return Ok(true);
            }
            if start.elapsed() >= max_wait {
                return Ok(false);
            }
            pause(self.timing.poll, cancel).await?;
        }
    }

    async fn confirm(
        &self,
        engine: &SessionEngine,
        binding: SessionBinding,
        marker: &str,
        attempts: usize,
        cancel: &TaskCancellation,
    ) -> Result<InitialPromptOutcome, SessionError> {
        for retried in [false, true] {
            let evidence = match self.wait_clear(engine, binding, marker, cancel).await? {
                ComposerClear::Unknown => Some(DeliveryEvidence::EchoObserved),
                ComposerClear::Cleared => Some(DeliveryEvidence::ComposerCleared),
                ComposerClear::Stuck => None,
            };
            if let Some(evidence) = evidence {
                return Ok(InitialPromptOutcome::Delivered {
                    evidence,
                    attempts,
                    composer_enter_retried: retried,
                });
            }
            if retried {
                break;
            }
            let (details, _) = observation(engine, binding, cancel)?;
            if details.approval.record.is_some() {
                return Ok(InitialPromptOutcome::Delivered {
                    evidence: DeliveryEvidence::ApprovalObservedAfterEcho,
                    attempts,
                    composer_enter_retried: retried,
                });
            }
            let receipt = engine
                .submit(
                    binding,
                    InputRequest {
                        bytes: b"\r".to_vec(),
                        authority: InputAuthority::InitialPrompt,
                    },
                    Timestamp::now(),
                    cancel,
                )
                .await;
            match input_result(receipt, cancel)? {
                WriteResult::Deferred | WriteResult::Unconfirmed(_) => {
                    return Ok(InitialPromptOutcome::Delivered {
                        evidence: DeliveryEvidence::EchoObservedSubmitUnconfirmed,
                        attempts,
                        composer_enter_retried: true,
                    });
                }
                WriteResult::Written => {}
            }
        }
        Ok(InitialPromptOutcome::Failed { detail: "the initial prompt is still sitting in the child's composer and two Enters did not submit it; it was NOT delivered. Open the child session and press Enter once, then instruct it with orchestrate send".into() })
    }

    async fn wait_clear(
        &self,
        engine: &SessionEngine,
        binding: SessionBinding,
        marker: &str,
        cancel: &TaskCancellation,
    ) -> Result<ComposerClear, SessionError> {
        let start = Instant::now();
        let mut clear_since = None;
        loop {
            let (details, lines) = observation(engine, binding, cancel)?;
            let Some(text) = screen::composer_text(&details.snapshot.provider, &lines)
                .filter(|_| !marker.is_empty())
            else {
                return Ok(ComposerClear::Unknown);
            };
            if text.contains(marker) {
                clear_since = None;
                if start.elapsed() >= self.timing.confirm_wait {
                    return Ok(ComposerClear::Stuck);
                }
            } else if clear_since.get_or_insert_with(Instant::now).elapsed()
                >= self.timing.clear_stable
            {
                return Ok(ComposerClear::Cleared);
            }
            pause(self.timing.poll, cancel).await?;
        }
    }
}

fn observation(
    engine: &SessionEngine,
    binding: SessionBinding,
    cancel: &TaskCancellation,
) -> Result<(SessionDetails, Vec<String>), SessionError> {
    if cancel.token().is_cancelled() {
        return Err(SessionError::Cancelled);
    }
    engine.initial_prompt_observation(binding)
}
async fn pause(duration: Duration, cancel: &TaskCancellation) -> Result<(), SessionError> {
    tokio::select! {
        biased;
        _ = cancel.token().cancelled() => Err(SessionError::Cancelled),
        _ = tokio::time::sleep(duration.max(Duration::from_millis(1))) => Ok(()),
    }
}
enum ComposerReady {
    Ready,
    Blocked(&'static str),
    Unknown,
    NoSignal,
}
enum ComposerClear {
    Cleared,
    Stuck,
    Unknown,
}
enum WriteResult {
    Written,
    Deferred,
    Unconfirmed(String),
}
fn input_result(
    receipt: InputReceipt,
    cancel: &TaskCancellation,
) -> Result<WriteResult, SessionError> {
    if cancel.token().is_cancelled() {
        return Err(SessionError::Cancelled);
    }
    match receipt.disposition {
        InputDisposition::TransportWritten { .. } => Ok(WriteResult::Written),
        InputDisposition::Deferred { .. } => Ok(WriteResult::Deferred),
        InputDisposition::Failed { detail, .. } => Ok(WriteResult::Unconfirmed(detail)),
        InputDisposition::MissingSession => Err(SessionError::NotFound(receipt.binding.session)),
        InputDisposition::StaleBinding => Err(SessionError::StaleBinding),
        InputDisposition::AuthenticationExpired => Err(SessionError::AuthenticationExpired),
    }
}

struct DeliveryGuard {
    engine: Arc<SessionEngine>,
    callbacks: Arc<dyn InitialPromptCallbacks>,
    binding: SessionBinding,
    recorded: bool,
    cleared: bool,
    finished: bool,
}
impl Drop for DeliveryGuard {
    fn drop(&mut self) {
        if !self.cleared {
            self.engine.clear_initial_gate_scoped(self.binding);
        }
        if !self.recorded {
            // A callback panic while the future itself is unwinding must not
            // double-panic and abort the entire Hub. Its task owner still
            // records cancellation/panic; no asynchronous work starts here.
            let recorded = catch_unwind(AssertUnwindSafe(|| {
                self.callbacks.record_outcome(
                    self.binding,
                    &InitialPromptOutcome::Interrupted,
                    Timestamp::now(),
                )
            }));
            if let Ok(Err(error)) = recorded {
                let _ = catch_unwind(AssertUnwindSafe(|| {
                    self.callbacks
                        .warning("record interrupted initial prompt", &error)
                }));
            }
        }
        if !self.finished {
            // Drop cannot await flush/board/parent I/O, nor create detached work.
            let _ = catch_unwind(AssertUnwindSafe(|| {
                self.callbacks.warning("initial prompt interrupted; synchronous gate cleanup attempted, pending-input flush/async notices incomplete", &SessionError::Cancelled)
            }));
        }
    }
}
