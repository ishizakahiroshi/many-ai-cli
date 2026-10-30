//! Read-only session launch through the existing ordinary Start path, so
//! `SessionEngine` stays the only session admission/registration owner and the
//! owner's authenticated confirmation request is checked against its epoch.
use super::github::RepoName;
use crate::application::ordinary_spawn::{OrdinarySpawnRequest, OrdinarySpawnService};
use crate::proto::{core::*, time::Timestamp};
use std::{path::PathBuf, sync::Arc};

/// Only providers whose read-only flags the Hub already validates. Others are
/// refused rather than launched with an unverified permission model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadOnlyProvider {
    /// `permission_mode = plan`
    Claude,
    /// `sandbox = read-only`, `ask_for_approval = untrusted`
    Codex,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadOnlyLaunch {
    /// Correlates the SessionEngine registration with the journal row.
    pub client_request_id: String,
    pub provider: ReadOnlyProvider,
    pub cwd: PathBuf,
    pub repo: RepoName,
    pub pull_request: u64,
    pub head_sha: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaunchError {
    /// The owner's confirmation belongs to an earlier Hub authentication.
    AuthenticationExpired,
    /// Anything after admission began: the session may exist.
    Unknown(String),
}
#[allow(async_fn_in_trait)]
pub trait ReadOnlyLauncher {
    fn auth_epoch(&self) -> AuthEpoch;
    async fn launch(
        &self,
        request: &ReadOnlyLaunch,
        owner: &VerifiedConfirmationRequest,
    ) -> Result<(), LaunchError>;
}

/// The prompt names only the typed target. No Deskly wire text, PR body or
/// comment is ever interpolated.
pub fn initial_prompt(request: &ReadOnlyLaunch) -> String {
    format!(
        "Read-only review request from Deskly. Repository {} pull request #{} at commit {}. \
         Do not modify files, run write operations, push, comment or merge. \
         If the checked-out HEAD is not {}, stop and report the mismatch. \
         Summarize the changes and risks.",
        request.repo.as_str(),
        request.pull_request,
        request.head_sha,
        request.head_sha
    )
}
pub fn ordinary_request(request: &ReadOnlyLaunch) -> OrdinarySpawnRequest {
    let mut body = OrdinarySpawnRequest {
        client_request_id: request.client_request_id.clone(),
        cwd: request.cwd.to_string_lossy().into_owned(),
        label: format!("deskly-pr-{}", request.pull_request),
        isolate_worktree: Some(false),
        delegation: Some(false),
        initial_prompt: initial_prompt(request),
        ..Default::default()
    };
    match request.provider {
        ReadOnlyProvider::Claude => {
            body.provider = "claude".into();
            body.permission_mode = "plan".into();
        }
        ReadOnlyProvider::Codex => {
            body.provider = "codex".into();
            body.sandbox = "read-only".into();
            body.ask_for_approval = "untrusted".into();
        }
    }
    body
}

pub struct OrdinarySpawnLauncher {
    pub service: Arc<OrdinarySpawnService>,
    pub now: Arc<dyn Fn() -> Timestamp + Send + Sync>,
    pub offset: i32,
}
impl ReadOnlyLauncher for OrdinarySpawnLauncher {
    fn auth_epoch(&self) -> AuthEpoch {
        self.service.core.auth_epoch()
    }
    async fn launch(
        &self,
        request: &ReadOnlyLaunch,
        owner: &VerifiedConfirmationRequest,
    ) -> Result<(), LaunchError> {
        if owner.auth_epoch() != self.auth_epoch() {
            return Err(LaunchError::AuthenticationExpired);
        }
        self.service
            .spawn(
                ordinary_request(request),
                owner,
                &HttpWaitCancellation::default(),
                (self.now)(),
                self.offset,
            )
            .await
            .map(drop)
            .map_err(|error| LaunchError::Unknown(error.code.to_owned()))
    }
}
