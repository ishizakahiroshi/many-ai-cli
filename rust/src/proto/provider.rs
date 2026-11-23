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
    // Declarative sections (Rust only; the frozen Go baseline has none of them).
    // Every field is skipped when empty, so a definition that does not use them
    // serializes, hashes and diffs exactly as before. The embedded manifests
    // leave them empty; the values of the seven built-ins live in
    // `profile::declared` so the Go oracle revision of the embedded layer is
    // unchanged.
    /// `""` or `"pty"` (the default) or `"acp"`. An `acp` provider is never
    /// reachable from an automated path.
    #[serde(
        rename = "connection",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub connection: String,
    #[serde(rename = "slash_commands", skip_serializing_if = "Option::is_none")]
    pub slash_commands: Option<SlashCommandsDefinition>,
    #[serde(rename = "permissions", skip_serializing_if = "Option::is_none")]
    pub permissions: Option<PermissionsDefinition>,
    #[serde(rename = "login", skip_serializing_if = "Option::is_none")]
    pub login: Option<LoginDefinition>,
    #[serde(rename = "links", skip_serializing_if = "Option::is_none")]
    pub links: Option<LinksDefinition>,
    #[serde(rename = "input", skip_serializing_if = "Option::is_none")]
    pub input: Option<InputDefinition>,
    #[serde(rename = "routines", skip_serializing_if = "Option::is_none")]
    pub routines: Option<RoutinesDefinition>,
    #[serde(rename = "screen", skip_serializing_if = "Option::is_none")]
    pub screen: Option<ScreenDefinition>,
    #[serde(rename = "transcript", skip_serializing_if = "Option::is_none")]
    pub transcript: Option<TranscriptDefinition>,
    #[serde(rename = "approval", skip_serializing_if = "Option::is_none")]
    pub approval: Option<ApprovalDefinition>,
    #[serde(rename = "instructions", skip_serializing_if = "Option::is_none")]
    pub instructions: Option<InstructionsDefinition>,
}

/// Where the Hub hands a provider the approval and done marker rules.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct InstructionsDefinition {
    /// The name of a Markdown file in the project root that the CLI always
    /// reads as instructions (`AGENTS.md`). While a session of this provider
    /// runs, the Hub keeps its rules block in that file and takes it out when
    /// the last such session ends. Empty = the Hub writes nothing for it.
    #[serde(
        rename = "rules_file",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub rules_file: String,
}

/// How the native approval prompt of a provider is recognised on its screen.
/// Every field is optional; a provider that declares nothing gets the plain
/// numbered-option detection (`1. Yes` / `2. No`). Sent keys are in `send`
/// (empty = the key text itself).
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ApprovalDefinition {
    /// `numbered` (default: lines like `1. Yes`) or `buttons` (one line of
    /// wording-matched buttons chosen with the right arrow key).
    #[serde(
        rename = "style",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub style: String,
    /// Extra option line shapes, tried after `numbered` in this order:
    /// `paren_marker` (`1 (•) Yes`), `bracket_keys` (`Run once (y)` looked up in
    /// `bracket_keys`), `key_lines` (`Approve (y)` looked up in `keys`).
    #[serde(
        rename = "line_extras",
        deserialize_with = "go_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub line_extras: Vec<String>,
    /// Single keys that answer the prompt. A numbered option whose label is, or
    /// ends with, `(key)` gets the key as its send text; with `key_lines` a
    /// line `label (key)` is an option of its own (numbered by `num`).
    #[serde(
        rename = "keys",
        deserialize_with = "go_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub keys: Vec<ApprovalKey>,
    /// Lines `label (key)` whose key is any of the listed spellings (compared
    /// in lower case), for providers that show their keys in free wording.
    #[serde(
        rename = "bracket_keys",
        deserialize_with = "go_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub bracket_keys: Vec<ApprovalKey>,
    /// Button rows for `style: buttons`, tried from the last matching line up.
    #[serde(
        rename = "buttons",
        deserialize_with = "go_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub buttons: Vec<ApprovalButtons>,
    /// The options are chosen by moving the cursor: each option's send text is
    /// the arrow keys from the highlighted option, then Enter.
    #[serde(
        rename = "arrow_send",
        deserialize_with = "null_default",
        skip_serializing_if = "is_false"
    )]
    pub arrow_send: bool,
    /// The provider shows a one-line `1 to review, 2 to send, 0 to dismiss`
    /// feedback survey that counts as a prompt.
    #[serde(
        rename = "feedback_survey",
        deserialize_with = "null_default",
        skip_serializing_if = "is_false"
    )]
    pub feedback_survey: bool,
    /// A prompt that carries send keys is of kind `native_<id>_shortcut`.
    #[serde(
        rename = "shortcut_kind",
        deserialize_with = "null_default",
        skip_serializing_if = "is_false"
    )]
    pub shortcut_kind: bool,
    /// Lower case wording that, anywhere in the prompt's text, shows that it asks
    /// for an approval.
    #[serde(
        rename = "hint_phrases",
        deserialize_with = "go_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub hint_phrases: Vec<String>,
    /// Lower case wording of screens that look like a prompt but only choose a
    /// model: a line with one of these never counts as an approval hint.
    #[serde(
        rename = "not_approval_phrases",
        deserialize_with = "go_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub not_approval_phrases: Vec<String>,
    /// Screens that are never a prompt: all of `all` and any of `any` appear.
    #[serde(
        rename = "suppress_screens",
        deserialize_with = "go_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub suppress_screens: Vec<ApprovalSuppress>,
}

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ApprovalKey {
    #[serde(
        rename = "keys",
        deserialize_with = "go_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub keys: Vec<String>,
    #[serde(rename = "num", deserialize_with = "null_default")]
    pub num: i64,
    #[serde(
        rename = "send",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub send: String,
}

