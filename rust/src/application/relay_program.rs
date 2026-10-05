//! Production relay owner. Transitions are Hub decisions over explicit DONE and
//! verdict evidence, with the existing session, child launch and task owners.
mod lifecycle;
#[cfg(test)]
mod tests;
mod transitions;
use super::{event_observer::EventWarning, orchestration_program::OrchestrationProgram};
use crate::{
    config::{self, ConfigStore, RuntimePaths},
    hub::task_owner::HubTaskHandle,
    orchestration::{
        child_launch::{
            ChildLaunchExecutor, ChildLaunchServices, ChildPreparation, board::BoardStore,
            worktree::WorktreeGit,
        },
        relay::{
            self, IMPLEMENTATION, REVIEW, ROLES, RelayFile, Role, Run, STRONG, store::RelayStore,
            worktree::RelayGit,
        },
    },
    proto::{
        self,
        core::*,
        time::{self, Timestamp},
    },
    terminal::session::{SessionEngine, confirmations::ConfirmedChildRequest},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Weak},
    time::Duration,
};
use tokio::sync::Mutex as AsyncMutex;
fn lock<T>(value: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    value
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[derive(Clone, Default, Deserialize)]
#[serde(default)]
pub struct RelayStartRequest {
    pub plan_path: String,
    pub max_rounds: i64,
    pub mode: String,
    #[serde(deserialize_with = "crate::orchestration::relay::wire::null_default")]
    pub roles: BTreeMap<String, Option<Role>>,
    pub escalate_after: i64,
    #[serde(deserialize_with = "crate::orchestration::relay::wire::null_default")]
    pub extra: BTreeMap<String, String>,
    pub acknowledge_child_full_bypass: bool,
}
#[derive(Clone, Default, Deserialize)]
#[serde(default)]
pub struct RelayControlRequest {
    pub orchestration_id: String,
}
#[derive(Clone, Serialize)]
pub struct RelayItem {
    pub relay: proto::RelayStatus,
    #[serde(serialize_with = "crate::orchestration::relay::wire::serialize_events")]
    pub events: Vec<proto::RelayEvent>,
}
#[derive(Clone, Debug)]
pub struct RelayError {
    pub status: u16,
    pub code: String,
    pub detail: String,
}
impl RelayError {
    fn new(status: u16, code: &str, detail: impl Into<String>) -> Self {
        Self {
            status,
            code: code.into(),
            detail: detail.into(),
        }
    }
    fn missing() -> Self {
        Self::new(404, "relay_not_found", "relay not found")
    }
    fn parent() -> Self {
        Self::new(404, "not_found", "parent session not found")
    }
    fn bad(detail: impl Into<String>) -> Self {
        Self::new(400, "bad_request", detail)
    }
}
impl From<SessionError> for RelayError {
    fn from(error: SessionError) -> Self {
        match error {
            SessionError::ChildLaunch {
                status,
                code,
                detail,
            } => Self {
                status,
                code,
                detail,
            },
            SessionError::Shutdown | SessionError::Cancelled => {
                Self::new(503, "unavailable", "relay owner is shutting down")
            }
            other => Self::new(
                500,
                "spawn_error",
                format!("relay request failed: {other:?}"),
            ),
        }
    }
}
impl std::fmt::Display for RelayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.detail)
    }
}
impl std::error::Error for RelayError {}

pub struct RelayDependencies {
    pub core: Weak<SessionEngine>,
    pub effects: Weak<dyn CoreEffectSink>,
    pub config: Arc<ConfigStore>,
    pub paths: RuntimePaths,
    pub orchestration: Arc<OrchestrationProgram>,
    pub executor: Arc<ChildLaunchExecutor>,
    pub git: WorktreeGit,
    pub hub_cwd: PathBuf,
    pub tasks: HubTaskHandle,
    pub warning: EventWarning,
}
struct Meta {
    sequence: u64,
    parent: i64,
    attached: bool,
    restoring: bool,
    parent_identity: (String, String, String),
    child_labels: Vec<(String, String)>,
    board_path: String,
    item: RelayItem,
}
#[derive(Default)]
struct State {
    runs: BTreeMap<String, Arc<AsyncMutex<Run>>>,
    starting_ids: BTreeSet<String>,
    starting_parents: BTreeSet<LiveSessionId>,
    meta: BTreeMap<String, Meta>,
    sequence: u64,
}
pub struct RelayProgram {
    deps: RelayDependencies,
    store: RelayStore,
    boards: Arc<BoardStore>,
    git: RelayGit,
    state: Mutex<State>,
    started: std::sync::atomic::AtomicBool,
}
pub struct RelayGuard {
    cancel: TaskCancellation,
    join: Option<tokio::task::JoinHandle<()>>,
}
impl Drop for RelayGuard {
    fn drop(&mut self) {
        self.cancel.token().cancel();
        if let Some(join) = self.join.take() {
            join.abort();
        }
    }
}
impl RelayGuard {
    pub async fn stop_and_join(mut self) {
        self.cancel.token().cancel();
        if let Some(mut join) = self.join.take()
            && tokio::time::timeout(Duration::from_secs(3), &mut join)
                .await
                .is_err()
        {
            join.abort();
            let _ = join.await;
        }
    }
}
/// Cleanup releases the async run lock while awaiting child dismissal. Keep an
/// explicit claim across that gap so resume cannot create a new worktree user.
/// Drop is synchronous and releases ownership even if the HTTP task is aborted.
struct CleanupClaim(Arc<std::sync::atomic::AtomicBool>);
impl CleanupClaim {
    fn acquire(flag: Arc<std::sync::atomic::AtomicBool>) -> Result<Self, RelayError> {
        flag.compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
        )
        .map_err(|_| Self::busy())?;
        Ok(Self(flag))
    }
    fn busy() -> RelayError {
        RelayError::new(
            409,
            "relay_cleanup_in_progress",
            "relay worktree cleanup is in progress",
        )
    }
}
impl Drop for CleanupClaim {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::Release);
    }
}

