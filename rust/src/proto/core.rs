//! C1 shared contracts, grounded in Go baseline 21d0bc7.
//! Implementations belong to C2/C3. These interfaces perform no I/O and do not
//! expose successful placeholder routes. Wire records retain Go field spelling.
//!
//! Live and durable IDs are intentionally not interchangeable:
//! ```compile_fail
//! use many_ai_cli::proto::core::{DbSessionId, LiveSessionId};
//! let database_id: DbSessionId = LiveSessionId(7);
//! ```
//! Replay/approval/storage generations are separate domains:
//! ```compile_fail
//! use many_ai_cli::proto::core::{ApprovalSourceEpoch, ReplayEpoch};
//! let approval_epoch: ApprovalSourceEpoch = ReplayEpoch(7);
//! ```
//! Trusted UI origin is never deserialized from a request:
//! ```compile_fail
//! use many_ai_cli::proto::core::VerifiedUiOrigin;
//! let proof: VerifiedUiOrigin = serde_json::from_str(r#"{"origin":"ui"}"#).unwrap();
//! ```
use super::{is_false, is_zero, null_default};
use crate::proto::time::Timestamp;
use crate::{
    config::RuntimePaths,
    process::{Cancellation, ProcessPlan},
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, future::Future, pin::Pin, sync::Arc, time::Duration};

pub type CoreFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
/// Only genuinely heterogeneous Go event payloads and message metadata use JSON.
pub type JsonObject = serde_json::Map<String, serde_json::Value>;

macro_rules! number_id {
    ($name:ident, $repr:ty) => {
        #[derive(
            Clone,
            Copy,
            Debug,
            Default,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
            Serialize,
            Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub $repr);
    };
}
number_id!(LiveSessionId, i64);
number_id!(DbSessionId, i64);
number_id!(WrapperConnectionId, u64);
number_id!(UiConnectionId, u64);
number_id!(SessionIncarnation, u64);
number_id!(InputSeq, i64);
number_id!(ReplayEpoch, u64);
number_id!(ApprovalSourceEpoch, u64);
number_id!(ApprovalStateVersion, u64);
number_id!(HistoryGeneration, u64);
number_id!(AuthEpoch, u64);
number_id!(EventSequence, u64);
number_id!(SpawnAttemptId, u64);
number_id!(ProviderUpdateId, u64);
number_id!(RelayProgressId, i64);
number_id!(ApprovalReservationId, u64);
macro_rules! text_id {
    ($name:ident) => {
        #[derive(
            Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub String);
    };
}
text_id!(OrchestrationId);
text_id!(AdmissionId);
text_id!(SpawnConfirmationId);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionBinding {
    pub session: LiveSessionId,
    pub incarnation: SessionIncarnation,
    pub wrapper: WrapperConnectionId,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UiBinding {
    pub connection: UiConnectionId,
    pub auth_epoch: AuthEpoch,
}

/// Source: `internal/hub/normal_worktree.go`, `normalWorktree`.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NormalWorktree {
    #[serde(rename = "Path", deserialize_with = "null_default")]
    pub path: String,
    #[serde(rename = "ParentDir", deserialize_with = "null_default")]
    pub parent_dir: String,
    #[serde(rename = "Branch", deserialize_with = "null_default")]
    pub branch: String,
    #[serde(rename = "Created", deserialize_with = "null_default")]
    pub created: bool,
}

/// Source: `internal/hub/server.go`, `session`.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionSnapshot {
    #[serde(rename = "id", deserialize_with = "null_default")]
    pub id: LiveSessionId,
    #[serde(rename = "provider", deserialize_with = "null_default")]
    pub provider: String,
    #[serde(
        rename = "provider_revision",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub provider_revision: String,
    #[serde(rename = "display_name", deserialize_with = "null_default")]
    pub display: String,
    #[serde(rename = "cwd", deserialize_with = "null_default")]
    pub cwd: String,
    #[serde(
        rename = "branch",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub branch: String,
    #[serde(
        rename = "project_id",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub project_id: String,
    #[serde(
        rename = "label",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub label: String,
    #[serde(
        rename = "launch_label",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub launch_label: String,
    #[serde(
        rename = "pinned",
        deserialize_with = "null_default",
        skip_serializing_if = "is_false"
    )]
    pub pinned: bool,
    #[serde(
        rename = "color",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub color: String,
    #[serde(
        rename = "note",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub note: String,
    #[serde(
        rename = "auto_title",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub auto_title: String,
    #[serde(
        rename = "model",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub model: String,
    #[serde(
        rename = "effort",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub effort: String,
    #[serde(
        rename = "execution_mode",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub execution_mode: String,
    #[serde(
        rename = "permission_mode",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub permission_mode: String,
    #[serde(
        rename = "route",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub route: String,
    #[serde(
        rename = "shell",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub shell: String,
    #[serde(
        rename = "parent_session_id",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub parent_session_id: LiveSessionId,
    #[serde(
        rename = "handoff_from",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub handoff_from: LiveSessionId,
    #[serde(
        rename = "role",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub role: String,
    #[serde(
        rename = "auto",
        deserialize_with = "null_default",
        skip_serializing_if = "is_false"
    )]
    pub auto: bool,
    #[serde(
        rename = "depth",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub depth: i64,
    #[serde(
        rename = "orchestration_id",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub orchestration_id: OrchestrationId,
    #[serde(
        rename = "board_path",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub board_path: String,
    #[serde(
        rename = "worktree_branch",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub worktree_branch: String,
    #[serde(rename = "NormalWorktree", deserialize_with = "null_default")]
    pub normal_worktree: NormalWorktree,
    #[serde(rename = "WorktreeCleanup", deserialize_with = "null_default")]
    pub worktree_cleanup: String,
    #[serde(
        rename = "board_notify_pending",
        deserialize_with = "null_default",
        skip_serializing_if = "is_false"
    )]
    pub board_notify_pending: bool,
    #[serde(
        rename = "relays",
        deserialize_with = "null_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub relays: Vec<Option<super::RelayStatus>>,
    #[serde(
        rename = "cross_session_messages",
        deserialize_with = "null_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub cross_session_messages: Vec<super::CrossSessionMessage>,
    #[serde(rename = "activity", deserialize_with = "null_default")]
    pub activity: super::SessionActivity,
    #[serde(rename = "state", deserialize_with = "null_default")]
    pub state: String,
    #[serde(
        rename = "last_output_at",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub last_output_at: String,
    #[serde(
        rename = "transcript_grew_at",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub transcript_grew_at: String,
    #[serde(
        rename = "started_at",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub started_at: String,
    /// Rust-only browser correlation. Only trusted spawn metadata populates this field.
    #[serde(
        default,
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub client_request_id: String,
    #[serde(
        rename = "first_message",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub first_message: String,
    #[serde(
        rename = "last_message",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub last_message: String,
    #[serde(
        rename = "end_reason",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub end_reason: String,
    #[serde(
        rename = "subscription_profile_id",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub subscription_profile_id: String,
    #[serde(
        rename = "subscription_profile_name",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub subscription_profile_name: String,
    #[serde(
        rename = "log_path",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub log_path: String,
    #[serde(
        rename = "jsonl_path",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub jsonl_path: String,
}

/// Source: `internal/sessionstore/store.go`, `SessionStart`.
#[derive(Clone, Default, PartialEq)]
pub struct SessionStart {
    pub live_session_id: LiveSessionId,
    pub provider: String,
    pub display: String,
    pub cwd: String,
    pub branch: String,
    pub label: String,
    pub model: String,
    pub route: String,
    pub shell: String,
    pub state: String,
    pub started_at: String,
    pub log_path: String,
    pub jsonl_path: String,
    pub parent_session_id: LiveSessionId,
    pub role: String,
    pub auto: bool,
    pub depth: i64,
    pub orchestration_id: OrchestrationId,
    pub board_path: String,
    pub worktree_branch: String,
    pub subscription_id: String,
}

#[derive(Clone, Default)]
pub struct SessionOrchestrationMeta {
    pub parent: LiveSessionId,
    pub role: String,
    pub auto: bool,
    pub depth: i64,
    pub orchestration: OrchestrationId,
    pub board_path: String,
}

/// Source sessionMetaPatch: omitted/null fields do not replace existing values.
#[derive(Clone, Default)]
pub struct SessionCardMetaPatch {
    pub label: Option<String>,
    pub pinned: Option<bool>,
    pub color: Option<String>,
    pub note: Option<String>,
}
pub struct SessionCardMetaUpdate {
    pub meta: SessionCardMeta,
    /// Persist first; only after success route `notification` through broadcast_ui.
    pub effects: CoreEffects,
    pub notification: super::Message,
}
/// Source: `internal/sessionstore/store.go`, `SessionCardMeta`.
#[derive(Clone, Default, PartialEq)]
pub struct SessionCardMeta {
    pub label: String,
    pub pinned: bool,
    pub color: String,
    pub note: String,
    pub auto_title: String,
}

/// Source: `internal/sessionstore/store.go`, `ChatMessage`.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChatMessage {
    #[serde(rename = "id", deserialize_with = "null_default")]
    pub id: i64,
    #[serde(
        rename = "session_db_id",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub session_id: DbSessionId,
    #[serde(
        rename = "session_id",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub live_session_id: LiveSessionId,
    #[serde(rename = "ts", deserialize_with = "null_default")]
    pub ts: String,
    #[serde(rename = "role", deserialize_with = "null_default")]
    pub role: String,
    #[serde(rename = "kind", deserialize_with = "null_default")]
    pub kind: String,
    #[serde(rename = "rawText", deserialize_with = "null_default")]
    pub raw_text: String,
    #[serde(
        rename = "normalizedText",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub normalized_text: String,
    #[serde(
        rename = "attachments",
        deserialize_with = "null_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub attachments: Vec<AttachmentRef>,
    #[serde(
        rename = "meta",
        deserialize_with = "null_default",
        skip_serializing_if = "serde_json::Map::is_empty"
    )]
    pub meta: JsonObject,
}

/// Source: `internal/sessionstore/store.go`, `AttachmentRef`.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AttachmentRef {
    #[serde(
        rename = "path",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub path: String,
    #[serde(
        rename = "filename",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub filename: String,
    #[serde(
        rename = "kind",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub kind: String,
}

