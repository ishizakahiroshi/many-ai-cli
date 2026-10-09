//! Pure fixed-Go relay progress parsing and worker instruction templates.
//!
//! The caller owns file inspection, delivery, and the relay state machine. The
//! existence callback must report true only for an existing non-directory file.
use crate::orchestration::child_launch::{prompt::sanitize_inject_text, safe_token};
use crate::proto::unicode::{simple_fold_key, simple_lower};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fmt,
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Verdict {
    pub kind: String,
    pub must: i64,
    pub should: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub file: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VerdictError {
    Missing,
    /// The partial verdict retains the path displayed by the source stop event.
    ReviewFileMissing(Verdict),
}
impl fmt::Display for VerdictError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Missing => "verdict line missing",
            Self::ReviewFileMissing(_) => "review file missing",
        })
    }
}
impl std::error::Error for VerdictError {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DoneCount {
    pub count: i64,
    pub final_seen: bool,
    pub escalate: bool,
}

fn sanitize_role(role: &str) -> String {
    safe_token(&simple_lower(role.trim()))
        .trim_matches(['-', '_', '.'])
        .to_owned()
}

/// Count every matching DONE/SUCCESS line, including repeated lines. Flags are
/// reset for each counted line; a foreign session cannot change the last flags.
pub fn count_done_lines(text: &str, role: &str, session_id: i64) -> DoneCount {
    let role = sanitize_role(role);
    let mut result = DoneCount::default();
    for line in text.split('\n').map(str::trim) {
        let Some(payload) = line
            .strip_prefix("## DONE ")
            .or_else(|| line.strip_prefix("## SUCCESS "))
        else {
            continue;
        };
        let fields: Vec<_> = payload.split_whitespace().collect();
        let Some(first) = fields.first() else {
            continue;
        };
        if sanitize_role(first) != role {
            continue;
        }
        // The first valid positive lowercase session= token wins. Invalid,
        // zero, negative, or differently cased tokens do not claim ownership.
        let owner = fields[1..]
            .iter()
            .find_map(|field| {
                field
                    .strip_prefix("session=")?
                    .parse::<i64>()
                    .ok()
                    .filter(|id| *id > 0)
            })
            .unwrap_or(0);
        if owner != 0 && owner != session_id {
            continue;
        }
        result.count += 1;
        result.final_seen = false;
        result.escalate = false;
        for field in &fields[1..] {
            let folded = simple_fold_key(field);
            if folded == "FINAL=TRUE" {
                result.final_seen = true;
            } else if folded == "ESCALATE=TRUE" {
                result.escalate = true;
            }
        }
    }
    result
}

/// Read the most recent verdict only when at least one verdict exists and its
/// count covers the caller's DONE baseline. Confinement is lexical, matching
/// filepath.Rel in the Go oracle; it intentionally does not resolve symlinks.
pub fn parse_verdict(
    text: &str,
    done_count: i64,
    board_dir: &Path,
    default_file: &Path,
    exists: impl Fn(&Path) -> bool,
) -> Result<Verdict, VerdictError> {
    let mut count = 0_i64;
    let mut raw = None;
    for line in text.split('\n').map(str::trim) {
        if line
            .as_bytes()
            .get(..8)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"verdict:"))
        {
            count += 1;
            raw = Some(line[8..].trim());
        }
    }
    let raw = raw
        .filter(|_| count >= done_count)
        .ok_or(VerdictError::Missing)?;
    let fields: Vec<_> = raw.split_whitespace().collect();
    let first = fields.first().ok_or(VerdictError::Missing)?;
    let mut verdict = Verdict {
        kind: simple_lower(first),
        ..Verdict::default()
    };
    match verdict.kind.as_str() {
        "pass" => {}
        "findings" => {
            let mut must_seen = false;
            let mut should_seen = false;
            for field in &fields[1..] {
                let field = simple_lower(field);
                if let Some(number) = field
                    .strip_prefix("must=")
                    .and_then(|n| n.parse::<i64>().ok())
                    .filter(|n| *n >= 0)
                {
                    verdict.must = number;
                    must_seen = true;
                } else if let Some(number) = field
                    .strip_prefix("should=")
                    .and_then(|n| n.parse::<i64>().ok())
                    .filter(|n| *n >= 0)
                {
                    verdict.should = number;
                    should_seen = true;
                }
            }
            if !must_seen && !should_seen {
                verdict.must = 1;
            }
        }
        "blocked" => {
            if let Some(marker) = lower_match(raw, "reason=") {
                verdict.reason = raw[marker.end..].trim().to_owned();
            }
            return Ok(verdict);
        }
        _ => return Err(VerdictError::Missing),
    }
    verdict.file = default_file.to_string_lossy().into_owned();
    if let Some(marker) = lower_match(raw, "file=") {
        let mut file = raw[marker.end..].trim().to_owned();
        // file= may precede the counts and its path may itself contain spaces.
        for stop in [" must=", " should="] {
            if let Some(marker) = lower_match(&file, stop) {
                file = file[..marker.start].trim().to_owned();
            }
        }
        let file = file.trim_matches(['`', '"', '\'']);
        if !file.is_empty() {
            let file = Path::new(file);
            let file = if file.is_absolute() {
                file.to_path_buf()
            } else {
                board_dir.join(file)
            };
            verdict.file = clean_path(&file).to_string_lossy().into_owned();
        }
    }
    if !path_inside(board_dir, Path::new(&verdict.file)) {
        return Err(VerdictError::Missing);
    }
    if verdict.kind == "findings" && verdict.must > 0 && !exists(Path::new(&verdict.file)) {
        return Err(VerdictError::ReviewFileMissing(verdict));
    }
    Ok(verdict)
}

