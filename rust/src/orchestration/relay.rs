//! Relay records and filesystem helpers. Session/admission ownership remains in
//! SessionEngine; the application relay driver owns transitions, never an AI.
pub mod store;
pub mod text;
mod trial_git;
pub(crate) mod wire;
pub mod worktree;

use crate::proto::{
    RelayEvent, RelayStatus,
    core::*,
    time::{self, Timestamp},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};

pub const IMPLEMENTATION: &str = "implementation";
pub const STRONG: &str = "implementation-strong";
pub const REVIEW: &str = "review";
pub const ROLES: [&str; 3] = [IMPLEMENTATION, STRONG, REVIEW];

#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Role {
    pub provider: String,
    pub model: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub subscription: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub effort: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub execution_mode: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub permission_preset: String,
}
impl From<crate::application::orchestration_program::RoleSettings> for Role {
    fn from(role: crate::application::orchestration_program::RoleSettings) -> Self {
        Self {
            provider: role.provider,
            model: role.model,
            subscription: role.subscription,
            effort: role.effort,
            execution_mode: role.execution_mode,
            permission_preset: role.permission_preset,
        }
    }
}
impl Role {
    pub fn overlay(&mut self, other: &Self) {
        for (current, supplied) in [
            (&mut self.provider, &other.provider),
            (&mut self.model, &other.model),
            (&mut self.subscription, &other.subscription),
            (&mut self.effort, &other.effort),
            (&mut self.execution_mode, &other.execution_mode),
            (&mut self.permission_preset, &other.permission_preset),
        ] {
            if !supplied.trim().is_empty() {
                current.clone_from(supplied);
            }
        }
    }
}

