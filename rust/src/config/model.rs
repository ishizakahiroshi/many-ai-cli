//! Typed configuration contracts ported from the fixed Go baseline.
//! Secrets serialize only through `to_private_yaml`; loading never uses the real home.
use super::{Resource, RuntimePaths};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, io,
    net::IpAddr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
pub type SessionOrderIDs = Vec<i64>;
pub type SubscriptionProfiles = BTreeMap<String, Vec<SubscriptionProfile>>;
pub type CustomProviders = Vec<CustomProvider>;
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct LogConfig {
    #[serde(rename = "enabled")]
    pub enabled: bool,
    #[serde(rename = "session_enabled")]
    pub session_enabled: bool,
    #[serde(rename = "legacy_logs_notice_shown")]
    pub legacy_logs_notice_shown: bool,
    #[serde(rename = "max_size_mb")]
    pub max_size_mb: i64,
    #[serde(rename = "max_backups")]
    pub max_backups: i64,
    #[serde(rename = "compress")]
    pub compress: bool,
    #[serde(rename = "session_retention_days")]
    pub session_retention_days: i64,
    #[serde(rename = "session_max_size_mb")]
    pub session_max_size_mb: i64,
    #[serde(rename = "attachment_retention_days")]
    pub attachment_retention_days: i64,
    #[serde(rename = "attachment_max_total_mb")]
    pub attachment_max_total_mb: i64,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct HandoffConfig {
    #[serde(rename = "enabled")]
    pub enabled: Option<bool>,
    #[serde(rename = "retention_days")]
    pub retention_days: i64,
    #[serde(rename = "intent_mode")]
    pub intent_mode: String,
    #[serde(rename = "notify_remaining_percent")]
    pub notify_remaining_percent: i64,
    #[serde(rename = "note_on_threshold")]
    pub note_on_threshold: String,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct ApprovalConfig {
    #[serde(rename = "enabled")]
    pub enabled: bool,
    #[serde(rename = "first_launch_shown")]
    pub first_launch_shown: bool,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct SlashCmdSources {
    #[serde(rename = "claude")]
    pub claude: String,
    #[serde(rename = "codex")]
    pub codex: String,
    #[serde(rename = "copilot")]
    pub copilot: String,
    #[serde(rename = "cursor-agent")]
    pub cursor_agent: String,
    #[serde(rename = "opencode")]
    pub opencode: String,
    #[serde(rename = "grok")]
    pub grok: String,
    #[serde(rename = "command-code")]
    pub command_code: String,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct ApprovalPatternSources {
    #[serde(rename = "claude")]
    pub claude: String,
    #[serde(rename = "codex")]
    pub codex: String,
    #[serde(rename = "copilot")]
    pub copilot: String,
    #[serde(rename = "cursor-agent")]
    pub cursor_agent: String,
    #[serde(rename = "opencode")]
    pub opencode: String,
    #[serde(rename = "grok")]
    pub grok: String,
    #[serde(rename = "command-code")]
    pub command_code: String,
    #[serde(rename = "common")]
    pub common: String,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct ApprovalProfiles {
    #[serde(rename = "claude")]
    pub claude: String,
    #[serde(rename = "codex")]
    pub codex: String,
    #[serde(rename = "copilot")]
    pub copilot: String,
    #[serde(rename = "cursor-agent")]
    pub cursor_agent: String,
    #[serde(rename = "opencode")]
    pub opencode: String,
    #[serde(rename = "grok")]
    pub grok: String,
    #[serde(rename = "command-code")]
    pub command_code: String,
    #[serde(rename = "common")]
    pub common: String,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct UserPrefsNotifySound {
    #[serde(rename = "enabled")]
    pub enabled: bool,
    #[serde(rename = "type")]
    pub r#type: String,
    #[serde(rename = "custom_file")]
    pub custom_file: String,
    #[serde(rename = "custom_mime")]
    pub custom_mime: String,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct UserPrefsTrigger {
    #[serde(rename = "enabled")]
    pub enabled: bool,
    #[serde(rename = "phrase")]
    pub phrase: String,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct UserPrefsApproval {
    #[serde(rename = "auto_switch")]
    pub auto_switch: bool,
    #[serde(rename = "auto_approval_enabled")]
    pub auto_approval_enabled: bool,
    #[serde(rename = "high_risk_confirmation_mode")]
    pub high_risk_confirmation_mode: String,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct UserPrefsDesktopNotifications {
    #[serde(rename = "enabled")]
    pub enabled: bool,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct UserPrefsPushNotifications {
    #[serde(rename = "enabled")]
    pub enabled: bool,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct UserPrefsQuickCmds {
    #[serde(rename = "cmd1")]
    pub cmd1: String,
    #[serde(rename = "cmd2")]
    pub cmd2: String,
    #[serde(rename = "cmd3")]
    pub cmd3: String,
    #[serde(rename = "cmd4")]
    pub cmd4: String,
    #[serde(rename = "cmd5")]
    pub cmd5: String,
    #[serde(rename = "show1")]
    pub show1: Option<bool>,
    #[serde(rename = "show2")]
    pub show2: Option<bool>,
    #[serde(rename = "show3")]
    pub show3: Option<bool>,
    #[serde(rename = "show4")]
    pub show4: Option<bool>,
    #[serde(rename = "show5")]
    pub show5: Option<bool>,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct UserPrefsTemplate {
    #[serde(rename = "label")]
    pub label: String,
    #[serde(rename = "body")]
    pub body: String,
    #[serde(rename = "providers")]
    pub providers: Vec<String>,
    #[serde(rename = "tags")]
    pub tags: Vec<String>,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct UserPrefsTemplateSend {
    #[serde(rename = "immediate")]
    pub immediate: Option<bool>,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct UserPrefsUsageLinks {
    #[serde(rename = "claude")]
    pub claude: String,
    #[serde(rename = "codex")]
    pub codex: String,
    #[serde(rename = "copilot")]
    pub copilot: String,
    #[serde(rename = "cursor-agent")]
    pub cursor_agent: String,
    #[serde(rename = "opencode")]
    pub opencode: String,
    #[serde(rename = "grok")]
    pub grok: String,
    #[serde(rename = "command-code")]
    pub command_code: String,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct UserPrefsVoice {
    #[serde(rename = "grace_seconds")]
    pub grace_seconds: i64,
    #[serde(rename = "wake_word_enabled")]
    pub wake_word_enabled: bool,
    #[serde(rename = "wake_word_phrase")]
    pub wake_word_phrase: String,
    #[serde(rename = "input_disabled")]
    pub input_disabled: bool,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct UserPrefsSpawn {
    #[serde(rename = "defaults")]
    pub defaults: BTreeMap<String, String>,
    #[serde(rename = "last_model")]
    pub last_model: BTreeMap<String, String>,
    #[serde(rename = "worktree_auto")]
    pub worktree_auto: bool,
    #[serde(rename = "worktree_cleanup")]
    pub worktree_cleanup: String,
    #[serde(rename = "delegation_auto")]
    pub delegation_auto: bool,
    #[serde(rename = "role_provider")]
    pub role_provider: BTreeMap<String, String>,
    #[serde(rename = "role_effort")]
    pub role_effort: BTreeMap<String, String>,
    #[serde(rename = "role_permission")]
    pub role_permission: BTreeMap<String, String>,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct UserPrefsDoneSummaryNotify {
    #[serde(rename = "enabled")]
    pub enabled: Option<bool>,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct UserPrefsWorkflowCompletionNotify {
    #[serde(rename = "enabled")]
    pub enabled: bool,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct UserPrefsTokenStatusbar {
    #[serde(rename = "enabled")]
    pub enabled: Option<bool>,
    #[serde(rename = "segments")]
    pub segments: BTreeMap<String, bool>,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct UserPrefsDisplay {
    #[serde(rename = "theme")]
    pub theme: String,
    #[serde(rename = "font_size")]
    pub font_size: String,
    #[serde(rename = "lang")]
    pub lang: String,
    #[serde(rename = "locked_mode")]
    pub locked_mode: String,
    #[serde(rename = "live_status_bg")]
    pub live_status_bg: String,
    #[serde(rename = "live_status_fg")]
    pub live_status_fg: String,
    #[serde(rename = "custom_themes")]
    pub custom_themes: Vec<UserPrefsCustomTheme>,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct UserPrefsCustomTheme {
    #[serde(rename = "id")]
    pub id: String,
    #[serde(rename = "name")]
    pub name: String,
    #[serde(rename = "mode")]
    pub mode: String,
    #[serde(rename = "hue")]
    pub hue: i64,
    #[serde(rename = "contrast")]
    pub contrast: i64,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct VoiceWhisperConfig {
    #[serde(rename = "server_url")]
    pub server_url: String,
    #[serde(rename = "request_path")]
    pub request_path: String,
    #[serde(rename = "language")]
    pub language: String,
    #[serde(rename = "timeout_seconds")]
    pub timeout_seconds: i64,
    #[serde(rename = "managed")]
    pub managed: bool,
    #[serde(rename = "model")]
    pub model: String,
    #[serde(rename = "server_port")]
    pub server_port: i64,
    #[serde(rename = "hallucination_phrases")]
    pub hallucination_phrases: Option<Vec<String>>,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct VoiceConfig {
    #[serde(rename = "whisper")]
    pub whisper: VoiceWhisperConfig,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct UserPrefsProjectView {
    #[serde(rename = "session_id")]
    pub session_id: i64,
    #[serde(rename = "tab")]
    pub tab: String,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct UserPrefs {
    #[serde(rename = "trigger")]
    pub trigger: UserPrefsTrigger,
    #[serde(rename = "notify_sound")]
    pub notify_sound: UserPrefsNotifySound,
    #[serde(rename = "desktop_notifications")]
    pub desktop_notifications: UserPrefsDesktopNotifications,
    #[serde(rename = "push_notifications")]
    pub push_notifications: UserPrefsPushNotifications,
    #[serde(rename = "approval")]
    pub approval: UserPrefsApproval,
    #[serde(rename = "quick_cmds")]
    pub quick_cmds: UserPrefsQuickCmds,
    #[serde(rename = "templates")]
    pub templates: Vec<UserPrefsTemplate>,
    #[serde(rename = "template_send")]
    pub template_send: UserPrefsTemplateSend,
    #[serde(rename = "usage_links")]
    pub usage_links: UserPrefsUsageLinks,
    #[serde(rename = "usage_probe_model")]
    pub usage_probe_model: String,
    #[serde(rename = "voice")]
    pub voice: UserPrefsVoice,
    #[serde(
        rename = "session_order",
        deserialize_with = "deserialize_session_order"
    )]
    pub session_order: SessionOrderIDs,
    #[serde(rename = "group_order")]
    pub group_order: Vec<String>,
    #[serde(rename = "project_favorites")]
    pub project_favorites: Vec<String>,
    #[serde(rename = "collapsed_nodes")]
    pub collapsed_nodes: Vec<String>,
    #[serde(rename = "cwd_history")]
    pub cwd_history: Vec<String>,
    #[serde(rename = "cwd_favorites")]
    pub cwd_favorites: Vec<String>,
    #[serde(rename = "spawn")]
    pub spawn: UserPrefsSpawn,
    #[serde(rename = "display")]
    pub display: UserPrefsDisplay,
    #[serde(rename = "migrated_from_localstorage")]
    pub migrated_from_localstorage: bool,
    #[serde(rename = "sidebar_pin_migrated")]
    pub sidebar_pin_migrated: bool,
    #[serde(rename = "project_views")]
    pub project_views: BTreeMap<String, UserPrefsProjectView>,
    #[serde(rename = "open_project")]
    pub open_project: String,
    #[serde(rename = "avatar")]
    pub avatar: String,
    #[serde(rename = "display_name")]
    pub display_name: String,
    #[serde(rename = "token_statusbar")]
    pub token_statusbar: UserPrefsTokenStatusbar,
    #[serde(rename = "done_summary_notify")]
    pub done_summary_notify: UserPrefsDoneSummaryNotify,
    #[serde(rename = "workflow_completion_notify")]
    pub workflow_completion_notify: UserPrefsWorkflowCompletionNotify,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct LocalModel {
    #[serde(rename = "id")]
    pub id: String,
    #[serde(rename = "label")]
    pub label: String,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct OllamaConfig {
    #[serde(rename = "base_url")]
    pub base_url: String,
    #[serde(rename = "allow_private_hosts")]
    pub allow_private_hosts: bool,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct LMStudioConfig {
    #[serde(rename = "base_url")]
    pub base_url: String,
    #[serde(rename = "allow_private_hosts")]
    pub allow_private_hosts: bool,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct NVIDIANIMConfig {
    #[serde(rename = "enabled")]
    pub enabled: bool,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct NotifyBackendConfig {
    #[serde(rename = "type")]
    pub r#type: String,
    #[serde(rename = "url")]
    pub url: String,
    #[serde(rename = "topic")]
    pub topic: String,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct NotifyConfig {
    #[serde(rename = "backends")]
    pub backends: Option<Vec<NotifyBackendConfig>>,
    #[serde(rename = "events")]
    pub events: Option<Vec<String>>,
    #[serde(rename = "include_body")]
    pub include_body: bool,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct OrchestrationConfig {
    #[serde(rename = "max_depth")]
    pub max_depth: i64,
    #[serde(rename = "max_children_per_parent")]
    pub max_children_per_parent: i64,
    #[serde(rename = "max_total_sessions")]
    pub max_total_sessions: i64,
    #[serde(rename = "child_timeout_seconds")]
    pub child_timeout_seconds: i64,
    #[serde(rename = "timeout_respawn")]
    pub timeout_respawn: bool,
    #[serde(rename = "max_timeout_respawns")]
    pub max_timeout_respawns: i64,
    #[serde(rename = "idle_done_threshold_seconds")]
    pub idle_done_threshold_sec: i64,
    #[serde(rename = "worktree_auto")]
    pub worktree_auto: Option<bool>,
    #[serde(rename = "worktree_dir_root")]
    pub worktree_dir_root: String,
    #[serde(rename = "board_notify_mode")]
    pub board_notify_mode: String,
    #[serde(rename = "spawn_confirm_mode")]
    pub spawn_confirm_mode: String,
    #[serde(rename = "spawn_confirm_providers")]
    pub spawn_confirm_providers: Vec<String>,
    #[serde(rename = "child_startup_grace_seconds")]
    pub child_startup_grace_seconds: i64,
    #[serde(rename = "child_startup_fail_enabled")]
    pub child_startup_fail: Option<bool>,
    #[serde(rename = "child_startup_kill")]
    pub child_startup_kill: Option<bool>,
    #[serde(rename = "child_full_bypass")]
    pub child_full_bypass: Option<bool>,
    #[serde(rename = "child_permission_default")]
    pub child_permission_default: String,
    #[serde(rename = "child_execution_mode")]
    pub child_execution_mode: String,
    #[serde(rename = "relay_execution_mode")]
    pub relay_execution_mode: String,
    #[serde(rename = "bounded_allowed_tools")]
    pub bounded_allowed_tools: BTreeMap<String, Vec<String>>,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct HubConfig {
    #[serde(rename = "port")]
    pub port: i64,
    #[serde(rename = "open_browser")]
    pub open_browser: bool,
    #[serde(rename = "auto_shutdown")]
    pub auto_shutdown: bool,
    #[serde(rename = "stale_binary_auto_restart")]
    pub stale_binary_auto_restart: bool,
    #[serde(rename = "log_dir")]
    pub log_dir: String,
    #[serde(rename = "idle_timeout_min")]
    pub idle_timeout_min: i64,
    #[serde(rename = "wrapper_reconnect_grace_sec")]
    pub wrapper_reconnect_grace_sec: i64,
    #[serde(rename = "wrapper_send_write_timeout_sec")]
    pub wrapper_send_write_timeout_sec: i64,
    #[serde(rename = "allow_loopback_without_token")]
    pub allow_loopback_without_token: bool,
    #[serde(rename = "trusted_networks")]
    pub trusted_networks: Vec<String>,
    #[serde(rename = "allowed_hosts")]
    pub allowed_hosts: Vec<String>,
    #[serde(rename = "env_kind")]
    pub env_kind: String,
    #[serde(rename = "terminal_color")]
    pub terminal_color: String,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct InputConfig {
    #[serde(rename = "deferred_enter_ms")]
    pub deferred_enter_ms: i64,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct SpawnConfig {
    #[serde(rename = "last_model")]
    pub last_model: BTreeMap<String, String>,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct WorkflowConfig {
    #[serde(rename = "journal_enabled")]
    pub journal_enabled: bool,
    #[serde(rename = "task_detail_enabled")]
    pub task_detail_enabled: bool,
    #[serde(rename = "subagent_tree_enabled")]
    pub subagent_tree_enabled: bool,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    #[serde(rename = "hub")]
    pub hub: HubConfig,
    #[serde(rename = "log")]
    pub log: LogConfig,
    #[serde(rename = "input")]
    pub input: InputConfig,
    #[serde(rename = "spawn")]
    pub spawn: SpawnConfig,
    #[serde(rename = "workflow")]
    pub workflow: WorkflowConfig,
    #[serde(rename = "approval")]
    pub approval: ApprovalConfig,
    #[serde(rename = "slash_cmd_sources")]
    pub slash_cmd_sources: SlashCmdSources,
    #[serde(rename = "models_source")]
    pub models_source: String,
    #[serde(rename = "approval_pattern_sources")]
    pub approval_pattern_sources: ApprovalPatternSources,
    #[serde(rename = "approval_profiles")]
    pub approval_profiles: ApprovalProfiles,
    #[serde(rename = "terminal_app")]
    pub terminal_app: String,
    #[serde(rename = "token", skip_serializing)]
    pub token: String,
    #[serde(rename = "auth_cookie_secret", skip_serializing)]
    pub auth_cookie_secret: String,
    #[serde(rename = "remote_pin_hash", skip_serializing)]
    pub remote_pin_hash: String,
    #[serde(rename = "ollama")]
    pub ollama: OllamaConfig,
    #[serde(rename = "lm_studio")]
    pub lm_studio: LMStudioConfig,
    #[serde(rename = "nvidia_nim")]
    pub nvidia_nim: NVIDIANIMConfig,
    #[serde(rename = "local_models")]
    pub local_models: Vec<LocalModel>,
    #[serde(rename = "user_prefs")]
    pub user_prefs: UserPrefs,
    #[serde(rename = "voice")]
    pub voice: VoiceConfig,
    #[serde(rename = "notify")]
    pub notify: NotifyConfig,
    #[serde(rename = "orchestration")]
    pub orchestration: OrchestrationConfig,
    #[serde(rename = "handoff")]
    pub handoff: HandoffConfig,
    #[serde(rename = "subscriptions")]
    pub subscriptions: SubscriptionProfiles,
    #[serde(rename = "custom_providers")]
    pub custom_providers: CustomProviders,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct SubscriptionProfile {
    #[serde(rename = "id")]
    pub id: String,
    #[serde(rename = "name")]
    pub name: String,
    #[serde(rename = "plan")]
    pub plan: String,
    #[serde(rename = "enabled")]
    pub enabled: Option<bool>,
    #[serde(rename = "dir")]
    pub dir: String,
    #[serde(rename = "profile_dir")]
    pub profile_dir: String,
    #[serde(rename = "settings_sync")]
    pub settings_sync: Option<bool>,
    #[serde(rename = "profile_owned_keys")]
    pub profile_owned_keys: Vec<String>,
    #[serde(rename = "default_wins_keys")]
    pub default_wins_keys: Vec<String>,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct CustomProvider {
    #[serde(rename = "id")]
    pub id: String,
    #[serde(rename = "label")]
    pub label: String,
    #[serde(rename = "command")]
    pub command: String,
    #[serde(rename = "approval_pattern_source")]
    pub approval_pattern_source: String,
    #[serde(rename = "headless")]
    pub headless: Option<HeadlessDef>,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct HeadlessDef {
    #[serde(rename = "args")]
    pub args: Vec<String>,
    #[serde(rename = "format")]
    pub format: String,
    #[serde(rename = "prompt_via")]
    pub prompt_via: String,
}
#[derive(Clone, Copy)]
pub(super) struct Field {
    pub(super) yaml: &'static str,
    json: &'static str,
    pub(super) kind: &'static str,
    yomit: bool,
    jomit: bool,
}
pub(super) fn schema(kind: &str) -> &'static [Field] {
    match kind {
        "LogConfig" => &[
            Field {
                yaml: "enabled",
                json: "enabled",
                kind: "bool",
                yomit: false,
                jomit: false,
            },
            Field {
                yaml: "session_enabled",
                json: "session_enabled",
                kind: "bool",
                yomit: false,
                jomit: false,
            },
            Field {
                yaml: "legacy_logs_notice_shown",
                json: "legacy_logs_notice_shown",
                kind: "bool",
                yomit: false,
                jomit: false,
            },
            Field {
                yaml: "max_size_mb",
                json: "max_size_mb",
                kind: "int",
                yomit: false,
                jomit: false,
            },
            Field {
                yaml: "max_backups",
                json: "max_backups",
                kind: "int",
                yomit: false,
                jomit: false,
            },
            Field {
                yaml: "compress",
                json: "compress",
                kind: "bool",
                yomit: false,
                jomit: false,
            },
            Field {
                yaml: "session_retention_days",
                json: "session_retention_days",
                kind: "int",
                yomit: false,
                jomit: false,
            },
            Field {
                yaml: "session_max_size_mb",
                json: "session_max_size_mb",
                kind: "int",
                yomit: false,
                jomit: false,
            },
            Field {
                yaml: "attachment_retention_days",
                json: "attachment_retention_days",
                kind: "int",
                yomit: false,
                jomit: false,
            },
            Field {
                yaml: "attachment_max_total_mb",
                json: "attachment_max_total_mb",
                kind: "int",
                yomit: false,
                jomit: false,
            },
        ],
        "HandoffConfig" => &[
            Field {
                yaml: "enabled",
                json: "enabled",
                kind: "*bool",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "retention_days",
                json: "retention_days",
                kind: "int",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "intent_mode",
                json: "intent_mode",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "notify_remaining_percent",
                json: "notify_remaining_percent",
                kind: "int",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "note_on_threshold",
                json: "note_on_threshold",
                kind: "string",
                yomit: true,
                jomit: true,
            },
        ],
        "ApprovalConfig" => &[
            Field {
                yaml: "enabled",
                json: "Enabled",
                kind: "bool",
                yomit: false,
                jomit: false,
            },
            Field {
                yaml: "first_launch_shown",
                json: "FirstLaunchShown",
                kind: "bool",
                yomit: false,
                jomit: false,
            },
        ],
        "SlashCmdSources" => &[
            Field {
                yaml: "claude",
                json: "claude",
                kind: "string",
                yomit: false,
                jomit: false,
            },
            Field {
                yaml: "codex",
                json: "codex",
                kind: "string",
                yomit: false,
                jomit: false,
            },
            Field {
                yaml: "copilot",
                json: "copilot",
                kind: "string",
                yomit: false,
                jomit: false,
            },
            Field {
                yaml: "cursor-agent",
                json: "cursor-agent",
                kind: "string",
                yomit: false,
                jomit: false,
            },
            Field {
                yaml: "opencode",
                json: "opencode",
                kind: "string",
                yomit: false,
                jomit: false,
            },
            Field {
                yaml: "grok",
                json: "grok",
                kind: "string",
                yomit: false,
                jomit: false,
            },
            Field {
                yaml: "command-code",
                json: "command-code",
                kind: "string",
                yomit: false,
                jomit: false,
            },
        ],
        "ApprovalPatternSources" => &[
            Field {
                yaml: "claude",
                json: "claude",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "codex",
                json: "codex",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "copilot",
                json: "copilot",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "cursor-agent",
                json: "cursor-agent",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "opencode",
                json: "opencode",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "grok",
                json: "grok",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "command-code",
                json: "command-code",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "common",
                json: "common",
                kind: "string",
                yomit: true,
                jomit: true,
            },
        ],
        "ApprovalProfiles" => &[
            Field {
                yaml: "claude",
                json: "claude",
                kind: "ApprovalProfileName",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "codex",
                json: "codex",
                kind: "ApprovalProfileName",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "copilot",
                json: "copilot",
                kind: "ApprovalProfileName",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "cursor-agent",
                json: "cursor-agent",
                kind: "ApprovalProfileName",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "opencode",
                json: "opencode",
                kind: "ApprovalProfileName",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "grok",
                json: "grok",
                kind: "ApprovalProfileName",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "command-code",
                json: "command-code",
                kind: "ApprovalProfileName",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "common",
                json: "common",
                kind: "ApprovalProfileName",
                yomit: true,
                jomit: true,
            },
        ],
        "UserPrefsNotifySound" => &[
            Field {
                yaml: "enabled",
                json: "enabled",
                kind: "bool",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "type",
                json: "type",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "custom_file",
                json: "custom_file",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "custom_mime",
                json: "custom_mime",
                kind: "string",
                yomit: true,
                jomit: true,
            },
        ],
        "UserPrefsTrigger" => &[
            Field {
                yaml: "enabled",
                json: "enabled",
                kind: "bool",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "phrase",
                json: "phrase",
                kind: "string",
                yomit: true,
                jomit: true,
            },
        ],
        "UserPrefsApproval" => &[
            Field {
                yaml: "auto_switch",
                json: "auto_switch",
                kind: "bool",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "auto_approval_enabled",
                json: "auto_approval_enabled",
                kind: "bool",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "high_risk_confirmation_mode",
                json: "high_risk_confirmation_mode",
                kind: "string",
                yomit: true,
                jomit: true,
            },
        ],
        "UserPrefsDesktopNotifications" => &[Field {
            yaml: "enabled",
            json: "enabled",
            kind: "bool",
            yomit: true,
            jomit: true,
        }],
        "UserPrefsPushNotifications" => &[Field {
            yaml: "enabled",
            json: "enabled",
            kind: "bool",
            yomit: true,
            jomit: true,
        }],
        "UserPrefsQuickCmds" => &[
            Field {
                yaml: "cmd1",
                json: "cmd1",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "cmd2",
                json: "cmd2",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "cmd3",
                json: "cmd3",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "cmd4",
                json: "cmd4",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "cmd5",
                json: "cmd5",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "show1",
                json: "show1",
                kind: "*bool",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "show2",
                json: "show2",
                kind: "*bool",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "show3",
                json: "show3",
                kind: "*bool",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "show4",
                json: "show4",
                kind: "*bool",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "show5",
                json: "show5",
                kind: "*bool",
                yomit: true,
                jomit: true,
            },
        ],
        "UserPrefsTemplate" => &[
            Field {
                yaml: "label",
                json: "label",
                kind: "string",
                yomit: false,
                jomit: false,
            },
            Field {
                yaml: "body",
                json: "body",
                kind: "string",
                yomit: false,
                jomit: false,
            },
            Field {
                yaml: "providers",
                json: "providers",
                kind: "[]string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "tags",
                json: "tags",
                kind: "[]string",
                yomit: true,
                jomit: true,
            },
        ],
        "UserPrefsTemplateSend" => &[Field {
            yaml: "immediate",
            json: "immediate",
            kind: "*bool",
            yomit: true,
            jomit: true,
        }],
        "UserPrefsUsageLinks" => &[
            Field {
                yaml: "claude",
                json: "claude",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "codex",
                json: "codex",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "copilot",
                json: "copilot",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "cursor-agent",
                json: "cursor-agent",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "opencode",
                json: "opencode",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "grok",
                json: "grok",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "command-code",
                json: "command-code",
                kind: "string",
                yomit: true,
                jomit: true,
            },
        ],
        "UserPrefsVoice" => &[
            Field {
                yaml: "grace_seconds",
                json: "grace_seconds",
                kind: "int",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "wake_word_enabled",
                json: "wake_word_enabled",
                kind: "bool",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "wake_word_phrase",
                json: "wake_word_phrase",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "input_disabled",
                json: "input_disabled",
                kind: "bool",
                yomit: true,
                jomit: true,
            },
        ],
        "UserPrefsSpawn" => &[
            Field {
                yaml: "defaults",
                json: "defaults",
                kind: "map[string]string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "last_model",
                json: "last_model",
                kind: "map[string]string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "worktree_auto",
                json: "worktree_auto",
                kind: "bool",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "worktree_cleanup",
                json: "worktree_cleanup",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "delegation_auto",
                json: "delegation_auto",
                kind: "bool",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "role_provider",
                json: "role_provider",
                kind: "map[string]string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "role_effort",
                json: "role_effort",
                kind: "map[string]string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "role_permission",
                json: "role_permission",
                kind: "map[string]string",
                yomit: true,
                jomit: true,
            },
        ],
        "UserPrefsDoneSummaryNotify" => &[Field {
            yaml: "enabled",
            json: "enabled",
            kind: "*bool",
            yomit: true,
            jomit: true,
        }],
        "UserPrefsWorkflowCompletionNotify" => &[Field {
            yaml: "enabled",
            json: "enabled",
            kind: "bool",
            yomit: true,
            jomit: true,
        }],
        "UserPrefsTokenStatusbar" => &[
            Field {
                yaml: "enabled",
                json: "enabled",
                kind: "*bool",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "segments",
                json: "segments",
                kind: "map[string]bool",
                yomit: true,
                jomit: true,
            },
        ],
        "UserPrefsDisplay" => &[
            Field {
                yaml: "theme",
                json: "theme",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "font_size",
                json: "font_size",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "lang",
                json: "lang",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "locked_mode",
                json: "locked_mode",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "live_status_bg",
                json: "live_status_bg",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "live_status_fg",
                json: "live_status_fg",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "custom_themes",
                json: "custom_themes",
                kind: "[]UserPrefsCustomTheme",
                yomit: true,
                jomit: true,
            },
        ],
        "UserPrefsCustomTheme" => &[
            Field {
                yaml: "id",
                json: "id",
                kind: "string",
                yomit: false,
                jomit: false,
            },
            Field {
                yaml: "name",
                json: "name",
                kind: "string",
                yomit: false,
                jomit: false,
            },
            Field {
                yaml: "mode",
                json: "mode",
                kind: "string",
                yomit: false,
                jomit: false,
            },
            Field {
                yaml: "hue",
                json: "hue",
                kind: "int",
                yomit: false,
                jomit: false,
            },
            Field {
                yaml: "contrast",
                json: "contrast",
                kind: "int",
                yomit: false,
                jomit: false,
            },
        ],
        "VoiceWhisperConfig" => &[
            Field {
                yaml: "server_url",
                json: "server_url",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "request_path",
                json: "request_path",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "language",
                json: "language",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "timeout_seconds",
                json: "timeout_seconds",
                kind: "int",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "managed",
                json: "managed",
                kind: "bool",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "model",
                json: "model",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "server_port",
                json: "server_port",
                kind: "int",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "hallucination_phrases",
                json: "hallucination_phrases",
                kind: "[]string",
                yomit: true,
                jomit: true,
            },
        ],
        "VoiceConfig" => &[Field {
            yaml: "whisper",
            json: "whisper",
            kind: "VoiceWhisperConfig",
            yomit: true,
            jomit: true,
        }],
        "UserPrefsProjectView" => &[
            Field {
                yaml: "session_id",
                json: "session_id",
                kind: "int",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "tab",
                json: "tab",
                kind: "string",
                yomit: true,
                jomit: true,
            },
        ],
        "UserPrefs" => &[
            Field {
                yaml: "trigger",
                json: "trigger",
                kind: "UserPrefsTrigger",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "notify_sound",
                json: "notify_sound",
                kind: "UserPrefsNotifySound",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "desktop_notifications",
                json: "desktop_notifications",
                kind: "UserPrefsDesktopNotifications",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "push_notifications",
                json: "push_notifications",
                kind: "UserPrefsPushNotifications",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "approval",
                json: "approval",
                kind: "UserPrefsApproval",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "quick_cmds",
                json: "quick_cmds",
                kind: "UserPrefsQuickCmds",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "templates",
                json: "templates",
                kind: "[]UserPrefsTemplate",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "template_send",
                json: "template_send",
                kind: "UserPrefsTemplateSend",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "usage_links",
                json: "usage_links",
                kind: "UserPrefsUsageLinks",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "usage_probe_model",
                json: "usage_probe_model",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "voice",
                json: "voice",
                kind: "UserPrefsVoice",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "session_order",
                json: "session_order",
                kind: "SessionOrderIDs",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "group_order",
                json: "group_order",
                kind: "[]string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "project_favorites",
                json: "project_favorites",
                kind: "[]string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "collapsed_nodes",
                json: "collapsed_nodes",
                kind: "[]string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "cwd_history",
                json: "cwd_history",
                kind: "[]string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "cwd_favorites",
                json: "cwd_favorites",
                kind: "[]string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "spawn",
                json: "spawn",
                kind: "UserPrefsSpawn",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "display",
                json: "display",
                kind: "UserPrefsDisplay",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "migrated_from_localstorage",
                json: "migrated_from_localstorage",
                kind: "bool",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "sidebar_pin_migrated",
                json: "sidebar_pin_migrated",
                kind: "bool",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "project_views",
                json: "project_views",
                kind: "map[string]UserPrefsProjectView",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "open_project",
                json: "open_project",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "avatar",
                json: "avatar",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "display_name",
                json: "display_name",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "token_statusbar",
                json: "token_statusbar",
                kind: "UserPrefsTokenStatusbar",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "done_summary_notify",
                json: "done_summary_notify",
                kind: "UserPrefsDoneSummaryNotify",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "workflow_completion_notify",
                json: "workflow_completion_notify",
                kind: "UserPrefsWorkflowCompletionNotify",
                yomit: true,
                jomit: true,
            },
        ],
        "LocalModel" => &[
            Field {
                yaml: "id",
                json: "id",
                kind: "string",
                yomit: false,
                jomit: false,
            },
            Field {
                yaml: "label",
                json: "label",
                kind: "string",
                yomit: true,
                jomit: true,
            },
        ],
        "OllamaConfig" => &[
            Field {
                yaml: "base_url",
                json: "base_url",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "allow_private_hosts",
                json: "allow_private_hosts",
                kind: "bool",
                yomit: true,
                jomit: true,
            },
        ],
        "LMStudioConfig" => &[
            Field {
                yaml: "base_url",
                json: "base_url",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "allow_private_hosts",
                json: "allow_private_hosts",
                kind: "bool",
                yomit: true,
                jomit: true,
            },
        ],
        "NVIDIANIMConfig" => &[Field {
            yaml: "enabled",
            json: "enabled",
            kind: "bool",
            yomit: true,
            jomit: true,
        }],
        "NotifyBackendConfig" => &[
            Field {
                yaml: "type",
                json: "type",
                kind: "string",
                yomit: false,
                jomit: false,
            },
            Field {
                yaml: "url",
                json: "url",
                kind: "string",
                yomit: false,
                jomit: false,
            },
            Field {
                yaml: "topic",
                json: "topic",
                kind: "string",
                yomit: true,
                jomit: true,
            },
        ],
        "NotifyConfig" => &[
            Field {
                yaml: "backends",
                json: "backends",
                kind: "[]NotifyBackendConfig",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "events",
                json: "events",
                kind: "[]string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "include_body",
                json: "include_body",
                kind: "bool",
                yomit: true,
                jomit: true,
            },
        ],
        "OrchestrationConfig" => &[
            Field {
                yaml: "max_depth",
                json: "max_depth",
                kind: "int",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "max_children_per_parent",
                json: "max_children_per_parent",
                kind: "int",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "max_total_sessions",
                json: "max_total_sessions",
                kind: "int",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "child_timeout_seconds",
                json: "child_timeout_seconds",
                kind: "int",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "timeout_respawn",
                json: "timeout_respawn",
                kind: "bool",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "max_timeout_respawns",
                json: "max_timeout_respawns",
                kind: "int",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "idle_done_threshold_seconds",
                json: "idle_done_threshold_seconds",
                kind: "int",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "worktree_auto",
                json: "worktree_auto",
                kind: "*bool",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "worktree_dir_root",
                json: "worktree_dir_root",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "board_notify_mode",
                json: "board_notify_mode",
                kind: "BoardNotifyMode",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "spawn_confirm_mode",
                json: "spawn_confirm_mode",
                kind: "SpawnConfirmMode",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "spawn_confirm_providers",
                json: "spawn_confirm_providers",
                kind: "[]string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "child_startup_grace_seconds",
                json: "child_startup_grace_seconds",
                kind: "int",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "child_startup_fail_enabled",
                json: "child_startup_fail_enabled",
                kind: "*bool",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "child_startup_kill",
                json: "child_startup_kill",
                kind: "*bool",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "child_full_bypass",
                json: "child_full_bypass",
                kind: "*bool",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "child_permission_default",
                json: "child_permission_default",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "child_execution_mode",
                json: "child_execution_mode",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "relay_execution_mode",
                json: "relay_execution_mode",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "bounded_allowed_tools",
                json: "bounded_allowed_tools",
                kind: "map[string][]string",
                yomit: true,
                jomit: true,
            },
        ],
        "HubConfig" => &[
            Field {
                yaml: "port",
                json: "Port",
                kind: "int",
                yomit: false,
                jomit: false,
            },
            Field {
                yaml: "open_browser",
                json: "OpenBrowser",
                kind: "bool",
                yomit: false,
                jomit: false,
            },
            Field {
                yaml: "auto_shutdown",
                json: "AutoShutdown",
                kind: "bool",
                yomit: false,
                jomit: false,
            },
            Field {
                yaml: "stale_binary_auto_restart",
                json: "StaleBinaryAutoRestart",
                kind: "bool",
                yomit: false,
                jomit: false,
            },
            Field {
                yaml: "log_dir",
                json: "LogDir",
                kind: "string",
                yomit: false,
                jomit: false,
            },
            Field {
                yaml: "idle_timeout_min",
                json: "IdleTimeoutMin",
                kind: "int",
                yomit: false,
                jomit: false,
            },
            Field {
                yaml: "wrapper_reconnect_grace_sec",
                json: "WrapperReconnectGraceSec",
                kind: "int",
                yomit: false,
                jomit: false,
            },
            Field {
                yaml: "wrapper_send_write_timeout_sec",
                json: "WrapperSendWriteTimeoutSec",
                kind: "int",
                yomit: false,
                jomit: false,
            },
            Field {
                yaml: "allow_loopback_without_token",
                json: "allow_loopback_without_token",
                kind: "bool",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "trusted_networks",
                json: "trusted_networks",
                kind: "[]string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "allowed_hosts",
                json: "allowed_hosts",
                kind: "[]string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "env_kind",
                json: "env_kind",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "terminal_color",
                json: "TerminalColor",
                kind: "string",
                yomit: false,
                jomit: false,
            },
        ],
        "InputConfig" => &[Field {
            yaml: "deferred_enter_ms",
            json: "deferred_enter_ms",
            kind: "int",
            yomit: true,
            jomit: true,
        }],
        "SpawnConfig" => &[Field {
            yaml: "last_model",
            json: "last_model",
            kind: "map[string]string",
            yomit: true,
            jomit: true,
        }],
        "WorkflowConfig" => &[
            Field {
                yaml: "journal_enabled",
                json: "journal_enabled",
                kind: "bool",
                yomit: false,
                jomit: false,
            },
            Field {
                yaml: "task_detail_enabled",
                json: "task_detail_enabled",
                kind: "bool",
                yomit: false,
                jomit: false,
            },
            Field {
                yaml: "subagent_tree_enabled",
                json: "subagent_tree_enabled",
                kind: "bool",
                yomit: false,
                jomit: false,
            },
        ],
        "Config" => &[
            Field {
                yaml: "hub",
                json: "Hub",
                kind: "HubConfig",
                yomit: false,
                jomit: false,
            },
            Field {
                yaml: "log",
                json: "Log",
                kind: "LogConfig",
                yomit: false,
                jomit: false,
            },
            Field {
                yaml: "input",
                json: "input",
                kind: "InputConfig",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "spawn",
                json: "spawn",
                kind: "SpawnConfig",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "workflow",
                json: "workflow",
                kind: "WorkflowConfig",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "approval",
                json: "Approval",
                kind: "ApprovalConfig",
                yomit: true,
                jomit: false,
            },
            Field {
                yaml: "slash_cmd_sources",
                json: "slash_cmd_sources",
                kind: "SlashCmdSources",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "models_source",
                json: "models_source",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "approval_pattern_sources",
                json: "approval_pattern_sources",
                kind: "ApprovalPatternSources",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "approval_profiles",
                json: "approval_profiles",
                kind: "ApprovalProfiles",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "terminal_app",
                json: "TerminalApp",
                kind: "string",
                yomit: true,
                jomit: false,
            },
            Field {
                yaml: "token",
                json: "Token",
                kind: "string",
                yomit: false,
                jomit: false,
            },
            Field {
                yaml: "auth_cookie_secret",
                json: "-",
                kind: "string",
                yomit: true,
                jomit: false,
            },
            Field {
                yaml: "remote_pin_hash",
                json: "-",
                kind: "string",
                yomit: true,
                jomit: false,
            },
            Field {
                yaml: "ollama",
                json: "ollama",
                kind: "OllamaConfig",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "lm_studio",
                json: "lm_studio",
                kind: "LMStudioConfig",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "nvidia_nim",
                json: "nvidia_nim",
                kind: "NVIDIANIMConfig",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "local_models",
                json: "local_models",
                kind: "[]LocalModel",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "user_prefs",
                json: "user_prefs",
                kind: "UserPrefs",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "voice",
                json: "voice",
                kind: "VoiceConfig",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "notify",
                json: "notify",
                kind: "NotifyConfig",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "orchestration",
                json: "orchestration",
                kind: "OrchestrationConfig",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "handoff",
                json: "handoff",
                kind: "HandoffConfig",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "subscriptions",
                json: "subscriptions",
                kind: "SubscriptionProfiles",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "custom_providers",
                json: "custom_providers",
                kind: "CustomProviders",
                yomit: true,
                jomit: true,
            },
        ],
        "SubscriptionProfile" => &[
            Field {
                yaml: "id",
                json: "id",
                kind: "string",
                yomit: false,
                jomit: false,
            },
            Field {
                yaml: "name",
                json: "name",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "plan",
                json: "plan",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "enabled",
                json: "enabled",
                kind: "*bool",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "dir",
                json: "dir",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "profile_dir",
                json: "profile_dir",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "settings_sync",
                json: "settings_sync",
                kind: "*bool",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "profile_owned_keys",
                json: "profile_owned_keys",
                kind: "[]string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "default_wins_keys",
                json: "default_wins_keys",
                kind: "[]string",
                yomit: true,
                jomit: true,
            },
        ],
        "CustomProvider" => &[
            Field {
                yaml: "id",
                json: "id",
                kind: "string",
                yomit: false,
                jomit: false,
            },
            Field {
                yaml: "label",
                json: "label",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "command",
                json: "command",
                kind: "string",
                yomit: false,
                jomit: false,
            },
            Field {
                yaml: "approval_pattern_source",
                json: "approval_pattern_source",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "headless",
                json: "headless",
                kind: "*HeadlessDef",
                yomit: true,
                jomit: true,
            },
        ],
        "HeadlessDef" => &[
            Field {
                yaml: "args",
                json: "args",
                kind: "[]string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "format",
                json: "format",
                kind: "string",
                yomit: true,
                jomit: true,
            },
            Field {
                yaml: "prompt_via",
                json: "prompt_via",
                kind: "string",
                yomit: true,
                jomit: true,
            },
        ],
        _ => &[],
    }
}

#[derive(Debug)]
pub enum ConfigError {
    Invalid(String),
    Parse(String),
    Io(io::Error),
    Conflict { expected: u64, actual: u64 },
    Poisoned,
}
impl ConfigError {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }
}
impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) | Self::Parse(message) => f.write_str(message),
            Self::Io(error) => write!(f, "config filesystem operation failed: {error}"),
            Self::Conflict { expected, actual } => write!(
                f,
                "config revision conflict: expected {expected}, actual {actual}"
            ),
            Self::Poisoned => f.write_str("config store lock was poisoned"),
        }
    }
}
impl std::error::Error for ConfigError {}
impl From<io::Error> for ConfigError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}
impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("public", &self.public_json())
            .finish_non_exhaustive()
    }
}

pub const DEFAULT_USAGE_PROBE_MODEL: &str = "claude-haiku-4-5";
pub const DEFAULT_OLLAMA_BASE_URL: &str = "http://localhost:11434";
pub const DEFAULT_LM_STUDIO_BASE_URL: &str = "http://localhost:1234";
pub const DEFAULT_MODELS_SOURCE: &str = "https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/models/defaults.json";
pub const DEFAULT_USAGE_LINK_SOURCE: &str = "https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/usage-links/defaults.json";
pub const DEFAULT_INSTALL_LINK_SOURCE: &str = "https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/install-links/defaults.json";
pub const BUILTIN_PROVIDER_IDS: &[&str] = &[
    "claude",
    "codex",
    "copilot",
    "cursor-agent",
    "opencode",
    "grok",
    "command-code",
];
pub const DEFAULT_WHISPER_HALLUCINATION_PHRASES: &[&str] = &[
    "ご視聴ありがとうございました",
    "ご清聴ありがとうございました",
    "チャンネル登録をお願いします",
    "チャンネル登録よろしくお願いします",
    "字幕視聴ありがとうございました",
    "thanks for watching",
    "thank you for watching",
    "please subscribe",
    "like and subscribe",
    "don't forget to subscribe",
];

pub fn valid_usage_probe_model(raw: &str) -> bool {
    let value = raw.trim();
    !value.is_empty()
        && !value.chars().any(|c| {
            c < '\u{20}' || c == '\u{7f}' || c.is_whitespace() || "\"'|&><^%();`$".contains(c)
        })
}
pub fn effective_usage_probe_model(raw: &str) -> String {
    if valid_usage_probe_model(raw) {
        raw.trim().into()
    } else {
        DEFAULT_USAGE_PROBE_MODEL.into()
    }
}
pub fn normalize_terminal_color(raw: &str) -> &'static str {
    match raw.trim().to_ascii_lowercase().as_str() {
        "inherit" => "inherit",
        "off" => "off",
        _ => "force",
    }
}
pub fn normalize_handoff_intent_mode(raw: &str) -> &'static str {
    if raw == "turn-summary" {
        "turn-summary"
    } else {
        "done-only"
    }
}
pub fn effective_board_notify_mode(raw: &str) -> &'static str {
    match raw {
        "queue-until-idle" => "queue-until-idle",
        "interrupt" => "interrupt",
        _ => "soft-notify",
    }
}
pub fn effective_spawn_confirm_mode(raw: &str) -> &'static str {
    match raw {
        "off" => "off",
        "providers" => "providers",
        _ => "on",
    }
}
pub fn effective_ollama_base_url(raw: &str) -> String {
    effective_base_url(raw, DEFAULT_OLLAMA_BASE_URL)
}
pub fn effective_lm_studio_base_url(raw: &str) -> String {
    effective_base_url(raw, DEFAULT_LM_STUDIO_BASE_URL)
}
fn effective_base_url(raw: &str, fallback: &str) -> String {
    let value = raw.trim().trim_end_matches('/');
    if value.is_empty() {
        fallback.into()
    } else {
        value.into()
    }
}
fn source_url(group: &str, provider: &str) -> String {
    format!(
        "https://raw.githubusercontent.com/ishizakahiroshi/many-ai-cli/main/resources/{group}/{provider}.md"
    )
}
impl SlashCmdSources {
    pub fn defaults() -> Self {
        Self {
            claude: source_url("slash-commands", "claude"),
            codex: source_url("slash-commands", "codex"),
            copilot: source_url("slash-commands", "copilot"),
            cursor_agent: source_url("slash-commands", "cursor-agent"),
            opencode: source_url("slash-commands", "opencode"),
            grok: source_url("slash-commands", "grok"),
            command_code: source_url("slash-commands", "command-code"),
        }
    }
    pub fn effective(mut self) -> Self {
        if self.claude == "https://code.claude.com/docs/en/commands.md" {
            self.claude.clear();
        }
        for (provider, field) in [
            ("claude", &mut self.claude),
            ("codex", &mut self.codex),
            ("copilot", &mut self.copilot),
            ("cursor-agent", &mut self.cursor_agent),
            ("opencode", &mut self.opencode),
            ("grok", &mut self.grok),
            ("command-code", &mut self.command_code),
        ] {
            if field.is_empty() {
                *field = source_url("slash-commands", provider);
            }
        }
        self
    }
}
impl ApprovalPatternSources {
    pub fn defaults() -> Self {
        Self::default().effective()
    }
    pub fn effective(mut self) -> Self {
        for (provider, field) in [
            ("claude", &mut self.claude),
            ("codex", &mut self.codex),
            ("copilot", &mut self.copilot),
            ("cursor-agent", &mut self.cursor_agent),
            ("opencode", &mut self.opencode),
            ("grok", &mut self.grok),
            ("command-code", &mut self.command_code),
            ("common", &mut self.common),
        ] {
            if field.is_empty() {
                *field = source_url("approval-patterns", provider);
            }
        }
        self
    }
}
impl ApprovalProfiles {
    pub fn defaults() -> Self {
        Self::default().effective()
    }
    pub fn effective(mut self) -> Self {
        for field in [
            &mut self.claude,
            &mut self.codex,
            &mut self.copilot,
            &mut self.cursor_agent,
            &mut self.opencode,
            &mut self.grok,
            &mut self.command_code,
            &mut self.common,
        ] {
            if field.is_empty() {
                *field = "official".into();
            }
        }
        self
    }
    pub fn for_provider(&self, provider: &str) -> &str {
        let field = match provider {
            "claude" => &self.claude,
            "codex" => &self.codex,
            "copilot" => &self.copilot,
            "cursor-agent" => &self.cursor_agent,
            "opencode" => &self.opencode,
            "grok" => &self.grok,
            "command-code" => &self.command_code,
            "common" => &self.common,
            _ => return "official",
        };
        if field.is_empty() { "official" } else { field }
    }
    pub fn with_provider(mut self, provider: &str, name: impl Into<String>) -> Self {
        let field = match provider {
            "claude" => &mut self.claude,
            "codex" => &mut self.codex,
            "copilot" => &mut self.copilot,
            "cursor-agent" => &mut self.cursor_agent,
            "opencode" => &mut self.opencode,
            "grok" => &mut self.grok,
            "command-code" => &mut self.command_code,
            "common" => &mut self.common,
            _ => return self,
        };
        *field = name.into();
        self
    }
}
impl HandoffConfig {
    pub fn enabled_or_default(&self) -> bool {
        self.enabled.unwrap_or(true)
    }
    pub fn retention_days_or_default(&self) -> i64 {
        if self.retention_days > 0 {
            self.retention_days
        } else {
            14
        }
    }
    pub fn notify_remaining_percent_or_default(&self) -> i64 {
        if self.notify_remaining_percent > 0 {
            self.notify_remaining_percent
        } else {
            10
        }
    }
    pub fn turn_summary_enabled(&self) -> bool {
        normalize_handoff_intent_mode(&self.intent_mode) == "turn-summary"
    }
    pub fn note_on_threshold_or_default(&self) -> &'static str {
        match self.note_on_threshold.trim() {
            "auto" => "auto",
            "off" => "off",
            _ => "ask",
        }
    }
}
impl UserPrefsTokenStatusbar {
    pub fn is_enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }
}
impl OrchestrationConfig {
    pub fn worktree_enabled(&self) -> bool {
        self.worktree_auto.unwrap_or(true)
    }
    pub fn child_startup_fail_enabled(&self) -> bool {
        self.child_startup_fail.unwrap_or(true)
    }
    pub fn child_startup_kill_enabled(&self) -> bool {
        self.child_startup_kill.unwrap_or(true)
    }
    pub fn child_full_bypass_enabled(&self) -> bool {
        self.child_full_bypass.unwrap_or(true)
    }
}

impl Config {
    /// Pure defaults. Token creation and all IO belong to `ConfigStore::load_or_create`.
    pub fn defaults(paths: &RuntimePaths) -> Self {
        let mut cfg = Self::default();
        cfg.hub.port = i64::from(paths.port());
        cfg.hub.open_browser = true;
        cfg.hub.auto_shutdown = true;
        cfg.hub.stale_binary_auto_restart = true;
        cfg.hub.terminal_color = "force".into();
        cfg.hub.log_dir = paths
            .resource(Resource::Logs)
            .to_string_lossy()
            .into_owned();
        cfg.hub.idle_timeout_min = 60;
        cfg.hub.wrapper_reconnect_grace_sec = 3600;
        cfg.hub.wrapper_send_write_timeout_sec = 5;
        cfg.log = LogConfig {
            enabled: true,
            max_size_mb: 10,
            max_backups: 3,
            session_retention_days: 7,
            session_max_size_mb: 50,
            attachment_retention_days: 7,
            attachment_max_total_mb: 500,
            ..LogConfig::default()
        };
        cfg.workflow = WorkflowConfig {
            journal_enabled: true,
            task_detail_enabled: true,
            subagent_tree_enabled: true,
        };
        cfg.slash_cmd_sources = SlashCmdSources::defaults();
        cfg.approval_pattern_sources = ApprovalPatternSources::defaults();
        cfg.approval_profiles = ApprovalProfiles::defaults();
        cfg.handoff.intent_mode = "done-only".into();
        cfg.apply_defaults(paths);
        cfg
    }
    pub fn from_yaml(text: &str, paths: &RuntimePaths) -> Result<Self, ConfigError> {
        // Never surface YAML parser excerpts: they may contain tokens or PIN hashes.
        let raw = super::yaml_nodes::decode_config(text).map_err(|error| match error {
            super::yaml_nodes::YamlDecodeError::Syntax => ConfigError::Parse(error.to_string()),
            super::yaml_nodes::YamlDecodeError::ResourceLimit => {
                ConfigError::Invalid(error.to_string())
            }
        })?;
        let normalized = normalize_node(raw, "Config", "config", false)?;
        let mut merged = Self::defaults(paths).private_value();
        overlay(&mut merged, normalized);
        let mut cfg: Self = serde_json::from_value(merged).map_err(|_| {
            ConfigError::Parse("config has an incompatible field type (content redacted)".into())
        })?;
        for (key, value) in std::mem::take(&mut cfg.spawn.last_model) {
            if !value.is_empty()
                && cfg
                    .user_prefs
                    .spawn
                    .last_model
                    .get(&key)
                    .is_none_or(String::is_empty)
            {
                cfg.user_prefs.spawn.last_model.insert(key, value);
            }
        }
        cfg.hub.terminal_color = normalize_terminal_color(&cfg.hub.terminal_color).into();
        cfg.handoff.intent_mode = normalize_handoff_intent_mode(&cfg.handoff.intent_mode).into();
        cfg.slash_cmd_sources = cfg.slash_cmd_sources.effective();
        cfg.approval_pattern_sources = cfg.approval_pattern_sources.effective();
        cfg.approval_profiles = cfg.approval_profiles.effective();
        cfg.apply_defaults(paths);
        cfg.validate()?;
        cfg.validate_paths(paths)?;
        Ok(cfg)
    }
    pub fn apply_defaults(&mut self, paths: &RuntimePaths) {
        self.user_prefs.usage_probe_model =
            effective_usage_probe_model(&self.user_prefs.usage_probe_model);
        let whisper = &mut self.voice.whisper;
        if whisper.language.trim().is_empty() {
            whisper.language = "ja".into();
        }
        if whisper.timeout_seconds <= 0 {
            whisper.timeout_seconds = 60;
        }
        if whisper.model.trim().is_empty() {
            whisper.model = "small".into();
        }
        if whisper.hallucination_phrases.is_none() {
            whisper.hallucination_phrases = Some(
                DEFAULT_WHISPER_HALLUCINATION_PHRASES
                    .iter()
                    .map(|s| (*s).into())
                    .collect(),
            );
        }
        let o = &mut self.orchestration;
        for (field, default) in [
            (&mut o.max_depth, 1),
            (&mut o.max_children_per_parent, 10),
            (&mut o.max_total_sessions, 257),
            (&mut o.child_timeout_seconds, 900),
            (&mut o.max_timeout_respawns, 1),
            (&mut o.idle_done_threshold_sec, 600),
            (&mut o.child_startup_grace_seconds, 60),
        ] {
            if *field <= 0 {
                *field = default;
            }
        }
        o.max_children_per_parent = o.max_children_per_parent.min(256);
        if o.worktree_dir_root.trim().is_empty() {
            o.worktree_dir_root = Path::new(".many-ai-cli")
                .join("worktrees")
                .to_string_lossy()
                .into_owned();
        }
        if o.board_notify_mode.is_empty() {
            o.board_notify_mode = "soft-notify".into();
        }
        if o.spawn_confirm_mode.is_empty() {
            o.spawn_confirm_mode = "on".into();
        }
        if self.handoff.retention_days <= 0 {
            self.handoff.retention_days = 14;
        }
        if paths.is_trial() {
            self.hub.port = i64::from(paths.port());
            self.hub.log_dir = paths
                .resource(Resource::Logs)
                .to_string_lossy()
                .into_owned();
            self.orchestration.worktree_dir_root = paths
                .root()
                .join("worktrees")
                .to_string_lossy()
                .into_owned();
        }
    }
    fn private_value(&self) -> Value {
        let mut value =
            serde_json::to_value(self).expect("configuration has only JSON-compatible values");
        value["token"] = Value::String(self.token.clone());
        value["auth_cookie_secret"] = Value::String(self.auth_cookie_secret.clone());
        value["remote_pin_hash"] = Value::String(self.remote_pin_hash.clone());
        value
    }
    /// Go JSON names/omitempty semantics, excluding every authentication secret.
    pub fn public_json(&self) -> Value {
        project(
            serde_json::to_value(self).expect("configuration is JSON-compatible"),
            "Config",
            true,
        )
    }
    /// A detached serialization payload. Calling this never mutates the live configuration.
    pub fn to_private_yaml(&self) -> Result<String, ConfigError> {
        serde_saphyr::to_string(&project(self.private_value(), "Config", false)).map_err(|_| {
            ConfigError::invalid("config YAML serialization failed (content redacted)")
        })
    }
    pub fn validate_paths(&self, paths: &RuntimePaths) -> Result<(), ConfigError> {
        if !paths.is_trial() {
            return Ok(());
        }
        for list in self.subscriptions.values() {
            for profile in list {
                if !profile.profile_dir.trim().is_empty() {
                    checked_trial_path(paths, Path::new(profile.profile_dir.trim()))?;
                }
            }
        }
        for path in [&self.hub.log_dir, &self.orchestration.worktree_dir_root] {
            checked_trial_path(paths, Path::new(path))?;
        }
        Ok(())
    }
}

fn checked_trial_path(paths: &RuntimePaths, path: &Path) -> Result<PathBuf, ConfigError> {
    if !path.is_absolute()
        || !path.starts_with(paths.root())
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(ConfigError::invalid(
            "configured path escapes the explicit trial root",
        ));
    }
    let mut ancestor = path;
    while !ancestor.exists() {
        ancestor = ancestor
            .parent()
            .ok_or_else(|| ConfigError::invalid("trial path has no existing ancestor"))?;
    }
    if !std::fs::canonicalize(ancestor)?.starts_with(paths.root()) {
        return Err(ConfigError::invalid(
            "configured path symlink escapes the explicit trial root",
        ));
    }
    Ok(path.to_owned())
}

fn empty(value: &Value, kind: &str, public: bool) -> bool {
    if value.is_null() {
        return true;
    }
    if kind.starts_with('*') {
        return false;
    }
    if !schema(kind).is_empty() {
        return !public
            && schema(kind).iter().all(|field| {
                value
                    .get(field.yaml)
                    .is_none_or(|v| empty(v, field.kind, false))
            });
    }
    match value {
        Value::Bool(v) => !v,
        Value::Number(v) => v.as_i64() == Some(0),
        Value::String(v) => v.is_empty(),
        Value::Array(v) => v.is_empty(),
        Value::Object(v) => v.is_empty(),
        Value::Null => true,
    }
}
fn project(value: Value, kind: &str, public: bool) -> Value {
    if let Some(inner) = kind.strip_prefix('*') {
        return if value.is_null() {
            value
        } else {
            project(value, inner, public)
        };
    }
    let kind = match kind {
        "SubscriptionProfiles" => "map[string][]SubscriptionProfile",
        "CustomProviders" => "[]CustomProvider",
        "SessionOrderIDs" => "[]int",
        other => other,
    };
    if let Some(inner) = kind.strip_prefix("[]") {
        return match value {
            Value::Array(list) => Value::Array(
                list.into_iter()
                    .map(|v| project(v, inner, public))
                    .collect(),
            ),
            other => other,
        };
    }
    if let Some(inner) = kind.strip_prefix("map[string]") {
        return match value {
            Value::Object(map) => Value::Object(
                map.into_iter()
                    .map(|(k, v)| (k, project(v, inner, public)))
                    .collect(),
            ),
            other => other,
        };
    }
    let fields = schema(kind);
    if fields.is_empty() {
        return value;
    }
    let mut result = Map::new();
    for field in fields {
        if public && (field.json == "-" || (kind == "Config" && field.yaml == "token")) {
            continue;
        }
        let Some(raw) = value.get(field.yaml) else {
            continue;
        };
        let omit = if public { field.jomit } else { field.yomit };
        if omit && empty(raw, field.kind, public) {
            continue;
        }
        result.insert(
            if public { field.json } else { field.yaml }.into(),
            project(raw.clone(), field.kind, public),
        );
    }
    Value::Object(result)
}
fn overlay(base: &mut Value, next: Value) {
    if let (Value::Object(base), Value::Object(next)) = (&mut *base, &next) {
        for (key, value) in next {
            if let Some(existing) = base.get_mut(key) {
                overlay(existing, value.clone());
            } else {
                base.insert(key.clone(), value.clone());
            }
        }
    } else {
        *base = next;
    }
}
fn zero(kind: &str) -> Value {
    if kind.starts_with('*') {
        return Value::Null;
    }
    if kind.starts_with("[]") || matches!(kind, "SessionOrderIDs" | "CustomProviders") {
        return Value::Array(vec![]);
    }
    if kind.starts_with("map[") || kind == "SubscriptionProfiles" || !schema(kind).is_empty() {
        return Value::Object(Map::new());
    }
    match kind {
        "bool" => Value::Bool(false),
        "int" => Value::from(0),
        _ => Value::String(String::new()),
    }
}
fn normalize_node(
    value: Value,
    kind: &str,
    path: &str,
    tolerant: bool,
) -> Result<Value, ConfigError> {
    let mismatch = || {
        ConfigError::Parse(format!(
            "{path} has an incompatible field type (content redacted)"
        ))
    };
    if kind == "SessionOrderIDs" {
        return Ok(Value::Array(
            session_order_from_value(value)
                .into_iter()
                .map(Value::from)
                .collect(),
        ));
    }
    if kind == "SubscriptionProfiles" {
        let mut out = Map::new();
        if let Value::Object(providers) = value {
            for (provider, list) in providers {
                let mut kept = vec![];
                if let Value::Array(list) = list {
                    for profile in list {
                        if !profile.is_object() {
                            continue;
                        }
                        let profile = normalize_node(
                            profile,
                            "SubscriptionProfile",
                            "subscriptions.profile",
                            true,
                        )?;
                        if profile
                            .get("id")
                            .and_then(Value::as_str)
                            .is_some_and(|id| !id.trim().is_empty())
                        {
                            kept.push(profile);
                        }
                    }
                }
                if !kept.is_empty() {
                    out.insert(provider, Value::Array(kept));
                }
            }
        }
        return Ok(Value::Object(out));
    }
    if kind == "CustomProviders" {
        let mut kept = vec![];
        if let Value::Array(list) = value {
            for profile in list {
                if !profile.is_object() {
                    continue;
                }
                if let Ok(profile) =
                    normalize_node(profile, "CustomProvider", "custom_providers.entry", false)
                    && ["id", "command"].iter().all(|key| {
                        profile
                            .get(key)
                            .and_then(Value::as_str)
                            .is_some_and(|v| !v.trim().is_empty())
                    })
                {
                    kept.push(profile);
                }
            }
        }
        return Ok(Value::Array(kept));
    }
    if value.is_null() {
        return Ok(zero(kind));
    }
    if let Some(inner) = kind.strip_prefix('*') {
        return normalize_node(value, inner, path, tolerant);
    }
    if let Some(inner) = kind.strip_prefix("[]") {
        let Value::Array(list) = value else {
            return if tolerant {
                Ok(zero(kind))
            } else {
                Err(mismatch())
            };
        };
        let mut out = vec![];
        for entry in list {
            match normalize_node(entry, inner, path, false) {
                Ok(v) => out.push(v),
                Err(_) if tolerant => {}
                Err(e) => return Err(e),
            }
        }
        return Ok(Value::Array(out));
    }
    if let Some(inner) = kind.strip_prefix("map[string]") {
        let Value::Object(map) = value else {
            return if tolerant {
                Ok(zero(kind))
            } else {
                Err(mismatch())
            };
        };
        let mut out = Map::new();
        for (key, value) in map {
            out.insert(key, normalize_node(value, inner, path, tolerant)?);
        }
        return Ok(Value::Object(out));
    }
    let fields = schema(kind);
    if !fields.is_empty() {
        let Value::Object(map) = value else {
            return if tolerant {
                Ok(zero(kind))
            } else {
                Err(mismatch())
            };
        };
        let mut out = Map::new();
        for field in fields {
            if let Some(value) = map.get(field.yaml) {
                // yaml.v3 null leaves scalar and struct defaults alone, but clears pointers/slices/maps.
                if value.is_null()
                    && !field.kind.starts_with('*')
                    && !field.kind.starts_with("[]")
                    && !field.kind.starts_with("map[")
                    && !matches!(
                        field.kind,
                        "SubscriptionProfiles" | "CustomProviders" | "SessionOrderIDs"
                    )
                {
                    continue;
                }
                if ((kind == "VoiceWhisperConfig" && field.yaml == "hallucination_phrases")
                    || (kind == "NotifyConfig" && matches!(field.yaml, "backends" | "events")))
                    && value.is_null()
                {
                    out.insert(field.yaml.into(), Value::Null);
                    continue;
                }
                match normalize_node(
                    value.clone(),
                    field.kind,
                    &format!("{path}.{}", field.yaml),
                    tolerant,
                ) {
                    Ok(value) => {
                        out.insert(field.yaml.into(), value);
                    }
                    Err(_) if tolerant => {
                        // yaml.v3 allocates a non-null *bool before reporting a
                        // bad scalar; SubscriptionProfiles deliberately keeps
                        // the partially decoded profile, including false.
                        if kind == "SubscriptionProfile" && field.kind == "*bool" {
                            out.insert(field.yaml.into(), Value::Bool(false));
                        }
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        return Ok(Value::Object(out));
    }
    match kind {
        "bool" => match value {
            Value::Bool(_) => Ok(value),
            Value::String(ref s) => match s.as_str() {
                "y" | "Y" | "yes" | "Yes" | "YES" | "on" | "On" | "ON" => Ok(Value::Bool(true)),
                "n" | "N" | "no" | "No" | "NO" | "off" | "Off" | "OFF" => Ok(Value::Bool(false)),
                _ => Err(mismatch()),
            },
            _ => Err(mismatch()),
        },
        "int" => {
            if value.as_i64().is_some() {
                Ok(value)
            } else if let Some(f) = value.as_f64() {
                if f >= i64::MIN as f64 && f < -(i64::MIN as f64) {
                    Ok(Value::from(f as i64))
                } else {
                    Err(mismatch())
                }
            } else {
                Err(mismatch())
            }
        }
        _ => match value {
            Value::String(_) => Ok(value),
            Value::Bool(v) => Ok(Value::String(v.to_string())),
            Value::Number(v) => Ok(Value::String(v.to_string())),
            _ => Err(mismatch()),
        },
    }
}
fn session_order_from_value(value: Value) -> Vec<i64> {
    let Value::Array(list) = value else {
        return vec![];
    };
    list.into_iter()
        .filter_map(|v| match v {
            Value::String(s) => s.trim().parse().ok(),
            Value::Number(n) => n.as_i64().or_else(|| {
                n.as_f64()
                    .filter(|f| {
                        f.fract() == 0.0 && *f >= i64::MIN as f64 && *f < -(i64::MIN as f64)
                    })
                    .map(|f| f as i64)
            }),
            _ => None,
        })
        .collect()
}
fn deserialize_session_order<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<i64>, D::Error> {
    Ok(session_order_from_value(Value::deserialize(deserializer)?))
}

impl Config {
    pub fn validate(&self) -> Result<(), ConfigError> {
        for raw in &self.hub.trusted_networks {
            let (ip, prefix) = raw.trim().split_once('/').ok_or_else(|| {
                ConfigError::invalid("hub.trusted_networks contains invalid CIDR")
            })?;
            let ip: IpAddr = ip
                .parse()
                .map_err(|_| ConfigError::invalid("hub.trusted_networks contains invalid CIDR"))?;
            let prefix: u8 = prefix
                .parse()
                .map_err(|_| ConfigError::invalid("hub.trusted_networks contains invalid CIDR"))?;
            let range = if ip.is_ipv4() { 24..=32 } else { 64..=128 };
            if !range.contains(&prefix) {
                return Err(ConfigError::invalid(
                    "hub.trusted_networks is too broad or has an invalid prefix",
                ));
            }
        }
        for raw in &self.hub.allowed_hosts {
            let host = raw
                .trim()
                .strip_suffix('.')
                .unwrap_or(raw.trim())
                .to_lowercase();
            let ip = host
                .trim_start_matches('[')
                .trim_end_matches(']')
                .parse::<IpAddr>();
            if host.is_empty()
                || host.contains(['*', '/', '\\'])
                || (ip.is_err() && !valid_hostname(&host))
            {
                return Err(ConfigError::invalid(
                    "hub.allowed_hosts contains an invalid host name or IP literal (ports are not allowed)",
                ));
            }
        }
        let whisper = &self.voice.whisper;
        if !whisper.server_url.trim().is_empty() {
            let url = parse_config_url(whisper.server_url.trim()).map_err(|_| {
                ConfigError::invalid("voice.whisper.server_url must be a localhost http URL")
            })?;
            if url.scheme != "http" || url.host.is_empty() {
                return Err(ConfigError::invalid(
                    "voice.whisper.server_url must be a localhost http URL",
                ));
            }
            let host = url.host.to_lowercase();
            let host = host.strip_suffix('.').unwrap_or(&host).to_owned();
            if !matches!(host.as_str(), "127.0.0.1" | "localhost" | "::1") {
                return Err(ConfigError::invalid(
                    "voice.whisper.server_url must point to localhost",
                ));
            }
        }
        let path = whisper.request_path.trim();
        if !path.is_empty() && (!path.starts_with('/') || path.contains("://")) {
            return Err(ConfigError::invalid(
                "voice.whisper.request_path must be empty or start with /",
            ));
        }
        if !(1..=300).contains(&whisper.timeout_seconds) {
            return Err(ConfigError::invalid(
                "voice.whisper.timeout_seconds must be between 1 and 300",
            ));
        }
        if whisper.server_port != 0 && !(1024..=65535).contains(&whisper.server_port) {
            return Err(ConfigError::invalid(
                "voice.whisper.server_port must be between 1024 and 65535",
            ));
        }
        validate_model_url("ollama", &self.ollama.base_url)?;
        validate_model_url("lm_studio", &self.lm_studio.base_url)?;
        if !matches!(
            self.orchestration.board_notify_mode.as_str(),
            "soft-notify" | "queue-until-idle" | "interrupt"
        ) {
            return Err(ConfigError::invalid(
                "orchestration.board_notify_mode must be soft-notify, queue-until-idle, or interrupt",
            ));
        }
        if !matches!(
            self.orchestration.spawn_confirm_mode.as_str(),
            "" | "on" | "off" | "providers"
        ) {
            return Err(ConfigError::invalid(
                "orchestration.spawn_confirm_mode must be on, off, or providers",
            ));
        }
        Ok(())
    }
    pub fn warnings(&self) -> Vec<String> {
        let mut out = vec![];
        for (key, raw, allowed) in [
            (
                "ollama",
                &self.ollama.base_url,
                self.ollama.allow_private_hosts,
            ),
            (
                "lm_studio",
                &self.lm_studio.base_url,
                self.lm_studio.allow_private_hosts,
            ),
        ] {
            if !allowed
                && let Ok(url) = parse_config_url(raw.trim())
                && is_private_model_host(&url.host)
            {
                let host = &url.host;
                out.push(format!("{key}.base_url points to a private host ({host}) but {key}.allow_private_hosts is false; the model list will be blocked at the transport layer. Set {key}.allow_private_hosts: true to use it."));
            }
        }
        out.extend(subscription_warnings(&self.subscriptions));
        let (kept, invalid, duplicate) = filter_custom_providers(&self.custom_providers);
        for p in kept {
            if let Some(def) = &p.headless
                && let Err(e) = validate_headless_def(def)
            {
                out.push(format!("custom_providers entry {:?} has an invalid headless definition ({e}) and can only be launched interactively", p.id));
            }
        }
        for id in invalid {
            out.push(format!("custom_providers entry {id:?} is invalid (bad id, collides with a built-in provider, or missing command) and will not appear as a spawn option"));
        }
        for id in duplicate {
            out.push(format!("custom_providers entry {id:?} duplicates another custom provider id and will not appear as a spawn option"));
        }
        out.extend(self.orchestration.launch_warnings());
        let raw = self.handoff.note_on_threshold.trim();
        if !matches!(raw, "" | "ask" | "auto" | "off") {
            out.push(format!("handoff.note_on_threshold {raw:?} is not one of ask/auto/off; the handoff memo button keeps the built-in default (ask)."));
        }
        out
    }
    pub fn is_custom_provider_id(&self, id: &str) -> bool {
        let id = normalize_subscription_id(id);
        effective_custom_providers(&self.custom_providers)
            .iter()
            .any(|p| p.id == id)
    }
}
fn valid_hostname(host: &str) -> bool {
    host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label.bytes().enumerate().all(|(i, c)| {
                    c.is_ascii_lowercase()
                        || c.is_ascii_digit()
                        || (c == b'-' && i > 0 && i + 1 < label.len())
                })
        })
}
struct ConfigUrl {
    scheme: String,
    host: String,
    credentials: bool,
    path: String,
    query: String,
    fragment: String,
}
fn parse_config_url(raw: &str) -> Result<ConfigUrl, ()> {
    if raw.bytes().any(|c| c < 0x20 || c == 0x7f) {
        return Err(());
    }
    let (scheme, rest) = raw.split_once("://").ok_or(())?;
    let (without_fragment, fragment) = rest.split_once('#').unwrap_or((rest, ""));
    let (without_query, query) = without_fragment
        .split_once('?')
        .unwrap_or((without_fragment, ""));
    let (authority, path) = match without_query.find('/') {
        Some(index) => (&without_query[..index], &without_query[index..]),
        None => (without_query, ""),
    };
    if authority.is_empty() || authority.contains('\\') {
        return Err(());
    }
    // Use the URL crate for syntax validation, but retain original host/path spelling:
    // WHATWG normalization must not turn 127.1 or /.. into accepted Go configuration.
    url::Url::parse(raw).map_err(|_| ())?;
    let credentials = authority.contains('@');
    let host_port = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host = if let Some(bracketed) = host_port.strip_prefix('[') {
        bracketed.split_once(']').ok_or(())?.0
    } else {
        host_port
            .rsplit_once(':')
            .map_or(host_port, |(host, _)| host)
    };
    Ok(ConfigUrl {
        scheme: scheme.to_ascii_lowercase(),
        host: host.into(),
        credentials,
        path: path.into(),
        query: query.into(),
        fragment: fragment.into(),
    })
}
fn validate_model_url(key: &str, raw: &str) -> Result<(), ConfigError> {
    if raw.trim().is_empty() {
        return Ok(());
    }
    let url = parse_config_url(raw.trim()).map_err(|_| {
        ConfigError::invalid(format!("{key}.base_url must be an http or https base URL"))
    })?;
    if url.host.is_empty() {
        return Err(ConfigError::invalid(format!(
            "{key}.base_url must be an http or https base URL"
        )));
    }
    if !matches!(url.scheme.as_str(), "http" | "https") {
        return Err(ConfigError::invalid(format!(
            "{key}.base_url must use http or https"
        )));
    }
    if url.credentials || !url.query.is_empty() || !url.fragment.is_empty() {
        return Err(ConfigError::invalid(format!(
            "{key}.base_url must not include credentials, query, or fragment"
        )));
    }
    if !matches!(url.path.as_str(), "" | "/") {
        return Err(ConfigError::invalid(format!(
            "{key}.base_url must not include a path"
        )));
    }
    Ok(())
}

fn is_private_model_host(raw: &str) -> bool {
    let host = raw
        .trim()
        .strip_suffix('.')
        .unwrap_or(raw.trim())
        .to_lowercase();
    if host == "localhost" {
        return true;
    }
    match host.trim_matches(['[', ']']).parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => {
            ip.is_unspecified()
                || ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_multicast()
        }
        Ok(IpAddr::V6(ip)) => {
            if let Some(v4) = ip.to_ipv4_mapped() {
                return is_private_model_host(&v4.to_string());
            }
            let bytes = ip.octets();
            ip.is_unspecified()
                || ip.is_loopback()
                || ip.is_multicast()
                || (bytes[0] & 0xfe == 0xfc)
                || (bytes[0] == 0xfe && bytes[1] & 0xc0 == 0x80)
        }
        Err(_) => false,
    }
}

pub fn normalize_subscription_id(raw: &str) -> String {
    crate::proto::unicode::simple_lower(raw.trim())
}
fn valid_id_shape(id: &str) -> bool {
    id.as_bytes()
        .first()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && id
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"._-".contains(&c))
}
pub fn validate_subscription_id(id: &str) -> Result<(), ConfigError> {
    if id.is_empty() {
        return Err(ConfigError::invalid("subscription id is required"));
    }
    if id.len() > 64 {
        return Err(ConfigError::invalid(
            "subscription id is longer than 64 characters",
        ));
    }
    if !valid_id_shape(id) {
        return Err(ConfigError::invalid(format!(
            "subscription id {id:?} may only contain lowercase letters, digits, dot, underscore and hyphen, and must start with a letter or digit"
        )));
    }
    if id.contains("..") {
        return Err(ConfigError::invalid(format!(
            "subscription id {id:?} must not contain a \"..\" sequence"
        )));
    }
    if id == "auto" {
        return Err(ConfigError::invalid(
            "subscription id \"auto\" is reserved for automatic selection",
        ));
    }
    Ok(())
}
pub fn validate_subscription_dir_name(name: &str) -> Result<(), ConfigError> {
    if name.is_empty() {
        return Err(ConfigError::invalid("subscription dir is required"));
    }
    if name.len() > 64 {
        return Err(ConfigError::invalid(
            "subscription dir is longer than 64 characters",
        ));
    }
    if !valid_id_shape(name) || name.contains("..") {
        return Err(ConfigError::invalid(format!(
            "subscription dir {name:?} may only contain lowercase letters, digits, dot, underscore and hyphen, and must start with a letter or digit"
        )));
    }
    Ok(())
}
pub fn validate_subscription_provider(provider: &str) -> Result<(), ConfigError> {
    if provider.trim().is_empty() {
        return Err(ConfigError::invalid("subscription provider is required"));
    }
    if provider != provider.to_lowercase() {
        return Err(ConfigError::invalid(format!(
            "subscription provider {provider:?} must be lowercase"
        )));
    }
    if !valid_id_shape(provider) || provider.contains("..") {
        return Err(ConfigError::invalid(format!(
            "subscription provider {provider:?} is not a valid provider key"
        )));
    }
    Ok(())
}
pub fn is_builtin_provider_id(id: &str) -> bool {
    BUILTIN_PROVIDER_IDS.contains(&normalize_subscription_id(id).as_str())
}
pub fn is_reserved_provider_id(id: &str) -> bool {
    is_builtin_provider_id(id) || normalize_subscription_id(id) == "shell"
}
pub fn validate_custom_provider_id(id: &str) -> Result<(), ConfigError> {
    if id.is_empty() {
        return Err(ConfigError::invalid("custom provider id is required"));
    }
    if id.len() > 64 {
        return Err(ConfigError::invalid(
            "custom provider id is longer than 64 characters",
        ));
    }
    if !valid_id_shape(id) {
        return Err(ConfigError::invalid(
            "custom provider id has an invalid shape",
        ));
    }
    if is_reserved_provider_id(id) {
        return Err(ConfigError::invalid(
            "custom provider id collides with a built-in provider or a reserved id",
        ));
    }
    Ok(())
}
impl SubscriptionProfile {
    pub fn dir_name(&self) -> String {
        let dir = normalize_subscription_id(&self.dir);
        if dir.is_empty() {
            normalize_subscription_id(&self.id)
        } else {
            dir
        }
    }
    pub fn is_enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }
    pub fn is_settings_sync_enabled(&self) -> bool {
        self.settings_sync.unwrap_or(true)
    }
    pub fn has_sync_overrides(&self) -> bool {
        self.settings_sync.is_some()
            || !self.profile_owned_keys.is_empty()
            || !self.default_wins_keys.is_empty()
    }
}
pub fn find_subscription(
    profiles: &SubscriptionProfiles,
    provider: &str,
    id: &str,
) -> Option<SubscriptionProfile> {
    let id = normalize_subscription_id(id);
    if id.is_empty() {
        return None;
    }
    profiles
        .get(provider)?
        .iter()
        .find(|p| normalize_subscription_id(&p.id) == id)
        .cloned()
}
/// The caller supplies the production home explicitly; trial mode never expands `~`.
pub fn resolve_subscription_profile_dir(
    paths: &RuntimePaths,
    provider: &str,
    profile: &SubscriptionProfile,
    home: Option<&Path>,
) -> Result<PathBuf, ConfigError> {
    validate_subscription_provider(provider)?;
    validate_subscription_id(&normalize_subscription_id(&profile.id))?;
    let custom = profile.profile_dir.trim();
    if !custom.is_empty() {
        let path = if custom == "~" || custom.starts_with("~/") || custom.starts_with("~\\") {
            if paths.is_trial() {
                return Err(ConfigError::invalid(
                    "trial profile_dir cannot expand the production home",
                ));
            }
            let home = home.ok_or_else(|| {
                ConfigError::invalid("profile_dir home expansion requires an explicit home")
            })?;
            if custom == "~" {
                home.to_path_buf()
            } else {
                home.join(&custom[2..])
            }
        } else {
            PathBuf::from(custom)
        };
        if !path.is_absolute() {
            return Err(ConfigError::invalid(
                "subscription profile_dir must be an absolute path",
            ));
        }
        return if paths.is_trial() {
            checked_trial_path(paths, &path)
        } else {
            Ok(clean_path(&path))
        };
    }
    let name = profile.dir_name();
    validate_subscription_dir_name(&name)?;
    let path = paths
        .resource(Resource::Subscriptions)
        .join(provider)
        .join(name);
    if paths.is_trial() {
        checked_trial_path(paths, &path)
    } else {
        Ok(path)
    }
}
fn clean_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            c => out.push(c.as_os_str()),
        }
    }
    out
}
fn subscription_warnings(profiles: &SubscriptionProfiles) -> Vec<String> {
    let mut out = vec![];
    for (provider, list) in profiles {
        if let Err(e) = validate_subscription_provider(provider) {
            out.push(format!("subscriptions: {e}; its profiles are ignored"));
            continue;
        }
        let mut seen = BTreeSet::new();
        let mut owners = BTreeMap::new();
        for p in list {
            let id = normalize_subscription_id(&p.id);
            if let Err(e) = validate_subscription_id(&id) {
                out.push(format!(
                    "subscriptions.{provider}: {e}; this profile cannot be selected"
                ));
                continue;
            }
            if !seen.insert(id.clone()) {
                out.push(format!("subscriptions.{provider}: duplicate profile id {id:?}; only the first entry is used"));
                continue;
            }
            let custom = p.profile_dir.trim();
            if !custom.is_empty() {
                if custom != "~"
                    && !custom.starts_with("~/")
                    && !custom.starts_with("~\\")
                    && !Path::new(custom).is_absolute()
                {
                    out.push(format!("subscriptions.{provider}.{id}: profile_dir {custom:?} must be an absolute path; this profile cannot be selected"));
                }
                continue;
            }
            let name = p.dir_name();
            if let Err(e) = validate_subscription_dir_name(&name) {
                out.push(format!(
                    "subscriptions.{provider}.{id}: {e}; this profile cannot be selected"
                ));
                continue;
            }
            if let Some(owner) = owners.get(&name) {
                out.push(format!("subscriptions.{provider}.{id}: folder {name:?} is already used by profile {owner:?}; both profiles share one login"));
                continue;
            }
            owners.insert(name, id);
        }
    }
    out
}
impl CustomProvider {
    pub fn effective_label(&self) -> &str {
        if self.label.trim().is_empty() {
            &self.id
        } else {
            &self.label
        }
    }
}
fn filter_custom_providers(
    raw: &[CustomProvider],
) -> (Vec<CustomProvider>, Vec<String>, Vec<String>) {
    let mut seen = BTreeSet::new();
    let (mut kept, mut invalid, mut duplicate) = (vec![], vec![], vec![]);
    for p in raw {
        let id = normalize_subscription_id(&p.id);
        if validate_custom_provider_id(&id).is_err() || p.command.trim().is_empty() {
            invalid.push(p.id.clone());
            continue;
        }
        if !seen.insert(id.clone()) {
            duplicate.push(p.id.clone());
            continue;
        }
        let mut normalized = p.clone();
        normalized.id = id;
        kept.push(normalized);
    }
    (kept, invalid, duplicate)
}
pub fn effective_custom_providers(raw: &[CustomProvider]) -> Vec<CustomProvider> {
    filter_custom_providers(raw).0
}

pub fn sanitize_custom_themes(raw: &[UserPrefsCustomTheme]) -> Vec<UserPrefsCustomTheme> {
    let (mut seen, mut out) = (BTreeSet::new(), vec![]);
    for raw in raw {
        let id = raw.id.trim();
        if !(4..=32).contains(&id.len())
            || !id.starts_with("u-")
            || !id[2..].bytes().all(|c| c.is_ascii_alphanumeric())
        {
            continue;
        }
        let name: String = raw
            .name
            .trim()
            .chars()
            .filter(|c| *c >= '\u{20}' && *c != '\u{7f}')
            .take(16)
            .collect();
        if name.is_empty() || !seen.insert(id.to_string()) {
            continue;
        }
        out.push(UserPrefsCustomTheme {
            id: id.into(),
            name,
            mode: if raw.mode == "light" { "light" } else { "dark" }.into(),
            hue: raw.hue.clamp(0, 359),
            contrast: raw.contrast.clamp(0, 100),
        });
        if out.len() == 20 {
            break;
        }
    }
    out
}
pub fn sanitize_display_theme(raw: &str, customs: &[UserPrefsCustomTheme]) -> String {
    let value = raw.trim();
    if matches!(value, "" | "light" | "dark") || customs.iter().any(|t| t.id == value) {
        value.into()
    } else {
        "light".into()
    }
}

#[derive(Clone, Debug)]
pub struct ConfigSnapshot {
    pub revision: u64,
    pub config: Config,
}
pub struct ConfigStore {
    paths: RuntimePaths,
    state: Mutex<ConfigSnapshot>,
    recovered_from_invalid_yaml: bool,
    directory: Mutex<Option<Arc<crate::files::safe_fs::Dir>>>,
}
impl ConfigStore {
    pub fn new(paths: RuntimePaths, mut config: Config) -> Result<Self, ConfigError> {
        config.apply_defaults(&paths);
        config.validate()?;
        config.validate_paths(&paths)?;
        Ok(Self {
            paths,
            state: Mutex::new(ConfigSnapshot {
                revision: 0,
                config,
            }),
            recovered_from_invalid_yaml: false,
            directory: Mutex::new(None),
        })
    }
    fn directory(&self) -> Result<Arc<crate::files::safe_fs::Dir>, ConfigError> {
        let mut directory = self.directory.lock().map_err(|_| ConfigError::Poisoned)?;
        if let Some(held) = &*directory {
            return Ok(held.clone());
        }
        let held = Arc::new(crate::files::safe_fs::Dir::open_or_create_private(
            self.paths.root(),
        )?);
        *directory = Some(held.clone());
        Ok(held)
    }
    pub fn snapshot(&self) -> Result<ConfigSnapshot, ConfigError> {
        self.state
            .lock()
            .map(|s| s.clone())
            .map_err(|_| ConfigError::Poisoned)
    }
    /// Serializes writers; failed validation or IO never publishes the replacement.
    pub fn persist(
        &self,
        expected_revision: u64,
        mut next: Config,
    ) -> Result<ConfigSnapshot, ConfigError> {
        let mut guard = self.state.lock().map_err(|_| ConfigError::Poisoned)?;
        if expected_revision != guard.revision {
            return Err(ConfigError::Conflict {
                expected: expected_revision,
                actual: guard.revision,
            });
        }
        let revision = guard
            .revision
            .checked_add(1)
            .ok_or_else(|| ConfigError::invalid("config revision exhausted"))?;
        next.apply_defaults(&self.paths);
        next.validate()?;
        next.validate_paths(&self.paths)?;
        let payload = next.to_private_yaml()?;
        self.directory()?
            .replace("config.yaml", payload.as_bytes(), 0o600)?;
        *guard = ConfigSnapshot {
            revision,
            config: next,
        };
        Ok(guard.clone())
    }
    /// Reserves source-visible state before asynchronous profile seeding. The
    /// revision receipt prevents a failed seed from rolling back another writer.
    pub fn publish_legacy_without_persist(
        &self,
        expected_revision: u64,
        mut next: Config,
    ) -> Result<ConfigSnapshot, ConfigError> {
        let mut guard = self.state.lock().map_err(|_| ConfigError::Poisoned)?;
        if expected_revision != guard.revision {
            return Err(ConfigError::Conflict {
                expected: expected_revision,
                actual: guard.revision,
            });
        }
        let revision = guard
            .revision
            .checked_add(1)
            .ok_or_else(|| ConfigError::invalid("config revision exhausted"))?;
        next.apply_defaults(&self.paths);
        next.validate_paths(&self.paths)?;
        *guard = ConfigSnapshot {
            revision,
            config: next,
        };
        Ok(guard.clone())
    }
    /// Saves the reserved generation without republishing it. Save failures
    /// preserve visible state, matching the source's mutation-before-save flow.
    pub fn persist_published_legacy(
        &self,
        expected_revision: u64,
    ) -> Result<ConfigSnapshot, ConfigError> {
        let guard = self.state.lock().map_err(|_| ConfigError::Poisoned)?;
        if expected_revision != guard.revision {
            return Err(ConfigError::Conflict {
                expected: expected_revision,
                actual: guard.revision,
            });
        }
        guard.config.validate()?;
        let payload = guard.config.to_private_yaml()?;
        self.directory()?
            .replace("config.yaml", payload.as_bytes(), 0o600)?;
        Ok(guard.clone())
    }
    /// Source-specific Go handlers assign shared memory before calling Save.
    /// Their defaults/validation/I/O failure leaves that memory observable and
    /// returns an error to the caller. Default `persist` remains transactional.
    /// Trial confinement is checked before publication as a mandatory boundary.
    pub fn publish_then_persist_legacy(
        &self,
        expected_revision: u64,
        next: Config,
    ) -> Result<ConfigSnapshot, ConfigError> {
        self.publish_then_persist_legacy_with(expected_revision, next, |_| {})
    }
    /// The bounded in-memory reconfiguration hook runs before Save validation
    /// and disk I/O, while this revision is still serialized. It must not call
    /// ConfigStore again or perform blocking external I/O.
    pub fn publish_then_persist_legacy_with(
        &self,
        expected_revision: u64,
        mut next: Config,
        published: impl FnOnce(&Config),
    ) -> Result<ConfigSnapshot, ConfigError> {
        let mut guard = self.state.lock().map_err(|_| ConfigError::Poisoned)?;
        if expected_revision != guard.revision {
            return Err(ConfigError::Conflict {
                expected: expected_revision,
                actual: guard.revision,
            });
        }
        let revision = guard
            .revision
            .checked_add(1)
            .ok_or_else(|| ConfigError::invalid("config revision exhausted"))?;
        next.apply_defaults(&self.paths);
        next.validate_paths(&self.paths)?;
        *guard = ConfigSnapshot {
            revision,
            config: next,
        };
        published(&guard.config);
        guard.config.validate()?;
        let payload = guard.config.to_private_yaml()?;
        self.directory()?
            .replace("config.yaml", payload.as_bytes(), 0o600)?;
        Ok(guard.clone())
    }
    /// Uses only the explicit runtime root and an injected cryptographic token source.
    /// Malformed YAML is backed up privately before regeneration, matching Go.
    /// Optional-section type errors are handled before reaching this recovery path.
    pub fn load_or_create<F>(paths: RuntimePaths, mut token_factory: F) -> Result<Self, ConfigError>
    where
        F: FnMut() -> Result<String, ConfigError>,
    {
        if !paths.is_trial()
            && !paths.root().exists()
            && let Some(home) = paths.root().parent()
        {
            let legacy = home.join(".any-ai-cli");
            if legacy.is_dir() {
                let _ = std::fs::rename(legacy, paths.root());
            }
        }
        // Match Go LoadOrCreate: privacy is restored even when a valid existing
        // file needs no write. Without this, copied 0755/0644 config is exposed.
        let directory = Arc::new(crate::files::safe_fs::Dir::open_or_create_private(
            paths.root(),
        )?);
        let path = paths.resource(Resource::Config);
        if paths.is_trial() {
            checked_trial_path(&paths, &path)?;
        }
        let mut recovered = false;
        let text = directory
            .read("config.yaml", 8 * 1024 * 1024 + 1)
            .and_then(|bytes| {
                String::from_utf8(bytes).map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "config YAML contains invalid UTF-8",
                    )
                })
            });
        let (mut config, missing) = match text {
            Ok(text) => match Config::from_yaml(&text, &paths) {
                Ok(config) => (config, false),
                Err(ConfigError::Parse(_)) => {
                    directory.replace("config.yaml.bak", text.as_bytes(), 0o600)?;
                    recovered = true;
                    (Config::defaults(&paths), true)
                }
                Err(error) => return Err(error),
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => (Config::defaults(&paths), true),
            Err(e) => return Err(e.into()),
        };
        let needs_token = config.token.is_empty();
        if needs_token {
            config.token = token_factory()?;
            if config.token.is_empty() {
                return Err(ConfigError::invalid(
                    "token generator returned an empty token",
                ));
            }
        }
        if missing || needs_token {
            directory.replace("config.yaml", config.to_private_yaml()?.as_bytes(), 0o600)?;
        }
        let mut store = Self::new(paths, config)?;
        store.directory = Mutex::new(Some(directory));
        store.recovered_from_invalid_yaml = recovered;
        Ok(store)
    }
    pub fn recovered_from_invalid_yaml(&self) -> bool {
        self.recovered_from_invalid_yaml
    }
}
// Pure launch/config helpers ported from internal/config/{effort,headless,
// child_permission,launch_prompt,custom_provider_command}.go.
// Integration: append inside config/model.rs, with Config, OrchestrationConfig,
// CustomProvider, HeadlessDef, ConfigError, normalize_subscription_id, and
// effective_custom_providers in scope. CustomProvider.headless must be
// Option<HeadlessDef>. OrchestrationConfig.bounded_allowed_tools is a map from
// String to Vec<String>. Config/OrchestrationConfig/CustomProvider need Default
// for the tests. Nil Go slices map to empty Vecs; optional lookups use Option.

pub const MAX_EFFORT_LEVEL_LEN: usize = 32;
pub const EXECUTION_MODE_UNSET: &str = "";
pub const EXECUTION_MODE_AUTO: &str = "auto";
pub const EXECUTION_MODE_INTERACTIVE: &str = "interactive";
pub const EXECUTION_MODE_HEADLESS: &str = "headless";
pub const PERMISSION_PRESET_UNSET: &str = "";
pub const PERMISSION_PRESET_ATTENDED: &str = "attended";
pub const PERMISSION_PRESET_BOUNDED: &str = "bounded";
pub const PERMISSION_PRESET_FULL: &str = "full";
pub const PERMISSION_MODE_BOUNDED: &str = "bounded";
pub const MAX_ALLOWED_TOOL_VALUE_LEN: usize = 120;
pub const HEADLESS_FORMAT_TEXT: &str = "text";
pub const HEADLESS_FORMAT_CLAUDE_STREAM_JSON: &str = "claude-stream-json";
pub const HEADLESS_PROMPT_VIA_STDIN: &str = "stdin";
pub const HEADLESS_PROMPT_VIA_ARG: &str = "arg";
pub const MAX_HEADLESS_ARGS: usize = 32;
pub const MAX_HEADLESS_ARG_LEN: usize = 200;
pub const LAUNCH_ORIGIN_CONDUCTOR: &str = "";
pub const LAUNCH_ORIGIN_UI: &str = "ui";

const KNOWN_EXECUTION_MODES: &[&str] = &[
    EXECUTION_MODE_AUTO,
    EXECUTION_MODE_INTERACTIVE,
    EXECUTION_MODE_HEADLESS,
];
const AVAILABLE_EXECUTION_MODES: &[&str] = KNOWN_EXECUTION_MODES;
const KNOWN_PERMISSION_PRESETS: &[&str] = &[
    PERMISSION_PRESET_ATTENDED,
    PERMISSION_PRESET_BOUNDED,
    PERMISSION_PRESET_FULL,
];
const AVAILABLE_PERMISSION_PRESETS: &[&str] = KNOWN_PERMISSION_PRESETS;

/// One provider's effort mapping; the returned levels are an owned copy.
#[derive(Clone, Debug)]
pub struct EffortSupport {
    pub provider: String,
    pub levels: Vec<String>,
    pub accepts_any_level: bool,
    /// Call only after validate_effort has accepted the value.
    pub args: fn(&str) -> Vec<String>,
}

struct EffortSupportRow {
    provider: &'static str,
    levels: &'static [&'static str],
    accepts_any_level: bool,
    args: fn(&str) -> Vec<String>,
}

const EFFORT_SUPPORT_ROWS: &[EffortSupportRow] = &[
    EffortSupportRow {
        provider: "claude",
        levels: &["low", "medium", "high", "xhigh", "max"],
        accepts_any_level: false,
        args: |level| vec!["--effort".into(), level.into()],
    },
    EffortSupportRow {
        provider: "codex",
        levels: &["minimal", "low", "medium", "high", "xhigh", "max"],
        accepts_any_level: false,
        args: |level| vec!["-c".into(), format!("model_reasoning_effort={level:?}")],
    },
    EffortSupportRow {
        provider: "opencode",
        levels: &["low", "medium", "high"],
        accepts_any_level: true,
        args: |level| vec!["--variant".into(), level.into()],
    },
];

fn launch_strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

pub fn effort_support_for(provider: &str) -> Option<EffortSupport> {
    EFFORT_SUPPORT_ROWS
        .iter()
        .find(|row| row.provider == provider)
        .map(|row| EffortSupport {
            provider: row.provider.into(),
            levels: launch_strings(row.levels),
            accepts_any_level: row.accepts_any_level,
            args: row.args,
        })
}

pub fn effort_providers() -> Vec<String> {
    EFFORT_SUPPORT_ROWS
        .iter()
        .map(|row| row.provider.to_owned())
        .collect()
}

pub fn effort_levels_for(provider: &str) -> Vec<String> {
    effort_support_for(provider)
        .map(|row| row.levels)
        .unwrap_or_default()
}

fn valid_effort_level_shape(level: &str) -> bool {
    !level.is_empty()
        && level.len() <= MAX_EFFORT_LEVEL_LEN
        && !level.starts_with('-')
        && level
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
}

pub fn validate_effort(provider: &str, level: &str) -> Result<(), ConfigError> {
    if level.is_empty() {
        return Ok(());
    }
    let row = effort_support_for(provider).ok_or_else(|| {
        ConfigError::invalid(format!("provider {provider:?} does not support effort"))
    })?;
    if !valid_effort_level_shape(level) {
        return Err(ConfigError::invalid("invalid effort value"));
    }
    if row.accepts_any_level || row.levels.iter().any(|known| known == level) {
        return Ok(());
    }
    Err(ConfigError::invalid(format!(
        "invalid effort {level:?} for provider {provider:?}"
    )))
}

/// Like the source helper, argument construction does not validate the level.
pub fn effort_args(provider: &str, level: &str) -> Vec<String> {
    if level.is_empty() {
        return Vec::new();
    }
    effort_support_for(provider)
        .map(|row| (row.args)(level))
        .unwrap_or_default()
}

pub fn normalize_execution_mode(mode: &str) -> String {
    mode.trim().to_owned()
}

pub fn normalize_permission_preset(preset: &str) -> String {
    preset.trim().to_owned()
}

/// Schema validation deliberately does not normalize its input.
pub fn validate_execution_mode(mode: &str) -> Result<(), ConfigError> {
    if mode.is_empty() || AVAILABLE_EXECUTION_MODES.contains(&mode) {
        Ok(())
    } else {
        Err(ConfigError::invalid(format!(
            "invalid execution_mode {mode:?}"
        )))
    }
}

/// Schema validation deliberately does not normalize its input.
pub fn validate_permission_preset(preset: &str) -> Result<(), ConfigError> {
    if preset.is_empty() || AVAILABLE_PERMISSION_PRESETS.contains(&preset) {
        Ok(())
    } else {
        Err(ConfigError::invalid(format!(
            "invalid permission_preset {preset:?}"
        )))
    }
}

pub fn known_execution_modes() -> Vec<String> {
    launch_strings(KNOWN_EXECUTION_MODES)
}

pub fn available_execution_modes() -> Vec<String> {
    launch_strings(AVAILABLE_EXECUTION_MODES)
}

pub fn known_permission_presets() -> Vec<String> {
    launch_strings(KNOWN_PERMISSION_PRESETS)
}

pub fn available_permission_presets() -> Vec<String> {
    launch_strings(AVAILABLE_PERMISSION_PRESETS)
}

struct BuiltinHeadlessDef {
    provider: &'static str,
    args: &'static [&'static str],
    format: &'static str,
    prompt_via: &'static str,
}

// Codex is deliberately absent: codex exec rejects the permission flags used
// by the current child permission table. These rows select print/output mode
// only; permission, model and effort arguments belong to the launch path.
const BUILTIN_HEADLESS_DEFS: &[BuiltinHeadlessDef] = &[
    BuiltinHeadlessDef {
        provider: "claude",
        args: &["-p", "--output-format", "stream-json", "--verbose"],
        format: HEADLESS_FORMAT_CLAUDE_STREAM_JSON,
        prompt_via: HEADLESS_PROMPT_VIA_STDIN,
    },
    BuiltinHeadlessDef {
        provider: "grok",
        args: &["--output-format", "plain", "-p"],
        format: HEADLESS_FORMAT_TEXT,
        prompt_via: HEADLESS_PROMPT_VIA_ARG,
    },
    BuiltinHeadlessDef {
        provider: "cursor-agent",
        args: &["-p", "--output-format", "text"],
        format: HEADLESS_FORMAT_TEXT,
        prompt_via: HEADLESS_PROMPT_VIA_ARG,
    },
    BuiltinHeadlessDef {
        provider: "opencode",
        args: &["run", "--format", "default"],
        format: HEADLESS_FORMAT_TEXT,
        prompt_via: HEADLESS_PROMPT_VIA_ARG,
    },
    BuiltinHeadlessDef {
        provider: "copilot",
        args: &["--output-format", "text", "-s", "-p"],
        format: HEADLESS_FORMAT_TEXT,
        prompt_via: HEADLESS_PROMPT_VIA_ARG,
    },
    BuiltinHeadlessDef {
        provider: "command-code",
        args: &["--output-format", "text", "-p"],
        format: HEADLESS_FORMAT_TEXT,
        prompt_via: HEADLESS_PROMPT_VIA_ARG,
    },
];

pub fn known_headless_formats() -> Vec<String> {
    launch_strings(&[HEADLESS_FORMAT_TEXT, HEADLESS_FORMAT_CLAUDE_STREAM_JSON])
}

pub fn normalize_headless_prompt_via(via: &str) -> String {
    if via.trim() == HEADLESS_PROMPT_VIA_ARG {
        HEADLESS_PROMPT_VIA_ARG.into()
    } else {
        HEADLESS_PROMPT_VIA_STDIN.into()
    }
}

pub fn validate_headless_def(def: &HeadlessDef) -> Result<(), ConfigError> {
    let format = def.format.trim();
    if format.is_empty() {
        return Err(ConfigError::invalid("headless.format is required"));
    }
    if !known_headless_formats().iter().any(|name| name == format) {
        return Err(ConfigError::invalid(format!(
            "headless.format {format:?} is not one of {}",
            known_headless_formats().join(", ")
        )));
    }
    match def.prompt_via.trim() {
        "" | HEADLESS_PROMPT_VIA_STDIN | HEADLESS_PROMPT_VIA_ARG => {}
        _ => {
            return Err(ConfigError::invalid(format!(
                "headless.prompt_via {:?} is not {} or {}",
                def.prompt_via, HEADLESS_PROMPT_VIA_STDIN, HEADLESS_PROMPT_VIA_ARG
            )));
        }
    }
    if def.args.len() > MAX_HEADLESS_ARGS {
        return Err(ConfigError::invalid(format!(
            "headless.args has more than {MAX_HEADLESS_ARGS} entries"
        )));
    }
    for arg in &def.args {
        valid_headless_arg(arg)?;
    }
    Ok(())
}

fn valid_headless_arg(arg: &str) -> Result<(), ConfigError> {
    if arg.is_empty() {
        return Err(ConfigError::invalid("headless.args has an empty entry"));
    }
    // The source uses Go len(string), so this limit counts UTF-8 bytes.
    if arg.len() > MAX_HEADLESS_ARG_LEN {
        return Err(ConfigError::invalid(format!(
            "headless.args entry is longer than {MAX_HEADLESS_ARG_LEN} characters"
        )));
    }
    if arg.bytes().any(|c| c < 0x20 || c == 0x7f) {
        return Err(ConfigError::invalid(
            "headless.args entry contains a control character",
        ));
    }
    Ok(())
}

fn clone_headless_def(def: &HeadlessDef) -> HeadlessDef {
    HeadlessDef {
        args: def.args.clone(),
        format: def.format.trim().to_owned(),
        prompt_via: normalize_headless_prompt_via(&def.prompt_via),
    }
}

/// None uses only the built-in table, equivalent to a nil Go config pointer.
pub fn headless_def_for(provider: &str, cfg: Option<&Config>) -> Option<HeadlessDef> {
    let provider = normalize_subscription_id(provider);
    if provider.is_empty() {
        return None;
    }
    if let Some(row) = BUILTIN_HEADLESS_DEFS
        .iter()
        .find(|row| row.provider == provider)
    {
        return Some(HeadlessDef {
            args: launch_strings(row.args),
            format: row.format.into(),
            prompt_via: row.prompt_via.into(),
        });
    }
    let cfg = cfg?;
    for custom in effective_custom_providers(&cfg.custom_providers) {
        if custom.id != provider {
            continue;
        }
        if let Some(def) = &custom.headless {
            return validate_headless_def(def)
                .ok()
                .map(|()| clone_headless_def(def));
        }
    }
    None
}

pub fn headless_providers(cfg: Option<&Config>) -> Vec<String> {
    let mut providers: std::collections::BTreeSet<String> = BUILTIN_HEADLESS_DEFS
        .iter()
        .map(|row| row.provider.to_owned())
        .collect();
    if let Some(cfg) = cfg {
        for custom in effective_custom_providers(&cfg.custom_providers) {
            if custom.id.is_empty() || providers.contains(&custom.id) {
                continue;
            }
            if let Some(def) = &custom.headless
                && validate_headless_def(def).is_ok()
            {
                providers.insert(custom.id);
            }
        }
    }
    providers.into_iter().collect()
}

pub fn resolve_execution_mode(
    requested: &str,
    capable: bool,
    origin: &str,
    unattended: bool,
) -> Result<String, ConfigError> {
    match requested.trim() {
        EXECUTION_MODE_UNSET => Ok(EXECUTION_MODE_UNSET.into()),
        EXECUTION_MODE_INTERACTIVE => Ok(EXECUTION_MODE_INTERACTIVE.into()),
        EXECUTION_MODE_HEADLESS if capable => Ok(EXECUTION_MODE_HEADLESS.into()),
        EXECUTION_MODE_HEADLESS => Err(ConfigError::invalid(
            "execution_mode headless is not supported for this provider",
        )),
        EXECUTION_MODE_AUTO => {
            if capable && (unattended || origin.trim() != LAUNCH_ORIGIN_UI) {
                Ok(EXECUTION_MODE_HEADLESS.into())
            } else {
                Ok(EXECUTION_MODE_INTERACTIVE.into())
            }
        }
        _ => Err(ConfigError::invalid(format!(
            "invalid execution_mode {requested:?}"
        ))),
    }
}

pub fn is_headless_execution_mode(mode: &str) -> bool {
    mode.trim() == EXECUTION_MODE_HEADLESS
}

fn execution_mode_default(mode: &str) -> String {
    match mode.trim() {
        EXECUTION_MODE_AUTO => EXECUTION_MODE_AUTO.into(),
        EXECUTION_MODE_INTERACTIVE => EXECUTION_MODE_INTERACTIVE.into(),
        EXECUTION_MODE_HEADLESS => EXECUTION_MODE_HEADLESS.into(),
        _ => EXECUTION_MODE_UNSET.into(),
    }
}

const DEFAULT_BOUNDED_ALLOWED_TOOLS: &[(&str, &[&str])] = &[
    (
        "claude",
        &[
            "Read",
            "Glob",
            "Grep",
            "Edit",
            "Write",
            "NotebookEdit",
            "TodoWrite",
            "Bash(git status)",
            "Bash(git status *)",
            "Bash(git diff)",
            "Bash(git diff *)",
            "Bash(git log)",
            "Bash(git log *)",
            "Bash(git add *)",
            "Bash(git commit *)",
            "Bash(go test *)",
            "Bash(go vet *)",
            "Bash(gofmt *)",
            "Bash(bun run check)",
            "Bash(bun run check *)",
        ],
    ),
    (
        "copilot",
        &[
            "write",
            "shell(git status:*)",
            "shell(git diff:*)",
            "shell(git log:*)",
            "shell(git add:*)",
            "shell(git commit:*)",
            "shell(go test:*)",
            "shell(go vet:*)",
            "shell(gofmt:*)",
            "shell(bun run check:*)",
        ],
    ),
];

pub fn valid_allowed_tool_value(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ALLOWED_TOOL_VALUE_LEN
        && !matches!(value.as_bytes()[0], b'-' | b' ')
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b" ()*.:/_-".contains(&c))
}

impl OrchestrationConfig {
    pub fn child_execution_mode_default(&self) -> String {
        execution_mode_default(&self.child_execution_mode)
    }

    pub fn relay_execution_mode_default(&self) -> String {
        execution_mode_default(&self.relay_execution_mode)
    }

    pub fn child_permission_default_tier(&self) -> String {
        match self.child_permission_default.trim() {
            PERMISSION_PRESET_ATTENDED => PERMISSION_PRESET_ATTENDED.into(),
            PERMISSION_PRESET_BOUNDED => PERMISSION_PRESET_BOUNDED.into(),
            _ => PERMISSION_PRESET_FULL.into(),
        }
    }

    /// An explicit empty override suppresses the built-in default.
    pub fn bounded_allowed_tools_for(&self, provider: &str) -> Vec<String> {
        let provider = provider.trim();
        if provider.is_empty() {
            return Vec::new();
        }
        let source: Vec<&str> = match self.bounded_allowed_tools.get(provider) {
            Some(values) => values.iter().map(String::as_str).collect(),
            None => DEFAULT_BOUNDED_ALLOWED_TOOLS
                .iter()
                .find(|(name, _)| *name == provider)
                .map(|(_, values)| values.to_vec())
                .unwrap_or_default(),
        };
        source
            .into_iter()
            .map(str::trim)
            .filter(|value| valid_allowed_tool_value(value))
            .map(str::to_owned)
            .collect()
    }

    /// Config::warnings should append this after custom-provider warnings.
    /// Ordering matches Go: permission-default, sorted allowlist providers,
    /// child execution mode, then relay execution mode.
    pub fn launch_warnings(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        let raw = self.child_permission_default.trim();
        if !raw.is_empty()
            && ![
                PERMISSION_PRESET_ATTENDED,
                PERMISSION_PRESET_BOUNDED,
                PERMISSION_PRESET_FULL,
            ]
            .contains(&raw)
        {
            warnings.push(format!(
                "orchestration.child_permission_default {raw:?} is not one of attended/bounded/full; unattended children keep the built-in default ({PERMISSION_PRESET_FULL})."
            ));
        }
        let mut providers: Vec<&String> = self.bounded_allowed_tools.keys().collect();
        providers.sort();
        for provider in providers {
            let dropped: Vec<&str> = self.bounded_allowed_tools[provider]
                .iter()
                .filter(|raw| !valid_allowed_tool_value(raw.trim()))
                .map(String::as_str)
                .collect();
            if !dropped.is_empty() {
                warnings.push(format!(
                    "orchestration.bounded_allowed_tools[{provider}] has {} entry/entries that are not a tool name or command pattern and were ignored: {}",
                    dropped.len(),
                    dropped.join(" | ")
                ));
            }
        }
        for (key, value, stays) in [
            (
                "orchestration.child_execution_mode",
                self.child_execution_mode.as_str(),
                "children",
            ),
            (
                "orchestration.relay_execution_mode",
                self.relay_execution_mode.as_str(),
                "relay roles",
            ),
        ] {
            let raw = value.trim();
            if raw.is_empty() || KNOWN_EXECUTION_MODES.contains(&raw) {
                continue;
            }
            warnings.push(format!(
                "{key} {raw:?} is not one of {}; {stays} keep the built-in default (interactive).",
                KNOWN_EXECUTION_MODES.join("/")
            ));
        }
        warnings
    }
}

const LAUNCH_PROMPT_VIA_ARG_PROVIDERS: &[&str] = &["claude", "codex"];

pub fn launch_prompt_via_arg(provider: &str) -> bool {
    LAUNCH_PROMPT_VIA_ARG_PROVIDERS.contains(&normalize_subscription_id(provider).as_str())
}

/// Split without invoking or interpreting a shell. Double quotes group spans;
/// doubled quotes inside a quoted span encode one literal quote. Backslashes
/// and single quotes are literal. Only ASCII space and tab delimit tokens.
pub fn split_command_line(command: &str) -> Result<Vec<String>, ConfigError> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut token_started = false;
    let mut in_quotes = false;
    let mut chars = command.chars().peekable();
    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    current.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
                continue;
            }
            if is_command_control_rune(c) {
                return Err(command_control_error(c));
            }
            current.push(c);
            continue;
        }
        match c {
            ' ' | '\t' => {
                if token_started {
                    tokens.push(std::mem::take(&mut current));
                    token_started = false;
                }
            }
            '"' => {
                in_quotes = true;
                token_started = true;
            }
            _ => {
                if is_command_control_rune(c) {
                    return Err(command_control_error(c));
                }
                token_started = true;
                current.push(c);
            }
        }
    }
    if in_quotes {
        return Err(ConfigError::invalid(
            "custom provider command has an unterminated \"",
        ));
    }
    if token_started {
        tokens.push(current);
    }
    if tokens.is_empty() {
        return Err(ConfigError::invalid("custom provider command is empty"));
    }
    Ok(tokens)
}

fn is_command_control_rune(c: char) -> bool {
    c < '\u{20}' || c == '\u{7f}'
}

fn command_control_error(c: char) -> ConfigError {
    ConfigError::invalid(format!(
        "custom provider command contains a control character (0x{:02x})",
        u32::from(c)
    ))
}

impl CustomProvider {
    pub fn argv(&self) -> Result<Vec<String>, ConfigError> {
        split_command_line(&self.command)
    }
}

/// Public user-preferences projection using the canonical config schema.
impl UserPrefs {
    pub fn public_json(&self) -> Value {
        project(
            serde_json::to_value(self).expect("preferences are JSON-compatible"),
            "UserPrefs",
            true,
        )
    }
}

/// Decode one Go HTTP struct value, retaining duplicate/null merge semantics.
pub fn decode_user_prefs_http_json(bytes: &[u8]) -> Result<UserPrefs, serde_json::Error> {
    use crate::proto::wire;
    static SCHEMAS: std::sync::LazyLock<Vec<wire::Schema>> = std::sync::LazyLock::new(|| {
        let mut pending = vec!["UserPrefs"];
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        while let Some(kind) = pending.pop() {
            if !seen.insert(kind) {
                continue;
            }
            let fields = schema(kind);
            if fields.is_empty() {
                continue;
            }
            let mut wire_fields = Vec::new();
            for field in fields {
                if field.kind == "SessionOrderIDs" {
                    continue;
                }
                let mut inner = field.kind;
                while let Some(next) = inner
                    .strip_prefix('*')
                    .or_else(|| inner.strip_prefix("[]"))
                    .or_else(|| inner.strip_prefix("map[string]"))
                {
                    inner = next;
                }
                pending.push(inner);
                wire_fields.push(wire::Field {
                    name: field.json,
                    kind: field.kind,
                });
            }
            out.push(wire::Schema {
                name: kind,
                fields: Box::leak(wire_fields.into_boxed_slice()),
            });
        }
        out
    });
    fn yaml_names(value: Value, kind: &str) -> Value {
        if let Some(inner) = kind.strip_prefix('*') {
            return yaml_names(value, inner);
        }
        if let Some(inner) = kind.strip_prefix("[]") {
            return match value {
                Value::Array(v) => {
                    Value::Array(v.into_iter().map(|v| yaml_names(v, inner)).collect())
                }
                v => v,
            };
        }
        if let Some(inner) = kind.strip_prefix("map[string]") {
            return match value {
                Value::Object(v) => Value::Object(
                    v.into_iter()
                        .map(|(k, v)| (k, yaml_names(v, inner)))
                        .collect(),
                ),
                v => v,
            };
        }
        let Value::Object(mut object) = value else {
            return value;
        };
        if schema(kind).is_empty() {
            return Value::Object(object);
        }
        Value::Object(
            schema(kind)
                .iter()
                .filter_map(|field| {
                    object
                        .remove(field.json)
                        .map(|v| (field.yaml.into(), yaml_names(v, field.kind)))
                })
                .collect(),
        )
    }
    let raw = wire::first_http_raw(bytes)?;
    let mut value: Value = wire::decode_http_schema(raw.get().as_bytes(), "UserPrefs", &SCHEMAS)?;
    // This custom Go UnmarshalJSON deliberately ignores malformed/type-invalid
    // session-order values while keeping the rest of the preferences request.
    if let Some(members) = wire::decode_go_json_members(raw.get().as_bytes())?
        && let Some(order) = wire::last_go_raw_field(&members, "session_order")
    {
        let order = Value::Array(
            session_order_http(order)
                .into_iter()
                .map(Value::from)
                .collect(),
        );
        value
            .as_object_mut()
            .expect("struct merge returns object")
            .insert("session_order".into(), order);
    }
    let normalized = normalize_node(
        yaml_names(value, "UserPrefs"),
        "UserPrefs",
        "user_prefs",
        false,
    )
    .map_err(|_| {
        <serde_json::Error as serde::de::Error>::custom("incompatible preferences field")
    })?;
    serde_json::from_value(normalized)
}

// SessionOrderIDs's custom []any decoder validates every nested number, but
// only top-level integral numbers and numeric strings become session IDs.
// Inspect RawValue containers iteratively: ignored deep values do not acquire
// the generic map/interface projection's narrower resource limit.
fn session_order_http(bytes: &[u8]) -> Vec<i64> {
    use serde_json::value::RawValue;
    let decoded = (|| -> Result<Vec<i64>, serde_json::Error> {
        let raw = crate::proto::wire::first_http_raw(bytes)?;
        let values: Vec<Box<RawValue>> = serde_json::from_str(raw.get())?;
        let mut result = Vec::new();
        for value in &values {
            let source = value.get();
            match source.as_bytes().first() {
                Some(b'"') => {
                    let value: String = serde_json::from_str(source)?;
                    if let Ok(id) = value.trim().parse::<isize>() {
                        result.push(id as i64);
                    }
                }
                Some(b'-' | b'0'..=b'9') => {
                    let number = source.parse::<f64>().map_err(|_| {
                        <serde_json::Error as serde::de::Error>::custom(
                            "invalid session-order number",
                        )
                    })?;
                    let minimum = isize::MIN as f64;
                    let maximum_exclusive = -(isize::MIN as f64);
                    if number >= minimum && number < maximum_exclusive {
                        let id = number as isize;
                        if id as f64 == number {
                            result.push(id as i64);
                        }
                    }
                }
                _ => {}
            }
        }
        let mut pending = values;
        while let Some(value) = pending.pop() {
            let source = value.get();
            match source.as_bytes().first() {
                Some(b'[') => pending.extend(serde_json::from_str::<Vec<Box<RawValue>>>(source)?),
                Some(b'{') => {
                    if let Some(members) =
                        crate::proto::wire::decode_go_json_members(source.as_bytes())?
                    {
                        for (_, value) in members {
                            pending.push(RawValue::from_string(
                                String::from_utf8(value).expect("normalized raw JSON is UTF-8"),
                            )?);
                        }
                    }
                }
                Some(b'-' | b'0'..=b'9') => {
                    if !source.parse::<f64>().is_ok_and(f64::is_finite) {
                        return Err(<serde_json::Error as serde::de::Error>::custom(
                            "session-order number overflows float64",
                        ));
                    }
                }
                _ => {}
            }
        }
        Ok(result)
    })();
    decoded.unwrap_or_default()
}

#[cfg(test)]
mod launch_helper_tests {
    use super::*;

    #[test]
    fn effort_table_and_validation_preserve_source_contract() {
        assert_eq!(effort_providers(), ["claude", "codex", "opencode"]);
        for provider in effort_providers() {
            let row = effort_support_for(&provider).unwrap();
            assert!(!row.levels.is_empty());
            for level in row.levels {
                assert!(validate_effort(&provider, &level).is_ok());
                assert!(!effort_args(&provider, &level).is_empty());
            }
        }
        for provider in ["CLAUDE", "copilot", "grok", "shell", "my-cli", ""] {
            assert!(effort_support_for(provider).is_none());
            assert!(validate_effort(provider, "high").is_err());
            assert!(validate_effort(provider, "").is_ok());
            assert!(effort_args(provider, "high").is_empty());
        }
        for level in ["turbo", "HIGH", "minimal"] {
            assert!(validate_effort("claude", level).is_err());
        }
        assert!(validate_effort("codex", "minimal").is_ok());
        assert!(validate_effort("opencode", "thinking-2").is_ok());
        assert!(validate_effort("opencode", &"a".repeat(32)).is_ok());
        for level in [
            "--permission-mode",
            "high high",
            "high;rm",
            "high$(id)",
            "高",
        ] {
            assert!(validate_effort("opencode", level).is_err());
        }
        assert!(validate_effort("opencode", &"a".repeat(33)).is_err());
        assert_eq!(effort_args("claude", "high"), ["--effort", "high"]);
        assert_eq!(
            effort_args("codex", "xhigh"),
            ["-c", "model_reasoning_effort=\"xhigh\""]
        );
        assert_eq!(effort_args("opencode", "low"), ["--variant", "low"]);
        let mut levels = effort_levels_for("claude");
        levels[0] = "tampered".into();
        assert_eq!(effort_levels_for("claude")[0], "low");
    }

    #[test]
    fn schemas_do_not_silently_normalize() {
        for mode in ["", "auto", "interactive", "headless"] {
            assert!(validate_execution_mode(mode).is_ok());
        }
        for mode in ["AUTO", "auto ", "batch"] {
            assert!(validate_execution_mode(mode).is_err());
        }
        for preset in ["", "attended", "bounded", "full"] {
            assert!(validate_permission_preset(preset).is_ok());
        }
        for preset in ["FULL", " full", "none"] {
            assert!(validate_permission_preset(preset).is_err());
        }
        assert_eq!(normalize_execution_mode("  auto\t"), "auto");
        assert_eq!(normalize_permission_preset(" full\n"), "full");
        assert_eq!(known_execution_modes(), available_execution_modes());
        assert_eq!(known_permission_presets(), available_permission_presets());
    }

    #[test]
    fn execution_resolution_is_explicit_and_origin_sensitive() {
        for capable in [false, true] {
            for origin in ["", "ui"] {
                for unattended in [false, true] {
                    assert_eq!(
                        resolve_execution_mode("", capable, origin, unattended).unwrap(),
                        ""
                    );
                    assert_eq!(
                        resolve_execution_mode("interactive", capable, origin, unattended).unwrap(),
                        "interactive"
                    );
                    let auto = if capable && (origin != "ui" || unattended) {
                        "headless"
                    } else {
                        "interactive"
                    };
                    assert_eq!(
                        resolve_execution_mode(" auto ", capable, origin, unattended).unwrap(),
                        auto
                    );
                    assert_eq!(
                        resolve_execution_mode("headless", capable, origin, unattended).is_ok(),
                        capable
                    );
                }
            }
        }
        assert!(resolve_execution_mode("batch", true, "", false).is_err());
        assert!(!is_headless_execution_mode("auto"));
        assert!(is_headless_execution_mode(" headless "));
    }

    #[test]
    fn headless_builtins_are_owned_sorted_and_exclude_codex() {
        assert_eq!(
            headless_providers(None),
            [
                "claude",
                "command-code",
                "copilot",
                "cursor-agent",
                "grok",
                "opencode"
            ]
        );
        for provider in headless_providers(None) {
            let def = headless_def_for(&provider, None).unwrap();
            assert!(validate_headless_def(&def).is_ok());
            assert!(!def.args.is_empty());
            if provider == "claude" {
                assert_eq!(def.format, HEADLESS_FORMAT_CLAUDE_STREAM_JSON);
                assert_eq!(def.prompt_via, HEADLESS_PROMPT_VIA_STDIN);
            } else {
                assert_eq!(def.format, HEADLESS_FORMAT_TEXT);
                assert_eq!(def.prompt_via, HEADLESS_PROMPT_VIA_ARG);
            }
            for flag in [
                "--permission-mode",
                "--allowedTools",
                "--allow-tool",
                "--allow-all",
                "--ask-for-approval",
                "--sandbox",
                "--yolo",
                "--force",
                "--auto",
                "--always-approve",
                "--dangerously-bypass-approvals-and-sandbox",
                "--model",
                "--effort",
                "--reasoning-effort",
                "--variant",
            ] {
                assert!(!def.args.iter().any(|arg| arg == flag));
            }
        }
        for provider in ["codex", "shell", "", "unknown"] {
            assert!(headless_def_for(provider, None).is_none());
        }
        let mut first = headless_def_for(" CLAUDE ", None).unwrap();
        first.args[0] = "tampered".into();
        assert_eq!(headless_def_for("claude", None).unwrap().args[0], "-p");
    }

    #[test]
    fn headless_custom_definitions_are_validated_and_normalized() {
        let cfg = Config {
            custom_providers: vec![
                CustomProvider {
                    id: "my-cli".into(),
                    command: "my-cli".into(),
                    headless: Some(HeadlessDef {
                        args: vec!["--print".into()],
                        format: " text ".into(),
                        prompt_via: String::new(),
                    }),
                    ..Default::default()
                },
                CustomProvider {
                    id: "plain-cli".into(),
                    command: "plain-cli".into(),
                    ..Default::default()
                },
                CustomProvider {
                    id: "broken-cli".into(),
                    command: "broken-cli".into(),
                    headless: Some(HeadlessDef {
                        args: Vec::new(),
                        format: "yaml".into(),
                        prompt_via: String::new(),
                    }),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let def = headless_def_for("MY-CLI", Some(&cfg)).unwrap();
        assert_eq!(def.format, "text");
        assert_eq!(def.prompt_via, "stdin");
        assert!(headless_def_for("plain-cli", Some(&cfg)).is_none());
        assert!(headless_def_for("broken-cli", Some(&cfg)).is_none());
        let providers = headless_providers(Some(&cfg));
        assert!(providers.iter().any(|provider| provider == "my-cli"));
        assert!(!providers.iter().any(|provider| provider == "broken-cli"));
        assert!(!providers.iter().any(|provider| provider == "plain-cli"));
    }

    #[test]
    fn headless_validation_uses_byte_bounds_and_rejects_controls() {
        let valid = HeadlessDef {
            args: Vec::new(),
            format: "text".into(),
            prompt_via: String::new(),
        };
        assert!(validate_headless_def(&valid).is_ok());
        for (format, via) in [("", ""), ("yaml", ""), ("text", "file")] {
            let def = HeadlessDef {
                args: Vec::new(),
                format: format.into(),
                prompt_via: via.into(),
            };
            assert!(validate_headless_def(&def).is_err());
        }
        for arg in [
            String::new(),
            "-p\n-x".into(),
            "\t".into(),
            "\u{7f}".into(),
            "x".repeat(201),
            "é".repeat(101),
        ] {
            assert!(valid_headless_arg(&arg).is_err());
        }
        assert!(valid_headless_arg(&"é".repeat(100)).is_ok());
        assert!(valid_headless_arg("a shell;metacharacter").is_ok());
        let too_many = HeadlessDef {
            args: vec!["x".into(); 33],
            format: "text".into(),
            prompt_via: "stdin".into(),
        };
        assert!(validate_headless_def(&too_many).is_err());
        assert_eq!(normalize_headless_prompt_via(" arg "), "arg");
        assert_eq!(normalize_headless_prompt_via("unknown"), "stdin");
    }

    #[test]
    fn bounded_tools_keep_narrow_defaults_and_explicit_empty_overrides() {
        let mut config = OrchestrationConfig::default();
        let default = config.bounded_allowed_tools_for(" claude ");
        assert_eq!(default.len(), 20);
        assert_eq!(default[0], "Read");
        for forbidden in [
            "Bash(git push *)",
            "Bash(git reset *)",
            "Bash(git clean *)",
            "Bash(rm *)",
        ] {
            assert!(!default.iter().any(|tool| tool == forbidden));
        }
        assert_eq!(config.bounded_allowed_tools_for("copilot").len(), 10);
        for provider in ["codex", "opencode", "grok", "CLAUDE", ""] {
            assert!(config.bounded_allowed_tools_for(provider).is_empty());
        }
        config
            .bounded_allowed_tools
            .insert("claude".into(), Vec::new());
        assert!(config.bounded_allowed_tools_for("claude").is_empty());
        config.bounded_allowed_tools.insert(
            "claude".into(),
            vec![
                " Read ".into(),
                "--oops".into(),
                "Read,Edit".into(),
                "Edit".into(),
            ],
        );
        assert_eq!(config.bounded_allowed_tools_for("claude"), ["Read", "Edit"]);
        for value in [
            "Read",
            "Bash(git log *)",
            "shell(git status:*)",
            "Bash(go test ./...)",
        ] {
            assert!(valid_allowed_tool_value(value));
        }
        for value in [
            "",
            "--allow-all",
            " Read",
            "Read;rm -rf /",
            "Read,Edit",
            "Read|Edit",
            "Bash(echo $HOME)",
            "Read\nEdit",
            "読み",
        ] {
            assert!(!valid_allowed_tool_value(value));
        }
        assert!(valid_allowed_tool_value(&"a".repeat(120)));
        assert!(!valid_allowed_tool_value(&"a".repeat(121)));
    }

    #[test]
    fn orchestration_defaults_and_launch_warning_order_are_stable() {
        let mut config = OrchestrationConfig::default();
        assert_eq!(config.child_permission_default_tier(), "full");
        assert_eq!(config.child_execution_mode_default(), "");
        assert_eq!(config.relay_execution_mode_default(), "");
        assert!(config.launch_warnings().is_empty());
        for (raw, expected) in [
            ("", "full"),
            ("whatever", "full"),
            ("Bounded", "full"),
            (" bounded ", "bounded"),
            ("attended", "attended"),
        ] {
            config.child_permission_default = raw.into();
            assert_eq!(config.child_permission_default_tier(), expected);
        }
        config.child_permission_default = "boundeed".into();
        config.child_execution_mode = "batch".into();
        config.relay_execution_mode = "HEADLESS".into();
        config
            .bounded_allowed_tools
            .insert("z-cli".into(), vec!["--bad".into()]);
        config
            .bounded_allowed_tools
            .insert("a-cli".into(), vec!["Read".into(), "\t".into()]);
        let warnings = config.launch_warnings();
        assert_eq!(warnings.len(), 5);
        for (index, setting) in [
            "child_permission_default",
            "bounded_allowed_tools[a-cli]",
            "bounded_allowed_tools[z-cli]",
            "child_execution_mode",
            "relay_execution_mode",
        ]
        .iter()
        .enumerate()
        {
            assert!(warnings[index].contains(setting));
        }
        for mode in ["auto", "interactive", "headless"] {
            config.child_execution_mode = format!(" {mode} ");
            config.relay_execution_mode = format!("\t{mode}\n");
            assert_eq!(config.child_execution_mode_default(), mode);
            assert_eq!(config.relay_execution_mode_default(), mode);
        }
    }

    #[test]
    fn launch_prompt_arguments_are_only_for_measured_providers() {
        for provider in ["claude", "codex", " CLAUDE "] {
            assert!(launch_prompt_via_arg(provider));
        }
        for provider in [
            "copilot",
            "cursor-agent",
            "opencode",
            "grok",
            "command-code",
            "shell",
            "gemini",
            "my-cli",
            "",
        ] {
            assert!(!launch_prompt_via_arg(provider));
        }
    }

    #[test]
    fn command_splitter_preserves_literal_data_and_empty_quoted_arguments() {
        let cases: &[(&str, &[&str])] = &[
            ("my-cli --agent x", &["my-cli", "--agent", "x"]),
            ("  my-cli  --agent\t\tx  ", &["my-cli", "--agent", "x"]),
            (r"C:\a\b.exe", &[r"C:\a\b.exe"]),
            (
                r#""C:\Program Files\my cli\cli.exe" --flag"#,
                &[r"C:\Program Files\my cli\cli.exe", "--flag"],
            ),
            (r#"--path="C:\a b\c""#, &[r"--path=C:\a b\c"]),
            (r#"--flag="a""b""#, &["--flag=a\"b"]),
            (r#"mycli """#, &["mycli", ""]),
            (r#""""#, &[""]),
            ("it's-fine", &["it's-fine"]),
            (
                "mycli $HOME %APPDATA% ~ * | && ; > <",
                &[
                    "mycli",
                    "$HOME",
                    "%APPDATA%",
                    "~",
                    "*",
                    "|",
                    "&&",
                    ";",
                    ">",
                    "<",
                ],
            ),
            ("mycli\u{a0}arg", &["mycli\u{a0}arg"]),
            ("mycli \u{85}", &["mycli", "\u{85}"]),
        ];
        for (input, expected) in cases {
            assert_eq!(split_command_line(input).unwrap(), launch_strings(expected));
        }
        let provider = CustomProvider {
            command: "my-cli --agent".into(),
            ..Default::default()
        };
        assert_eq!(provider.argv().unwrap(), ["my-cli", "--agent"]);
    }

    #[test]
    fn command_splitter_rejects_unterminated_quotes_and_ascii_controls() {
        for input in [
            "",
            "  \t  ",
            "my-cli \"unterminated",
            "my-cli\nnewline",
            "my-cli\u{0}",
            "my-cli\u{7f}",
            "my-cli \"a\tb\"",
        ] {
            assert!(split_command_line(input).is_err(), "{input:?}");
        }
        let error = split_command_line("my-cli\n").unwrap_err();
        assert!(error.to_string().contains("control character (0x0a)"));
    }
}
