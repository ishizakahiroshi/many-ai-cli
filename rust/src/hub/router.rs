use super::{
    assets, auth,
    http::{Request, Response, random_hex},
    pin::AuthService,
    settings,
};
use crate::{
    config::{ConfigStore, RuntimePaths},
    proto::core::{CoreEffects, LiveSessionId, SessionCore, SessionStorage},
};
use serde_json::json;
use std::sync::Arc;

pub struct ServiceRouter {
    pub config: Arc<ConfigStore>,
    pub paths: RuntimePaths,
    pub auth: AuthService,
    pub(crate) core: Option<Arc<dyn SessionCore>>,
    bound_port: u16,
    memos: Option<Arc<crate::routine::memo::MemoManager>>,
    files: Option<Arc<crate::files::FilesService>>,
    storage: Option<Arc<dyn SessionStorage>>,
    workspace: Option<Arc<dyn crate::files::WorkspaceReads>>,
    approvals: Option<ApprovalRoutes>,
    routines: Option<Arc<super::routines::RoutineHttp>>,
    confirmations: Option<Arc<super::confirmations::ConfirmationHttp>>,
    children: Option<Arc<super::child_routes::ChildHttp>>,
    child_controls: Option<Arc<super::child_control::ChildControl>>,
    relay: Option<Arc<super::relay_routes::RelayHttp>>,
    auto_approval: Option<Arc<super::auto_approval::AutoApprovalHttp>>,
    handoff: Option<Arc<super::handoff_routes::HandoffHttp>>,
    tasks: Option<super::task_owner::HubTaskHandle>,
    sessions: Option<SessionRoutes>,
    providers: Option<ProviderRoutes>,
    cli_versions: Option<Arc<super::cli_version::CliVersionHttp>>,
    models: Option<Arc<crate::application::main_program::model_catalog::ModelsCatalog>>,
    host: Option<Arc<super::host_routes::HostHttp>>,
    servers: Option<Arc<super::server_routes::ServerHttp>>,
    slash: Option<Arc<super::slash_routes::SlashHttp>>,
    whisper: Option<Arc<super::whisper_routes::WhisperHttp>>,
    subscriptions: Option<Arc<super::subscription_routes::SubscriptionHttp>>,
    usage: Option<Arc<super::usage_routes::UsageHttp>>,
    updates: Option<Arc<super::update_routes::UpdateHttp>>,
    runtime: Option<(Arc<super::runtime_routes::NetHints>, String)>,
    links: Option<Arc<crate::application::link_defaults::LinkDefaults>>,
    jev: Option<Arc<super::jev_routes::JevHttp>>,
    push: Option<Arc<super::push_routes::PushHttp>>,
    security: Option<Arc<crate::application::push::security::SecurityNotifications>>,
    icons: Option<Arc<super::icon_routes::IconHttp>>,
    distributions: Option<Arc<super::distribution_routes::DistributionHttp>>,
    diagnostics: Option<Arc<super::diagnostic_routes::DiagnosticHttp>>,
    agent_history: Option<Arc<super::agent_history_routes::AgentHistoryHttp>>,
    grid: Option<Arc<super::grid_spawn_routes::GridHttp>>,
    mobile: Option<Arc<super::mobile_routes::MobileHttp>>,
    nvidia: Option<Arc<super::nvidia_routes::NvidiaHttp>>,
    patterns: Option<Arc<super::approval_pattern_routes::ApprovalPatternHttp>>,
    instruction_rules: Option<Arc<super::approval_status_routes::ApprovalStatusHttp>>,
    pub(crate) voice: Option<Arc<super::voice_routes::VoiceHttp>>,
    spawn: Option<(Arc<super::spawn_routes::SpawnHttp>, Arc<SpawnOffset>)>,
    settings_published: Option<Arc<SettingsPublished>>,
    notifications: Option<Arc<crate::notify::Manager>>,
    assets: Option<Arc<dyn assets::AssetSource>>,
    shutdown: Option<Arc<super::lifecycle::ShutdownOwner>>,
    push_presence: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
}
pub type ProviderCommandFound = dyn Fn(&str) -> bool + Send + Sync;
pub type SpawnOffset = dyn Fn() -> i32 + Send + Sync;
pub type SettingsPublished = dyn Fn(&str, &crate::config::Config) + Send + Sync;
pub type ProviderWarning =
    dyn Fn(&'static str, &str, &crate::profile::store::StoreError) + Send + Sync;
struct ProviderRoutes {
    store: Arc<crate::profile::store::ProviderRegistryStore>,
    command_found: Arc<ProviderCommandFound>,
    warning: Arc<ProviderWarning>,
}
pub type InfoFactory = dyn Fn(&super::session_routes::SessionHttp, &crate::config::Config) -> Result<Dispatch, Response>
    + Send
    + Sync;
struct SessionRoutes {
    http: Arc<super::session_routes::SessionHttp>,
    info: Arc<InfoFactory>,
    effects: Arc<dyn crate::proto::core::CoreEffectSink>,
}
struct ApprovalRoutes {
    actions: Arc<super::approval_actions::ApprovalActionHttp>,
    rules: Arc<dyn super::approval_actions::ApprovalBatchRules>,
}
/// Side effects must be driven by the actual WebSocket owner, in order, before
/// acknowledging an auth revocation. They are never dropped inside the router.
pub struct Dispatch {
    pub response: Response,
    pub effects: CoreEffects,
}
impl Dispatch {
    fn response(response: Response) -> Self {
        Self {
            response,
            effects: CoreEffects::default(),
        }
    }
}
impl ServiceRouter {
    /// Header-only authentication before the transport reads any request body.
    /// Handler entry repeats this check against a fresh config snapshot so a
    /// concurrent revocation cannot authorize a request that was still reading.
    pub fn preflight(&self, request: &Request, now: i64) -> Result<(), Response> {
        self.preflight_inner(request, now).map_err(|error| {
            if request.path == "/api/info" {
                error.no_store()
            } else {
                error
            }
        })
    }
    fn preflight_inner(&self, request: &Request, now: i64) -> Result<(), Response> {
        if let Some(response) = provider_early_response(request) {
            return Err(response);
        }
        if assets::is_static(&request.path)
            || request.path == "/"
            || (!request.path.starts_with("/api/")
                && request.path != "/ws"
                && !request.path.starts_with("/approval-patterns/"))
        {
            return Ok(());
        }
        let snapshot = self
            .config
            .snapshot()
            .map_err(|_| Response::error(500, "internal", "configuration unavailable"))?;
        let config = &snapshot.config;
        let port = i64::from(self.bound_port);
        if let Some(error) = session_prefix_error(request, config) {
            return Err(error);
        }
        if super::confirmations::is_confirmation_route(&request.path)
            || super::child_routes::is_spawn_route(&request.path)
            || super::child_control::is_control_route(&request.path)
        {
            // Go handleSessionAPI checks token, path shape and ID before the
            // endpoint's ordinary method/Host/Origin/PIN guard.
            if !auth::token_or_trusted(request, config) {
                return Err(Response::error(401, "unauthorized", "unauthorized"));
            }
            super::confirmations::parent_path(&request.path)?;
        }
        if super::session_routes::is_metadata_route(&request.path) {
            if !auth::token_or_trusted(request, config) {
                return Err(Response::error(401, "unauthorized", "unauthorized"));
            }
            super::session_routes::metadata_path(&request.path)?;
        }
        if request.path.starts_with("/api/approval-action/") {
            super::approval_actions::one_tap_token(&request.path)?;
            super::http::require_method(request, &["POST"])?;
            auth::require_host(request, config, port)?;
            return auth::require_origin(request, config, port);
        }
        let base = matches!(
            request.path.as_str(),
            "/api/auth/status" | "/api/auth/login" | "/api/auth/logout"
        );
        let methods = guard_methods(&request.path);
        if base {
            auth::guard_base(request, config, port, methods)?;
        } else {
            self.auth.guard(request, config, port, methods, now)?;
        }
        if let Some(host) = &self.host {
            host.preflight_authenticated(request)?;
        }
        if let Some(push) = &self.push {
            push.preflight_authenticated(request)?;
        }
        if let Some(icons) = &self.icons {
            icons.preflight_authenticated(request)?;
        }
        if request.path == super::jev_routes::PATH
            && self.jev.as_ref().is_some_and(|owner| !owner.configured())
        {
            return Err(Response::error(
                503,
                "jev_not_configured",
                "Set TYPESAFE_API_KEY in the Hub process environment",
            ));
        }
        Ok(())
    }
    pub fn new(
        config: Arc<ConfigStore>,
        paths: RuntimePaths,
        core: Arc<dyn SessionCore>,
        bound_port: u16,
    ) -> std::io::Result<Self> {
        validate_bound_port(&paths, bound_port)?;
        Ok(Self {
            config,
            paths,
            auth: AuthService::default(),
            core: Some(core),
            bound_port,
            memos: None,
            files: None,
            storage: None,
            workspace: None,
            approvals: None,
            routines: None,
            confirmations: None,
            children: None,
            child_controls: None,
            relay: None,
            auto_approval: None,
            handoff: None,
            tasks: None,
            sessions: None,
            providers: None,
            cli_versions: None,
            models: None,
            host: None,
            servers: None,
            slash: None,
            whisper: None,
            subscriptions: None,
            usage: None,
            updates: None,
            runtime: None,
            links: None,
            jev: None,
            push: None,
            security: None,
            icons: None,
            distributions: None,
            diagnostics: None,
            agent_history: None,
            grid: None,
            mobile: None,
            nvidia: None,
            patterns: None,
            instruction_rules: None,
            voice: None,
            spawn: None,
            settings_published: None,
            notifications: None,
            assets: None,
            shutdown: None,
            push_presence: None,
        })
    }
    pub fn with_provider_routes(
        mut self,
        store: Arc<crate::profile::store::ProviderRegistryStore>,
        command_found: Arc<ProviderCommandFound>,
        warning: Arc<ProviderWarning>,
    ) -> Self {
        self.providers = Some(ProviderRoutes {
            store,
            command_found,
            warning,
        });
        self
    }
    pub fn with_cli_versions(mut self, versions: Arc<super::cli_version::CliVersionHttp>) -> Self {
        self.cli_versions = Some(versions);
        self
    }
    pub fn with_spawn(
        mut self,
        spawn: Arc<super::spawn_routes::SpawnHttp>,
        offset: Arc<dyn Fn() -> i32 + Send + Sync>,
    ) -> Self {
        self.spawn = Some((spawn, offset));
        self
    }
    pub fn with_settings_published(mut self, published: Arc<SettingsPublished>) -> Self {
        self.settings_published = Some(published);
        self
    }
    pub fn with_notifications(mut self, notifications: Arc<crate::notify::Manager>) -> Self {
        self.notifications = Some(notifications);
        self
    }
    pub fn with_models(
        mut self,
        models: Arc<crate::application::main_program::model_catalog::ModelsCatalog>,
    ) -> Self {
        self.models = Some(models);
        self
    }
    pub fn with_host(mut self, host: Arc<super::host_routes::HostHttp>) -> Self {
        self.host = Some(host);
        self
    }
    pub fn with_assets(mut self, source: Arc<dyn assets::AssetSource>) -> Self {
        self.assets = Some(source);
        self
    }
    pub fn with_shutdown(mut self, shutdown: Arc<super::lifecycle::ShutdownOwner>) -> Self {
        self.shutdown = Some(shutdown);
        self
    }
    pub fn with_push_presence(mut self, presence: Arc<dyn Fn() -> bool + Send + Sync>) -> Self {
        self.push_presence = Some(presence);
        self
    }
    pub fn with_session_routes(
        mut self,
        http: Arc<super::session_routes::SessionHttp>,
        info: Arc<InfoFactory>,
        effects: Arc<dyn crate::proto::core::CoreEffectSink>,
    ) -> Self {
        self.sessions = Some(SessionRoutes {
            http,
            info,
            effects,
        });
        self
    }
    pub fn with_approval_actions(
        mut self,
        actions: Arc<super::approval_actions::ApprovalActionHttp>,
        rules: Arc<dyn super::approval_actions::ApprovalBatchRules>,
    ) -> Self {
        self.approvals = Some(ApprovalRoutes { actions, rules });
        self
    }
    pub fn with_task_owner(mut self, tasks: super::task_owner::HubTaskHandle) -> Self {
        self.tasks = Some(tasks);
        self
    }
    pub fn with_confirmations(
        mut self,
        confirmations: Arc<super::confirmations::ConfirmationHttp>,
    ) -> Self {
        self.confirmations = Some(confirmations);
        self
    }
    pub fn with_children(mut self, children: Arc<super::child_routes::ChildHttp>) -> Self {
        self.children = Some(children);
        self
    }
    pub fn with_relay(mut self, relay: Arc<super::relay_routes::RelayHttp>) -> Self {
        self.relay = Some(relay);
        self
    }
    pub fn with_child_controls(
        mut self,
        controls: Arc<super::child_control::ChildControl>,
    ) -> Self {
        self.child_controls = Some(controls);
        self
    }
    pub fn with_auto_approval(
        mut self,
        owner: Arc<super::auto_approval::AutoApprovalHttp>,
    ) -> Self {
        self.auto_approval = Some(owner);
        self
    }
    pub fn with_handoff(mut self, owner: Arc<super::handoff_routes::HandoffHttp>) -> Self {
        self.handoff = Some(owner);
        self
    }
    pub fn with_servers(mut self, owner: Arc<super::server_routes::ServerHttp>) -> Self {
        self.servers = Some(owner);
        self
    }
    pub fn with_slash_commands(mut self, owner: Arc<super::slash_routes::SlashHttp>) -> Self {
        self.slash = Some(owner);
        self
    }
    pub fn with_voice(mut self, owner: Arc<super::voice_routes::VoiceHttp>) -> Self {
        self.voice = Some(owner);
        self
    }
    pub fn with_whisper(mut self, owner: Arc<super::whisper_routes::WhisperHttp>) -> Self {
        self.whisper = Some(owner);
        self
    }
    pub fn with_subscriptions(
        mut self,
        owner: Arc<super::subscription_routes::SubscriptionHttp>,
    ) -> Self {
        self.subscriptions = Some(owner);
        self
    }
    pub fn with_session_usage(mut self, owner: Arc<super::usage_routes::UsageHttp>) -> Self {
        self.usage = Some(owner);
        self
    }
    pub fn with_cli_updates(mut self, owner: Arc<super::update_routes::UpdateHttp>) -> Self {
        self.updates = Some(owner);
        self
    }
    pub fn with_runtime_observations(
        mut self,
        hints: Arc<super::runtime_routes::NetHints>,
        shell: String,
    ) -> Self {
        self.runtime = Some((hints, shell));
        self
    }
    pub fn with_link_defaults(
        mut self,
        links: Arc<crate::application::link_defaults::LinkDefaults>,
    ) -> Self {
        self.links = Some(links);
        self
    }
    pub fn with_jev(mut self, owner: Arc<super::jev_routes::JevHttp>) -> Self {
        self.jev = Some(owner);
        self
    }
    pub fn with_push(mut self, owner: Arc<super::push_routes::PushHttp>) -> Self {
        self.push = Some(owner);
        self
    }
    pub fn with_security(
        mut self,
        owner: Arc<crate::application::push::security::SecurityNotifications>,
    ) -> Self {
        self.security = Some(owner);
        self
    }
    pub(crate) fn note_authenticated_remote(
        &self,
        request: &Request,
        via: &str,
        now: crate::proto::time::Timestamp,
    ) {
        if let Some(owner) = &self.security {
            owner.note_authenticated(request, via, now);
        }
    }
    pub fn with_provider_assets(
        mut self,
        icons: Arc<super::icon_routes::IconHttp>,
        distributions: Arc<super::distribution_routes::DistributionHttp>,
    ) -> Self {
        self.icons = Some(icons);
        self.distributions = Some(distributions);
        self
    }
    pub fn with_diagnostics(
        mut self,
        owner: Arc<super::diagnostic_routes::DiagnosticHttp>,
    ) -> Self {
        self.diagnostics = Some(owner);
        self
    }
    pub fn with_agent_history(
        mut self,
        owner: Arc<super::agent_history_routes::AgentHistoryHttp>,
    ) -> Self {
        self.agent_history = Some(owner);
        self
    }
    pub fn with_grid(mut self, owner: Arc<super::grid_spawn_routes::GridHttp>) -> Self {
        self.grid = Some(owner);
        self
    }
    pub fn with_mobile(mut self, owner: Arc<super::mobile_routes::MobileHttp>) -> Self {
        self.mobile = Some(owner);
        self
    }
    pub fn with_nvidia(mut self, owner: Arc<super::nvidia_routes::NvidiaHttp>) -> Self {
        self.nvidia = Some(owner);
        self
    }
    pub fn with_approval_patterns(
        mut self,
        owner: Arc<super::approval_pattern_routes::ApprovalPatternHttp>,
    ) -> Self {
        self.patterns = Some(owner);
        self
    }
    pub fn with_instruction_rules(
        mut self,
        owner: Arc<super::approval_status_routes::ApprovalStatusHttp>,
    ) -> Self {
        self.instruction_rules = Some(owner);
        self
    }
    pub fn with_routines(mut self, routines: Arc<super::routines::RoutineHttp>) -> Self {
        self.routines = Some(routines);
        self
    }
    pub(crate) fn owned_request_permit(
        &self,
    ) -> Result<super::task_owner::OwnedTaskPermit, crate::proto::core::SessionError> {
        self.tasks
            .as_ref()
            .ok_or(crate::proto::core::SessionError::Shutdown)?
            .request_permit()
    }
    pub(crate) async fn handle_owned_async_at(
        &self,
        request: &Request,
        at: crate::proto::time::Timestamp,
        cancellation: &crate::proto::core::TaskCancellation,
        waiter: &crate::proto::core::HttpWaitCancellation,
    ) -> Dispatch {
        let epoch = self.core.as_ref().map(|core| core.auth_epoch());
        let now = at
            .duration_since(crate::proto::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        if let Err(error) = self.preflight(request, now) {
            return Dispatch::response(error);
        }
        if super::approval_pattern_routes::is_route(&request.path) {
            let Some(owner) = self.patterns.clone() else {
                return Dispatch::response(Response::error(
                    503,
                    "approval_patterns_unavailable",
                    "approval patterns unavailable",
                ));
            };
            let request = request.clone();
            let response =
                tokio::task::spawn_blocking(move || owner.handle_authenticated(&request))
                    .await
                    .unwrap_or_else(|_| {
                        Response::error(
                            500,
                            "approval_patterns_failed",
                            "approval pattern operation failed",
                        )
                    });
            return Dispatch::response(response);
        }
        if super::approval_status_routes::methods(&request.path).is_some() {
            let Some(owner) = &self.instruction_rules else {
                return Dispatch::response(Response::error(
                    503,
                    "approval_instructions_unavailable",
                    "approval instructions unavailable",
                ));
            };
            let cancel = crate::process::Cancellation::default();
            let operation = owner.handle_authenticated(request, &cancel);
            tokio::pin!(operation);
            let response = tokio::select! {
                response=&mut operation=>response,
                _=cancellation.token().cancelled()=>{cancel.cancel();operation.await},
            };
            return Dispatch::response(response);
        }
        if super::agent_history_routes::methods(&request.path).is_some() {
            let Some(owner) = self.agent_history.clone() else {
                return Dispatch::response(Response::error(
                    503,
                    "agent_history_unavailable",
                    "agent history unavailable",
                ));
            };
            let request = request.clone();
            let cancel = crate::process::Cancellation::default();
            let actor_cancel = cancel.clone();
            let operation = tokio::task::spawn_blocking(move || {
                owner.handle_authenticated(&request, &actor_cancel)
            });
            tokio::pin!(operation);
            let response = tokio::select! {
                result = &mut operation => result,
                _ = cancellation.token().cancelled() => { cancel.cancel(); operation.await },
                _ = waiter.token().cancelled() => { cancel.cancel(); operation.await },
            }
            .unwrap_or_else(|_| {
                Response::error(
                    500,
                    "agent_history_failed",
                    "agent history operation failed",
                )
            });
            return Dispatch::response(response);
        }
        if super::diagnostic_routes::DiagnosticHttp::methods(&request.path).is_some() {
            let Some(owner) = &self.diagnostics else {
                return Dispatch::response(Response::error(
                    503,
                    "diagnostics_unavailable",
                    "diagnostic service unavailable",
                ));
            };
            // Propagate request disconnect as cancellation, then await the
            // actor so contained child/gist processes finish their shutdown.
            let cancel = crate::process::Cancellation::default();
            let operation = owner.handle_authenticated(request, &cancel);
            tokio::pin!(operation);
            let response = tokio::select! {
                response = &mut operation => response,
                _ = cancellation.token().cancelled() => {
                    cancel.cancel();
                    operation.await
                },
                _ = waiter.token().cancelled() => {
                    cancel.cancel();
                    operation.await
                },
            };
            return Dispatch::response(response.expect("recognized diagnostic route"));
        }
        if super::mobile_routes::methods(&request.path).is_some() {
            let Some(owner) = &self.mobile else {
                return Dispatch::response(Response::error(
                    503,
                    "mobile_unavailable",
                    "mobile connection service unavailable",
                ));
            };
            let cancel = crate::process::Cancellation::default();
            let operation = owner.handle_authenticated(request, &cancel);
            tokio::pin!(operation);
            let response = tokio::select! {
                response = &mut operation => response,
                _ = cancellation.token().cancelled() => { cancel.cancel(); operation.await },
                _ = waiter.token().cancelled() => { cancel.cancel(); operation.await },
            };
            return Dispatch::response(response);
        }
        if super::nvidia_routes::NvidiaHttp::methods(&request.path).is_some() {
            let Some(owner) = &self.nvidia else {
                return Dispatch::response(Response::error(
                    503,
                    "nvidia_unavailable",
                    "NVIDIA service unavailable",
                ));
            };
            let cancel = crate::process::Cancellation::default();
            let operation = owner.handle_authenticated(request, &cancel);
            tokio::pin!(operation);
            let response = tokio::select! {
                response = &mut operation => response,
                _ = cancellation.token().cancelled() => { cancel.cancel(); operation.await },
                _ = waiter.token().cancelled() => { cancel.cancel(); operation.await },
            };
            return Dispatch::response(response.expect("recognized NVIDIA route"));
        }
        if request.path == "/api/kill-all" {
            let (Some(core), Some(sessions)) = (&self.core, &self.sessions) else {
                return Dispatch::response(Response::error(
                    503,
                    "sessions_unavailable",
                    "session services unavailable",
                ));
            };
            // Stop each connected wrapper in source order without dismissing
            // its session, changing its worktree, or purging its history.
            let ids = core.registered_session_ids();
            for id in ids {
                if core.details(id).is_some_and(|details| details.connected)
                    && let Ok(effects) = core.stop(id, crate::proto::core::StopReason::KillAll, at)
                {
                    let _ = sessions.effects.apply(effects).await;
                }
            }
            return Dispatch::response(Response::json(200, &json!({"ok":true})));
        }
        if request.path == "/api/spawn-grid" {
            let (Some(owner), Some(epoch)) = (&self.grid, epoch) else {
                return Dispatch::response(Response::error(
                    503,
                    "spawn_unavailable",
                    "grid spawn service unavailable",
                ));
            };
            return Dispatch::response(owner.spawn_authenticated(request,
                &crate::proto::core::VerifiedConfirmationRequest::after_server_authentication(epoch)).await);
        }
        if request.path == super::handoff_routes::PATH
            || request.path.starts_with(super::handoff_routes::PREFIX)
        {
            return Dispatch::response(match &self.handoff {
                Some(owner) => owner
                    .handle_authenticated(request, at)
                    .await
                    .unwrap_or_else(|| Response::error(404, "not_found", "not found")),
                None => Response::error(
                    503,
                    "handoff_unavailable",
                    "handoff service is not initialized",
                ),
            });
        }
        if super::relay_routes::is_relay_route(&request.path) {
            let Some(owner) = &self.relay else {
                return Dispatch::response(Response::error(
                    503,
                    "relay_unavailable",
                    "relay service unavailable",
                ));
            };
            return Dispatch::response(
                owner
                    .handle_authenticated(request, waiter, at)
                    .await
                    .expect("recognized relay route"),
            );
        }
        if super::child_control::is_control_route(&request.path) {
            return Dispatch::response(
                self.child_controls
                    .as_ref()
                    .map(|controls| controls.handle_authenticated(request, at))
                    .unwrap_or_else(|| {
                        Response::error(
                            503,
                            "child_control_unavailable",
                            "child controls are not initialized",
                        )
                    }),
            );
        }
        if super::host_routes::methods(&request.path).is_some() {
            return Dispatch::response(match &self.host {
                Some(host) => host
                    .handle_authenticated(request)
                    .await
                    .unwrap_or_else(|| Response::error(404, "not_found", "not found")),
                None => Response::error(
                    503,
                    "host_service_unavailable",
                    "host service is not initialized",
                ),
            });
        }
        if request.path == "/api/spawn" {
            return Dispatch::response(match (&self.spawn, epoch) {
                (Some((spawn, offset)), Some(epoch)) => spawn.spawn_authenticated(
                    request,
                    &crate::proto::core::VerifiedConfirmationRequest::after_server_authentication(epoch),
                    waiter,
                    at,
                    offset(),
                ).await,
                _ => Response::error(503, "spawn_unavailable", "spawn service is not initialized"),
            });
        }
        if super::child_routes::is_spawn_route(&request.path) {
            let ui = self.config.snapshot().ok().and_then(|snapshot| {
                let config = snapshot.config;
                let bound = self.auth.valid_ui_cookie(request, &config, now)
                    && request
                        .header("Sec-Fetch-Site")
                        .trim()
                        .eq_ignore_ascii_case("same-origin")
                    && !request.header("Origin").trim().is_empty()
                    && auth::allowed_origin(
                        request.header("Origin").trim(),
                        i64::from(self.bound_port),
                        &config.hub.allowed_hosts,
                    );
                epoch
                    .filter(|_| bound)
                    .map(crate::proto::core::VerifiedUiOrigin::after_server_verification)
            });
            return Dispatch::response(match &self.children {
                Some(children) => children.spawn_authenticated(request, ui, waiter, at).await,
                None => Response::error(
                    503,
                    "spawn_unavailable",
                    "child spawn service is not initialized",
                ),
            });
        }
        if request.path == "/api/notify-test" {
            return Dispatch::response(match &self.notifications {
                Some(manager) => {
                    let waiter_token = waiter.token();
                    let owner_token = cancellation.token();
                    tokio::select! {
                        result = super::notify_routes::send_test(request, manager) => result,
                        _ = waiter_token.cancelled() => Response::error(503, "notify_cancelled", "notification test was cancelled"),
                        _ = owner_token.cancelled() => Response::error(503, "notify_cancelled", "notification test was cancelled"),
                    }
                }
                None => {
                    Response::error(503, "notify_unavailable", "notify manager not initialized")
                }
            });
        }
        if request.path == "/api/shutdown" {
            return Dispatch::response(match &self.shutdown {
                Some(shutdown) => match shutdown.request().await {
                    Ok(()) => Response::json(200, &json!({"ok":true})),
                    Err(_) => {
                        Response::error(503, "shutdown_unavailable", "Hub shutdown is unavailable")
                    }
                },
                None => Response::error(
                    503,
                    "shutdown_unavailable",
                    "Hub shutdown is not initialized",
                ),
            });
        }
        if request.path == super::cli_version::PATH {
            let Some(versions) = &self.cli_versions else {
                return Dispatch::response(Response::error(
                    503,
                    "cli_version_unavailable",
                    "CLI version service is not initialized",
                ));
            };
            let registry = self
                .providers
                .as_ref()
                .and_then(|providers| providers.store.snapshot().ok());
            return Dispatch::response(
                versions
                    .handle_authenticated(
                        request,
                        registry.as_ref().map(|snapshot| snapshot.registry.as_ref()),
                    )
                    .await,
            );
        }
        if super::subscription_routes::methods(&request.path).is_some() {
            return Dispatch::response(match &self.subscriptions {
                Some(owner) => owner
                    .handle_authenticated(request, at)
                    .await
                    .expect("recognized subscription route"),
                None => Response::error(
                    503,
                    "subscriptions_unavailable",
                    "subscription service unavailable",
                ),
            });
        }
        if super::update_routes::methods(&request.path).is_some() {
            return Dispatch::response(match &self.updates {
                Some(owner) => owner
                    .handle_authenticated(request, at)
                    .expect("recognized update route"),
                None => Response::error(503, "update_unavailable", "CLI updater unavailable"),
            });
        }
        if request.path == super::jev_routes::PATH {
            return Dispatch::response(match &self.jev {
                Some(owner) => tokio::select! {
                    response=owner.handle_authenticated(request,cancellation.token())=>response,
                    _=waiter.token().cancelled()=>Response::error(502,"jev_failed","Jev evaluation failed or returned an unexpected response"),
                },
                None => Response::error(
                    503,
                    "jev_not_configured",
                    "Set TYPESAFE_API_KEY in the Hub process environment",
                ),
            });
        }
        if super::session_routes::MAINTENANCE_ROUTES.contains(&request.path.as_str())
            || super::session_routes::LOG_MAINTENANCE_ROUTES.contains(&request.path.as_str())
        {
            let Some(sessions) = &self.sessions else {
                return Dispatch::response(Response::error(
                    503,
                    "sessions_unavailable",
                    "session services unavailable",
                ));
            };
            let http = sessions.http.clone();
            let config = self.config.clone();
            let paths = self.paths.clone();
            let request = request.clone();
            return Dispatch::response(
                tokio::task::spawn_blocking(move || {
                    if super::session_routes::LOG_MAINTENANCE_ROUTES
                        .contains(&request.path.as_str())
                    {
                        http.handle_log_maintenance_authenticated(&request, &config, &paths)
                    } else {
                        http.handle_maintenance_authenticated(&request)
                    }
                })
                .await
                .unwrap_or_else(|_| {
                    Response::error(
                        500,
                        "session_store_reset_failed",
                        "failed to reset saved session history",
                    )
                }),
            );
        }
        if matches!(
            request.path.as_str(),
            "/api/usage-link-defaults" | "/api/install-link-defaults"
        ) {
            return Dispatch::response(match &self.links {
                Some(links) => Response::json(
                    200,
                    &links
                        .get(request.path == "/api/install-link-defaults")
                        .await,
                ),
                None => Response::error(503, "links_unavailable", "link catalogs unavailable"),
            });
        }
        if super::usage_routes::is_route(&request.path) {
            return Dispatch::response(match &self.usage {
                Some(owner) => owner
                    .handle_authenticated(request, at)
                    .await
                    .expect("recognized usage route"),
                None => Response::error(503, "usage_unavailable", "usage service unavailable"),
            });
        }
        if request.path == super::voice_routes::PATH {
            // The raw transport prepares managed voice before reading audio.
            return Dispatch::response(match &self.voice {
                Some(voice) => {
                    voice
                        .handle_authenticated(request, cancellation.token())
                        .await
                }
                None => Response::error(
                    503,
                    "whisper_unavailable",
                    "voice service is not initialized",
                ),
            });
        }
        if super::whisper_routes::WhisperHttp::methods(&request.path).is_some() {
            return Dispatch::response(match &self.whisper {
                Some(whisper) => whisper
                    .handle_authenticated(request, cancellation.token())
                    .await
                    .expect("recognized whisper route"),
                None => Response::error(
                    503,
                    "whisper_unavailable",
                    "managed Whisper service is not initialized",
                ),
            });
        }
        if request.path == "/api/models" {
            return Dispatch::response(match &self.models {
                Some(models) => models.handle(request, cancellation.token()).await,
                None => Response::error(
                    503,
                    "models_unavailable",
                    "model catalog is not initialized",
                ),
            });
        }
        if super::server_routes::methods(&request.path).is_some() {
            return Dispatch::response(match &self.servers {
                Some(servers) => {
                    servers
                        .handle_authenticated(request, cancellation.token())
                        .await
                }
                None => Response::error(
                    503,
                    "servers_unavailable",
                    "server connection manager is not initialized",
                ),
            });
        }
        if super::slash_routes::is_route(&request.path) {
            return Dispatch::response(match &self.slash {
                Some(slash) => slash
                    .handle_authenticated(request, at)
                    .await
                    .expect("recognized slash route"),
                None => Response::error(
                    503,
                    "slash_unavailable",
                    "slash command service unavailable",
                ),
            });
        }
        if super::session_routes::is_metadata_route(&request.path) {
            return Dispatch::response(match (&self.sessions, &self.tasks) {
                (Some(sessions), Some(tasks)) => {
                    sessions
                        .http
                        .handle_metadata_authenticated(request, sessions.effects.clone(), tasks)
                        .await
                }
                _ => Response::error(
                    503,
                    "session_meta_unavailable",
                    "session metadata service is not initialized",
                ),
            });
        }
        if super::confirmations::is_confirmation_route(&request.path) {
            return Dispatch::response(match (&self.confirmations, &self.tasks, epoch) {
                (Some(confirmations), Some(tasks), Some(epoch)) => confirmations
                    .handle_authenticated(
                    request,
                    crate::proto::core::VerifiedConfirmationRequest::after_server_authentication(
                        epoch,
                    ),
                    at,
                    tasks,
                ),
                _ => Response::error(
                    503,
                    "confirmation_unavailable",
                    "confirmation service is not initialized",
                ),
            });
        }
        if is_routine_route(&request.path) {
            return Dispatch::response(
                self.routines
                    .as_ref()
                    .and_then(|routines| routines.handle_authenticated(request, at))
                    .unwrap_or_else(|| {
                        Response::error(
                            503,
                            "routine_unavailable",
                            "routine service is not initialized",
                        )
                    }),
            );
        }
        let Some(approvals) = &self.approvals else {
            return Dispatch::response(Response::error(
                503,
                "approval_unavailable",
                "approval service is not initialized",
            ));
        };
        let response = if request
            .path
            .starts_with(super::approval_actions::ONE_TAP_PREFIX)
        {
            match super::approval_actions::one_tap_token(&request.path) {
                Ok(token) => approvals.actions.one_tap_guarded(token, cancellation).await,
                Err(error) => error,
            }
        } else if request.path == "/api/approval/batch" {
            approvals
                .actions
                .batch_authenticated(request, approvals.rules.as_ref(), cancellation)
                .await
        } else {
            Response::error(404, "not_found", "not found")
        };
        Dispatch::response(response)
    }
    pub fn bound_port(&self) -> u16 {
        self.bound_port
    }
    #[cfg(test)]
    pub(crate) fn isolated(config: Arc<ConfigStore>, paths: RuntimePaths) -> Self {
        let bound_port = paths.port();
        Self::isolated_at_port(config, paths, bound_port).expect("valid isolated port")
    }
    #[cfg(test)]
    pub(crate) fn isolated_at_port(
        config: Arc<ConfigStore>,
        paths: RuntimePaths,
        bound_port: u16,
    ) -> std::io::Result<Self> {
        validate_bound_port(&paths, bound_port)?;
        Ok(Self {
            assets: None,
            config,
            paths,
            auth: AuthService::default(),
            core: None,
            bound_port,
            memos: None,
            files: None,
            storage: None,
            workspace: None,
            approvals: None,
            routines: None,
            confirmations: None,
            children: None,
            child_controls: None,
            relay: None,
            auto_approval: None,
            handoff: None,
            tasks: None,
            sessions: None,
            providers: None,
            cli_versions: None,
            models: None,
            host: None,
            servers: None,
            slash: None,
            whisper: None,
            subscriptions: None,
            usage: None,
            updates: None,
            runtime: None,
            links: None,
            jev: None,
            push: None,
            security: None,
            icons: None,
            distributions: None,
            diagnostics: None,
            agent_history: None,
            grid: None,
            mobile: None,
            nvidia: None,
            patterns: None,
            instruction_rules: None,
            voice: None,
            spawn: None,
            settings_published: None,
            notifications: None,
            shutdown: None,
            push_presence: None,
        })
    }
    pub fn with_memos(mut self, memos: Arc<crate::routine::memo::MemoManager>) -> Self {
        self.memos = Some(memos);
        self
    }
    pub fn with_files(
        mut self,
        files: Arc<crate::files::FilesService>,
        storage: Option<Arc<dyn SessionStorage>>,
        workspace: Arc<dyn crate::files::WorkspaceReads>,
    ) -> Self {
        self.files = Some(files);
        self.storage = storage;
        self.workspace = Some(workspace);
        self
    }
    /// Called only while the WebSocket owner holds core's accepted UI guard.
    pub(crate) fn record_ws_attachment(
        &self,
        message: &crate::proto::Message,
    ) -> Result<(), crate::proto::core::SessionError> {
        let (Some(files), Some(core)) = (&self.files, &self.core) else {
            return Err(crate::proto::core::SessionError::InvalidRequest(
                "attachment service is not initialized".into(),
            ));
        };
        files
            .record_ws_attachment(message, core.as_ref(), self.storage.as_deref())
            .map_err(|_| {
                crate::proto::core::SessionError::Storage(crate::proto::core::StorageError {
                    kind: crate::proto::core::StorageErrorKind::Write,
                    detail: "attachment operation failed".into(),
                })
            })
    }
    pub async fn handle_async(&self, request: &Request, now: i64) -> Dispatch {
        self.handle_async_at(
            request,
            crate::proto::time::UNIX_EPOCH + std::time::Duration::from_secs(now.max(0) as u64),
        )
        .await
    }
    pub async fn handle_async_at(
        &self,
        request: &Request,
        at: crate::proto::time::Timestamp,
    ) -> Dispatch {
        let now = at
            .duration_since(crate::proto::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        if matches!(
            request.path.as_str(),
            super::auto_approval::STATUS | super::auto_approval::SIMULATE
        ) {
            if let Err(error) = self.preflight(request, now) {
                return Dispatch::response(error);
            }
            return Dispatch::response(
                self.auto_approval
                    .as_ref()
                    .and_then(|owner| owner.handle_authenticated(request))
                    .unwrap_or_else(|| {
                        Response::error(
                            503,
                            "auto_approval_unavailable",
                            "automatic approval service is not initialized",
                        )
                    }),
            );
        }
        if request.path == "/api/info"
            || super::session_routes::READ_ROUTES.contains(&request.path.as_str())
        {
            if let Err(error) = self.preflight(request, now) {
                return Dispatch::response(error);
            }
            let unavailable = || {
                Response::error(
                    503,
                    "session_service_unavailable",
                    "session service is not initialized",
                )
            };
            let Some(sessions) = &self.sessions else {
                let error = unavailable();
                return Dispatch::response(if request.path == "/api/info" {
                    error.no_store()
                } else {
                    error
                });
            };
            let snapshot = match self.config.snapshot() {
                Ok(snapshot) => snapshot,
                Err(_) => {
                    return Dispatch::response(if request.path == "/api/info" {
                        unavailable().no_store()
                    } else {
                        unavailable()
                    });
                }
            };
            return if request.path == "/api/info" {
                match (sessions.info)(sessions.http.as_ref(), &snapshot.config) {
                    Ok(mut dispatch) => {
                        dispatch.response = dispatch.response.no_store();
                        dispatch
                    }
                    Err(error) => Dispatch::response(error.no_store()),
                }
            } else {
                Dispatch::response(
                    sessions
                        .http
                        .handle_read_authenticated(request, &snapshot.config),
                )
            };
        }
        if is_routine_route(&request.path) {
            if let Err(error) = self.preflight(request, now) {
                return Dispatch::response(error);
            }
            let response = self
                .routines
                .as_ref()
                .and_then(|routines| routines.handle_authenticated(request, at))
                .unwrap_or_else(|| {
                    Response::error(
                        503,
                        "routine_unavailable",
                        "routine service is not initialized",
                    )
                });
            return Dispatch::response(response);
        }
        if crate::files::ROUTES.contains(&request.path.as_str()) {
            if let Err(error) = self.preflight(request, now) {
                return Dispatch::response(error);
            }
            let (Some(files), Some(core), Some(workspace)) =
                (&self.files, &self.core, &self.workspace)
            else {
                return Dispatch::response(Response::error(
                    503,
                    "files_unavailable",
                    "file services are not initialized",
                ));
            };
            if let Some(response) = files
                .handle(
                    request,
                    core.as_ref(),
                    self.storage.as_deref(),
                    auth::logically_remote(request),
                    workspace.as_ref(),
                )
                .await
            {
                return Dispatch::response(response);
            }
        }
        self.handle_with_time(request, now, at)
    }
    pub fn handle(&self, request: &Request, now: i64) -> Dispatch {
        self.handle_with_time(
            request,
            now,
            crate::proto::time::UNIX_EPOCH + std::time::Duration::from_secs(now.max(0) as u64),
        )
    }
    fn handle_with_time(
        &self,
        request: &Request,
        now: i64,
        at: crate::proto::time::Timestamp,
    ) -> Dispatch {
        if let Some(response) = provider_early_response(request) {
            return Dispatch::response(response);
        }
        if assets::is_static(&request.path) {
            return Dispatch::response(self.assets.as_ref().map_or_else(
                || assets::static_asset(request),
                |source| source.static_response(request),
            ));
        }
        let snapshot = match self.config.snapshot() {
            Ok(s) => s,
            Err(_) => {
                return Dispatch::response(Response::error(
                    500,
                    "internal",
                    "configuration unavailable",
                ));
            }
        };
        let config = &snapshot.config;
        let port = i64::from(self.bound_port);
        if let Some(error) = session_prefix_error(request, config) {
            return Dispatch::response(error);
        }
        let response = if request.path == "/"
            || (!request.path.starts_with("/api/")
                && request.path != "/ws"
                && !request.path.starts_with("/approval-patterns/"))
        {
            if request.method == "GET"
                && auth::token_or_trusted(request, config)
                && auth::require_host(request, config, port).is_ok()
            {
                self.note_authenticated_remote(request, "page", at);
            }
            assets::index_with_source(
                request,
                config,
                port,
                &self.auth,
                now,
                self.assets.as_deref(),
            )
        } else if request.path.starts_with("/api/approval-action/") {
            // One-tap action tokens do not authenticate with the Hub token. Keep
            // this dispatcher isolated until its consuming core action is wired.
            Response::error(
                501,
                "migration_unimplemented",
                "one-tap approval service is not implemented",
            )
        } else {
            let base = matches!(
                request.path.as_str(),
                "/api/auth/status" | "/api/auth/login" | "/api/auth/logout"
            );
            let methods = guard_methods(&request.path);
            let guard = if base {
                auth::guard_base(request, config, port, methods)
            } else {
                self.auth.guard(request, config, port, methods, now)
            };
            if let Err(error) = guard {
                return Dispatch::response(error);
            }
            match request.path.as_str() {
                p if super::icon_routes::IconHttp::methods(p).is_some() => match &self.icons {
                    Some(owner) => owner
                        .handle_authenticated(request)
                        .expect("recognized icon route"),
                    None => Response::error(
                        503,
                        "provider_registry_unavailable",
                        "provider registry is unavailable",
                    ),
                },
                p if super::distribution_routes::DistributionHttp::methods(p).is_some() => {
                    match &self.distributions {
                        Some(owner) => owner
                            .handle_authenticated(request)
                            .expect("recognized distribution route"),
                        None => Response::error(
                            503,
                            "distribution_store_unavailable",
                            "distribution store is unavailable",
                        ),
                    }
                }
                path if super::push_routes::methods(path).is_some() => match &self.push {
                    Some(owner) => owner
                        .handle_authenticated(request, at)
                        .expect("recognized push route"),
                    None => super::push_routes::PushHttp::new(None)
                        .handle_authenticated(request, at)
                        .expect("recognized push route"),
                },
                "/api/net-hint" => match &self.runtime {
                    Some((hints, _)) => hints.receive(request),
                    None => Response::error(
                        503,
                        "runtime_unavailable",
                        "runtime observations unavailable",
                    ),
                },
                "/api/encoding-check" => match &self.runtime {
                    Some((_, shell)) => super::runtime_routes::encoding(shell),
                    None => Response::error(
                        503,
                        "runtime_unavailable",
                        "runtime observations unavailable",
                    ),
                },
                super::preferences::PATH => {
                    let response = super::preferences::handle_authenticated(
                        request,
                        &self.config,
                        &self.paths,
                        || {
                            self.push_presence
                                .as_ref()
                                .is_some_and(|presence| presence())
                        },
                    )
                    .expect("recognized preferences path");
                    if request.method == "PUT"
                        && let Ok(current) = self.config.snapshot()
                        && current.revision != snapshot.revision
                        && let Some(published) = &self.settings_published
                    {
                        published(super::preferences::PATH, &current.config);
                    }
                    response
                }
                path if super::preference_media::is_route(path) => {
                    super::preference_media::handle_authenticated(
                        request,
                        &self.config,
                        &self.paths,
                        |detail| {
                            crate::logging::write_diagnostic(&format!(
                                "Hub preference media: {detail}\n"
                            ));
                        },
                    )
                    .expect("recognized media path")
                }
                p if p == "/api/providers" || p.starts_with("/api/providers/") => {
                    if p == "/api/providers/" {
                        Response::error(404, "not_found", "provider endpoint not found")
                    } else if let Some(providers) = &self.providers {
                        super::provider_routes::handle(
                            request,
                            &providers.store,
                            &self.paths,
                            providers.command_found.as_ref(),
                            providers.warning.as_ref(),
                        )
                        .unwrap_or_else(|| {
                            Response::error(
                                501,
                                "migration_unimplemented",
                                "provider operation is not implemented",
                            )
                        })
                    } else {
                        Response::error(
                            503,
                            "provider_registry_unavailable",
                            "provider registry is unavailable",
                        )
                    }
                }
                p if p == "/api/memos" || p.starts_with("/api/memos/") => match &self.memos {
                    Some(memos) => memos.handle_memos(
                        request,
                        |id| {
                            if id == 0 {
                                return String::new();
                            }
                            self.core
                                .as_ref()
                                .and_then(|core| core.snapshot(LiveSessionId(id)))
                                .map(|session| {
                                    let ceiling = if self.paths.is_trial() {
                                        Some(self.paths.root())
                                    } else {
                                        self.paths.root().parent()
                                    };
                                    crate::files::scope::find_git_root_with_home(
                                        std::path::Path::new(&session.cwd),
                                        ceiling,
                                    )
                                    .to_string_lossy()
                                    .into_owned()
                                })
                                .unwrap_or_default()
                        },
                        at,
                    ),
                    None => Response::error(
                        503,
                        "memo_store_unavailable",
                        "memo storage is unavailable",
                    ),
                },
                p if p == "/api/memo-images" || p.starts_with("/api/memo-images/") => {
                    match &self.memos {
                        Some(memos) => memos.handle_images(request),
                        None => Response::error(
                            503,
                            "memo_store_unavailable",
                            "memo storage is unavailable",
                        ),
                    }
                }
                "/api/auth/status" => self.auth.status_at(request, config, at),
                "/api/auth/login" => {
                    let response = self.auth.login_at(request, config, at);
                    if response.status == 200 {
                        self.note_authenticated_remote(request, "pin_login", at);
                    }
                    response
                }
                "/api/auth/logout" => self.auth.logout(request, config),
                "/api/auth/set-pin" => self.auth.set_pin(request, &self.config, now),
                "/api/auth/revoke-all" => {
                    let Some(core) = &self.core else {
                        return Dispatch::response(Response::error(
                            503,
                            "core_unavailable",
                            "session core not initialized",
                        ));
                    };
                    match self.auth.rotate(request, &self.config, port, now) {
                        Ok(response) => {
                            return Dispatch {
                                response,
                                effects: core.invalidate_all_ui(),
                            };
                        }
                        Err(e) => e,
                    }
                }
                "/api/notify-generate-topic" => match random_hex(12) {
                    Ok(topic) => Response::json(200, &json!({"topic":format!("anyaicli-{topic}")})),
                    Err(_) => {
                        Response::error(500, "generate_failed", "generate random topic failed")
                    }
                },
                p if settings::PATHS.contains(&p) => {
                    settings::handle_with_published(request, &self.config, &self.paths, &|config| {
                        if let Some(published) = &self.settings_published {
                            published(&request.path, config);
                        }
                    })
                }
                _ => Response::error(
                    501,
                    "migration_unimplemented",
                    "service operation is not implemented",
                ),
            }
        };
        Dispatch::response(response)
    }
}
fn validate_bound_port(paths: &RuntimePaths, port: u16) -> std::io::Result<()> {
    if port == 0 || (paths.is_trial() && port != paths.port()) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "bound port must match explicit trial isolation; production uses the selected listener port",
        ));
    }
    Ok(())
}
pub(crate) fn needs_owned_request(path: &str) -> bool {
    super::diagnostic_routes::DiagnosticHttp::methods(path).is_some()
        || super::agent_history_routes::methods(path).is_some()
        || super::approval_pattern_routes::is_route(path)
        || super::approval_status_routes::methods(path).is_some()
        || super::mobile_routes::methods(path).is_some()
        || super::nvidia_routes::NvidiaHttp::methods(path).is_some()
        || path == "/api/kill-all"
        || path == super::cli_version::PATH
        || path == "/api/models"
        || super::host_routes::methods(path).is_some()
        || super::server_routes::methods(path).is_some()
        || super::slash_routes::is_route(path)
        || super::whisper_routes::WhisperHttp::methods(path).is_some()
        || path == super::voice_routes::PATH
        || super::subscription_routes::methods(path).is_some()
        || super::usage_routes::is_route(path)
        || super::update_routes::methods(path).is_some()
        || path == super::jev_routes::PATH
        || super::session_routes::MAINTENANCE_ROUTES.contains(&path)
        || super::session_routes::LOG_MAINTENANCE_ROUTES.contains(&path)
        || matches!(
            path,
            "/api/usage-link-defaults" | "/api/install-link-defaults"
        )
        || path == "/api/spawn"
        || path == "/api/spawn-grid"
        || path == "/api/notify-test"
        || path == "/api/shutdown"
        || is_routine_route(path)
        || path.starts_with(super::approval_actions::ONE_TAP_PREFIX)
        || path == "/api/approval/batch"
        || super::confirmations::is_confirmation_route(path)
        || super::child_routes::is_spawn_route(path)
        || super::child_control::is_control_route(path)
        || super::relay_routes::is_relay_route(path)
        || path == super::handoff_routes::PATH
        || path.starts_with(super::handoff_routes::PREFIX)
        || super::session_routes::is_metadata_route(path)
}
fn is_routine_route(path: &str) -> bool {
    matches!(path, "/api/routines" | "/api/routine-runs")
        || path.starts_with("/api/routines/")
        || path.starts_with("/api/routine-runs/")
}
/// The fixed Go prefix dispatcher decides unknown path/method results before
/// entering endpoint guards. Keep recognized but unfinished operations routed
/// to their explicit unavailable implementation, rather than returning success.
fn provider_early_response(request: &Request) -> Option<Response> {
    let path = request.path.strip_prefix("/api/providers/")?;
    let parts: Vec<_> = path.split('/').collect();
    let recognized = path == "validate"
        || (parts.len() == 2
            && matches!(
                parts[1],
                "history" | "reset" | "restore" | "backups" | "recovery"
            ))
        || (parts.len() == 4
            && ((parts[1] == "history" && parts[3] == "diff")
                || (parts[1] == "backups" && matches!(parts[3], "verify" | "restore"))));
    if recognized || path.is_empty() {
        return None;
    }
    if path.contains('/') {
        Some(Response::error(
            404,
            "not_found",
            "provider endpoint not found",
        ))
    } else if !matches!(request.method.as_str(), "GET" | "PATCH" | "DELETE") {
        Some(Response::error(
            405,
            "method_not_allowed",
            "method not allowed",
        ))
    } else {
        None
    }
}
fn guard_methods(path: &str) -> &'static [&'static str] {
    match path {
        p if super::approval_status_routes::methods(p).is_some() => {
            super::approval_status_routes::methods(p).unwrap()
        }
        p if p.starts_with("/api/approval-patterns/") => &[],
        p if super::agent_history_routes::methods(p).is_some() => {
            super::agent_history_routes::methods(p).unwrap()
        }
        p if super::approval_pattern_routes::is_route(p) => {
            super::approval_pattern_routes::methods(p)
        }
        p if super::nvidia_routes::NvidiaHttp::methods(p).is_some() => {
            super::nvidia_routes::NvidiaHttp::methods(p).unwrap()
        }
        p if super::mobile_routes::methods(p).is_some() => {
            super::mobile_routes::methods(p).unwrap()
        }
        "/api/kill-all" => &["POST"],
        p if super::diagnostic_routes::DiagnosticHttp::methods(p).is_some() => {
            super::diagnostic_routes::DiagnosticHttp::methods(p).unwrap()
        }
        p if super::icon_routes::IconHttp::methods(p).is_some() => {
            super::icon_routes::IconHttp::methods(p).unwrap()
        }
        p if super::distribution_routes::DistributionHttp::methods(p).is_some() => {
            super::distribution_routes::DistributionHttp::methods(p).unwrap()
        }
        p if p.starts_with("/api/providers/") && p.ends_with("/recovery") => &["GET", "POST"],
        p if p.starts_with("/api/providers/")
            && (p.ends_with("/backups") || p.ends_with("/verify")) =>
        {
            &["GET"]
        }
        p if p.starts_with("/api/providers/") && p.ends_with("/diff") => &["GET"],
        p if p.starts_with("/api/providers/")
            && (p.ends_with("/reset") || p.ends_with("/restore")) =>
        {
            &["POST"]
        }
        p if super::session_routes::MAINTENANCE_ROUTES.contains(&p) => &["POST"],
        "/api/logs/purge" => &["POST"],
        "/api/logs/legacy-notice" => &["GET", "POST"],
        p if super::push_routes::methods(p).is_some() => super::push_routes::methods(p).unwrap(),
        super::jev_routes::PATH => &["POST"],
        "/api/usage-link-defaults" | "/api/install-link-defaults" => &["GET"],
        "/api/net-hint" => &["POST"],
        "/api/encoding-check" => &["GET"],
        super::usage_routes::PATH => &["POST"],
        p if super::update_routes::methods(p).is_some() => {
            super::update_routes::methods(p).unwrap()
        }
        p if super::subscription_routes::methods(p).is_some() => {
            super::subscription_routes::methods(p).unwrap()
        }
        super::voice_routes::PATH => &["POST"],
        p if super::whisper_routes::WhisperHttp::methods(p).is_some() => {
            super::whisper_routes::WhisperHttp::methods(p).unwrap()
        }
        p if super::slash_routes::is_route(p) => &["GET", "POST"],
        p if super::server_routes::methods(p).is_some() => {
            super::server_routes::methods(p).unwrap()
        }
        p if super::host_routes::methods(p).is_some() => super::host_routes::methods(p).unwrap(),
        super::auto_approval::STATUS | super::auto_approval::SIMULATE => &["GET"],
        p if p == super::handoff_routes::PATH || p.starts_with(super::handoff_routes::PREFIX) => {
            super::handoff_routes::methods(p)
        }
        super::cli_version::PATH => &["GET", "POST"],
        "/api/models" => &["GET", "POST"],
        "/api/spawn" => &["POST"],
        "/api/spawn-grid" => &["POST"],
        "/api/idle-timeout" => &["GET", "POST"],
        "/api/notify-config" => &["GET", "POST"],
        "/api/notify-test" => &["POST"],
        "/api/shutdown" => &["POST"],
        super::preferences::PATH => &["GET", "PUT"],
        super::preference_media::AVATAR_PATH => &["GET"],
        super::preference_media::AVATAR_UPLOAD_PATH => &["PUT", "DELETE"],
        super::preference_media::SOUND_PATH => &["GET", "PUT"],
        "/api/providers" => &["GET", "POST"],
        "/api/providers/validate" => &["POST"],
        p if p.starts_with("/api/providers/") && p.ends_with("/history") => &["GET"],
        p if p.starts_with("/api/providers/")
            && !p[15..].contains('/')
            && p != "/api/providers/" =>
        {
            &["GET", "PATCH", "DELETE"]
        }
        "/api/info" => &["GET"],
        p if super::session_routes::READ_ROUTES.contains(&p) => &["GET"],
        p if super::session_routes::is_metadata_route(p) => &["PATCH"],
        "/api/approval/batch" => &["POST"],
        p if super::confirmations::is_confirmation_route(p) => &["POST"],
        p if super::child_routes::is_spawn_route(p) => &["POST"],
        p if super::relay_routes::is_relay_route(p) => super::relay_routes::methods(p),
        p if super::child_control::is_control_route(p) => {
            if p.trim_end_matches('/').ends_with("/children") {
                &["GET"]
            } else {
                &["POST"]
            }
        }
        p if p == "/api/routines" || p.starts_with("/api/routines/") => {
            &["GET", "POST", "PUT", "DELETE"]
        }
        p if p == "/api/routine-runs" || p.starts_with("/api/routine-runs/") => &["GET"],
        p if p == "/api/memos" || p.starts_with("/api/memos/") => {
            &["GET", "POST", "PATCH", "DELETE"]
        }
        p if p == "/api/memo-images" || p.starts_with("/api/memo-images/") => &["GET", "POST"],
        p if crate::files::ROUTES.contains(&p) => {
            if matches!(
                p,
                "/api/files-list"
                    | "/api/files-content"
                    | "/api/files-asset"
                    | "/api/files-download"
                    | "/api/files-roots"
                    | "/api/git-log"
                    | "/api/git-show"
                    | "/api/git-refs"
                    | "/api/git-status"
                    | "/api/git-diff"
                    | "/api/git-turns"
                    | "/api/git-turn-diff"
            ) {
                &["GET"]
            } else {
                &["POST"]
            }
        }
        "/api/auth/status" => &["GET"],
        "/api/auth/login"
        | "/api/auth/logout"
        | "/api/auth/set-pin"
        | "/api/auth/revoke-all"
        | "/api/notify-generate-topic" => &["POST"],
        p if settings::PATHS.contains(&p) => &["GET", "POST"],
        _ => &[],
    }
}

