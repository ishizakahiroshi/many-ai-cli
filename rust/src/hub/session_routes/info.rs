//! /api/info projects the actual lifecycle/provider context plus config. Inputs
//! have no invented defaults: the owner supplies startup identity and current
//! binary/network/registry observations on every request.
use super::*;
use crate::{
    config,
    hub::Dispatch,
    profile::registry::Registry,
    proto::{self, core::CoreEffects},
};
use std::collections::{BTreeMap, BTreeSet};

pub struct InfoContext<'a> {
    pub cwd: &'a str,
    pub version: &'a str,
    pub git_commit: &'a str,
    pub build_time: &'a str,
    pub binary_sha256: &'a str,
    pub binary_stale: bool,
    pub web_src_hash: &'a str,
    pub web_dist_fresh: bool,
    /// Go names: windows-wsl, windows-native, wsl, darwin, linux.
    pub runtime_mode: &'a str,
    /// USERNAME, otherwise USER, resolved by the lifecycle environment owner.
    pub user_name_fallback: &'a str,
    pub ssh: bool,
    pub host_ip: &'a str,
    pub env_kind_override: &'a str,
    pub net_hint_ssh: bool,
    pub net_hint_host: &'a str,
    pub net_hint_env_kind: &'a str,
    /// None is the source's genuinely absent registry, not an empty fallback.
    pub registry: Option<&'a Registry>,
}
impl SessionHttp {
    /// Router MUST also set no-store on method/auth/Host/Origin/PIN failures.
    pub fn handle_info_authenticated(
        &self,
        config: &Config,
        context: &InfoContext<'_>,
    ) -> Dispatch {
        let avatar = &config.user_prefs.avatar;
        let avatar = if !avatar.is_empty()
            && !avatar.starts_with("http://")
            && !avatar.starts_with("https://")
        {
            "/api/avatar"
        } else {
            avatar
        };
        let display = if config.user_prefs.display_name.is_empty() {
            context.user_name_fallback
        } else {
            &config.user_prefs.display_name
        };
        let ssh = context.ssh || context.net_hint_ssh;
        let host = if context.net_hint_host.is_empty() {
            context.host_ip
        } else {
            context.net_hint_host
        };
        let env = environment(context, &config.hub.env_kind, ssh, host);
        let custom: Vec<_> = config::effective_custom_providers(&config.custom_providers)
            .iter()
            .map(|provider| json!({"id":provider.id,"label":provider.effective_label()}))
            .collect();
        let (effort, headless) = capabilities(config, context.registry);
        let role_permission: BTreeMap<_, _> = config
            .user_prefs
            .spawn
            .role_permission
            .iter()
            .filter(|(_, tier)| {
                !tier.is_empty() && config::validate_permission_preset(tier).is_ok()
            })
            .collect();
        let effects = {
            let mut notified = self
                .stale_notified
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            if *notified == context.binary_stale {
                CoreEffects::default()
            } else {
                *notified = context.binary_stale;
                self.core.broadcast_ui(proto::Message {
                    r#type: "binary_stale".into(),
                    binary_stale: Some(context.binary_stale),
                    ..Default::default()
                })
            }
        };
        Dispatch { effects, response: Response::json(200, &json!({
            "cwd":context.cwd,
            "custom_providers":custom,
            "version":context.version,
            "binary_sha256":context.binary_sha256,
            "binary_stale":context.binary_stale,
            "web_src_hash":context.web_src_hash,
            "web_dist_fresh":context.web_dist_fresh,
            "active_sessions":self.core.registered_session_count(),
            "git_commit":context.git_commit,
            "build_time":context.build_time,
            "runtime_mode":context.runtime_mode,
            "runtime_label":runtime_label(context.runtime_mode),
            "ssh":ssh,
            "host_ip":host,
            "env_kind":env.kind,
            "env_label":env.label,
            "env_short":env.short,
            "env_color":env.color,
            "env_title":env.title,
            "env_host_label":env.host_label,
            "userAvatar":avatar,
            "userDisplayName":display,
            "handoff_notify_remaining_percent":config.handoff.notify_remaining_percent_or_default(),
            "handoff_note_on_threshold":config.handoff.note_on_threshold_or_default(),
            "effort_levels":effort,
            "execution_modes":config::available_execution_modes(),
            "permission_presets":config::available_permission_presets(),
            "headless_providers":headless,
            "child_permission_preview":crate::orchestration::child_options::preview_ui_tiers(config),
            "child_permission_default":config.orchestration.child_permission_default_tier(),
            "role_permission":role_permission,
        })).no_store() }
    }
}
fn capabilities(
    config: &Config,
    registry: Option<&Registry>,
) -> (BTreeMap<String, Vec<String>>, Vec<String>) {
    let mut effort: BTreeMap<_, _> = config::effort_providers()
        .into_iter()
        .map(|provider| {
            let levels = config::effort_levels_for(&provider);
            (provider, levels)
        })
        .filter(|(_, levels)| !levels.is_empty())
        .collect();
    let mut headless = config::headless_providers(Some(config));
    let mut seen: BTreeSet<_> = headless.iter().cloned().collect();
    if let Some(registry) = registry {
        for summary in registry.list() {
            if let Some(definition) = registry.lookup(&summary.id)
                && let Some(launch) = definition.definition.launch
            {
                if !launch.effort_levels.is_empty() {
                    effort.insert(summary.id.clone(), launch.effort_levels);
                }
                if launch.headless.is_some() && seen.insert(summary.id.clone()) {
                    headless.push(summary.id);
                }
            }
        }
    }
    (effort, headless)
}
fn runtime_label(mode: &str) -> &str {
    match mode {
        "windows-wsl" => "Windows + WSL (many-ai-cli-launcher.exe)",
        "windows-native" => "Windows native",
        "wsl" => "WSL Linux",
        "darwin" => "macOS",
        "linux" => "Linux",
        _ => mode,
    }
}
fn env_kind(raw: &str) -> &'static str {
    match raw.trim().to_lowercase().as_str() {
        "local" => "local",
        "wsl" => "wsl",
        "remote" => "remote",
        "remote-tunnel" | "remotetunnel" | "remote_tunnel" => "remote-tunnel",
        _ => "",
    }
}
struct Environment<'a> {
    kind: &'static str,
    label: &'static str,
    short: &'static str,
    color: &'static str,
    title: &'static str,
    host_label: &'a str,
}
fn environment<'a>(
    context: &InfoContext<'_>,
    config_kind: &str,
    ssh: bool,
    host: &'a str,
) -> Environment<'a> {
    let explicit = [
        context.env_kind_override,
        config_kind,
        context.net_hint_env_kind,
    ]
    .into_iter()
    .find(|kind| !kind.trim().is_empty());
    let kind = if let Some(explicit) = explicit {
        match env_kind(explicit) {
            "" => "local",
            valid => valid,
        }
    } else if context.net_hint_ssh {
        "remote-tunnel"
    } else if matches!(context.runtime_mode, "wsl" | "windows-wsl") {
        "wsl"
    } else if ssh {
        "remote"
    } else {
        "local"
    };
    let (label, short, color, title) = match kind {
        "wsl" => ("WSL", "W", "#3b82f6", "W MANY-AI-CLI"),
        "remote" => ("Remote server", "R", "#f97316", "R MANY-AI-CLI"),
        "remote-tunnel" => ("Remote server (tunnel)", "T", "#ef4444", "T MANY-AI-CLI"),
        _ => ("Local", "L", "#22c55e", "L MANY-AI-CLI"),
    };
    Environment {
        kind,
        label,
        short,
        color,
        title,
        host_label: host.trim(),
    }
}