/// A row of buttons: the last line holding every `line_all` word starts a
/// prompt whose first line is the nearest line (up to 12 lines above) holding
/// `start_line` (required, or else 12 lines above is used).
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ApprovalButtons {
    #[serde(
        rename = "line_all",
        deserialize_with = "go_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub line_all: Vec<String>,
    #[serde(
        rename = "start_line",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub start_line: String,
    #[serde(
        rename = "start_required",
        deserialize_with = "null_default",
        skip_serializing_if = "is_false"
    )]
    pub start_required: bool,
    #[serde(
        rename = "labels",
        deserialize_with = "go_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub labels: Vec<String>,
}

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ApprovalSuppress {
    #[serde(
        rename = "all",
        deserialize_with = "go_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub all: Vec<String>,
    #[serde(
        rename = "any",
        deserialize_with = "go_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub any: Vec<String>,
}

/// Where the raw conversation file of a provider lives, how a Hub session is
/// matched to it, and (optionally) how to read it as a chat. Paths are written
/// as patterns that start with `{home}` or `{env:NAME}`; `{cwd_slug:alnum}` and
/// `{cwd_slug:path}` insert the session's working directory as a folder name.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TranscriptDefinition {
    /// Directory that holds the conversation files.
    #[serde(
        rename = "root",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub root: String,
    /// One file shared by every session (location only, never read as a chat).
    #[serde(
        rename = "file",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub file: String,
    /// File extension without the dot (`json` by default).
    #[serde(
        rename = "extension",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub extension: String,
    /// Ways to find the session's file, tried in order: `session_id` (the file
    /// is `<agent session id>.<extension>`), `meta` (the first line of each file
    /// names the working directory and start time), `index` (a SQLite table
    /// maps working directory and start time to the file name).
    #[serde(
        rename = "find",
        deserialize_with = "go_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub find: Vec<String>,
    /// `uuid` requires the session id to be a UUID; empty accepts a plain
    /// file-name-safe id.
    #[serde(
        rename = "id_format",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub id_format: String,
    /// When no file is found, point at `root` itself if it exists.
    #[serde(
        rename = "dir_fallback",
        deserialize_with = "null_default",
        skip_serializing_if = "is_false"
    )]
    pub dir_fallback: bool,
    /// Text shown for the located file.
    #[serde(
        rename = "label",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub label: String,
    /// Text shown when only the folder is located.
    #[serde(
        rename = "dir_label",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub dir_label: String,
    /// Text shown when the pattern cannot be resolved for this session.
    #[serde(
        rename = "unavailable_reason",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub unavailable_reason: String,
    /// Text shown when no file matches the session (yet).
    #[serde(
        rename = "not_found_reason",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub not_found_reason: String,
    #[serde(rename = "meta", skip_serializing_if = "Option::is_none")]
    pub meta: Option<TranscriptMeta>,
    #[serde(rename = "index", skip_serializing_if = "Option::is_none")]
    pub index: Option<TranscriptIndex>,
    #[serde(rename = "reader", skip_serializing_if = "Option::is_none")]
    pub reader: Option<TranscriptReader>,
}

/// Keys (dotted paths) read from the first line of each candidate file. Only a
/// line whose `kind_key` equals `kind_value` counts (both optional).
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TranscriptMeta {
    #[serde(
        rename = "kind_key",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub kind_key: String,
    #[serde(
        rename = "kind_value",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub kind_value: String,
    #[serde(
        rename = "cwd_key",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub cwd_key: String,
    #[serde(
        rename = "time_key",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub time_key: String,
}

