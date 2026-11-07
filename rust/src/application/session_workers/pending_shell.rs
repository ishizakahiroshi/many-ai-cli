//! Reads the unanswered shell calls of a Claude transcript for approval
//! authorization (audit U23, `approval::shell_authority`). Runs on a blocking
//! thread, never under the session lock, and reads only a bounded tail of the
//! parent transcript and of the most recent sub-agent logs (sub-agent calls are
//! drawn in the same prompt but logged beside the parent, see subagent_activity).
use super::{subagent_activity, transcript_path};
use crate::{
    application::session_observations::subagents::{artifact_metadata, entries, open_artifact},
    approval::shell_authority::pending_claude_shell_commands,
    config::RuntimePaths,
    proto::core::TranscriptSessionIdentity,
    terminal::session::PendingShellLookup,
};
use std::{
    io::{Read, Seek, SeekFrom},
    path::Path,
    sync::Arc,
    time::SystemTime,
};

/// A pending call is the newest record of its log; the tail only needs to hold it.
const TAIL_BYTES: u64 = 512 * 1024;
/// Sub-agent logs looked at, newest first.
const SUBAGENT_LOGS: usize = 4;

pub(crate) fn lookup(paths: RuntimePaths) -> PendingShellLookup {
    Arc::new(move |identity: TranscriptSessionIdentity| {
        let paths = paths.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || read(&paths, &identity))
                .await
                .ok()
                .flatten()
        })
    })
}

fn read(paths: &RuntimePaths, identity: &TranscriptSessionIdentity) -> Option<Vec<String>> {
    if identity.provider != "claude" {
        return None;
    }
    let transcript = transcript_path::resolve(paths, identity)?;
    let mut pending = tail(paths, &transcript)?;
    if let Some(dir) = subagent_activity::dir_for_transcript(&identity.provider, &transcript)
        && let Ok(found) = entries(paths, &dir)
    {
        let mut logs: Vec<(SystemTime, std::path::PathBuf)> = found
            .into_iter()
            .filter(|entry| !entry.is_dir && entry.name.ends_with(".jsonl"))
            .filter_map(|entry| {
                let path = dir.join(&entry.name);
                let modified = artifact_metadata(paths, &path).ok()?.modified().ok()?;
                Some((modified, path))
            })
            .collect();
        logs.sort_by(|left, right| right.0.cmp(&left.0));
        for (_, log) in logs.into_iter().take(SUBAGENT_LOGS) {
            pending.extend(tail(paths, &log).unwrap_or_default());
        }
    }
    Some(pending)
}

fn tail(paths: &RuntimePaths, path: &Path) -> Option<Vec<String>> {
    let mut file = open_artifact(paths, path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(TAIL_BYTES);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.take(TAIL_BYTES).read_to_end(&mut bytes).ok()?;
    Some(pending_claude_shell_commands(&bytes, start > 0))
}