/// Source: `internal/sessionstore/store.go`, `ApprovalRow`.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ApprovalRow {
    #[serde(rename = "id", deserialize_with = "null_default")]
    pub id: i64,
    #[serde(rename = "session_db_id", deserialize_with = "null_default")]
    pub session_db_id: DbSessionId,
    #[serde(rename = "session_id", deserialize_with = "null_default")]
    pub live_session_id: LiveSessionId,
    #[serde(
        rename = "provider",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub provider: String,
    #[serde(
        rename = "cwd",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub cwd: String,
    #[serde(rename = "sig", deserialize_with = "null_default")]
    pub sig: String,
    #[serde(
        rename = "source",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub source: String,
    #[serde(
        rename = "kind",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub kind: String,
    #[serde(
        rename = "question",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub question: String,
    #[serde(
        rename = "context",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub context: String,
    #[serde(
        rename = "block",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub block: String,
    #[serde(
        rename = "candidate_key",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub candidate_key: String,
    #[serde(
        rename = "source_epoch",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub source_epoch: ApprovalSourceEpoch,
    #[serde(
        rename = "options",
        deserialize_with = "null_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub options: Vec<super::ApprovalOption>,
    #[serde(
        rename = "selected_text",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub selected_text: String,
    #[serde(rename = "state", deserialize_with = "null_default")]
    pub state: String,
    #[serde(
        rename = "detected_at",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub detected_at: String,
    #[serde(
        rename = "resolved_at",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub resolved_at: String,
}

/// Source: `internal/sessionstore/store.go`, `SearchResult`.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchResult {
    #[serde(rename = "message_id", deserialize_with = "null_default")]
    pub message_id: i64,
    #[serde(rename = "session_db_id", deserialize_with = "null_default")]
    pub session_db_id: DbSessionId,
    #[serde(rename = "session_id", deserialize_with = "null_default")]
    pub live_session_id: LiveSessionId,
    #[serde(rename = "provider", deserialize_with = "null_default")]
    pub provider: String,
    #[serde(rename = "cwd", deserialize_with = "null_default")]
    pub cwd: String,
    #[serde(
        rename = "branch",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub branch: String,
    #[serde(
        rename = "model",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub model: String,
    #[serde(
        rename = "state",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub state: String,
    #[serde(
        rename = "started_at",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub started_at: String,
    #[serde(rename = "ts", deserialize_with = "null_default")]
    pub ts: String,
    #[serde(rename = "role", deserialize_with = "null_default")]
    pub role: String,
    #[serde(rename = "kind", deserialize_with = "null_default")]
    pub kind: String,
    #[serde(rename = "text", deserialize_with = "null_default")]
    pub text: String,
    #[serde(rename = "snippet", deserialize_with = "null_default")]
    pub snippet: String,
}

/// Source: `internal/sessionstore/store.go`, `SessionOverview`.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionOverview {
    #[serde(rename = "id", deserialize_with = "null_default")]
    pub id: DbSessionId,
    #[serde(rename = "session_id", deserialize_with = "null_default")]
    pub live_session_id: LiveSessionId,
    #[serde(
        rename = "provider",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub provider: String,
    #[serde(
        rename = "display_name",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub display: String,
    #[serde(
        rename = "cwd",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub cwd: String,
    #[serde(
        rename = "branch",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub branch: String,
    #[serde(
        rename = "label",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub label: String,
    #[serde(
        rename = "model",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub model: String,
    #[serde(
        rename = "route",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub route: String,
    #[serde(
        rename = "shell",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub shell: String,
    #[serde(
        rename = "state",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub state: String,
    #[serde(
        rename = "started_at",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub started_at: String,
    #[serde(
        rename = "last_output_at",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub last_output_at: String,
    #[serde(
        rename = "ended_at",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub ended_at: String,
    #[serde(
        rename = "first_message",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub first_message: String,
    #[serde(
        rename = "last_message",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub last_message: String,
    #[serde(
        rename = "end_reason",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub end_reason: String,
    #[serde(
        rename = "title",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub title: String,
    #[serde(
        rename = "tags",
        deserialize_with = "null_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub tags: Vec<String>,
    #[serde(
        rename = "summary",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub summary: String,
    #[serde(rename = "archived", deserialize_with = "null_default")]
    pub archived: bool,
    #[serde(
        rename = "log_path",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub log_path: String,
    #[serde(
        rename = "jsonl_path",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub jsonl_path: String,
    #[serde(
        rename = "parent_session_id",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub parent_session_id: LiveSessionId,
    #[serde(
        rename = "role",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub role: String,
    #[serde(
        rename = "auto",
        deserialize_with = "null_default",
        skip_serializing_if = "is_false"
    )]
    pub auto: bool,
    #[serde(
        rename = "depth",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub depth: i64,
    #[serde(
        rename = "orchestration_id",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub orchestration_id: OrchestrationId,
    #[serde(
        rename = "board_path",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub board_path: String,
    #[serde(
        rename = "worktree_branch",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub worktree_branch: String,
    #[serde(
        rename = "subscription_profile_id",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub subscription_id: String,
    #[serde(
        rename = "message_count",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub message_count: i64,
    #[serde(
        rename = "event_count",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub event_count: i64,
    #[serde(
        rename = "approval_count",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub approval_count: i64,
    #[serde(
        rename = "pending_count",
        deserialize_with = "null_default",
        skip_serializing_if = "is_zero"
    )]
    pub pending_count: i64,
}

/// Source: `internal/sessionstore/store.go`, `TimelineEvent`.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TimelineEvent {
    #[serde(rename = "id", deserialize_with = "null_default")]
    pub id: i64,
    #[serde(rename = "session_db_id", deserialize_with = "null_default")]
    pub session: DbSessionId,
    #[serde(
        rename = "ts",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub ts: String,
    #[serde(rename = "type", deserialize_with = "null_default")]
    pub r#type: String,
    #[serde(
        rename = "payload",
        deserialize_with = "null_default",
        skip_serializing_if = "serde_json::Map::is_empty"
    )]
    pub payload: JsonObject,
}

/// Source: `internal/sessionstore/store.go`, `UsageBucket`.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UsageBucket {
    #[serde(
        rename = "provider",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub provider: String,
    #[serde(
        rename = "model",
        deserialize_with = "null_default",
        skip_serializing_if = "String::is_empty"
    )]
    pub model: String,
    #[serde(rename = "sessions", deserialize_with = "null_default")]
    pub sessions: i64,
    #[serde(rename = "messages", deserialize_with = "null_default")]
    pub messages: i64,
    #[serde(rename = "user_messages", deserialize_with = "null_default")]
    pub user_msgs: i64,
    #[serde(rename = "ai_messages", deserialize_with = "null_default")]
    pub ai_msgs: i64,
}

/// Source: `internal/sessionstore/store.go`, `UsageSummary`.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UsageSummary {
    #[serde(rename = "total_sessions", deserialize_with = "null_default")]
    pub total_sessions: i64,
    #[serde(rename = "total_messages", deserialize_with = "null_default")]
    pub total_messages: i64,
    #[serde(rename = "providers")]
    pub providers: Option<Vec<UsageBucket>>,
}

/// Source: `internal/sessionstore/store.go`, `ResetResult`.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ResetResult {
    #[serde(rename = "sessions", deserialize_with = "null_default")]
    pub sessions: i64,
    #[serde(rename = "events", deserialize_with = "null_default")]
    pub events: i64,
    #[serde(rename = "messages", deserialize_with = "null_default")]
    pub messages: i64,
    #[serde(rename = "approvals", deserialize_with = "null_default")]
    pub approvals: i64,
    #[serde(rename = "attachments", deserialize_with = "null_default")]
    pub attachments: i64,
    #[serde(rename = "preserved_sessions", deserialize_with = "null_default")]
    pub preserved: i64,
}