// Simple lowercase maps one scalar to one scalar, but their UTF-8 byte widths
// may differ. Translate both marker boundaries before slicing the original text,
// including a length-changing character within a marker (for example FİLE=).
fn lower_match(value: &str, marker: &str) -> Option<std::ops::Range<usize>> {
    let lower = simple_lower(value);
    let start = lower.find(marker)?;
    let end = start + marker.len();
    let mut boundaries = lower
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(lower.len()))
        .zip(
            value
                .char_indices()
                .map(|(index, _)| index)
                .chain(std::iter::once(value.len())),
        );
    let (_, original_start) = boundaries.find(|(index, _)| *index == start)?;
    let (_, original_end) = boundaries.find(|(index, _)| *index == end)?;
    Some(original_start..original_end)
}

// Short IDs retain the fixed Go byte-tail behavior for a non-ASCII suffix.
fn byte_tail(value: &str, index: usize) -> String {
    String::from_utf8_lossy(value.as_bytes().get(index..).unwrap_or_default()).into_owned()
}

/// Lexical filepath.Clean, preserving leading relative parent components.
fn clean_path(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(result.components().next_back(), Some(Component::Normal(_))) {
                    result.pop();
                } else if !result.has_root() {
                    result.push("..");
                }
            }
            component => result.push(component.as_os_str()),
        }
    }
    if result.as_os_str().is_empty() {
        result.push(".");
    }
    result
}

