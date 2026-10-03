//! The single Hub session state owner. Registration explicitly initializes
//! optional persistence outside the state lock before publishing the new card.
//! All returned CoreEffects are unapplied. Only asynchronous input/actions call
//! the injected effect sink, and never while holding the state mutex.
use super::{
    events::CoreEventBus,
    input::InputState,
    journal::SessionJournal,
    replay::{self, ReplayBuffer},
    vt::VtBuffer,
};
use crate::proto::time::Timestamp;
use crate::{
    approval::{marker::TranscriptSource, record::ApprovalState},
    proto::{self, core::*},
    storage::mask_secrets,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

mod approvals;
mod authorization;
pub mod confirmations;
mod input;
mod lane;
mod lifecycle;
mod observations;
mod spawning;
mod ui;

pub const USER_TURN_MARKER: &[u8] = b"\x1b]47777;user-turn\x07";
pub type LiveApprovalPolicy =
    Arc<dyn Fn(&str, &str, &proto::ApprovalSummary) -> bool + Send + Sync>;
pub type EngineWarningHandler = Arc<dyn Fn(&str, &SessionError) + Send + Sync>;
pub struct EngineOptions {
    pub hub_instance: String,
    pub token_statusbar: bool,
    pub submit_timing: SubmitTiming,
    pub idle_after: Duration,
    pub approval_phrases: BTreeMap<String, Vec<String>>,
    pub custom_providers: BTreeSet<String>,
    /// Live policy lookup; absence means automatic approvals are disabled.
    pub approval_policy: Option<LiveApprovalPolicy>,
    /// Required for confirmation disclosure and actual child/refusal dispatch.
    /// Absence leaves the capability explicitly unavailable.
    pub confirmation_executor: Option<Arc<dyn confirmations::ConfirmationExecutor>>,
    /// Reports optional persistence degradation without copying user bodies.
    pub warning: EngineWarningHandler,
}
impl Default for EngineOptions {
    fn default() -> Self {
        Self {
            hub_instance: String::new(),
            token_statusbar: true,
            submit_timing: SubmitTiming::default(),
            idle_after: Duration::from_secs(3),
            approval_phrases: BTreeMap::new(),
            custom_providers: BTreeSet::new(),
            approval_policy: None,
            confirmation_executor: None,
            warning: Arc::new(|operation, error| eprintln!("session core {operation}: {error:?}")),
        }
    }
}
pub struct SessionEngine {
    options: EngineOptions,
    state: Mutex<State>,
    /// Serializes init/restore I/O without blocking output/input under state.
    registration: tokio::sync::Mutex<()>,
    journal: Arc<SessionJournal>,
    transport: Arc<dyn WrapperTransport>,
    effects: Arc<dyn CoreEffectSink>,
    spawner: Arc<dyn WrappedSessionSpawner>,
    events: CoreEventBus,
}
struct PendingSpawn {
    lease: ProviderSpawnLease,
    metadata: SpawnRegistrationMetadata,
}
struct State {
    next_id: i64,
    next_incarnation: u64,
    next_reservation: u64,
    auth_epoch: AuthEpoch,
    sessions: BTreeMap<LiveSessionId, Session>,
    dismissed: BTreeSet<LiveSessionId>,
    uis: BTreeMap<UiConnectionId, UiState>,
    retired_uis: BTreeMap<(UiConnectionId, AuthEpoch), Arc<authorization::UiWorkState>>,
    last_ui_size: TerminalSize,
    admission: AdmissionState,
    spawn_proofs: BTreeMap<String, PendingSpawn>,
    persistence_lane: Arc<lane::InputLane>,
    confirmations: BTreeMap<SpawnConfirmationId, ConfirmationEntry>,
    next_confirmation: u64,
    next_confirmation_decision: u64,
    confirmations_stopped: bool,
}
struct Session {
    binding: SessionBinding,
    snapshot: SessionSnapshot,
    connected: bool,
    pid: i64,
    db_id: Option<DbSessionId>,
    git_root: Option<PathBuf>,
    transcript: TranscriptSessionIdentity,
    transcript_offset: i64,
    approval: ApprovalState,
    marker_source: TranscriptSource,
    approval_reservation: Option<(ApprovalReservationId, ApprovalActionBinding)>,
    workflow: Option<proto::WorkflowProgress>,
    subagents: Option<proto::SubagentTree>,
    done: Option<proto::DoneSummary>,
    replay: ReplayBuffer,
    replay_epoch: ReplayEpoch,
    pty_bytes_seen: i64,
    vt: VtBuffer,
    size: TerminalSize,
    resize_debounce: Option<Timestamp>,
    controlling_ui: Option<UiConnectionId>,
    input: InputState,
    input_lane: Arc<lane::InputLane>,
    awaiting_submit_enter: bool,
    last_output: Option<Timestamp>,
    output_generation: u64,
    usage_probe: bool,
    custom_provider: bool,
    native_tail: String,
    ended: Option<SessionEnd>,
}
struct UiState {
    binding: UiBinding,
    active: Option<LiveSessionId>,
    sizes: BTreeMap<LiveSessionId, TerminalSize>,
    priming: bool,
    queued: Vec<UiFrame>,
    work: Arc<authorization::UiWorkState>,
}
enum UiFrame {
    Message(Box<proto::Message>),
    GitTurn(GitTurnNotification),
}
struct ConfirmationEntry {
    pending: PendingSpawnConfirmation,
    completion: Arc<confirmations::Completion>,
    decision: Option<u64>,
}
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}
fn timestamp(now: Timestamp) -> Result<String, SessionError> {
    proto::time::format_rfc3339(now).map_err(|_| {
        SessionError::InvalidRequest("timestamp is outside the supported range".into())
    })
}
fn terminal(state: &str) -> bool {
    matches!(
        state,
        "completed" | "error" | "disconnected" | "done" | "timeout" | "dismissed"
    )
}
fn history(
    id: LiveSessionId,
    now: Timestamp,
    kind: &str,
    mut values: JsonObject,
) -> Result<CoreEffect, SessionError> {
    values.insert("ts".into(), timestamp(now)?.into());
    values.insert("type".into(), kind.into());
    values.insert("session_id".into(), id.0.into());
    Ok(CoreEffect::Persist(PersistenceEffect::Event {
        session: id,
        event: HistoryEvent(values),
    }))
}
fn object(value: serde_json::Value) -> JsonObject {
    value.as_object().cloned().expect("internal history object")
}
impl SessionEngine {
    pub fn new(
        options: EngineOptions,
        journal: Arc<SessionJournal>,
        transport: Arc<dyn WrapperTransport>,
        effects: Arc<dyn CoreEffectSink>,
        spawner: Arc<dyn WrappedSessionSpawner>,
        events: CoreEventBus,
    ) -> Self {
        Self {
            options,
            state: Mutex::new(State {
                next_id: 0,
                next_incarnation: 0,
                next_reservation: 0,
                auth_epoch: AuthEpoch(0),
                sessions: BTreeMap::new(),
                dismissed: BTreeSet::new(),
                uis: BTreeMap::new(),
                retired_uis: BTreeMap::new(),
                last_ui_size: TerminalSize::default(),
                admission: AdmissionState::default(),
                spawn_proofs: BTreeMap::new(),
                persistence_lane: Arc::new(lane::InputLane::default()),
                confirmations: BTreeMap::new(),
                next_confirmation: 0,
                next_confirmation_decision: 0,
                confirmations_stopped: false,
            }),
            registration: tokio::sync::Mutex::new(()),
            journal,
            transport,
            effects,
            spawner,
            events,
        }
    }
    pub fn journal(&self) -> &Arc<SessionJournal> {
        &self.journal
    }
    pub fn snapshot(&self, id: LiveSessionId) -> Option<SessionSnapshot> {
        lock(&self.state)
            .sessions
            .get(&id)
            .map(|s| s.snapshot.clone())
    }
    pub fn snapshots(&self) -> Vec<SessionSnapshot> {
        lock(&self.state)
            .sessions
            .values()
            .filter(|s| !s.usage_probe)
            .map(|s| s.snapshot.clone())
            .collect()
    }
    pub fn details(&self, id: LiveSessionId) -> Option<SessionDetails> {
        lock(&self.state).sessions.get(&id).map(|s| SessionDetails {
            binding: s.binding,
            last_output_at: s.last_output,
            snapshot: s.snapshot.clone(),
            db_id: s.db_id,
            git_root: s.git_root.clone(),
            transcript: s.transcript.clone(),
            approval: s.approval.snapshot(),
            workflow: s.workflow.clone(),
            subagents: s.subagents.clone(),
            done: s.done.clone(),
            connected: s.connected,
        })
    }
    pub fn active_ids(&self) -> Vec<LiveSessionId> {
        lock(&self.state)
            .sessions
            .values()
            .filter(|s| s.connected)
            .map(|s| s.binding.session)
            .collect()
    }
    pub fn is_current(&self, binding: SessionBinding) -> bool {
        lock(&self.state)
            .sessions
            .get(&binding.session)
            .is_some_and(|s| s.binding == binding && s.connected)
    }
    pub fn auth_epoch(&self) -> AuthEpoch {
        lock(&self.state).auth_epoch
    }
    pub fn subscribe(&self) -> Box<dyn CoreEventSubscription> {
        self.events.subscribe()
    }
    fn warn(&self, operation: &str, error: &SessionError) {
        (self.options.warning)(operation, error);
    }
    async fn apply_effects(&self, effects: CoreEffects) -> Result<(), SessionError> {
        self.effects
            .apply(effects)
            .await
            .map_err(|failure| failure.error)
    }
}
impl State {
    fn session(&mut self, binding: SessionBinding) -> Result<&mut Session, SessionError> {
        let s = self
            .sessions
            .get_mut(&binding.session)
            .ok_or(SessionError::NotFound(binding.session))?;
        if s.binding != binding {
            return Err(SessionError::StaleBinding);
        }
        Ok(s)
    }
    fn authorized(&self, ui: UiBinding) -> bool {
        self.auth_epoch == ui.auth_epoch
            && self
                .uis
                .get(&ui.connection)
                .is_some_and(|s| s.binding == ui)
    }
    /// Expand broadcast effects while holding the exact state snapshot lock.
    /// Priming is unbounded like the Go baseline; do not invent a disconnect cap.
    fn route(&mut self, effects: CoreEffects) -> CoreEffects {
        let mut out = Vec::new();
        let ended: BTreeMap<_, _> = effects
            .0
            .iter()
            .filter_map(|effect| match effect {
                CoreEffect::Persist(PersistenceEffect::EndSession { binding, .. }) => {
                    Some((binding.session, *binding))
                }
                _ => None,
            })
            .collect();
        for effect in effects.0 {
            match effect {
                CoreEffect::Broadcast(message) => {
                    if message.session_id > 0
                        && self
                            .sessions
                            .get(&LiveSessionId(message.session_id))
                            .is_some_and(|s| s.usage_probe)
                    {
                        continue;
                    }
                    for ui in self.uis.values_mut() {
                        if ui.priming {
                            ui.queued.push(UiFrame::Message(Box::new(message.clone())));
                        } else {
                            out.push(CoreEffect::SendUiBestEffort {
                                binding: ui.binding,
                                message: message.clone(),
                            });
                        }
                    }
                }
                CoreEffect::BroadcastGitTurn(event) => {
                    if self
                        .sessions
                        .get(&LiveSessionId(event.session_id))
                        .is_some_and(|s| s.usage_probe)
                    {
                        continue;
                    }
                    for ui in self.uis.values_mut() {
                        if ui.priming {
                            ui.queued.push(UiFrame::GitTurn(event.clone()));
                        } else {
                            out.push(CoreEffect::SendUiGitTurn {
                                binding: ui.binding,
                                event: event.clone(),
                                best_effort: true,
                            });
                        }
                    }
                }
                CoreEffect::Persist(effect) => {
                    let id = super::journal::persistence_session(&effect);
                    let binding = self
                        .sessions
                        .get(&id)
                        .map(|s| s.binding)
                        .or_else(|| ended.get(&id).copied());
                    if let Some(binding) = binding {
                        let scope = if matches!(
                            effect,
                            PersistenceEffect::EndSession { .. }
                                | PersistenceEffect::SessionState { .. }
                        ) {
                            PersistenceBindingScope::ExactWrapper
                        } else {
                            PersistenceBindingScope::Incarnation
                        };
                        out.push(CoreEffect::PersistBound {
                            binding,
                            scope,
                            effect,
                            order: Box::new(self.persistence_lane.reserve()),
                        });
                    } else {
                        out.push(CoreEffect::Persist(effect));
                    }
                }
                other => out.push(other),
            }
        }
        CoreEffects(out)
    }
}
impl Session {
    fn card_meta(&self) -> SessionCardMeta {
        SessionCardMeta {
            label: self.snapshot.label.clone(),
            pinned: self.snapshot.pinned,
            color: self.snapshot.color.clone(),
            note: self.snapshot.note.clone(),
            auto_title: self.snapshot.auto_title.clone(),
        }
    }
    fn update_message(&self) -> proto::Message {
        let s = &self.snapshot;
        proto::Message {
            r#type: "session_update".into(),
            session_id: s.id.0,
            provider: s.provider.clone(),
            display_name: s.display.clone(),
            cwd: s.cwd.clone(),
            branch: s.branch.clone(),
            project_id: s.project_id.clone(),
            label: s.label.clone(),
            launch_label: s.launch_label.clone(),
            model: s.model.clone(),
            execution_mode: s.execution_mode.clone(),
            permission_mode: s.permission_mode.clone(),
            route: s.route.clone(),
            state: s.state.clone(),
            output_idle: s.activity.output_idle,
            workflow_active: s.activity.workflow_active,
            awaiting_user: s.activity.awaiting_user,
            awaiting_approval: s.activity.awaiting_approval,
            activity: Some(s.activity.clone()),
            approval_source_epoch: self.approval.epoch().0,
            last_output_at: s.last_output_at.clone(),
            started_at: s.started_at.clone(),
            parent_session_id: s.parent_session_id.0,
            handoff_from: s.handoff_from.0,
            role: s.role.clone(),
            auto: s.auto,
            depth: s.depth,
            orchestration_id: s.orchestration_id.0.clone(),
            board_path: s.board_path.clone(),
            worktree_branch: s.worktree_branch.clone(),
            board_notify_pending: s.board_notify_pending,
            relays: s.relays.iter().flatten().cloned().collect(),
            cross_session_messages: s.cross_session_messages.clone(),
            subscription_id: s.subscription_profile_id.clone(),
            subscription_name: s.subscription_profile_name.clone(),
            ..Default::default()
        }
    }
    fn short_update(&self) -> proto::Message {
        let s = &self.snapshot;
        proto::Message {
            r#type: "session_update".into(),
            session_id: s.id.0,
            provider: s.provider.clone(),
            display_name: s.display.clone(),
            cwd: s.cwd.clone(),
            branch: s.branch.clone(),
            label: s.label.clone(),
            model: s.model.clone(),
            route: s.route.clone(),
            state: s.state.clone(),
            last_output_at: s.last_output_at.clone(),
            ..Default::default()
        }
    }
    fn activity_update(&self) -> proto::Message {
        let mut m = self.short_update();
        let a = &self.snapshot.activity;
        m.output_idle = a.output_idle;
        m.workflow_active = a.workflow_active;
        m.awaiting_user = a.awaiting_user;
        m.awaiting_approval = a.awaiting_approval;
        m.activity = Some(a.clone());
        m.transcript_grew_at = self.snapshot.transcript_grew_at.clone();
        m
    }
    fn summary_update(&self, meta: bool) -> proto::Message {
        let mut m = self.short_update();
        m.first_message = self.snapshot.first_message.clone();
        m.last_message = self.snapshot.last_message.clone();
        if meta {
            m.session_meta = Some(self.wire_card_meta());
        }
        m
    }
    fn wire_card_meta(&self) -> proto::SessionMeta {
        let s = &self.snapshot;
        proto::SessionMeta {
            label: s.label.clone(),
            pinned: s.pinned,
            color: s.color.clone(),
            note: s.note.clone(),
            auto_title: s.auto_title.clone(),
        }
    }
    fn card_update(&self) -> proto::Message {
        let s = &self.snapshot;
        proto::Message {
            r#type: "session_update".into(),
            session_id: s.id.0,
            provider: s.provider.clone(),
            display_name: s.display.clone(),
            cwd: s.cwd.clone(),
            branch: s.branch.clone(),
            state: s.state.clone(),
            session_meta: Some(self.wire_card_meta()),
            ..Default::default()
        }
    }
    fn replay_done(&self) -> proto::Message {
        let c = self.approval.consumed();
        proto::Message {
            r#type: "reattach_replay_done".into(),
            session_id: self.binding.session.0,
            replay: true,
            replay_epoch: self.replay_epoch.0,
            approval_source_epoch: self.approval.epoch().0,
            approval_consumed: c.is_some_and(|c| !c.key.is_empty()),
            approval_candidate_key: c.map(|c| c.key.clone()).unwrap_or_default(),
            approval_candidate_shape: c.map(|c| c.shape.clone()).unwrap_or_default(),
            approval_consumed_epoch: c.map(|c| c.source_epoch.0).unwrap_or_default(),
            ..Default::default()
        }
    }
}

