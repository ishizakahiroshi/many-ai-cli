//! Provider data contracts from the frozen Go baseline (21d0bc7).
//! This module does not load registries, execute commands, or validate new writes.
//! Executable candidates in `ResolvedLaunch` have not undergone path resolution;
//! callers use `crate::proto::core::ProviderCommandPlan` and its
//! `crate::process::ProcessPlan` for the actual resolved process boundary.
//!
//! JSON follows encoding/json: pointer false is distinct from absent, value
//! structs remain `{}` even with omitempty, required slices/maps preserve nil,
//! and unknown fields and unknown string-enum values are accepted on read.
//! Command-bearing records deliberately do not implement Debug.

use super::{is_false, is_zero, null_default};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;

pub const CURRENT_SCHEMA_VERSION: i64 = 1;
pub const BUILTIN_PROVIDER_IDS: &[&str] = &[
    "claude",
    "codex",
    "copilot",
    "cursor-agent",
    "opencode",
    "grok",
    "command-code",
];

macro_rules! open_string {
    ($name:ident) => {
        #[derive(
            Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub String);
        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }
            pub fn is_empty(&self) -> bool {
                self.0.is_empty()
            }
        }
        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }
        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }
        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}
open_string!(Origin);
open_string!(Severity);
open_string!(AdapterKind);
pub const ORIGIN_EMBEDDED: &str = "embedded";
pub const ORIGIN_DISTRIBUTION: &str = "distribution";
pub const ORIGIN_USER: &str = "user";
pub const ORIGIN_LEGACY: &str = "legacy";
pub const ORIGIN_OVERRIDE: &str = "override";
pub const SEVERITY_WARNING: &str = "warning";
pub const SEVERITY_ERROR: &str = "error";
pub const ADAPTER_LAUNCH: &str = "launch";
pub const ADAPTER_APPROVAL: &str = "approval";
pub const ADAPTER_HISTORY: &str = "history";
pub const ADAPTER_USAGE: &str = "usage";
pub const ADAPTER_SUBSCRIPTION: &str = "subscription";
pub const ADAPTER_PERMISSIONS: &str = "permissions";
pub const ADAPTER_SUBAGENTS: &str = "subagent";

/// The Go catalog's map values are empty JSON objects, not JSON null.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmptyObject {}

// encoding/json accepts a null element for a nonpointer value and leaves zero.
fn go_optional_vec<'de, D, T>(deserializer: D) -> Result<Option<Vec<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<Vec<Option<T>>>::deserialize(deserializer)?
        .map(|items| items.into_iter().map(Option::unwrap_or_default).collect()))
}
fn go_vec<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(go_optional_vec(deserializer)?.unwrap_or_default())
}
fn go_optional_map<'de, D, T>(deserializer: D) -> Result<Option<BTreeMap<String, T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(
        Option::<BTreeMap<String, Option<T>>>::deserialize(deserializer)?.map(|items| {
            items
                .into_iter()
                .map(|(key, value)| (key, value.unwrap_or_default()))
                .collect()
        }),
    )
}
fn go_map<'de, D, T>(deserializer: D) -> Result<BTreeMap<String, T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(go_optional_map(deserializer)?.unwrap_or_default())
}

/// Source: `internal/provider/schema.go`, `SourceRef`.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SourceRef {
    #[serde(
        rename = "origin",
        deserialize_with = "null_default",
        skip_serializing_if = "Origin::is_empty"
    )]
    pub origin: Origin,
    #[serde(
        rename = "version",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub version: String,
    #[serde(
        rename = "digest",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub digest: String,
    #[serde(
        rename = "revision",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub revision: String,
}

