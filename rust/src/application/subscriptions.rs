//! Fixed Go subscription profile services. The config store is the single
//! registry; CLI status and usage contain no credential/account identity fields.
pub mod appserver;
pub mod cli;
pub mod local;
pub mod probe;
pub mod usage;
use crate::{
    config::{self, Config, ConfigError, ConfigStore, Resource, RuntimePaths, SubscriptionProfile},
    files::safe_fs::Dir,
    hub::task_owner::HubTaskHandle,
    profile::subscriptions::SubscriptionLauncher,
    proto::{core::*, time::Timestamp},
    terminal::session::SessionEngine,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
pub struct ServiceError {
    pub status: u16,
    pub code: &'static str,
    pub detail: String,
}
impl ServiceError {
    fn new(status: u16, code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            status,
            code,
            detail: detail.into(),
        }
    }
    fn bad(detail: impl Into<String>) -> Self {
        Self::new(400, "bad_request", detail)
    }
}
pub struct Resolved {
    pub provider: String,
    pub id: String,
    pub profile: SubscriptionProfile,
    pub path: PathBuf,
}
pub struct SubscriptionDependencies {
    pub paths: RuntimePaths,
    pub home: PathBuf,
    pub config: Arc<ConfigStore>,
    pub launcher: Arc<SubscriptionLauncher>,
    pub core: Arc<SessionEngine>,
    pub cli: Arc<dyn appserver::SubscriptionUsageCli>,
    pub environment: Vec<String>,
    pub tasks: HubTaskHandle,
    pub warning: Arc<dyn Fn(&'static str) + Send + Sync>,
    pub probe_hooks: Arc<dyn probe::SubscriptionProbeHooks>,
}
pub struct SubscriptionService {
    pub(super) usage: Arc<usage::UsageStore>,
    probes: Arc<probe::ProbeManager>,
    pub(super) probe_hooks: Arc<dyn probe::SubscriptionProbeHooks>,
    pub paths: RuntimePaths,
    pub(super) home: PathBuf,
    pub config: Arc<ConfigStore>,
    pub(super) launcher: Arc<SubscriptionLauncher>,
    pub core: Arc<SessionEngine>,
    pub(super) cli: Arc<dyn appserver::SubscriptionUsageCli>,
    pub(super) environment: Vec<String>,
    pub(super) tasks: HubTaskHandle,
    pub(super) warning: Arc<dyn Fn(&'static str) + Send + Sync>,
}
impl SubscriptionService {
    pub(crate) fn effect_permit(
        &self,
    ) -> Result<crate::hub::task_owner::OwnedTaskPermit, SessionError> {
        self.tasks.effect_permit()
    }
    pub fn new(deps: SubscriptionDependencies) -> Arc<Self> {
        Arc::new(Self {
            usage: Arc::new(usage::UsageStore::default()),
            probes: Arc::new(probe::ProbeManager::default()),
            probe_hooks: deps.probe_hooks,
            paths: deps.paths,
            home: deps.home,
            config: deps.config,
            launcher: deps.launcher,
            core: deps.core,
            cli: deps.cli,
            environment: deps.environment,
            tasks: deps.tasks,
            warning: deps.warning,
        })
    }
    pub fn resolve(&self, provider: &str, raw: &str) -> Result<Option<Resolved>, ServiceError> {
        let id = config::normalize_subscription_id(raw);
        if id.is_empty() {
            return Ok(None);
        }
        if cli::env_key(provider).is_none() {
            return Err(ServiceError::new(
                400,
                "invalid_subscription",
                format!("provider does not support subscription profiles: {provider}"),
            ));
        }
        let cfg = self.config.snapshot().map_err(save_error)?.config;
        let profile =
            config::find_subscription(&cfg.subscriptions, provider, &id).ok_or_else(|| {
                ServiceError::new(
                    404,
                    "invalid_subscription",
                    format!("subscription profile not found: {provider}/{id}"),
                )
            })?;
        if !profile.is_enabled() {
            return Err(ServiceError::new(
                400,
                "invalid_subscription",
                format!("subscription profile is disabled: {provider}/{id}"),
            ));
        }
        let path = config::resolve_subscription_profile_dir(
            &self.paths,
            provider,
            &profile,
            Some(&self.home),
        )
        .map_err(|e| ServiceError::new(400, "invalid_subscription", e.to_string()))?;
        Ok(Some(Resolved {
            provider: provider.into(),
            id,
            profile,
            path,
        }))
    }
    pub fn list(&self) -> Result<Value, ServiceError> {
        let cfg = self.config.snapshot().map_err(save_error)?.config;
        Ok(
            json!({"providers":self.provider_list(&cfg),"root":self.paths.resource(Resource::Subscriptions)}),
        )
    }
    fn provider_list(&self, cfg: &Config) -> Vec<Value> {
        let mut providers: Vec<String> = cli::PROVIDERS.iter().map(|v| (*v).into()).collect();
        providers.extend(
            cfg.subscriptions
                .keys()
                .filter(|v| !cli::PROVIDERS.contains(&v.as_str()))
                .cloned(),
        );
        providers.into_iter().map(|provider|{let env=cli::env_key(&provider);let mut section=json!({"provider":provider,"supported":env.is_some(),"profiles":[]});if let Some(env)=env{section["env_var"]=env.into();}let mut seen=BTreeSet::new();let mut entries=vec![];
   for profile in cfg.subscriptions.get(&provider).into_iter().flatten(){let id=config::normalize_subscription_id(&profile.id);let mut row=json!({"provider":provider,"id":id,"enabled":profile.is_enabled(),"exists":false});if !profile.name.is_empty(){row["name"]=profile.name.clone().into();}
   if !profile.plan.is_empty(){row["plan"]=profile.plan.clone().into();}
    let issue=if let Err(error)=config::validate_subscription_provider(&provider){Some(error.to_string())}else if let Err(error)=config::validate_subscription_id(&id){row["id"]=profile.id.clone().into();Some(error.to_string())}else if !seen.insert(id.clone()){Some(format!("duplicate profile id {}",crate::proto::go_quote::quote(&id)))}else{match config::resolve_subscription_profile_dir(&self.paths,&provider,profile,Some(&self.home)){Ok(path)=>{row["profile_dir"]=path.to_string_lossy().into_owned().into();row["exists"]=path.is_dir().into();None},Err(error)=>Some(error.to_string())}};
    if let Some(issue)=issue{row["issue"]=issue.into();}entries.push(row);
   }section["profiles"]=entries.into();section}).collect()
    }
    pub fn add(&self, provider: &str, raw_id: &str, name: &str) -> Result<Value, ServiceError> {
        let provider = provider.trim();
        if cli::env_key(provider).is_none() {
            return Err(ServiceError::bad(
                "provider does not support subscription profiles",
            ));
        }
        let name = display_name(name);
        let reserved = loop {
            let mut snapshot = self.config.snapshot().map_err(save_error)?;
            let list = snapshot
                .config
                .subscriptions
                .get(provider)
                .cloned()
                .unwrap_or_default();
            let existing: BTreeSet<_> = list
                .iter()
                .map(|profile| config::normalize_subscription_id(&profile.id))
                .collect();
            let used: BTreeSet<_> = list.iter().map(SubscriptionProfile::dir_name).collect();
            let raw = config::normalize_subscription_id(raw_id);
            let id = if raw.is_empty() {
                unique_id(&slug(&name), &existing)
            } else {
                raw
            };
            config::validate_subscription_id(&id).map_err(|e| ServiceError::bad(e.to_string()))?;
            if existing.contains(&id) {
                return Err(ServiceError::new(
                    409,
                    "duplicate_id",
                    format!(
                        "subscription id {} already exists for {provider}",
                        crate::proto::go_quote::quote(&id)
                    ),
                ));
            }
            let profile = SubscriptionProfile {
                id,
                name: name.clone(),
                dir: next_dir(
                    &self.paths.resource(Resource::Subscriptions).join(provider),
                    &used,
                ),
                ..Default::default()
            };
            config::resolve_subscription_profile_dir(
                &self.paths,
                provider,
                &profile,
                Some(&self.home),
            )
            .map_err(|e| ServiceError::bad(e.to_string()))?;
            snapshot
                .config
                .subscriptions
                .entry(provider.into())
                .or_default()
                .push(profile.clone());
            match self
                .config
                .publish_legacy_without_persist(snapshot.revision, snapshot.config)
            {
                Ok(_) => break profile,
                Err(ConfigError::Conflict { .. }) => continue,
                Err(error) => return Err(save_error(error)),
            }
        };
        if let Err(error) =
            self.launcher
                .seed_existing_profile(provider, &reserved, &self.environment)
        {
            self.rollback_add(provider, &reserved);
            return Err(ServiceError::new(
                500,
                "profile_dir_error",
                format!("create profile dir: {error}"),
            ));
        }
        if let Err(error) = self.persist_current() {
            self.rollback_add(provider, &reserved);
            return Err(error);
        }
        self.list()
    }
    fn rollback_add(&self, provider: &str, reserved: &SubscriptionProfile) {
        loop {
            let Ok(mut snapshot) = self.config.snapshot() else {
                (self.warning)("subscription rollback failed");
                return;
            };
            let Some(list) = snapshot.config.subscriptions.get_mut(provider) else {
                return;
            };
            list.retain(|profile| {
                !(config::normalize_subscription_id(&profile.id)
                    == config::normalize_subscription_id(&reserved.id)
                    && profile.dir_name() == reserved.dir_name())
            });
            if list.is_empty() {
                snapshot.config.subscriptions.remove(provider);
            }
            match self
                .config
                .publish_legacy_without_persist(snapshot.revision, snapshot.config)
            {
                Ok(_) => return,
                Err(ConfigError::Conflict { .. }) => continue,
                Err(_) => {
                    (self.warning)("subscription rollback failed");
                    return;
                }
            }
        }
    }
    fn persist_current(&self) -> Result<(), ServiceError> {
        loop {
            let snapshot = self.config.snapshot().map_err(save_error)?;
            match self.config.persist_published_legacy(snapshot.revision) {
                Ok(_) => return Ok(()),
                Err(ConfigError::Conflict { .. }) => continue,
                Err(error) => return Err(save_error(error)),
            }
        }
    }
    pub fn update(
        &self,
        provider: &str,
        raw: &str,
        name: Option<&str>,
        enabled: Option<bool>,
    ) -> Result<Value, ServiceError> {
        let provider = provider.trim();
        let id = config::normalize_subscription_id(raw);
        config::validate_subscription_id(&id).map_err(|e| ServiceError::bad(e.to_string()))?;
        loop {
            let mut snapshot = self.config.snapshot().map_err(save_error)?;
            let profile = snapshot
                .config
                .subscriptions
                .get_mut(provider)
                .and_then(|list| {
                    list.iter_mut()
                        .find(|p| config::normalize_subscription_id(&p.id) == id)
                })
                .ok_or_else(|| {
                    ServiceError::new(404, "not_found", "subscription profile not found")
                })?;
            if let Some(name) = name {
                profile.name = display_name(name);
            }
            if let Some(enabled) = enabled {
                profile.enabled = Some(enabled);
            }
            match self
                .config
                .publish_legacy_without_persist(snapshot.revision, snapshot.config)
            {
                Ok(_) => break,
                Err(ConfigError::Conflict { .. }) => continue,
                Err(error) => return Err(save_error(error)),
            }
        }
        self.persist_current()?;
        self.list()
    }
    pub fn remove(
        &self,
        provider: &str,
        raw: &str,
        delete_credentials: bool,
    ) -> Result<Value, ServiceError> {
        let provider = provider.trim();
        let id = config::normalize_subscription_id(raw);
        config::validate_subscription_id(&id).map_err(|e| ServiceError::bad(e.to_string()))?;
        if delete_credentials
            && self.core.snapshots().iter().any(|s| {
                !terminal_state(&s.state)
                    && crate::proto::unicode::simple_fold_key(s.provider.trim())
                        == crate::proto::unicode::simple_fold_key(provider)
                    && config::normalize_subscription_id(&s.subscription_profile_id) == id
            })
        {
            return Err(ServiceError::new(
                409,
                "profile_in_use",
                "cannot delete credentials while the subscription profile is used by a live session",
            ));
        }
        let removed = loop {
            let mut snapshot = self.config.snapshot().map_err(save_error)?;
            let list = snapshot
                .config
                .subscriptions
                .get_mut(provider)
                .ok_or_else(|| {
                    ServiceError::new(404, "not_found", "subscription profile not found")
                })?;
            let index = list
                .iter()
                .position(|p| config::normalize_subscription_id(&p.id) == id)
                .ok_or_else(|| {
                    ServiceError::new(404, "not_found", "subscription profile not found")
                })?;
            let removed = list.remove(index);
            if list.is_empty() {
                snapshot.config.subscriptions.remove(provider);
            }
            match self
                .config
                .publish_legacy_without_persist(snapshot.revision, snapshot.config)
            {
                Ok(_) => break removed,
                Err(ConfigError::Conflict { .. }) => continue,
                Err(error) => return Err(save_error(error)),
            }
        };
        self.persist_current()?;
        let deleted = if delete_credentials && removed.profile_dir.trim().is_empty() {
            match config::resolve_subscription_profile_dir(
                &self.paths,
                provider,
                &removed,
                Some(&self.home),
            ) {
                Ok(path) => {
                    if crate::profile::subscriptions::check_path(&self.paths, &path).is_err() {
                        (self.warning)("subscription profile directory not deleted");
                        false
                    } else {
                        match std::fs::remove_dir_all(&path) {
                            Ok(()) => true,
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
                            Err(_) => {
                                (self.warning)("subscription profile directory delete failed");
                                false
                            }
                        }
                    }
                }
                Err(_) => {
                    (self.warning)("subscription profile directory not deleted");
                    false
                }
            }
        } else {
            false
        };
        let mut response = self.list()?;
        response["credentials_deleted"] = deleted.into();
        Ok(response)
    }
    pub async fn test(
        &self,
        provider: &str,
        raw: &str,
        cancel: &TaskCancellation,
    ) -> Result<Value, ServiceError> {
        let provider = provider.trim();
        if cli::env_key(provider).is_none() {
            return Err(ServiceError::bad(
                "provider does not support subscription profiles",
            ));
        }
        let resolved = self
            .resolve(provider, raw)?
            .ok_or_else(|| ServiceError::bad("subscription id is required"))?;
        let status = match self
            .cli
            .status(provider, &resolved.path, Duration::from_secs(20), cancel)
            .await
        {
            Ok(status) => status,
            Err(error) => {
                self.usage
                    .set_auth(provider, &resolved.id, "status_unknown", Timestamp::now());
                return Err(ServiceError::new(502, "status_failed", error.to_string()));
            }
        };
        self.usage.set_auth(
            provider,
            &resolved.id,
            if status.logged_in {
                "ready"
            } else {
                "login_required"
            },
            Timestamp::now(),
        );
        if status.logged_in && !status.plan.trim().is_empty() {
            loop {
                let Ok(mut snapshot) = self.config.snapshot() else {
                    break;
                };
                if let Some(profile) =
                    snapshot
                        .config
                        .subscriptions
                        .get_mut(provider)
                        .and_then(|profiles| {
                            profiles
                                .iter_mut()
                                .find(|p| config::normalize_subscription_id(&p.id) == resolved.id)
                        })
                {
                    profile.plan = status.plan.clone();
                }
                match self
                    .config
                    .publish_legacy_without_persist(snapshot.revision, snapshot.config)
                {
                    Ok(_) => {
                        if self.persist_current().is_err() {
                            (self.warning)("failed to persist subscription plan");
                        }
                        break;
                    }
                    Err(ConfigError::Conflict { .. }) => continue,
                    Err(_) => {
                        (self.warning)("failed to persist subscription plan");
                        break;
                    }
                }
            }
        }
        serde_json::to_value(status)
            .map_err(|_| ServiceError::new(500, "internal", "cannot serialize status"))
    }
    pub async fn login(
        &self,
        provider: &str,
        raw: &str,
        cancel: &TaskCancellation,
    ) -> Result<Value, ServiceError> {
        let provider = provider.trim();
        if cli::env_key(provider).is_none() {
            return Err(ServiceError::bad(
                "provider does not support subscription profiles",
            ));
        }
        let resolved = self
            .resolve(provider, raw)?
            .ok_or_else(|| ServiceError::bad("subscription id is required"))?;
        let cwd = self.paths.root().join("subscription-login");
        Dir::open_or_create_private(&cwd).map_err(|e| {
            ServiceError::new(500, "login_cwd", format!("login workspace error: {e}"))
        })?;
        let label = format!("login-{provider}-{}", resolved.id);
        if !crate::profile::validation::valid_spawn_model_label(&label) {
            return Err(ServiceError::bad("invalid subscription id"));
        }
        let spec = WrappedSpawnSpec {
            registration_metadata: Default::default(),
            spawn_attempt: None,
            registration_proof: None,
            provider: provider.into(),
            cwd,
            model: String::new(),
            model_selection: String::new(),
            risk_confirmed: false,
            label,
            permission_mode: String::new(),
            sandbox: String::new(),
            ask_for_approval: String::new(),
            route: String::new(),
            utf8_session: false,
            effort: String::new(),
            execution_mode: String::new(),
            permission_preset: String::new(),
            initial_prompt: String::new(),
            subscription_profile_id: resolved.id.clone(),
            subscription_login: true,
            usage_probe: false,
            grants: Default::default(),
            cancellation: cancel.clone(),
        };
        match self
            .core
            .spawn_and_wait(
                spec,
                Duration::from_secs(20),
                &HttpWaitCancellation::default(),
            )
            .await
        {
            SpawnWaitOutcome::Registered(binding) => {
                self.usage.invalidate_auth(provider, &resolved.id);
                Ok(json!({"ok":true,"session_id":binding.session.0}))
            }
            _ => Err(ServiceError::new(
                500,
                "spawn_error",
                "login session spawn error: wrapper did not register",
            )),
        }
    }
}
fn save_error(error: ConfigError) -> ServiceError {
    ServiceError::new(500, "save_failed", format!("save failed: {error}"))
}
fn terminal_state(state: &str) -> bool {
    matches!(
        state,
        "completed" | "error" | "disconnected" | "done" | "timeout" | "dismissed"
    )
}
fn display_name(name: &str) -> String {
    name.trim()
        .chars()
        .filter(|c| (*c as u32) >= 0x20 && *c != '\u{7f}')
        .take(80)
        .collect()
}
fn slug(name: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in crate::proto::unicode::simple_lower(name).chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
            dash = false;
        } else if matches!(c, '-' | '_' | '.' | ' ') && !out.is_empty() && !dash {
            out.push('-');
            dash = true;
        }
    }
    let out = out.trim_matches('-');
    out[..out.len().min(64)].trim_matches('-').into()
}
fn unique_id(base: &str, existing: &BTreeSet<String>) -> String {
    let base = if base.is_empty() { "profile" } else { base };
    if !existing.contains(base) {
        return base.into();
    }
    for i in 2..1000 {
        let candidate = format!("{base}-{i}");
        if !existing.contains(&candidate) {
            return candidate;
        }
    }
    base.into()
}
fn next_dir(base: &Path, used: &BTreeSet<String>) -> String {
    for index in 1.. {
        let candidate = format!("p{index}");
        if !used.contains(&candidate) && std::fs::symlink_metadata(base.join(&candidate)).is_err() {
            return candidate;
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests;
