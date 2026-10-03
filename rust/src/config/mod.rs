//! Explicit resource roots and private persistence. No trial operation resolves the real home.
pub mod paths;
pub mod private_io;
pub use paths::{Resource, RuntimePaths};

#[cfg(windows)]
mod private_windows;

pub mod model;
pub use model::*;

pub mod provider_legacy;
pub use provider_legacy::legacy_provider_definitions;

mod yaml_nodes;
pub use yaml_nodes::{YamlDecodeError, YamlField, YamlSchema, decode_yaml_schema};
