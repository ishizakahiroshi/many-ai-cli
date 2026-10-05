//! HTTP and WebSocket service boundary for the migration candidate.
//! `route_coverage.json` records unresolved operations; a route inventory alone
//! must never be read as a claim that the application is implemented.
pub mod assets;
pub mod auth;
pub mod http;
pub mod network;
pub mod pin;
pub mod router;
pub mod settings;
pub mod sockets;
pub mod transport;
pub mod websocket;
pub use router::{Dispatch, ServiceRouter};

#[cfg(test)]
mod tests;

pub mod routines;

pub mod approval_actions;

pub mod task_owner;

pub mod agent_history_routes;
pub mod approval_pattern_routes;
pub mod approval_status_routes;
pub mod auto_approval;
pub mod child_control;
pub mod child_routes;
pub mod confirmations;
pub mod diagnostic_routes;
pub mod distribution_routes;
pub mod handoff_routes;
pub mod host_routes;
pub mod icon_routes;
pub mod jev_routes;
pub mod mobile_routes;
pub mod nvidia_routes;
pub mod push_routes;
pub mod runtime_routes;
pub mod server_routes;
pub mod slash_routes;
pub mod subscription_routes;
pub mod update_routes;
pub mod usage_routes;
pub mod voice_routes;
pub mod whisper_routes;

pub mod provider_routes;
#[cfg(test)]
mod provider_transport_tests;
pub mod session_routes;

#[cfg(test)]
mod approval_order_tests;
pub mod cli_version;
#[cfg(test)]
mod cli_version_transport_tests;
pub mod grid_spawn_routes;
pub mod lifecycle;
#[cfg(test)]
mod notify_http_tests;
pub mod notify_routes;
pub mod preference_media;
pub mod preferences;
pub mod spawn_routes;
