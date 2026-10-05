//! Usage observations and secret-free authentication cache. Token/session state
//! stays in SessionEngine; this owner indexes only provider/profile usage.
use super::{
    ServiceError, SubscriptionService,
    appserver::{CodexUsage, Credits, Window},
    local::{self, GrokUsage},
};
use crate::{
    config::{self, Config},
    proto::{
        self,
        core::*,
        time::{self, Timestamp},
    },
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
    time::Duration,
};
#[derive(Default)]
pub struct UsageStore {
    state: Mutex<State>,
}
#[derive(Default)]
struct State {
    entries: BTreeMap<(String, String), Entry>,
    auth: BTreeMap<(String, String), Auth>,
    checking: BTreeSet<(String, String)>,
    refreshing: BTreeSet<String>,
}
#[derive(Default)]
struct Entry {
    claude: Option<Value>,
    codex: Option<CodexUsage>,
    grok: Option<GrokUsage>,
    observed: Option<Timestamp>,
    observed_display: String,
    retrieved: String,
    checked: Option<Timestamp>,
    source: &'static str,
}
struct Auth {
    state: &'static str,
    checked: Timestamp,
}
struct Observation {
    at: Option<Timestamp>,
    display: String,
    fallback: Timestamp,
    source: &'static str,
}
impl UsageStore {
    pub fn set_auth(&self, provider: &str, id: &str, state: &'static str, at: Timestamp) {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .auth
            .insert(
                (provider.into(), config::normalize_subscription_id(id)),
                Auth { state, checked: at },
            );
    }
    pub fn invalidate_auth(&self, provider: &str, id: &str) {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .auth
            .remove(&(provider.into(), config::normalize_subscription_id(id)));
    }
    fn put(&self, provider: &str, id: &str, mut incoming: Entry, observation: Observation) {
        let Observation {
            at: observed,
            display,
            fallback,
            source,
        } = observation;
        let key: (String, String) = (
            provider.trim().into(),
            config::normalize_subscription_id(id),
        );
        if key.0.is_empty() || key.1.is_empty() {
            return;
        }
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(current) = state.entries.get(&key) {
            if provider == "codex" {
                if current.source == "app-server"
                    && observed
                        .zip(current.checked)
                        .is_none_or(|(observed, checked)| observed <= checked)
                {
                    return;
                }
                if !observed.is_some_and(|incoming| {
                    current.observed.is_none_or(|current| incoming > current)
                }) && current.source != "app-server"
                {
                    return;
                }
            } else if source == "local" && current.source == "live" {
                return;
            }
        }
        incoming.observed = observed;
        incoming.observed_display = display.clone();
        incoming.retrieved = if observed.is_some() {
            display
        } else {
            time::format_rfc3339(fallback).unwrap_or_default()
        };
        incoming.source = source;
        state.entries.insert(key, incoming);
    }
    fn app_server(&self, id: &str, usage: CodexUsage, at: Timestamp) {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .entries
            .insert(
                ("codex".into(), config::normalize_subscription_id(id)),
                Entry {
                    codex: Some(usage),
                    checked: Some(at),
                    retrieved: time::format_rfc3339(at).unwrap_or_default(),
                    source: "app-server",
                    ..Default::default()
                },
            );
    }
}
impl SubscriptionService {
    pub fn record_session_usage<'a>(
        &'a self,
        binding: SessionBinding,
        stat: &'a proto::Message,
        record_at: Timestamp,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            let details = self
                .core
                .details(binding.session)
                .ok_or(SessionError::StaleBinding)?;
            if details.binding != binding {
                return Err(SessionError::StaleBinding);
            }
            let snapshot = details.snapshot;
            if snapshot.subscription_profile_id.trim().is_empty() {
                return Ok(());
            }
            let provider = snapshot.provider.as_str();
            let mut entry = Entry::default();
            if provider == "claude" {
                if !(stat.claude_rate_limits_present
                    || stat.rl_5h_pct != 0.0
                    || stat.rl_5h_reset != 0
                    || stat.rl_7d_pct != 0.0
                    || stat.rl_7d_reset != 0)
                {
                    return Ok(());
                }
                let mut usage = json!({});
                if (stat.claude_5h_present || stat.rl_5h_pct != 0.0 || stat.rl_5h_reset != 0)
                    && let Some(window) = window(stat.rl_5h_pct, 0, stat.rl_5h_reset)
                {
                    usage["five_hour"] = serde_json::to_value(window)
                        .map_err(|_| SessionError::InvalidRequest("invalid usage window".into()))?;
                }
                if (stat.claude_7d_present || stat.rl_7d_pct != 0.0 || stat.rl_7d_reset != 0)
                    && let Some(window) = window(stat.rl_7d_pct, 0, stat.rl_7d_reset)
                {
                    usage["seven_day"] = serde_json::to_value(window)
                        .map_err(|_| SessionError::InvalidRequest("invalid usage window".into()))?;
                }
                entry.claude = Some(usage);
            } else if provider == "codex" {
                if !stat.codex_rate_limits_present {
                    return Ok(());
                }
                let mut usage = CodexUsage {
                    plan_type: stat.codex_plan_type.clone(),
                    ..Default::default()
                };
                if stat.codex_primary_present
                    || stat.codex_primary_used_pct != 0.0
                    || stat.codex_primary_window_minutes != 0
                    || stat.codex_primary_reset != 0
                {
                    usage.primary = window(
                        stat.codex_primary_used_pct,
                        stat.codex_primary_window_minutes,
                        stat.codex_primary_reset,
                    );
                }
                if stat.codex_secondary_present
                    || stat.codex_secondary_used_pct != 0.0
                    || stat.codex_secondary_window_minutes != 0
                    || stat.codex_secondary_reset != 0
                {
                    usage.secondary = window(
                        stat.codex_secondary_used_pct,
                        stat.codex_secondary_window_minutes,
                        stat.codex_secondary_reset,
                    );
                }
                if stat.codex_credits_present {
                    usage.credits = Some(Credits {
                        has_credits: stat.codex_has_credits,
                        unlimited: stat.codex_credits_unlimited,
                        balance: stat.codex_credits_balance.clone(),
                    });
                    if stat.codex_has_credits && !stat.codex_credits_unlimited {
                        usage.credits_balance = stat.codex_credits_balance.clone();
                    }
                }
                entry.codex = Some(usage);
            } else {
                return Ok(());
            }
            let observed = if provider == "codex" {
                time::parse_rfc3339(&stat.usage_observed_at).ok()
            } else {
                Some(record_at)
            };
            let display = if provider == "codex" {
                observed_display(&stat.usage_observed_at)
            } else {
                time::format_rfc3339(record_at).unwrap_or_default()
            };
            self.usage.put(
                provider,
                &snapshot.subscription_profile_id,
                entry,
                Observation {
                    at: observed,
                    display,
                    fallback: record_at,
                    source: "live",
                },
            );
            Ok(())
        })
    }
    pub fn usage_snapshot(
        self: &Arc<Self>,
        force_auth: bool,
        now: Timestamp,
    ) -> Result<Value, ServiceError> {
        let cfg = self.config.snapshot().map_err(super::save_error)?.config;
        self.refresh_local(&cfg, now);
        let mut sections = vec![];
        let mut targets = vec![];
        {
            let mut state = self.usage.state.lock().unwrap_or_else(|p| p.into_inner());
            for provider in ["claude", "codex", "grok"] {
                let mut profiles = vec![];
                for profile in cfg.subscriptions.get(provider).into_iter().flatten() {
                    if !profile.is_enabled() {
                        continue;
                    }
                    let id = config::normalize_subscription_id(&profile.id);
                    if config::validate_subscription_id(&id).is_err() {
                        continue;
                    }
                    let key = (provider.into(), id.clone());
                    let name = if profile.name.trim().is_empty() {
                        id.as_str()
                    } else {
                        profile.name.trim()
                    };
                    let mut row = json!({"id":id,"name":name});
                    if !profile.plan.trim().is_empty() {
                        row["plan"] = profile.plan.trim().into();
                    }
                    if provider == "claude" {
                        row["probe_available"] = true.into();
                    }
                    if self.probes.running(provider, &id) {
                        row["probe_state"] = "running".into();
                    }
                    if let Some(entry) = state.entries.get(&key) {
                        row["retrieved_at"] = entry.retrieved.clone().into();
                        if let Some(checked) = entry.checked {
                            row["checked_at"] =
                                time::format_rfc3339(checked).unwrap_or_default().into();
                        }
                        if entry.observed.is_some() {
                            row["observed_at"] = entry.observed_display.clone().into();
                        }
                        if let Some(claude) = &entry.claude {
                            row["claude"] = claude.clone();
                        }
                        if let Some(codex) = &entry.codex {
                            row["codex"] = serde_json::to_value(codex).map_err(|_| {
                                ServiceError::new(500, "internal", "cannot serialize usage")
                            })?;
                        }
                        if let Some(grok) = &entry.grok {
                            row["grok"] = serde_json::to_value(grok).map_err(|_| {
                                ServiceError::new(500, "internal", "cannot serialize usage")
                            })?;
                        }
                    }
                    if force_auth {
                        state.auth.remove(&key);
                    }
                    let cached = state.auth.get(&key);
                    if let Some(cached) = cached {
                        row["auth_status"] = cached.state.into();
                    }
                    let fresh = cached.is_some_and(|cached| {
                        now.unix_nanos() - cached.checked.unix_nanos() < 60_000_000_000
                    });
                    let unknown = cached.is_none();
                    if !fresh {
                        if state.checking.insert(key.clone()) {
                            targets.push(key);
                        }
                        if unknown {
                            row["auth_checking"] = true.into();
                        }
                    }
                    profiles.push(row);
                }
                if !profiles.is_empty() {
                    sections.push(json!({"provider":provider,"profiles":profiles}));
                }
            }
        }
        if !targets.is_empty() {
            self.refresh_auth(cfg, targets)?;
        }
        Ok(json!({"providers":sections}))
    }
    fn refresh_local(&self, cfg: &Config, now: Timestamp) {
        for provider in ["codex", "grok"] {
            for profile in cfg.subscriptions.get(provider).into_iter().flatten() {
                if !profile.is_enabled() {
                    continue;
                }
                let id = config::normalize_subscription_id(&profile.id);
                if config::validate_subscription_id(&id).is_err() {
                    continue;
                }
                let Ok(path) = config::resolve_subscription_profile_dir(
                    &self.paths,
                    provider,
                    profile,
                    Some(&self.home),
                ) else {
                    continue;
                };
                if let Some(local) = local::read_profile(&self.paths, provider, &path) {
                    self.usage.put(
                        provider,
                        &id,
                        Entry {
                            codex: local.codex,
                            grok: local.grok,
                            ..Default::default()
                        },
                        Observation {
                            at: local.observed,
                            display: local.observed_display,
                            fallback: now,
                            source: "local",
                        },
                    );
                }
            }
        }
    }
    fn refresh_auth(
        self: &Arc<Self>,
        cfg: Config,
        targets: Vec<(String, String)>,
    ) -> Result<(), ServiceError> {
        let permit = match self.tasks.effect_permit() {
            Ok(permit) => permit,
            Err(_) => {
                let mut state = self.usage.state.lock().unwrap_or_else(|p| p.into_inner());
                for target in targets {
                    state.checking.remove(&target);
                }
                return Err(ServiceError::new(
                    503,
                    "shutdown",
                    "Hub task admission stopped",
                ));
            }
        };
        let cancel = permit.cancellation();
        let service = self.clone();
        let batch = AuthBatchClaim {
            usage: self.usage.clone(),
            keys: targets.clone(),
        };
        let _waiter = permit.start(async move {
            let _batch = batch;
            let sem = Arc::new(tokio::sync::Semaphore::new(3));
            let cfg = Arc::new(cfg);
            let futures = targets.into_iter().map(|key| {
                let service = service.clone(); let cancel = cancel.clone();
                let sem = sem.clone(); let cfg = cfg.clone();
                async move {
                    let _claim = AuthClaim { usage: service.usage.clone(), key: key.clone() };
                    let _slot = tokio::select! {slot=sem.acquire_owned()=>match slot{Ok(slot)=>slot,Err(_)=>return},_=cancel.token().cancelled()=>return};
                    let mut state = "status_unknown";
                    if let Some(profile) = config::find_subscription(&cfg.subscriptions, &key.0, &key.1).filter(|p| p.is_enabled())
                        && let Ok(path) = config::resolve_subscription_profile_dir(&service.paths, &key.0, &profile, Some(&service.home))
                        && let Ok(status) = service.cli.status(&key.0, &path, Duration::from_secs(2), &cancel).await {
                        state = if status.logged_in { "ready" } else { "login_required" };
                    }
                    service.usage.set_auth(&key.0, &key.1, state, Timestamp::now());
                }
            });
            futures_util::future::join_all(futures).await;
        });
        Ok(())
    }
    pub async fn refresh_codex(
        &self,
        provider: &str,
        raw: &str,
        session_id: i64,
        cancel: &TaskCancellation,
    ) -> Result<Value, ServiceError> {
        if provider.trim() != "codex" {
            return Err(ServiceError::bad(
                "manual refresh supports Codex profiles only",
            ));
        }
        let id = config::normalize_subscription_id(raw);
        config::validate_subscription_id(&id).map_err(|e| ServiceError::bad(e.to_string()))?;
        let resolved = self.resolve("codex", &id).ok().flatten().ok_or_else(|| {
            ServiceError::new(400, "invalid_subscription", "Codex profile is unavailable")
        })?;
        let key = directory_key(&resolved.path);
        if session_id != 0 {
            let matches = self
                .core
                .details(LiveSessionId(session_id))
                .is_some_and(|details| {
                    details.snapshot.provider == "codex"
                        && config::normalize_subscription_id(
                            &details.snapshot.subscription_profile_id,
                        ) == id
                        && directory_key(std::path::Path::new(&details.transcript.codex_home))
                            == key
                });
            if !matches {
                return Err(ServiceError::new(
                    409,
                    "session_profile_changed",
                    "session login directory no longer matches this profile",
                ));
            }
        }
        {
            let mut state = self.usage.state.lock().unwrap_or_else(|p| p.into_inner());
            if !state.refreshing.insert(key.clone()) {
                return Err(ServiceError::new(
                    409,
                    "refresh_running",
                    "usage refresh is already running for this profile",
                ));
            }
        }
        let _claim = RefreshClaim {
            usage: self.usage.clone(),
            key: key.clone(),
        };
        let usage = self
            .cli
            .codex_usage(&resolved.path, cancel)
            .await
            .map_err(|e| {
                ServiceError::new(
                    if e.kind() == std::io::ErrorKind::TimedOut {
                        504
                    } else {
                        502
                    },
                    "usage_refresh_failed",
                    "Codex usage could not be retrieved; previous values were kept",
                )
            })?;
        if self
            .resolve("codex", &id)
            .ok()
            .flatten()
            .is_none_or(|resolved| directory_key(&resolved.path) != key)
        {
            return Err(ServiceError::new(
                409,
                "profile_changed",
                "Codex profile changed during usage refresh; previous values were kept",
            ));
        }
        let checked = Timestamp::now();
        self.usage.app_server(&id, usage, checked);
        Ok(
            json!({"ok":true,"provider":"codex","id":id,"checked_at":time::format_rfc3339(checked).unwrap_or_default()}),
        )
    }
}
struct AuthClaim {
    usage: Arc<UsageStore>,
    key: (String, String),
}
impl Drop for AuthClaim {
    fn drop(&mut self) {
        self.usage
            .state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .checking
            .remove(&self.key);
    }
}
struct RefreshClaim {
    usage: Arc<UsageStore>,
    key: String,
}
impl Drop for RefreshClaim {
    fn drop(&mut self) {
        self.usage
            .state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .refreshing
            .remove(&self.key);
    }
}
fn window(used: f64, minutes: i64, resets: i64) -> Option<Window> {
    if used.is_nan() {
        return None;
    }
    let used = used.clamp(0.0, 100.0);
    Some(Window {
        used_percent: used,
        remaining_percent: 100.0 - used,
        window_minutes: minutes,
        resets_at: resets,
    })
}
fn observed_display(raw: &str) -> String {
    let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(raw) else {
        return String::new();
    };
    let Ok(stamp) = Timestamp::from_unix(parsed.timestamp(), parsed.timestamp_subsec_nanos())
    else {
        return String::new();
    };
    time::format_with_offset(stamp, parsed.offset().local_minus_utc(), false).unwrap_or_default()
}
pub(super) fn directory_key(path: &std::path::Path) -> String {
    let clean =
        crate::hub::preference_media::clean_native_path(&path.to_string_lossy(), cfg!(windows));
    if cfg!(windows) {
        proto::unicode::simple_lower(&clean)
    } else {
        clean
    }
}

struct AuthBatchClaim {
    usage: Arc<UsageStore>,
    keys: Vec<(String, String)>,
}
impl Drop for AuthBatchClaim {
    fn drop(&mut self) {
        let mut state = self.usage.state.lock().unwrap_or_else(|p| p.into_inner());
        for key in &self.keys {
            state.checking.remove(key);
        }
    }
}

impl crate::application::session_usage::UsageSubscription for SubscriptionService {
    fn record_session_usage<'a>(
        &'a self,
        binding: SessionBinding,
        stat: &'a proto::Message,
        record_at: Timestamp,
    ) -> CoreFuture<'a, Result<(), SessionError>> {
        SubscriptionService::record_session_usage(self, binding, stat, record_at)
    }
}
