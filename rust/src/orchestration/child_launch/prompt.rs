//! Fixed-Go orchestration.go child prompt rendering. Delivery is selected once.
use super::safe_token;
use crate::orchestration::headless::SESSION_ID_PLACEHOLDER;
use std::path::{Path, PathBuf};

pub fn sanitize_inject_text(value: &str) -> String {
    value
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .chars()
        .filter(|c| !(*c < ' ' && *c != '\t' && *c != '\n') && *c != '\u{7f}')
        .collect()
}

pub fn normalize_role(role: &str) -> Result<String, &'static str> {
    if role.trim().is_empty() {
        return Err("role is required");
    }
    // Keep Go's surprising safeToken fallback: punctuation-only roles become
    // "item", rather than being rejected after sanitization.
    Ok(
        safe_token(&crate::proto::unicode::simple_lower(role.trim()))
            .trim_matches(['-', '_', '.'])
            .to_owned(),
    )
}

pub fn child_progress_path(board: &Path, id: &str) -> PathBuf {
    board
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join(format!("child-{id}.md"))
}

pub fn child_initial_prompt(
    base: &str,
    board: &Path,
    role: &str,
    branch: &str,
    id: &str,
) -> String {
    let mut out = format!(
        "You are an orchestration child session.\nRole: {role}\nSession ID: {id}\nShared board (read-only for you): {}\nYour progress file (write here): {}\n",
        board.display(),
        child_progress_path(board, id).display(),
    );
    if !branch.is_empty() {
        out.push_str(&format!("Worktree branch: {branch}\n"));
    }
    out.push_str(&format!("Read the board before acting; the conductor posts instructions there. Write your progress ONLY to your progress file (create it on first write), as `## {role} session={id} <RFC3339 time>` sections, each including a `status: running|blocked|done|failed` line. If you need an answer before continuing, include `status: blocked` and append `## QUESTION {role} session={id}`; the conductor will answer with orchestrate send. When complete, append `## DONE {role} session={id}` (or the explicit success form `## SUCCESS {role} session={id}`) and a concise summary to your progress file. Do not write to the shared board.\n\n"));
    out.push_str(&sanitize_inject_text(base));
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromptDelivery {
    /// The wrapper substitutes the real ID after registration.
    AtLaunch(String),
    /// The caller must enqueue owned initial injection after registration.
    AfterRegistration,
}

pub fn child_launch_prompt(
    execution_mode: &str,
    via_arg: bool,
    base: &str,
    board: &Path,
    role: &str,
    branch: &str,
) -> PromptDelivery {
    if !crate::config::is_headless_execution_mode(execution_mode) && !via_arg {
        PromptDelivery::AfterRegistration
    } else {
        PromptDelivery::AtLaunch(child_initial_prompt(
            base,
            board,
            role,
            branch,
            SESSION_ID_PLACEHOLDER,
        ))
    }
}