/// Source: sessionstore.ApprovalDetected. Not a wire request.
#[derive(Clone)]
pub struct ApprovalDetected {
    pub live_session_id: LiveSessionId,
    pub sig: String,
    pub source: String,
    pub kind: String,
    pub provider: String,
    pub question: String,
    pub context: String,
    pub block: String,
    pub candidate_key: String,
    pub source_epoch: ApprovalSourceEpoch,
    pub options: Vec<super::ApprovalOption>,
    /// None preserves Go zero-time's use-the-current-time behavior.
    pub detected_at: Option<Timestamp>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StorageErrorKind {
    Open,
    Query,
    Write,
    Reset,
    Closed,
    Cancelled,
    Timeout,
    InvalidData,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageError {
    pub kind: StorageErrorKind,
    /// Sanitized diagnostic; never raw event bodies or database secrets.
    pub detail: String,
}
impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.detail)
    }
}
impl std::error::Error for StorageError {}
pub type StorageResult<T> = Result<T, StorageError>;
pub type WriteErrorHandler = Arc<dyn Fn(LiveSessionId, StorageError) + Send + Sync>;
/// A nil Go slice remains distinct from an allocated empty result. HTTP adapters
/// normalize only where their baseline handler explicitly did so.
pub type StoredRows<T> = Option<Vec<T>>;
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HistoryEvent(pub JsonObject);
#[derive(Clone, Debug, PartialEq)]
pub struct QueuedHistoryEvent {
    pub live_session_id: LiveSessionId,
    pub generation: HistoryGeneration,
    pub event: HistoryEvent,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnqueueOutcome {
    Queued { generation: HistoryGeneration },
    Dropped { cumulative_count: i64 },
    Unavailable,
    Closed,
}
impl EnqueueOutcome {
    /// StoreEventAsync's legacy return is nonzero only for a full-queue drop.
    pub fn legacy_drop_count(self) -> i64 {
        match self {
            Self::Dropped { cumulative_count } => cumulative_count,
            _ => 0,
        }
    }
}
#[derive(Clone, Debug)]
pub struct StorageOptions {
    /// Configured logs directory is explicit; implementation derives the sibling
    /// any-ai-cli.db and companions only through the supplied RuntimePaths.
    pub log_dir: std::path::PathBuf,
    pub queue_capacity: usize,
    pub query_timeout: Duration,
    pub init_timeout: Duration,
}
impl StorageOptions {
    pub fn baseline(log_dir: std::path::PathBuf) -> Self {
        Self {
            log_dir,
            queue_capacity: 4096,
            query_timeout: Duration::from_secs(3),
            init_timeout: Duration::from_secs(30),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShutdownPolicy {
    /// Baseline Close signals quit and waits at most six seconds; queued writes
    /// are not guaranteed to drain. This is not silently upgraded to Drain.
    CompatibilityStop,
    Drain {
        timeout: Duration,
    },
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShutdownReport {
    pub written: u64,
    pub discarded_old_generation: u64,
    pub dropped: u64,
    pub remaining: u64,
    pub timed_out: bool,
    pub cancelled: bool,
    pub error: Option<StorageError>,
}

/// Exact public Store coverage: internal/sessionstore/store.go (34 methods).
/// Reads preserve baseline limit normalization/order. Synchronous void Go writes
/// stay best-effort; their failures go through the configured error observer.
/// Async enqueue owns an immutable event, captures generation, and never waits
/// for disk. Reset and writer generation checks share one reset barrier.
pub trait SessionStorage: Send + Sync {
    fn open(paths: &RuntimePaths, options: StorageOptions) -> StorageResult<Self>
    where
        Self: Sized;
    fn close(&self) -> StorageResult<()>;
    fn schedule_file_reset(&self) -> StorageResult<()>;
    fn file_reset_pending(&self) -> bool;
    fn set_on_write_error(&self, handler: Option<WriteErrorHandler>);
    fn store_event_async(&self, session: LiveSessionId, event: HistoryEvent) -> EnqueueOutcome;
    fn start_session(&self, start: SessionStart) -> StorageResult<DbSessionId>;
    fn close_stale_sessions(&self, ended_at: Timestamp, reason: &str) -> StorageResult<i64>;
    /// Additive recovery capability; updates only the selected persisted row's
    /// orchestration fields and never revives a completed row or reopens logs.
    fn update_session_orchestration(
        &self,
        _id: DbSessionId,
        _meta: &SessionOrchestrationMeta,
    ) -> StorageResult<()> {
        Err(StorageError {
            kind: StorageErrorKind::Write,
            detail: "orchestration metadata update unavailable".into(),
        })
    }
    fn update_session_messages(&self, session: LiveSessionId, first: &str, last: &str);
    fn update_session_state(&self, session: LiveSessionId, state: &str, last_output_at: &str);
    fn session_card_meta_by_live_session(
        &self,
        session: LiveSessionId,
    ) -> StorageResult<SessionCardMeta>;
    fn update_session_card_meta(
        &self,
        session: LiveSessionId,
        meta: SessionCardMeta,
    ) -> StorageResult<()>;
    fn end_session(&self, session: LiveSessionId, state: &str, reason: &str, ended_at: Timestamp);
    fn clear_session_history(&self, session: LiveSessionId) -> StorageResult<()>;
    fn store_event(&self, session: LiveSessionId, event: HistoryEvent) -> StorageResult<()>;
    fn store_approval_detected(&self, detected: ApprovalDetected);
    fn store_approval_consumed(
        &self,
        session: LiveSessionId,
        sig: &str,
        selected_text: &str,
        resolved_at: Timestamp,
    );
    fn approvals_by_live_session(
        &self,
        session: LiveSessionId,
        limit: i64,
        pending_only: bool,
    ) -> StorageResult<StoredRows<ApprovalRow>>;
    fn approvals_by_session_id(
        &self,
        session: DbSessionId,
        limit: i64,
        pending_only: bool,
    ) -> StorageResult<StoredRows<ApprovalRow>>;
    fn latest_approval_of_kinds(
        &self,
        session: LiveSessionId,
        source: &str,
        kinds: &[String],
    ) -> StorageResult<Option<ApprovalRow>>;
    fn recent_approvals(
        &self,
        limit: i64,
        pending_only: bool,
    ) -> StorageResult<StoredRows<ApprovalRow>>;
    fn chat_messages_by_live_session(
        &self,
        session: LiveSessionId,
        limit: i64,
    ) -> StorageResult<StoredRows<ChatMessage>>;
    fn chat_messages_by_session_id(
        &self,
        session: DbSessionId,
        limit: i64,
    ) -> StorageResult<StoredRows<ChatMessage>>;
    fn search_messages(&self, query: &str, limit: i64) -> StorageResult<StoredRows<SearchResult>>;
    /// Searches USER messages only; assistant text never grants file access.
    fn messages_mention_text(
        &self,
        session: LiveSessionId,
        variants: &[String],
    ) -> StorageResult<bool>;
    fn list_sessions(
        &self,
        limit: i64,
        include_archived: bool,
    ) -> StorageResult<StoredRows<SessionOverview>>;
    fn session_overview_by_live_session(
        &self,
        session: LiveSessionId,
    ) -> StorageResult<SessionOverview>;
    fn session_overview_by_session_id(
        &self,
        session: DbSessionId,
    ) -> StorageResult<SessionOverview>;
    fn update_session_meta(
        &self,
        session: LiveSessionId,
        title: &str,
        tags: &[String],
        summary: &str,
        archived: bool,
    ) -> StorageResult<SessionOverview>;
    fn timeline_by_live_session(
        &self,
        session: LiveSessionId,
        limit: i64,
    ) -> StorageResult<StoredRows<TimelineEvent>>;
    fn usage_summary(&self) -> StorageResult<UsageSummary>;
    fn stale_sessions(
        &self,
        cutoff: Timestamp,
        limit: i64,
    ) -> StorageResult<StoredRows<SessionOverview>>;
    fn prune_older_than(&self, cutoff: Timestamp) -> StorageResult<()>;
    fn prune_transcript_noise(&self) -> StorageResult<i64>;
    /// Counts describe pre-reset totals, not affected SQL row counts.
    fn reset_history(&self, preserve: &[LiveSessionId]) -> StorageResult<ResetResult>;
    fn history_generation(&self) -> HistoryGeneration;
    fn shutdown<'a>(
        &'a self,
        policy: ShutdownPolicy,
        cancellation: &'a HubShutdownCancellation,
    ) -> CoreFuture<'a, ShutdownReport>;
}

/// Separate cancellation ownership: dropping/cancelling an HTTP waiter must not
/// cancel a persistent confirmation, an active task, or the Hub.
macro_rules! cancellation_domain {
    ($name:ident) => {
        #[derive(Clone, Default)]
        pub struct $name(Cancellation);
        impl $name {
            pub fn cancel(&self) {
                self.0.cancel();
            }
            pub fn token(&self) -> &Cancellation {
                &self.0
            }
        }
    };
}
cancellation_domain!(HttpWaitCancellation);
cancellation_domain!(TaskCancellation);
cancellation_domain!(HubShutdownCancellation);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionError {
    NotFound(LiveSessionId),
    StaleBinding,
    AuthenticationExpired,
    ConfirmationMissing,
    ConfirmationDecided,
    ChildLaunch {
        status: u16,
        code: String,
        detail: String,
    },
    ProviderUpdating(String),
    InvalidRequest(String),
    Transport(String),
    Storage(StorageError),
    Cancelled,
    TimedOut,
    Shutdown,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TerminalSize {
    pub cols: i64,
    pub rows: i64,
}
#[derive(Clone)]
pub struct RegisterRequest {
    /// Exact register DTO, already authenticated by the transport. Token/home
    /// fields are internal and never copied wholesale into SessionSnapshot.
    pub message: super::Message,
    /// Untrusted, optional one-use claim from the wrapper handshake. Only core
    /// resolves this to a server-owned admission; invalid claims never fall back.
    pub spawn_proof: Option<String>,
}
#[derive(Clone)]
pub struct ReattachRequest {
    pub message: super::Message,
    /// Server-owned cold-recovery metadata, never decoded from a wrapper frame.
    pub restored_metadata: Option<SpawnRegistrationMetadata>,
}
pub struct Registration {
    /// Present only after core consumes its one-use launch proof. The transport
    /// must hand it to the process startup owner before writing registered ACK.
    /// Manual wrapper registration never acquires startup-process authority.
    pub startup_receipt: Option<crate::process::wrapper_startup::SpawnRegistrationReceipt>,
    /// Source initial-prompt/board caller work, to be owned after ACK. This is
    /// the same server metadata already applied to persistence and the snapshot.
    pub startup_metadata: SpawnRegistrationMetadata,
    pub binding: SessionBinding,
    pub snapshot: SessionSnapshot,
    /// Must be delivered before any later wrapper message.
    pub registered: super::Message,
    pub after_registered: CoreEffects,
}
pub struct Reattachment {
    pub binding: SessionBinding,
    pub snapshot: SessionSnapshot,
    pub reattached: super::Message,
    pub after_reattached: CoreEffects,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputChunk {
    pub bytes: Vec<u8>,
    pub total_pty_bytes: i64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionEnd {
    /// Wrapper-declared state remains independent of exit code/signal.
    pub declared_state: String,
    /// Go's wire `int` is signed 64-bit on supported platforms. It is not an OS
    /// process exit status and must not be narrowed to the latter's i32 domain.
    pub exit_code: i64,
    pub reason: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StopReason {
    User,
    KillAll,
    IdleTimeout,
    Dismissed,
    ParentEnded,
    StartupFailed,
    Timeout,
    HubShutdown,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResizeOutcome {
    Applied,
    Duplicate,
    NotController,
    InvalidSize,
    MissingSession,
    StaleUi,
}
#[derive(Clone)]
pub struct SessionDetails {
    pub binding: SessionBinding,
    /// Exact core clock for source-compatible inactivity decisions; never parse
    /// the seconds-formatted display snapshot back into runtime authority.
    pub last_output_at: Option<Timestamp>,
    pub snapshot: SessionSnapshot,
    pub db_id: Option<DbSessionId>,
    pub git_root: Option<std::path::PathBuf>,
    pub transcript: TranscriptSessionIdentity,
    pub approval: ApprovalSessionSnapshot,
    pub workflow: Option<super::WorkflowProgress>,
    pub subagents: Option<super::SubagentTree>,
    pub done: Option<super::DoneSummary>,
    pub connected: bool,
}
/// Internal-only path identity consumed by transcript/file services.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TranscriptSessionIdentity {
    pub provider: String,
    pub cwd: String,
    pub started_at: String,
    pub home_dir: String,
    pub codex_home: String,
    pub claude_dir: String,
    pub grok_home: String,
    pub agent_session_id: String,
    pub native_log_path: String,
}

/// Core owns one session map and serializes lifecycle/input/admission decisions.
/// Returning effects does not send or persist them; apply in order after locks
/// are released. Delayed work must validate its full binding at execution time.
pub trait SessionCore:
    InputQueue
    + ApprovalActions
    + SpawnAdmission
    + ProviderUpdateAdmission
    + WrappedSessionSpawner
    + Send
    + Sync
{
    fn register<'a>(
        &'a self,
        request: RegisterRequest,
        connection: WrapperConnectionId,
        now: Timestamp,
    ) -> CoreFuture<'a, Result<Registration, SessionError>>;
    fn reattach<'a>(
        &'a self,
        request: ReattachRequest,
        connection: WrapperConnectionId,
        now: Timestamp,
    ) -> CoreFuture<'a, Result<Reattachment, SessionError>>;
    fn snapshot(&self, session: LiveSessionId) -> Option<SessionSnapshot>;
    fn snapshots(&self) -> Vec<SessionSnapshot>;
    fn registered_session_ids(&self) -> Vec<LiveSessionId> {
        self.snapshots().into_iter().map(|s| s.id).collect()
    }
    fn details(&self, session: LiveSessionId) -> Option<SessionDetails>;
    fn active_ids(&self) -> Vec<LiveSessionId>;
    /// Release only the matching terminal startup failure. An HTTP waiter
    /// timeout/drop alone is not evidence that an accepted child cannot register.
    fn spawn_failed(
        &self,
        attempt: SpawnAttemptId,
        provider: &str,
    ) -> Result<CoreEffects, SessionError>;
    fn is_current(&self, binding: SessionBinding) -> bool;
    /// C3 workflow/transcript/done observers update the same state owner rather
    /// than retaining a parallel mutable session map. Full binding is validated.
    fn apply_observation(
        &self,
        binding: SessionBinding,
        observation: SessionObservation,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError>;
    fn observe_output(
        &self,
        binding: SessionBinding,
        chunk: OutputChunk,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError>;
    fn observe_end(
        &self,
        binding: SessionBinding,
        end: SessionEnd,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError>;
    fn disconnected(
        &self,
        binding: SessionBinding,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError>;
    fn resize(
        &self,
        ui: UiBinding,
        session: LiveSessionId,
        size: TerminalSize,
        now: Timestamp,
    ) -> (ResizeOutcome, CoreEffects);
    fn reset_history(
        &self,
        session: LiveSessionId,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError>;
    fn dismiss(&self, session: LiveSessionId, now: Timestamp) -> Result<CoreEffects, SessionError>;
    fn reset_history_from_ui(
        &self,
        ui: UiBinding,
        session: LiveSessionId,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError>;
    fn dismiss_from_ui(
        &self,
        ui: UiBinding,
        session: LiveSessionId,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError>;
    fn stop(
        &self,
        session: LiveSessionId,
        reason: StopReason,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError>;
    fn registered_session_count(&self) -> usize;
    fn patch_card_meta(
        &self,
        session: LiveSessionId,
        patch: SessionCardMetaPatch,
    ) -> Result<SessionCardMetaUpdate, SessionError>;
    fn update_card_meta(
        &self,
        session: LiveSessionId,
        meta: SessionCardMeta,
    ) -> Result<CoreEffects, SessionError>;
    fn auth_epoch(&self) -> AuthEpoch;
    fn attach_ui(
        &self,
        ui: UiBinding,
        active: Option<LiveSessionId>,
        initial_size: Option<TerminalSize>,
    ) -> Result<UiPriming, SessionError>;
    /// Nonempty drains keep priming active. Apply the batch, repeat, and only
    /// an empty drain atomically transitions the UI to live delivery.
    fn finish_ui_priming(&self, ui: UiBinding) -> Result<CoreEffects, SessionError>;
    fn broadcast_ui(&self, message: super::Message) -> CoreEffects;
    fn broadcast_git_turn(&self, event: GitTurnNotification) -> CoreEffects;
    /// Accept work atomically with the same membership check used by revoke.
    /// Accepted operations retain this guard until their I/O is complete.
    fn authorize_ui_work(&self, ui: UiBinding) -> Result<Box<dyn AcceptedUiWork>, SessionError>;
    /// Revoke removes membership first, then drains accepted work outside locks.
    /// New/queued work cannot enter while this waits; accepted work is not killed.
    fn drain_ui_work(&self, ui: UiBinding) -> CoreFuture<'_, ()>;
    /// Source: ui_active_session and the pre-enqueue input ownership claim.
    /// A missing/zero size still changes ownership and never creates a prompt epoch.
    fn claim_ui_session(
        &self,
        ui: UiBinding,
        session: LiveSessionId,
        size: Option<TerminalSize>,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError>;
    /// Advisory after input was already sent; validates record/epoch without resending.
    fn consume_approval(
        &self,
        ui: UiBinding,
        message: super::Message,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError>;
    /// Replies from the current immutable record; never re-detect from a stale UI tail.
    fn resync_approval(
        &self,
        ui: UiBinding,
        session: LiveSessionId,
        now: Timestamp,
    ) -> Result<CoreEffects, SessionError>;

    fn detach_ui(&self, ui: UiBinding) -> CoreEffects;
    /// Invalidates queued UI work/connections while retaining wrapper bindings.
    fn invalidate_all_ui(&self) -> CoreEffects;
    fn subscribe(&self) -> Box<dyn CoreEventSubscription>;
}
/// Opaque RAII authorization lease owned by the single SessionCore. Dropping
/// the lease releases accepted-work accounting even on future cancellation.
pub trait AcceptedUiWork: Send {}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub enum GitTurnType {
    #[default]
    #[serde(rename = "git_turn")]
    GitTurn,
}
/// The one non-Message Hub broadcast in the fixed Go source (git_turns.go).
/// It must use the same core-owned UI priming queue as ordinary protocol frames.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct GitTurnNotification {
    pub r#type: GitTurnType,
    pub session_id: i64,
    pub turn: i64,
    pub started_at: String,
    pub ended_at: String,
    pub files_changed: i64,
    pub added: i64,
    pub removed: i64,
}
/// Additive Rust-only notification; generated Message remains frozen.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub enum SpawnCorrelationType {
    #[default]
    #[serde(rename = "session_spawn_correlated")]
    Correlated,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct SpawnCorrelationNotification {
    pub r#type: SpawnCorrelationType,
    pub hub_instance: String,
    pub session_id: i64,
    pub started_at: String,
    pub client_request_id: String,
}
/// Correlation is not a credential or an idempotency key. Empty preserves old clients.
pub fn valid_client_request_id(value: &str) -> bool {
    value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}
pub struct UiPriming {
    pub hub_instance: String,
    pub sessions: Vec<SessionSnapshot>,
    pub ordered_frames: Vec<super::Message>,
}

pub const INPUT_QUEUE_LIMIT: usize = 100;
pub const INITIAL_PROMPT_GATE_TIMEOUT: Duration = Duration::from_secs(90);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeferredReason {
    InitialPrompt,
    Wrapper,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputFrame {
    pub seq: InputSeq,
    pub bytes: Vec<u8>,
}
#[derive(Clone)]
pub enum InputAuthority {
    Ui(UiBinding),
    Internal,
    /// Only the initial prompt may bypass older gated user input.
    InitialPrompt,
    NativeApproval(ApprovalActionBinding),
}
#[derive(Clone)]
pub struct InputRequest {
    pub bytes: Vec<u8>,
    pub authority: InputAuthority,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputDisposition {
    MissingSession,
    StaleBinding,
    AuthenticationExpired,
    Deferred {
        reason: DeferredReason,
        dropped_oldest: bool,
    },
    /// A websocket write is neither PTY completion nor provider acceptance.
    TransportWritten {
        sequences: Vec<InputSeq>,
    },
    Failed {
        unsent_remainder: Vec<u8>,
        detail: String,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputReceipt {
    pub binding: SessionBinding,
    pub disposition: InputDisposition,
    pub submit: SubmitReceipt,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmitReceipt {
    NotRequested,
    PendingEnter,
    EnterWritten,
    OutputObserved,
    UnconfirmedAfterRetry,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AckDisposition {
    Removed,
    /// Any ACK, including duplicate/nonpositive, advertises ACK capability.
    CapabilityObserved,
    /// Capability is still observed, but another connection's inflight entry stays.
    WrongConnection,
    MissingSession,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputReservation {
    pub binding: SessionBinding,
    pub sequence: InputSeq,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputHighWatermarks {
    pub processed: InputSeq,
    pub received: InputSeq,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SubmitTiming {
    pub idle_settle: Duration,
    pub minimum: Duration,
    pub slow_minimum: Duration,
    pub maximum: Duration,
    pub poll: Duration,
    pub confirm_window: Duration,
}
impl Default for SubmitTiming {
    fn default() -> Self {
        Self {
            idle_settle: Duration::from_millis(120),
            minimum: Duration::from_millis(120),
            slow_minimum: Duration::from_millis(700),
            maximum: Duration::from_secs(30),
            poll: Duration::from_millis(20),
            confirm_window: Duration::from_millis(1500),
        }
    }
}
/// FIFO lane survives reattach. Reserve before transport write and undo only
/// that reservation on failure. ACK removes only a matching connection's frame.
/// Replay unacknowledged original sequences in sorted order before pending input;
/// received (unfinished) watermark never suppresses a retried PTY write.
pub trait InputQueue: Send + Sync {
    fn submit<'a>(
        &'a self,
        binding: SessionBinding,
        request: InputRequest,
        now: Timestamp,
        cancellation: &'a TaskCancellation,
    ) -> CoreFuture<'a, InputReceipt>;
    fn reserve_frame(
        &self,
        binding: SessionBinding,
        bytes: Vec<u8>,
    ) -> Result<(InputReservation, InputFrame), SessionError>;
    fn release_frame(&self, reservation: InputReservation);
    fn acknowledge(&self, binding: SessionBinding, seq: InputSeq) -> AckDisposition;
    fn transport_failed(&self, binding: SessionBinding, now: Timestamp) -> CoreEffects;
    fn flush<'a>(
        &'a self,
        binding: SessionBinding,
        cancellation: &'a TaskCancellation,
    ) -> CoreFuture<'a, CoreEffects>;
    fn clear_initial_gate(&self, session: LiveSessionId) -> CoreEffects;
}
/// Wrapper-side sequence bookkeeping is distinct from Hub ACK tracking.
pub trait ProcessedInput: Send + Sync {
    fn watermarks(&self) -> InputHighWatermarks;
    fn received(&self, sequence: InputSeq);
    /// Call only after the entire frame was written successfully to the PTY.
    fn mark_processed(&self, sequence: InputSeq);
    fn already_processed(&self, sequence: InputSeq) -> bool;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandidateIdentity {
    pub key: String,
    pub shape: String,
    pub source_epoch: ApprovalSourceEpoch,
}
impl CandidateIdentity {
    /// Shape assists candidate recovery; sig/version/replay epoch are not identity.
    pub fn same_candidate(&self, other: &Self) -> bool {
        !self.key.is_empty() && self.key == other.key && self.source_epoch == other.source_epoch
    }
}
#[derive(Clone, PartialEq)]
pub struct ApprovalRecordData {
    pub candidate: CandidateIdentity,
    pub sig: String,
    pub origin: String,
    pub source: String,
    pub kind: String,
    pub block: String,
    pub question: String,
    pub context: String,
    pub options: Vec<super::ApprovalOption>,
    pub summary: super::ApprovalSummary,
    pub detected_at: Timestamp,
}
/// After publishing, replace the record rather than editing shared fields.
#[derive(Clone, PartialEq)]
pub struct ImmutableApprovalRecord(Arc<ApprovalRecordData>);
impl ImmutableApprovalRecord {
    pub fn new(data: ApprovalRecordData) -> Self {
        Self(Arc::new(data))
    }
    pub fn data(&self) -> &ApprovalRecordData {
        &self.0
    }
}
#[derive(Clone, PartialEq)]
pub struct ApprovalSessionSnapshot {
    pub session: LiveSessionId,
    pub version: ApprovalStateVersion,
    pub record: Option<ImmutableApprovalRecord>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalCloseReason {
    Answered,
    AnsweredTerminal,
    Superseded,
    Vanished,
    SessionEnd,
    HistoryReset,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApprovalActionBinding {
    pub session: SessionBinding,
    pub candidate_key: String,
    pub source_epoch: ApprovalSourceEpoch,
    pub sig: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeActionOrigin {
    User,
    OneTap { nonce: String },
    Batch,
    Automatic { rule_id: String },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeActionSelection {
    /// Exact interactive user choice, validated against freshly detected options.
    Exact {
        selected_text: String,
        send_text: String,
    },
    /// Resolve the one-shot option after acquiring the session input FIFO.
    ApproveOnce,
    RejectOnce,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeActionRequest {
    pub binding: ApprovalActionBinding,
    pub selection: NativeActionSelection,
    pub origin: NativeActionOrigin,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApprovalActionError {
    NotFound,
    StaleCandidate,
    StaleWrapper,
    Reserved,
    HighRisk,
    PersistentOption,
    PolicyChanged,
    Transport(String),
    Cancelled,
}
/// A single-lock, freshly re-detected native approval snapshot for batch
/// matching. Disconnected entries remain visible, as in Go matched counts;
/// prepare_and_send still validates the exact live transport before input.
#[derive(Clone)]
pub struct PendingNativeApprovalAction {
    pub binding: ApprovalActionBinding,
    pub provider: String,
    pub cwd: String,
    pub summary: super::ApprovalSummary,
    pub connected: bool,
}
pub struct ReservedApprovalAction {
    pub reservation: ApprovalReservationId,
    pub binding: ApprovalActionBinding,
    pub selected_text: String,
    pub origin: NativeActionOrigin,
}
/// A lease spans validate/send/nonce-consume/commit, and is checked after FIFO
/// acquisition and before each delayed frame. Failure releases it. Commit cannot
/// clear a replacement candidate or silently retarget a reattached wrapper.
pub trait ApprovalActions: Send + Sync {
    fn pending_native_approval_actions(&self) -> Vec<PendingNativeApprovalAction>;
    fn prepare_and_send<'a>(
        &'a self,
        request: NativeActionRequest,
        cancel: &'a TaskCancellation,
    ) -> CoreFuture<'a, Result<ReservedApprovalAction, ApprovalActionError>>;
    fn commit(
        &self,
        action: ReservedApprovalAction,
        now: Timestamp,
    ) -> Result<CoreEffects, ApprovalActionError>;
    fn release(&self, action: ReservedApprovalAction);
}

pub enum PersistenceEffect {
    OrchestrationMeta {
        session: LiveSessionId,
        database: DbSessionId,
        meta: SessionOrchestrationMeta,
    },
    Event {
        session: LiveSessionId,
        event: HistoryEvent,
    },
    ApprovalDetected(ApprovalDetected),
    ApprovalConsumed {
        session: LiveSessionId,
        sig: String,
        selected_text: String,
        resolved_at: Timestamp,
    },
    CardMeta {
        session: LiveSessionId,
        meta: SessionCardMeta,
    },
    /// Input auto-title persistence follows already accepted/sent input and
    /// reports degradation without converting it to a failed user operation.
    CardMetaBestEffort {
        session: LiveSessionId,
        meta: SessionCardMeta,
    },
    SessionState {
        session: LiveSessionId,
        state: String,
        last_output_at: String,
    },
    SessionMessages {
        session: LiveSessionId,
        first: String,
        last: String,
    },
    EndSession {
        binding: SessionBinding,
        state: String,
        reason: String,
        ended_at: Timestamp,
    },
    ClearSessionHistory(LiveSessionId),
}
/// Typed updates supplied by transcript/workflow/services observers. No arbitrary
/// metadata bag may carry hidden session fields through this boundary.
#[derive(Clone)]
pub enum SessionObservation {
    Messages {
        first: String,
        last: String,
    },
    Branch {
        branch: String,
        git_root: Option<std::path::PathBuf>,
        changes: (i64, i64, i64),
        /// None is an unresolved lookup; Some("") is a confirmed non-repository.
        project_id: Option<String>,
    },
    Model {
        model: String,
        effort: String,
    },
    Transcript {
        path: std::path::PathBuf,
        agent_session_id: String,
        safe_offset: i64,
        grew_at: String,
    },
    Workflow(super::WorkflowProgress),
    Subagents(super::SubagentTree),
    Done(super::DoneSummary),
    CrossSessionMessage(super::CrossSessionMessage),
    Relays(Vec<super::RelayStatus>),
    BoardNotifyPending(bool),
}

/// Cross-owner notifications, not a new external WebSocket protocol.
#[derive(Clone)]
pub enum CoreEvent {
    RoutineCompleted {
        binding: SessionBinding,
        summary: super::DoneSummary,
    },
    HandoffNoteWritten {
        binding: SessionBinding,
        path: std::path::PathBuf,
    },
    ApprovalOpened {
        session: LiveSessionId,
        record: ImmutableApprovalRecord,
    },
    ApprovalPublished {
        session: LiveSessionId,
        record: ImmutableApprovalRecord,
    },
    Registered(SessionBinding),
    /// Source reattach restarts retained artifact polls even before new input.
    /// Initial registration emits only Registered; this accompanies it only
    /// in the authoritative after-reattach-ACK effect sequence.
    Reattached(SessionBinding),
    SessionState {
        binding: SessionBinding,
        activity: super::SessionActivity,
        display_state: String,
    },
    UserMessage {
        binding: SessionBinding,
        first: String,
        last: String,
    },
    CompletionRecord {
        binding: SessionBinding,
        summary: super::DoneSummary,
    },
    Completed {
        binding: SessionBinding,
        summary: super::DoneSummary,
        fallback: bool,
    },
    TranscriptChanged {
        binding: SessionBinding,
        path: std::path::PathBuf,
        safe_offset: i64,
        grew_at: String,
    },
    TurnSummary {
        binding: SessionBinding,
        turn: i64,
        text: String,
    },
    OutputObserved {
        binding: SessionBinding,
        clean: String,
        at: Timestamp,
    },
    WorkflowChanged {
        binding: SessionBinding,
        progress: super::WorkflowProgress,
    },
    GitTurnCapture {
        binding: SessionBinding,
        started_at: String,
        ended_at: Option<String>,
    },
    HandoffUpdated {
        session: LiveSessionId,
        record_path: std::path::PathBuf,
    },
    SubagentsChanged {
        binding: SessionBinding,
        tree: super::SubagentTree,
    },
    CrossSessionMessage {
        binding: SessionBinding,
        message: super::CrossSessionMessage,
    },
    Ended {
        binding: SessionBinding,
        end: SessionEnd,
    },
    Dismissed(LiveSessionId),
    HistoryReset(LiveSessionId),
}
#[derive(Clone)]
pub struct VersionedCoreEvent {
    pub sequence: EventSequence,
    pub event: CoreEvent,
}
pub enum CoreEventPoll {
    Event(Box<VersionedCoreEvent>),
    Lagged { missed: u64 },
    Closed,
    Cancelled,
}
pub trait CoreEventSubscription: Send {
    fn next<'a>(&'a mut self, cancel: &'a HubShutdownCancellation)
    -> CoreFuture<'a, CoreEventPoll>;
}
pub enum CoreEffect {
    SendWrapper {
        binding: SessionBinding,
        message: super::Message,
    },
    /// Native resize delivery is best-effort in Go; its failure cannot veto
    /// the UI resize broadcast/history or later input. Input sends stay strict.
    SendWrapperBestEffort {
        binding: SessionBinding,
        message: super::Message,
    },
    SendUi {
        binding: UiBinding,
        message: super::Message,
    },
    /// Source broadcasts are best-effort. A failed UI writer is closed/reported
    /// without vetoing already accepted wrapper or persistence work.
    SendUiBestEffort {
        binding: UiBinding,
        message: super::Message,
    },
    Broadcast(super::Message),
    BroadcastGitTurn(GitTurnNotification),
    BroadcastSpawnCorrelation(SpawnCorrelationNotification),
    SendUiSpawnCorrelation {
        binding: UiBinding,
        event: SpawnCorrelationNotification,
        best_effort: bool,
    },
    SendUiGitTurn {
        binding: UiBinding,
        event: GitTurnNotification,
        best_effort: bool,
    },
    Persist(PersistenceEffect),
    PersistBound {
        binding: SessionBinding,
        scope: PersistenceBindingScope,
        effect: PersistenceEffect,
        order: Box<dyn PersistenceOrder>,
    },
    Notify(CoreEvent),
    CancelSession {
        binding: SessionBinding,
        reason: StopReason,
    },
    /// Closes exactly the stale socket, even after a replacement binding exists.
    /// This never terminates the wrapper's PTY/process owner.
    CloseWrapper {
        binding: SessionBinding,
    },
    DrainUi(UiBinding),
    CloseUi(UiBinding),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PersistenceBindingScope {
    Incarnation,
    ExactWrapper,
}
/// Core reserves this cancellation-safe ticket while producing persistence
/// effects. The driver waits immediately before DB/journal application and
/// drops it immediately afterward, never holding it across socket I/O.
/// Dropping an unapplied ticket skips that position, not any later work.
pub trait PersistenceOrder: Send + Sync {
    fn wait(&self) -> CoreFuture<'_, ()>;
}
/// Ordering is significant: close old approval ledger entry before inserting a
/// new record reusing its sig; registered must precede outbound wrapper effects.
#[derive(Default)]
pub struct CoreEffects(pub Vec<CoreEffect>);

/// C3 owns socket writers. A full binding is checked immediately before sending;
/// transport writes are not PTY acknowledgement or provider acceptance.
pub trait WrapperTransport: Send + Sync {
    fn send<'a>(
        &'a self,
        binding: SessionBinding,
        message: super::Message,
    ) -> CoreFuture<'a, Result<(), SessionError>>;
}
/// Earlier effects were applied in order. The failed effect may have performed
/// partial external I/O; never retry the whole batch on this result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoreEffectFailure {
    pub index: usize,
    pub error: SessionError,
}
/// C3 applies ordered socket/persistence/notification effects after C2 releases
/// state locks. Async input/action operations inject this same sink rather than
/// silently dropping their persistence or notification side effects.
pub trait CoreEffectSink: Send + Sync {
    fn apply<'a>(&'a self, effects: CoreEffects) -> CoreFuture<'a, Result<(), CoreEffectFailure>>;
}
/// Internal publisher for the very same bus observed by SessionCore::subscribe.
/// C2 creates this handle with its bus before constructing the C3 effect driver.
/// Only CoreEffect::Notify application publishes, after all earlier effects.
pub trait CoreEventPublisher: Send + Sync {
    fn publish(&self, event: CoreEvent) -> Result<EventSequence, SessionError>;
}
/// C2's ordered journal + SQLite adapter. Its state contains owned writer handles,
/// not a competing session map. C3 delegates Persist here and propagates errors.
pub trait PersistenceEffectSink: Send + Sync {
    fn apply(&self, effect: PersistenceEffect) -> Result<(), SessionError>;
    fn apply_bound(
        &self,
        _binding: SessionBinding,
        _scope: PersistenceBindingScope,
        _effect: PersistenceEffect,
    ) -> Result<(), SessionError> {
        Err(SessionError::InvalidRequest(
            "bound persistence adapter is not implemented".into(),
        ))
    }
}

/// Source: orchestration.go spawnChildRequest. Claims confer no authority.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChildSpawnRequest {
    #[serde(rename = "role", deserialize_with = "null_default")]
    pub role: String,
    #[serde(rename = "provider", deserialize_with = "null_default")]
    pub provider: String,
    #[serde(rename = "model", deserialize_with = "null_default")]
    pub model: String,
    #[serde(rename = "initial_prompt", deserialize_with = "null_default")]
    pub initial_prompt: String,
    #[serde(rename = "cwd", deserialize_with = "null_default")]
    pub cwd: String,
    #[serde(rename = "auto", deserialize_with = "null_default")]
    pub auto: bool,
    #[serde(rename = "permission_mode", deserialize_with = "null_default")]
    pub permission_mode: String,
    #[serde(rename = "sandbox", deserialize_with = "null_default")]
    pub sandbox: String,
    #[serde(rename = "ask_for_approval", deserialize_with = "null_default")]
    pub ask_for_approval: String,
    #[serde(rename = "route", deserialize_with = "null_default")]
    pub route: String,
    #[serde(rename = "model_selection_mode", deserialize_with = "null_default")]
    pub model_selection: String,
    #[serde(rename = "risk_confirmed", deserialize_with = "null_default")]
    pub risk_confirmed: bool,
    #[serde(rename = "force", deserialize_with = "null_default")]
    pub force: bool,
    #[serde(rename = "subscription_profile_id", deserialize_with = "null_default")]
    pub subscription_profile_id: String,
    #[serde(rename = "same_tree")]
    pub same_tree: Option<bool>,
    #[serde(rename = "effort", deserialize_with = "null_default")]
    pub effort: String,
    #[serde(rename = "execution_mode", deserialize_with = "null_default")]
    pub execution_mode: String,
    #[serde(rename = "permission_preset", deserialize_with = "null_default")]
    pub permission_preset: String,
    #[serde(rename = "origin", deserialize_with = "null_default")]
    pub claimed_origin: String,
    #[serde(rename = "remember_permission")]
    pub remember_permission: Option<bool>,
}

/// Source: orchestration.go spawnConfirmationResponse. Claims confer no authority.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpawnConfirmationResponse {
    #[serde(rename = "confirmation_id", deserialize_with = "null_default")]
    pub confirmation_id: SpawnConfirmationId,
    #[serde(rename = "approved", deserialize_with = "null_default")]
    pub approved: bool,
    #[serde(rename = "provider", deserialize_with = "null_default")]
    pub provider: String,
    #[serde(rename = "model", deserialize_with = "null_default")]
    pub model: String,
    #[serde(rename = "effort")]
    pub effort: Option<String>,
    #[serde(rename = "execution_mode")]
    pub execution_mode: Option<String>,
    #[serde(rename = "permission_preset")]
    pub permission_preset: Option<String>,
    #[serde(rename = "remember_permission")]
    pub remember_permission: Option<bool>,
    #[serde(rename = "grant_folder_trust")]
    pub grant_folder_trust: Option<bool>,
}

/// Produced only by the server after same-origin/capability and auth checks.
/// It cannot be constructed by deserializing an `origin: "ui"` claim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedUiOrigin {
    auth_epoch: AuthEpoch,
}
impl VerifiedUiOrigin {
    /// Issue only after the existing signed UI cookie, Fetch Metadata, Origin
    /// and ordinary HTTP guard succeed. No active WebSocket is required by Go.
    #[allow(dead_code)]
    pub(crate) fn after_server_verification(auth_epoch: AuthEpoch) -> Self {
        Self { auth_epoch }
    }
    pub fn auth_epoch(&self) -> AuthEpoch {
        self.auth_epoch
    }
}
/// The existing confirmation endpoint's authenticated HTTP request. This is
/// deliberately not a direct-UI-origin capability or proof of human presence.
/// It has no wire/serde constructor and cannot authorize a fresh UI-origin claim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedConfirmationRequest {
    auth_epoch: AuthEpoch,
}
impl VerifiedConfirmationRequest {
    /// Issue only after token/method/Host/Origin and applicable PIN checks.
    pub(crate) fn after_server_authentication(auth_epoch: AuthEpoch) -> Self {
        Self { auth_epoch }
    }
    pub fn auth_epoch(&self) -> AuthEpoch {
        self.auth_epoch
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VerifiedSpawnOrigin {
    Autonomous,
    HumanUi(VerifiedUiOrigin),
    HumanConfirmation {
        request: VerifiedConfirmationRequest,
        confirmation: SpawnConfirmationId,
    },
}
impl VerifiedSpawnOrigin {
    pub fn is_human(&self) -> bool {
        !matches!(self, Self::Autonomous)
    }
    pub fn auth_epoch(&self) -> Option<AuthEpoch> {
        match self {
            Self::Autonomous => None,
            Self::HumanUi(origin) => Some(origin.auth_epoch()),
            Self::HumanConfirmation { request, .. } => Some(request.auth_epoch()),
        }
    }
}
impl ChildSpawnRequest {
    pub fn verify_origin(
        &self,
        proof: Option<VerifiedUiOrigin>,
    ) -> Result<VerifiedSpawnOrigin, SessionError> {
        match self.claimed_origin.as_str() {
            "" => Ok(VerifiedSpawnOrigin::Autonomous),
            "ui" => proof
                .map(VerifiedSpawnOrigin::HumanUi)
                .ok_or(SessionError::AuthenticationExpired),
            _ => Err(SessionError::InvalidRequest("unknown spawn origin".into())),
        }
    }
    /// Baseline: provider/model empty means keep; optional launch fields with
    /// Some("") deliberately clear. Only a verified human may apply this method.
    pub fn apply_human_decision(
        &mut self,
        decision: &SpawnConfirmationResponse,
        _proof: &VerifiedConfirmationRequest,
    ) {
        if !decision.provider.trim().is_empty() {
            self.provider = decision.provider.trim().to_owned();
        }
        if !decision.model.trim().is_empty() {
            self.model = decision.model.trim().to_owned();
        }
        if let Some(value) = &decision.effort {
            self.effort = value.trim().to_owned();
        }
        if let Some(value) = &decision.execution_mode {
            self.execution_mode = value.trim().to_owned();
        }
        if let Some(value) = &decision.permission_preset {
            self.permission_preset = value.trim().to_owned();
        }
        if let Some(value) = decision.remember_permission {
            self.remember_permission = Some(value);
        }
    }
}
/// Internal grants never appear in ChildSpawnRequest or serde input/output.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InternalSpawnGrants {
    allowed_tools: Vec<String>,
    grant_folder_trust: bool,
}
impl InternalSpawnGrants {
    pub fn allowed_tools(&self) -> &[String] {
        &self.allowed_tools
    }
    pub fn grant_folder_trust(&self) -> bool {
        self.grant_folder_trust
    }
    /// Tools originate in built-in/config bounded permission tables only.
    #[allow(dead_code)]
    pub(crate) fn from_config(allowed_tools: Vec<String>) -> Self {
        Self {
            allowed_tools,
            grant_folder_trust: false,
        }
    }
    pub fn with_human_folder_trust(
        mut self,
        decision: &SpawnConfirmationResponse,
        _proof: &VerifiedConfirmationRequest,
    ) -> Self {
        self.grant_folder_trust = decision.approved && decision.grant_folder_trust == Some(true);
        self
    }
}
#[derive(Clone)]
pub struct ResolvedChildSpawn {
    request: ChildSpawnRequest,
    origin: VerifiedSpawnOrigin,
    grants: InternalSpawnGrants,
}
impl ResolvedChildSpawn {
    /// Request claims are resolved against server-issued proof before any human
    /// admission/default/memory behavior is available to orchestration.
    pub fn from_request(
        mut request: ChildSpawnRequest,
        proof: Option<VerifiedUiOrigin>,
        grants: InternalSpawnGrants,
    ) -> Result<Self, SessionError> {
        let origin = request.verify_origin(proof)?;
        if !origin.is_human() {
            request.remember_permission = None;
        }
        Ok(Self {
            request,
            origin,
            grants,
        })
    }
    pub fn request(&self) -> &ChildSpawnRequest {
        &self.request
    }
    pub fn origin(&self) -> &VerifiedSpawnOrigin {
        &self.origin
    }
    pub fn grants(&self) -> &InternalSpawnGrants {
        &self.grants
    }
    /// Only source-derived server configuration supplies this fallback. Preserve
    /// existing bounded tools and the independently verified folder-trust grant.
    pub(crate) fn fill_allowed_tools_from_config(&mut self, tools: Vec<String>) {
        if self.grants.allowed_tools.is_empty() {
            self.grants.allowed_tools = tools;
        }
    }
    /// Role/provider normalization is server-owned, never a second JSON decode.
    #[allow(dead_code)]
    pub(crate) fn request_mut(&mut self) -> &mut ChildSpawnRequest {
        &mut self.request
    }
    pub fn approve(
        mut self,
        decision: &SpawnConfirmationResponse,
        proof: VerifiedConfirmationRequest,
    ) -> Result<Self, SessionError> {
        if !decision.approved {
            return Err(SessionError::InvalidRequest(
                "refused confirmation is not an approval".into(),
            ));
        }
        self.request.apply_human_decision(decision, &proof);
        self.grants = self.grants.with_human_folder_trust(decision, &proof);
        self.origin = VerifiedSpawnOrigin::HumanConfirmation {
            request: proof,
            confirmation: decision.confirmation_id.clone(),
        };
        Ok(self)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdmissionLimits {
    pub max_children_per_parent: i64,
    pub max_total_sessions: i64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmissionRequest {
    pub parent: LiveSessionId,
    pub slots: i64,
    pub origin: VerifiedSpawnOrigin,
    /// A same-role confirmation can replace its reservation in ONE transaction.
    pub replace: Option<AdmissionId>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdmissionError {
    /// Required runtime capability/configuration is unavailable; no reservation
    /// or successful dialog was published. Callers map this gate explicitly.
    Unavailable,
    ParentNotFound,
    InvalidSlots,
    InvalidReplacement,
    ChildrenPerParent {
        running_relays: i64,
        used: i64,
        maximum: i64,
    },
    TotalSessions {
        running_relays: i64,
        used: i64,
        maximum: i64,
    },
    MissingReservation,
    IdentityExhausted,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmissionReservation {
    pub id: AdmissionId,
    pub parent: LiveSessionId,
    pub slots: i64,
}
pub trait SpawnAdmission: Send + Sync {
    /// Query live counts + reserve under the same state-owner transaction.
    fn reserve_children(
        &self,
        request: AdmissionRequest,
        limits: AdmissionLimits,
    ) -> Result<AdmissionReservation, AdmissionError>;
    fn release_children(&self, admission: &AdmissionId) -> bool;
    fn consume_children(&self, admission: &AdmissionId, slots: i64) -> bool;
    fn admission_matches(&self, admission: &AdmissionId, parent: LiveSessionId, slots: i64)
    -> bool;
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderUpdateLease {
    pub provider: String,
    pub id: ProviderUpdateId,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderSpawnLease {
    pub provider: String,
    pub id: SpawnAttemptId,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderAdmissionError {
    AlreadyUpdating,
    RunningSessions {
        count: usize,
    },
    /// Accepted starts awaiting registration must also exclude an update.
    /// C1 closes the baseline check-then-process-start interleaving explicitly.
    PendingSpawns {
        count: usize,
    },
    StaleLease,
    IdentityExhausted,
}
pub trait ProviderUpdateAdmission: Send + Sync {
    fn begin_provider_spawn(
        &self,
        provider: &str,
    ) -> Result<ProviderSpawnLease, ProviderAdmissionError>;
    fn end_provider_spawn(&self, lease: ProviderSpawnLease);
    fn begin_provider_update(
        &self,
        provider: &str,
    ) -> Result<ProviderUpdateLease, ProviderAdmissionError>;
    fn end_provider_update(&self, lease: ProviderUpdateLease);
    fn provider_session_count(&self, provider: &str) -> usize;
}
#[derive(Clone)]
pub struct ProviderCommandPlan {
    pub provider: String,
    pub purpose: ProviderCommandPurpose,
    /// Preview, logs and execution all derive argv from this exact resolved plan.
    /// For update B, never retain provider launch A as `executable`.
    pub process: ProcessPlan,
    pub cancellation: Cancellation,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderCommandPurpose {
    Interactive,
    Headless,
    Login,
    UsageProbe,
    Update,
    Version,
}
/// Server-owned registration metadata from the fixed Go pendingChild contract.
/// It is bound to a one-use spawn attempt, never resolved by an editable label
/// or deserialized from wrapper/browser input. Prompt text is not persisted here.
#[derive(Clone, Default)]
pub struct SpawnRegistrationMetadata {
    pub client_request_id: String,
    pub parent: LiveSessionId,
    pub role: String,
    pub auto: bool,
    pub depth: i64,
    pub orchestration: OrchestrationId,
    pub board_path: String,
    pub worktree_branch: String,
    pub normal_worktree: NormalWorktree,
    pub worktree_cleanup: String,
    pub spawned_at: Option<Timestamp>,
    pub initial_prompt: String,
    pub handoff_from: LiveSessionId,
    pub prompt_at_launch: bool,
}
impl SpawnRegistrationMetadata {
    pub fn needs_initial_gate(&self) -> bool {
        (!self.orchestration.0.is_empty() || !self.initial_prompt.is_empty())
            && !self.prompt_at_launch
    }
}
/// The exact policy resolved before a native launch. Credential-bearing
/// environment values deliberately have no Debug or serialization exposure.
pub struct ResolvedSpawnPolicy {
    /// Full sanitized inherited snapshot. Windows policy must also refresh and
    /// expand PATH from its authorized platform sources as Go expandPathEntries.
    pub base_environment: Vec<String>,
    pub route_environment: Vec<String>,
    pub subscription_environment: Vec<String>,
    pub effective_route: String,
    pub current_model: String,
    /// Final source-resolved model, including suppression for custom definitions
    /// without model_args. Empty must not fall back to the requested model.
    pub resolved_model: String,
    /// Exact registry/fallback wrapper argv, already resolved once. Some Go
    /// registry mappings are raw provider flags; never rewrite them to --effort.
    pub effort_args: Vec<String>,
    pub ordinary_model_args: Vec<String>,
}

#[derive(Clone)]
pub struct WrappedSpawnSpec {
    pub registration_metadata: SpawnRegistrationMetadata,
    /// Internal admission correlation; never serialized onto conductor JSON.
    pub spawn_attempt: Option<SpawnAttemptId>,
    pub registration_proof: Option<SpawnRegistrationProof>,
    pub provider: String,
    pub cwd: std::path::PathBuf,
    pub model: String,
    pub model_selection: String,
    pub risk_confirmed: bool,
    pub label: String,
    pub permission_mode: String,
    pub sandbox: String,
    pub ask_for_approval: String,
    pub route: String,
    pub utf8_session: bool,
    pub effort: String,
    pub execution_mode: String,
    pub permission_preset: String,
    pub initial_prompt: String,
    pub subscription_profile_id: String,
    pub subscription_login: bool,
    pub usage_probe: bool,
    pub grants: InternalSpawnGrants,
    pub cancellation: TaskCancellation,
}
pub const SPAWN_PROOF_HEADER: &str = "x-many-ai-internal-spawn-proof";
pub const SPAWN_PROOF_ENV: &str = "MANY_AI_CLI_INTERNAL_SPAWN_PROOF";
/// Ephemeral internal correlation, not human approval. Never Debug/Serialize or
/// persist this value; wrapper removes the environment entry before provider exec.
#[derive(Clone)]
pub struct SpawnRegistrationProof(String);
impl SpawnRegistrationProof {
    pub(crate) fn issue() -> std::io::Result<Self> {
        crate::process::random_token().map(Self)
    }
    pub fn as_header_value(&self) -> &str {
        &self.0
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChildSpawnResult {
    pub id: LiveSessionId,
    pub board_path: String,
    pub cwd: String,
    pub worktree_branch: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpawnWaitOutcome {
    Registered(SessionBinding),
    Failed(String),
    TimedOut,
    WaiterCancelled,
    HubStopped,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WrappedStartKind {
    Bare,
    /// Source spawn-grid passes only provider and label; log setup is optional.
    Grid,
    OrdinaryAi {
        delegation: bool,
    },
}
pub struct WrappedStartRequest {
    pub spec: WrappedSpawnSpec,
    pub policy: ResolvedSpawnPolicy,
    pub kind: WrappedStartKind,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpawnStartOutcome {
    Started { pid: u32 },
    Failed(String),
    WaiterCancelled,
    HubStopped,
}
pub trait WrappedSessionSpawner: Send + Sync {
    /// A native Start receipt is distinct from registration. Implementations
    /// without native start support fail explicitly, retaining existing callers.
    fn start_wrapped<'a>(
        &'a self,
        _request: WrappedStartRequest,
        _waiter: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnStartOutcome> {
        Box::pin(async { SpawnStartOutcome::Failed("native wrapper Start is unavailable".into()) })
    }
    fn spawn_and_wait<'a>(
        &'a self,
        spec: WrappedSpawnSpec,
        wait: Duration,
        waiter: &'a HttpWaitCancellation,
    ) -> CoreFuture<'a, SpawnWaitOutcome>;
}
#[derive(Clone)]
pub struct PendingSpawnConfirmation {
    pub id: SpawnConfirmationId,
    pub parent: LiveSessionId,
    pub requested_provider: String,
    pub body: ResolvedChildSpawn,
    pub requested_at: Timestamp,
    pub admission: AdmissionId,
    pub waiter_gone: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfirmationOutcome {
    Approved(ChildSpawnResult),
    Refused,
    Superseded,
    ParentEnded,
    SpawnFailed(SessionError),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfirmationWaitOutcome {
    Decided(ConfirmationOutcome),
    WaiterCancelled,
    HubStopped,
}
pub struct ConfirmationRequest {
    pub parent: LiveSessionId,
    pub requested_provider: String,
    pub body: ResolvedChildSpawn,
    pub requested_at: Timestamp,
}
pub struct ConfirmationRegistration {
    pub pending: PendingSpawnConfirmation,
    pub effects: CoreEffects,
    pub waiter: Box<dyn ConfirmationWaiter>,
}
/// Captured at registration; a fast completion cannot disappear before wait.
/// Drop/cancellation marks only this original request waiter as gone.
pub trait ConfirmationWaiter: Send {
    fn id(&self) -> &SpawnConfirmationId;
    fn wait(
        self: Box<Self>,
        cancel: HttpWaitCancellation,
    ) -> CoreFuture<'static, ConfirmationWaitOutcome>;
}
/// Before run, dropping this handle releases only its matching decision lease,
/// never restoring a superseded/expired entry. The HTTP owner obtains a task
/// permit first. run commits synchronously (not in a lazy future body); the
/// caller transfers its returned future without an intervening await or a
/// fallible enqueue, before returning success. Dropping a committed future,
/// even before first poll, releases admission and publishes failure, without
/// pretending asynchronous board/socket cleanup was completed.
pub trait AcceptedSpawnDecision: Send {
    fn id(&self) -> &SpawnConfirmationId;
    fn run(
        self: Box<Self>,
        cancel: TaskCancellation,
    ) -> Result<CoreFuture<'static, ConfirmationOutcome>, SessionError>;
}
pub trait SpawnConfirmations: Send + Sync {
    fn pending(&self) -> Vec<PendingSpawnConfirmation>;
    fn register(
        &self,
        request: ConfirmationRequest,
    ) -> Result<ConfirmationRegistration, AdmissionError>;
    fn accept_decision(
        self: Arc<Self>,
        response: SpawnConfirmationResponse,
        request: VerifiedConfirmationRequest,
        now: Timestamp,
    ) -> Result<Box<dyn AcceptedSpawnDecision>, SessionError>;
    fn parent_ended(&self, parent: LiveSessionId) -> CoreEffects;
}

/// Reference admission transaction state, to be held INSIDE the one SessionCore
/// state lock/actor (never a second live session manager). Exclusive &mut access
/// makes count/check/reserve indivisible. It runs no provider or application route.
#[derive(Default)]
pub struct AdmissionState {
    live: BTreeMap<LiveSessionId, AdmissionSession>,
    reservations: BTreeMap<AdmissionId, AdmissionReservation>,
    spawns: BTreeMap<SpawnAttemptId, String>,
    updates: BTreeMap<String, ProviderUpdateId>,
    running_relays: BTreeMap<LiveSessionId, i64>,
    next_id: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmissionSession {
    pub parent: LiveSessionId,
    pub provider: String,
}
impl AdmissionState {
    pub fn provider_updating(&self, provider: &str) -> bool {
        self.updates.contains_key(provider)
    }

    fn next(&mut self) -> Result<u64, ProviderAdmissionError> {
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or(ProviderAdmissionError::IdentityExhausted)?;
        Ok(self.next_id)
    }
    pub fn observe_session(
        &mut self,
        id: LiveSessionId,
        session: AdmissionSession,
    ) -> Result<(), ProviderAdmissionError> {
        if self.updates.contains_key(&session.provider) {
            return Err(ProviderAdmissionError::AlreadyUpdating);
        }
        self.live.insert(id, session);
        Ok(())
    }
    /// A transport disconnect must NOT call this: retained cards count for updates.
    pub fn dismiss_session(&mut self, id: LiveSessionId) {
        self.live.remove(&id);
    }
    pub fn set_running_relays(&mut self, parent: LiveSessionId, running: i64) {
        self.running_relays.insert(parent, running.max(0));
    }
    pub fn provider_session_count(&self, provider: &str) -> usize {
        self.live
            .values()
            .filter(|s| s.provider == provider)
            .count()
    }
    pub fn begin_provider_spawn(
        &mut self,
        provider: &str,
    ) -> Result<ProviderSpawnLease, ProviderAdmissionError> {
        if self.updates.contains_key(provider) {
            return Err(ProviderAdmissionError::AlreadyUpdating);
        }
        let id = SpawnAttemptId(self.next()?);
        self.spawns.insert(id, provider.to_owned());
        Ok(ProviderSpawnLease {
            id,
            provider: provider.to_owned(),
        })
    }
    pub fn end_provider_spawn(&mut self, lease: &ProviderSpawnLease) -> bool {
        if self.spawns.get(&lease.id) != Some(&lease.provider) {
            return false;
        }
        self.spawns.remove(&lease.id);
        true
    }
    /// Converts a pending launch to a registered card in the same transaction.
    pub fn register_spawn(
        &mut self,
        lease: &ProviderSpawnLease,
        id: LiveSessionId,
        parent: LiveSessionId,
    ) -> Result<(), ProviderAdmissionError> {
        if self.spawns.get(&lease.id) != Some(&lease.provider) {
            return Err(ProviderAdmissionError::StaleLease);
        }
        self.observe_session(
            id,
            AdmissionSession {
                parent,
                provider: lease.provider.clone(),
            },
        )?;
        self.spawns.remove(&lease.id);
        Ok(())
    }
    pub fn begin_provider_update(
        &mut self,
        provider: &str,
    ) -> Result<ProviderUpdateLease, ProviderAdmissionError> {
        if self.updates.contains_key(provider) {
            return Err(ProviderAdmissionError::AlreadyUpdating);
        }
        let count = self.provider_session_count(provider);
        if count > 0 {
            return Err(ProviderAdmissionError::RunningSessions { count });
        }
        let count = self
            .spawns
            .values()
            .filter(|p| p.as_str() == provider)
            .count();
        if count > 0 {
            return Err(ProviderAdmissionError::PendingSpawns { count });
        }
        let id = ProviderUpdateId(self.next()?);
        self.updates.insert(provider.to_owned(), id);
        Ok(ProviderUpdateLease {
            provider: provider.to_owned(),
            id,
        })
    }
    pub fn end_provider_update(&mut self, lease: &ProviderUpdateLease) -> bool {
        if self.updates.get(&lease.provider) != Some(&lease.id) {
            return false;
        }
        self.updates.remove(&lease.provider);
        true
    }
    pub fn reserve_children(
        &mut self,
        request: AdmissionRequest,
        limits: AdmissionLimits,
    ) -> Result<AdmissionReservation, AdmissionError> {
        let human_capacity = request.origin.is_human();
        self.reserve_capacity(
            request.parent,
            request.slots,
            request.replace,
            limits,
            human_capacity,
        )
    }
    /// Source pre-decision confirmation capacity, without fabricating a human
    /// request proof. Only the actual SessionEngine transaction calls this lane.
    pub(crate) fn reserve_confirmation(
        &mut self,
        parent: LiveSessionId,
        slots: i64,
        replace: Option<AdmissionId>,
    ) -> Result<AdmissionReservation, AdmissionError> {
        self.reserve_capacity(
            parent,
            slots,
            replace,
            AdmissionLimits {
                max_children_per_parent: 256,
                max_total_sessions: 257,
            },
            true,
        )
    }
    fn reserve_capacity(
        &mut self,
        parent: LiveSessionId,
        slots: i64,
        replace: Option<AdmissionId>,
        limits: AdmissionLimits,
        human: bool,
    ) -> Result<AdmissionReservation, AdmissionError> {
        if slots <= 0 {
            return Err(AdmissionError::InvalidSlots);
        }
        if !self.live.contains_key(&parent) {
            return Err(AdmissionError::ParentNotFound);
        }
        if let Some(id) = &replace
            && self.reservations.get(id).is_none_or(|r| r.parent != parent)
        {
            return Err(AdmissionError::InvalidReplacement);
        }
        let child_limit = if human {
            256
        } else if limits.max_children_per_parent <= 0 {
            10
        } else {
            limits.max_children_per_parent.min(256)
        };
        let total_limit = limits.max_total_sessions.max(child_limit + 1);
        let child_count = self.live.values().filter(|s| s.parent == parent).count() as i64;
        let mut parent_reserved = 0;
        let mut total_reserved = 0;
        for (id, r) in &self.reservations {
            if replace.as_ref() == Some(id) {
                continue;
            }
            total_reserved += r.slots;
            if r.parent == parent {
                parent_reserved += r.slots;
            }
        }
        let running_relays = *self.running_relays.get(&parent).unwrap_or(&0);
        let used = child_count + parent_reserved;
        if slots > child_limit.saturating_sub(used) {
            return Err(AdmissionError::ChildrenPerParent {
                running_relays,
                used,
                maximum: child_limit,
            });
        }
        let used = self.live.len() as i64 + total_reserved;
        if !human && slots > total_limit.saturating_sub(used) {
            return Err(AdmissionError::TotalSessions {
                running_relays,
                used,
                maximum: total_limit,
            });
        }
        let sequence = self.next().map_err(|_| AdmissionError::IdentityExhausted)?;
        let id = AdmissionId(format!("oa-{sequence}"));
        let reservation = AdmissionReservation {
            id: id.clone(),
            parent,
            slots,
        };
        if let Some(old) = replace {
            self.reservations.remove(&old);
        }
        self.reservations.insert(id, reservation.clone());
        Ok(reservation)
    }
    pub fn release_children(&mut self, id: &AdmissionId) -> bool {
        self.reservations.remove(id).is_some()
    }
    pub fn consume_children(&mut self, id: &AdmissionId, slots: i64) -> bool {
        if slots <= 0 {
            return false;
        }
        let Some(r) = self.reservations.get_mut(id) else {
            return false;
        };
        if slots >= r.slots {
            self.reservations.remove(id);
        } else {
            r.slots -= slots;
        }
        true
    }
    pub fn admission_matches(&self, id: &AdmissionId, parent: LiveSessionId, slots: i64) -> bool {
        slots > 0
            && self
                .reservations
                .get(id)
                .is_some_and(|r| r.parent == parent && r.slots >= slots)
    }
    /// Parent teardown releases confirmations' slots without recording refusal.
    pub fn release_parent(&mut self, parent: LiveSessionId) -> Vec<AdmissionId> {
        let ids: Vec<_> = self
            .reservations
            .iter()
            .filter(|(_, r)| r.parent == parent)
            .map(|(id, _)| id.clone())
            .collect();
        for id in &ids {
            self.reservations.remove(id);
        }
        ids
    }
}

impl super::SessionActivity {
    pub fn is_idle(&self) -> bool {
        self.output_idle && !self.workflow_active
    }
    pub fn normalize(&mut self) {
        if self.awaiting_approval {
            self.awaiting_user = true;
        }
    }
    pub fn display_state(&self) -> &'static str {
        if self.awaiting_user {
            "waiting"
        } else if self.workflow_active || !self.output_idle {
            "running"
        } else {
            "standby"
        }
    }
}

#[cfg(test)]
mod internal_contract_tests {
    use super::*;
    #[test]
    fn human_decision_trims_supplied_fields_and_preserves_absent_memory_and_origin() {
        let proof = VerifiedConfirmationRequest::after_server_authentication(AuthEpoch(1));
        let mut body = ChildSpawnRequest {
            provider: "previous".into(),
            model: "old-model".into(),
            effort: "old-effort".into(),
            execution_mode: "old-mode".into(),
            permission_preset: "old-preset".into(),
            remember_permission: Some(true),
            claimed_origin: String::new(),
            ..Default::default()
        };
        body.apply_human_decision(
            &SpawnConfirmationResponse {
                provider: " \t ".into(),
                model: "\n".into(),
                ..Default::default()
            },
            &proof,
        );
        assert_eq!(body.provider, "previous");
        assert_eq!(body.model, "old-model");
        assert_eq!(body.effort, "old-effort");
        assert_eq!(body.remember_permission, Some(true));
        body.apply_human_decision(
            &SpawnConfirmationResponse {
                provider: " codex ".into(),
                model: " model ".into(),
                effort: Some("  ".into()),
                execution_mode: Some(" headless ".into()),
                permission_preset: Some(" strict ".into()),
                remember_permission: Some(false),
                ..Default::default()
            },
            &proof,
        );
        assert_eq!(body.provider, "codex");
        assert_eq!(body.model, "model");
        assert!(body.effort.is_empty());
        assert_eq!(body.execution_mode, "headless");
        assert_eq!(body.permission_preset, "strict");
        assert_eq!(body.remember_permission, Some(false));
        assert!(
            body.claimed_origin.is_empty(),
            "confirmation must not turn autonomous launch origin into UI"
        );
    }

    #[test]
    fn verified_decision_keeps_absent_but_clears_explicit_empty() {
        let proof = VerifiedConfirmationRequest::after_server_authentication(AuthEpoch(4));
        let mut request = ChildSpawnRequest {
            provider: "codex".into(),
            effort: "high".into(),
            execution_mode: "headless".into(),
            ..Default::default()
        };
        let decision = SpawnConfirmationResponse {
            effort: Some(String::new()),
            remember_permission: Some(false),
            grant_folder_trust: Some(true),
            approved: true,
            ..Default::default()
        };
        request.apply_human_decision(&decision, &proof);
        assert_eq!(request.provider, "codex");
        assert_eq!(request.effort, "");
        assert_eq!(request.execution_mode, "headless");
        assert_eq!(request.remember_permission, Some(false));
        assert!(
            InternalSpawnGrants::default()
                .with_human_folder_trust(&decision, &proof)
                .grant_folder_trust()
        );
    }
    #[test]
    fn human_admission_skips_autonomous_total_but_keeps_256_cap() {
        let mut state = AdmissionState::default();
        state
            .observe_session(
                LiveSessionId(1),
                AdmissionSession {
                    parent: LiveSessionId(0),
                    provider: "codex".into(),
                },
            )
            .unwrap();
        let proof = VerifiedUiOrigin::after_server_verification(AuthEpoch(1));
        let request = AdmissionRequest {
            parent: LiveSessionId(1),
            slots: 256,
            origin: VerifiedSpawnOrigin::HumanUi(proof),
            replace: None,
        };
        let lease = state
            .reserve_children(
                request.clone(),
                AdmissionLimits {
                    max_children_per_parent: 1,
                    max_total_sessions: 1,
                },
            )
            .unwrap();
        assert_eq!(lease.slots, 256);
        assert!(matches!(
            state.reserve_children(
                AdmissionRequest {
                    slots: 1,
                    ..request
                },
                AdmissionLimits {
                    max_children_per_parent: 1,
                    max_total_sessions: 1
                }
            ),
            Err(AdmissionError::ChildrenPerParent { maximum: 256, .. })
        ));
    }
}