/// Source: `internal/provider/schema.go`, `Definition`.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Definition {
    #[serde(
        rename = "schema_version",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub schema_version: i64,
    #[serde(
        rename = "id",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub id: String,
    #[serde(
        rename = "display_name",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub display_name: String,
    #[serde(
        rename = "description",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub description: String,
    #[serde(rename = "enabled", skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(rename = "launch", skip_serializing_if = "Option::is_none")]
    pub launch: Option<LaunchDefinition>,
    #[serde(rename = "models", skip_serializing_if = "Option::is_none")]
    pub models: Option<ModelsDefinition>,
    #[serde(
        rename = "capabilities",
        deserialize_with = "go_map",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub capabilities: BTreeMap<String, bool>,
    #[serde(rename = "adapters", deserialize_with = "null_default")]
    pub adapters: AdapterRefs,
    #[serde(rename = "presentation", skip_serializing_if = "Option::is_none")]
    pub presentation: Option<PresentationDefinition>,
    #[serde(
        rename = "approval_pattern_source",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub approval_pattern_source: String,
    #[serde(rename = "source", deserialize_with = "null_default")]
    pub source: SourceRef,
    #[serde(rename = "update", skip_serializing_if = "Option::is_none")]
    pub update: Option<UpdateDefinition>,
}

/// Source: `internal/provider/schema.go`, `LaunchDefinition`.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LaunchDefinition {
    #[serde(
        rename = "executable",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub executable: String,
    #[serde(
        rename = "executable_candidates",
        deserialize_with = "go_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub executable_candidates: Vec<String>,
    #[serde(
        rename = "args",
        deserialize_with = "go_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub args: Vec<String>,
    #[serde(
        rename = "model_args",
        deserialize_with = "go_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub model_args: Vec<String>,
    #[serde(
        rename = "effort_args",
        deserialize_with = "go_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub effort_args: Vec<String>,
    #[serde(
        rename = "effort_levels",
        deserialize_with = "go_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub effort_levels: Vec<String>,
    #[serde(
        rename = "allowed_env",
        deserialize_with = "go_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub allowed_env: Vec<String>,
    #[serde(rename = "headless", skip_serializing_if = "Option::is_none")]
    pub headless: Option<HeadlessDefinition>,
}

/// Source: `internal/provider/schema.go`, `ModelsDefinition`.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelsDefinition {
    #[serde(
        rename = "allow_custom",
        deserialize_with = "null_default",
        skip_serializing_if = "is_false"
    )]
    pub allow_custom: bool,
    #[serde(
        rename = "items",
        deserialize_with = "go_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub items: Vec<ModelDefinition>,
    #[serde(
        rename = "source",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub source: String,
}

/// Source: `internal/provider/schema.go`, `ModelDefinition`.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelDefinition {
    #[serde(
        rename = "id",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub id: String,
    #[serde(
        rename = "label",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub label: String,
}

/// Source: `internal/provider/schema.go`, `HeadlessDefinition`.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HeadlessDefinition {
    #[serde(
        rename = "args",
        deserialize_with = "go_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub args: Vec<String>,
    #[serde(
        rename = "format",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub format: String,
    #[serde(
        rename = "prompt_via",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub prompt_via: String,
}

/// Source: `internal/provider/schema.go`, `PresentationDefinition`.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PresentationDefinition {
    #[serde(
        rename = "icon_text",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub icon_text: String,
    #[serde(
        rename = "color",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub color: String,
}

/// Source: `internal/provider/schema.go`, `UpdateDefinition`.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateDefinition {
    #[serde(
        rename = "version_args",
        deserialize_with = "go_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub version_args: Vec<String>,
    #[serde(
        rename = "args",
        deserialize_with = "go_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub args: Vec<String>,
    #[serde(
        rename = "executable",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub executable: String,
    #[serde(rename = "enabled", skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(
        rename = "login_may_be_required",
        deserialize_with = "null_default",
        skip_serializing_if = "is_false"
    )]
    pub login_may_be_required: bool,
    #[serde(
        rename = "timeout_seconds",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub timeout_seconds: i64,
}

/// Source: `internal/provider/schema.go`, `AdapterRefs`.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AdapterRefs {
    #[serde(
        rename = "launch",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub launch: String,
    #[serde(
        rename = "approval",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub approval: String,
    #[serde(
        rename = "transcript",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub transcript: String,
    #[serde(
        rename = "usage",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub usage: String,
    #[serde(
        rename = "subscription",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub subscription: String,
    #[serde(
        rename = "permissions",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub permissions: String,
    #[serde(
        rename = "subagents",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub subagents: String,
}

