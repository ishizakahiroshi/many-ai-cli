//! Read-only provider artifacts resolved from the authoritative session owner.
mod discovery;
mod grok;
#[cfg(test)]
mod tests;
pub use discovery::Location as AgentLogLocation;
pub use grok::resolve_grok;
/// Shared provider-owned transcript resolver for HTTP and observation workers.
pub fn resolve_transcript(
    paths: &RuntimePaths,
    identity: &TranscriptSessionIdentity,
) -> Option<PathBuf> {
    if identity.provider == "grok" {
        resolve_grok(paths, identity)
    } else {
        discovery::structured_path(paths, identity)
    }
}
use crate::{
    application::host_actions::{HostDispatch, OpenKind},
    approval::transcript::{
        parser::{ParseState, ProviderFormat},
        reader::ReadBudget,
    },
    config::RuntimePaths,
    process::Cancellation,
    proto::{
        core::{LiveSessionId, TranscriptSessionIdentity},
        time::parse_rfc3339,
    },
    terminal::session::SessionEngine,
};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
pub type Warning = dyn Fn(&'static str) + Send + Sync;
#[derive(Debug)]
pub struct Error {
    pub status: u16,
    pub code: &'static str,
    pub detail: &'static str,
}
impl Error {
    fn bad(detail: &'static str) -> Self {
        Self {
            status: 400,
            code: "bad_request",
            detail,
        }
    }
    fn missing(detail: &'static str) -> Self {
        Self {
            status: 404,
            code: "not_found",
            detail,
        }
    }
}
pub(super) fn trim_space(value: &str) -> &str {
    crate::application::mobile_connect::trim_space(value)
}
pub fn parse_integer(raw: &str) -> Option<i64> {
    let digits = raw.strip_prefix(['+', '-']).unwrap_or(raw);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    raw.parse().ok()
}
pub fn session_id(raw: &str) -> Result<LiveSessionId, Error> {
    let id = parse_integer(raw)
        .filter(|v| *v > 0)
        .ok_or_else(|| Error::bad("invalid session_id"))?;
    Ok(LiveSessionId(id))
}
pub struct AgentHistory {
    core: Arc<SessionEngine>,
    paths: RuntimePaths,
    actor_home: PathBuf,
    dispatch: Arc<dyn HostDispatch>,
    warning: Arc<Warning>,
}
impl AgentHistory {
    pub fn new(
        core: Arc<SessionEngine>,
        paths: RuntimePaths,
        actor_home: PathBuf,
        dispatch: Arc<dyn HostDispatch>,
        warning: Arc<Warning>,
    ) -> Arc<Self> {
        Arc::new(Self {
            core,
            paths,
            actor_home,
            dispatch,
            warning,
        })
    }
    fn identity(&self, id: LiveSessionId) -> Result<TranscriptSessionIdentity, Error> {
        self.core
            .details(id)
            .map(|v| v.transcript)
            .ok_or_else(|| Error::missing("session not found"))
    }
    pub fn location(&self, id: LiveSessionId) -> AgentLogLocation {
        match self.identity(id) {
            Ok(identity) => discovery::location(&self.paths, &identity),
            Err(_) => AgentLogLocation {
                reason: "session not found".into(),
                ..Default::default()
            },
        }
    }
    pub fn chat(
        &self,
        id: LiveSessionId,
        raw_limit: &str,
        raw_cursor: &str,
        raw_offset: &str,
    ) -> Result<Value, Error> {
        let identity = self.identity(id)?;
        let Some(format) = ProviderFormat::for_provider(&identity.provider) else {
            return Ok(empty_chat(false));
        };
        let limit = limit(raw_limit)?;
        let raw = if !raw_cursor.is_empty() {
            raw_cursor
        } else {
            raw_offset
        };
        let cursor = if raw.is_empty() {
            -1
        } else {
            parse_integer(raw)
                .filter(|v| *v >= -1)
                .ok_or_else(|| Error::bad("invalid cursor"))?
        };
        let Some(path) = discovery::structured_path(&self.paths, &identity) else {
            return Ok(empty_chat(true));
        };
        let file =
            crate::application::session_observations::subagents::open_artifact(&self.paths, &path)
                .map_err(|_| Error::missing("agent transcript not readable"))?;
        let mut state = ParseState::new(limit, 8 * 1024 * 1024);
        let messages = state
            .read_tail_file(
                file,
                format,
                (cursor >= 0).then_some(cursor as u64),
                &ReadBudget::page(),
            )
            .map_err(|_| {
                (self.warning)("agent_chat_read");
                Error::missing("agent transcript not readable")
            })?;
        let offset = state.last_read.offset;
        Ok(
            json!({"ok":true,"available":true,"total":-1,"total_known":false,"offset":offset,"cursor":offset,"next_cursor":offset,"has_more":offset>0,"messages":messages}),
        )
    }
    pub fn grok_history(
        &self,
        id: LiveSessionId,
        raw_limit: &str,
        raw_offset: &str,
    ) -> Result<Value, Error> {
        let mut identity = self.identity(id)?;
        if identity.provider.is_empty() {
            return Err(Error::missing("session not found"));
        }
        if identity.provider != "grok" {
            return Err(Error::bad("not a grok session"));
        }
        if discovery::home(&identity.grok_home, &identity.home_dir, ".grok").is_none() {
            if self.actor_home.as_os_str().is_empty() {
                return Err(Error::missing("user home directory unavailable"));
            }
            identity.home_dir = self.actor_home.to_string_lossy().into_owned();
        }
        if parse_rfc3339(&identity.started_at).is_err() {
            return Err(Error::missing("session start time unavailable"));
        }
        let path = grok::resolve_grok(&self.paths, &identity)
            .ok_or_else(|| Error::missing("grok chat history not found"))?;
        // Fixed source reads history before validating pagination. Parse pagination without publishing its errors, then scan
        // once into the bounded page before returning validation errors.
        let limit_result = limit(raw_limit);
        let offset_result = if raw_offset.is_empty() {
            Ok(-1)
        } else {
            parse_integer(raw_offset).ok_or_else(|| Error::bad("invalid offset"))
        };
        let read_limit = limit_result.as_ref().copied().unwrap_or(50);
        let read_offset = offset_result.as_ref().copied().unwrap_or(-1);
        let (total, offset, messages) = grok::page(
            &self.paths,
            &path,
            (read_offset >= 0).then_some(read_offset as usize),
            read_limit,
        )
        .map_err(|_| Error::missing("grok chat history not readable"))?;
        limit_result?;
        offset_result?;
        Ok(json!({"ok":true,"total":total,"offset":offset,"messages":messages}))
    }
    pub fn open_log(&self, id: LiveSessionId, cancel: &Cancellation) -> Result<Value, Error> {
        let location = self.location(id);
        if !location.available {
            return Err(Error {
                status: 404,
                code: "not_found",
                detail: reason(&location.reason),
            });
        }
        if self.paths.is_trial() || cancel.is_cancelled() {
            return Err(Error {
                status: 500,
                code: "open_failed",
                detail: "open failed",
            });
        }
        let path = Path::new(&location.path);
        let dir = if discovery::existing_dir(&self.paths, path) {
            path
        } else {
            path.parent().unwrap_or(Path::new("."))
        };
        self.dispatch
            .open(OpenKind::Directory, dir, "")
            .map_err(|_| Error {
                status: 500,
                code: "open_failed",
                detail: "open failed",
            })?;
        Ok(json!({"ok":true}))
    }
}
fn limit(raw: &str) -> Result<usize, Error> {
    if raw.is_empty() {
        return Ok(50);
    }
    parse_integer(raw)
        .filter(|v| *v > 0)
        .map(|v| v.min(200) as usize)
        .ok_or_else(|| Error::bad("invalid limit"))
}
fn empty_chat(available: bool) -> Value {
    json!({"ok":true,"available":available,"total":0,"total_known":true,"offset":0,"cursor":0,"next_cursor":0,"has_more":false,"messages":[]})
}
fn reason(value: &str) -> &'static str {
    const REASONS: &[&str] = &[
        "session not found",
        "Claude project directory is unavailable",
        "Claude Code transcript file not found yet",
        "Claude Code transcript directory not found yet",
        "Codex home directory is unavailable",
        "Codex rollout log is available after the first completed turn",
        "Grok session directory is unavailable",
        "Grok session start time is unavailable",
        "Grok Build chat history not found yet",
        "Copilot session directory is unavailable",
        "Copilot session start time is unavailable",
        "Copilot session state not found yet",
        "Cursor Agent session directory is unavailable",
        "Cursor Agent session start time is unavailable",
        "Cursor Agent chat history not found yet",
        "Command Code project directory is unavailable",
        "Command Code transcript not found yet",
        "opencode home directory is unavailable",
        "opencode session store not found",
        "This provider's native transcript location is not supported yet",
    ];
    REASONS
        .iter()
        .copied()
        .find(|v| *v == value)
        .unwrap_or("agent transcript not found")
}
