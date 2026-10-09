//! Native provider prompt detector, ported from approval_detector.go. The
//! caller supplies cached provider/common trigger phrases; detection performs no IO.
use super::{
    identity::candidate,
    summary::{mask_secrets, summarize},
};
use crate::proto::{ApprovalOption, ApprovalSummary, core::ApprovalSourceEpoch};
use regex::Regex;
use std::{collections::HashSet, sync::LazyLock};
#[derive(Clone, PartialEq)]
pub struct NativeApproval {
    pub sig: String,
    pub kind: String,
    pub question: String,
    pub context: String,
    pub options: Vec<ApprovalOption>,
    pub summary: ApprovalSummary,
}
fn re(pattern: &str) -> Regex {
    Regex::new(
        &pattern
            .replace(r"\s", r"(?-u:\s)")
            .replace(r"\d", "[0-9]")
            .replace(r"\b", r"(?-u:\b)"),
    )
    .expect("baseline regex")
}
static NUMBERED: LazyLock<Regex> = LazyLock::new(|| re(r"^\s*([>❯›❱])?\s*(\d{1,2})\.\s*(.+?)\s*$"));
static SHORTCUT: LazyLock<Regex> =
    LazyLock::new(|| re(r"^\s*([>❯›❱])?\s*(.+?)\s+\((y|p|n|!|#|\?|esc|escape)\)\s*$"));
static CURSOR: LazyLock<Regex> =
    LazyLock::new(|| re(r"^\s*([-*•>❯›❱])?\s*(.+?)\s+\(([^()]+)\)\s*$"));