/// Source: `internal/provider/schema.go`, `Layers`.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layers {
    #[serde(rename = "Embedded", deserialize_with = "go_optional_vec")]
    pub embedded: Option<Vec<Definition>>,
    #[serde(rename = "AcceptedDistribution", deserialize_with = "go_optional_vec")]
    pub accepted_distribution: Option<Vec<Definition>>,
    #[serde(rename = "Distribution", deserialize_with = "go_optional_vec")]
    pub distribution: Option<Vec<Definition>>,
    #[serde(rename = "Legacy", deserialize_with = "go_optional_vec")]
    pub legacy: Option<Vec<Definition>>,
    #[serde(rename = "User", deserialize_with = "go_optional_vec")]
    pub user: Option<Vec<Definition>>,
    #[serde(rename = "Overrides", deserialize_with = "go_optional_vec")]
    pub overrides: Option<Vec<Definition>>,
}

/// Source: `internal/provider/schema.go`, `AdapterCatalog`.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AdapterCatalog {
    #[serde(rename = "Keys", deserialize_with = "go_optional_map")]
    pub keys: Option<BTreeMap<String, EmptyObject>>,
}

/// Source: `internal/provider/schema.go`, `CapabilitySummary`.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CapabilitySummary {
    #[serde(rename = "launch", deserialize_with = "null_default")]
    pub launch: bool,
    #[serde(rename = "models", deserialize_with = "null_default")]
    pub models: bool,
    #[serde(rename = "effort", deserialize_with = "null_default")]
    pub effort: bool,
    #[serde(rename = "headless", deserialize_with = "null_default")]
    pub headless: bool,
    #[serde(rename = "approval", deserialize_with = "null_default")]
    pub approval: bool,
    #[serde(rename = "transcript", deserialize_with = "null_default")]
    pub transcript: bool,
    #[serde(rename = "usage", deserialize_with = "null_default")]
    pub usage: bool,
    #[serde(rename = "subscription", deserialize_with = "null_default")]
    pub subscription: bool,
    #[serde(rename = "permissions", deserialize_with = "null_default")]
    pub permissions: bool,
    #[serde(rename = "subagents", deserialize_with = "null_default")]
    pub subagents: bool,
}

/// Source: `internal/provider/schema.go`, `EffectiveDefinition`.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EffectiveDefinition {
    #[serde(flatten)]
    pub definition: Definition,
    #[serde(rename = "effective_source", deserialize_with = "null_default")]
    pub effective_source: SourceRef,
    #[serde(
        rename = "field_origins",
        deserialize_with = "go_map",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub field_origins: BTreeMap<String, SourceRef>,
    #[serde(rename = "revision", deserialize_with = "null_default")]
    pub revision: String,
    #[serde(rename = "capabilities_summary", deserialize_with = "null_default")]
    pub capabilities: CapabilitySummary,
}

/// Source: `internal/provider/schema.go`, `Summary`.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Summary {
    #[serde(rename = "id", deserialize_with = "null_default")]
    pub id: String,
    #[serde(rename = "display_name", deserialize_with = "null_default")]
    pub display_name: String,
    #[serde(rename = "enabled", deserialize_with = "null_default")]
    pub enabled: bool,
    #[serde(rename = "origin", deserialize_with = "null_default")]
    pub origin: Origin,
    #[serde(rename = "revision", deserialize_with = "null_default")]
    pub revision: String,
    #[serde(rename = "capabilities", deserialize_with = "null_default")]
    pub capabilities: CapabilitySummary,
    #[serde(rename = "presentation", skip_serializing_if = "Option::is_none")]
    pub presentation: Option<PresentationDefinition>,
    #[serde(
        rename = "icon_image_version",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub icon_image_version: String,
}

/// Source: `internal/provider/schema.go`, `Diagnostic`.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Diagnostic {
    #[serde(rename = "code", deserialize_with = "null_default")]
    pub code: String,
    #[serde(rename = "severity", deserialize_with = "null_default")]
    pub severity: Severity,
    #[serde(
        rename = "field",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub field: String,
    #[serde(rename = "message", deserialize_with = "null_default")]
    pub message: String,
}

/// Source: `internal/provider/launch.go`, `LaunchRequest`.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LaunchRequest {
    #[serde(rename = "Model", deserialize_with = "null_default")]
    pub model: String,
    #[serde(rename = "Effort", deserialize_with = "null_default")]
    pub effort: String,
    #[serde(rename = "SessionID", deserialize_with = "null_default")]
    pub session_id: String,
    #[serde(rename = "Prompt", deserialize_with = "null_default")]
    pub prompt: String,
    #[serde(rename = "Headless", deserialize_with = "null_default")]
    pub headless: bool,
}

