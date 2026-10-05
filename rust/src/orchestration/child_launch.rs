//! Fixed-Go orchestration.go child preparation and confirmation execution.
//!
//! SessionEngine remains the only session/admission owner. This module owns
//! board-file I/O and real Git preparation; application-owned delivery/restart
//! tracking are explicit required dependencies, never successful placeholders.
pub mod board;
pub mod prompt;
pub mod worktree;

use crate::{
    config::{self, Config, ConfigError, ConfigStore, OrchestrationConfig, RuntimePaths},
    proto::{core::*, time::Timestamp},
    terminal::session::{
        SessionEngine,
        confirmations::{ConfirmationExecutor, ConfirmationPresentation, ConfirmedChildRequest},
    },
};
use board::BoardStore;
use prompt::{PromptDelivery, child_initial_prompt, child_launch_prompt};
use std::{
    fmt, fs,
    path::{Component, Path, PathBuf},
    sync::{Arc, Weak},
    time::Duration,
};
use worktree::{WorktreeError, WorktreeGit};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChildLaunchError {
    pub status: u16,
    pub code: String,
    pub detail: String,
}
impl ChildLaunchError {
    fn new(status: u16, code: &str, detail: impl Into<String>) -> Self {
        Self {
            status,
            code: code.into(),
            detail: detail.into(),
        }
    }
    fn board(error: impl fmt::Display) -> Self {
        Self::new(500, "board_error", format!("board error: {error}"))
    }
}
impl fmt::Display for ChildLaunchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.detail)
    }
}
impl std::error::Error for ChildLaunchError {}
impl From<ChildLaunchError> for SessionError {
    fn from(error: ChildLaunchError) -> Self {
        Self::ChildLaunch {
            status: error.status,
            code: error.code,
            detail: error.detail,
        }
    }
}

