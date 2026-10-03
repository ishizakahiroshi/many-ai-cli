use super::*;
impl SpawnAdmission for SessionEngine {
    fn reserve_children(
        &self,
        request: AdmissionRequest,
        limits: AdmissionLimits,
    ) -> Result<AdmissionReservation, AdmissionError> {
        let mut state = lock(&self.state);
        // HTTP proofs retain auth-revocation epochs, but are not WebSocket
        // memberships. Go permits these requests without an open browser socket.
        if request
            .origin
            .auth_epoch()
            .is_some_and(|epoch| epoch != state.auth_epoch)
        {
            return Err(AdmissionError::ParentNotFound);
        }
        state.admission.reserve_children(request, limits)
    }
    fn release_children(&self, admission: &AdmissionId) -> bool {
        lock(&self.state).admission.release_children(admission)
    }
    fn consume_children(&self, admission: &AdmissionId, slots: i64) -> bool {
        lock(&self.state)
            .admission
            .consume_children(admission, slots)
    }
    fn admission_matches(
        &self,
        admission: &AdmissionId,
        parent: LiveSessionId,
        slots: i64,
    ) -> bool {
        lock(&self.state)
            .admission
            .admission_matches(admission, parent, slots)
    }
}
impl ProviderUpdateAdmission for SessionEngine {
    fn begin_provider_spawn(
        &self,
        provider: &str,
    ) -> Result<ProviderSpawnLease, ProviderAdmissionError> {
        lock(&self.state).admission.begin_provider_spawn(provider)
    }
    fn end_provider_spawn(&self, lease: ProviderSpawnLease) {
        let mut state = lock(&self.state);
        state.spawn_proofs.retain(|_, pending| {
            pending.lease.id != lease.id || pending.lease.provider != lease.provider
        });
        state.admission.end_provider_spawn(&lease);
    }
    fn begin_provider_update(
        &self,
        provider: &str,
    ) -> Result<ProviderUpdateLease, ProviderAdmissionError> {
        lock(&self.state).admission.begin_provider_update(provider)
    }
    fn end_provider_update(&self, lease: ProviderUpdateLease) {
        lock(&self.state).admission.end_provider_update(&lease);
    }
    fn provider_session_count(&self, provider: &str) -> usize {
        lock(&self.state).admission.provider_session_count(provider)
    }
}
impl WrappedSessionSpawner for SessionEngine {
    fn spawn_and_wait<'a>(
        &'a self,
        mut spec: WrappedSpawnSpec,
        wait: Duration,
        waiter: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome> {
        Box::pin(async move {
            let lease = if let Some(id) = spec.spawn_attempt {
                ProviderSpawnLease {
                    id,
                    provider: spec.provider.clone(),
                }
            } else {
                match self.begin_provider_spawn(&spec.provider) {
                    Ok(lease) => lease,
                    Err(error) => {
                        return SpawnWaitOutcome::Failed(format!(
                            "provider admission failed: {error:?}"
                        ));
                    }
                }
            };
            let proof = match SpawnRegistrationProof::issue() {
                Ok(proof) => proof,
                Err(_) => {
                    self.end_provider_spawn(lease);
                    return SpawnWaitOutcome::Failed("registration proof generation failed".into());
                }
            };
            {
                let mut state = lock(&self.state);
                if state
                    .spawn_proofs
                    .values()
                    .any(|pending| pending.lease.id == lease.id)
                {
                    return SpawnWaitOutcome::Failed("start attempt is already launched".into());
                }
                state.spawn_proofs.insert(
                    crate::approval::identity::digest(proof.as_header_value()),
                    PendingSpawn {
                        lease: lease.clone(),
                        metadata: spec.registration_metadata.clone(),
                    },
                );
            }
            spec.registration_proof = Some(proof);
            spec.spawn_attempt = Some(lease.id);
            let outcome = self.spawner.spawn_and_wait(spec, wait, waiter).await;
            // An expired/disconnected HTTP waiter says nothing about an accepted
            // wrapper's lifetime. Its lease is consumed only by registration or by
            // the process owner's explicit terminal startup-failure callback.
            match &outcome {
                SpawnWaitOutcome::Registered(binding) => {
                    if self
                        .details(binding.session)
                        .is_none_or(|s| s.binding != *binding)
                    {
                        return SpawnWaitOutcome::Failed(
                            "launcher reported an unregistered binding".into(),
                        );
                    }
                }
                SpawnWaitOutcome::Failed(_) | SpawnWaitOutcome::HubStopped => {
                    // A real process owner releases on terminal failure even if
                    // the HTTP waiter is gone. This observer cleanup is exact
                    // and idempotent; it cannot remove a registered session or
                    // a different provider's lease after the owner won first.
                    self.end_provider_spawn(lease);
                }
                _ => {}
            }
            outcome
        })
    }
}
impl SessionEngine {
    pub fn spawn_failed(
        &self,
        attempt: SpawnAttemptId,
        provider: &str,
    ) -> Result<CoreEffects, SessionError> {
        let mut state = lock(&self.state);
        state
            .spawn_proofs
            .retain(|_, pending| pending.lease.id != attempt || pending.lease.provider != provider);
        // A terminal owner may race registration consuming this exact lease.
        // Cleanup is idempotent and never removes a registered session or any
        // differently identified/provider-bound pending launch.
        state.admission.end_provider_spawn(&ProviderSpawnLease {
            id: attempt,
            provider: provider.into(),
        });
        Ok(CoreEffects::default())
    }
}