/// Source: `internal/provider/launch.go`, `ResolvedLaunch`.
/// Unresolved executable candidates and argv; never execute this DTO directly.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ResolvedLaunch {
    #[serde(rename = "ExecutableCandidates", deserialize_with = "go_optional_vec")]
    pub executable_candidates: Option<Vec<String>>,
    #[serde(rename = "Args", deserialize_with = "go_optional_vec")]
    pub args: Option<Vec<String>>,
    #[serde(rename = "AllowedEnv", deserialize_with = "go_optional_vec")]
    pub allowed_env: Option<Vec<String>>,
    #[serde(rename = "Revision", deserialize_with = "null_default")]
    pub revision: String,
}

/// Source: `internal/provider/adapters.go`, `AdapterDescriptor`.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AdapterDescriptor {
    #[serde(rename = "key", deserialize_with = "null_default")]
    pub key: String,
    #[serde(rename = "kind", deserialize_with = "null_default")]
    pub kind: AdapterKind,
    #[serde(rename = "version", deserialize_with = "null_default")]
    pub version: String,
    #[serde(
        rename = "provider",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub provider: String,
}

/// Source: `internal/provider/distribution.go`, `DistributionPayload`.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DistributionPayload {
    #[serde(rename = "schema_version", deserialize_with = "null_default")]
    pub schema_version: i64,
    #[serde(rename = "catalog_version", deserialize_with = "null_default")]
    pub catalog_version: String,
    #[serde(rename = "created_at", deserialize_with = "null_default")]
    pub created_at: String,
    #[serde(
        rename = "minimum_app_version",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub minimum_app_version: String,
    #[serde(rename = "definitions", deserialize_with = "go_optional_vec")]
    pub definitions: Option<Vec<Definition>>,
    #[serde(rename = "digests", deserialize_with = "go_optional_map")]
    pub digests: Option<BTreeMap<String, String>>,
}

/// Source: `internal/provider/distribution.go`, `DistributionBundle`.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DistributionBundle {
    #[serde(rename = "payload", deserialize_with = "null_default")]
    pub payload: DistributionPayload,
    #[serde(rename = "key_id", deserialize_with = "null_default")]
    pub key_id: String,
    #[serde(rename = "signature", deserialize_with = "null_default")]
    pub signature: String,
}

/// Source: `internal/provider/distribution.go`, `DistributionStatus`.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DistributionStatus {
    #[serde(rename = "catalog_version", deserialize_with = "null_default")]
    pub catalog_version: String,
    #[serde(rename = "digest", deserialize_with = "null_default")]
    pub digest: String,
    #[serde(rename = "state", deserialize_with = "null_default")]
    pub state: String,
    #[serde(
        rename = "path",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub path: String,
}

/// Source: `internal/provider/distribution_diff.go`, `DistributionFieldDiff`.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DistributionFieldDiff {
    #[serde(rename = "field", deserialize_with = "null_default")]
    pub field: String,
    #[serde(
        rename = "current",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub current: String,
    #[serde(
        rename = "candidate",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub candidate: String,
    #[serde(
        rename = "override",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub r#override: String,
    #[serde(rename = "conflict", deserialize_with = "null_default")]
    pub conflict: bool,
}

/// Source: `internal/provider/distribution_diff.go`, `DistributionProviderDiff`.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DistributionProviderDiff {
    #[serde(rename = "id", deserialize_with = "null_default")]
    pub id: String,
    #[serde(rename = "status", deserialize_with = "null_default")]
    pub status: String,
    #[serde(
        rename = "changes",
        deserialize_with = "go_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub changes: Vec<DistributionFieldDiff>,
}