/// SQLite table (opened read-only) that maps a session to its file name.
/// `time_column` holds Unix seconds (`s`, default) or milliseconds (`ms`).
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TranscriptIndex {
    #[serde(
        rename = "file",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub file: String,
    #[serde(
        rename = "table",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub table: String,
    #[serde(
        rename = "id_column",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub id_column: String,
    #[serde(
        rename = "cwd_column",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub cwd_column: String,
    #[serde(
        rename = "time_column",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub time_column: String,
    #[serde(
        rename = "time_unit",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub time_unit: String,
}

/// How to read the file as a chat. `format` is `json` (one document; `messages`
/// is the dotted path of the array) or `jsonl` (one message per line). `roles`
/// maps the role value in the file to `user` or `assistant`; other roles are
/// skipped.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TranscriptReader {
    #[serde(
        rename = "format",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub format: String,
    #[serde(
        rename = "messages",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub messages: String,
    #[serde(
        rename = "role_key",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub role_key: String,
    #[serde(
        rename = "text_key",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub text_key: String,
    #[serde(
        rename = "time_key",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub time_key: String,
    #[serde(
        rename = "roles",
        deserialize_with = "go_map",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub roles: BTreeMap<String, String>,
}

/// Where the slash command candidates of a provider come from. At most one of
/// `source` (an https URL that serves a Markdown table) and `items` is needed;
/// `items` wins when both are present.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SlashCommandsDefinition {
    #[serde(
        rename = "source",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub source: String,
    #[serde(
        rename = "items",
        deserialize_with = "go_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub items: Vec<SlashCommandItem>,
    /// Prefix that starts a skill invocation (`/` by default, `$` for codex).
    #[serde(
        rename = "skill_prefix",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub skill_prefix: String,
}

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SlashCommandItem {
    #[serde(rename = "cmd", deserialize_with = "null_default")]
    pub cmd: String,
    #[serde(
        rename = "desc",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub desc: String,
}

/// Permission mode name -> extra launch arguments. The mode `*` is used for a
/// mode that is not listed, and `{mode}` in an argument is replaced by the mode
/// name. `allowed_tools_args` turns an allowed tool list into arguments: an
/// entry that is exactly `{tools}` becomes every tool as its own argument, an
/// entry that contains `{tool}` is repeated once per tool.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PermissionsDefinition {
    #[serde(
        rename = "modes",
        deserialize_with = "go_map",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub modes: BTreeMap<String, Vec<String>>,
    #[serde(
        rename = "allowed_tools_args",
        deserialize_with = "go_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub allowed_tools_args: Vec<String>,
    /// The permission modes the new-session form offers, in order. Empty: the
    /// form shows no permission selector for this provider.
    #[serde(
        rename = "choices",
        deserialize_with = "go_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub choices: Vec<String>,
}

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LoginDefinition {
    #[serde(
        rename = "args",
        deserialize_with = "go_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub args: Vec<String>,
}

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LinksDefinition {
    /// https URL of the provider's usage / billing page.
    #[serde(
        rename = "usage_url",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub usage_url: String,
}

/// Terminal input habits of the inner CLI.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct InputDefinition {
    /// The CLI drops a quickly pasted body: wait longer before the Enter.
    #[serde(
        rename = "slow_paste",
        deserialize_with = "null_default",
        skip_serializing_if = "is_false"
    )]
    pub slow_paste: bool,
    /// `esc` (default) or `ctrl-c`: the key that interrupts a running turn.
    #[serde(
        rename = "interrupt_key",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub interrupt_key: String,
    /// How an attached file is written into the prompt: `path` (the bare path,
    /// default), `at-path` (`@path`) or `at-path-slash` (`@path` with `/`
    /// separators).
    #[serde(
        rename = "attach_format",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub attach_format: String,
    /// How a one-line `/command` is submitted: `once` (default) or `paste`
    /// (paste the body, then a separate Enter).
    #[serde(
        rename = "slash_submit",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub slash_submit: String,
    /// The CLI paints its body and its frames with absolute cursor moves, so the
    /// screen must not drop those writes as status bar updates.
    #[serde(
        rename = "full_frame_cursor",
        deserialize_with = "null_default",
        skip_serializing_if = "is_false"
    )]
    pub full_frame_cursor: bool,
    /// The CLI does not read Ctrl+U as "clear the line"; sending it breaks the
    /// submit that follows.
    #[serde(
        rename = "no_line_clear",
        deserialize_with = "null_default",
        skip_serializing_if = "is_false"
    )]
    pub no_line_clear: bool,
}

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RoutinesDefinition {
    /// Whether a routine (scheduled run) may use this provider. Absent = yes
    /// for a provider with a `launch` definition and a PTY connection.
    #[serde(rename = "enabled", skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// `marker` (default) or `native_or_marker`.
    #[serde(
        rename = "completion_mode",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub completion_mode: String,
}

