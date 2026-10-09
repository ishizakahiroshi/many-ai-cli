//! Pure text-question grammar from fixed Go approval_text_question.go.
//! Opening, idle timing and answered-ledger admission remain with session core.
use super::{identity, marker};
use crate::proto::ApprovalOption;
use regex::Regex;
use std::{collections::HashSet, sync::LazyLock};
#[cfg(test)]
mod tests;

#[derive(Clone)]
pub struct TextQuestion {
    pub kind: String,
    pub block: String,
    pub sig: String,
    pub question: String,
    pub options: Vec<ApprovalOption>,
}
fn new(kind: &str, block: String, question: String, options: Vec<ApprovalOption>) -> TextQuestion {
    TextQuestion {
        kind: kind.into(),
        sig: identity::digest(&identity::normalize(&block)),
        block,
        question,
        options,
    }
}
// Go regexp's Perl classes/boundaries are ASCII; string field/trim operations
// below deliberately retain Go's separate Unicode whitespace behavior.
fn regex(pattern: &str) -> Regex {
    Regex::new(
        &pattern
            .replace(r"\s", r"(?-u:\s)")
            .replace(r"\S", r"[^\t\n\f\r ]")
            .replace(r"\d", "[0-9]")
            .replace(r"\b", r"(?-u:\b)"),
    )
    .unwrap()
}
macro_rules! re {
    ($name:ident, $pattern:literal) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| regex($pattern));
    };
}
mod unglue;
pub use unglue::unglued_lines;
re!(
    YESNO,
    r"(?i)[（(]\s*[YＹ]\s*[:：]\s*1\s*[/／]\s*[NＮ]\s*[:：]\s*0\s*[）)]"
);
re!(PLACEHOLDER, r"(?i)^question\s*\d*\s*[?？]$");
re!(QUESTION_END, r"[?？]\s*$");
re!(
    HEADER,
    r"(?i)^\s*([A-Z]{1,3}\d{1,3}|Q\d{1,3}|問\d{1,3})\s*[:：]\s*(.+?)\s*$"
);
re!(OPTION, r"^\s*(\d{1,2})\.\s*(.+?)\s*$");
re!(SEQUENTIAL_N, r"(?i)^\s*N\.\s*(User specifies|その他指定)");
re!(TWO_SPACES, r"^\s{2,}");
re!(USER, r"(?i)user specifies|その他指定");
re!(
    HUB_QUESTION,
    r"(?i)どれで進めますか|どれで進める|どちらで進め|どの選択肢|選択してください|how would you like to proceed|which option"
);
re!(RECOMMENDED, r"(?i)\(recommended\)|（recommended）|推奨");
re!(CURSOR, r"^\s*[>❯›❱]\s*(\d{1,2})\.\s*(.+?)\s*$");
re!(RADIO, r"^\s*(\d{1,2})\s+\(([•●○*\-])\)\s+(.+?)\s*$");
re!(DOUBLE_TAIL, r"\s{2,}.*$");
re!(GLUED_NEXT, r"\s*\d+\.\s*[A-Za-z].*$");
re!(SHORTCUT, r"(?i)\((y|p|n|!|#|\?|esc|escape)\)\s*$");
re!(OPTION_LINE, r"^\s*(?:[>❯›❱]\s*)?\d{1,2}\.\s*\S");
fn marker_token(text: &str) -> bool {
    text.contains(marker::OPEN) || text.contains(marker::CLOSE)
}
fn remove_tokens(text: &str) -> String {
    text.replace(marker::OPEN, "").replace(marker::CLOSE, "")
}
fn fields(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
fn yesno_question(text: &str) -> String {
    YESNO
        .find_iter(text)
        .last()
        .map(|m| fields(&remove_tokens(&text[..m.start()])))
        .unwrap_or_default()
}
fn looks_yesno(text: &str) -> bool {
    if !YESNO.is_match(text) {
        return false;
    }
    let before = yesno_question(text);
    if PLACEHOLDER.is_match(&before) {
        return false;
    }
    QUESTION_END.is_match(before.trim())
        || before
            .chars()
            .rev()
            .take(120)
            .any(|c| matches!(c, '?' | '？'))
}
fn yesno(text: &str) -> TextQuestion {
    new(
        "plain_yes_no",
        fields(&remove_tokens(text)),
        yesno_question(text),
        vec![
            ApprovalOption {
                num: 1,
                label: "Yes (1)".into(),
                is_current: true,
                preserve_order: true,
                ..Default::default()
            },
            ApprovalOption {
                num: 0,
                label: "No (0)".into(),
                preserve_order: true,
                ..Default::default()
            },
        ],
    )
}
fn plain(lines: &[String]) -> Option<TextQuestion> {
    let start = lines.len().saturating_sub(20);
    for line in lines[start..].iter().rev().map(|s| s.trim()) {
        if !line.is_empty() && !marker_token(line) && looks_yesno(line) {
            return Some(yesno(line));
        }
    }
    let text = lines[start..]
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    (!marker_token(&text) && looks_yesno(&text)).then(|| yesno(&text))
}
struct Sequential {
    key: String,
    question: String,
    options: Vec<ApprovalOption>,
}
fn sequential(lines: &[String]) -> Option<TextQuestion> {
    let mut prompts = Vec::new();
    let mut current: Option<Sequential> = None;
    for raw in &lines[lines.len().saturating_sub(80)..] {
        let rawline = raw.trim_end();
        let line = rawline.trim();
        if line.is_empty() {
            continue;
        }
        if marker_token(line) {
            return None;
        }
        if let Some(m) = HEADER.captures(line) {
            if let Some(previous) = current.take().filter(|p| p.options.len() >= 2) {
                prompts.push(previous);
            }
            current = Some(Sequential {
                key: m[1].trim().into(),
                question: m[2].trim().into(),
                options: vec![],
            });
            continue;
        }
        let Some(prompt) = &mut current else {
            continue;
        };
        if let Some(m) = OPTION.captures(line) {
            prompt.options.push(ApprovalOption {
                num: m[1].parse().unwrap_or(0),
                label: m[2].trim().into(),
                is_current: prompt.options.is_empty(),
                ..Default::default()
            });
            continue;
        }
        if SEQUENTIAL_N.is_match(line) {
            continue;
        }
        if !prompt.options.is_empty()
            && !TWO_SPACES.is_match(rawline)
            && let Some(previous) = current.take().filter(|p| p.options.len() >= 2)
        {
            prompts.push(previous);
        }
    }
    if let Some(previous) = current.filter(|p| p.options.len() >= 2) {
        prompts.push(previous);
    }
    let mut seen = HashSet::new();
    prompts.retain(|p| seen.insert(format!("{}:{}", p.key, p.question)));
    if prompts.len() < 2 {
        return None;
    }
    let mut block = Vec::new();
    let mut questions = Vec::new();
    let mut options = Vec::new();
    for mut p in prompts {
        p.options.sort_by_key(|v| v.num);
        let header = format!("{}: {}", p.key, p.question);
        block.push(header.clone());
        questions.push(header);
        for option in p.options {
            block.push(format!("  {}. {}", option.num, option.label));
            options.push(ApprovalOption {
                num: option.num,
                label: option.label,
                ..Default::default()
            });
        }
    }
    Some(new(
        "sequential_choice",
        block.join("\n"),
        questions.join("\n"),
        options,
    ))
}
fn fallback_option(number: &str, label: &str, current: bool) -> ApprovalOption {
    let label = DOUBLE_TAIL.replace_all(label.trim(), "");
    let label = GLUED_NEXT.replace_all(&label, "").trim().to_owned();
    let send_text = SHORTCUT
        .captures(&label)
        .map(|m| match m[1].to_lowercase().as_str() {
            "esc" | "escape" => "\x1b".into(),
            v => v.into(),
        })
        .unwrap_or_default();
    ApprovalOption {
        num: number.parse().unwrap_or(0),
        label,
        is_current: current,
        send_text,
        ..Default::default()
    }
}
struct Fallback {
    options: Vec<ApprovalOption>,
    lines: Vec<String>,
    start: usize,
    end: usize,
}
fn consume(label: &str, continuation: &mut Vec<String>) -> String {
    if continuation.is_empty() {
        return label.into();
    }
    let tail = continuation
        .iter()
        .rev()
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    continuation.clear();
    fields(&format!("{label} {tail}"))
}
fn fallback(raw: &[String]) -> Option<Fallback> {
    let lines = unglued_lines(raw);
    let mut options = Vec::new();
    let mut start = 0;
    let mut end = None;
    let mut blank = 0;
    let mut continuation = Vec::new();
    for (i, line) in lines.iter().enumerate().rev() {
        let option = if let Some(m) = CURSOR.captures(line) {
            Some(fallback_option(
                &m[1],
                &consume(&m[2], &mut continuation),
                true,
            ))
        } else if let Some(m) = OPTION.captures(line) {
            Some(fallback_option(
                &m[1],
                &consume(&m[2], &mut continuation),
                false,
            ))
        } else {
            let radio = line
                .trim_start_matches(['│', '┃'])
                .trim_end_matches(['│', '┃'])
                .trim();
            RADIO
                .captures(radio)
                .map(|m| fallback_option(&m[1], &consume(&m[3], &mut continuation), &m[2] != "○"))
        };
        if let Some(option) = option {
            options.insert(0, option);
            end.get_or_insert(i);
            start = i;
            blank = 0;
            continue;
        }
        if end.is_none() {
            continue;
        }
        if line.trim().is_empty() {
            blank += 1;
            if blank > 4 {
                break;
            }
            continuation.clear();
            continue;
        }
        if blank == 0 {
            continuation.push(line.trim().to_owned());
            continue;
        }
        break;
    }
    let min = options.iter().map(|v| v.num).min()?;
    let max = options.iter().map(|v| v.num).max()?;
    if max > 20 || max - min > 15 || options.len() > 12 {
        return None;
    }
    let mut seen = HashSet::new();
    options.retain(|v| seen.insert(format!("{}:{}", v.num, v.label)));
    if options.len() < 2
        || options.iter().any(|v| {
            let l = v.label.trim().to_lowercase();
            l.starts_with("type something") || l.starts_with("chat about")
        })
    {
        return None;
    }
    Some(Fallback {
        options,
        lines,
        start,
        end: end?,
    })
}
fn hub(lines: &[String]) -> Option<TextQuestion> {
    if !lines.iter().any(|s| USER.is_match(s)) {
        return None;
    }
    let mut found = fallback(lines)?;
    let context =
        &found.lines[found.start.saturating_sub(10)..(found.end + 11).min(found.lines.len())];
    let first = context
        .iter()
        .position(|s| OPTION_LINE.is_match(s))
        .unwrap_or(context.len());
    if !context[..first].iter().any(|s| HUB_QUESTION.is_match(s))
        || !(context.iter().any(|s| USER.is_match(s))
            || found.options.iter().any(|s| USER.is_match(&s.label)))
    {
        return None;
    }
    let question = (found.start.saturating_sub(10)..found.start)
        .rev()
        .find_map(|i| {
            HUB_QUESTION
                .is_match(&found.lines[i])
                .then(|| fields(&found.lines[i]))
        })
        .unwrap_or_default();
    if !found.options.iter().any(|v| v.is_current) {
        let i = found
            .options
            .iter()
            .position(|v| RECOMMENDED.is_match(&v.label))
            .or_else(|| found.options.iter().position(|v| v.num == 1))
            .unwrap_or(0);
        found.options[i].is_current = true;
    }
    let mut block = Vec::new();
    if !question.is_empty() {
        block.push(question.clone());
    }
    for option in &mut found.options {
        option.send_text.clear();
        block.push(format!("{}. {}", option.num, option.label));
    }
    block.push("N. User specifies".into());
    Some(new("hub_choice", block.join("\n"), question, found.options))
}
pub fn detect(lines: &[String]) -> Option<TextQuestion> {
    let end = lines
        .iter()
        .rposition(|v| !v.trim().is_empty())
        .map_or(0, |i| i + 1);
    let lines = &lines[..end];
    plain(lines)
        .or_else(|| sequential(lines))
        .or_else(|| hub(lines))
}
