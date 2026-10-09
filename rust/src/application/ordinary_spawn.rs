//! Authenticated ordinary UI Start. Registration/session ownership stays in core.
use super::orchestration_program::{OrchestrationProgram, RoleSettings};
use super::{spawn_policy::RegistrySnapshot, wrapped_spawn::SpawnLaunchPolicy};
use crate::orchestration::child_launch::ChildLaunchServices;
use crate::{
    config::{self, ConfigStore},
    hub::task_owner::HubTaskHandle,
    orchestration::{
        child_launch::{cwd_too_broad, prompt::sanitize_inject_text},
        normal_worktree::{
            NormalWorktreeLifecycle, effective_worktree_cleanup, valid_worktree_cleanup,
        },
    },
    proto::{core::*, time::Timestamp},
    terminal::session::SessionEngine,
};
use std::{path::PathBuf, sync::Arc};
mod request;
pub use request::OrdinarySpawnRequest;
#[cfg(test)]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpawnError {
    pub status: u16,
    pub code: &'static str,
    pub detail: &'static str,
}
impl SpawnError {
    fn bad(detail: &'static str) -> Self {
        Self {
            status: 400,
            code: "bad_request",
            detail,
        }
    }
}
pub trait OrdinarySpawnHooks: Send + Sync {
    /// Required registration delivery owner; execute only after core accepts
    /// metadata, never against an unregistered label or an invented session ID.
    fn registered<'a>(
        &'a self,
        binding: SessionBinding,
        metadata: SpawnRegistrationMetadata,
    ) -> CoreFuture<'a, Result<(), SessionError>>;
    fn warning(&self, operation: &'static str);
}
pub struct OrdinarySpawnService {
    pub core: Arc<SessionEngine>,
    config: Arc<ConfigStore>,
    policy: Arc<dyn SpawnLaunchPolicy>,
    registry: RegistrySnapshot,
    worktrees: NormalWorktreeLifecycle,
    hooks: Arc<dyn OrdinarySpawnHooks>,
    hub_cwd: PathBuf,
    home: PathBuf,
    environment: Vec<String>,
    tasks: HubTaskHandle,
    orchestration: Option<Arc<OrchestrationProgram>>,
}
pub struct OrdinarySpawnDependencies {
    pub core: Arc<SessionEngine>,
    pub config: Arc<ConfigStore>,
    pub policy: Arc<dyn SpawnLaunchPolicy>,
    pub registry: RegistrySnapshot,
    pub worktrees: NormalWorktreeLifecycle,
    pub hooks: Arc<dyn OrdinarySpawnHooks>,
    pub hub_cwd: PathBuf,
    pub home: PathBuf,
    pub environment: Vec<String>,
    pub tasks: HubTaskHandle,
    pub orchestration: Option<Arc<OrchestrationProgram>>,
}
// Before native ownership begins, retain both the exact provider lease and
// created directory. A detached HTTP observer cannot abandon this workflow;
// an unwinding owned worker still releases admission and schedules rollback.
struct PreparationOwnership {
    core: Arc<SessionEngine>,
    lease: Option<ProviderSpawnLease>,
    worktrees: NormalWorktreeLifecycle,
    hooks: Arc<dyn OrdinarySpawnHooks>,
    tasks: HubTaskHandle,
    tree: Option<NormalWorktree>,
}
impl PreparationOwnership {
    fn transferred(&mut self) {
        self.lease = None;
        self.tree = None;
    }
}
impl Drop for PreparationOwnership {
    fn drop(&mut self) {
        if let Some(lease) = self.lease.take() {
            self.core.end_provider_spawn(lease);
        }
        if let Some(tree) = self.tree.take().filter(|tree| tree.created) {
            let worktrees = self.worktrees.clone();
            let hooks = self.hooks.clone();
            // Drop executes inside the owned Tokio worker. Cleanup remains
            // bounded by NormalWorktreeLifecycle's Git command deadlines.
            match self.tasks.effect_permit() {
                Ok(permit) => {
                    drop(permit.start(async move {
                        if worktrees.cleanup(&tree, "delete").await.is_err() {
                            hooks.warning("abandoned worktree cleanup failed");
                        }
                    }));
                }
                Err(_) => hooks.warning("abandoned worktree cleanup admission failed"),
            }
        }
    }
}
impl OrdinarySpawnService {
    pub fn new(deps: OrdinarySpawnDependencies) -> Result<Self, SpawnError> {
        if !deps.hub_cwd.is_absolute() || !deps.home.is_absolute() {
            return Err(SpawnError::bad("explicit absolute runtime paths required"));
        }
        Ok(Self {
            core: deps.core,
            config: deps.config,
            policy: deps.policy,
            registry: deps.registry,
            worktrees: deps.worktrees,
            hooks: deps.hooks,
            hub_cwd: deps.hub_cwd,
            home: deps.home,
            environment: deps.environment,
            tasks: deps.tasks,
            orchestration: deps.orchestration,
        })
    }
    pub async fn spawn(
        self: &Arc<Self>,
        body: OrdinarySpawnRequest,
        verified: &VerifiedConfirmationRequest,
        waiter: &HttpWaitCancellation,
        now: Timestamp,
        offset: i32,
    ) -> Result<Option<OrchestrationId>, SpawnError> {
        if verified.auth_epoch() != self.core.auth_epoch() {
            return Err(SpawnError {
                status: 401,
                code: "unauthorized",
                detail: "authentication expired",
            });
        }
        body.validate()?;
        let service = self.clone();
        let waiter = waiter.clone();
        // Dropping TaskWaiter detaches only the HTTP observer. The retained Hub
        // request lane drains the owned worktree/native workflow at shutdown.
        let permit = self
            .tasks
            .request_permit()
            .map_err(|_| unavailable("Hub is stopping"))?;
        permit
            .start(async move { service.spawn_owned(body, waiter, now, offset).await })
            .wait()
            .await
            .unwrap_or({
                Err(SpawnError {
                    status: 500,
                    code: "spawn_error",
                    detail: "spawn workflow failed",
                })
            })
    }
    async fn spawn_owned(
        &self,
        mut body: OrdinarySpawnRequest,
        waiter: HttpWaitCancellation,
        now: Timestamp,
        offset: i32,
    ) -> Result<Option<OrchestrationId>, SpawnError> {
        if body.orchestration && self.orchestration.is_none() {
            return Err(unavailable("conductor launch composition is unavailable"));
        }
        let registry =
            (self.registry)().map_err(|_| unavailable("provider registry unavailable"))?;
        let definition = registry.lookup(&body.provider);
        if body.provider != "shell"
            && !definition.as_ref().is_some_and(|d| {
                d.definition.enabled.unwrap_or(true) && d.definition.launch.is_some()
            })
        {
            return Err(SpawnError::bad("invalid provider"));
        }
        let lease = self
            .core
            .begin_provider_spawn(&body.provider)
            .map_err(|_| SpawnError {
                status: 409,
                code: "provider_updating",
                detail: "provider is currently updating",
            })?;
        let mut ownership = PreparationOwnership {
            core: self.core.clone(),
            lease: Some(lease.clone()),
            worktrees: self.worktrees.clone(),
            hooks: self.hooks.clone(),
            tasks: self.tasks.clone(),
            tree: None,
        };
        let prepared = self
            .prepare(
                &mut body,
                now,
                offset,
                lease.id,
                &mut ownership,
                definition.as_ref().is_some_and(|d| {
                    d.definition
                        .launch
                        .as_ref()
                        .is_some_and(|l| l.headless.is_some())
                }),
            )
            .await;
        let (spec, policy, kind) = match prepared {
            Ok(value) => value,
            Err(error) => {
                if let Some(tree) = ownership.tree.take()
                    && self.worktrees.cleanup(&tree, "delete").await.is_err()
                {
                    self.hooks.warning("preparation worktree cleanup failed");
                }
                return Err(error);
            }
        };
        let metadata = spec.registration_metadata.clone();
        ownership.tree = Some(metadata.normal_worktree.clone());
        match self
            .core
            .start_wrapped(WrappedStartRequest { spec, policy, kind }, &waiter)
            .await
        {
            SpawnStartOutcome::Started { .. } => {
                ownership.transferred();
                Ok((!metadata.orchestration.0.is_empty()).then_some(metadata.orchestration))
            }
            SpawnStartOutcome::WaiterCancelled => {
                ownership.transferred();
                Err(SpawnError {
                    status: 499,
                    code: "request_cancelled",
                    detail: "request observer cancelled",
                })
            }
            SpawnStartOutcome::Failed(_) | SpawnStartOutcome::HubStopped => {
                self.startup_failed(&metadata).await;
                ownership.tree = None;
                Err(SpawnError {
                    status: 500,
                    code: "spawn_error",
                    detail: "spawn error",
                })
            }
        }
    }
    async fn prepare(
        &self,
        body: &mut OrdinarySpawnRequest,
        now: Timestamp,
        offset: i32,
        attempt: SpawnAttemptId,
        ownership: &mut PreparationOwnership,
        headless: bool,
    ) -> Result<(WrappedSpawnSpec, ResolvedSpawnPolicy, WrappedStartKind), SpawnError> {
        let cfg = self
            .config
            .snapshot()
            .map_err(|_| unavailable("spawn configuration unavailable"))?
            .config;
        body.effort = body.effort.trim().to_owned();
        body.execution_mode = config::normalize_execution_mode(&body.execution_mode);
        body.permission_preset = config::normalize_permission_preset(&body.permission_preset);
        config::validate_effort(&body.provider, &body.effort)
            .map_err(|_| SpawnError::bad("invalid effort"))?;
        config::validate_permission_preset(&body.permission_preset)
            .map_err(|_| SpawnError::bad("invalid permission_preset"))?;
        body.execution_mode = config::resolve_execution_mode(
            &body.execution_mode,
            headless || config::headless_def_for(&body.provider, Some(&cfg)).is_some(),
            config::LAUNCH_ORIGIN_UI,
            false,
        )
        .map_err(|_| SpawnError::bad("unsupported execution_mode"))?;
        let raw = if body.cwd.is_empty() {
            self.hub_cwd.clone()
        } else {
            PathBuf::from(&body.cwd)
        };
        let cwd = if raw.is_absolute() {
            raw
        } else {
            self.hub_cwd.join(raw)
        };
        if !cwd.is_dir() {
            return Err(SpawnError::bad("cwd does not exist or is not a directory"));
        }
        if cwd_too_broad(&cwd, Some(&self.home)) {
            return Err(SpawnError::bad(
                "cwd is too broad (system root or home root)",
            ));
        }
        if !body.subscription_profile_id.trim().is_empty() && body.provider == "shell" {
            return Err(SpawnError::bad(
                "shell sessions do not use subscription profiles",
            ));
        }
        if !body.subscription_profile_id.trim().is_empty()
            && cfg.is_custom_provider_id(&body.provider)
        {
            return Err(SpawnError::bad(
                "custom providers do not use subscription profiles",
            ));
        }
        let mut grants = InternalSpawnGrants::default();
        if body.provider != "shell" && !body.permission_preset.is_empty() {
            let preset = crate::orchestration::child_options::permission_preset_approval(
                &body.provider,
                &body.permission_preset,
                &cfg.orchestration,
            );
            if body.permission_mode.is_empty() {
                body.permission_mode = preset.permission_mode;
            }
            if body.sandbox.is_empty() {
                body.sandbox = preset.sandbox;
            }
            if body.ask_for_approval.is_empty() {
                body.ask_for_approval = preset.ask_for_approval;
            }
            grants = InternalSpawnGrants::from_config(preset.allowed_tools);
        }
        let mut spec = body.spec(cwd, attempt, grants);
        // Policy resolves profile/route/model exactly once before worktree or
        // pending metadata. Start consumes this exact policy without reselecting.
        let policy = if body.provider == "shell" {
            // Source shell early path uses hubSpawnEnv directly and bypasses
            // AI route/profile/model/probe and last-model work entirely.
            ResolvedSpawnPolicy {
                base_environment: self.environment.clone(),
                route_environment: vec![],
                subscription_environment: vec![],
                effective_route: String::new(),
                current_model: String::new(),
                resolved_model: String::new(),
                effort_args: vec![],
                ordinary_model_args: vec![],
            }
        } else {
            self.policy
                .resolve(&spec, &self.environment)
                .map_err(policy_error)?
        };
        if risk(&spec, &policy) {
            return Err(SpawnError {
                status: 400,
                code: "risk_confirmation_required",
                detail: "risk confirmation required",
            });
        }
        let prefs = self
            .config
            .snapshot()
            .map_err(|_| unavailable("spawn configuration unavailable"))?
            .config
            .user_prefs
            .spawn;
        let cleanup = effective_worktree_cleanup(if body.worktree_cleanup.is_empty() {
            &prefs.worktree_cleanup
        } else {
            &body.worktree_cleanup
        })
        .to_owned();
        let mut prompt = sanitize_inject_text(&body.initial_prompt).trim().to_owned();
        let mut limit = prompt.len().min(8192);
        while !prompt.is_char_boundary(limit) {
            limit -= 1;
        }
        prompt.truncate(limit);
        let stamp = i128::from(now.unix_seconds()) * 1_000_000_000 + i128::from(now.subsec_nanos());
        if body.isolate_worktree.unwrap_or(prefs.worktree_auto) {
            if spec.label.is_empty() {
                spec.label = format!("worktree-{stamp}");
            }
            let tree = self
                .worktrees
                .prepare(&spec.cwd, &spec.label, now, offset)
                .await
                .map_err(|_| SpawnError {
                    status: 400,
                    code: "worktree_error",
                    detail: "worktree error",
                })?;
            ownership.tree = Some(tree.clone());
            spec.cwd = PathBuf::from(&tree.path);
            spec.registration_metadata.worktree_branch = tree.branch.clone();
            spec.registration_metadata.normal_worktree = tree;
            spec.registration_metadata.worktree_cleanup = cleanup;
            spec.registration_metadata.spawned_at = Some(now);
        }
        if body.orchestration {
            let owner = self
                .orchestration
                .as_ref()
                .expect("conductor owner checked");
            let roles = body.orchestration_roles.as_ref().map(|roles| {
                roles
                    .iter()
                    .map(|(role, settings)| {
                        (
                            role.clone(),
                            settings.as_ref().map(|settings| RoleSettings {
                                provider: settings.provider.clone(),
                                model: settings.model.clone(),
                                subscription: settings.subscription.clone(),
                                effort: settings.effort.clone(),
                                execution_mode: settings.execution_mode.clone(),
                                permission_preset: settings.permission_preset.clone(),
                            }),
                        )
                    })
                    .collect()
            });
            spec.registration_metadata.orchestration = owner
                .reserve_conductor(&mut spec.label, roles.as_ref(), now)
                .map_err(|error| match error {
                    SessionError::InvalidRequest(detail)
                        if detail == "invalid orchestration role provider" =>
                    {
                        SpawnError::bad("invalid orchestration role provider")
                    }
                    SessionError::InvalidRequest(_) => {
                        SpawnError::bad("invalid orchestration role model")
                    }
                    _ => unavailable("conductor reservation unavailable"),
                })?;
            spec.registration_metadata.spawned_at = Some(now);
        }
        if !prompt.is_empty() || body.orchestration {
            if spec.label.is_empty() {
                spec.label = format!("spawn-{stamp}");
            }
            let launch = config::is_headless_execution_mode(&spec.execution_mode)
                || (config::launch_prompt_via_arg(&spec.provider)
                    && self.orchestration.as_ref().is_some_and(|owner| {
                        owner.launch_arg_usable(&spec.provider).unwrap_or(false)
                    }));
            if launch {
                spec.initial_prompt = if body.orchestration {
                    self.orchestration
                        .as_ref()
                        .expect("conductor owner checked")
                        .conductor_prompt(&spec.registration_metadata.orchestration)
                } else {
                    prompt.clone()
                };
            } else if !prompt.is_empty() {
                spec.registration_metadata.initial_prompt = prompt.clone();
            }
            if !prompt.is_empty() {
                spec.registration_metadata.handoff_from = LiveSessionId(body.handoff_from);
            }
            spec.registration_metadata.prompt_at_launch = launch;
            spec.registration_metadata.spawned_at = Some(now);
        }
        let kind = if body.provider == "shell" {
            WrappedStartKind::Bare
        } else {
            WrappedStartKind::OrdinaryAi {
                delegation: body.delegation.unwrap_or(prefs.delegation_auto),
            }
        };
        Ok((spec, policy, kind))
    }
    pub async fn registered(
        &self,
        binding: SessionBinding,
        metadata: SpawnRegistrationMetadata,
    ) -> Result<(), SessionError> {
        self.hooks.registered(binding, metadata).await
    }
    pub async fn startup_failed(&self, metadata: &SpawnRegistrationMetadata) {
        if self
            .worktrees
            .cleanup(&metadata.normal_worktree, "delete")
            .await
            .is_err()
        {
            self.hooks.warning("unregistered worktree cleanup failed");
        }
    }
    pub async fn dismissed(&self, snapshot: &SessionSnapshot) {
        if self
            .worktrees
            .cleanup(&snapshot.normal_worktree, &snapshot.worktree_cleanup)
            .await
            .is_err()
        {
            self.hooks.warning("dismissed worktree cleanup failed");
        }
    }
}
pub(crate) fn policy_error(error: std::io::Error) -> SpawnError {
    use super::spawn_policy::{PolicyFailureStage, failure_stage};
    match failure_stage(&error) {
        Some(PolicyFailureStage::Route) => SpawnError {
            status: 400,
            code: "route_error",
            detail: "route error",
        },
        _ => SpawnError {
            status: 400,
            code: "invalid_subscription",
            detail: "subscription profile error",
        },
    }
}
fn unavailable(detail: &'static str) -> SpawnError {
    SpawnError {
        status: 503,
        code: "not_ready",
        detail,
    }
}
fn risk(spec: &WrappedSpawnSpec, policy: &ResolvedSpawnPolicy) -> bool {
    if spec.risk_confirmed || spec.provider == "shell" {
        return false;
    }
    let changed = !policy.resolved_model.trim().is_empty()
        && policy.resolved_model.trim() != policy.current_model.trim();
    match spec.provider.as_str() {
        "claude" => {
            spec.model_selection == "required"
                || changed
                || spec.permission_mode == "bypassPermissions"
        }
        "codex" => {
            spec.model_selection == "required"
                || changed
                || spec.sandbox == "danger-full-access"
                || spec.ask_for_approval == "never"
        }
        "opencode" | "command-code" => spec.permission_mode == "bypassPermissions",
        _ => false,
    }
}
