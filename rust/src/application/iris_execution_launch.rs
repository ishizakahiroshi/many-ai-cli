//! Trusted launch seam for an admitted Iris coordinator. No HTTP capability or
//! production enablement is changed here. Registration is not native root start.
//! Callers must commit the one-use journal claim before invoking the port and
//! independently observe OwnerStore root ownership before bind_owner/running.
use crate::{
    hub::{
        iris_execution_grants::ApprovedGrant,
        iris_execution_owner::{OwnerStore, OwnerTicket},
        iris_execution_runtime::{BridgeError, SessionBridge},
    },
    proto::core::{
        CoreEffectSink, CoreFuture, HttpWaitCancellation, InternalSpawnGrants, SessionBinding,
        SpawnRegistrationMetadata, SpawnWaitOutcome, TaskCancellation, WrappedSessionSpawner,
        WrappedSpawnSpec,
    },
    terminal::session::SessionEngine,
};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub trait OwnedLaunchPort: Send + Sync {
    fn launch_registered<'a>(
        &'a self,
        spec: WrappedSpawnSpec,
        wait: Duration,
        cancel: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome>;
    fn is_current(&self, binding: SessionBinding) -> bool;
    fn bind_owner(
        &self,
        ticket: &OwnerTicket,
        binding: SessionBinding,
    ) -> Result<Value, BridgeError>;
    fn stop_owner<'a>(
        &'a self,
        ticket: &'a OwnerTicket,
        stop_request_id: &'a str,
    ) -> CoreFuture<'a, Result<Value, BridgeError>>;
}
/// Production composition can only forward through SessionEngine, preserving
/// provider admission, core-issued proof, and registered binding validation.
pub struct CoreLaunchPort {
    core: Arc<SessionEngine>,
    bridge: SessionBridge,
    owners: Arc<Mutex<OwnerStore>>,
}
impl CoreLaunchPort {
    pub fn new(
        core: Arc<SessionEngine>,
        effects: Arc<dyn CoreEffectSink>,
        owners: Arc<Mutex<OwnerStore>>,
    ) -> Self {
        Self {
            bridge: SessionBridge::new_with_owner_store(core.clone(), effects, owners.clone()),
            core,
            owners,
        }
    }
}
impl OwnedLaunchPort for CoreLaunchPort {
    fn launch_registered<'a>(
        &'a self,
        spec: WrappedSpawnSpec,
        wait: Duration,
        cancel: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome> {
        self.core.spawn_and_wait(spec, wait, cancel)
    }
    fn is_current(&self, binding: SessionBinding) -> bool {
        self.core.is_current(binding)
    }
    fn bind_owner(
        &self,
        ticket: &OwnerTicket,
        binding: SessionBinding,
    ) -> Result<Value, BridgeError> {
        self.bridge.bind_owner_attempt(ticket, binding)
    }
    fn stop_owner<'a>(
        &'a self,
        ticket: &'a OwnerTicket,
        stop_request_id: &'a str,
    ) -> CoreFuture<'a, Result<Value, BridgeError>> {
        Box::pin(async move {
            // The bridge retains its originally bound owner ticket. Also check
            // the caller's whole ticket, including owner_id/epoch; matching the
            // attempt/fence alone must not authorize a different owner caller.
            {
                let owners = self
                    .owners
                    .lock()
                    .map_err(|_| BridgeError::StateUnavailable)?;
                let record = owners
                    .query(&ticket.binding)
                    .map_err(|_| BridgeError::StateUnavailable)?;
                if record["ticket"] != json!(ticket) {
                    return Err(BridgeError::OwnershipConflict);
                }
            }
            let b = &ticket.binding;
            self.bridge
                .stop_bound(
                    &b.attempt_id,
                    b.fence,
                    &b.executor_instance_id,
                    stop_request_id,
                )
                .await
        })
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrepareError {
    BindingMismatch,
    InvalidIdentity,
    StaleGrant,
    UnsupportedPolicy,
    InstructionUnavailable,
    InvalidInstruction,
    CwdUnavailable,
    ClockUnavailable,
}
impl std::fmt::Display for PrepareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::BindingMismatch => "LAUNCH_BINDING_MISMATCH",
            Self::InvalidIdentity => "LAUNCH_INVALID_IDENTITY",
            Self::StaleGrant => "LAUNCH_STALE_GRANT",
            Self::UnsupportedPolicy => "LAUNCH_UNSUPPORTED_POLICY",
            Self::InstructionUnavailable => "LAUNCH_INSTRUCTION_UNAVAILABLE",
            Self::InvalidInstruction => "LAUNCH_INVALID_INSTRUCTION",
            Self::CwdUnavailable => "LAUNCH_CWD_UNAVAILABLE",
            Self::ClockUnavailable => "LAUNCH_CLOCK_UNAVAILABLE",
        })
    }
}
impl std::error::Error for PrepareError {}
fn identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}
fn static_binding(mut value: Value) -> Value {
    if let Some(fields) = value.as_object_mut() {
        for field in ["attempt_id", "fence", "executor_instance_id"] {
            fields.remove(field);
        }
    }
    value
}
/// Only a freshly loaded fixed operator grant supplies launch content. This
/// structural preparation is not a grant refresh or journal dispatch claim.
/// No custom command, caller argv/environment, auto profile, login, headless,
/// elevated risk approval, folder trust or tool grants are synthesized.
pub fn prepare_spec(
    grant: &ApprovedGrant,
    ticket: &OwnerTicket,
) -> Result<WrappedSpawnSpec, PrepareError> {
    let b = &ticket.binding;
    if static_binding(json!(grant.grant.binding)) != static_binding(json!(b)) {
        return Err(PrepareError::BindingMismatch);
    }
    if !identity(&ticket.owner_id)
        || !identity(&b.attempt_id)
        || !identity(&b.executor_instance_id)
        || ticket.owner_epoch == 0
        || ticket.owner_epoch > i64::MAX as u64
        || b.fence == 0
        || b.fence > i64::MAX as u64
    {
        return Err(PrepareError::InvalidIdentity);
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| PrepareError::ClockUnavailable)?
        .as_secs();
    if grant.grant.revoked || grant.grant.expires_at <= 0 || grant.grant.expires_at as u64 <= now {
        return Err(PrepareError::StaleGrant);
    }
    if !(crate::proto::provider::BUILTIN_PROVIDER_IDS.contains(&b.provider.as_str())
        || b.provider == "synthetic")
        || !identity(&b.profile_id)
        || b.profile_id.eq_ignore_ascii_case("auto")
        || grant.allow_remote_jobs
    {
        return Err(PrepareError::UnsupportedPolicy);
    }
    let cwd = grant
        .approved_cwd
        .canonicalize()
        .map_err(|_| PrepareError::CwdUnavailable)?;
    if !cwd.is_dir() || cwd != grant.approved_cwd || !cwd.is_absolute() {
        return Err(PrepareError::CwdUnavailable);
    }
    let artifact = grant
        .instruction
        .verify(Some(&b.instruction_hash))
        .map_err(|_| PrepareError::InstructionUnavailable)?;
    let instruction =
        std::str::from_utf8(artifact.bytes()).map_err(|_| PrepareError::InvalidInstruction)?;
    if instruction.trim().is_empty() || instruction.contains('\0') {
        return Err(PrepareError::InvalidInstruction);
    }
    let context = serde_json::to_string(ticket).map_err(|_| PrepareError::InvalidIdentity)?;
    Ok(WrappedSpawnSpec {
        registration_metadata: SpawnRegistrationMetadata {
            iris_owner_context: context,
            prompt_at_launch: true,
            ..Default::default()
        },
        spawn_attempt: None,
        registration_proof: None,
        provider: b.provider.clone(),
        cwd,
        model: String::new(),
        model_selection: String::new(),
        risk_confirmed: false,
        label: format!("iris-{}", b.attempt_id),
        permission_mode: "default".into(),
        sandbox: if b.provider == "codex" {
            "read-only".into()
        } else {
            String::new()
        },
        ask_for_approval: if b.provider == "codex" {
            "on-request".into()
        } else {
            String::new()
        },
        route: String::new(),
        utf8_session: false,
        effort: String::new(),
        execution_mode: "interactive".into(),
        permission_preset: String::new(),
        initial_prompt: instruction.into(),
        subscription_profile_id: b.profile_id.clone(),
        subscription_login: false,
        usage_probe: false,
        grants: InternalSpawnGrants::default(),
        cancellation: TaskCancellation::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hub::{
        iris_execution_journal::{Binding, Grant},
        iris_execution_owner::ArtifactPermit,
    };
    use sha2::{Digest, Sha256};
    fn hash(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
    fn fixture(bytes: &[u8]) -> (tempfile::TempDir, ApprovedGrant, OwnerTicket) {
        let root = tempfile::tempdir().unwrap();
        let cwd = root.path().canonicalize().unwrap();
        std::fs::write(cwd.join("instruction.txt"), bytes).unwrap();
        let b = Binding {
            principal_id: "iris".into(),
            workspace_id: "workspace".into(),
            plan_id: "plan".into(),
            plan_revision: 1,
            task_id: "task".into(),
            instruction_revision: 1,
            instruction_hash: hash(bytes),
            approval_id: "approval".into(),
            approval_revision: 1,
            scope_hash: hash(b"scope"),
            execution_host_id: "host".into(),
            executor_instance_id: "instance".into(),
            attempt_id: "attempt".into(),
            fence: 1,
            provider: "synthetic".into(),
            profile_id: "synthetic".into(),
        };
        let instruction =
            ArtifactPermit::approved(&cwd, "instruction.txt", &b, 65536, vec!["none".into()])
                .unwrap();
        let result =
            ArtifactPermit::approved(&cwd, "result.json", &b, 65536, vec!["none".into()]).unwrap();
        let grant = ApprovedGrant {
            grant: Grant {
                binding: b.clone(),
                expires_at: 10_000_000_000,
                revoked: false,
            },
            approved_cwd: cwd,
            instruction,
            result,
            effect_scope: vec!["none".into()],
            allow_remote_jobs: false,
        };
        (
            root,
            grant,
            OwnerTicket {
                binding: b,
                owner_id: "owner".into(),
                owner_epoch: 1,
            },
        )
    }
    #[test]
    fn exact_instruction_and_owner_metadata_preserve_core_admission_and_safe_defaults() {
        let bytes = "  合成instruction\r\n\n".as_bytes();
        let (_root, grant, ticket) = fixture(bytes);
        let spec = prepare_spec(&grant, &ticket).unwrap();
        assert_eq!(spec.initial_prompt.as_bytes(), bytes);
        assert_eq!(spec.cwd, grant.approved_cwd);
        assert_eq!(spec.provider, ticket.binding.provider);
        assert_eq!(spec.subscription_profile_id, ticket.binding.profile_id);
        assert!(spec.spawn_attempt.is_none() && spec.registration_proof.is_none());
        assert!(!spec.risk_confirmed && !spec.subscription_login && !spec.usage_probe);
        assert_eq!(spec.execution_mode, "interactive");
        assert_eq!(spec.grants, InternalSpawnGrants::default());
        assert!(spec.registration_metadata.prompt_at_launch);
        assert!(!spec.registration_metadata.needs_initial_gate());
        assert_eq!(
            serde_json::from_str::<OwnerTicket>(&spec.registration_metadata.iris_owner_context)
                .unwrap(),
            ticket
        );
    }
    #[test]
    fn changed_binding_revocation_expiry_policy_and_artifact_never_prepare_launch() {
        let (root, mut grant, ticket) = fixture(b"synthetic instruction");
        let mut wrong = ticket.clone();
        wrong.binding.profile_id = "other".into();
        assert!(matches!(
            prepare_spec(&grant, &wrong),
            Err(PrepareError::BindingMismatch)
        ));
        grant.grant.revoked = true;
        assert!(matches!(
            prepare_spec(&grant, &ticket),
            Err(PrepareError::StaleGrant)
        ));
        grant.grant.revoked = false;
        grant.grant.expires_at = 1;
        assert!(matches!(
            prepare_spec(&grant, &ticket),
            Err(PrepareError::StaleGrant)
        ));
        grant.grant.expires_at = 10_000_000_000;
        grant.allow_remote_jobs = true;
        assert!(matches!(
            prepare_spec(&grant, &ticket),
            Err(PrepareError::UnsupportedPolicy)
        ));
        grant.allow_remote_jobs = false;
        std::fs::write(root.path().join("instruction.txt"), b"modified instruction").unwrap();
        assert!(matches!(
            prepare_spec(&grant, &ticket),
            Err(PrepareError::InstructionUnavailable)
        ));
        let (_root, grant, ticket) = fixture(&[0xff]);
        assert!(matches!(
            prepare_spec(&grant, &ticket),
            Err(PrepareError::InvalidInstruction)
        ));
    }
}
