//! Provider configuration/profile services. Vendor defaults are read-only inputs.
pub mod persistence;
mod provider_schema;
pub mod registry;
pub mod seed;
#[cfg(test)]
mod tests;
pub mod validation;
