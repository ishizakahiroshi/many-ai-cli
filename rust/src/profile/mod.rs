//! Provider configuration/profile services. Vendor defaults are read-only inputs.
pub mod capability_report;
pub mod declared;
pub mod persistence;
pub(crate) mod provider_schema;
pub mod registry;
pub mod seed;
#[cfg(test)]
mod tests;
pub mod validation;

pub mod store;
pub mod subscriptions;