/// Exact version-one persisted record. Unknown fields/versions are tolerated as
/// in the fixed Go loader; validation of new requests is a separate boundary.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RelayFile {
    pub version: i64,
    pub orchestration_id: String,
    pub board_path: String,
    pub parent_session_id: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub parent_started_at: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub parent_provider: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub parent_cwd: String,
    /// Repository cwd that resolves a relative worktree root.
    /// Adoption may replace `parent_cwd` with the new session cwd.
    /// Empty on legacy records: `repository_cwd` falls back to `parent_cwd`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub worktree_origin_cwd: String,
    pub plan_path: String,
    pub mode: String,
    pub max_rounds: i64,
    pub escalate_after: i64,
    pub completed_cs: i64,
    pub round: i64,
    pub final_seen: bool,
    pub state: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub reason: String,
    pub active_implementer: String,
    #[serde(deserialize_with = "wire::null_default")]
    pub roles: BTreeMap<String, Role>,
    #[serde(deserialize_with = "wire::null_default")]
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, String>,
    /// Additive recovery safety field; the fixed Go loader ignores it.
    #[serde(
        default,
        deserialize_with = "wire::null_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub revoked_child_labels: Vec<String>,
    pub child_cwd: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub implementation_label: String,
    pub implementation_session_id: i64,
    #[serde(skip_serializing_if = "is_zero")]
    pub implementation_progress_id: i64,
    pub impl_done_baseline: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub strong_label: String,
    pub strong_session_id: i64,
    #[serde(skip_serializing_if = "is_zero")]
    pub strong_progress_id: i64,
    pub strong_done_baseline: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub review_label: String,
    pub review_session_id: i64,
    #[serde(skip_serializing_if = "is_zero")]
    pub review_progress_id: i64,
    pub review_done_baseline: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub review_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_verdict: Option<text::Verdict>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub worktree_path: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub branch: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub base_commit: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub last_reviewed_commit: String,
    #[serde(deserialize_with = "wire::null_default")]
    #[serde(serialize_with = "wire::serialize_events")]
    pub events: Vec<RelayEvent>,
    pub updated_at: String,
}
impl RelayFile {
    /// Cwd used to resolve and confine the relay worktree.
    /// A stored origin wins. Legacy records keep using `parent_cwd`.
    pub fn repository_cwd(&self) -> &str {
        if !self.worktree_origin_cwd.trim().is_empty() {
            &self.worktree_origin_cwd
        } else {
            &self.parent_cwd
        }
    }
    /// Freeze the original repository cwd before adoption overwrites `parent_cwd`.
    /// Same-tree relays have no worktree root to preserve. A second adoption
    /// must not replace an origin that is already stored.
    pub fn adopt_parent_context(&mut self, cwd: &str) {
        if self.mode == "worktree"
            && self.worktree_origin_cwd.trim().is_empty()
            && !self.parent_cwd.trim().is_empty()
        {
            self.worktree_origin_cwd.clone_from(&self.parent_cwd);
        }
        self.parent_cwd = cwd.to_owned();
    }
    pub fn terminal(&self) -> bool {
        matches!(self.state.as_str(), "completed" | "stopped")
    }
    pub fn resumable(&self) -> bool {
        self.state == "stopped"
            && matches!(
                self.reason.as_str(),
                "hub_restart" | "child_exited" | "timeout"
            )
    }
    pub fn current_c(&self) -> i64 {
        self.completed_cs + i64::from(!self.terminal())
    }
    pub fn board_dir(&self) -> PathBuf {
        PathBuf::from(&self.board_path)
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .to_owned()
    }
    pub fn child_id(&self, role: &str) -> i64 {
        match role {
            IMPLEMENTATION => self.implementation_session_id,
            STRONG => self.strong_session_id,
            REVIEW => self.review_session_id,
            _ => 0,
        }
    }
    pub fn role_of(&self, child: i64) -> Option<&'static str> {
        ROLES
            .into_iter()
            .find(|role| child != 0 && self.child_id(role) == child)
    }
    pub fn child_ids(&self) -> Vec<i64> {
        let mut seen = BTreeSet::new();
        ROLES
            .into_iter()
            .map(|r| self.child_id(r))
            .filter(|id| *id > 0 && seen.insert(*id))
            .collect()
    }
    pub fn progress_id(&self, role: &str) -> i64 {
        let id = match role {
            IMPLEMENTATION => self.implementation_progress_id,
            STRONG => self.strong_progress_id,
            REVIEW => self.review_progress_id,
            _ => 0,
        };
        if id != 0 { id } else { self.child_id(role) }
    }
    pub fn baseline(&self, role: &str) -> i64 {
        match role {
            IMPLEMENTATION => self.impl_done_baseline,
            STRONG => self.strong_done_baseline,
            REVIEW => self.review_done_baseline,
            _ => 0,
        }
    }
    pub fn set_baseline(&mut self, role: &str, count: i64) {
        match role {
            IMPLEMENTATION => self.impl_done_baseline = count,
            STRONG => self.strong_done_baseline = count,
            REVIEW => self.review_done_baseline = count,
            _ => {}
        }
    }
    pub fn set_child(&mut self, role: &str, id: i64, label: String, progress_id: i64) {
        match role {
            IMPLEMENTATION => {
                self.implementation_session_id = id;
                self.implementation_label = label;
                self.implementation_progress_id = progress_id;
            }
            STRONG => {
                self.strong_session_id = id;
                self.strong_label = label;
                self.strong_progress_id = progress_id;
            }
            REVIEW => {
                self.review_session_id = id;
                self.review_label = label;
                self.review_progress_id = progress_id;
            }
            _ => {}
        }
    }
    pub fn child_label(&self, role: &str) -> &str {
        match role {
            IMPLEMENTATION => &self.implementation_label,
            STRONG => &self.strong_label,
            REVIEW => &self.review_label,
            _ => "",
        }
    }
    pub fn headless(&self, role: &str) -> bool {
        self.roles
            .get(role)
            .is_some_and(|r| crate::config::is_headless_execution_mode(&r.execution_mode))
    }
    pub fn awaited(&self) -> i64 {
        match self.state.as_str() {
            "implementing" | "fixing" => self.child_id(&self.active_implementer),
            "reviewing" => self.review_session_id,
            _ => 0,
        }
    }
    pub fn status(&self) -> RelayStatus {
        RelayStatus {
            orchestration_id: self.orchestration_id.clone(),
            plan_path: self.plan_path.clone(),
            mode: self.mode.clone(),
            state: self.state.clone(),
            reason: self.reason.clone(),
            completed_cs: self.completed_cs,
            round: self.round,
            max_rounds: self.max_rounds,
            final_seen: self.final_seen,
            implementation_session_id: self.implementation_session_id,
            strong_session_id: self.strong_session_id,
            active_implementer: self.active_implementer.clone(),
            escalate_after: self.escalate_after,
            review_session_id: self.review_session_id,
            review_path: self.review_path.clone(),
            worktree_path: self.worktree_path.clone(),
            branch: self.branch.clone(),
            base_commit: self.base_commit.clone(),
            updated_at: self.updated_at.clone(),
        }
    }
    pub fn prompts(&self) -> text::PromptContext {
        text::PromptContext {
            orchestration_id: self.orchestration_id.clone(),
            plan_path: self.plan_path.clone(),
            mode: self.mode.clone(),
            board_dir: self.board_dir().to_string_lossy().into_owned(),
            worktree_path: self.worktree_path.clone(),
            branch: self.branch.clone(),
            base_commit: self.base_commit.clone(),
            last_reviewed_commit: self.last_reviewed_commit.clone(),
            active_impl: self.active_implementer.clone(),
            review_path: self.review_path.clone(),
            current_c: self.current_c(),
            completed_cs: self.completed_cs,
            round: self.round,
            has_strong_role: self.roles.contains_key(STRONG),
            extra: self.extra.clone(),
            last_verdict: self.last_verdict.clone(),
        }
    }
    pub fn event(&mut self, kind: &str, text: String, at: Timestamp) {
        self.events.push(RelayEvent {
            at: time::format_rfc3339(at).unwrap_or_default(),
            kind: kind.into(),
            c: self.current_c(),
            round: self.round,
            text,
            review_path: self.review_path.clone(),
            ..Default::default()
        });
    }
}

