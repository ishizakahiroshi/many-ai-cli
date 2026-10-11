//! Structured headless observations belong to the existing session incarnation.
use super::*;
use crate::orchestration::headless_progress::{HeadlessProgress, ProgressState};

impl SessionEngine {
    pub fn observe_headless_progress(
        &self,
        binding: SessionBinding,
        mut progress: HeadlessProgress,
    ) -> Result<CoreEffects, SessionError> {
        progress
            .validate()
            .map_err(|_| SessionError::InvalidRequest("invalid headless progress".into()))?;
        let mut state = lock(&self.state);
        let session = state.session(binding)?;
        if !session.connected
            || session.snapshot.execution_mode != "headless"
            || session.ended.is_some()
        {
            return Err(SessionError::StaleBinding);
        }
        let Some(previous) = session.snapshot.headless_progress.as_ref() else {
            return Err(SessionError::InvalidRequest(
                "headless progress was not admitted".into(),
            ));
        };
        if progress.run_id != previous.run_id {
            return Err(SessionError::StaleBinding);
        }
        if progress.sequence <= previous.sequence {
            return Ok(CoreEffects::default());
        }
        if progress.elapsed_ms < previous.elapsed_ms
            || (previous.sequence > 0 && progress.started_at_ms != previous.started_at_ms)
        {
            return Err(SessionError::InvalidRequest(
                "headless clock regressed".into(),
            ));
        }
        // This label comes only from the server-owned spawn selection.
        progress.billing_mode = previous.billing_mode.clone();
        session.snapshot.headless_progress = Some(progress.clone());
        Ok(
            state.route(CoreEffects(vec![CoreEffect::BroadcastHeadlessProgress(
                HeadlessProgressNotification {
                    r#type: "headless_progress".into(),
                    session_id: binding.session.0,
                    headless_progress: progress,
                },
            )])),
        )
    }
}

