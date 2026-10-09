//! Relay metadata and completion reuse the single canonical session owner.
use super::*;
impl SessionEngine {
    /// Atomically revoke immutable relay launch identities and snapshot their
    /// current bindings. This closes admission before cleanup releases its run
    /// lock, including cold wrappers that have not reached pre-ACK yet.
    pub(crate) fn revoke_relay_children(
        &self,
        _orchestration: &OrchestrationId,
        labels: &[String],
        dismiss_current: bool,
    ) -> Result<Vec<SessionBinding>, SessionError> {
        let mut state = lock(&self.state);
        let children = state
            .sessions
            .values()
            .filter(|session| {
                !session.snapshot.launch_label.is_empty()
                    && labels.contains(&session.snapshot.launch_label)
            })
            .map(|session| session.binding)
            .collect::<Vec<_>>();
        if !dismiss_current && !children.is_empty() {
            return Err(SessionError::InvalidRequest(
                "prior relay children are still present".into(),
            ));
        }
        state
            .revoked_relay_children
            .extend(labels.iter().filter(|label| !label.is_empty()).cloned());
        Ok(children)
    }

    /// Source relayReattachMatch.applyLocked. The caller supplies an already
    /// matched immutable relay identity; mutable display labels grant nothing.
    pub fn apply_relay_binding(
        &self,
        binding: SessionBinding,
        parent: LiveSessionId,
        orchestration: OrchestrationId,
        board_path: String,
        role: String,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError> {
        timestamp(now)?;
        let mut state = lock(&self.state);
        let (previous, update, parent, provider, persisted) = {
            let session = state.session(binding)?;
            let previous = session.snapshot.clone();
            session.snapshot.orchestration_id = orchestration;
            session.snapshot.board_path = board_path;
            if !role.is_empty() {
                session.snapshot.parent_session_id = parent;
                session.snapshot.role = role;
                session.snapshot.auto = true;
                if session.snapshot.depth == 0 {
                    session.snapshot.depth = 1;
                }
            }
            let update = session.update_message();
            let parent = session.snapshot.parent_session_id;
            let provider = session.snapshot.provider.clone();
            let persisted = session.db_id.map(|database| {
                (
                    database,
                    SessionOrchestrationMeta {
                        parent,
                        role: session.snapshot.role.clone(),
                        auto: session.snapshot.auto,
                        depth: session.snapshot.depth,
                        orchestration: session.snapshot.orchestration_id.clone(),
                        board_path: session.snapshot.board_path.clone(),
                    },
                )
            });
            (previous, update, parent, provider, persisted)
        };
        if let Err(error) = state
            .admission
            .observe_session(binding.session, AdmissionSession { parent, provider })
        {
            // The state lock has remained held since validating this binding;
            // admission observation cannot remove or replace its session.
            state
                .sessions
                .get_mut(&binding.session)
                .expect("relay session remains present under the state lock")
                .snapshot = previous;
            return Err(SessionError::InvalidRequest(format!(
                "relay parent admission: {error:?}"
            )));
        }
        let mut effects = CoreEffects::default();
        if let Some((database, meta)) = persisted {
            effects
                .0
                .push(CoreEffect::Persist(PersistenceEffect::OrchestrationMeta {
                    session: binding.session,
                    database,
                    meta,
                }));
        }
        effects.0.push(CoreEffect::Broadcast(update));
        Ok(state.route(effects))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::RuntimePaths, terminal::journal::JournalOptions};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct Io {
        applied_effects: AtomicUsize,
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
        fn apply<'a>(
            &'a self,
            effects: CoreEffects,
        ) -> CoreFuture<'a, Result<(), CoreEffectFailure>> {
            Box::pin(async move {
                self.applied_effects
                    .fetch_add(effects.0.len(), Ordering::SeqCst);
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
            Box::pin(async { SpawnWaitOutcome::Failed("synthetic no spawn".into()) })
        }
    }

    #[tokio::test]
    async fn admission_failure_restores_full_relay_snapshot_without_effects() {
        let root = tempfile::tempdir().unwrap();
        let runtime = root.path().join("trial");
        std::fs::create_dir(&runtime).unwrap();
        let paths = RuntimePaths::trial(&runtime, 49688, &root.path().join("installed")).unwrap();
        let io = Arc::new(Io::default());
        let core = SessionEngine::new(
            EngineOptions::default(),
            Arc::new(SessionJournal::new(paths, None, JournalOptions::default())),
            io.clone(),
            io.clone(),
            io.clone(),
            CoreEventBus::new(64).unwrap(),
        );
        let now = Timestamp::from_unix(1791158400, 0).unwrap();
        let binding = core
            .register(
                RegisterRequest {
                    message: proto::Message {
                        provider: "synthetic-relay-provider".into(),
                        pid: 1911,
                        cwd: "synthetic-relay-cwd".into(),
                        ..Default::default()
                    },
                    spawn_proof: None,
                },
                WrapperConnectionId(1911),
                now,
            )
            .await
            .unwrap()
            .binding;
        let ui = UiBinding {
            connection: UiConnectionId(1912),
            auth_epoch: core.auth_epoch(),
        };
        core.attach_ui(ui, None, None).unwrap();
        let (previous, update) = {
            let mut state = lock(&core.state);
            let session = state.session(binding).unwrap();
            session.db_id = Some(DbSessionId(1913));
            session.snapshot.orchestration_id = OrchestrationId("synthetic-prior-relay".into());
            session.snapshot.board_path = "synthetic-prior-board".into();
            session.snapshot.parent_session_id = LiveSessionId(1914);
            session.snapshot.role = "synthetic-prior-role".into();
            session.snapshot.auto = false;
            session.snapshot.depth = 0;
            let previous = session.snapshot.clone();
            // Deliberately construct the admission rejection under the lock.
            // Normal update admission rejects live sessions before this state.
            state.admission.dismiss_session(binding.session);
            let update = state
                .admission
                .begin_provider_update("synthetic-relay-provider")
                .unwrap();
            (previous, update)
        };
        for role in ["synthetic-new-role", ""] {
            let result = core.apply_relay_binding(
                binding,
                LiveSessionId(1915),
                OrchestrationId("synthetic-new-relay".into()),
                "synthetic-new-board".into(),
                role.into(),
                now,
            );
            assert!(
                matches!(result, Err(SessionError::InvalidRequest(ref detail))
                if detail.contains("AlreadyUpdating"))
            );
            let state = lock(&core.state);
            assert!(state.sessions[&binding.session].snapshot == previous);
            assert!(state.uis[&ui.connection].queued.is_empty());
            assert_eq!(io.applied_effects.load(Ordering::SeqCst), 0);
            assert_eq!(
                state
                    .admission
                    .provider_session_count("synthetic-relay-provider"),
                0
            );
        }
        assert!(lock(&core.state).admission.end_provider_update(&update));
    }
}
