//! Agent Client Protocol (ACP) client foundation.
//!
//! The client starts an agent as a child process and talks to it over stdio:
//! JSON-RPC 2.0, one message per line (`\n`, no embedded newlines), exactly as
//! the public ACP transport page describes.
//!
//! Boundaries this module keeps (see the plan C9 and its research note):
//!
//! - It only exchanges protocol messages. Sign-in stays inside the agent's own
//!   runtime. The client never reads, extracts, refreshes or relays any
//!   credential, and nothing in this directory refers to credential files
//!   (`tests::acp_sources_do_not_touch_credentials` fails if that changes).
//! - The child gets a minimal, allow-listed environment (`spawn::minimal_env`).
//!   The Hub's secret and its session variables are never passed on.
//! - `session/request_permission` is only ever answered by a decision the
//!   caller supplies (`AcpClient::answer_permission`). The one exception is
//!   what the protocol itself requires: when the caller cancels a turn, the
//!   still-open permission requests of that session are answered `cancelled`.
//! - Nothing here retries, resends or restarts. When the process ends or the
//!   stream breaks, every pending request finishes with an error.
//! - Who may call the client (a person at the screen, never a timer, a child
//!   session or another AI) is enforced by the Hub entry points, not here.

mod client;
mod framing;
mod spawn;
mod types;

#[cfg(test)]
pub(crate) mod mock_agent;
#[cfg(test)]
mod tests;

pub use client::{AcpClient, AcpEvents, Incoming, text_block};
pub use framing::{LineDecoder, MAX_LINE_BYTES, encode_message};
pub use spawn::{AcpProcessSpec, minimal_env, spawn_process};
pub use types::{
    AcpError, AcpEvent, AgentCapabilities, AuthMethod, InitializeResult, PermissionDecision,
    PermissionOption, PermissionRequest, SessionInfo, SessionList,
};
