//! Explicit resource roots and private persistence. No trial operation resolves the real home.
pub mod paths;
pub mod private_io;
pub use paths::{Resource, RuntimePaths};

#[cfg(windows)]
mod private_windows;

pub mod model;
pub use model::*;

pub mod provider_legacy;
pub use provider_legacy::{legacy_provider_definitions, provider_definitions_for_app};

mod yaml_nodes;
pub use yaml_nodes::{YamlDecodeError, YamlField, YamlSchema, decode_yaml_schema};

pub mod backup;
pub mod removed;
pub mod safety;
pub use backup::{BackupRecord, ConfigBackups, is_partial_failure};
pub use safety::{
    CHECK_COMMAND, ConfigFileState, ConfigProblem, ConfigUnreadable, ProblemCode, ProblemText,
    ResetError, ResetOptions, ResetOutcome, TokenOutcome, inspect_file, reset_config,
    reset_config_with, unreadable_from_io,
};