impl SessionCore for SessionEngine {
    fn register<'a>(
        &'a self,
        request: RegisterRequest,
        connection: WrapperConnectionId,
        now: Timestamp,
    ) -> CoreFuture<'a, Result<Registration, SessionError>> {
        Box::pin(SessionEngine::register(self, request, connection, now))
    }
    fn reattach<'a>(
        &'a self,
        request: ReattachRequest,
        connection: WrapperConnectionId,
        now: Timestamp,
    ) -> CoreFuture<'a, Result<Reattachment, SessionError>> {
        Box::pin(SessionEngine::reattach(self, request, connection, now))
    }
    fn snapshot(&self, id: LiveSessionId) -> Option<SessionSnapshot> {
        SessionEngine::snapshot(self, id)
    }
    fn snapshots(&self) -> Vec<SessionSnapshot> {
        SessionEngine::snapshots(self)
    }
    fn details(&self, id: LiveSessionId) -> Option<SessionDetails> {
        SessionEngine::details(self, id)
    }
    fn active_ids(&self) -> Vec<LiveSessionId> {
        SessionEngine::active_ids(self)
    }
    fn spawn_failed(
        &self,
        attempt: SpawnAttemptId,
        provider: &str,
    ) -> Result<CoreEffects, SessionError> {
        SessionEngine::spawn_failed(self, attempt, provider)
    }
    fn is_current(&self, binding: SessionBinding) -> bool {
        SessionEngine::is_current(self, binding)
    }
    fn apply_observation(
        &self,
        binding: SessionBinding,
        observation: SessionObservation,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError> {
        SessionEngine::apply_observation(self, binding, observation, now)
    }
    fn observe_output(
        &self,
        binding: SessionBinding,
        chunk: OutputChunk,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError> {
        SessionEngine::observe_output(self, binding, chunk, now)
    }
    fn observe_end(
        &self,
        binding: SessionBinding,
        end: SessionEnd,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError> {
        SessionEngine::observe_end(self, binding, end, now)
    }
    fn disconnected(
        &self,
        binding: SessionBinding,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError> {
        SessionEngine::disconnected(self, binding, now)
    }
    fn resize(
        &self,
        ui: UiBinding,
        id: LiveSessionId,
        size: TerminalSize,
        now: Timestamp,
    ) -> (ResizeOutcome, CoreEffects) {
        SessionEngine::resize(self, ui, id, size, now)
    }
    fn reset_history(
        &self,
        id: LiveSessionId,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError> {
        SessionEngine::reset_history(self, id, now)
    }
    fn dismiss(&self, id: LiveSessionId, now: Timestamp) -> Result<CoreEffects, SessionError> {
        SessionEngine::dismiss(self, id, now)
    }
    fn reset_history_from_ui(
        &self,
        ui: UiBinding,
        id: LiveSessionId,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError> {
        SessionEngine::reset_history_from_ui(self, ui, id, now)
    }
    fn dismiss_from_ui(
        &self,
        ui: UiBinding,
        id: LiveSessionId,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError> {
        SessionEngine::dismiss_from_ui(self, ui, id, now)
    }
    fn stop(
        &self,
        id: LiveSessionId,
        reason: StopReason,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError> {
        SessionEngine::stop(self, id, reason, now)
    }
    fn update_card_meta(
        &self,
        id: LiveSessionId,
        meta: SessionCardMeta,
    ) -> Result<CoreEffects, SessionError> {
        SessionEngine::update_card_meta(self, id, meta)
    }
    fn auth_epoch(&self) -> AuthEpoch {
        SessionEngine::auth_epoch(self)
    }
    fn attach_ui(
        &self,
        ui: UiBinding,
        active: Option<LiveSessionId>,
        size: Option<TerminalSize>,
    ) -> Result<UiPriming, SessionError> {
        SessionEngine::attach_ui(self, ui, active, size)
    }
    fn finish_ui_priming(&self, ui: UiBinding) -> Result<CoreEffects, SessionError> {
        SessionEngine::finish_ui_priming(self, ui)
    }
    fn broadcast_ui(&self, message: proto::Message) -> CoreEffects {
        SessionEngine::broadcast_ui(self, message)
    }
    fn broadcast_git_turn(&self, event: GitTurnNotification) -> CoreEffects {
        SessionEngine::broadcast_git_turn(self, event)
    }
    fn authorize_ui_work(&self, ui: UiBinding) -> Result<Box<dyn AcceptedUiWork>, SessionError> {
        SessionEngine::authorize_ui_work(self, ui)
    }
    fn drain_ui_work(&self, ui: UiBinding) -> CoreFuture<'_, ()> {
        SessionEngine::drain_ui_work(self, ui)
    }
    fn claim_ui_session(
        &self,
        ui: UiBinding,
        id: LiveSessionId,
        size: Option<TerminalSize>,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError> {
        SessionEngine::claim_ui_session(self, ui, id, size, now)
    }
    fn consume_approval(
        &self,
        ui: UiBinding,
        message: proto::Message,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError> {
        SessionEngine::consume_approval(self, ui, message, now)
    }
    fn resync_approval(
        &self,
        ui: UiBinding,
        id: LiveSessionId,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError> {
        SessionEngine::resync_approval(self, ui, id, now)
    }
    fn detach_ui(&self, ui: UiBinding) -> CoreEffects {
        SessionEngine::detach_ui(self, ui)
    }
    fn invalidate_all_ui(&self) -> CoreEffects {
        SessionEngine::invalidate_all_ui(self)
    }
    fn subscribe(&self) -> Box<dyn CoreEventSubscription> {
        SessionEngine::subscribe(self)
    }
}
