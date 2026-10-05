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
        let session = state.session(binding)?;
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
        let mut effects = CoreEffects::default();
        if let Some(database) = session.db_id {
            effects
                .0
                .push(CoreEffect::Persist(PersistenceEffect::OrchestrationMeta {
                    session: binding.session,
                    database,
                    meta: SessionOrchestrationMeta {
                        parent,
                        role: session.snapshot.role.clone(),
                        auto: session.snapshot.auto,
                        depth: session.snapshot.depth,
                        orchestration: session.snapshot.orchestration_id.clone(),
                        board_path: session.snapshot.board_path.clone(),
                    },
                }));
        }
        state
            .admission
            .observe_session(binding.session, AdmissionSession { parent, provider })
            .map_err(|e| SessionError::InvalidRequest(format!("relay parent admission: {e:?}")))?;
        effects.0.push(CoreEffect::Broadcast(update));
        Ok(state.route(effects))
    }
}
