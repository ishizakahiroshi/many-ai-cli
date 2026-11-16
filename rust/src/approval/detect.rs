//! Native provider prompt detector, ported from approval_detector.go. The
//! caller supplies cached provider/common trigger phrases; detection performs no IO.
use super::{
    identity::candidate,
    summary::{mask_secrets, risk_before_masking, summarize},
};
use crate::proto::{
    ApprovalOption, ApprovalSummary,
    core::ApprovalSourceEpoch,
    provider::{ApprovalButtons, ApprovalDefinition, ApprovalKey},
};
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
    LazyLock::new(|| re(r"^\s*([>❯›❱])?\s*(.+?)\s+\(([^()]+)\)\s*$"));
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
static NO_RULES: LazyLock<ApprovalDefinition> = LazyLock::new(ApprovalDefinition::default);
/// Runs `read` with the approval declaration of `provider`: its own, the
/// built-in one, or (for a provider that declares nothing) the plain numbered
/// detection.
fn with_rules<R>(provider: &str, read: impl FnOnce(&ApprovalDefinition) -> R) -> R {
    let mut read = Some(read);
    if let Some(out) = crate::profile::declared::with_declared(provider, |d| {
        (read.take().expect("read once"))(d.approval.as_ref().unwrap_or(&NO_RULES))
    }) {
        return out;
    }
    (read.take().expect("read once"))(&NO_RULES)
}
fn buttons_style(rules: &ApprovalDefinition) -> bool {
    rules.style == "buttons"
}
fn has_keys(rules: &ApprovalDefinition) -> bool {
    !rules.keys.is_empty()
}
fn key_entry<'a>(table: &'a [ApprovalKey], key: &str) -> Option<(&'a ApprovalKey, &'a str)> {
    table.iter().find_map(|entry| {
        entry
            .keys
            .iter()
            .find(|k| k.as_str() == key)
            .map(|k| (entry, k.as_str()))
    })
}
fn key_send(entry: &ApprovalKey, key: &str) -> String {
    if entry.send.is_empty() {
        key.into()
    } else {
        entry.send.clone()
    }
}
pub fn shortcut_send(provider: &str, label: &str) -> String {
    with_rules(provider, |rules| shortcut_send_with(rules, label))
}
fn shortcut_send_with(rules: &ApprovalDefinition, label: &str) -> String {
    let lower = label.trim().to_lowercase();
    for entry in &rules.keys {
        for key in &entry.keys {
            if lower == *key || lower.ends_with(&format!("({key})")) {
                return key_send(entry, key);
            }
        }
    }
    String::new()
}
pub fn parse_option(provider: &str, line: &str) -> Option<ApprovalOption> {
    with_rules(provider, |rules| parse_option_with(rules, line))
}
fn parse_option_with(rules: &ApprovalDefinition, line: &str) -> Option<ApprovalOption> {
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
        let send = shortcut_send_with(rules, &label);
        return Some(ApprovalOption {
            num,
            label,
            is_current: c.get(1).is_some_and(|v| !v.as_str().is_empty()),
            preserve_order: !send.is_empty(),
            send_text: send,
        });
    }
    for extra in &rules.line_extras {
        match extra.as_str() {
            "paren_marker" => {
                if let Some(c) = GROK.captures(line) {
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
            }
            "bracket_keys" => {
                let c = CURSOR.captures(line)?;
                let key = c[3].trim();
                let lower = key.to_lowercase();
                let (entry, _) = key_entry(&rules.bracket_keys, &lower)?;
                let label = clean_label(&format!("{} ({key})", &c[2]));
                if label.is_empty() {
                    return None;
                }
                return Some(ApprovalOption {
                    num: entry.num,
                    label,
                    is_current: c.get(1).is_some_and(|m| !m.as_str().is_empty()),
                    send_text: key_send(entry, &lower),
                    preserve_order: true,
                });
            }
            "key_lines" if has_keys(rules) => {
                if let Some(c) = SHORTCUT.captures(line) {
                    let key = c[3].to_lowercase();
                    let label = clean_label(&format!("{} ({})", &c[2], &c[3]));
                    // The key must be spelled exactly as declared (the line is
                    // matched case-sensitively, the stored key in lower case).
                    let declared = key_entry(&rules.keys, &c[3]).is_some();
                    let send = shortcut_send_with(rules, &key);
                    if declared && !label.is_empty() && !send.is_empty() {
                        return Some(ApprovalOption {
                            num: key_entry(&rules.keys, &key).map_or(0, |(e, _)| e.num),
                            label,
                            is_current: c.get(1).is_some(),
                            send_text: send,
                            preserve_order: true,
                        });
                    }
                }
            }
            _ => {}
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
/// Wording-matched button rows (`style: buttons`). The last line that holds
/// every word of a row's `line_all` ends the prompt.
fn button_options(
    rows: &[ApprovalButtons],
    lines: &[String],
) -> Option<(Vec<ApprovalOption>, usize, usize)> {
    for (i, line) in lines.iter().enumerate().rev() {
        let lower = line.to_lowercase();
        for row in rows {
            if row.line_all.is_empty() || !row.line_all.iter().all(|w| lower.contains(w.as_str())) {
                continue;
            }
            let found = (!row.start_line.is_empty())
                .then(|| {
                    (i.saturating_sub(12)..=i)
                        .rev()
                        .find(|j| lines[*j].to_lowercase().contains(row.start_line.as_str()))
                })
                .flatten();
            match found {
                Some(start) => return Some((buttons(&row.labels), start, i)),
                None if !row.start_required => {
                    return Some((buttons(&row.labels), i.saturating_sub(12), i));
                }
                None => {}
            }
        }
    }
    None
}
fn buttons(labels: &[String]) -> Vec<ApprovalOption> {
    labels
        .iter()
        .enumerate()
        .map(|(i, l)| ApprovalOption {
            num: i as i64 + 1,
            label: l.clone(),
            send_text: format!("{}\r", "\x1b[C".repeat(i)),
            is_current: i == 0,
            preserve_order: true,
        })
        .collect()
}
fn options(
    rules: &ApprovalDefinition,
    lines: &[String],
) -> Option<(Vec<ApprovalOption>, usize, usize)> {
    if buttons_style(rules) {
        return button_options(&rules.buttons, lines);
    }
    let feedback = if rules.feedback_survey {
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
        .filter_map(|(i, l)| parse_option_with(rules, l).map(|o| (o, i)))
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
fn model_hint(rules: &ApprovalDefinition, lower: &str) -> bool {
    rules
        .not_approval_phrases
        .iter()
        .any(|h| lower.contains(h.as_str()))
}
fn valid(
    rules: &ApprovalDefinition,
    lines: &[String],
    options: &[ApprovalOption],
    phrases: &[String],
) -> bool {
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
                || (!model_hint(rules, &lower) && phrases.iter().any(|p| lower.contains(p)))
                || lower.contains("user specifies")
                || lower.contains("その他指定")
        });
    }
    if !hint {
        hint = rules
            .hint_phrases
            .iter()
            .any(|s| context.contains(s.as_str()));
    }
    if rules.feedback_survey && is_feedback(options) {
        return true;
    }
    if has_keys(rules) && options.iter().any(|o| !o.send_text.is_empty()) {
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
/// The masked terminal-text summary of a native prompt. Callers that re-check a
/// recorded prompt start from this, never from an already authorized summary.
///
/// Audit N01: the risk is judged on the text as the terminal drew it. Only the
/// risk leaves this function; every string in the summary is masked as before.
pub fn native_summary(question: &str, context: &str) -> ApprovalSummary {
    let judged = summarize(question, context);
    let mut summary = summarize(&mask_secrets(question), &mask_secrets(context));
    summary.command = mask_secrets(&summary.command);
    summary.risk = risk_before_masking(&judged.command, &summary.command).into();
    summary.raw = mask_secrets(&summary.raw);
    for p in &mut summary.paths {
        *p = mask_secrets(p);
    }
    summary
}
pub fn detect_native(
    provider: &str,
    lines: &[String],
    phrases: &[String],
) -> Option<NativeApproval> {
    with_rules(provider, |rules| {
        detect_with(provider, rules, lines, phrases)
    })
}
fn detect_with(
    provider: &str,
    rules: &ApprovalDefinition,
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
    let (mut options, start, end) = options(rules, &recent)?;
    if rules.arrow_send {
        let current = options.iter().position(|o| o.is_current).unwrap_or(0);
        for (i, o) in options.iter_mut().enumerate() {
            let direction = if i < current { "\x1b[A" } else { "\x1b[B" };
            o.send_text = format!("{}\r", direction.repeat(i.abs_diff(current)));
            o.preserve_order = true;
        }
    }
    let buttons = buttons_style(rules);
    let (context_start, context_end) = if buttons {
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
    let all = recent.join("\n").to_lowercase();
    if rules.suppress_screens.iter().any(|s| {
        s.all.iter().all(|w| all.contains(w.as_str()))
            && (s.any.is_empty() || s.any.iter().any(|w| all.contains(w.as_str())))
    }) {
        return None;
    }
    if buttons {
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
        rules,
        if buttons { &recent } else { context_lines },
        &options,
        phrases,
    ) {
        return None;
    }
    let shortcut_kind = format!("native_{}_shortcut", provider.replace('-', "_"));
    let kind = if rules.shortcut_kind && options.iter().any(|o| !o.send_text.is_empty()) {
        shortcut_kind.as_str()
    } else {
        "native"
    };
    let summary = native_summary(&question, &context);
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
#[cfg(test)]
mod tests {
    use super::*;
    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_owned).collect()
    }
    fn rules(json: &str) -> ApprovalDefinition {
        serde_json::from_str(json).unwrap()
    }
    fn found(r: &ApprovalDefinition, text: &str) -> Option<NativeApproval> {
        detect_with("synthetic-tool", r, &lines(text), &[])
    }
    #[test]
    fn a_provider_that_declares_nothing_gets_the_plain_numbered_detection() {
        let none = ApprovalDefinition::default();
        let a = found(&none, "Do you want to proceed?\n❯ 1. Yes\n  2. No").unwrap();
        assert_eq!(a.kind, "native");
        assert_eq!(a.options.len(), 2);
        assert!(a.options.iter().all(|o| o.send_text.is_empty()));
        // No key, button or marker shape is read without a declaration.
        assert!(found(&none, "Approve (y)\nDeny (n)").is_none());
        assert!(
            found(
                &none,
                "Permission required\nAllow once  Allow always  Reject"
            )
            .is_none()
        );
        assert!(found(&none, "Allow? \n1 (•) Yes\n2 (○) No").is_none());
    }
    #[test]
    fn declared_keys_make_key_lines_and_shortcut_kinds() {
        let r = rules(
            r#"{"line_extras":["key_lines"],"shortcut_kind":true,
            "keys":[{"keys":["y"],"num":1},{"keys":["n"],"num":2},{"keys":["esc"],"num":3,"send":"\u001b"}]}"#,
        );
        let a = found(
            &r,
            "Would you like to run?\nApprove (y)\nDeny (n)\nCancel (esc)",
        )
        .unwrap();
        assert_eq!(a.kind, "native_synthetic_tool_shortcut");
        let sends: Vec<_> = a
            .options
            .iter()
            .map(|o| (o.num, o.send_text.as_str()))
            .collect();
        assert_eq!(sends, [(1, "y"), (2, "n"), (3, "\x1b")]);
        // A key that is not declared, or spelled in another case, is no option.
        assert!(found(&r, "Would you like to run?\nApprove (Y)\nDeny (N)").is_none());
        assert!(found(&r, "Would you like to run?\nApprove (q)\nDeny (z)").is_none());
        // A numbered option whose label ends with a declared key sends it.
        let n = found(&r, "Would you like to run?\n❯ 1. Yes (y)\n  2. No (n)").unwrap();
        assert_eq!(n.options[0].send_text, "y");
        assert!(n.options.iter().all(|o| o.preserve_order));
    }
    #[test]
    fn declared_buttons_arrows_and_suppressed_screens_follow_the_wording() {
        let b = rules(
            r#"{"style":"buttons","shortcut_kind":true,
            "buttons":[{"line_all":["run","skip"],"start_line":"needs approval","start_required":true,"labels":["Allow","Deny"]}],
            "suppress_screens":[{"all":["pick a model"],"any":["recent"]}]}"#,
        );
        let a = found(&b, "Tool needs approval\nrm file\nRun   Skip").unwrap();
        assert_eq!(
            a.options
                .iter()
                .map(|o| o.send_text.as_str())
                .collect::<Vec<_>>(),
            ["\r", "\x1b[C\r"]
        );
        assert!(a.options[0].is_current);
        // The start line is required for this row.
        assert!(found(&b, "rm file\nRun   Skip").is_none());
        assert!(found(&b, "Pick a model\nRecent\nTool needs approval\nRun   Skip").is_none());
        let arrows = rules(r#"{"arrow_send":true,"hint_phrases":["trust this folder"]}"#);
        let a = found(&arrows, "Trust this folder?\n  1. Yes\n❯ 2. No\n  3. Never").unwrap();
        assert_eq!(
            a.options
                .iter()
                .map(|o| o.send_text.as_str())
                .collect::<Vec<_>>(),
            ["\x1b[A\r", "\r", "\x1b[B\r"]
        );
        // Without the declared wording the same screen is not an approval.
        assert!(
            found(
                &ApprovalDefinition::default(),
                "Trust this folder?\n  1. Yes\n❯ 2. No"
            )
            .is_none()
        );
    }
    #[test]
    fn a_model_choice_screen_is_not_taken_for_a_prompt_by_a_learned_phrase() {
        let r = rules(r#"{"not_approval_phrases":["select model"]}"#);
        let text = "Select model\n❯ 1. Yes\n  2. No";
        let phrases = vec!["select model".to_owned()];
        assert!(detect_with("synthetic-tool", &r, &lines(text), &phrases).is_none());
        assert!(
            detect_with(
                "synthetic-tool",
                &ApprovalDefinition::default(),
                &lines(text),
                &phrases
            )
            .is_some()
        );
    }
}