pub fn safe_token(value: &str) -> String {
    let mut out = String::new();
    let mut invalid_run = false;
    for c in value.trim().chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
            out.push(c);
            invalid_run = false;
        } else if !invalid_run {
            out.push('-');
            invalid_run = true;
        }
    }
    let mut out = out.trim_matches(['-', '_', '.']).to_owned();
    if out.is_empty() {
        return "item".into();
    }
    out.truncate(out.len().min(80));
    out
}
/// Lexical filepath.Clean behavior. Do not resolve symlinks for requested cwd:
/// Go's broad-directory check and displayed absolute cwd are lexical too.
fn clean_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(out.components().next_back(), Some(Component::Normal(_))) {
                    out.pop();
                } else if !out.has_root() {
                    out.push("..");
                }
            }
            component => out.push(component.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

pub fn cwd_too_broad(cwd: &Path, home: Option<&Path>) -> bool {
    if cwd.as_os_str().is_empty() {
        return false;
    }
    let cwd = clean_path(cwd);
    if cwd.has_root() && cwd.parent().is_none() {
        return true;
    }
    if let Some(home) = home {
        let home = clean_path(home);
        if cwd == home || home.parent().is_some_and(|parent| cwd == parent) {
            return true;
        }
    }
    let text = cwd.to_string_lossy();
    if [
        "/etc", "/usr", "/var", "/bin", "/sbin", "/lib", "/root", "/home", "/Users",
    ]
    .contains(&text.as_ref())
    {
        return true;
    }
    [
        r"C:\Windows",
        r"C:\Program Files",
        r"C:\Program Files (x86)",
        r"C:\Users",
    ]
    .iter()
    .any(|path| text.eq_ignore_ascii_case(path))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChildPreparation {
    pub orchestration: OrchestrationId,
    pub board_path: PathBuf,
    /// Original JSON request remains on ChildSpawnRequest.cwd.
    pub absolute_cwd: PathBuf,
    pub child_cwd: PathBuf,
    pub branch: String,
}
pub struct ChildPreparer {
    pub boards: Arc<BoardStore>,
    git: WorktreeGit,
    hub_cwd: PathBuf,
    home: Option<PathBuf>,
}
impl ChildPreparer {
    /// Home is an explicit production or synthetic value used only for the
    /// source broad-cwd check. There is no real-home lookup/fallback here.
    pub fn new(
        paths: &RuntimePaths,
        hub_cwd: PathBuf,
        home: Option<PathBuf>,
        git: WorktreeGit,
    ) -> Result<Self, ChildLaunchError> {
        if !hub_cwd.is_absolute() {
            return Err(ChildLaunchError::new(
                500,
                "board_error",
                "Hub cwd must be absolute",
            ));
        }
        Ok(Self {
            boards: Arc::new(BoardStore::new(paths)),
            git,
            hub_cwd,
            home,
        })
    }
    pub fn resolve_cwd(
        &self,
        parent: &SessionSnapshot,
        requested: &str,
    ) -> Result<PathBuf, ChildLaunchError> {
        let requested = requested.trim();
        let cwd = if requested.is_empty() {
            if parent.cwd.is_empty() {
                self.hub_cwd.clone()
            } else {
                PathBuf::from(&parent.cwd)
            }
        } else if Path::new(requested).is_absolute() {
            PathBuf::from(requested)
        } else {
            let base = if parent.cwd.is_empty() {
                &self.hub_cwd
            } else {
                Path::new(&parent.cwd)
            };
            clean_path(&base.join(requested))
        };
        if !fs::metadata(&cwd).is_ok_and(|meta| meta.is_dir()) {
            return Err(ChildLaunchError::new(
                400,
                "bad_request",
                "cwd does not exist or is not a directory",
            ));
        }
        if cwd_too_broad(&cwd, self.home.as_deref()) {
            return Err(ChildLaunchError::new(
                400,
                "bad_request",
                "cwd is too broad (system root or home root)",
            ));
        }
        Ok(cwd)
    }
    /// mark_conductor is deliberately between board creation and worktree I/O,
    /// matching Go even when subsequent reuse validation fails.
    pub async fn prepare<F, Fut>(
        &self,
        parent: &SessionSnapshot,
        body: &ChildSpawnRequest,
        cfg: &OrchestrationConfig,
        now: Timestamp,
        cancel: &TaskCancellation,
        mark_conductor: F,
    ) -> Result<ChildPreparation, SessionError>
    where
        F: FnOnce(&OrchestrationId, &Path) -> Fut,
        Fut: std::future::Future<Output = Result<(), SessionError>>,
    {
        if cancel.token().is_cancelled() {
            return Err(SessionError::Cancelled);
        }
        let cwd = self.resolve_cwd(parent, &body.cwd)?;
        let orchestration = orchestration_id(parent, now);
        let board = self
            .boards
            .ensure(&orchestration.0, parent, &body.initial_prompt, now)?;
        mark_conductor(&orchestration, &board).await?;
        let worktree = if body.same_tree == Some(true) {
            Ok(worktree::WorktreePreparation {
                cwd: cwd.clone(),
                branch: String::new(),
                note: "worktree skip: requested by --same-tree".into(),
            })
        } else {
            self.git
                .prepare(&cwd, &orchestration.0, &body.role, cfg, cancel)
                .await
        };
        let worktree = match worktree {
            Ok(worktree) => worktree,
            Err(WorktreeError::Cancelled) => return Err(SessionError::Cancelled),
            Err(WorktreeError::IdentityMismatch(note)) => {
                let _ = self.boards.append(&board, "hub", &format!("{note}\n"), now);
                return Err(ChildLaunchError::new(409, "worktree_identity_mismatch", note).into());
            }
        };
        if !worktree.note.is_empty() {
            let _ = self
                .boards
                .append(&board, "hub", &format!("{}\n", worktree.note), now);
        }
        Ok(ChildPreparation {
            orchestration,
            board_path: board,
            absolute_cwd: cwd,
            child_cwd: worktree.cwd,
            branch: worktree.branch,
        })
    }
}
fn orchestration_id(parent: &SessionSnapshot, now: Timestamp) -> OrchestrationId {
    if parent.orchestration_id.0.is_empty() {
        OrchestrationId(format!("s{}-{}", parent.id.0, now.unix_nanos()))
    } else {
        parent.orchestration_id.clone()
    }
}

/// Mandatory application services for the still-separate board polling/initial
/// prompt task owner. None has a default implementation. Implementations must
/// retain restart data and schedule typed prompts once, not silently drop them.
pub trait ChildLaunchServices: Send + Sync {
    fn launch_arg_usable(&self, provider: &str) -> Result<bool, SessionError>;
    fn role_subscription(
        &self,
        orchestration: &OrchestrationId,
        role: &str,
    ) -> Result<Option<String>, SessionError>;
    fn trust_grant_providers(&self) -> Result<Vec<String>, SessionError>;
    fn apply_effects<'a>(
        &'a self,
        effects: CoreEffects,
    ) -> CoreFuture<'a, Result<(), SessionError>>;
    fn register_conductor(
        &self,
        parent: SessionBinding,
        orchestration: &OrchestrationId,
        board: &Path,
    ) -> Result<(), SessionError>;
    /// Accept ownership of board/restart metadata and enqueue any typed prompt.
    /// Return after enqueue, not after waiting for delivery/echo completion.
    fn child_registered<'a>(
        &'a self,
        child: RegisteredChild,
        cancel: TaskCancellation,
    ) -> CoreFuture<'a, Result<(), SessionError>>;
    fn notify_board_event<'a>(
        &'a self,
        parent: SessionBinding,
        orchestration: &'a OrchestrationId,
        text: String,
        cancel: TaskCancellation,
    ) -> CoreFuture<'a, Result<(), SessionError>>;
    /// Source notifier handles registered conductor boards, relay ownership,
    /// and the pre-board parent injection fallback in its existing owner.
    fn notify_orchestration_error<'a>(
        &'a self,
        parent: SessionBinding,
        limit: &'a str,
        detail: &'a str,
        cancel: TaskCancellation,
    ) -> CoreFuture<'a, Result<(), SessionError>>;
    fn confirmation_changed(
        &self,
        parent: SessionBinding,
        role: &str,
        provider_note: &str,
        option_note: &str,
    );
    fn warning(&self, operation: &'static str, error: &SessionError);
}
/// Real registration identity plus restart inputs, never a parallel session map.
pub struct RegisteredChild {
    pub binding: SessionBinding,
    pub parent: SessionBinding,
    /// Exact launch role retained before restart-only fields are cleared.
    pub role: String,
    pub preparation: ChildPreparation,
    /// Fixed source restart fields. Launch-only metadata/proof/prompt/label and
    /// one-shot folder trust are cleared; retry caller supplies a fresh context.
    pub restart_spec: WrappedSpawnSpec,
    pub initial_prompt: String,
    pub prompt_via_launch_arg: bool,
    pub inject_after_registration: Option<String>,
    pub spawned_at: Timestamp,
}