static GROK: LazyLock<Regex> = LazyLock::new(|| re(r"^\s*(\d{1,2})\s+\(([•●○*\-])\)\s+(.+?)\s*$"));
static FEEDBACK: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)(\d{1,2})\s+to\s+(review|send|dismiss)\b"));
static SUBMIT: LazyLock<Regex> = LazyLock::new(|| re(r"\bSubmit\b"));
fn clean_label(label: &str) -> String {
    label
        .trim()
        .trim_matches(['│', '┃'])
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
fn supports_shortcut(provider: &str) -> bool {
    matches!(provider, "codex" | "copilot" | "cursor-agent")
}
fn shortcut_num(provider: &str, key: &str) -> i64 {
    match key {
        "y" => 1,
        "p" if provider == "codex" => 2,
        "n" => {
            if provider == "copilot" || provider == "cursor-agent" {
                2
            } else {
                3
            }
        }
        "!" if provider != "codex" => 3,
        "#" if provider != "codex" => 4,
        "?" if provider != "codex" => 5,
        "esc" | "escape" => {
            if provider != "codex" {
                6
            } else {
                4
            }
        }
        _ => 0,
    }
}
pub fn shortcut_send(provider: &str, label: &str) -> String {
    if !supports_shortcut(provider) {
        return String::new();
    }
    let lower = label.trim().to_lowercase();
    for key in ["y", "p", "n", "!", "#", "?", "esc", "escape"] {
        if key == "p" && provider != "codex"
            || matches!(key, "!" | "#" | "?") && provider == "codex"
        {
            continue;
        }
        if lower == key || lower.ends_with(&format!("({key})")) {
            return if key == "esc" || key == "escape" {
                "\x1b".into()
            } else {
                key.into()
            };
        }
    }
    String::new()
}
pub fn parse_option(provider: &str, line: &str) -> Option<ApprovalOption> {
    let line = line.trim().trim_matches(['│', '┃']).trim();
    if line.is_empty() {
        return None;
    }
    if let Some(c) = NUMBERED.captures(line) {
        let num = c[2].parse().ok()?;
        let label = clean_label(&c[3]);
        if label.is_empty() || num > 20 {
            return None;
        }
        let send = shortcut_send(provider, &label);
        return Some(ApprovalOption {
            num,
            label,
            is_current: c.get(1).is_some_and(|v| !v.as_str().is_empty()),
            preserve_order: !send.is_empty(),
            send_text: send,
        });
    }
    if provider == "grok"
        && let Some(c) = GROK.captures(line)
    {
        let num = c[1].parse().ok()?;
        let label = clean_label(&c[3]);
        if label.is_empty() || num > 20 {
            return None;
        }
        return Some(ApprovalOption {
            num,
            label,
            is_current: &c[2] != "○",
            ..Default::default()
        });
    }
    if provider == "cursor-agent" {
        let c = CURSOR.captures(line)?;
        let key = c[3].trim();
        let (send, num) = match key.to_lowercase().as_str() {
            "y" => ("y", 1),
            "tab" => ("\t", 2),
            "shift+tab" | "shift + tab" => ("\x1b[Z", 3),
            "esc or n" | "n or esc" | "esc" | "escape" => ("\x1b", 4),
            "n" => ("n", 4),
            _ => return None,
        };
        let label = clean_label(&format!("{} ({key})", &c[2]));
        if label.is_empty() {
            return None;
        }
        return Some(ApprovalOption {
            num,
            label,
            is_current: c.get(1).is_some_and(|m| !m.as_str().is_empty()),
            send_text: send.into(),
            preserve_order: true,
        });
    }
    if supports_shortcut(provider)
        && let Some(c) = SHORTCUT.captures(line)
    {
        let key = c[3].to_lowercase();
        let label = clean_label(&format!("{} ({})", &c[2], &c[3]));
        let send = shortcut_send(provider, &key);
        if !label.is_empty() && !send.is_empty() {
            return Some(ApprovalOption {
                num: shortcut_num(provider, &key),
                label,
                is_current: c.get(1).is_some(),
                send_text: send,
                preserve_order: true,
            });
        }
    }
    None
}
fn feedback(line: &str) -> Option<Vec<ApprovalOption>> {
    let captures: Vec<_> = FEEDBACK.captures_iter(line).collect();
    if captures.len() != 3 {
        return None;
    }
    let (mut nums, mut actions) = (HashSet::new(), HashSet::new());
    let mut out = Vec::new();
    for c in captures {
        let n = c[1].parse::<i64>().ok()?;
        let action = c[2].to_lowercase();
        if n > 20 || !nums.insert(n) || !actions.insert(action.clone()) {
            return None;
        }
        let label = match action.as_str() {
            "review" => "Review",
            "send" => "Send",
            "dismiss" => "Dismiss",
            _ => return None,
        };
        out.push(ApprovalOption {
            num: n,
            label: label.into(),
            send_text: n.to_string(),
            preserve_order: true,
            ..Default::default()
        });
    }
    Some(out)
}
fn is_feedback(options: &[ApprovalOption]) -> bool {
    let mut seen = HashSet::new();
    options.len() == 3
        && options.iter().all(|o| {
            let label = match o.num {
                0 => "Dismiss",
                1 => "Review",
                2 => "Send",
                _ => return false,
            };
            o.label == label
                && o.send_text == o.num.to_string()
                && o.preserve_order
                && seen.insert(o.num)
        })
}
pub fn is_multi_question(lines: &[String]) -> bool {
    lines.iter().any(|l| {
        let lower = l.to_lowercase();
        lower.contains("review your answers")
            || lower.contains("ready to submit your answers")
            || (l.contains('←')
                && l.contains('→')
                && (l.contains(['◻', '□', '☐', '✓', '☑']) || SUBMIT.is_match(l)))
    })
}
fn open_code_options(lines: &[String]) -> Option<(Vec<ApprovalOption>, usize, usize)> {
    for (i, line) in lines.iter().enumerate().rev() {
        let lower = line.to_lowercase();
        if lower.contains("confirm")
            && lower.contains("cancel")
            && let Some(start) = (i.saturating_sub(12)..=i)
                .rev()
                .find(|j| lines[*j].to_lowercase().contains("always allow"))
        {
            return Some((buttons(&["Confirm", "Cancel"]), start, i));
        }
        if lower.contains("allow once") {
            let start = (i.saturating_sub(12)..=i)
                .rev()
                .find(|j| lines[*j].to_lowercase().contains("permission required"))
                .unwrap_or(i.saturating_sub(12));
            return Some((buttons(&["Allow once", "Allow always", "Reject"]), start, i));
        }
    }
    None
}
fn buttons(labels: &[&str]) -> Vec<ApprovalOption> {
    labels
        .iter()
        .enumerate()
        .map(|(i, l)| ApprovalOption {
            num: i as i64 + 1,
            label: (*l).into(),
            send_text: format!("{}\r", "\x1b[C".repeat(i)),
            is_current: i == 0,
            preserve_order: true,
        })
        .collect()
}
fn options(provider: &str, lines: &[String]) -> Option<(Vec<ApprovalOption>, usize, usize)> {
    if provider == "opencode" {
        return open_code_options(lines);
    }
    let feedback = if provider == "claude" {
        lines
            .iter()
            .enumerate()
            .rev()
            .find_map(|(i, l)| feedback(l).map(|o| (o, i, i)))
    } else {
        None
    };
    let parsed: Vec<_> = lines
        .iter()
        .enumerate()
        .filter_map(|(i, l)| parse_option(provider, l).map(|o| (o, i)))
        .collect();
    if parsed.is_empty() {
        return feedback;
    }
    if feedback
        .as_ref()
        .is_some_and(|(_, i, _)| *i > parsed.last().unwrap().1)
    {
        return feedback;
    }
    let (mut best_start, mut best_end, mut cur) = (0, 0, 0);
    for i in 1..parsed.len() {
        if parsed[i].1 - parsed[i - 1].1 > 4 {
            if i - cur > best_end - best_start {
                best_start = cur;
                best_end = i - 1;
            }
            cur = i;
        }
    }
    if parsed.len() - cur > best_end - best_start {
        best_start = cur;
        best_end = parsed.len() - 1;
    }
    let cluster = &parsed[best_start..=best_end];
    if cluster.len() < 2 {
        return None;
    }
    let mut seen = HashSet::new();
    let options: Vec<_> = cluster
        .iter()
        .filter(|(o, _)| seen.insert((o.num, o.label.clone(), o.send_text.clone())))
        .map(|(o, _)| o.clone())
        .collect();
    if options.len() < 2
        || options.len() > 12
        || !options
            .iter()
            .any(|o| o.is_current || !o.send_text.is_empty())
    {
        return None;
    }
    Some((options, cluster[0].1, cluster.last().unwrap().1))
}
const WEB_HINTS: &[&str] = &[
    "requires approval",
    "would you like to run the following command",
    "would you like to run",
    "do you want to proceed?",
    "this command requires approval",
    "permission required",
    "permissions required",
    "requires permission",
    "requires confirmation",
    "prompts for user confirmation",
    "allow all similar",
    "deny all similar",
    "enter to select",
    "enter select",
    "↑/↓ to navigate",
    "do you trust the files in this folder",
    "tool permission",
    "command code needs to run",
    "do you want to make this edit",
    "esc to cancel",
    "tab:next option",
    "always-approve mode",
    "type to add feedback",
    "ctrl+o:always-approve",
];
fn model_hint(provider: &str, lower: &str) -> bool {
    let hints: &[&str] = match provider {
        "codex" => &[
            "select model",
            "select effort",
            "model and effort",
            "reasoning effort",
            "esc to go back",
            "↑/↓ to change",
            "arrow keys",
        ],
        "opencode" => &[
            "select model",
            "connect provider",
            "favorite ctrl+f",
            "opencode zen",
            "ollama (local)",
        ],
        _ => &[],
    };
    hints.iter().any(|h| lower.contains(h))
}
fn valid(provider: &str, lines: &[String], options: &[ApprovalOption], phrases: &[String]) -> bool {
    let context = lines.join("\n").to_lowercase();
    let mut hint = [
        "approval",
        "allow tool",
        "allow this",
        "requires approval",
        "requires permission",
        "requires confirmation",
        "permission required",
        "permissions required",
        "user confirmation",
        "would you like to run",
        "do you want to proceed",
        "press enter to confirm",
        "enter to select",
        "esc to cancel",
        "許可",
        "承認",
        "続行",
        "実行しますか",
        "よろしいですか",
        "確認してください",
    ]
    .iter()
    .any(|s| context.contains(s));
    if !hint {
        hint = lines.iter().any(|line| {
            let lower = line.to_lowercase();
            WEB_HINTS.iter().any(|s| lower.contains(s))
                || (lower.contains("press enter to confirm") && !lower.contains("esc to go back"))
                || (!model_hint(provider, &lower) && phrases.iter().any(|p| lower.contains(p)))
                || lower.contains("user specifies")
                || lower.contains("その他指定")
        });
    }
    if !hint {
        let extra: &[&str] = match provider {
            "cursor-agent" => &["allowlist", "run this command", "auto-run"],
            "opencode" => &["always allow", "until opencode is restarted"],
            "grok" => &["tab:next option", "always-approve", "type to add feedback"],
            "command-code" => &[
                "do you trust the files in this folder",
                "tool permission",
                "command code needs to run",
                "do you want to make this edit",
                "enter select",
            ],
            _ => &[],
        };
        hint = extra.iter().any(|s| context.contains(s));
    }
    if provider == "claude" && is_feedback(options) {
        return true;
    }
    if supports_shortcut(provider) && options.iter().any(|o| !o.send_text.is_empty()) {
        return hint;
    }
    hint && options.iter().any(|o| {
        let lower = o.label.to_lowercase();
        [
            "yes",
            "no",
            "allow",
            "deny",
            "once",
            "always",
            "all similar",
            "details",
            "proceed",
            "cancel",
            "don't ask",
            "dont ask",
            "(y)",
            "(n)",
            "(esc)",
        ]
        .iter()
        .any(|s| lower.contains(s))
    })
}
fn notice(provider: &str, question: &str) -> NativeApproval {
    let mut a = NativeApproval {
        sig: String::new(),
        kind: "ask_user_question".into(),
        question: question.trim().into(),
        context: String::new(),
        options: Vec::new(),
        summary: ApprovalSummary::default(),
    };
    a.sig = candidate(
        provider,
        &a.kind,
        &a.question,
        &a.context,
        &a.options,
        ApprovalSourceEpoch(1),
    )
    .key;
    a
}
pub fn detect_native(
    provider: &str,
    lines: &[String],
    phrases: &[String],
) -> Option<NativeApproval> {
    let recent: Vec<_> = lines[lines.len().saturating_sub(90)..]
        .iter()
        .map(|l| l.trim_end_matches(' ').to_owned())
        .collect();
    if recent.is_empty() {
        return None;
    }
    if is_multi_question(&recent) {
        return Some(notice(provider, ""));
    }
    let (mut options, start, end) = options(provider, &recent)?;
    if provider == "command-code" {
        let current = options.iter().position(|o| o.is_current).unwrap_or(0);
        for (i, o) in options.iter_mut().enumerate() {
            let direction = if i < current { "\x1b[A" } else { "\x1b[B" };
            o.send_text = format!("{}\r", direction.repeat(i.abs_diff(current)));
            o.preserve_order = true;
        }
    }
    let (context_start, context_end) = if provider == "opencode" {
        (start, (end + 1).min(recent.len()))
    } else {
        (start.saturating_sub(12), (end + 6).min(recent.len()))
    };
    let context_lines = &recent[context_start..context_end];
    let context = context_lines.join("\n");
    let mut question = context_lines[..start - context_start]
        .iter()
        .rev()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.trim().to_owned())
        .unwrap_or_default();
    if provider == "opencode" {
        let all = recent.join("\n").to_lowercase();
        if all.contains("select model")
            && [
                "connect provider",
                "favorite",
                "opencode zen",
                "ollama (local)",
                "recent",
            ]
            .iter()
            .any(|s| all.contains(s))
        {
            return None;
        }
        let always = context_lines
            .iter()
            .find(|l| !l.trim().is_empty())
            .is_some_and(|l| l.to_lowercase().contains("always allow"));
        if always {
            let mut fallback = String::new();
            question = String::new();
            for l in context_lines {
                let text = clean_label(l);
                let lower = text.to_lowercase();
                if text.is_empty()
                    || lower.contains("always allow")
                    || (lower.contains("confirm") && lower.contains("cancel"))
                {
                    continue;
                }
                if let Some(pattern) = text.strip_prefix('-')
                    && !pattern.trim().is_empty()
                {
                    question = pattern.trim().into();
                    break;
                }
                if fallback.is_empty() {
                    fallback = text;
                }
            }
            if question.is_empty() {
                question = fallback;
            }
        } else {
            question = context_lines
                .iter()
                .map(|l| l.trim())
                .find(|l| {
                    !l.is_empty()
                        && !l.to_lowercase().contains("permission required")
                        && !l.to_lowercase().contains("allow once")
                })
                .unwrap_or_default()
                .into();
        }
    }
    if options.iter().any(|o| {
        let lower = o.label.trim().to_lowercase();
        lower.starts_with("type something") || lower.starts_with("chat about")
    }) {
        return Some(notice(provider, &question));
    }
    if !valid(
        provider,
        if provider == "opencode" {
            &recent
        } else {
            context_lines
        },
        &options,
        phrases,
    ) {
        return None;
    }
    let kind = if options.iter().any(|o| !o.send_text.is_empty()) {
        match provider {
            "codex" => "native_codex_shortcut",
            "copilot" => "native_copilot_shortcut",
            "cursor-agent" => "native_cursor_agent_shortcut",
            "opencode" => "native_opencode_shortcut",
            _ => "native",
        }
    } else {
        "native"
    };
    let mut summary = summarize(&mask_secrets(&question), &mask_secrets(&context));
    summary.command = mask_secrets(&summary.command);
    summary.raw = mask_secrets(&summary.raw);
    for p in &mut summary.paths {
        *p = mask_secrets(p);
    }
    let sig = candidate(
        provider,
        kind,
        &question,
        &context,
        &options,
        ApprovalSourceEpoch(1),
    )
    .key;
    Some(NativeApproval {
        sig,
        kind: kind.into(),
        question,
        context,
        options,
        summary,
    })
}
