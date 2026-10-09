//! Metadata-only provider file discovery, using each registered home and cwd.
//! No process-global home fallback or transcript-body persistence.
use crate::application::session_observations::subagents::{
    artifact_metadata, entries, open_artifact,
};
use crate::{
    config::RuntimePaths,
    proto::{
        core::*,
        time::{parse_rfc3339, utc},
    },
};
use chrono::{Datelike, Local};
use std::{
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    time::Duration,
};

pub(super) fn check_trial_path(paths: &RuntimePaths, path: &Path) -> Result<(), SessionError> {
    open_artifact(paths, path)
        .map(|_| ())
        .map_err(|_| SessionError::InvalidRequest("trial file unavailable".into()))
}
fn home(root: &str, home: &str, suffix: &str) -> Option<PathBuf> {
    if !root.trim().is_empty() {
        Some(root.trim().into())
    } else if !home.trim().is_empty() {
        Some(Path::new(home).join(suffix))
    } else {
        None
    }
}
fn metadata(paths: &RuntimePaths, path: &Path, lines: usize) -> Option<serde_json::Value> {
    let reader = BufReader::new(open_artifact(paths, path).ok()?.take(4 * 1024 * 1024));
    for line in reader.lines().take(lines) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line.ok()?) else {
            continue;
        };
        let payload = if value["type"] == "session_meta" {
            &value["payload"]
        } else {
            &value
        };
        if payload["cwd"].as_str().is_some_and(|v| !v.is_empty())
            && payload["timestamp"].as_str().is_some_and(|v| !v.is_empty())
        {
            return Some(payload.clone());
        }
    }
    None
}
fn same_path(left: &str, right: &str) -> bool {
    let clean = |s: &str| s.replace('\\', "/").trim_end_matches('/').to_owned();
    if cfg!(windows) {
        clean(left).eq_ignore_ascii_case(&clean(right))
    } else {
        clean(left) == clean(right)
    }
}
pub(super) fn resolve(
    paths: &RuntimePaths,
    identity: &TranscriptSessionIdentity,
) -> Option<PathBuf> {
    crate::application::agent_history::resolve_transcript(paths, identity)
}
pub(super) fn thread_switch(
    paths: &RuntimePaths,
    identity: &TranscriptSessionIdentity,
    current: &Path,
    peers: &[TranscriptSessionIdentity],
    now: crate::proto::time::Timestamp,
) -> (Option<PathBuf>, bool) {
    let pick = || -> Option<(Option<PathBuf>, bool)> {
        let root = home(&identity.codex_home, &identity.home_dir, ".codex")?;
        let meta = metadata(paths, current, 1)?;
        let start = parse_rfc3339(meta["timestamp"].as_str()?).ok()?;
        let modified = crate::proto::time::Timestamp::from_system_time(
            artifact_metadata(paths, current).ok()?.modified().ok()?,
        )
        .ok()?;
        let peers: Vec<_> = peers
            .iter()
            .filter(|peer| {
                same_path(&peer.cwd, &identity.cwd)
                    && home(&peer.codex_home, &peer.home_dir, ".codex").is_some_and(|peer_root| {
                        same_path(&peer_root.to_string_lossy(), &root.to_string_lossy())
                    })
            })
            .collect();
        let start_date = utc(start).ok()?.with_timezone(&Local).date_naive();
        let end_date = utc(now).ok()?.with_timezone(&Local).date_naive();
        let mut day = start_date.max(end_date - chrono::Duration::days(2));
        let mut best: Option<(crate::proto::time::Timestamp, PathBuf)> = None;
        while day <= end_date {
            let dir = root
                .join("sessions")
                .join(format!("{:04}", day.year()))
                .join(format!("{:02}", day.month()))
                .join(format!("{:02}", day.day()));
            if let Ok(entries) = entries(paths, &dir) {
                for entry in entries {
                    let path = dir.join(entry.name);
                    if path.extension().is_none_or(|ext| ext != "jsonl")
                        || same_path(&path.to_string_lossy(), &current.to_string_lossy())
                    {
                        continue;
                    }
                    let Some(candidate) = metadata(paths, &path, 1) else {
                        continue;
                    };
                    if !candidate["parent_thread_id"]
                        .as_str()
                        .unwrap_or("")
                        .is_empty()
                        || !matches!(
                            candidate["thread_source"].as_str().unwrap_or(""),
                            "" | "user"
                        )
                        || !same_path(candidate["cwd"].as_str().unwrap_or(""), &identity.cwd)
                    {
                        continue;
                    }
                    let Some(stamp) = candidate["timestamp"]
                        .as_str()
                        .and_then(|value| parse_rfc3339(value).ok())
                    else {
                        continue;
                    };
                    if stamp <= start || modified > stamp + Duration::from_secs(5) {
                        continue;
                    }
                    let Some(file_modified) = artifact_metadata(paths, &path)
                        .ok()
                        .and_then(|meta| meta.modified().ok())
                        .and_then(|at| crate::proto::time::Timestamp::from_system_time(at).ok())
                    else {
                        continue;
                    };
                    if file_modified < start {
                        continue;
                    }
                    let claimed = peers.iter().any(|peer| {
                        if !peer.native_log_path.is_empty() {
                            same_path(&peer.native_log_path, &path.to_string_lossy())
                        } else {
                            parse_rfc3339(&peer.started_at)
                                .ok()
                                .is_some_and(|peer_start| {
                                    stamp
                                        .duration_since(peer_start)
                                        .or_else(|_| peer_start.duration_since(stamp))
                                        .is_ok_and(|delta| delta <= Duration::from_secs(600))
                                })
                        }
                    });
                    if claimed {
                        continue;
                    }
                    if best
                        .as_ref()
                        .is_none_or(|(best_start, _)| stamp > *best_start)
                    {
                        best = Some((stamp, path));
                    }
                }
            }
            day = day.succ_opt()?;
        }
        if best.is_some() && !peers.is_empty() {
            Some((None, true))
        } else {
            Some((best.map(|(_, path)| path), false))
        }
    };
    pick().unwrap_or((None, false))
}
pub(super) fn turn_summary_prompt(ja: bool) -> &'static str {
    if ja {
        "[many-ai-cli] 直前の 1 ターンで何をしたかを 1 行で要約してください。前置きや説明は一切付けず、[MANY-AI-CLI-TURN-SUMMARY] の直後に要約本文（1 行）を書き、続けて [/MANY-AI-CLI-TURN-SUMMARY] を出力してください（すべて 1 行に収め、装飾を付けず、コードブロックで囲まないこと）。"
    } else {
        "[many-ai-cli] Summarize what you just did in this one turn, in a single line. Do not add any preamble: output [MANY-AI-CLI-TURN-SUMMARY] immediately followed by the one-line summary, then [/MANY-AI-CLI-TURN-SUMMARY] (keep it all on one line, undecorated, and not wrapped in a code block)."
    }
}
pub(super) fn conductor_prompt(id: &str) -> String {
    format!(
        "You are an orchestration conductor session (many-ai-cli).\nOrchestration ID: {id}\nNo child role mapping was configured. Decide provider/model yourself whenever a child is needed and run:\n  <MANY_AI_CLI_BIN> orchestrate spawn --role <role> --provider <provider> --model <model> \"<prompt>\"\nDo not call the Hub HTTP API or handle any auth token directly; this subcommand does it for you.\nThe Hub exposes its exact executable path in MANY_AI_CLI_BIN. Invoke that path, not a many-ai-cli resolved from PATH, for every orchestrate command.\nTo run a plan file through the implementation→review→fix relay without conducting it yourself, run: <MANY_AI_CLI_BIN> orchestrate relay --plan <path-to-plan.md> (roles come from the mapping above; pass --impl/--review provider[/model] if no mapping is configured; add --strong provider[/model] to hand a C to a stronger implementer when it keeps failing review; the relay works in its own git worktree unless you pass --same-tree). Relay children are driven by the Hub: do not spawn or send to them yourself, and you may close this session while a relay is running — it continues without you and you will be notified when it finishes.\nUse `<MANY_AI_CLI_BIN> orchestrate relay status [--id <orchestration-id>]` to inspect relay state and `<MANY_AI_CLI_BIN> orchestrate relay stop [--id <orchestration-id>]` to stop it.\nOperating rules:\n- To give follow-up instructions to an existing live child, run `<MANY_AI_CLI_BIN> orchestrate send --role <role> \"<text>\"` instead of spawning again. spawn is rejected (409) while a live child exists for the role; send injects the text into the child and records it on the board automatically.\n- When a child reports `## QUESTION <role> session=<id>`, read its progress file and answer with `<MANY_AI_CLI_BIN> orchestrate send --role <role> \"<answer>\"`.\n- When the user asks you to stop, confirm in one line whether they mean immediately or after the current work unit completes (default: after completion).\n- Do not dispatch a reviewer while the implementation child is still working on fixes; wait for its `## DONE` entry on the board first.\n"
    )
}