struct AdmissionRelease {
    core: Arc<SessionEngine>,
    id: AdmissionId,
    armed: bool,
}
struct StartupRollback {
    parent: SessionBinding,
    previous_conductor: (OrchestrationId, String),
    detail: String,
    now: Timestamp,
}
impl Drop for AdmissionRelease {
    fn drop(&mut self) {
        if self.armed {
            self.core.release_children(&self.id);
        }
    }
}
/// Reserve the stable relay ID before worktree preparation can yield. The ID
/// is moved to `runs` when the startup checkpoint is installed.
struct StartIdClaim<'a> {
    state: &'a Mutex<State>,
    id: String,
    armed: bool,
}
impl StartIdClaim<'_> {
    fn install(mut self, run: Arc<AsyncMutex<Run>>) {
        let mut state = lock(self.state);
        state.starting_ids.remove(&self.id);
        state.runs.insert(self.id.clone(), run);
        self.armed = false;
    }
}
impl Drop for StartIdClaim<'_> {
    fn drop(&mut self) {
        if self.armed {
            lock(self.state).starting_ids.remove(&self.id);
        }
    }
}
/// Only one startup may replace a parent's single conductor card at a time.
/// Keep this claim through child spawn or rollback so a failed predecessor can
/// never be restored by a later startup's compensation.
struct StartParentClaim<'a> {
    state: &'a Mutex<State>,
    parent: LiveSessionId,
}
impl Drop for StartParentClaim<'_> {
    fn drop(&mut self) {
        lock(self.state).starting_parents.remove(&self.parent);
    }
}
/// An owned operation may be dropped by Hub shutdown while awaiting IO. Keep
/// its registered children recoverable and release the canonical reservation
/// synchronously; HTTP waiter cancellation does not drop this operation.
struct TransitionGuard<'a> {
    owner: &'a RelayProgram,
    run: &'a mut Run,
    armed: bool,
}
impl Drop for TransitionGuard<'_> {
    fn drop(&mut self) {
        if !self.armed || self.run.file.terminal() {
            return;
        }
        if let Ok(core) = self.owner.core() {
            core.release_children(&self.run.admission);
        }
        self.run.admission = AdmissionId::default();
        self.run.file.state = "stopped".into();
        self.run.file.reason = "hub_restart".into();
        let now = Timestamp::now();
        self.run.file.updated_at = time::format_rfc3339(now).unwrap_or_default();
        self.run.file.event(
            "stopped",
            "Hub canceled an in-progress relay transition; resume to continue".into(),
            now,
        );
        self.owner.board(
            self.run,
            "relay transition interrupted by Hub shutdown; children and worktree retained",
            now,
        );
        self.owner.publish_meta(self.run);
        if let Err(error) = self.owner.store.save(&self.run.file) {
            self.owner.warn("relay interrupted transition save", error);
        }
    }
}
impl RelayProgram {
    pub fn new(deps: RelayDependencies) -> Result<Arc<Self>, SessionError> {
        crate::profile::subscriptions::check_path(&deps.paths, &deps.hub_cwd)
            .map_err(|e| SessionError::InvalidRequest(e.to_string()))?;
        Ok(Arc::new(Self {
            store: RelayStore::new(&deps.paths),
            boards: deps.orchestration.board_store(),
            git: RelayGit::new(deps.paths.clone(), deps.git.clone(), deps.hub_cwd.clone()),
            deps,
            state: Mutex::new(State::default()),
            started: false.into(),
        }))
    }
    pub fn tasks(&self) -> HubTaskHandle {
        self.deps.tasks.clone()
    }
    fn core(&self) -> Result<Arc<SessionEngine>, SessionError> {
        self.deps.core.upgrade().ok_or(SessionError::Shutdown)
    }
    /// Restored numeric IDs may already belong to unrelated sessions. Match the
    /// immutable launch identity even without restored relay metadata, including children that have
    /// reattached under a new ID but are still waiting for the run's pre-ACK lock.
    fn owned_children(&self, run: &Run, core: &SessionEngine) -> Vec<SessionBinding> {
        core.snapshots()
            .into_iter()
            .filter_map(|snapshot| {
                let launch_label_owned = ROLES.iter().any(|role| {
                    !run.file.child_label(role).is_empty()
                        && snapshot.launch_label == run.file.child_label(role)
                });
                // A child checkpoint may fail after the session was registered.
                // Its trusted launch metadata still carries the relay identity,
                // allowing a later cleanup or restart to find it by role.
                let relay_metadata_owned = snapshot.orchestration_id.0 == run.file.orchestration_id
                    && ROLES.contains(&snapshot.role.as_str());
                let owned = launch_label_owned || relay_metadata_owned;
                owned
                    .then(|| core.details(snapshot.id))
                    .flatten()
                    .map(|details| details.binding)
            })
            .collect()
    }
    fn cfg(&self) -> Result<config::Config, RelayError> {
        self.deps
            .config
            .snapshot()
            .map(|s| s.config)
            .map_err(|e| RelayError::new(500, "spawn_error", e.to_string()))
    }
    fn warn(&self, operation: &str, error: impl std::fmt::Debug) {
        (self.deps.warning)(operation, &SessionError::Transport(format!("{error:?}")));
    }
    async fn effects(&self, effects: CoreEffects) -> Result<(), SessionError> {
        self.deps
            .effects
            .upgrade()
            .ok_or(SessionError::Shutdown)?
            .apply(effects)
            .await
            .map_err(|e| e.error)
    }
    /// Immutable restored identities are resolved before core initialization,
    /// allowing its existing StartSession writer to persist metadata before ACK.
    /// No run lock or asynchronous work is allowed at this boundary.
    pub fn resolve_reattach_metadata(
        &self,
        message: &proto::Message,
    ) -> Option<SpawnRegistrationMetadata> {
        let label = message.label.trim();
        let state = lock(&self.state);
        for (id, meta) in &state.meta {
            if !meta.restoring {
                continue;
            }
            if !label.is_empty()
                && let Some((role, _)) = meta.child_labels.iter().find(|(_, saved)| saved == label)
            {
                return Some(SpawnRegistrationMetadata {
                    parent: LiveSessionId(if meta.attached { meta.parent } else { 0 }),
                    role: role.clone(),
                    auto: true,
                    depth: 1,
                    orchestration: OrchestrationId(id.clone()),
                    board_path: meta.board_path.clone(),
                    ..Default::default()
                });
            }
            if !meta.attached
                && !meta.parent_identity.0.is_empty()
                && message.started_at == meta.parent_identity.0
                && message.cwd == meta.parent_identity.1
                && message.provider == meta.parent_identity.2
            {
                return Some(SpawnRegistrationMetadata {
                    orchestration: OrchestrationId(id.clone()),
                    board_path: meta.board_path.clone(),
                    ..Default::default()
                });
            }
        }
        None
    }
    pub fn owns(&self, id: &str) -> bool {
        let state = lock(&self.state);
        state.runs.contains_key(id) || state.starting_ids.contains(id)
    }
    fn claim_start_id(&self, id: &str) -> Result<StartIdClaim<'_>, RelayError> {
        let mut state = lock(&self.state);
        if state.runs.contains_key(id) || !state.starting_ids.insert(id.to_owned()) {
            return Err(RelayError::new(
                409,
                "relay_identity_collision",
                "relay identity already exists",
            ));
        }
        Ok(StartIdClaim {
            state: &self.state,
            id: id.to_owned(),
            armed: true,
        })
    }
    fn claim_start_parent(
        &self,
        parent: LiveSessionId,
    ) -> Result<StartParentClaim<'_>, RelayError> {
        if !lock(&self.state).starting_parents.insert(parent) {
            return Err(RelayError::new(
                409,
                "relay_startup_in_progress",
                "another relay startup for this parent is still in progress",
            ));
        }
        Ok(StartParentClaim {
            state: &self.state,
            parent,
        })
    }
    pub fn list(&self, parent: LiveSessionId) -> Result<Vec<RelayItem>, RelayError> {
        if self.core()?.details(parent).is_none() {
            return Err(RelayError::parent());
        }
        let state = lock(&self.state);
        let mut items = state
            .meta
            .values()
            .filter(|m| m.attached && m.parent == parent.0)
            .collect::<Vec<_>>();
        items.sort_by_key(|m| m.sequence);
        Ok(items.into_iter().map(|m| m.item.clone()).collect())
    }
    fn resolve(
        &self,
        parent: LiveSessionId,
        id: &str,
        resume: bool,
    ) -> Result<Arc<AsyncMutex<Run>>, RelayError> {
        let state = lock(&self.state);
        let id = id.trim();
        if !id.is_empty() {
            let meta = state.meta.get(id).ok_or_else(RelayError::missing)?;
            if !resume && (!meta.attached || meta.parent != parent.0) {
                return Err(RelayError::missing());
            }
            return state.runs.get(id).cloned().ok_or_else(RelayError::missing);
        }
        let mut selected = state
            .meta
            .iter()
            .filter(|(_, m)| m.attached && m.parent == parent.0)
            .filter(|(_, m)| {
                if resume {
                    m.item.relay.state == "stopped"
                        && matches!(
                            m.item.relay.reason.as_str(),
                            "hub_restart" | "child_exited" | "timeout"
                        )
                } else {
                    !matches!(m.item.relay.state.as_str(), "completed" | "stopped")
                }
            });
        let Some((id, _)) = selected.next() else {
            return Err(RelayError::missing());
        };
        if selected.next().is_some() {
            let ids = state
                .meta
                .iter()
                .filter(|(_, meta)| {
                    meta.attached
                        && meta.parent == parent.0
                        && !matches!(meta.item.relay.state.as_str(), "completed" | "stopped")
                })
                .map(|(id, _)| id.as_str())
                .collect::<Vec<_>>();
            let detail = if ids.is_empty() {
                "more than one relay is running; specify orchestration_id".to_owned()
            } else {
                format!(
                    "more than one relay is running; specify orchestration_id (active: {})",
                    ids.join(", ")
                )
            };
            return Err(RelayError::new(400, "relay_ambiguous", detail));
        }
        state.runs.get(id).cloned().ok_or_else(RelayError::missing)
    }
    fn reserve(&self, parent: LiveSessionId, slots: i64) -> Result<AdmissionRelease, RelayError> {
        let core = self.core()?;
        let cfg = self.cfg()?.orchestration;
        let reservation = core
            .reserve_children(
                AdmissionRequest {
                    parent,
                    slots,
                    origin: VerifiedSpawnOrigin::Autonomous,
                    replace: None,
                },
                AdmissionLimits {
                    max_children_per_parent: cfg.max_children_per_parent,
                    max_total_sessions: cfg.max_total_sessions,
                },
            )
            .map_err(|error| match error {
                AdmissionError::ChildrenPerParent{used,maximum,..} => {
                    let running=lock(&self.state).meta.values().filter(|m|m.parent==parent.0&&m.attached&&!matches!(m.item.relay.state.as_str(),"completed"|"stopped")).count();
                    RelayError::new(429,"orchestration_limit",format!("orchestration limit children_per_parent reached (sessions and reserved slots={used}, max={maximum}); relay running={running} max_children_per_parent={maximum}; dismiss open children or adjust the automatic child limit in Settings (up to 256)"))
                },
                error => RelayError::from(crate::orchestration::child_launch::admission_error(error))
            })?;
        Ok(AdmissionRelease {
            core,
            id: reservation.id,
            armed: true,
        })
    }
    pub async fn start_relay(
        &self,
        parent: LiveSessionId,
        mut request: RelayStartRequest,
        now: Timestamp,
        cancel: TaskCancellation,
    ) -> Result<(proto::RelayStatus, String), RelayError> {
        let core = self.core()?;
        let parent = core.details(parent).ok_or_else(RelayError::parent)?;
        let plan = self
            .git
            .resolve_plan(Path::new(&parent.snapshot.cwd), &request.plan_path, &cancel)
            .await
            .map_err(RelayError::bad)?;
        request.max_rounds = positive_range(request.max_rounds, 3, 9, "max_rounds")?;
        request.escalate_after = positive_range(request.escalate_after, 2, 5, "escalate_after")?;
        request.mode = request.mode.trim().to_owned();
        if request.mode.is_empty() {
            request.mode = "worktree".into()
        }
        if !matches!(request.mode.as_str(), "worktree" | "same-tree") {
            return Err(RelayError::bad(
                "mode must be \"worktree\" or \"same-tree\"",
            ));
        }
        let mut roles = self
            .deps
            .orchestration
            .roles(&parent.snapshot.orchestration_id)
            .into_iter()
            .map(|(key, value)| (key, Role::from(value)))
            .collect::<BTreeMap<_, _>>();
        let prior = {
            let state = lock(&self.state);
            state
                .meta
                .values()
                .filter(|m| m.attached && m.parent == parent.binding.session.0)
                .min_by_key(|m| m.sequence)
                .and_then(|m| state.runs.get(&m.item.relay.orchestration_id))
                .cloned()
        };
        if let Some(prior) = prior {
            for (key, value) in &prior.lock().await.file.roles {
                roles.entry(key.clone()).or_insert_with(|| value.clone());
            }
        }
        for (role, value) in &request.roles {
            if let Some(value) = value {
                roles.entry(role.clone()).or_default().overlay(value);
            }
        }
        let cfg = self.cfg()?;
        if requires_bypass_ack(&cfg.orchestration, &roles) && !request.acknowledge_child_full_bypass
        {
            return Err(RelayError::new(
                400,
                "full_bypass_acknowledgment_required",
                "relay children default to full permission bypass; set acknowledge_child_full_bypass:true to confirm (or set orchestration.child_full_bypass:false / child_permission_default / per-role permission_preset)",
            ));
        }
        let roles = validate_roles(roles, &cfg)?;
        if parent.snapshot.depth >= cfg.orchestration.max_depth {
            return Err(RelayError::new(
                429,
                "orchestration_limit",
                format!(
                    "orchestration limit depth reached (sessions and reserved slots=0, max={})",
                    cfg.orchestration.max_depth
                ),
            ));
        }
        let id = format!("r{}-{}", parent.binding.session.0, now.unix_nanos());
        let start_id_claim = self.claim_start_id(&id)?;
        let _start_parent_claim = self.claim_start_parent(parent.binding.session)?;
        let mut admission = self.reserve(parent.binding.session, 2)?;
        let (worktree, branch, base) = if request.mode == "worktree" {
            self.git
                .prepare(
                    Path::new(&parent.snapshot.cwd),
                    &id,
                    &cfg.orchestration,
                    &cancel,
                )
                .await
                .map_err(|e| {
                    if e.not_git {
                        RelayError::new(400, "relay_not_git", e.detail)
                    } else {
                        RelayError::new(500, "relay_worktree_error", "relay worktree setup failed")
                    }
                })?
        } else {
            (PathBuf::new(), String::new(), String::new())
        };
        // `prepare` returns an empty base when it reuses an existing worktree.
        // A failed startup may clean only a worktree this invocation created.
        let created_worktree = request.mode == "worktree" && !base.is_empty();
        let child_cwd = if worktree.as_os_str().is_empty() {
            parent.snapshot.cwd.clone()
        } else {
            worktree.to_string_lossy().into_owned()
        };
        let base = if base.is_empty() {
            self.git
                .head(Path::new(&child_cwd), &cancel)
                .await
                .unwrap_or_default()
        } else {
            base
        };
        let board_path = self.boards.path(&id);
        let mut previous_conductor = (
            parent.snapshot.orchestration_id.clone(),
            parent.snapshot.board_path.clone(),
        );
        let parent_cwd = parent.snapshot.cwd.clone();
        let mut run = Run::new(
            RelayFile {
                version: 1,
                orchestration_id: id.clone(),
                board_path: board_path.to_string_lossy().into_owned(),
                parent_session_id: parent.binding.session.0,
                parent_started_at: parent.snapshot.started_at.clone(),
                parent_provider: parent.snapshot.provider.clone(),
                parent_cwd: parent_cwd.clone(),
                worktree_origin_cwd: if request.mode == "worktree" {
                    parent_cwd.clone()
                } else {
                    String::new()
                },
                plan_path: plan.to_string_lossy().into_owned(),
                mode: request.mode,
                max_rounds: request.max_rounds,
                escalate_after: request.escalate_after,
                active_implementer: IMPLEMENTATION.into(),
                roles,
                extra: request.extra,
                child_cwd,
                worktree_path: worktree.to_string_lossy().into_owned(),
                branch,
                base_commit: base.clone(),
                last_reviewed_commit: base,
                updated_at: time::format_rfc3339(now).unwrap_or_default(),
                ..Default::default()
            },
            now,
        );
        run.parent_attached = true;
        run.admission = admission.id.clone();
        {
            let mut state = lock(&self.state);
            state.sequence += 1;
            run.sequence = state.sequence;
        }
        run.file.event(
            "started",
            plan.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            now,
        );
        if let Some(event) = run.file.events.last_mut() {
            event.commit = run.file.base_commit.clone();
        }
        let run = Arc::new(AsyncMutex::new(run));
        start_id_claim.install(run.clone());
        let mut run = run.lock().await;
        self.publish_meta(&run);
        if let Err(error) = self.store.save(&run.file) {
            self.warn("relay startup checkpoint", &error);
            if created_worktree {
                match self
                    .git
                    .cleanup(Path::new(&parent_cwd), &worktree, &cancel)
                    .await
                {
                    Ok(()) => {
                        run.file.worktree_path.clear();
                        run.file.child_cwd = parent_cwd.clone();
                    }
                    Err(cleanup_error) => {
                        self.warn("relay startup worktree rollback", &cleanup_error);
                    }
                }
            }
            self.rollback_startup(
                &core,
                &mut run,
                &mut admission,
                StartupRollback {
                    parent: parent.binding,
                    previous_conductor: previous_conductor.clone(),
                    detail: "relay startup record could not be saved".into(),
                    now,
                },
            )
            .await;
            return Err(RelayError::new(
                500,
                "relay_persistence_error",
                "relay startup record could not be saved",
            ));
        }
        let board = match self.boards.ensure(
            &id,
            &parent.snapshot,
            &format!(
                "relay: {}",
                plan.file_name().unwrap_or_default().to_string_lossy()
            ),
            now,
        ) {
            Ok(board) => board,
            Err(error) => {
                let relay_error = RelayError::new(500, "board_error", error.to_string());
                self.rollback_startup(
                    &core,
                    &mut run,
                    &mut admission,
                    StartupRollback {
                        parent: parent.binding,
                        previous_conductor: previous_conductor.clone(),
                        detail: relay_error.detail.clone(),
                        now,
                    },
                )
                .await;
                return Err(relay_error);
            }
        };
        self.deps
            .orchestration
            .claim_relay_board(&OrchestrationId(id.clone()));
        let (mark_effects, replaced_conductor) = match core.mark_conductor_with_previous(
            parent.binding,
            OrchestrationId(id.clone()),
            board.to_string_lossy().into_owned(),
        ) {
            Ok(receipt) => receipt,
            Err(error) => {
                let relay_error = RelayError::from(error);
                self.rollback_startup(
                    &core,
                    &mut run,
                    &mut admission,
                    StartupRollback {
                        parent: parent.binding,
                        previous_conductor: previous_conductor.clone(),
                        detail: relay_error.detail.clone(),
                        now,
                    },
                )
                .await;
                return Err(relay_error);
            }
        };
        previous_conductor = replaced_conductor;
        if let Err(error) = self.effects(mark_effects).await {
            let relay_error = RelayError::from(error);
            self.rollback_startup(
                &core,
                &mut run,
                &mut admission,
                StartupRollback {
                    parent: parent.binding,
                    previous_conductor: previous_conductor.clone(),
                    detail: relay_error.detail.clone(),
                    now,
                },
            )
            .await;
            return Err(relay_error);
        }
        if let Err(error) = self.deps.orchestration.register_conductor(
            parent.binding,
            &OrchestrationId(id.clone()),
            &board,
        ) {
            let relay_error = RelayError::from(error);
            self.rollback_startup(
                &core,
                &mut run,
                &mut admission,
                StartupRollback {
                    parent: parent.binding,
                    previous_conductor: previous_conductor.clone(),
                    detail: relay_error.detail.clone(),
                    now,
                },
            )
            .await;
            return Err(relay_error);
        }
        self.board(
            &run,
            &format!(
                "relay started plan={} mode={} max_rounds={} escalate_after={} base={}",
                run.file.plan_path,
                run.file.mode,
                run.file.max_rounds,
                run.file.escalate_after,
                run.file.base_commit
            ),
            now,
        );
        let prompt = run.file.prompts().implementation_prompt();
        if let Err(error) = self
            .spawn(&mut run, IMPLEMENTATION, prompt, None, &cancel, now)
            .await
        {
            self.finish(
                &mut run,
                "stopped",
                failure_reason(&error),
                &error.to_string(),
                now,
            )
            .await;
            return Err(error);
        }
        self.transition(&mut run, "implementing", "", now).await;
        admission.armed = false;
        Ok((run.file.status(), run.file.board_path.clone()))
    }
    async fn rollback_startup(
        &self,
        core: &SessionEngine,
        run: &mut Run,
        admission: &mut AdmissionRelease,
        rollback: StartupRollback,
    ) {
        let StartupRollback {
            parent,
            previous_conductor,
            detail,
            now,
        } = rollback;
        let id = OrchestrationId(run.file.orchestration_id.clone());
        match core.restore_conductor_if_matches(
            parent,
            &id,
            &run.file.board_path,
            previous_conductor.0,
            previous_conductor.1,
        ) {
            Ok(effects) => {
                if let Err(error) = self.effects(effects).await {
                    self.warn("relay startup conductor rollback", &error);
                }
            }
            Err(error) => self.warn("relay startup conductor rollback", &error),
        }
        self.deps.orchestration.release_relay_board_if_matches(
            parent,
            &id,
            Path::new(&run.file.board_path),
        );
        self.finish(run, "stopped", "startup_failed", &detail, now)
            .await;
        admission.armed = false;
    }
    pub async fn stop_relay(
        &self,
        parent: LiveSessionId,
        id: &str,
        now: Timestamp,
    ) -> Result<proto::RelayStatus, RelayError> {
        let handle = self.resolve(parent, id, false)?;
        let mut run = handle.lock().await;
        if run.file.terminal() {
            return Err(RelayError::new(
                409,
                "relay_not_running",
                "relay is not running",
            ));
        }
        self.finish(&mut run, "stopped", "user_stop", "", now).await;
        Ok(run.file.status())
    }
    fn board(&self, run: &Run, text: &str, now: Timestamp) {
        if let Err(error) = self.boards.append(
            Path::new(&run.file.board_path),
            "hub",
            &format!("{text}\n"),
            now,
        ) {
            self.warn("relay board append failed", error);
        }
    }
    fn publish_meta(&self, run: &Run) {
        let mut state = lock(&self.state);
        state.meta.insert(
            run.file.orchestration_id.clone(),
            Meta {
                sequence: run.sequence,
                parent: run.file.parent_session_id,
                attached: run.parent_attached,
                restoring: !run.parent_attached || run.awaiting_reconnect,
                parent_identity: (
                    run.file.parent_started_at.clone(),
                    run.file.parent_cwd.clone(),
                    run.file.parent_provider.clone(),
                ),
                child_labels: ROLES
                    .iter()
                    .filter(|role| {
                        run.file.child_id(role) != 0 && !run.reconnected.contains(**role)
                    })
                    .map(|role| ((*role).to_owned(), run.file.child_label(role).to_owned()))
                    .filter(|(_, label)| !label.is_empty())
                    .collect(),
                board_path: run.file.board_path.clone(),
                item: RelayItem {
                    relay: run.file.status(),
                    events: run.file.events.clone(),
                },
            },
        );
    }
    async fn save(&self, run: &Run) {
        self.publish_meta(run);
        if let Err(error) = self.store.save(&run.file) {
            self.warn("relay save failed", error)
        }
        if !run.parent_attached {
            return;
        }
        let Ok(core) = self.core() else { return };
        let Some(parent) = core.details(LiveSessionId(run.file.parent_session_id)) else {
            return;
        };
        let effects = {
            let state = lock(&self.state);
            let mut statuses = state
                .meta
                .values()
                .filter(|m| m.attached && m.parent == run.file.parent_session_id)
                .collect::<Vec<_>>();
            statuses.sort_by_key(|m| m.sequence);
            core.apply_observation(
                parent.binding,
                SessionObservation::Relays(
                    statuses.into_iter().map(|m| m.item.relay.clone()).collect(),
                ),
                Timestamp::now(),
            )
        };
        match effects {
            Ok(effects) => {
                if let Err(error) = self.effects(effects).await {
                    self.warn("relay session publication", error)
                }
            }
            Err(error) => self.warn("relay session snapshot", error),
        }
    }
    async fn transition(&self, run: &mut Run, state: &str, reason: &str, now: Timestamp) {
        run.file.state = state.into();
        run.file.reason = reason.into();
        run.file.updated_at = time::format_rfc3339(now).unwrap_or_default();
        self.board(run,&format!("relay c={} round={} state={state} reason={reason} impl=#{} strong=#{} active={} review=#{} review_file={}",run.file.current_c(),run.file.round,run.file.implementation_session_id,run.file.strong_session_id,run.file.active_implementer,run.file.review_session_id,run.file.review_path),now);
        self.save(run).await;
    }
    async fn finish(&self, run: &mut Run, state: &str, reason: &str, detail: &str, now: Timestamp) {
        run.file.state = state.into();
        run.file.reason = if reason == "blocked" && !detail.is_empty() {
            format!("blocked: {detail}")
        } else {
            reason.into()
        };
        if !run.admission.0.is_empty() {
            if let Ok(core) = self.core() {
                core.release_children(&run.admission);
            }
            run.admission = AdmissionId::default();
        }
        run.file.updated_at = time::format_rfc3339(now).unwrap_or_default();
        run.file.event(
            if state == "completed" {
                "completed"
            } else {
                "stopped"
            },
            if detail.is_empty() {
                run.file.reason.clone()
            } else {
                detail.into()
            },
            now,
        );
        self.board(
            run,
            &format!(
                "relay {state} reason={} c={} round={} review_file={} branch={} {detail}",
                run.file.reason,
                run.file.completed_cs,
                run.file.round,
                run.file.review_path,
                run.file.branch
            ),
            now,
        );
        self.save(run).await;
        if run.parent_attached {
            let mut notice = format!(
                "\n{} relay {state}: plan={} c={} round={} reason={} review={} branch={}\n",
                run.file.prompts().tag(),
                run.file.plan_path,
                run.file.completed_cs,
                run.file.round,
                run.file.reason,
                run.file.review_path,
                run.file.branch
            );
            if state == "completed" && !run.file.branch.trim().is_empty() {
                notice.push_str(&format!(
                    "Merge when ready (not run automatically):\n  git merge {}\n",
                    run.file.branch
                ));
            }
            if let Err(error) = self
                .deps
                .orchestration
                .notify_relay_parent(
                    LiveSessionId(run.file.parent_session_id),
                    &OrchestrationId(run.file.orchestration_id.clone()),
                    notice,
                )
                .await
            {
                self.warn("relay parent notice", error)
            }
        }
        // The dedicated relay publisher is integrated in SessionEngine, so this
        // completion cannot consume the parent's normal Git-turn capture gate.
        if let Ok(core) = self.core() {
            let summary = proto::DoneSummary {
                session_id: run.file.parent_session_id,
                provider: run.file.parent_provider.clone(),
                title: format!("relay {state}"),
                text: format!(
                    "{}: {state} (c={}, round={}{}){}",
                    Path::new(&run.file.plan_path)
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy(),
                    run.file.completed_cs,
                    run.file.round,
                    if run.file.reason.is_empty() {
                        String::new()
                    } else {
                        format!(", reason={}", run.file.reason)
                    },
                    if run.file.branch.is_empty() {
                        String::new()
                    } else {
                        format!(" branch={}", run.file.branch)
                    }
                ),
                kind: "relay".into(),
                at: run.file.updated_at.clone(),
                fallback: false,
            };
            if let Err(error) = self.effects(core.publish_relay_done(summary)).await {
                self.warn("relay completion publication", error)
            }
        }
    }
    async fn spawn(
        &self,
        run: &mut Run,
        role: &str,
        prompt: String,
        replace: Option<LiveSessionId>,
        cancel: &TaskCancellation,
        now: Timestamp,
    ) -> Result<(), RelayError> {
        let mut transaction = TransitionGuard {
            owner: self,
            run,
            armed: true,
        };
        let result = self
            .spawn_inner(transaction.run, role, prompt, replace, cancel, now)
            .await;
        if cancel.token().is_cancelled() {
            transaction.armed = false;
            return Err(SessionError::Cancelled.into());
        }
        transaction.armed = false;
        result
    }
    async fn spawn_inner(
        &self,
        run: &mut Run,
        role: &str,
        prompt: String,
        replace: Option<LiveSessionId>,
        cancel: &TaskCancellation,
        now: Timestamp,
    ) -> Result<(), RelayError> {
        if !run.parent_attached {
            return Err(RelayError::new(
                409,
                "relay_parent_detached",
                "relay parent has not reattached",
            ));
        }
        let core = self.core()?;
        let parent = core
            .details(LiveSessionId(run.file.parent_session_id))
            .ok_or_else(RelayError::parent)?;
        let role_settings = run.file.roles.get(role).cloned().unwrap_or_default();
        let extra_admission = if replace.is_none()
            && (role == STRONG
                || !core.admission_matches(&run.admission, parent.binding.session, 1))
        {
            Some(self.reserve(parent.binding.session, 1)?)
        } else {
            None
        };
        let admission = extra_admission
            .as_ref()
            .map(|a| a.id.clone())
            .unwrap_or_else(|| run.admission.clone());
        let body = ResolvedChildSpawn::from_request(
            ChildSpawnRequest {
                role: role.into(),
                provider: role_settings.provider.clone(),
                model: role_settings.model,
                initial_prompt: prompt,
                cwd: run.file.child_cwd.clone(),
                force: true,
                subscription_profile_id: role_settings.subscription,
                effort: role_settings.effort,
                execution_mode: role_settings.execution_mode,
                permission_preset: role_settings.permission_preset,
                ..Default::default()
            },
            None,
            InternalSpawnGrants::default(),
        )?;
        let preparation = ChildPreparation {
            orchestration: OrchestrationId(run.file.orchestration_id.clone()),
            board_path: PathBuf::from(&run.file.board_path),
            absolute_cwd: PathBuf::from(&run.file.child_cwd),
            child_cwd: PathBuf::from(&run.file.child_cwd),
            branch: String::new(),
        };
        let result = self
            .deps
            .executor
            .launch_relay(
                ConfirmedChildRequest {
                    original_body: body.clone(),
                    body,
                    parent: parent.binding,
                    requested_provider: role_settings.provider,
                    admission,
                },
                preparation,
                replace,
                &mut |binding, label| {
                    run.file
                        .set_child(role, binding.session.0, label.to_owned(), 0);
                    run.file.set_baseline(role, 0);
                    run.nudged.remove(&binding.session.0);
                    run.timers.insert(
                        binding.session.0,
                        relay::ChildTimer {
                            assigned_at: Some(now),
                            last_write_at: Some(now),
                            ..Default::default()
                        },
                    );
                    if let Err(error) = self.store.save(&run.file) {
                        self.warn("relay registered child save", &error);
                        // Keep the immutable launch identity in memory even
                        // when its checkpoint fails. Child-launch rollback may
                        // be refused or race a reattach; cleanup must still be
                        // able to find the live process by this label.
                        self.publish_meta(run);
                        return Err(SessionError::Transport(error.to_string()));
                    }
                    self.publish_meta(run);
                    Ok(())
                },
                cancel.clone(),
            )
            .await?;
        self.publish_meta(run);
        self.board(
            run,
            &format!(
                "relay spawned role={role} session={} cwd={}",
                result.id.0, run.file.child_cwd
            ),
            now,
        );
        Ok(())
    }
    async fn inject(
        &self,
        run: &mut Run,
        role: &str,
        text: String,
        cancel: &TaskCancellation,
        now: Timestamp,
    ) -> Result<(), RelayError> {
        let id = run.file.child_id(role);
        if id == 0 {
            return Err(RelayError::new(
                500,
                "spawn_error",
                "relay has no child for that role",
            ));
        }
        let progress = self
            .store
            .read_progress(&run.file, role)
            .map(|r| r.0)
            .unwrap_or_default();
        let baseline =
            relay::text::count_done_lines(&progress, role, run.file.progress_id(role)).count;
        run.file.set_baseline(role, baseline);
        run.nudged.remove(&id);
        let safe = crate::orchestration::child_launch::prompt::sanitize_inject_text(&text);
        self.board(run, &format!("@{role} session={id} への指示:\n{safe}"), now);
        let timer = run.timers.entry(id).or_default();
        timer.assigned_at = Some(now);
        timer.last_write_at = Some(now);
        timer.exit_handled = false;
        self.deliver(LiveSessionId(id), format!("\n{safe}\n"), cancel, now)
            .await
    }
    async fn deliver(
        &self,
        id: LiveSessionId,
        text: String,
        cancel: &TaskCancellation,
        now: Timestamp,
    ) -> Result<(), RelayError> {
        let core = self.core()?;
        let details = core.details(id).ok_or(SessionError::NotFound(id))?;
        let framed = format!(
            "\x1b[200~{}\x1b[201~\r",
            crate::orchestration::child_launch::prompt::sanitize_inject_text(&text)
                .trim_end_matches(['\r', '\n'])
        );
        let receipt = core
            .submit(
                details.binding,
                InputRequest {
                    bytes: framed.into_bytes(),
                    authority: InputAuthority::Internal,
                },
                now,
                cancel,
            )
            .await;
        match receipt.disposition {
            InputDisposition::TransportWritten { .. } | InputDisposition::Deferred { .. } => Ok(()),
            other => Err(RelayError::new(
                500,
                "spawn_error",
                format!("relay instruction delivery failed: {other:?}"),
            )),
        }
    }
    async fn dispatch(
        &self,
        run: &mut Run,
        role: &str,
        text: String,
        cancel: &TaskCancellation,
        now: Timestamp,
    ) -> Result<(), RelayError> {
        if !run.file.headless(role) {
            return self.inject(run, role, text, cancel, now).await;
        }
        let previous = run.file.child_id(role);
        let prompt = run.file.prompts().headless_instruction(&text);
        self.board(run,&format!("@{role} への指示（headless: 新しいプロセスを立てる。前の子 #{previous} は終了済み）:\n{text}"),now);
        let replace = (previous != 0 && self.core()?.details(LiveSessionId(previous)).is_some())
            .then_some(LiveSessionId(previous));
        self.spawn(run, role, prompt, replace, cancel, now).await
    }
}
fn failure_reason(error: &RelayError) -> &'static str {
    if error.status == 503 {
        "hub_restart"
    } else {
        "spawn_error"
    }
}
pub fn positive_range(value: i64, default: i64, max: i64, name: &str) -> Result<i64, RelayError> {
    if value == 0 {
        Ok(default)
    } else if value < 1 || value > max {
        Err(RelayError::bad(format!(
            "{name} must be between 1 and {max}"
        )))
    } else {
        Ok(value)
    }
}
pub fn requires_bypass_ack(
    cfg: &config::OrchestrationConfig,
    roles: &BTreeMap<String, Role>,
) -> bool {
    cfg.child_full_bypass_enabled()
        && cfg.child_permission_default_tier() == config::PERMISSION_PRESET_FULL
        && (roles.is_empty()
            || roles
                .values()
                .any(|r| r.permission_preset.trim().is_empty()))
}
fn validate_roles(
    mut roles: BTreeMap<String, Role>,
    cfg: &config::Config,
) -> Result<BTreeMap<String, Role>, RelayError> {
    roles.retain(|role, _| ROLES.contains(&role.as_str()));
    for role in [IMPLEMENTATION, REVIEW, STRONG] {
        if role == STRONG && !roles.contains_key(role) {
            continue;
        }
        let Some(assignment) = roles.get_mut(role) else {
            return Err(RelayError::new(
                400,
                "relay_roles_missing",
                format!("missing role: {role}"),
            ));
        };
        assignment.provider = assignment.provider.trim().into();
        assignment.model = assignment.model.trim().into();
        if !crate::orchestration::child_options::ORCHESTRATION_PROVIDERS
            .contains(&assignment.provider.as_str())
            || assignment.model.starts_with('-')
            || !crate::profile::validation::valid_spawn_model_label(&assignment.model)
        {
            return Err(RelayError::new(
                400,
                "relay_roles_missing",
                format!("missing role: {role}"),
            ));
        }
        let mut mode = config::normalize_execution_mode(&assignment.execution_mode);
        if mode.is_empty() {
            mode = cfg.orchestration.relay_execution_mode_default()
        }
        assignment.execution_mode = config::resolve_execution_mode(
            &mode,
            config::headless_def_for(&assignment.provider, Some(cfg)).is_some(),
            config::LAUNCH_ORIGIN_CONDUCTOR,
            true,
        )
        .map_err(|e| {
            RelayError::new(
                400,
                "relay_execution_mode",
                format!("role {role:?} ({}): {e}", assignment.provider),
            )
        })?;
    }
    Ok(roles)
}