pub fn path_inside(board_dir: &Path, path: &Path) -> bool {
    let board_dir = clean_path(board_dir);
    let path = clean_path(path);
    let mut path_components = path.components().filter(|c| *c != Component::CurDir);
    for board_component in board_dir.components().filter(|c| *c != Component::CurDir) {
        let Some(path_component) = path_components.next() else {
            return false;
        };
        #[cfg(windows)]
        let equal = simple_fold_key(&board_component.as_os_str().to_string_lossy())
            == simple_fold_key(&path_component.as_os_str().to_string_lossy());
        #[cfg(not(windows))]
        let equal = board_component == path_component;
        if !equal {
            return false;
        }
    }
    // A relative path below "." must not begin outside that directory; an
    // absolute target also cannot be made relative to a relative board path.
    !path_components.any(|c| {
        matches!(
            c,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    })
}

/// The display tag shortens only the nanosecond tail; stable keys stay intact.
pub fn short_id(orchestration_id: &str) -> String {
    let Some(index) = orchestration_id.rfind('-') else {
        return orchestration_id.to_owned();
    };
    let tail = &orchestration_id[index + 1..];
    if tail.len() <= 6 {
        return orchestration_id.to_owned();
    }
    format!(
        "{}{}",
        &orchestration_id[..index + 1],
        byte_tail(tail, tail.len() - 6)
    )
}

pub fn review_file(board_dir: &Path, c: i64, round: i64, strong: bool) -> PathBuf {
    let name = if strong {
        format!("review-c{c}-strong-r{round}.md")
    } else {
        format!("review-c{c}-r{round}.md")
    };
    clean_path(&board_dir.join(name))
}

fn relay_join(parts: &[&str]) -> String {
    parts
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// An owned rendering snapshot. The relay owner computes current_c (terminal:
/// completed_cs; otherwise completed_cs + 1) while holding its own state lock.
#[derive(Clone, Debug, Default)]
pub struct PromptContext {
    pub orchestration_id: String,
    pub plan_path: String,
    pub mode: String,
    pub board_dir: String,
    pub worktree_path: String,
    pub branch: String,
    pub base_commit: String,
    pub last_reviewed_commit: String,
    pub active_impl: String,
    pub review_path: String,
    pub current_c: i64,
    pub completed_cs: i64,
    pub round: i64,
    pub has_strong_role: bool,
    pub extra: BTreeMap<String, String>,
    pub last_verdict: Option<Verdict>,
}

impl PromptContext {
    pub fn tag(&self) -> String {
        let path = Path::new(&self.plan_path);
        let base = if self.plan_path.is_empty() {
            ".".into()
        } else if path.has_root() && path.parent().is_none() {
            std::path::MAIN_SEPARATOR.to_string()
        } else {
            path.components()
                .next_back()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .unwrap_or_else(|| ".".into())
        };
        format!("[relay {} {base}]", short_id(&self.orchestration_id))
    }

    pub fn review_file(&self, round: i64) -> String {
        review_file(
            Path::new(&self.board_dir),
            self.current_c,
            round,
            self.active_impl == "implementation-strong",
        )
        .to_string_lossy()
        .into_owned()
    }

    pub fn commit_line(&self) -> &'static str {
        if self.mode != "worktree" {
            return "";
        }
        "commit everything for this C on the current branch with `git add -A && git commit -m \"relay: C<n> <short summary>\"` (a fix round is `relay: C<n> fix r<r>`; never switch branches, never push), then"
    }

    pub fn scope_line(&self) -> String {
        if self.mode == "worktree" && !self.last_reviewed_commit.is_empty() {
            let from = &self.last_reviewed_commit;
            return format!(
                "the commits {from}..HEAD — run `git log --oneline {from}..HEAD`, `git diff {from}..HEAD`, and `git status --short` for anything left uncommitted"
            );
        }
        "run `git status --short` and `git diff` in the working directory; changes that pre-date the relay are also visible, judge them against the plan".into()
    }

    pub fn worktree_line(&self) -> String {
        if self.mode != "worktree" || self.worktree_path.is_empty() {
            return String::new();
        }
        format!(
            "Your working directory is the relay worktree {} on branch {} (forked from {}); stay on that branch.",
            self.worktree_path, self.branch, self.base_commit
        )
    }

    pub fn resync_line(&self) -> String {
        let log_cmd = if self.last_reviewed_commit.is_empty() {
            "git log --oneline -20".into()
        } else {
            format!("git log --oneline {}..HEAD", self.last_reviewed_commit)
        };
        format!(
            "The working tree may have been changed by another worker. Before you continue, run `{log_cmd}` and `git status --short` to see the current state, and trust the working tree over your own memory."
        )
    }

    fn finish(&self, text: String, role: &str) -> String {
        let extra = self.extra.get(role).map(|v| v.trim()).unwrap_or_default();
        if extra.is_empty() {
            sanitize_inject_text(&text)
        } else {
            sanitize_inject_text(&format!(
                "{text}\n\nAdditional instructions from the user:\n{extra}"
            ))
        }
    }

    pub fn implementation_prompt(&self) -> String {
        let strong_rule = if self.has_strong_role {
            "If a C's row in the plan's context table is marked [strong], do not implement it: append `## DONE implementation escalate=true` with the C number to your progress file and wait (add `final=true` as well if it is the last C). When the relay tells you another worker has taken over a C, wait until you are told to continue."
        } else {
            "Ignore any [strong] marks in the plan and implement every C yourself."
        };
        let text = relay_join(&[
            &self.tag(),
            &format!("Relay task: implement the plan at {}.", self.plan_path),
            &self.worktree_line(),
            "Read the repository instructions (CLAUDE.md / AGENTS.md if present) and the whole plan before changing anything.",
            "Work through the plan's C entries in order, one at a time. After finishing EACH C:",
            self.commit_line(),
            "append a summary of changed files and verification results, then `## DONE implementation`, to your progress file — and then WAIT for the next relay instruction; do not start the next C on your own.",
            "On the LAST C of the plan write `## DONE implementation final=true` instead.",
            strong_rule,
            "Do not push, and do not touch branches other than the current one.",
        ]);
        self.finish(text, "implementation")
    }

    pub fn strong_prompt(&self, kind: &str, verdict: &Verdict) -> String {
        let text = relay_join(&[
            &self.tag(),
            &format!(
                "Relay escalation: you are the stronger implementer for the plan at {}.",
                self.plan_path
            ),
            &self.worktree_line(),
            "Read the repository instructions (CLAUDE.md / AGENTS.md if present) and the plan.",
            &format!(
                "Your job now is C {} only: {}.",
                self.current_c,
                strong_kind_line(kind, verdict)
            ),
            &self.resync_line(),
            self.commit_line(),
            "When finished, append a summary and `## DONE implementation-strong` to your progress file (add `final=true` if this is the last C of the plan), then wait; the next C goes back to the other implementer, so do nothing until the relay calls you again.",
            "Do not push, and do not touch branches other than the current one.",
        ]);
        self.finish(text, "implementation-strong")
    }

    pub fn strong_text(&self, kind: &str, verdict: &Verdict) -> String {
        let text = relay_join(&[
            &self.tag(),
            &format!(
                "Relay escalation: C {}: {}.",
                self.current_c,
                strong_kind_line(kind, verdict)
            ),
            &self.resync_line(),
            self.commit_line(),
            "Then append `## DONE implementation-strong` (add `final=true` if this is the last C of the plan) and wait.",
        ]);
        self.finish(text, "implementation-strong")
    }

    pub fn wait_text(&self) -> String {
        sanitize_inject_text(&relay_join(&[
            &self.tag(),
            "Relay: another worker has taken over the current C. Do nothing until the relay tells you to continue.",
        ]))
    }

    pub fn self_implement_text(&self) -> String {
        let text = relay_join(&[
            &self.tag(),
            &format!(
                "Relay: no stronger implementer is available for C {}, so implement it yourself now. Same rules:",
                self.current_c
            ),
            self.commit_line(),
            "append `## DONE implementation` when the C is finished (`final=true` on the last C) and wait.",
        ]);
        self.finish(text, "implementation")
    }

    pub fn review_prompt(&self) -> String {
        let text = relay_join(&[
            &self.tag(),
            &format!(
                "Relay review (C {}, round {}): adversarially review the changes for the C that was just finished against the plan at {}.",
                self.current_c, self.round, self.plan_path
            ),
            &self.worktree_line(),
            &format!("Scope: {}.", self.scope_line()),
            "Assume the implementation is wrong until proven otherwise; inspect deleted lines as carefully as added lines. Do not modify any file.",
            &format!(
                "Write your findings to {} as two numbered lists, \"must\" (blocks acceptance: broken behaviour, safety, deviation from the plan) and \"should\" (worth fixing, not blocking); each item needs file:line, what is wrong, and how to verify the fix.",
                self.review_file(self.round)
            ),
            "If there are no findings write the single line `verdict: pass`.",
            "Then, in your progress file, write ONE line: `verdict: pass` or `verdict: findings must=<n> should=<m> file=<that path>` or `verdict: blocked reason=<why a human must decide>`, followed by `## DONE review`.",
        ]);
        self.finish(text, "review")
    }

    pub fn fix_text(&self, verdict: &Verdict) -> String {
        let text = relay_join(&[
            &self.tag(),
            &format!(
                "Relay fix (C {}, round {}): the review found {} must-fix and {} should-fix items in {}.",
                self.current_c, self.round, verdict.must, verdict.should, verdict.file
            ),
            "Fix every must item. Fix should items when cheap; otherwise record the item number and the reason in your progress file. Do not touch unrelated code.",
            "When finished:",
            self.commit_line(),
            &format!(
                "append `## DONE {}` to your progress file and wait.",
                self.active_impl
            ),
        ]);
        self.finish(text, &self.active_impl)
    }

    pub fn rereview_text(&self) -> String {
        let previous = self
            .last_verdict
            .as_ref()
            .filter(|v| !v.file.is_empty())
            .map(|v| v.file.clone())
            .unwrap_or_else(|| self.review_file(self.round - 1));
        let text = relay_join(&[
            &self.tag(),
            &format!(
                "Relay review (C {}, round {}): re-review after the fixes for {}.",
                self.current_c, self.round, previous
            ),
            "Verify each earlier must item is resolved and look for regressions.",
            &format!("Scope: {}.", self.scope_line()),
            &format!(
                "Write {} the same way, then the `verdict:` line and `## DONE review` in your progress file.",
                self.review_file(self.round)
            ),
        ]);
        self.finish(text, "review")
    }

    pub fn proceed_text(&self) -> String {
        let text = relay_join(&[
            &self.tag(),
            &format!(
                "Relay: the review of C {} passed. Continue with the next C of the plan.",
                self.completed_cs
            ),
            &self.resync_line(),
            "Same rules:",
            self.commit_line(),
            "append `## DONE implementation` when the C is finished (`final=true` on the last C) and wait.",
        ]);
        self.finish(text, "implementation")
    }

    pub fn nudge_text(&self, role: &str) -> String {
        sanitize_inject_text(&relay_join(&[
            &self.tag(),
            &format!(
                "Relay: no update to your progress file has been seen for a while. If your current work unit is finished, append `## DONE {role}` now (review: write the `verdict:` line first). If you are still working, continue. This reminder is sent only once."
            ),
        ]))
    }

    pub fn resume_prompt(&self) -> String {
        let log_cmd = if self.base_commit.is_empty() {
            "git log --oneline -20".into()
        } else {
            format!("git log --oneline {}..HEAD", self.base_commit)
        };
        let pending_review = if self
            .last_verdict
            .as_ref()
            .is_some_and(|v| v.kind == "findings" && v.must > 0)
            && !self.review_path.is_empty()
        {
            format!("; first fix the must items in {}", self.review_path)
        } else {
            String::new()
        };
        let text = relay_join(&[
            &self.tag(),
            &format!(
                "Relay resume: the plan at {} was partly implemented before an interruption.",
                self.plan_path
            ),
            &self.worktree_line(),
            "Read the repository instructions (CLAUDE.md / AGENTS.md if present) and the whole plan.",
            &format!(
                "Run `{log_cmd}` to see which C entries are already committed{pending_review}."
            ),
            &self.resync_line(),
            "Continue from the next unfinished C with the same rules: after finishing EACH C:",
            self.commit_line(),
            "append a summary and `## DONE implementation` to your progress file (`final=true` on the last C) and wait for the next relay instruction.",
            "Do not push, and do not touch branches other than the current one.",
        ]);
        self.finish(text, "implementation")
    }

    /// One preamble keeps the actual instruction identical on both routes.
    pub fn headless_instruction(&self, text: &str) -> String {
        let preamble = relay_join(&[
            &self.tag(),
            "You are a relay worker started for this one instruction. You have no memory of earlier rounds of this relay, and you will exit when this instruction is finished — that exit is how the relay knows you are done.",
            &format!("The plan is at {}.", self.plan_path),
            &self.worktree_line(),
            "Read the repository instructions (CLAUDE.md / AGENTS.md if present) and the plan before changing anything.",
            &self.resync_line(),
            "Your instruction follows.",
        ]);
        format!("{}\n\n{text}", sanitize_inject_text(&preamble))
    }
}

fn strong_kind_line(kind: &str, verdict: &Verdict) -> String {
    if kind == "fix" {
        format!(
            "the review found {} must-fix items in {}; fix every must item and re-check the C against the plan",
            verdict.must, verdict.file
        )
    } else {
        "implement it from the plan".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn context() -> PromptContext {
        PromptContext {
            orchestration_id: "r17-1756275600123456789".into(),
            plan_path: "plan.md".into(),
            mode: "same-tree".into(),
            board_dir: "board".into(),
            active_impl: "implementation".into(),
            current_c: 3,
            completed_cs: 2,
            round: 2,
            ..PromptContext::default()
        }
    }

    fn board_file(name: &str) -> PathBuf {
        clean_path(&Path::new("board").join(name))
    }

    #[test]
    fn done_count_repeats_owner_and_last_flags_match_source() {
        let text = "## implementation session=10 2026-08-27T15:00:00Z\nstatus: running\n\
                    ## DONE implementation session=10 final=true escalate=true\n\
                    ## DONE review session=11\n## SUCCESS implementation\n\
                    ## DONE implementation session=12 escalate=true\n\
                    ## DONE implementation session=10 final=true\n";
        assert_eq!(
            count_done_lines(text, "implementation", 10),
            DoneCount {
                count: 3,
                final_seen: true,
                escalate: false
            }
        );
        assert_eq!(
            count_done_lines(text, "review", 11),
            DoneCount {
                count: 1,
                ..DoneCount::default()
            }
        );
        assert_eq!(count_done_lines(text, "tester", 10), DoneCount::default());
        assert_eq!(
            count_done_lines(
                "## DONE implementation final=true\n## DONE implementation\n",
                "implementation",
                10
            ),
            DoneCount {
                count: 2,
                ..DoneCount::default()
            }
        );
        assert_eq!(
            count_done_lines(
                "## DONE implementation-strong session=12 ESCALATE=TRUE FINAL=TRUE",
                "implementation-strong",
                12
            ),
            DoneCount {
                count: 1,
                final_seen: true,
                escalate: true
            }
        );
    }

    #[test]
    fn done_sanitizes_roles_but_requires_exact_heading_and_valid_session_token() {
        for text in [
            "## DONE IMPLEMENTATION session=bad session=0 session=-3 session=+7",
            "## DONE --implementation._ session=7 session=8",
            "## DONE implementation SESSION=99 session=9223372036854775808",
            " \t## SUCCESS implementation session=7 FINAL=TRUE eſcalate=true \r",
        ] {
            assert_eq!(
                count_done_lines(text, " IMPLEMENTATION ", 7).count,
                1,
                "{text}"
            );
        }
        for text in [
            "## done implementation",
            "## DONE\timplementation",
            "# DONE implementation",
            "## DONE implementation-other",
            "## DONE implementation session=8 session=7",
        ] {
            assert_eq!(
                count_done_lines(text, "implementation", 7).count,
                0,
                "{text}"
            );
        }
        assert_eq!(count_done_lines("## DONE !!!\n", "", 7).count, 1);
        assert!(
            count_done_lines("## DONE implementation eſcalate=true", "implementation", 7).escalate
        );
    }

    #[test]
    fn verdict_counts_and_positive_must_defaults_match_source() {
        let default = board_file("review-c3-r2.md");
        let cases = [
            ("verdict: pass", "pass", 0, 0),
            (" VERDICT: FINDINGS MUST=+2 SHOULD=1\r", "findings", 2, 1),
            ("verdict: findings", "findings", 1, 0),
            ("verdict: findings must=-1 should=bad", "findings", 1, 0),
            (
                "verdict: findings must=9223372036854775808",
                "findings",
                1,
                0,
            ),
            ("verdict: findings should=0", "findings", 0, 0),
            ("verdict: findings must=-2 should=3", "findings", 0, 3),
            (
                "verdict: findings must=3 must=bad should=2 should=-1",
                "findings",
                3,
                2,
            ),
            (
                "verdict: findings must=3 must=0 should=2 should=7",
                "findings",
                0,
                7,
            ),
        ];
        for (text, kind, must, should) in cases {
            let calls = Cell::new(0);
            let got = parse_verdict(text, 1, Path::new("board"), &default, |p| {
                calls.set(calls.get() + 1);
                p == default
            })
            .unwrap();
            assert_eq!(
                got,
                Verdict {
                    kind: kind.into(),
                    must,
                    should,
                    file: default.to_string_lossy().into_owned(),
                    reason: String::new()
                },
                "{text}"
            );
            assert_eq!(
                calls.get(),
                i32::from(kind == "findings" && must > 0),
                "{text}"
            );
        }
        assert_eq!(
            parse_verdict(
                "verdict: pass\n## DONE review\n## DONE review",
                2,
                Path::new("board"),
                &default,
                |_| true
            ),
            Err(VerdictError::Missing)
        );
        for text in [
            "",
            "verdict:",
            "verdict: unknown",
            "verdict : pass",
            "# verdict: pass",
            "verdict: pass\nverdict:",
        ] {
            assert_eq!(
                parse_verdict(text, 1, Path::new("board"), &default, |_| true),
                Err(VerdictError::Missing),
                "{text}"
            );
        }
        let got = parse_verdict(
            "verdict: findings\n## DONE review\nverdict: pass\n## DONE review",
            2,
            Path::new("board"),
            &default,
            |_| panic!("pass does not stat"),
        )
        .unwrap();
        assert_eq!(got.kind, "pass");
        assert_eq!(
            parse_verdict("verdict: pass", 0, Path::new("board"), &default, |_| false)
                .unwrap()
                .kind,
            "pass"
        );
    }

    #[test]
    fn verdict_paths_quotes_spaces_order_and_missing_file_are_preserved() {
        let default = board_file("review-c3-r2.md");
        let expected = board_file("nested/review notes.md");
        for text in [
            "verdict: findings must=2 should=1 file=`nested/review notes.md`",
            "verdict: findings file=\"nested/review notes.md\" MUST=2 SHOULD=1",
            "verdict: findings file='nested/../nested/review notes.md' should=1 must=2",
        ] {
            let got =
                parse_verdict(text, 1, Path::new("board"), &default, |p| p == expected).unwrap();
            assert_eq!(
                (got.must, got.should, got.file),
                (2, 1, expected.to_string_lossy().into_owned())
            );
        }
        let expected = Verdict {
            kind: "findings".into(),
            must: 1,
            file: default.to_string_lossy().into_owned(),
            ..Verdict::default()
        };
        assert_eq!(
            parse_verdict("verdict: findings", 1, Path::new("board"), &default, |_| {
                false
            }),
            Err(VerdictError::ReviewFileMissing(expected))
        );
        for file in ["../outside.md", "nested/../../outside.md"] {
            assert_eq!(
                parse_verdict(
                    &format!("verdict: pass file={file}"),
                    1,
                    Path::new("board"),
                    &default,
                    |_| panic!("outside does not stat")
                ),
                Err(VerdictError::Missing)
            );
        }
        let got = parse_verdict(
            "verdict: pass file=``",
            1,
            Path::new("board"),
            &default,
            |_| false,
        )
        .unwrap();
        assert_eq!(got.file, default.to_string_lossy());
        assert_eq!(
            parse_verdict(
                "verdict: pass",
                1,
                Path::new("board"),
                Path::new("outside.md"),
                |_| false
            ),
            Err(VerdictError::Missing)
        );
    }

    #[test]
    fn verdict_markers_preserve_original_offsets_after_unicode_lowercase() {
        let board = Path::new("board");
        let default = board.join("review.md");
        let long_name = format!("{}.md", "Ⱥ".repeat(16));
        for (text, file) in [
            (
                "verdict: pass Ⱥ FILE=review.md".to_owned(),
                "review.md".to_owned(),
            ),
            (
                "verdict: pass K FİLE=レビュー.md".to_owned(),
                "レビュー.md".to_owned(),
            ),
            (
                "verdict: findings FILE=Ⱥ.md MUST=0 SHOULD=2".to_owned(),
                "Ⱥ.md".to_owned(),
            ),
            (
                "verdict: findings FILE=İ.md SHOULD=2 MUST=0".to_owned(),
                "İ.md".to_owned(),
            ),
            (
                format!("verdict: findings FILE={long_name} MUST=0"),
                long_name,
            ),
        ] {
            let got = parse_verdict(&text, 1, board, &default, |_| true).unwrap();
            assert_eq!(Path::new(&got.file), board.join(file), "{text}");
        }
        for prefix in ["Ⱥ", "K", "İ"] {
            let got = parse_verdict(
                &format!("verdict: blocked {prefix} REASON=理由 Ⱥ file=keep this"),
                1,
                board,
                &default,
                |_| panic!("blocked does not stat"),
            )
            .unwrap();
            assert_eq!(got.reason, "理由 Ⱥ file=keep this");
        }
    }

    #[test]
    fn blocked_bypasses_file_checks_and_keeps_reason_to_end_of_line() {
        let got = parse_verdict(
            "VerDict: BlOcKeD REASON= needs decision on API shape file=../outside.md  ",
            1,
            Path::new("board"),
            Path::new("outside.md"),
            |_| panic!("blocked does not stat"),
        )
        .unwrap();
        assert_eq!(
            got,
            Verdict {
                kind: "blocked".into(),
                reason: "needs decision on API shape file=../outside.md".into(),
                ..Verdict::default()
            }
        );
        assert_eq!(
            parse_verdict(
                "verdict: blocked",
                1,
                Path::new("board"),
                Path::new("outside.md"),
                |_| false
            )
            .unwrap()
            .reason,
            ""
        );
    }

    #[test]
    fn lexical_confinement_keeps_parent_and_component_boundaries() {
        for (board, path, expected) in [
            ("board", "board", true),
            ("board", "board/x.md", true),
            ("board", "board/sub/../x.md", true),
            ("board", "board-other/x.md", false),
            ("board", "board/../x.md", false),
            (".", "x.md", true),
            (".", "../x.md", false),
            ("..", "../x.md", true),
            ("../board", "../board/x.md", true),
            ("../board", "../x.md", false),
            ("", "", true),
            (".", "x/../../y", false),
        ] {
            assert_eq!(
                path_inside(Path::new(board), Path::new(path)),
                expected,
                "{board} -> {path}"
            );
        }
        #[cfg(unix)]
        for (board, path, expected) in [
            ("/", "/x", true),
            (".", "/x", false),
            ("/board", "board/x", false),
            ("/board", "/board2/x", false),
            ("/board", "/board/../board/x", true),
        ] {
            assert_eq!(
                path_inside(Path::new(board), Path::new(path)),
                expected,
                "{board} -> {path}"
            );
        }
        #[cfg(windows)]
        for (board, path, expected) in [
            (r"C:\board", r"c:\BOARD\x", true),
            (r"C:\board", r"D:\board\x", false),
            (r"C:\board", r"C:\board2\x", false),
        ] {
            assert_eq!(
                path_inside(Path::new(board), Path::new(path)),
                expected,
                "{board} -> {path}"
            );
        }
    }

    #[test]
    fn verdict_serialization_preserves_go_field_names_and_omissions() {
        assert_eq!(
            serde_json::to_value(Verdict {
                kind: "pass".into(),
                ..Verdict::default()
            })
            .unwrap(),
            serde_json::json!({"kind":"pass","must":0,"should":0})
        );
        assert_eq!(
            serde_json::from_str::<Verdict>(
                r#"{"kind":"findings","must":1,"file":"board/review.md"}"#
            )
            .unwrap()
            .should,
            0
        );
        assert_eq!(VerdictError::Missing.to_string(), "verdict line missing");
        assert_eq!(
            VerdictError::ReviewFileMissing(Verdict::default()).to_string(),
            "review file missing"
        );
    }

    #[test]
    fn tag_review_file_and_worktree_helpers_match_source() {
        for (id, expected) in [
            ("r1-1756275600123456789", "r1-456789"),
            ("r12-987654", "r12-987654"),
            ("r1-123", "r1-123"),
            ("plain", "plain"),
        ] {
            assert_eq!(short_id(id), expected);
        }
        let mut ctx = context();
        assert_eq!(ctx.tag(), "[relay r17-456789 plan.md]");
        assert_eq!(
            ctx.review_file(2),
            board_file("review-c3-r2.md").to_string_lossy()
        );
        ctx.active_impl = "implementation-strong".into();
        assert_eq!(
            ctx.review_file(2),
            board_file("review-c3-strong-r2.md").to_string_lossy()
        );
        assert_eq!(ctx.commit_line(), "");
        assert_eq!(ctx.worktree_line(), "");
        assert_eq!(
            ctx.scope_line(),
            "run `git status --short` and `git diff` in the working directory; changes that pre-date the relay are also visible, judge them against the plan"
        );
        ctx.mode = "worktree".into();
        ctx.worktree_path = "relay-tree".into();
        ctx.branch = "relay/topic".into();
        ctx.base_commit = "base012".into();
        ctx.last_reviewed_commit = "reviewed123".into();
        assert_eq!(
            ctx.commit_line(),
            "commit everything for this C on the current branch with `git add -A && git commit -m \"relay: C<n> <short summary>\"` (a fix round is `relay: C<n> fix r<r>`; never switch branches, never push), then"
        );
        assert_eq!(
            ctx.worktree_line(),
            "Your working directory is the relay worktree relay-tree on branch relay/topic (forked from base012); stay on that branch."
        );
        assert_eq!(
            ctx.scope_line(),
            "the commits reviewed123..HEAD — run `git log --oneline reviewed123..HEAD`, `git diff reviewed123..HEAD`, and `git status --short` for anything left uncommitted"
        );
        assert_eq!(
            ctx.resync_line(),
            "The working tree may have been changed by another worker. Before you continue, run `git log --oneline reviewed123..HEAD` and `git status --short` to see the current state, and trust the working tree over your own memory."
        );
    }

    #[test]
    fn implementation_prompt_full_snapshot_and_strong_rule() {
        let mut ctx = context();
        assert_eq!(
            ctx.implementation_prompt(),
            concat!(
                "[relay r17-456789 plan.md] Relay task: implement the plan at plan.md. ",
                "Read the repository instructions (CLAUDE.md / AGENTS.md if present) and the whole plan before changing anything. ",
                "Work through the plan's C entries in order, one at a time. After finishing EACH C: ",
                "append a summary of changed files and verification results, then `## DONE implementation`, to your progress file — and then WAIT for the next relay instruction; do not start the next C on your own. ",
                "On the LAST C of the plan write `## DONE implementation final=true` instead. ",
                "Ignore any [strong] marks in the plan and implement every C yourself. ",
                "Do not push, and do not touch branches other than the current one."
            )
        );
        let original = ctx.implementation_prompt();
        ctx.has_strong_role = true;
        assert_eq!(ctx.implementation_prompt(), original.replace(
            "Ignore any [strong] marks in the plan and implement every C yourself.",
            "If a C's row in the plan's context table is marked [strong], do not implement it: append `## DONE implementation escalate=true` with the C number to your progress file and wait (add `final=true` as well if it is the last C). When the relay tells you another worker has taken over a C, wait until you are told to continue."
        ));
    }

    #[test]
    fn review_and_rereview_full_snapshots() {
        let mut ctx = context();
        let file = board_file("review-c3-r2.md").to_string_lossy().into_owned();
        assert_eq!(
            ctx.review_prompt(),
            format!(
                concat!(
                    "[relay r17-456789 plan.md] Relay review (C 3, round 2): adversarially review the changes for the C that was just finished against the plan at plan.md. ",
                    "Scope: run `git status --short` and `git diff` in the working directory; changes that pre-date the relay are also visible, judge them against the plan. ",
                    "Assume the implementation is wrong until proven otherwise; inspect deleted lines as carefully as added lines. Do not modify any file. ",
                    "Write your findings to {} as two numbered lists, \"must\" (blocks acceptance: broken behaviour, safety, deviation from the plan) and \"should\" (worth fixing, not blocking); each item needs file:line, what is wrong, and how to verify the fix. ",
                    "If there are no findings write the single line `verdict: pass`. ",
                    "Then, in your progress file, write ONE line: `verdict: pass` or `verdict: findings must=<n> should=<m> file=<that path>` or `verdict: blocked reason=<why a human must decide>`, followed by `## DONE review`."
                ),
                file
            )
        );
        ctx.last_verdict = Some(Verdict {
            file: "board/custom review.md".into(),
            ..Verdict::default()
        });
        assert_eq!(
            ctx.rereview_text(),
            format!(
                concat!(
                    "[relay r17-456789 plan.md] Relay review (C 3, round 2): re-review after the fixes for board/custom review.md. ",
                    "Verify each earlier must item is resolved and look for regressions. ",
                    "Scope: run `git status --short` and `git diff` in the working directory; changes that pre-date the relay are also visible, judge them against the plan. ",
                    "Write {} the same way, then the `verdict:` line and `## DONE review` in your progress file."
                ),
                file
            )
        );
        ctx.last_verdict = None;
        assert!(
            ctx.rereview_text()
                .contains(&board_file("review-c3-r1.md").to_string_lossy().to_string())
        );
    }

    #[test]
    fn fix_wait_self_implementation_and_nudge_full_snapshots() {
        let mut ctx = context();
        ctx.active_impl = "implementation-strong".into();
        let verdict = Verdict {
            must: 2,
            should: 1,
            file: "board/custom.md".into(),
            ..Verdict::default()
        };
        assert_eq!(
            ctx.fix_text(&verdict),
            concat!(
                "[relay r17-456789 plan.md] Relay fix (C 3, round 2): the review found 2 must-fix and 1 should-fix items in board/custom.md. ",
                "Fix every must item. Fix should items when cheap; otherwise record the item number and the reason in your progress file. Do not touch unrelated code. ",
                "When finished: append `## DONE implementation-strong` to your progress file and wait."
            )
        );
        assert_eq!(
            ctx.wait_text(),
            "[relay r17-456789 plan.md] Relay: another worker has taken over the current C. Do nothing until the relay tells you to continue."
        );
        assert_eq!(
            ctx.self_implement_text(),
            "[relay r17-456789 plan.md] Relay: no stronger implementer is available for C 3, so implement it yourself now. Same rules: append `## DONE implementation` when the C is finished (`final=true` on the last C) and wait."
        );
        assert_eq!(
            ctx.nudge_text("review"),
            "[relay r17-456789 plan.md] Relay: no update to your progress file has been seen for a while. If your current work unit is finished, append `## DONE review` now (review: write the `verdict:` line first). If you are still working, continue. This reminder is sent only once."
        );
    }

    #[test]
    fn strong_and_proceed_full_snapshots() {
        let ctx = context();
        let resync = "The working tree may have been changed by another worker. Before you continue, run `git log --oneline -20` and `git status --short` to see the current state, and trust the working tree over your own memory.";
        let verdict = Verdict {
            must: 2,
            file: "board/custom.md".into(),
            ..Verdict::default()
        };
        assert_eq!(
            ctx.strong_prompt("implement", &verdict),
            format!(
                concat!(
                    "[relay r17-456789 plan.md] Relay escalation: you are the stronger implementer for the plan at plan.md. ",
                    "Read the repository instructions (CLAUDE.md / AGENTS.md if present) and the plan. ",
                    "Your job now is C 3 only: implement it from the plan. {} ",
                    "When finished, append a summary and `## DONE implementation-strong` to your progress file (add `final=true` if this is the last C of the plan), then wait; the next C goes back to the other implementer, so do nothing until the relay calls you again. ",
                    "Do not push, and do not touch branches other than the current one."
                ),
                resync
            )
        );
        assert_eq!(
            ctx.strong_text("fix", &verdict),
            format!(
                concat!(
                    "[relay r17-456789 plan.md] Relay escalation: C 3: the review found 2 must-fix items in board/custom.md; fix every must item and re-check the C against the plan. {} ",
                    "Then append `## DONE implementation-strong` (add `final=true` if this is the last C of the plan) and wait."
                ),
                resync
            )
        );
        assert_eq!(
            ctx.proceed_text(),
            format!(
                concat!(
                    "[relay r17-456789 plan.md] Relay: the review of C 2 passed. Continue with the next C of the plan. {} ",
                    "Same rules: append `## DONE implementation` when the C is finished (`final=true` on the last C) and wait."
                ),
                resync
            )
        );
    }

    #[test]
    fn resume_and_headless_full_snapshots() {
        let mut ctx = context();
        ctx.base_commit = "base012".into();
        ctx.review_path = "board/pending.md".into();
        ctx.last_verdict = Some(Verdict {
            kind: "findings".into(),
            must: 2,
            file: "board/other.md".into(),
            ..Verdict::default()
        });
        let resync = "The working tree may have been changed by another worker. Before you continue, run `git log --oneline -20` and `git status --short` to see the current state, and trust the working tree over your own memory.";
        assert_eq!(
            ctx.resume_prompt(),
            format!(
                concat!(
                    "[relay r17-456789 plan.md] Relay resume: the plan at plan.md was partly implemented before an interruption. ",
                    "Read the repository instructions (CLAUDE.md / AGENTS.md if present) and the whole plan. ",
                    "Run `git log --oneline base012..HEAD` to see which C entries are already committed; first fix the must items in board/pending.md. {} ",
                    "Continue from the next unfinished C with the same rules: after finishing EACH C: ",
                    "append a summary and `## DONE implementation` to your progress file (`final=true` on the last C) and wait for the next relay instruction. ",
                    "Do not push, and do not touch branches other than the current one."
                ),
                resync
            )
        );
        assert_eq!(
            ctx.headless_instruction("instruction\nkept intact"),
            format!(
                concat!(
                    "[relay r17-456789 plan.md] You are a relay worker started for this one instruction. You have no memory of earlier rounds of this relay, and you will exit when this instruction is finished — that exit is how the relay knows you are done. ",
                    "The plan is at plan.md. Read the repository instructions (CLAUDE.md / AGENTS.md if present) and the plan before changing anything. {} ",
                    "Your instruction follows.\n\ninstruction\nkept intact"
                ),
                resync
            )
        );
        ctx.base_commit.clear();
        ctx.last_verdict.as_mut().unwrap().must = 0;
        assert!(
            ctx.resume_prompt().contains(
                "Run `git log --oneline -20` to see which C entries are already committed."
            )
        );
        assert!(!ctx.resume_prompt().contains("first fix"));
    }

    #[test]
    fn extra_instructions_are_role_scoped_trimmed_and_sanitized_after_joining() {
        let mut ctx = context();
        ctx.extra.insert(
            "implementation".into(),
            " \r\nkeep\rgoing\u{1b}\u{7f}\tplease  ".into(),
        );
        ctx.extra
            .insert("implementation-strong".into(), "strong-only".into());
        ctx.extra.insert("review".into(), "review-only".into());
        let impl_suffix = "\n\nAdditional instructions from the user:\nkeep\ngoing\tplease";
        for text in [
            ctx.implementation_prompt(),
            ctx.self_implement_text(),
            ctx.proceed_text(),
            ctx.resume_prompt(),
            ctx.fix_text(&Verdict::default()),
        ] {
            assert!(text.ends_with(impl_suffix), "{text}");
        }
        for text in [
            ctx.strong_prompt("fix", &Verdict::default()),
            ctx.strong_text("implement", &Verdict::default()),
        ] {
            assert!(text.ends_with("\n\nAdditional instructions from the user:\nstrong-only"));
        }
        for text in [ctx.review_prompt(), ctx.rereview_text()] {
            assert!(text.ends_with("\n\nAdditional instructions from the user:\nreview-only"));
        }
        for text in [ctx.wait_text(), ctx.nudge_text("review")] {
            assert!(!text.contains("Additional instructions"));
        }
    }
}