pub struct ChildLaunchExecutor {
    engine: Weak<SessionEngine>,
    config: Arc<ConfigStore>,
    preparation: ChildPreparer,
    services: Arc<dyn ChildLaunchServices>,
}
impl ChildLaunchExecutor {
    pub fn new(
        engine: Weak<SessionEngine>,
        config: Arc<ConfigStore>,
        preparation: ChildPreparer,
        services: Arc<dyn ChildLaunchServices>,
    ) -> Self {
        Self {
            engine,
            config,
            preparation,
            services,
        }
    }
    fn engine(&self) -> Result<Arc<SessionEngine>, SessionError> {
        self.engine.upgrade().ok_or(SessionError::Shutdown)
    }
    fn cfg(&self) -> Result<Config, SessionError> {
        self.config
            .snapshot()
            .map(|snapshot| snapshot.config)
            .map_err(|e| SessionError::Transport(e.to_string()))
    }
    async fn launch(
        &self,
        request: ConfirmedChildRequest,
        cancel: TaskCancellation,
    ) -> Result<ChildSpawnResult, SessionError> {
        let engine = self.engine()?;
        if cancel.token().is_cancelled() {
            return Err(SessionError::Cancelled);
        }
        let raw = request.body.request();
        let original = request.original_body.request();
        let provider_note = provider_confirmation_change_note(&original.provider, &raw.provider);
        let option_note = launch_option_change_note(original, raw);
        if !provider_note.is_empty() || !option_note.is_empty() {
            self.services.confirmation_changed(
                request.parent,
                &raw.role,
                &provider_note,
                &option_note,
            );
        }
        if !super::child_options::ORCHESTRATION_PROVIDERS.contains(&raw.provider.as_str()) {
            return Err(ChildLaunchError::new(
                400,
                "bad_request",
                format!(
                    "invalid provider {}; valid providers are: {}",
                    go_quote(&raw.provider),
                    super::child_options::ORCHESTRATION_PROVIDERS.join(", ")
                ),
            )
            .into());
        }
        if !valid_model(&raw.model) {
            return Err(ChildLaunchError::new(
                400,
                "bad_request",
                "invalid model selected for child",
            )
            .into());
        }
        let details = engine
            .details(request.parent.session)
            .ok_or_else(|| ChildLaunchError::new(404, "not_found", "parent session not found"))?;
        // A reconnect changes the connection epoch, not the retained parent.
        if details.binding.incarnation != request.parent.incarnation {
            return Err(SessionError::StaleBinding);
        }
        let parent_binding = details.binding;
        let parent = details.snapshot;
        let cfg = self.cfg()?;
        let resolved = super::child_options::prepare(request.body, &cfg)
            .map_err(|e| ChildLaunchError::new(400, "bad_request", e.to_string()))?;
        let body = resolved.request();
        if parent.depth >= cfg.orchestration.max_depth {
            self.notify_limit(
                parent_binding,
                "depth",
                "max orchestration depth reached",
                cancel.clone(),
            )
            .await;
            return Err(ChildLaunchError::new(
                429,
                "orchestration_limit",
                "max orchestration depth reached",
            )
            .into());
        }
        let admission = if engine.admission_matches(&request.admission, parent.id, 1) {
            request.admission
        } else {
            match engine.reserve_children(
                AdmissionRequest {
                    parent: parent.id,
                    slots: 1,
                    origin: resolved.origin().clone(),
                    replace: None,
                },
                AdmissionLimits {
                    max_children_per_parent: cfg.orchestration.max_children_per_parent,
                    max_total_sessions: cfg.orchestration.max_total_sessions,
                },
            ) {
                Ok(reservation) => reservation.id,
                Err(error) => {
                    let limit = match &error {
                        AdmissionError::ChildrenPerParent { .. } => Some("children_per_parent"),
                        AdmissionError::TotalSessions { .. } => Some("total_sessions"),
                        _ => None,
                    };
                    let error = admission_error(error);
                    if let (Some(limit), SessionError::ChildLaunch { detail, .. }) = (limit, &error)
                    {
                        self.notify_limit(parent_binding, limit, detail, cancel.clone())
                            .await;
                    }
                    return Err(error);
                }
            }
        };
        let _release = AdmissionRelease {
            engine: engine.clone(),
            admission,
        };
        if !body.force
            && let Some(id) = live_child_for_role(&engine, parent.id, &body.role)
        {
            return Err(ChildLaunchError::new(409, "duplicate_role_child", format!("live child #{} already exists for role {:?}; use `many-ai-cli orchestrate send --role {} \"<text>\"` to instruct it, or pass --force to spawn another", id.0, body.role, body.role)).into());
        }
        let prep = self
            .preparation
            .prepare(
                &parent,
                body,
                &cfg.orchestration,
                Timestamp::now(),
                &cancel,
                |id, board| {
                    let effects = engine.mark_conductor(
                        parent_binding,
                        id.clone(),
                        board.to_string_lossy().into_owned(),
                    );
                    async move { self.services.apply_effects(effects?).await }
                },
            )
            .await?;
        if !provider_note.is_empty() {
            let _ = self.preparation.boards.append(
                &prep.board_path,
                "hub",
                &format!("{provider_note}\n"),
                Timestamp::now(),
            );
        }
        let via_arg = !config::is_headless_execution_mode(&body.execution_mode)
            && config::launch_prompt_via_arg(&body.provider)
            && self.services.launch_arg_usable(&body.provider)?;
        let delivery = child_launch_prompt(
            &body.execution_mode,
            via_arg,
            &body.initial_prompt,
            &prep.board_path,
            &body.role,
            &prep.branch,
        );
        let spawned_at = Timestamp::now();
        let role_subscription = if body.subscription_profile_id.trim().is_empty() {
            self.services
                .role_subscription(&parent.orchestration_id, &body.role)?
        } else {
            None
        };
        let subscription = resolve_subscription(body, &parent, role_subscription, &cfg);
        let spec = WrappedSpawnSpec {
            registration_metadata: SpawnRegistrationMetadata {
                parent: parent.id,
                role: body.role.clone(),
                auto: true,
                depth: parent.depth + 1,
                orchestration: prep.orchestration.clone(),
                board_path: prep.board_path.to_string_lossy().into_owned(),
                worktree_branch: prep.branch.clone(),
                spawned_at: Some(spawned_at),
                prompt_at_launch: matches!(delivery, PromptDelivery::AtLaunch(_)),
                ..Default::default()
            },
            spawn_attempt: None,
            registration_proof: None,
            provider: body.provider.clone(),
            cwd: prep.child_cwd.clone(),
            model: body.model.clone(),
            model_selection: body.model_selection.clone(),
            risk_confirmed: body.risk_confirmed,
            label: format!(
                "orch-{}-{}-{}",
                safe_token(&prep.orchestration.0),
                body.role,
                spawned_at.unix_nanos()
            ),
            permission_mode: body.permission_mode.clone(),
            sandbox: body.sandbox.clone(),
            ask_for_approval: body.ask_for_approval.clone(),
            route: body.route.clone(),
            utf8_session: false,
            effort: body.effort.clone(),
            execution_mode: body.execution_mode.clone(),
            permission_preset: body.permission_preset.clone(),
            initial_prompt: match &delivery {
                PromptDelivery::AtLaunch(prompt) => prompt.clone(),
                PromptDelivery::AfterRegistration => String::new(),
            },
            subscription_profile_id: subscription,
            subscription_login: false,
            usage_probe: false,
            grants: resolved.grants().clone(),
            cancellation: cancel.clone(),
        };
        let waiter = HttpWaitCancellation::default();
        let binding = match engine
            .spawn_and_wait(spec.clone(), Duration::from_secs(20), &waiter)
            .await
        {
            SpawnWaitOutcome::Registered(binding) => binding,
            SpawnWaitOutcome::Failed(error) => {
                return Err(ChildLaunchError::new(
                    500,
                    "spawn_error",
                    format!("spawn error: {error}"),
                )
                .into());
            }
            SpawnWaitOutcome::TimedOut => {
                return Err(ChildLaunchError::new(
                    500,
                    "spawn_error",
                    "spawn error: context deadline exceeded",
                )
                .into());
            }
            SpawnWaitOutcome::WaiterCancelled => return Err(SessionError::Cancelled),
            SpawnWaitOutcome::HubStopped => return Err(SessionError::Shutdown),
        };
        self.services
            .register_conductor(parent_binding, &prep.orchestration, &prep.board_path)?;
        let inject = matches!(delivery, PromptDelivery::AfterRegistration).then(|| {
            child_initial_prompt(
                &body.initial_prompt,
                &prep.board_path,
                &body.role,
                &prep.branch,
                &binding.session.0.to_string(),
            )
        });
        self.services
            .child_registered(
                RegisteredChild {
                    binding,
                    parent: parent_binding,
                    role: body.role.clone(),
                    preparation: prep.clone(),
                    restart_spec: restart_spec(spec),
                    initial_prompt: body.initial_prompt.clone(),
                    prompt_via_launch_arg: via_arg,
                    inject_after_registration: inject,
                    spawned_at,
                },
                cancel,
            )
            .await?;
        self.remember_success(&request.requested_provider, body);
        Ok(ChildSpawnResult {
            id: binding.session,
            board_path: prep.board_path.to_string_lossy().into_owned(),
            cwd: prep.child_cwd.to_string_lossy().into_owned(),
            worktree_branch: prep.branch,
        })
    }
    async fn notify_limit(
        &self,
        parent: SessionBinding,
        limit: &str,
        detail: &str,
        cancel: TaskCancellation,
    ) {
        if let Err(error) = self
            .services
            .notify_orchestration_error(parent, limit, detail, cancel)
            .await
        {
            self.services.warning("notify orchestration limit", &error);
        }
    }
    fn remember_success(&self, requested_provider: &str, body: &ChildSpawnRequest) {
        if requested_provider.trim().is_empty() {
            self.update_memory("remember role provider", |cfg| remember_provider(cfg, body));
        }
        self.update_memory("remember role effort", |cfg| remember_effort(cfg, body));
        self.update_memory("remember role permission", |cfg| {
            remember_permission(cfg, body)
        });
    }
    fn update_memory(&self, operation: &'static str, apply: impl Fn(&mut Config) -> bool) {
        let result = loop {
            let mut snapshot = match self.config.snapshot() {
                Ok(snapshot) => snapshot,
                Err(error) => break Err(error),
            };
            if !apply(&mut snapshot.config) {
                return;
            }
            match self
                .config
                .publish_then_persist_legacy(snapshot.revision, snapshot.config)
            {
                Err(ConfigError::Conflict { .. }) => continue,
                result => break result.map(|_| ()),
            }
        };
        if let Err(error) = result {
            self.services
                .warning(operation, &SessionError::Transport(error.to_string()));
        }
    }
}
impl ConfirmationExecutor for ChildLaunchExecutor {
    fn presentation(&self) -> Result<ConfirmationPresentation, SessionError> {
        Ok(ConfirmationPresentation {
            config: self.cfg()?,
            trust_grant_providers: self.services.trust_grant_providers()?,
        })
    }
    fn spawn<'a>(
        &'a self,
        request: ConfirmedChildRequest,
        cancel: TaskCancellation,
    ) -> CoreFuture<'a, Result<ChildSpawnResult, SessionError>> {
        Box::pin(self.launch(request, cancel))
    }
    fn record_refusal<'a>(
        &'a self,
        pending: &'a PendingSpawnConfirmation,
        cancel: TaskCancellation,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            if cancel.token().is_cancelled() {
                return Err(SessionError::Cancelled);
            }
            let engine = self.engine()?;
            let Some(parent) = engine.details(pending.parent) else {
                return Ok(());
            };
            let now = Timestamp::now();
            let id = orchestration_id(&parent.snapshot, now);
            let board = self.preparation.boards.ensure(
                &id.0,
                &parent.snapshot,
                &pending.body.request().initial_prompt,
                now,
            )?;
            let effects = engine.mark_conductor(
                parent.binding,
                id.clone(),
                board.to_string_lossy().into_owned(),
            )?;
            self.services.apply_effects(effects).await?;
            self.services
                .register_conductor(parent.binding, &id, &board)?;
            self.preparation
                .boards
                .append(
                    &board,
                    "hub",
                    &format!(
                        "refused: role={} reason=user_refusal\n",
                        pending.body.request().role
                    ),
                    now,
                )
                .map_err(|e| ChildLaunchError::board(e).into())
        })
    }
    fn notify_waiter_gone<'a>(
        &'a self,
        pending: &'a PendingSpawnConfirmation,
        result: &'a Result<ChildSpawnResult, SessionError>,
        cancel: TaskCancellation,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            let role = &pending.body.request().role;
            let text = match result {
                Ok(child) => {
                    if let Err(error) = self.preparation.boards.append(Path::new(&child.board_path), "hub", &format!("spawned (confirmed after the requesting session disconnected): role={role} session={}\n", child.id.0), Timestamp::now()) {
                        self.services.warning("record child spawned after waiter gone", &ChildLaunchError::board(error).into());
                    }
                    format!(
                        "[MANY-AI-CLI] child spawned after your earlier request lost its connection: role={role} session=#{}",
                        child.id.0
                    )
                }
                Err(error) => {
                    let detail = match error {
                        SessionError::ChildLaunch { detail, .. } => detail.clone(),
                        _ => format!("{error:?}"),
                    };
                    format!(
                        "[MANY-AI-CLI] child spawn failed after your earlier request lost its connection: role={role} detail={detail}"
                    )
                }
            };
            let engine = self.engine()?;
            let Some(parent) = engine.details(pending.parent) else {
                return Ok(());
            };
            if parent.snapshot.orchestration_id.0.is_empty() {
                return Ok(());
            }
            self.services
                .notify_board_event(
                    parent.binding,
                    &parent.snapshot.orchestration_id,
                    text,
                    cancel,
                )
                .await
        })
    }
}
fn go_quote(text: &str) -> String {
    crate::proto::go_quote::quote(text)
}
pub fn provider_confirmation_change_note(requested: &str, decided: &str) -> String {
    if requested == decided {
        String::new()
    } else {
        format!("provider changed at confirmation: requested={requested} decided={decided}")
    }
}
pub fn launch_option_change_note(
    requested: &ChildSpawnRequest,
    decided: &ChildSpawnRequest,
) -> String {
    [
        ("effort", &requested.effort, &decided.effort),
        (
            "execution_mode",
            &requested.execution_mode,
            &decided.execution_mode,
        ),
        (
            "permission_preset",
            &requested.permission_preset,
            &decided.permission_preset,
        ),
    ]
    .into_iter()
    .filter(|(_, before, after)| before != after)
    .map(|(name, before, after)| {
        format!(
            "{name}: requested={} decided={}",
            go_quote(before),
            go_quote(after)
        )
    })
    .collect::<Vec<_>>()
    .join(", ")
}
fn restart_spec(mut spec: WrappedSpawnSpec) -> WrappedSpawnSpec {
    spec.registration_metadata = SpawnRegistrationMetadata::default();
    spec.spawn_attempt = None;
    spec.registration_proof = None;
    spec.label.clear();
    spec.initial_prompt.clear();
    spec.grants = InternalSpawnGrants::from_config(spec.grants.allowed_tools().to_vec());
    spec.cancellation = TaskCancellation::default();
    spec
}
struct AdmissionRelease {
    engine: Arc<SessionEngine>,
    admission: AdmissionId,
}
impl Drop for AdmissionRelease {
    fn drop(&mut self) {
        self.engine.release_children(&self.admission);
    }
}
fn live_child_for_role(
    engine: &SessionEngine,
    parent: LiveSessionId,
    role: &str,
) -> Option<LiveSessionId> {
    engine
        .snapshots()
        .into_iter()
        .filter(|session| {
            session.parent_session_id == parent
                && session.role == role
                && !matches!(
                    session.state.as_str(),
                    "completed" | "error" | "disconnected" | "done" | "timeout" | "dismissed"
                )
                && engine
                    .details(session.id)
                    .is_some_and(|details| details.connected)
        })
        .map(|session| session.id)
        .max()
}
pub(crate) fn admission_error(error: AdmissionError) -> SessionError {
    let detail = match error {
        AdmissionError::ParentNotFound => {
            return ChildLaunchError::new(404, "not_found", "parent session not found").into();
        }
        AdmissionError::ChildrenPerParent { used, maximum, .. } => format!(
            "orchestration limit children_per_parent reached (sessions and reserved slots={used}, max={maximum})"
        ),
        AdmissionError::TotalSessions { used, maximum, .. } => format!(
            "orchestration limit total_sessions reached (sessions and reserved slots={used}, max={maximum})"
        ),
        error => {
            return ChildLaunchError::new(500, "orchestration_limit", format!("{error:?}")).into();
        }
    };
    ChildLaunchError::new(429, "orchestration_limit", detail).into()
}
fn valid_model(model: &str) -> bool {
    !model.starts_with('-')
        && !model
            .chars()
            .any(|c| c < ' ' || c == '\u{7f}' || " \"'|&><^%();`$".contains(c))
}
fn resolve_subscription(
    body: &ChildSpawnRequest,
    parent: &SessionSnapshot,
    role: Option<String>,
    cfg: &Config,
) -> String {
    if !body.subscription_profile_id.trim().is_empty() {
        return body.subscription_profile_id.trim().into();
    }
    if let Some(role) = role.filter(|role| !role.trim().is_empty()) {
        return role;
    }
    if parent.provider == body.provider && !parent.subscription_profile_id.trim().is_empty() {
        return parent.subscription_profile_id.clone();
    }
    cfg.user_prefs
        .spawn
        .defaults
        .get(&format!("subscription_{}", body.provider))
        .map(|s| s.trim().into())
        .unwrap_or_default()
}
fn replace_memory(
    map: &mut std::collections::BTreeMap<String, String>,
    role: &str,
    value: &str,
) -> bool {
    if map.get(role).map(String::as_str).unwrap_or("") == value {
        return false;
    }
    if value.is_empty() {
        map.remove(role);
    } else {
        map.insert(role.into(), value.into());
    }
    true
}
fn remember_provider(cfg: &mut Config, body: &ChildSpawnRequest) -> bool {
    let role = body.role.trim();
    let provider = body.provider.trim();
    if role.is_empty() || !super::child_options::ORCHESTRATION_PROVIDERS.contains(&provider) {
        return false;
    }
    replace_memory(&mut cfg.user_prefs.spawn.role_provider, role, provider)
}
fn remember_effort(cfg: &mut Config, body: &ChildSpawnRequest) -> bool {
    let role = body.role.trim();
    let effort = body.effort.trim();
    if role.is_empty()
        || (!effort.is_empty() && config::validate_effort(&body.provider, effort).is_err())
    {
        return false;
    }
    replace_memory(&mut cfg.user_prefs.spawn.role_effort, role, effort)
}
fn remember_permission(cfg: &mut Config, body: &ChildSpawnRequest) -> bool {
    let Some(remember) = body.remember_permission else {
        return false;
    };
    let role = body.role.trim();
    let tier = body.permission_preset.trim();
    if role.is_empty() || (remember && config::validate_permission_preset(tier).is_err()) {
        return false;
    }
    replace_memory(
        &mut cfg.user_prefs.spawn.role_permission,
        role,
        if remember { tier } else { "" },
    )
}

#[cfg(test)]
mod tests;
