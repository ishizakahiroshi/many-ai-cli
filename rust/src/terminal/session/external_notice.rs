//! Conditional external notices. Admission and frame reservation happen under
//! the session lock after owning the input ticket; no reconnect replay or retry.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoticeTarget {
    pub hub_instance: String,
    pub session_id: i64,
    pub incarnation: u64,
    pub wrapper: u64,
    pub auth_epoch: u64,
    pub started_at: String,
    pub cwd: String,
    pub codex_session_id: String,
}
impl NoticeTarget {
    fn binding(&self) -> SessionBinding {
        SessionBinding {
            session: LiveSessionId(self.session_id),
            incarnation: SessionIncarnation(self.incarnation),
            wrapper: WrapperConnectionId(self.wrapper),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeWrite {
    Written,
    Blocked,
    InvalidTarget,
    Unknown,
}

impl SessionEngine {
    pub fn notice_target(&self, id: LiveSessionId) -> Option<NoticeTarget> {
        let state = lock(&self.state);
        let s = state.sessions.get(&id)?;
        if !s.connected
            || terminal(&s.snapshot.state)
            || (s.snapshot.provider == "codex" && s.transcript.agent_session_id.is_empty())
        {
            return None;
        }
        Some(NoticeTarget {
            hub_instance: self.options.hub_instance.clone(),
            session_id: id.0,
            incarnation: s.binding.incarnation.0,
            wrapper: s.binding.wrapper.0,
            auth_epoch: state.auth_epoch.0,
            started_at: s.snapshot.started_at.clone(),
            cwd: s.snapshot.cwd.clone(),
            codex_session_id: if s.snapshot.provider == "codex" {
                s.transcript.agent_session_id.clone()
            } else {
                String::new()
            },
        })
    }
    pub fn deliver_external_notice<'a>(
        &'a self,
        expected: &'a NoticeTarget,
        text: &'a str,
        cancel: &'a TaskCancellation,
    ) -> CoreFuture<'a, NoticeWrite> {
        let binding = expected.binding();
        let ticket = self.input_ticket(binding);
        Box::pin(async move {
            let Ok(ticket) = ticket else {
                return NoticeWrite::InvalidTarget;
            };
            if !ticket.wait(cancel).await {
                return NoticeWrite::Blocked;
            }
            let frame = {
                let mut state = lock(&self.state);
                if expected.hub_instance != self.options.hub_instance
                    || expected.auth_epoch != state.auth_epoch.0
                {
                    return NoticeWrite::InvalidTarget;
                }
                let Ok(s) = state.session(binding) else {
                    return NoticeWrite::InvalidTarget;
                };
                if !s.connected
                    || terminal(&s.snapshot.state)
                    || s.snapshot.started_at != expected.started_at
                    || s.snapshot.cwd != expected.cwd
                    || (s.snapshot.provider == "codex"
                        && (s.transcript.agent_session_id.is_empty()
                            || s.transcript.agent_session_id != expected.codex_session_id))
                {
                    return NoticeWrite::InvalidTarget;
                }
                if cancel.token().is_cancelled()
                    || s.input.initial_prompt_phase()
                    || s.input.gated(Timestamp::now())
                    || s.input.pending_len() != 0
                    || s.input.resend_len() != 0
                    || s.input.inflight_len() != 0
                    || s.snapshot.activity.awaiting_user
                    || s.snapshot.activity.awaiting_approval
                    || s.approval.record().is_some()
                    || !s.snapshot.activity.is_idle()
                {
                    return NoticeWrite::Blocked;
                }
                let lines = s.vt.lines();
                let screen: String = lines
                    .concat()
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .collect();
                // Fail closed for providers whose normal composer is unknown.
                let composer = match s.snapshot.provider.as_str() {
                    "codex" => {
                        lines
                            .iter()
                            .rev()
                            .filter_map(|line| line.trim().strip_prefix('›'))
                            .next()
                            .is_some_and(|composer| composer.trim() == "Ask Codex to do anything")
                            && !screen.contains("Sidefrommainthread")
                    }
                    _ => false,
                };
                if !composer
                    || crate::orchestration::initial_prompt::screen_blocker(
                        &s.snapshot.provider,
                        &lines,
                    )
                    .is_some()
                {
                    return NoticeWrite::Blocked;
                }
                // The caller is a typed trusted controller; reject control bytes
                // anyway so no external data can become terminal framing.
                if text.len() > 2048 || text.chars().any(char::is_control) {
                    return NoticeWrite::InvalidTarget;
                }
                s.snapshot.activity.output_idle = false;
                let frame = s.input.reserve(
                    binding.wrapper,
                    format!("\x1b[200~{text}\x1b[201~\r").into_bytes(),
                );
                // Remove replay ownership under the same lock, BEFORE awaiting
                // transport. Disconnect can run during send(), so a later RAII
                // release would be too late to prevent reconnect replay.
                s.input.release(binding.wrapper, frame.seq);
                frame
            };
            // Any error after transport invocation is uncertain. Never feed
            // this notice into pending/resend queues and never retry Enter.
            let sent = self
                .transport
                .send(
                    binding,
                    proto::Message {
                        r#type: "pty_input".into(),
                        session_id: binding.session.0,
                        input_seq: frame.seq.0,
                        data: frame.bytes,
                        ..Default::default()
                    },
                )
                .await;
            if sent.is_err() {
                return NoticeWrite::Unknown;
            }
            NoticeWrite::Written
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{
        config::RuntimePaths,
        terminal::{events::CoreEventBus, journal::JournalOptions},
    };
    #[derive(Default)]
    pub struct Io {
        pub sent: Mutex<Vec<Vec<u8>>>,
        pub fail: std::sync::atomic::AtomicBool,
        disconnect_on_send: std::sync::atomic::AtomicBool,
        engine: Mutex<std::sync::Weak<SessionEngine>>,
    }
    impl WrapperTransport for Io {
        fn send<'a>(
            &'a self,
            binding: SessionBinding,
            m: proto::Message,
        ) -> CoreFuture<'a, Result<(), SessionError>> {
            Box::pin(async move {
                lock(&self.sent).push(m.data);
                if self
                    .disconnect_on_send
                    .load(std::sync::atomic::Ordering::SeqCst)
                {
                    if let Some(core) = lock(&self.engine).upgrade() {
                        let _ = core.disconnected(binding, Timestamp::now());
                    }
                }
                if self.fail.load(std::sync::atomic::Ordering::SeqCst) {
                    Err(SessionError::Transport("synthetic uncertain".into()))
                } else {
                    Ok(())
                }
            })
        }
    }
    impl CoreEffectSink for Io {
        fn apply<'a>(&'a self, _: CoreEffects) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
            Box::pin(async { Ok(()) })
        }
    }
    impl WrappedSessionSpawner for Io {
        fn spawn_and_wait<'a>(
            &'a self,
            _: WrappedSpawnSpec,
            _: Duration,
            _: &'a HttpWaitCancellation,
        ) -> CoreFuture<'a, SpawnWaitOutcome> {
            Box::pin(async { panic!("notices never spawn") })
        }
    }
    pub async fn fixture() -> (tempfile::TempDir, Arc<SessionEngine>, Arc<Io>, NoticeTarget) {
        let root = tempfile::tempdir().unwrap();
        let runtime = root.path().join("runtime");
        std::fs::create_dir(&runtime).unwrap();
        let paths = RuntimePaths::trial(&runtime, 49780, &root.path().join("installed")).unwrap();
        let io = Arc::new(Io::default());
        let core = Arc::new(SessionEngine::new(
            EngineOptions {
                hub_instance: "synthetic-hub".into(),
                ..Default::default()
            },
            Arc::new(SessionJournal::new(paths, None, JournalOptions::default())),
            io.clone(),
            io.clone(),
            io.clone(),
            CoreEventBus::new(64).unwrap(),
        ));
        *lock(&io.engine) = Arc::downgrade(&core);
        let binding = core
            .register(
                RegisterRequest {
                    message: proto::Message {
                        provider: "codex".into(),
                        pid: 1871,
                        cwd: "synthetic-cwd".into(),
                        cols: 120,
                        rows: 30,
                        ..Default::default()
                    },
                    spawn_proof: None,
                },
                WrapperConnectionId(1871),
                Timestamp::now(),
            )
            .await
            .unwrap()
            .binding;
        {
            let mut state = lock(&core.state);
            let s = state.session(binding).unwrap();
            s.input.clear_initial_gate();
            s.snapshot.activity.output_idle = true;
            s.snapshot.state = "standby".into();
            s.transcript.agent_session_id = "synthetic-codex".into();
            s.vt.write("› Ask Codex to do anything".as_bytes());
        }
        let target = core.notice_target(binding.session).unwrap();
        (root, core, io, target)
    }
    #[tokio::test]
    async fn external_notice_idle_sends_one_neutral_frame_without_retry() {
        let (_root, core, io, target) = fixture().await;
        assert_eq!(
            core.deliver_external_notice(&target, "neutral receipt", &TaskCancellation::default())
                .await,
            NoticeWrite::Written
        );
        assert_eq!(
            *lock(&io.sent),
            vec![b"\x1b[200~neutral receipt\x1b[201~\r".to_vec()]
        );
        assert_eq!(
            lock(&core.state)
                .sessions
                .get(&target.binding().session)
                .unwrap()
                .input
                .inflight_len(),
            0
        );
        let _effects = core
            .disconnected(target.binding(), Timestamp::now())
            .unwrap();
        let state = lock(&core.state);
        let s = state.sessions.get(&target.binding().session).unwrap();
        assert_eq!(s.input.pending_len(), 0);
        assert_eq!(s.input.resend_len(), 0);
    }
    #[tokio::test]
    async fn external_notice_disconnect_during_transport_does_not_queue_replay() {
        let (_root, core, io, target) = fixture().await;
        io.disconnect_on_send
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            core.deliver_external_notice(&target, "neutral", &TaskCancellation::default())
                .await,
            NoticeWrite::Written
        );
        let state = lock(&core.state);
        let s = state.sessions.get(&target.binding().session).unwrap();
        assert_eq!(s.input.inflight_len(), 0);
        assert_eq!(s.input.pending_len(), 0);
        assert_eq!(s.input.resend_len(), 0);
    }
    #[tokio::test]
    async fn external_notice_blocks_busy_approval_gate_and_unknown_composer() {
        for condition in 0..7 {
            let (_root, core, io, target) = fixture().await;
            {
                let mut state = lock(&core.state);
                let s = state.session(target.binding()).unwrap();
                match condition {
                    0 => s.snapshot.activity.workflow_active = true,
                    1 => s.snapshot.activity.awaiting_approval = true,
                    2 => s.input.set_initial_gate(Timestamp::now()),
                    3 => {
                        s.input.enqueue(b"other input".to_vec());
                    }
                    4 => {
                        s.vt = VtBuffer::new(120, 30);
                        s.vt.write(b"unknown screen");
                    }
                    5 => {
                        s.snapshot.provider = "claude".into();
                        s.vt = VtBuffer::new(120, 30);
                        s.vt.write(b"synthetic unsent draft\nshift+tab to cycle");
                    }
                    _ => {
                        s.vt = VtBuffer::new(120, 30);
                        s.vt.write(
                            "prior transcript: Ask Codex to do anything\r\n› unsent user draft"
                                .as_bytes(),
                        );
                    }
                }
            }
            assert_eq!(
                core.deliver_external_notice(&target, "neutral", &TaskCancellation::default())
                    .await,
                NoticeWrite::Blocked
            );
            assert!(lock(&io.sent).is_empty());
        }
    }
    #[tokio::test]
    async fn external_notice_revalidates_identity_inside_queued_ticket() {
        let (_root, core, io, target) = fixture().await;
        let held = core.input_ticket(target.binding()).unwrap();
        let cancel = TaskCancellation::default();
        assert!(held.wait(&cancel).await);
        let sending = core.deliver_external_notice(&target, "neutral", &cancel);
        {
            let mut state = lock(&core.state);
            state
                .session(target.binding())
                .unwrap()
                .transcript
                .agent_session_id = "replacement-codex".into();
        }
        drop(held);
        assert_eq!(sending.await, NoticeWrite::InvalidTarget);
        assert!(lock(&io.sent).is_empty());
    }
    #[tokio::test]
    async fn external_notice_stale_tuple_restart_and_transport_uncertainty() {
        let (_root, core, io, target) = fixture().await;
        let mut stale = target.clone();
        stale.hub_instance = "previous-hub".into();
        assert_eq!(
            core.deliver_external_notice(&stale, "neutral", &TaskCancellation::default())
                .await,
            NoticeWrite::InvalidTarget
        );
        stale = target.clone();
        stale.cwd = "different-cwd".into();
        assert_eq!(
            core.deliver_external_notice(&stale, "neutral", &TaskCancellation::default())
                .await,
            NoticeWrite::InvalidTarget
        );
        io.fail.store(true, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            core.deliver_external_notice(&target, "neutral", &TaskCancellation::default())
                .await,
            NoticeWrite::Unknown
        );
        let state = lock(&core.state);
        let s = state.sessions.get(&target.binding().session).unwrap();
        assert_eq!(s.input.pending_len(), 0);
        assert_eq!(s.input.resend_len(), 0);
    }
}
