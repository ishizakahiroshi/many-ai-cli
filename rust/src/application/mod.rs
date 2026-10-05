//! Main application composition and shared runtime ledger. Binary dispatch is
//! integrated only when every invoked component owns its real side effects.
pub mod agent_history;
pub mod diagnostic_cli;
pub mod diagnostics;
pub mod hub_runtime;
pub mod hub_status;
pub mod issue_cli;
pub mod mobile_connect;
pub mod nvidia_nim;
pub mod orchestrate_cli;

pub mod launcher_program;
pub mod runtime_context;

pub mod wrapped_spawn;

pub mod approval_patterns;
pub mod approval_rules;
pub mod cli_updates;
pub mod event_observer;
pub mod grid_spawn;
pub mod host_actions;
pub mod instruction_rules;
pub mod link_defaults;
pub mod main_program;
pub mod maintenance_cli;
pub mod maintenance_entry;
pub mod orchestration_program;
pub mod ordinary_spawn;
pub mod output_model;
pub mod platform_cli;
pub mod profile_export;
pub mod provider_assets;
pub mod push;
pub mod session_observations;
pub mod session_usage;
pub mod session_workers;
pub mod slash_commands;
pub mod spawn_policy;
pub mod subscriptions;
pub mod tray;
pub mod usage_hooks;
pub mod usage_relay;
pub mod whisper;