fn session_prefix_error(request: &Request, config: &crate::config::Config) -> Option<Response> {
    let rest = request.path.strip_prefix("/api/sessions/")?;
    if !auth::token_or_trusted(request, config) {
        return Some(Response::error(401, "unauthorized", "unauthorized"));
    }
    if let Err(error) = super::confirmations::parent_path(&request.path) {
        return Some(error);
    }
    let suffix = rest.trim_matches('/').rsplit('/').next().unwrap_or("");
    if !matches!(
        suffix,
        "meta"
            | "spawn-child"
            | "spawn-confirm"
            | "send-child"
            | "inject"
            | "children"
            | "relay"
            | "relay-stop"
            | "relay-resume"
            | "relay-cleanup"
    ) {
        return Some(Response::error(404, "not_found", "not found"));
    }
    None
}

#[cfg(test)]
mod recovery_prefix_tests {
    use super::*;
    #[test]
    fn unknown_session_suffix_keeps_token_shape_id_precedence_before_host_and_method() {
        let config = crate::config::Config {
            token: "synthetic".into(),
            ..Default::default()
        };
        let mut request = Request {
            method: "TRACE".into(),
            path: "/api/sessions/no-id/unknown".into(),
            host: "outside.invalid".into(),
            ..Default::default()
        };
        assert_eq!(session_prefix_error(&request, &config).unwrap().status, 401);
        request.query = "token=synthetic".into();
        assert_eq!(session_prefix_error(&request, &config).unwrap().status, 400);
        request.path = "/api/sessions/1/unknown/extra".into();
        assert_eq!(session_prefix_error(&request, &config).unwrap().status, 404);
        request.path = "/api/sessions/1/unknown".into();
        assert_eq!(session_prefix_error(&request, &config).unwrap().status, 404);
        request.path = "/api/sessions/1/relay".into();
        assert!(session_prefix_error(&request, &config).is_none());
    }
}