/// A provider's screen signals, compared with the screen text after every
/// whitespace character is removed (so `Ask me anything` matches `Askmeanything`).
/// The built-ins carry theirs in `assets/provider/declarations.json`; a
/// registered AI CLI declares its own. One shape for every provider.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScreenDefinition {
    /// Text on the screen once the CLI is ready for a prompt (its input box).
    #[serde(
        rename = "ready_markers",
        deserialize_with = "go_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub ready_markers: Vec<String>,
    /// Text of a startup screen that holds the CLI before its input box (a
    /// folder-trust question, an update notice). The Hub tells the conductor
    /// the child is waiting there instead of typing the instructions into it.
    #[serde(
        rename = "startup_blockers",
        deserialize_with = "go_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub startup_blockers: Vec<String>,
    /// The glyph that starts the input line (for example `❯`). With it the Hub
    /// reads the text still sitting in the input box after a submit and presses
    /// Enter again when the pasted prompt was not taken.
    #[serde(
        rename = "composer_prefix",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub composer_prefix: String,
    /// Text on a line below the input line that ends the input box (a footer or
    /// status line). A rule line of box-drawing characters always ends it.
    #[serde(
        rename = "composer_end_markers",
        deserialize_with = "go_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub composer_end_markers: Vec<String>,
}

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelListDefinition {
    /// Arguments of the command that prints the model list.
    #[serde(
        rename = "args",
        deserialize_with = "go_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub args: Vec<String>,
    #[serde(
        rename = "timeout_seconds",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub timeout_seconds: i64,
    /// Extra arguments appended when the user asks for a refresh.
    #[serde(
        rename = "refresh_args",
        deserialize_with = "go_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub refresh_args: Vec<String>,
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
    /// Approval mode (`plan` / `acceptEdits` / `auto` / `bypassPermissions`) ->
    /// the provider's own argv for it. A mode without an entry is not offered
    /// for this provider, so an unmapped flag is never guessed.
    #[serde(
        rename = "permission_args",
        deserialize_with = "go_map",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub permission_args: BTreeMap<String, Vec<String>>,
    #[serde(
        rename = "allowed_env",
        deserialize_with = "go_vec",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub allowed_env: Vec<String>,
    #[serde(rename = "headless", skip_serializing_if = "Option::is_none")]
    pub headless: Option<HeadlessDefinition>,
    /// Name of the environment variable that moves the CLI's config directory
    /// (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, ...). Rust-only declarative field.
    #[serde(
        rename = "config_dir_env",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub config_dir_env: String,
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
    /// Command that prints the model list. Rust-only declarative field.
    #[serde(rename = "list", skip_serializing_if = "Option::is_none")]
    pub list: Option<ModelListDefinition>,
    /// Default context window in tokens (0 = unknown). Rust-only declarative field.
    #[serde(
        rename = "context_limit",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub context_limit: i64,
    /// Regular expression (one capture group: the model) that matches the line
    /// the CLI prints when the model is changed in the session. Rust-only.
    #[serde(
        rename = "change_pattern",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub change_pattern: String,
    /// The captured model may end in `with <effort> effort`; split it off.
    #[serde(
        rename = "change_effort_suffix",
        deserialize_with = "null_default",
        skip_serializing_if = "is_false"
    )]
    pub change_effort_suffix: bool,
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
    /// One-letter badge used by the pane and session list. Rust-only.
    #[serde(
        rename = "letter",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub letter: String,
    /// Position in the provider list (smaller first; 0 = after the others).
    #[serde(
        rename = "order",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub order: i64,
    /// Position in the "hand this session to" list (smaller first; 0 = after the
    /// others). Rust-only.
    #[serde(
        rename = "handoff_order",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub handoff_order: i64,
    /// Short name the lists, badges and cards show. Rust-only.
    #[serde(
        rename = "label",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub label: String,
    /// Full product name the launch pickers and role lists show. Rust-only.
    #[serde(
        rename = "long_label",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub long_label: String,
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
    /// The CLI opens a suggestion popup for "/" input and consumes the first
    /// Enter to pick it: the Hub wraps "/" text in bracketed paste and defers
    /// the Enter. Set with `capabilities.slash_suggestion_popup` in the
    /// definition; no adapter provides it.
    #[serde(rename = "slash_suggestion_popup", deserialize_with = "null_default")]
    pub slash_suggestion_popup: bool,
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