/// Source: `internal/provider/history.go`, `RevisionRecord`.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RevisionRecord {
    #[serde(rename = "schema_version", deserialize_with = "null_default")]
    pub schema_version: i64,
    #[serde(rename = "provider_id", deserialize_with = "null_default")]
    pub provider_id: String,
    #[serde(rename = "revision", deserialize_with = "null_default")]
    pub revision: String,
    #[serde(rename = "created_at", deserialize_with = "null_default")]
    pub created_at: String,
    #[serde(rename = "reason", deserialize_with = "null_default")]
    pub reason: String,
    #[serde(
        rename = "parent_revision",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub parent_revision: String,
    #[serde(rename = "content_digest", deserialize_with = "null_default")]
    pub content_digest: String,
    #[serde(rename = "payload", deserialize_with = "null_default")]
    pub payload: Definition,
}

/// Source: `internal/provider/recovery.go`, `QuarantineRecord`.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct QuarantineRecord {
    #[serde(rename = "original_name", deserialize_with = "null_default")]
    pub original_name: String,
    #[serde(rename = "quarantined", deserialize_with = "null_default")]
    pub quarantined: String,
    #[serde(rename = "detected_at", deserialize_with = "null_default")]
    pub detected_at: String,
    #[serde(rename = "reason", deserialize_with = "null_default")]
    pub reason: String,
    #[serde(rename = "digest", deserialize_with = "null_default")]
    pub digest: String,
}

impl Diagnostic {
    pub fn is_error(&self) -> bool {
        self.severity.as_str() == SEVERITY_ERROR
    }
}

/// Pure catalog helpers from schema.go; no executable names are adapter keys.
impl AdapterCatalog {
    pub fn new<I, S>(keys: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            keys: Some(
                keys.into_iter()
                    .map(Into::into)
                    .filter(|key| !key.is_empty())
                    .map(|key| (key, EmptyObject {}))
                    .collect(),
            ),
        }
    }
    pub fn has(&self, key: &str) -> bool {
        matches!(key, "" | "none" | "unsupported")
            || self
                .keys
                .as_ref()
                .is_some_and(|keys| keys.contains_key(key))
    }
}

pub fn default_adapter_catalog() -> AdapterCatalog {
    AdapterCatalog::new(DEFAULT_ADAPTER_KEYS.iter().copied())
}

pub const DEFAULT_ADAPTER_KEYS: &[&str] = &[
    "approval:claude-v1",
    "approval:codex-v1",
    "approval:copilot-v1",
    "approval:cursor-agent-v1",
    "approval:opencode-v1",
    "approval:grok-v1",
    "approval:command-code-v1",
    "approval:generic-v1",
    "history:claude-v1",
    "history:codex-v1",
    "history:cursor-agent-v1",
    "history:opencode-v1",
    "history:command-code-v1",
    "usage:claude-v1",
    "usage:codex-v1",
    "usage:opencode-v1",
    "usage:grok-v1",
    "subscription:claude-v1",
    "subscription:codex-v1",
    "subscription:opencode-v1",
    "subscription:grok-v1",
    "launch:generic-v1",
    "permissions:generic-v1",
    "subagent:claude-v1",
    "subagent:codex-v1",
    "subagent:grok-v1",
];

pub fn builtin_order(id: &str) -> usize {
    BUILTIN_PROVIDER_IDS
        .iter()
        .position(|builtin| *builtin == id)
        .unwrap_or(BUILTIN_PROVIDER_IDS.len())
}

/// Built-ins first in shipped order, custom identifiers lexicographically.
pub fn sort_ids(ids: &mut [String]) {
    ids.sort_by(|a, b| {
        builtin_order(a)
            .cmp(&builtin_order(b))
            .then_with(|| a.cmp(b))
    });
}

/// Encode typed provider DTOs using the Go baseline's JSON byte contract.
///
/// Hashes and signed distribution payloads must use this helper rather than a
/// serde_json::Value intermediate: declaration field order and HTML/JavaScript
/// escapes are part of Go encoding/json's deterministic digest input. Callers
/// still apply the source's provider ordering before encoding a collection.
pub fn to_go_json<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, serde_json::Error> {
    let json = serde_json::to_string(value)?;
    let mut escaped = String::with_capacity(json.len());
    for character in json.chars() {
        match character {
            '<' => escaped.push_str("\\u003c"),
            '>' => escaped.push_str("\\u003e"),
            '&' => escaped.push_str("\\u0026"),
            '\u{2028}' => escaped.push_str("\\u2028"),
            '\u{2029}' => escaped.push_str("\\u2029"),
            other => escaped.push(other),
        }
    }
    Ok(escaped.into_bytes())
}
