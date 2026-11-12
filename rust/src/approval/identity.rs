//! Candidate identity material exactly follows approval_identity.go. Labels,
//! record signature and replay epoch are deliberately not suppression identity.
use super::summary::{command_from_line, command_heading, summarize};
use crate::proto::{
    ApprovalOption,
    core::{ApprovalSourceEpoch, CandidateIdentity},
};
use regex::Regex;
use sha2::{Digest, Sha256};
use std::sync::LazyLock;
static ANSI: LazyLock<[Regex; 3]> = LazyLock::new(|| {
    [
        Regex::new(r"\x1b\][^\x07]*(?:\x07|\x1b\\)").unwrap(),
        Regex::new(r"\x1b\[[0-?]*[ -/]*[@-~]").unwrap(),
        Regex::new(r"\x1b[@-_]").unwrap(),
    ]
});
static QUESTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?-u:\s)*(?:Q[0-9]{1,3}|#multi)(?-u:\s)*[:：]?(?-u:\s)*(.*?)(?-u:\s)*$").unwrap()
});
static OPTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?-u:\s)*([0-9]{1,2})\.(?-u:\s)*(.*?)(?-u:\s)*$").unwrap());
static YESNO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[（(](?-u:\s)*[YＹ](?-u:\s)*[:：](?-u:\s)*1(?-u:\s)*[／/](?-u:\s)*[NＮ](?-u:\s)*[:：](?-u:\s)*0(?-u:\s)*[）)]").unwrap()
});
pub fn strip_ansi(text: &str) -> String {
    ANSI.iter()
        .fold(text.to_owned(), |s, re| re.replace_all(&s, "").into_owned())
}
pub fn normalize(text: &str) -> String {
    strip_ansi(text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
pub fn digest(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub fn candidate_shape(
    provider: &str,
    kind: &str,
    question: &str,
    options: &[ApprovalOption],
) -> String {
    let mut options = options.to_vec();
    options.sort_by(|a, b| {
        a.num
            .cmp(&b.num)
            .then_with(|| a.send_text.cmp(&b.send_text))
    });
    let mut shape = format!(
        "{}\n{}\n{}",
        normalize(&provider.to_lowercase()),
        normalize(&kind.to_lowercase()),
        normalize(question)
    );
    for option in options {
        shape.push_str(&format!(
            "\n{}:{}",
            option.num,
            normalize(&option.send_text)
        ));
    }
    shape
}
pub fn candidate(
    provider: &str,
    kind: &str,
    question: &str,
    context: &str,
    options: &[ApprovalOption],
    epoch: ApprovalSourceEpoch,
) -> CandidateIdentity {
    let question = question_with_context(question, context, options);
    let shape = candidate_shape(provider, kind, &question, options);
    let key = digest(&shape)[..16].into();
    CandidateIdentity {
        key,
        shape,
        source_epoch: epoch,
    }
}
pub fn question_with_context(question: &str, context: &str, options: &[ApprovalOption]) -> String {
    if options.iter().any(|o| !o.send_text.is_empty()) || command_from_line(question).is_some() {
        return question.into();
    }
    let subject =
        heading_subject(question, context).unwrap_or_else(|| summarize(question, context).command);
    if subject.is_empty() || normalize(&subject) == normalize(question) {
        question.into()
    } else {
        format!("{}\n{subject}", question.trim())
    }
}
/// Everything a command heading prompt shows between the heading and the
/// question (audit N03). `summarize` keeps only the first line under the heading,
/// so two commands that start alike (`git commit -m "$(cat <<'EOF'`) were one
/// candidate, and the second prompt was taken for the one just answered. White
/// space is dropped: the same command wraps differently at another width.
fn heading_subject(question: &str, context: &str) -> Option<String> {
    let question = question.trim();
    if question.is_empty() || command_heading(question) {
        return None;
    }
    let mut lines = context
        .split('\n')
        .skip_while(|line| !command_heading(line.trim()));
    lines.next()?;
    let body: String = lines
        .take_while(|line| line.trim() != question)
        .flat_map(str::chars)
        .filter(|c| !c.is_whitespace())
        .collect();
    (!body.is_empty()).then_some(body)
}
pub fn marker_candidate(
    provider: &str,
    block: &str,
    epoch: ApprovalSourceEpoch,
) -> CandidateIdentity {
    let clean = strip_ansi(block);
    let mut questions = Vec::new();
    let mut options = Vec::new();
    let mut seen = false;
    for line in clean.lines().map(str::trim) {
        if line.is_empty() || line.contains("[MANY-AI-CLI]") || line.contains("[/MANY-AI-CLI]") {
            continue;
        }
        if let Some(c) = QUESTION.captures(line) {
            let q = normalize(&c[1]);
            if !q.is_empty() {
                questions.push(q);
                seen = true;
            }
            continue;
        }
        if let Some(c) = OPTION.captures(line) {
            options.push(ApprovalOption {
                num: c[1].parse().unwrap_or(0),
                ..Default::default()
            });
            continue;
        }
        if YESNO.is_match(line) {
            let q = normalize(&YESNO.replace_all(line, ""));
            if !q.is_empty() {
                questions.push(q);
                seen = true;
            }
            options.extend([
                ApprovalOption {
                    num: 1,
                    ..Default::default()
                },
                ApprovalOption::default(),
            ]);
            continue;
        }
        if !seen && (line.ends_with('?') || line.ends_with('？')) {
            questions.push(normalize(line));
            seen = true;
        }
    }
    if questions.is_empty() {
        for line in clean.lines().map(str::trim) {
            if line.is_empty()
                || line.contains("[MANY-AI-CLI]")
                || line.contains("[/MANY-AI-CLI]")
                || OPTION.is_match(line)
                || line.starts_with("N.")
            {
                continue;
            }
            questions.push(normalize(line));
            break;
        }
    }
    let shape = candidate_shape(provider, "marker", &questions.join("\n"), &options);
    let key = digest(&shape)[..16].into();
    CandidateIdentity {
        key,
        shape,
        source_epoch: epoch,
    }
}
