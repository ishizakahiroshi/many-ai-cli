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
