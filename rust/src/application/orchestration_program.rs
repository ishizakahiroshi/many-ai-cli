//! Real board/role/restart ownership for the fixed Go child launch executor.
//! Session identities and input admission remain exclusively in SessionEngine.
mod lifecycle;
mod progress;
#[cfg(test)]
mod tests;
use super::{
    event_observer::EventWarning, session_workers::SessionWorkers, spawn_policy::RegistrySnapshot,
};
use crate::{
    config::{ConfigStore, Resource, RuntimePaths},
    hub::task_owner::HubTaskHandle,
    orchestration::{
        child_launch::{self, ChildLaunchServices, RegisteredChild, board::BoardStore},
        initial_prompt::InitialPromptOutcome,
    },
    proto::{core::*, time::Timestamp},
    terminal::session::{SessionEngine, board_input::BoardEventWrite},
};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Weak},
    time::Duration,
};
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
#[derive(Clone, Default)]
pub struct RoleSettings {
    pub provider: String,
    pub model: String,
    pub subscription: String,
    pub effort: String,
    pub execution_mode: String,
    pub permission_preset: String,
}
struct Notice {
    id: u64,
    parent: LiveSessionId,
    text: String,
    remainder: Vec<u8>,
}
struct Board {
    path: PathBuf,
    sessions: BTreeMap<LiveSessionId, String>,
    children: BTreeMap<LiveSessionId, Child>,
    events: VecDeque<Notice>,
    overflow: BTreeMap<LiveSessionId, u64>,
    next_event: u64,
    pending: BTreeMap<LiveSessionId, String>,
    stamp: Option<(u64, std::time::SystemTime)>,
    cursor: usize,
    writer: progress::Writer,
}
impl Board {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            sessions: BTreeMap::new(),
            children: BTreeMap::new(),
            events: VecDeque::new(),
            overflow: BTreeMap::new(),
            next_event: 0,
            pending: BTreeMap::new(),
            stamp: None,
            cursor: 0,
            writer: Default::default(),
        }
    }
    fn conductor(&self) -> Option<LiveSessionId> {
        let mut ids = self
            .sessions
            .iter()
            .filter_map(|(id, role)| (role == "conductor").then_some(*id));
        let first = ids.next()?;
        ids.next().is_none().then_some(first)
    }
}
struct Child {
    registration: RegisteredChild,
    stamp: Option<(u64, std::time::SystemTime)>,
    cursor: usize,
    last_board_write: Timestamp,
    done: bool,
    standby_since: Option<Timestamp>,
    idle_warned: bool,
    timed_out: bool,
    startup_failed: bool,
    startup_wait_notified: bool,
    retries: i64,
}
#[derive(Default)]
struct State {
    relay_boards: BTreeSet<OrchestrationId>,
    boards: BTreeMap<OrchestrationId, Board>,
    roles: BTreeMap<OrchestrationId, BTreeMap<String, RoleSettings>>,
}
pub struct OrchestrationDependencies {
    pub core: Weak<SessionEngine>,
    pub effects: Weak<dyn CoreEffectSink>,
    pub config: Arc<ConfigStore>,
    pub paths: RuntimePaths,
    pub registry: RegistrySnapshot,
    pub environment: Vec<String>,
    pub hub_cwd: PathBuf,
    pub workers: Arc<SessionWorkers>,
    pub tasks: HubTaskHandle,
    pub warning: EventWarning,
}
pub struct OrchestrationProgram {
    deps: OrchestrationDependencies,
    state: Mutex<State>,
    boards: Arc<BoardStore>,
    flush: tokio::sync::Mutex<()>,
    started: std::sync::atomic::AtomicBool,
    this: Weak<OrchestrationProgram>,
}
pub struct OrchestrationGuard {
    cancel: HubShutdownCancellation,
    join: Option<tokio::task::JoinHandle<()>>,
}
impl Drop for OrchestrationGuard {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(join) = self.join.take() {
            join.abort();
        }
    }
}
impl OrchestrationGuard {
    pub async fn stop_and_join(mut self) {
        self.cancel.cancel();
        if let Some(mut join) = self.join.take()
            && tokio::time::timeout(Duration::from_secs(2), &mut join)
                .await
                .is_err()
        {
            join.abort();
            let _ = join.await;
        }
    }
}
impl OrchestrationProgram {
    /// Relay children retain ordinary prompt/restart metadata but their DONE,
    /// idle and timeout transitions belong solely to the relay state machine.
    pub fn board_store(&self) -> Arc<BoardStore> {
        self.boards.clone()
    }
    pub fn claim_relay_board(&self, id: &OrchestrationId) {
        lock(&self.state).relay_boards.insert(id.clone());
    }
    pub fn rebind_relay_parent(
        &self,
        id: &OrchestrationId,
        old: LiveSessionId,
        new: LiveSessionId,
    ) {
        let mut state = lock(&self.state);
        if let Some(board) = state.boards.get_mut(id) {
            board.sessions.remove(&old);
            board.sessions.insert(new, "conductor".into());
            for child in board.children.values_mut() {
                if child.registration.parent.session == old {
                    child.registration.parent.session = new;
                }
            }
        }
    }
    pub fn record_relay_progress(
        &self,
        id: &OrchestrationId,
        child: LiveSessionId,
        stamp: (u64, std::time::SystemTime),
        now: Timestamp,
    ) {
        let mut state = lock(&self.state);
        if let Some(child) = state
            .boards
            .get_mut(id)
            .and_then(|board| board.children.get_mut(&child))
        {
            child.stamp = Some(stamp);
            child.last_board_write = now;
        }
    }
    pub fn relay_launch_startup_wait(
        &self,
        session: LiveSessionId,
        now: Timestamp,
    ) -> Result<Option<String>, SessionError> {
        let spawned = {
            let state = lock(&self.state);
            state
                .boards
                .values()
                .find_map(|board| board.children.get(&session))
                .filter(|child| {
                    child.registration.prompt_via_launch_arg && child.stamp.is_none() && !child.done
                })
                .map(|child| child.registration.spawned_at)
        };
        let Some(spawned) = spawned else {
            return Ok(None);
        };
        let core = self.core()?;
        let Some(details) = core.details(session) else {
            return Ok(None);
        };
        if matches!(
            core.initial_prompt_outcome(details.binding)?,
            Some((InitialPromptOutcome::Delivered { .. }, _))
        ) {
            return Ok(None);
        }
        let (details, screen) = match core.initial_prompt_observation(details.binding) {
            Ok(view) => view,
            Err(SessionError::StaleBinding) => return Ok(None),
            Err(error) => return Err(error),
        };
        Ok(
            crate::orchestration::initial_prompt::launch_arg_startup_screen(
                &details.snapshot.provider,
                &screen,
                spawned,
                details.last_output_at,
                now,
            )
            .map(str::to_owned),
        )
    }
    pub async fn notify_relay_parent(
        &self,
        parent: LiveSessionId,
        id: &OrchestrationId,
        text: String,
    ) -> Result<(), SessionError> {
        let core = self.core()?;
        let Some(details) = core.details(parent) else {
            return Ok(());
        };
        if !details.snapshot.orchestration_id.0.is_empty()
            && details.snapshot.parent_session_id.0 == 0
        {
            return self.progress_notice(parent, id, text).await;
        }
        if core.initial_gate_pending(details.binding, Timestamp::now())? {
            return Ok(());
        }
        self.inject(parent, sanitize_notice(&text), TaskCancellation::default())
            .await
    }
    pub fn relay_owns(&self, id: &OrchestrationId) -> bool {
        lock(&self.state).relay_boards.contains(id)
    }
    pub fn launch_instruction_not_taken(
        &self,
        session: LiveSessionId,
    ) -> Result<bool, SessionError> {
        let pending = {
            let state = lock(&self.state);
            state
                .boards
                .values()
                .find_map(|board| {
                    board.children.get(&session).map(|child| {
                        child.registration.prompt_via_launch_arg
                            && child.stamp.is_none()
                            && !child.done
                    })
                })
                .unwrap_or(false)
        };
        if !pending {
            return Ok(false);
        }
        let core = self.core()?;
        let Some(details) = core.details(session) else {
            return Ok(false);
        };
        if matches!(
            core.initial_prompt_outcome(details.binding)?,
            Some((InitialPromptOutcome::Delivered { .. }, _))
        ) {
            return Ok(false);
        }
        let (details, lines) = match core.initial_prompt_observation(details.binding) {
            Ok(view) => view,
            Err(SessionError::StaleBinding) => return Ok(false),
            Err(error) => return Err(error),
        };
        let screen: String = lines
            .concat()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        let signals: &[&str] = match details.snapshot.provider.as_str() {
            "codex" => &["AskCodextodoanything"],
            "claude" => &["shift+tabtocycle"],
            _ => &[],
        };
        Ok(!signals.iter().any(|signal| screen.contains(signal)))
    }
    pub fn append_conductor_instruction(
        &self,
        path: &Path,
        child: &SessionSnapshot,
        text: &str,
        now: Timestamp,
    ) {
        if let Err(error) = self.boards.append(
            path,
            "conductor",
            &format!("@{} session={} への指示:\n{text}\n", child.role, child.id.0),
            now,
        ) {
            self.warning(
                "send-child board append failed",
                &SessionError::Transport(error.to_string()),
            );
        }
    }
    pub fn new(deps: OrchestrationDependencies) -> Result<Arc<Self>, SessionError> {
        if !deps.hub_cwd.is_absolute() {
            return Err(SessionError::InvalidRequest(
                "orchestration cwd must be absolute".into(),
            ));
        }
        crate::profile::subscriptions::check_path(&deps.paths, &deps.hub_cwd)
            .map_err(|_| SessionError::InvalidRequest("orchestration cwd outside trial".into()))?;
        Ok(Arc::new_cyclic(|this| Self {
            boards: Arc::new(BoardStore::new(&deps.paths)),
            deps,
            state: Mutex::new(State::default()),
            flush: tokio::sync::Mutex::new(()),
            started: false.into(),
            this: this.clone(),
        }))
    }
    fn core(&self) -> Result<Arc<SessionEngine>, SessionError> {
        self.deps.core.upgrade().ok_or(SessionError::Shutdown)
    }
    fn config(&self) -> Result<crate::config::Config, SessionError> {
        self.deps
            .config
            .snapshot()
            .map(|s| s.config)
            .map_err(|_| SessionError::Transport("orchestration configuration unavailable".into()))
    }
    pub fn reserve_conductor(
        &self,
        label: &mut String,
        roles: Option<&BTreeMap<String, Option<RoleSettings>>>,
        now: Timestamp,
    ) -> Result<OrchestrationId, SessionError> {
        let mut assigned = BTreeMap::new();
        for (role, settings) in roles.into_iter().flatten() {
            if let Some(settings) = settings {
                if !settings.provider.is_empty()
                    && !crate::orchestration::child_options::ORCHESTRATION_PROVIDERS
                        .contains(&settings.provider.as_str())
                {
                    return Err(SessionError::InvalidRequest(
                        "invalid orchestration role provider".into(),
                    ));
                }
                if settings.model.starts_with('-')
                    || !crate::profile::validation::valid_spawn_model_label(&settings.model)
                {
                    return Err(SessionError::InvalidRequest(
                        "invalid orchestration role model".into(),
                    ));
                }
                assigned.insert(
                    role.clone(),
                    RoleSettings {
                        provider: settings.provider.clone(),
                        model: settings.model.clone(),
                        subscription: settings.subscription.clone(),
                        effort: settings.effort.clone(),
                        execution_mode: settings.execution_mode.clone(),
                        permission_preset: settings.permission_preset.clone(),
                    },
                );
            }
        }
        if label.is_empty() {
            *label = format!("orch-conductor-{}", now.unix_nanos());
        }
        let id = OrchestrationId(format!("o{}", now.unix_nanos()));
        if !assigned.is_empty() {
            lock(&self.state).roles.insert(id.clone(), assigned);
        }
        Ok(id)
    }
    /// Caller binds JSON origin to server-issued UI evidence before resolving.
    /// This performs Go's provider/model/memory lookup before disclosure; launch
    /// permission and execution-mode preparation remains child_options::prepare.
    pub fn resolve_child_request(
        &self,
        parent: &SessionSnapshot,
        body: &mut ChildSpawnRequest,
    ) -> Result<&'static str, SessionError> {
        body.role = child_launch::prompt::normalize_role(&body.role)
            .map_err(|error| SessionError::InvalidRequest(error.into()))?;
        let role = self.roles(&parent.orchestration_id).remove(&body.role);
        if body.model.is_empty()
            && let Some(role) = &role
        {
            body.model.clone_from(&role.model);
        }
        let cfg = self.config()?;
        let remembered = cfg
            .user_prefs
            .spawn
            .role_provider
            .get(&body.role)
            .map(String::as_str)
            .unwrap_or("");
        let mapped = role
            .as_ref()
            .map(|role| role.provider.as_str())
            .unwrap_or("");
        let valid = |provider: &str| {
            crate::orchestration::child_options::ORCHESTRATION_PROVIDERS.contains(&provider)
        };
        let (provider, source) = if !body.provider.trim().is_empty() {
            (body.provider.clone(), "explicit")
        } else if !mapped.trim().is_empty() {
            (mapped.to_owned(), "role_map")
        } else if valid(remembered) {
            (remembered.to_owned(), "remembered")
        } else if valid(&parent.provider) {
            (parent.provider.clone(), "parent")
        } else {
            ("codex".to_owned(), "default")
        };
        body.provider = provider;
        if body.effort.trim().is_empty()
            && let Some(effort) =
                cfg.user_prefs
                    .spawn
                    .role_effort
                    .get(&body.role)
                    .filter(|effort| {
                        !effort.is_empty()
                            && crate::config::validate_effort(&body.provider, effort).is_ok()
                    })
        {
            body.effort.clone_from(effort);
        }
        if body.claimed_origin.is_empty()
            && body.permission_preset.trim().is_empty()
            && let Some(permission) =
                cfg.user_prefs
                    .spawn
                    .role_permission
                    .get(&body.role)
                    .filter(|permission| {
                        !permission.is_empty()
                            && crate::config::validate_permission_preset(permission).is_ok()
                    })
        {
            body.permission_preset.clone_from(permission);
        }
        Ok(source)
    }
    pub fn roles(&self, id: &OrchestrationId) -> BTreeMap<String, RoleSettings> {
        lock(&self.state).roles.get(id).cloned().unwrap_or_default()
    }
    pub fn conductor_prompt(&self, id: &OrchestrationId) -> String {
        let base = SessionWorkers::conductor_prompt(&id.0);
        let roles = self.roles(id);
        if roles.is_empty() {
            return base;
        }
        let start = base
            .find("No child role mapping was configured.")
            .expect("fixed conductor guide");
        let end = base[start..]
            .find("Do not call the Hub HTTP API")
            .expect("fixed conductor guide")
            + start;
        let mut configured =
            String::from("Configured child roles (provider/model already decided by the user):\n");
        for (role, settings) in roles {
            configured.push_str(&format!(
                "- {role}: provider={} model={}\n",
                settings.provider, settings.model
            ));
        }
        configured.push_str("To spawn a child for a role above, run:\n  <MANY_AI_CLI_BIN> orchestrate spawn --role <role> \"<prompt>\"\n(provider/model are resolved automatically from the mapping above; pass --provider/--model to override a specific spawn.)\n");
        format!("{}{}{}", &base[..start], configured, &base[end..])
    }
    pub fn start(self: &Arc<Self>) -> Result<OrchestrationGuard, SessionError> {
        self.core()?;
        if self.started.swap(true, std::sync::atomic::Ordering::AcqRel) {
            return Err(SessionError::InvalidRequest(
                "orchestration worker already started".into(),
            ));
        }
        let cancel = HubShutdownCancellation::default();
        let task_cancel = cancel.clone();
        let weak = Arc::downgrade(self);
        let join = tokio::spawn(async move {
            let mut ticks = tokio::time::interval(Duration::from_secs(2));
            ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {biased;_=task_cancel.token().cancelled()=>break,_=ticks.tick()=>{let Some(owner)=weak.upgrade()else{break};let result=owner.poll().await;if let Err(error)=result{(owner.deps.warning)("orchestration poll",&error);}}}
            }
        });
        Ok(OrchestrationGuard {
            cancel,
            join: Some(join),
        })
    }
    async fn pending_flag(&self, parent: LiveSessionId, pending: bool) -> Result<(), SessionError> {
        let core = self.core()?;
        let Some(details) = core.details(parent) else {
            return Ok(());
        };
        if details.snapshot.board_notify_pending == pending {
            return Ok(());
        }
        let effects = core.apply_observation(
            details.binding,
            SessionObservation::BoardNotifyPending(pending),
            Timestamp::now(),
        )?;
        self.apply_effects(effects).await
    }
    pub async fn queue_event(
        &self,
        parent: LiveSessionId,
        id: &OrchestrationId,
        text: String,
    ) -> Result<(), SessionError> {
        if id.0.is_empty() || parent.0 <= 0 {
            return Ok(());
        }
        let text = sanitize_notice(&text);
        let path = {
            let state = lock(&self.state);
            let Some(board) = state.boards.get(id) else {
                return Err(SessionError::InvalidRequest(
                    "orchestration event has no registered board".into(),
                ));
            };
            board.path.clone()
        };
        if let Err(error) = self.boards.append(
            &path,
            "hub",
            &format!("event pending: {}", text.trim()),
            Timestamp::now(),
        ) {
            self.warning(
                "orchestration event board record",
                &SessionError::Transport(error.to_string()),
            );
        }
        {
            let mut state = lock(&self.state);
            let Some(board) = state.boards.get_mut(id) else {
                return Err(SessionError::StaleBinding);
            };
            if board.conductor() != Some(parent) {
                return Err(SessionError::StaleBinding);
            }
            if board.events.len() < 256 {
                board.next_event += 1;
                board.events.push_back(Notice {
                    id: board.next_event,
                    parent,
                    text,
                    remainder: Vec::new(),
                });
            } else {
                *board.overflow.entry(parent).or_default() += 1;
            }
        }
        self.pending_flag(parent, true).await
    }
    async fn progress_notice(
        &self,
        parent: LiveSessionId,
        id: &OrchestrationId,
        text: String,
    ) -> Result<(), SessionError> {
        let mode = self.config()?.orchestration.board_notify_mode;
        let text = sanitize_notice(&text);
        match mode.as_str() {
            "interrupt" => {
                self.pending_flag(parent, false).await?;
                self.inject(parent, text, TaskCancellation::default()).await
            }
            "queue-until-idle" => {
                if let Some(board) = lock(&self.state).boards.get_mut(id) {
                    board.pending.insert(parent, text);
                }
                self.pending_flag(parent, true).await
            }
            _ => self.pending_flag(parent, true).await,
        }
    }
    async fn inject(
        &self,
        parent: LiveSessionId,
        text: String,
        cancel: TaskCancellation,
    ) -> Result<(), SessionError> {
        let core = self.core()?;
        let details = core.details(parent).ok_or(SessionError::NotFound(parent))?;
        let receipt = core
            .submit(
                details.binding,
                InputRequest {
                    bytes: framed(&text),
                    authority: InputAuthority::Internal,
                },
                Timestamp::now(),
                &cancel,
            )
            .await;
        match receipt.disposition {
            InputDisposition::TransportWritten { .. } | InputDisposition::Deferred { .. } => Ok(()),
            _ => Err(SessionError::Transport(
                "orchestration injection failed".into(),
            )),
        }
    }
    async fn flush_events(&self) -> Result<(), SessionError> {
        let Ok(_guard) = self.flush.try_lock() else {
            return Ok(());
        };
        let ids = lock(&self.state).boards.keys().cloned().collect::<Vec<_>>();
        let core = self.core()?;
        for id in ids {
            let selected = {
                let mut state = lock(&self.state);
                let board = state.boards.get_mut(&id).expect("registered board");
                let overflows = board
                    .overflow
                    .iter()
                    .map(|(k, v)| (*k, *v))
                    .collect::<Vec<_>>();
                for (parent, count) in overflows {
                    if board.events.len() >= 256 {
                        break;
                    }
                    board.next_event += 1;
                    let text = format!(
                        "\n[orchestration] {count} additional events were retained on the board; read the event pending sections: {}\n",
                        board.path.display()
                    );
                    board.events.push_back(Notice {
                        id: board.next_event,
                        parent,
                        text,
                        remainder: Vec::new(),
                    });
                    board.overflow.remove(&parent);
                }
                board.events.front().map(|event| {
                    (
                        event.id,
                        event.parent,
                        if event.remainder.is_empty() {
                            framed(&event.text)
                        } else {
                            event.remainder.clone()
                        },
                    )
                })
            };
            if let Some((event_id, parent, bytes)) = selected {
                let Some(details) = core.details(parent) else {
                    continue;
                };
                let result = core
                    .deliver_board_event(details.binding, &id, bytes, &TaskCancellation::default())
                    .await;
                let pending = {
                    let mut state = lock(&self.state);
                    let board = state.boards.get_mut(&id).expect("registered board");
                    if board
                        .events
                        .front()
                        .is_none_or(|event| event.id != event_id)
                    {
                        continue;
                    }
                    match result {
                        BoardEventWrite::Written => {
                            board.events.pop_front();
                            board.pending.remove(&parent);
                        }
                        BoardEventWrite::Blocked => {}
                        BoardEventWrite::Failed {
                            remainder,
                            error: _,
                        } => {
                            board.events.front_mut().expect("event checked").remainder = remainder;
                        }
                    }
                    board.events.iter().any(|event| event.parent == parent)
                        || board.overflow.contains_key(&parent)
                        || board.pending.contains_key(&parent)
                };
                self.pending_flag(parent, pending).await?;
            }
            let notices = lock(&self.state)
                .boards
                .get(&id)
                .map(|b| {
                    b.pending
                        .iter()
                        .map(|(p, t)| (*p, t.clone()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            for (parent, text) in notices {
                let Some(details) = core.details(parent) else {
                    continue;
                };
                if details.snapshot.parent_session_id.0 != 0
                    || details.snapshot.orchestration_id != id
                    || !core.board_notice_output_idle(details.binding)?
                {
                    continue;
                }
                let ready = {
                    let mut state = lock(&self.state);
                    let board = state.boards.get_mut(&id).expect("registered board");
                    if board.pending.get(&parent) == Some(&text) {
                        board.pending.remove(&parent);
                        true
                    } else {
                        false
                    }
                };
                if ready {
                    self.pending_flag(parent, false).await?;
                    self.inject(parent, text, TaskCancellation::default())
                        .await?;
                }
            }
        }
        Ok(())
    }
    async fn poll(&self) -> Result<(), SessionError> {
        progress::poll(self).await?;
        lifecycle::poll(self, Timestamp::now()).await?;
        self.flush_events().await
    }
}
impl ChildLaunchServices for OrchestrationProgram {
    fn launch_arg_usable(&self, provider: &str) -> Result<bool, SessionError> {
        let registry = (self.deps.registry)()
            .map_err(|_| SessionError::Transport("provider registry unavailable".into()))?;
        let Some(definition) = registry.lookup(provider) else {
            return Err(SessionError::InvalidRequest(
                "provider not registered".into(),
            ));
        };
        let launch = definition.definition.launch.ok_or_else(|| {
            SessionError::InvalidRequest("provider launch definition unavailable".into())
        })?;
        let argv = std::iter::once(launch.executable)
            .chain(launch.args)
            .collect::<Vec<_>>();
        let fs = crate::process::execpath::NativeFs;
        let resolver = crate::process::execpath::Resolver::new(
            crate::process::execpath::Platform::native(),
            &self.deps.environment,
            &self.deps.hub_cwd,
            &fs,
        );
        let command = resolver.resolve_provider(
            provider,
            (!argv[0].is_empty()).then_some(argv.as_slice()),
            &[],
        );
        Ok(crate::wrapper::launch::launch_prompt_arg_usable(
            command.shell_shim.as_deref(),
            &self.deps.paths.resource(Resource::Temporary),
        )
        .is_ok())
    }
    fn role_subscription(
        &self,
        id: &OrchestrationId,
        role: &str,
    ) -> Result<Option<String>, SessionError> {
        Ok(self
            .roles(id)
            .get(role)
            .map(|settings| settings.subscription.clone())
            .filter(|value| !value.trim().is_empty()))
    }
    fn trust_grant_providers(&self) -> Result<Vec<String>, SessionError> {
        Ok(crate::orchestration::child_options::ORCHESTRATION_PROVIDERS
            .iter()
            .filter(|provider| matches!(**provider, "claude" | "codex"))
            .map(|provider| (*provider).to_owned())
            .collect())
    }
    fn apply_effects<'a>(
        &'a self,
        effects: CoreEffects,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            let sink = self.deps.effects.upgrade().ok_or(SessionError::Shutdown)?;
            sink.apply(effects).await.map_err(|failure| failure.error)
        })
    }
    fn register_conductor(
        &self,
        parent: SessionBinding,
        id: &OrchestrationId,
        path: &Path,
    ) -> Result<(), SessionError> {
        let details = self
            .core()?
            .details(parent.session)
            .ok_or(SessionError::NotFound(parent.session))?;
        if details.binding.incarnation != parent.incarnation
            || (details.snapshot.orchestration_id != *id && !self.relay_owns(id))
        {
            return Err(SessionError::StaleBinding);
        }
        crate::profile::subscriptions::check_path(&self.deps.paths, path)
            .map_err(|_| SessionError::InvalidRequest("board outside trial".into()))?;
        let mut state = lock(&self.state);
        let board = state
            .boards
            .entry(id.clone())
            .or_insert_with(|| Board::new(path.to_owned()));
        if board.path != path {
            return Err(SessionError::InvalidRequest(
                "orchestration board path mismatch".into(),
            ));
        }
        board.sessions.insert(parent.session, "conductor".into());
        Ok(())
    }
    fn child_registered<'a>(
        &'a self,
        child: RegisteredChild,
        cancel: TaskCancellation,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            if cancel.token().is_cancelled() {
                return Err(SessionError::Cancelled);
            }
            let core = self.core()?;
            let details = core
                .details(child.binding.session)
                .ok_or(SessionError::NotFound(child.binding.session))?;
            if details.binding != child.binding
                || details.snapshot.parent_session_id != child.parent.session
                || details.snapshot.orchestration_id != child.preparation.orchestration
            {
                return Err(SessionError::StaleBinding);
            }
            // Required initial instruction ownership is accepted only after the real
            // binding and restart context are installed; an enqueue failure rolls back.
            let id = child.preparation.orchestration.clone();
            let child_id = child.binding.session;
            {
                let mut state = lock(&self.state);
                let board = state
                    .boards
                    .get_mut(&id)
                    .ok_or(SessionError::StaleBinding)?;
                board.sessions.insert(child_id, child.role.clone());
                let spawned = child.spawned_at;
                board.children.insert(
                    child_id,
                    Child {
                        registration: child,
                        stamp: None,
                        cursor: 0,
                        last_board_write: spawned,
                        done: false,
                        standby_since: None,
                        idle_warned: false,
                        timed_out: false,
                        startup_failed: false,
                        startup_wait_notified: false,
                        retries: 0,
                    },
                );
            }
            let request = {
                let state = lock(&self.state);
                let child = &state
                    .boards
                    .get(&id)
                    .expect("registered board")
                    .children
                    .get(&child_id)
                    .expect("registered child")
                    .registration;
                crate::orchestration::initial_prompt::InitialPromptRequest::for_child(child)
            };
            if let Some(request) = request {
                self.deps.workers.enqueue_initial_request(request)?;
            }
            Ok(())
        })
    }
    fn notify_board_event<'a>(
        &'a self,
        parent: SessionBinding,
        id: &'a OrchestrationId,
        text: String,
        cancel: TaskCancellation,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            if cancel.token().is_cancelled() {
                return Err(SessionError::Cancelled);
            }
            self.queue_event(parent.session, id, text).await
        })
    }
    fn notify_orchestration_error<'a>(
        &'a self,
        parent: SessionBinding,
        limit: &'a str,
        detail: &'a str,
        cancel: TaskCancellation,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            let text = format!(
                "\n[MANY-AI-CLI-ORCHESTRATION-ERROR] limit={} detail={}\n",
                child_launch::safe_token(limit),
                detail.replace('\n', " ")
            );
            let core = self.core()?;
            let Some(details) = core.details(parent.session) else {
                return Ok(());
            };
            if details.binding.incarnation != parent.incarnation {
                return Err(SessionError::StaleBinding);
            }
            let registered = if details.snapshot.parent_session_id.0 == 0
                && !details.snapshot.orchestration_id.0.is_empty()
            {
                let state = lock(&self.state);
                let id = &details.snapshot.orchestration_id;
                state
                    .boards
                    .get(id)
                    .filter(|board| board.conductor() == Some(parent.session))
                    .map(|_| id.clone())
            } else {
                None
            };
            if let Some(id) = registered {
                if self.relay_owns(&id) {
                    return Ok(());
                }
                self.notify_board_event(parent, &id, text, cancel).await
            } else {
                self.inject(parent.session, text, cancel).await
            }
        })
    }
    fn confirmation_changed(
        &self,
        _parent: SessionBinding,
        _role: &str,
        provider_note: &str,
        option_note: &str,
    ) {
        let detail = format!("{} {}", provider_note, option_note);
        (self.deps.warning)(
            "orchestration launch confirmation changed",
            &SessionError::Transport(crate::storage::mask_secrets(&detail)),
        );
    }
    fn warning(&self, operation: &'static str, error: &SessionError) {
        (self.deps.warning)(operation, error)
    }
}
fn sanitize_notice(text: &str) -> String {
    let mut text = child_launch::prompt::sanitize_inject_text(text);
    let mut limit = text.len().min(4096);
    while !text.is_char_boundary(limit) {
        limit -= 1
    }
    text.truncate(limit);
    text
}
fn framed(text: &str) -> Vec<u8> {
    format!(
        "\x1b[200~{}\x1b[201~\r",
        text.trim_end_matches(['\r', '\n'])
    )
    .into_bytes()
}
