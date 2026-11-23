//! Structured headless observations belong to the existing session incarnation.
//! The Hub issues the run identity at registration; only the bound wrapper can
//! replace the snapshot, and a newer sequence always replaces (never adds).
use super::*;
use crate::orchestration::headless_progress::{HeadlessProgress, ProgressState};

impl SessionEngine {
    pub fn observe_headless_progress(
        &self,
        binding: SessionBinding,
        progress: HeadlessProgress,
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
        session.snapshot.headless_progress = Some(progress.clone());
        Ok(
            state.route(CoreEffects(vec![CoreEffect::BroadcastHeadlessProgress(
                HeadlessProgressNotification::new(binding.session.0, progress),
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
            // An unreadable record only marks the progress incomplete; the
            // process owner's end frame decides success, as on the wrapper.
            if end.exit_code != 0 || end.declared_state != "completed" {
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
            HeadlessProgressNotification::new(self.binding.session.0, progress.clone()),
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
    /// A headless child registered through the same one-use proof a real
    /// spawn issues, so the admission table records its parent.
    async fn register_child(core: &SessionEngine, parent: LiveSessionId, pid: i64) -> Registration {
        let proof = SpawnRegistrationProof::issue().unwrap();
        {
            let mut state = lock(&core.state);
            let lease = state.admission.begin_provider_spawn("claude").unwrap();
            state.spawn_proofs.insert(
                crate::approval::identity::digest(proof.as_header_value()),
                PendingSpawn {
                    lease,
                    metadata: SpawnRegistrationMetadata {
                        parent,
                        ..Default::default()
                    },
                },
            );
        }
        let mut registration = core
            .register(
                RegisterRequest {
                    message: proto::Message {
                        provider: "claude".into(),
                        pid,
                        execution_mode: "headless".into(),
                        cwd: "synthetic-progress".into(),
                        ..Default::default()
                    },
                    spawn_proof: Some(proof.as_header_value().to_owned()),
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
    #[tokio::test]
    async fn registration_issues_the_run_id_only_to_headless_wrappers() {
        let (_root, core) = fixture();
        let headless = register(&core, true, 1).await;
        let run_id = headless.snapshot.headless_progress.unwrap().run_id;
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&headless.registered.data).unwrap(),
            serde_json::json!({ "headless_run_id": run_id })
        );
        let interactive = register(&core, false, 2).await;
        assert!(interactive.snapshot.headless_progress.is_none());
        assert!(interactive.registered.data.is_empty());
        // The new card is announced with its (still unknown) progress.
        let fresh = core
            .register(
                RegisterRequest {
                    message: proto::Message {
                        provider: "claude".into(),
                        pid: 3,
                        execution_mode: "headless".into(),
                        cwd: "synthetic-progress".into(),
                        ..Default::default()
                    },
                    spawn_proof: None,
                },
                WrapperConnectionId(3),
                now(),
            )
            .await
            .unwrap();
        let ui = UiBinding {
            connection: UiConnectionId(9),
            auth_epoch: core.auth_epoch(),
        };
        assert!(
            core.attach_ui(ui, None, None)
                .unwrap()
                .sessions
                .iter()
                .any(|s| s.id == fresh.binding.session
                    && s.headless_progress
                        .as_ref()
                        .is_some_and(|p| p.sequence == 0))
        );
    }
    #[tokio::test]
    async fn clean_end_completes_even_when_a_record_was_unreadable() {
        let (_root, core) = fixture();
        let one = register(&core, true, 1).await;
        let mut progress = one.snapshot.headless_progress.unwrap();
        progress.sequence = 1;
        progress.state = ProgressState::Finishing;
        progress.data_incomplete = true;
        core.observe_headless_progress(one.binding, progress)
            .unwrap();
        core.observe_end(
            one.binding,
            SessionEnd {
                declared_state: "completed".into(),
                exit_code: 0,
                reason: String::new(),
            },
            now(),
        )
        .unwrap();
        let final_progress = core
            .snapshot(one.binding.session)
            .unwrap()
            .headless_progress
            .unwrap();
        assert_eq!(final_progress.state, ProgressState::Completed);
        assert!(final_progress.data_incomplete);
        assert!(final_progress.final_cost_missing);
        assert!(final_progress.ended_at_ms.is_some());
    }
    #[tokio::test]
    async fn a_completed_headless_card_keeps_no_admission_slot() {
        let (_root, core) = fixture();
        let end = |state: &str, code: i64| SessionEnd {
            declared_state: state.into(),
            exit_code: code,
            reason: String::new(),
        };
        // Same order as the wrapper: end frame first, then the transport closes.
        let done = register(&core, true, 1).await;
        let failed = register(&core, true, 2).await;
        let interactive = register(&core, false, 3).await;
        assert_eq!(core.provider_session_count("claude"), 3);
        core.observe_end(done.binding, end("completed", 0), now())
            .unwrap();
        // The process is still exiting until the transport closes.
        assert_eq!(core.provider_session_count("claude"), 3);
        core.disconnected(done.binding, now()).unwrap();
        core.observe_end(failed.binding, end("error", 1), now())
            .unwrap();
        core.disconnected(failed.binding, now()).unwrap();
        core.observe_end(interactive.binding, end("completed", 0), now())
            .unwrap();
        core.disconnected(interactive.binding, now()).unwrap();
        // Only the completed headless run left the count; its card and final
        // progress remain for the UI until dismissal.
        assert_eq!(core.provider_session_count("claude"), 2);
        assert_eq!(
            core.snapshot(done.binding.session)
                .unwrap()
                .headless_progress
                .unwrap()
                .state,
            ProgressState::Completed
        );
        let ui = UiBinding {
            connection: UiConnectionId(7),
            auth_epoch: core.auth_epoch(),
        };
        core.attach_ui(ui, None, None).unwrap();
        core.dismiss_from_ui(ui, failed.binding.session, now())
            .unwrap();
        core.dismiss_from_ui(ui, interactive.binding.session, now())
            .unwrap();
        // With nothing else retained, a CLI update is admitted even though the
        // completed headless card is still listed.
        assert_eq!(core.provider_session_count("claude"), 0);
        let lease = core.begin_provider_update("claude").unwrap();
        core.end_provider_update(lease);
        assert!(core.snapshot(done.binding.session).is_some());
    }
    #[tokio::test]
    async fn completed_headless_children_free_the_per_parent_and_total_limits() {
        let (_root, core) = fixture();
        let conductor = register(&core, false, 1).await;
        let parent = conductor.binding.session;
        let end = |state: &str, code: i64| SessionEnd {
            declared_state: state.into(),
            exit_code: code,
            reason: String::new(),
        };
        let request = AdmissionRequest {
            parent,
            slots: 1,
            origin: VerifiedSpawnOrigin::Autonomous,
            replace: None,
        };
        // The default automatic limit, with a total budget that the conductor
        // plus a full set of children would also exhaust.
        let limits = AdmissionLimits {
            max_children_per_parent: 10,
            max_total_sessions: 11,
        };
        let mut children = Vec::new();
        for pid in 2..=11 {
            let child = register_child(&core, parent, pid).await;
            assert_eq!(
                core.snapshot(child.binding.session)
                    .unwrap()
                    .parent_session_id,
                parent
            );
            core.observe_end(child.binding, end("completed", 0), now())
                .unwrap();
            core.disconnected(child.binding, now()).unwrap();
            children.push(child);
        }
        // Ten completed cards are still listed, yet the 11th automatic spawn
        // is admitted by both the per-parent and the total limit.
        assert!(
            children
                .iter()
                .all(|c| core.snapshot(c.binding.session).is_some())
        );
        let reservation = core.reserve_children(request.clone(), limits).unwrap();
        assert!(core.release_children(&reservation.id));
        // A child that failed keeps its slot until its card is closed.
        let failed = register_child(&core, parent, 12).await;
        core.observe_end(failed.binding, end("error", 1), now())
            .unwrap();
        core.disconnected(failed.binding, now()).unwrap();
        let one = AdmissionLimits {
            max_children_per_parent: 1,
            max_total_sessions: 2,
        };
        assert!(matches!(
            core.reserve_children(request.clone(), one),
            Err(AdmissionError::ChildrenPerParent { used: 1, .. })
        ));
        let ui = UiBinding {
            connection: UiConnectionId(7),
            auth_epoch: core.auth_epoch(),
        };
        core.attach_ui(ui, None, None).unwrap();
        core.dismiss_from_ui(ui, failed.binding.session, now())
            .unwrap();
        let reservation = core.reserve_children(request, one).unwrap();
        assert!(core.release_children(&reservation.id));
    }
    #[tokio::test]
    async fn a_headless_run_cannot_be_reattached() {
        let (_root, core) = fixture();
        let one = register(&core, true, 1).await;
        let error = core
            .reattach(
                ReattachRequest {
                    restored_metadata: None,
                    message: proto::Message {
                        session_id: one.binding.session.0,
                        provider: "claude".into(),
                        pid: 1,
                        cwd: "synthetic-progress".into(),
                        started_at: one.snapshot.started_at.clone(),
                        ..Default::default()
                    },
                },
                WrapperConnectionId(2),
                now(),
            )
            .await;
        assert!(matches!(error, Err(SessionError::InvalidRequest(_))));
    }
}
