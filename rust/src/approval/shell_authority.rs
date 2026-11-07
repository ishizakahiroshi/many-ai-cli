//! The whole shell command behind a heading-style approval prompt (audit U23).
//!
//! Claude draws a Bash permission as a `Bash command` heading, then the command
//! (wrapped, possibly over several lines) and its description in the same indent.
//! Terminal text alone cannot tell where the command ends, so
//! `summary::extract_command` keeps only the first line under the heading. Risk,
//! automatic rules, batch approval and one-tap approval must not be decided on that
//! line: the next line can hold anything.
//!
//! Claude writes the tool call, with the exact command, to its transcript before
//! it draws the prompt. A heading prompt is *verified* when exactly one pending
//! (still unanswered) shell call in the transcript is what the screen shows; the
//! whole command is then what is classified and matched. Otherwise it is
//! *unverified*: its risk is at least `mid`, and only a person choosing on the
//! prompt may approve it. Prompts without a heading keep their summary unchanged.
//!
//! The transcript is read by the caller outside the session lock (see
//! `EngineOptions::pending_shell`); this module only parses and compares.
use crate::{
    approval::summary::{classify_risk, command_heading, mask_secrets},
    proto::ApprovalSummary,
};
use serde_json::Value;

/// Bounds keep a corrupt or hostile transcript from growing memory.
const MAX_PENDING: usize = 16;
const MAX_COMMAND_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandAuthority {
    /// No command heading: the one-line extraction already is the whole command.
    NotHeading,
    /// Exactly one pending transcript call is what the prompt shows.
    Verified,
    /// A heading prompt whose whole command could not be confirmed.
    Unverified,
}

/// Cheap check on raw terminal output: whether a chunk may draw a command
/// heading, so the transcript is read again before the prompt is recorded.
pub fn mentions_command_heading(bytes: &[u8]) -> bool {
    [b"bash command".as_slice(), b"shell command".as_slice()]
        .iter()
        .any(|heading| {
            bytes
                .windows(heading.len())
                .any(|window| window.eq_ignore_ascii_case(heading))
        })
}

/// Text below the first command heading, read in the order `extract_command` uses.
pub fn heading_tail(question: &str, context: &str) -> Option<String> {
    let lines: Vec<&str> = std::iter::once(question)
        .chain(context.split('\n'))
        .collect();
    let at = lines.iter().position(|line| command_heading(line.trim()))?;
    Some(lines[at + 1..].join("\n"))
}

/// Unanswered Claude `Bash` tool calls in a transcript tail, oldest first.
/// `starts_mid_line` drops the first line, which a tail read may have cut.
pub fn pending_claude_shell_commands(tail: &[u8], starts_mid_line: bool) -> Vec<String> {
    let mut pending: Vec<(String, String)> = Vec::new();
    let mut lines = tail.split(|byte| *byte == b'\n');
    if starts_mid_line {
        lines.next();
    }
    for line in lines {
        if !contains(line, b"\"tool_use\"") && !contains(line, b"\"tool_result\"") {
            continue;
        }
        let Ok(record) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        let Some(blocks) = record.pointer("/message/content").and_then(Value::as_array) else {
            continue;
        };
        for block in blocks {
            match block.get("type").and_then(Value::as_str) {
                Some("tool_use") if block.get("name").and_then(Value::as_str) == Some("Bash") => {
                    let id = block.get("id").and_then(Value::as_str).unwrap_or_default();
                    let Some(command) = block.pointer("/input/command").and_then(Value::as_str)
                    else {
                        continue;
                    };
                    if id.is_empty() || command.len() > MAX_COMMAND_BYTES {
                        continue;
                    }
                    pending.retain(|(known, _)| known != id);
                    pending.push((id.to_owned(), command.to_owned()));
                    if pending.len() > MAX_PENDING {
                        pending.remove(0);
                    }
                }
                Some("tool_result") => {
                    if let Some(id) = block.get("tool_use_id").and_then(Value::as_str) {
                        pending.retain(|(known, _)| known != id);
                    }
                }
                _ => {}
            }
        }
    }
    pending.into_iter().map(|(_, command)| command).collect()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// The pending command the prompt shows. Whitespace is ignored on both sides so
/// terminal wrapping and indent do not matter; the description that follows the
/// command on screen is allowed. Two different matches are ambiguous: no match.
pub fn shown_command<'a>(after_heading: &str, pending: &'a [String]) -> Option<&'a str> {
    let screen = without_whitespace(after_heading);
    let mut found: Option<&'a str> = None;
    for command in pending {
        let wanted = without_whitespace(command);
        if wanted.is_empty() || !screen.starts_with(&wanted) {
            continue;
        }
        match found {
            Some(previous) if previous != command.as_str() => return None,
            _ => found = Some(command),
        }
    }
    found
}

