//! Main application composition and shared runtime ledger. Binary dispatch is
//! integrated only when every invoked component owns its real side effects.
pub mod hub_runtime;

pub mod launcher_program;
pub mod runtime_context;

pub mod wrapped_spawn;
