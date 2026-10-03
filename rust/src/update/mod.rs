//! Provider updater. One immutable resolved plan supplies eligibility, preview,
//! logs, and the executed process, including a distinct configured updater.
pub mod job;
pub mod plan;
#[cfg(test)]
mod tests;