fn without_whitespace(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Applies transcript evidence to a summary built from terminal text.
/// `pending` is `None` when the transcript could not be read.
pub fn authorize(
    summary: ApprovalSummary,
    question: &str,
    context: &str,
    pending: Option<&[String]>,
) -> (ApprovalSummary, CommandAuthority) {
    let Some(after) = heading_tail(question, context) else {
        return (summary, CommandAuthority::NotHeading);
    };
    if let Some(command) = pending.and_then(|pending| shown_command(&after, pending)) {
        let command = mask_secrets(&compact_lines(command));
        let risk = stricter(classify_risk(&command), &summary.risk).to_owned();
        return (
            ApprovalSummary {
                command,
                risk,
                ..summary
            },
            CommandAuthority::Verified,
        );
    }
    let risk = stricter("mid", &summary.risk).to_owned();
    (
        ApprovalSummary { risk, ..summary },
        CommandAuthority::Unverified,
    )
}

/// Keeps line breaks (each line is a separate command for `classify_risk`) and
/// collapses runs of blanks inside a line.
fn compact_lines(command: &str) -> String {
    command
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn rank(risk: &str) -> u8 {
    match risk {
        "low" => 0,
        "high" => 2,
        _ => 1,
    }
}

fn stricter<'a>(left: &'a str, right: &'a str) -> &'a str {
    if rank(right) > rank(left) {
        right
    } else {
        left
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approval::summary::summarize;

    const QUESTION: &str = "Do you want to proceed?";

    fn tool_use(id: &str, name: &str, command: &str) -> String {
        serde_json::json!({
            "type": "assistant",
            "message": {"role": "assistant", "content": [
                {"type": "tool_use", "id": id, "name": name,
                 "input": {"command": command, "description": "synthetic"}}
            ]}
        })
        .to_string()
    }
    fn tool_result(id: &str) -> String {
        serde_json::json!({
            "type": "user",
            "message": {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": id, "content": "ok"}
            ]}
        })
        .to_string()
    }

    #[test]
    fn transcript_keeps_only_unanswered_bash_calls() {
        let tail = [
            // A tail read may start inside a record; even a parseable first line
            // is dropped.
            tool_use("z", "Bash", "first line of a cut read"),
            tool_use("a", "Bash", "git status"),
            tool_result("a"),
            tool_use("b", "Read", "ignored"),
            "not json \"tool_use\"".to_owned(),
            tool_use("c", "Bash", "ls -la\nrm -rf ./x"),
        ]
        .join("\n");
        assert_eq!(
            pending_claude_shell_commands(tail.as_bytes(), true),
            vec!["ls -la\nrm -rf ./x".to_owned()]
        );
    }

    #[test]
    fn transcript_pending_list_is_bounded() {
        let tail = (0..40)
            .map(|i| tool_use(&format!("id{i}"), "Bash", &format!("echo {i}")))
            .collect::<Vec<_>>()
            .join("\n");
        let pending = pending_claude_shell_commands(tail.as_bytes(), false);
        assert_eq!(pending.len(), MAX_PENDING);
        assert_eq!(pending.last().map(String::as_str), Some("echo 39"));
    }

    #[test]
    fn second_line_of_a_multi_line_command_is_classified() {
        // Report U23 regression: the first line alone was classified as low.
        let context = "Bash command\n\n  ls -la\n  rm -rf ./x\n  desc";
        let base = summarize(QUESTION, context);
        assert_eq!(
            base.risk, "low",
            "fixture must reproduce the one-line extraction"
        );
        let pending = vec!["ls -la\nrm -rf ./x".to_owned()];
        let (summary, authority) = authorize(base.clone(), QUESTION, context, Some(&pending));
        assert_eq!(authority, CommandAuthority::Verified);
        assert_eq!(summary.risk, "high");
        assert_eq!(summary.command, "ls -la\nrm -rf ./x");
        let (summary, authority) = authorize(base, QUESTION, context, None);
        assert_eq!(authority, CommandAuthority::Unverified);
        assert_eq!(summary.risk, "mid");
    }

    #[test]
    fn single_line_command_stays_low_when_verified() {
        let context = "Bash command\n\n  ls -la\n  List files";
        let pending = vec!["ls -la".to_owned()];
        let (summary, authority) = authorize(
            summarize(QUESTION, context),
            QUESTION,
            context,
            Some(&pending),
        );
        assert_eq!(authority, CommandAuthority::Verified);
        assert_eq!(
            (summary.command.as_str(), summary.risk.as_str()),
            ("ls -la", "low")
        );
    }

    #[test]
    fn wrapped_command_matches_without_whitespace() {
        let full = "git log --oneline --graph --decorate --all";
        let context =
            "Bash command\n\n  git log --oneline --gra\n  ph --decorate --all\n  Show history";
        let pending = vec![full.to_owned()];
        assert_eq!(
            heading_tail(QUESTION, context)
                .as_deref()
                .and_then(|t| shown_command(t, &pending)),
            Some(full)
        );
    }

    #[test]
    fn mismatch_or_ambiguity_is_unverified() {
        let context = "Bash command\n\n  ls -la\n  List files";
        for pending in [
            vec!["pwd".to_owned()],
            vec!["ls".to_owned(), "ls -la".to_owned()],
            Vec::new(),
        ] {
            let (summary, authority) = authorize(
                summarize(QUESTION, context),
                QUESTION,
                context,
                Some(&pending),
            );
            assert_eq!(authority, CommandAuthority::Unverified, "{pending:?}");
            assert_ne!(summary.risk, "low");
        }
    }

    #[test]
    fn inline_prompts_are_unchanged() {
        let context = "Run: git status\nDo you want to proceed?";
        let base = summarize(QUESTION, context);
        let (summary, authority) = authorize(base.clone(), QUESTION, context, None);
        assert_eq!(authority, CommandAuthority::NotHeading);
        assert!(summary == base, "inline prompts keep their summary");
    }
}