pub struct Run {
    pub cleanup_in_progress: Arc<AtomicBool>,
    pub file: RelayFile,
    pub sequence: u64,
    pub admission: AdmissionId,
    pub parent_attached: bool,
    pub awaiting_reconnect: bool,
    pub restored_at: Timestamp,
    pub reconnected: BTreeSet<String>,
    pub nudged: BTreeSet<i64>,
    pub timers: BTreeMap<i64, ChildTimer>,
}
#[derive(Default)]
pub struct ChildTimer {
    pub assigned_at: Option<Timestamp>,
    pub last_write_at: Option<Timestamp>,
    pub stamp: Option<(u64, std::time::SystemTime)>,
    pub standby_since: Option<Timestamp>,
    pub startup_wait_notified: bool,
    pub nudge_wait_notified: bool,
    pub exit_handled: bool,
    pub pending_done_at: Option<Timestamp>,
}
fn is_zero(value: &i64) -> bool {
    *value == 0
}
impl Run {
    pub fn new(mut file: RelayFile, now: Timestamp) -> Self {
        file.updated_at = time::parse_rfc3339(&file.updated_at)
            .ok()
            .and_then(|at| {
                let text = &file.updated_at;
                let offset = if text.ends_with('Z') {
                    0
                } else {
                    let tail = &text[text.len() - 6..];
                    let hour = tail[1..3].parse::<i32>().ok()?;
                    let minute = tail[4..6].parse::<i32>().ok()?;
                    (hour * 3600 + minute * 60) * if tail.starts_with('-') { -1 } else { 1 }
                };
                time::format_with_offset(at, offset, false).ok()
            })
            .unwrap_or_default();
        if file.active_implementer.is_empty() {
            file.active_implementer = IMPLEMENTATION.into();
        }
        if file.max_rounds <= 0 {
            file.max_rounds = 3;
        }
        if file.escalate_after <= 0 {
            file.escalate_after = 2;
        }
        Self {
            file,
            cleanup_in_progress: Arc::new(AtomicBool::new(false)),
            sequence: 0,
            admission: AdmissionId::default(),
            parent_attached: false,
            awaiting_reconnect: false,
            restored_at: now,
            reconnected: BTreeSet::new(),
            nudged: BTreeSet::new(),
            timers: BTreeMap::new(),
        }
    }
}