impl Session {
    pub(super) fn finalize_headless(
        &mut self,
        disconnected: bool,
        end: Option<&SessionEnd>,
        effects: &mut CoreEffects,
    ) {
        let Some(progress) = self.snapshot.headless_progress.as_mut() else {
            return;
        };
        let terminal = matches!(
            progress.state,
            ProgressState::Completed
                | ProgressState::Failed
                | ProgressState::Canceled
                | ProgressState::Disconnected
        );
        if disconnected && terminal {
            return;
        }
        if disconnected {
            progress.state = ProgressState::Disconnected;
            progress.data_incomplete = true;
        } else if let Some(end) = end {
            if end.exit_code != 0 || end.declared_state != "completed" || progress.data_incomplete {
                // Preserve an explicit canceled state emitted by the process owner.
                if progress.state != ProgressState::Canceled {
                    progress.state = ProgressState::Failed;
                }
            } else if !terminal {
                progress.state = ProgressState::Completed;
            }
        }
        progress.final_cost_missing =
            progress.cost.kind != crate::orchestration::headless_progress::CostKind::Result;
        progress.active_tools.clear();
        progress.sequence = progress.sequence.saturating_add(1);
        progress.ended_at_ms.get_or_insert(
            progress
                .started_at_ms
                .saturating_add(progress.elapsed_ms.min(i64::MAX as u64) as i64),
        );
        effects.0.push(CoreEffect::BroadcastHeadlessProgress(
            HeadlessProgressNotification {
                r#type: "headless_progress".into(),
                session_id: self.binding.session.0,
                headless_progress: progress.clone(),
            },
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::RuntimePaths,
        terminal::{events::CoreEventBus, journal::JournalOptions},
    };
    struct Io;
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
            Box::pin(async { SpawnWaitOutcome::Failed("synthetic".into()) })
        }
    }
    fn now() -> Timestamp {
        Timestamp::from_unix(1_791_584_000, 0).unwrap()
    }
    fn fixture() -> (tempfile::TempDir, SessionEngine) {
        let root = tempfile::tempdir().unwrap();
        let runtime = root.path().join("trial");
        std::fs::create_dir(&runtime).unwrap();
        let paths = RuntimePaths::trial(&runtime, 49688, &root.path().join("installed")).unwrap();
        let core = SessionEngine::new(
            EngineOptions::default(),
            Arc::new(SessionJournal::new(paths, None, JournalOptions::default())),
            Arc::new(Io),
            Arc::new(Io),
            Arc::new(Io),
            CoreEventBus::new(64).unwrap(),
        );
        (root, core)
    }
    async fn register(core: &SessionEngine, headless: bool, pid: i64) -> Registration {
        let mut registration = core
            .register(
                RegisterRequest {
                    message: proto::Message {
                        provider: "claude".into(),
                        pid,
                        execution_mode: if headless { "headless" } else { "" }.into(),
                        cwd: "synthetic-progress".into(),
                        ..Default::default()
                    },
                    spawn_proof: None,
                },
                WrapperConnectionId(pid as u64),
                now(),
            )
            .await
            .unwrap();
        registration.after_registered = CoreEffects::default();
        registration
    }
    #[tokio::test]
    async fn binding_run_sequence_and_mode_are_checked_before_mutation() {
        let (_root, core) = fixture();
        let one = register(&core, true, 1).await;
        let two = register(&core, true, 2).await;
        let interactive = register(&core, false, 3).await;
        let mut progress = one.snapshot.headless_progress.unwrap();
        progress.sequence = 1;
        progress.billing_mode = "api-key".into();
        progress.observed_turns = 2;
        assert!(
            core.observe_headless_progress(two.binding, progress.clone())
                .is_err()
        );
        assert!(
            core.observe_headless_progress(interactive.binding, progress.clone())
                .is_err()
        );
        let mut stale = one.binding;
        stale.wrapper = WrapperConnectionId(99);
        assert!(
            core.observe_headless_progress(stale, progress.clone())
                .is_err()
        );
        core.observe_headless_progress(one.binding, progress.clone())
            .unwrap();
        let accepted = core
            .snapshot(one.binding.session)
            .unwrap()
            .headless_progress
            .unwrap();
        assert_eq!(accepted.billing_mode, "");
        assert_eq!(accepted.observed_turns, 2);
        progress.observed_turns = 90;
        assert!(
            core.observe_headless_progress(one.binding, progress)
                .unwrap()
                .0
                .is_empty()
        );
        assert_eq!(
            core.snapshot(one.binding.session)
                .unwrap()
                .headless_progress
                .unwrap()
                .observed_turns,
            2
        );
    }
    #[tokio::test]
    async fn snapshot_and_coalesced_live_progress_use_the_priming_lane() {
        let (_root, core) = fixture();
        let one = register(&core, true, 1).await;
        let ui = UiBinding {
            connection: UiConnectionId(8),
            auth_epoch: core.auth_epoch(),
        };
        let prime = core.attach_ui(ui, None, None).unwrap();
        assert_eq!(
            prime.sessions[0]
                .headless_progress
                .as_ref()
                .unwrap()
                .sequence,
            0
        );
        let mut progress = one.snapshot.headless_progress.unwrap();
        for sequence in 1..=3 {
            progress.sequence = sequence;
            progress.elapsed_ms = sequence * 1000;
            assert!(
                core.observe_headless_progress(one.binding, progress.clone())
                    .unwrap()
                    .0
                    .is_empty()
            );
        }
        let queued = core.finish_ui_priming(ui).unwrap();
        assert_eq!(queued.0.len(), 1);
        assert!(
            matches!(&queued.0[0], CoreEffect::SendUiHeadlessProgress { event, best_effort:false, .. } if event.headless_progress.sequence == 3)
        );
        assert!(core.finish_ui_priming(ui).unwrap().0.is_empty());
        progress.sequence = 4;
        assert!(matches!(
            &core
                .observe_headless_progress(one.binding, progress)
                .unwrap()
                .0[0],
            CoreEffect::SendUiHeadlessProgress {
                best_effort: true,
                ..
            }
        ));
    }
    #[tokio::test]
    async fn end_failure_and_disconnect_freeze_progress_without_changing_cost() {
        let (_root, core) = fixture();
        let one = register(&core, true, 1).await;
        let mut progress = one.snapshot.headless_progress.unwrap();
        progress.sequence = 1;
        progress.state = ProgressState::Finishing;
        progress.elapsed_ms = 2100;
        progress.cost = crate::orchestration::headless_progress::ProgressCost {
            amount_usd: Some(0.04),
            currency: "USD".into(),
            kind: crate::orchestration::headless_progress::CostKind::Result,
            coverage: crate::orchestration::headless_progress::CostCoverage::CliResult,
        };
        core.observe_headless_progress(one.binding, progress)
            .unwrap();
        core.observe_end(
            one.binding,
            SessionEnd {
                declared_state: "error".into(),
                exit_code: 1,
                reason: String::new(),
            },
            now(),
        )
        .unwrap();
        core.disconnected(one.binding, now()).unwrap();
        let final_progress = core
            .snapshot(one.binding.session)
            .unwrap()
            .headless_progress
            .unwrap();
        assert_eq!(final_progress.state, ProgressState::Failed);
        assert_eq!(final_progress.cost.amount_usd, Some(0.04));
        assert_eq!(final_progress.elapsed_ms, 2100);
        let two = register(&core, true, 2).await;
        core.disconnected(two.binding, now()).unwrap();
        assert_eq!(
            core.snapshot(two.binding.session)
                .unwrap()
                .headless_progress
                .unwrap()
                .state,
            ProgressState::Disconnected
        );
    }
}
