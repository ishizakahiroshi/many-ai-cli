//! Provider-neutral approval facts/risk, ported from internal/approval/summary.go.
use crate::proto::ApprovalSummary;
use regex::Regex;
use std::{collections::HashSet, sync::LazyLock};
static PREFIX: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"(?i)^(?-u:\s)*(?:run|command|bash command|shell command)(?-u:\s)*:(?-u:\s)*(.+)$",
        r"(?i)^(?-u:\s)*not in allowlist(?-u:\s)*:(?-u:\s)*(.+)$",
        r"(?i)^(?-u:\s)*(?:execute|executing)(?-u:\s)+(.+)$",
    ]
    .map(|p| Regex::new(p).unwrap())
    .into()
});
static HIGH: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
 r"(?i)(?-u:\b)rm(?-u:\s)+(?:-[a-z]*r[a-z]*|--recursive)",
 r"(?i)(?-u:\b)(?:chmod|chown)(?-u:\s)+(?:-[a-z]*r[a-z]*|--recursive)",
 r"(?i)(?-u:\b)remove-item(?-u:\b)[^\n]*-recurse",
 r"(?i)(?-u:\b)(?:curl|wget|iwr|invoke-webrequest)(?-u:\b)[^\n]*\|(?-u:\s)*(?:sudo(?-u:\s)+)?(?:sh|bash|zsh|python[0-9.]*|perl|node|pwsh|powershell|iex)(?-u:\b)",
 r"(?i)(?-u:\b)git(?-u:\s)+reset(?-u:\s)+--hard(?-u:\b)",r"(?i)(?-u:\b)git(?-u:\s)+clean(?-u:\s)+-",r"(?i)(?-u:\b)git(?-u:\s)+push(?-u:\b)[^\n]*(?:--force|--force-with-lease|(?-u:\s)-f(?-u:\b))",
 r"(?i)(?-u:\b)(?:sudo|doas)(?-u:\s)",r"(?i)(?-u:\b)(?:mkfs[^\t\n\f\r ]*|shutdown|reboot|halt|poweroff)(?-u:\b)",r"(?i)(?-u:\b)dd(?-u:\s)+[^\n]*(?-u:\b)of=",r"(?i)(?:^|(?-u:\s))(?:del|rmdir)(?-u:\s)+/s(?-u:\b)",r"(?i)(?-u:\b)format(?-u:\s)+[a-z]:"
].map(|p|Regex::new(p).unwrap()).into()
});
static FIND_EFFECT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(?-u:\b)find(?-u:\b)[^\n]*(?-u:\s)-(?:exec|execdir|ok|okdir|delete|fls|fprint|fprint0|fprintf)(?-u:\b)",
    )
    .unwrap()
});
static GIT_OUTPUT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?-u:\b)git(?-u:\b)[^\n]*(?-u:\s)--output(?-u:\b)").unwrap());
static GIT_BRANCH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(?-u:\s)*git(?-u:\s)+branch(?:(?-u:\s)|$)").unwrap());
static EXTERNAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)(?:^|(?-u:\s))["']?--(?:pre|hostname-bin)(?:=|(?-u:\s)|["']|$)"#).unwrap()
});
static PATHS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)(?:[a-z]:[\\/][^\t\n\f\r '"`<>|]+|(?:\.\.?[\\/]|/)[^\t\n\f\r '"`<>|]+|\*\.[a-z0-9_-]+)"#)
        .unwrap()
});
static SPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?-u:\s)+").unwrap());
fn compact(s: &str) -> String {
    SPACE.replace_all(s, " ").trim().to_owned()
}

pub fn command_from_line(line: &str) -> Option<String> {
    PREFIX
        .iter()
        .find_map(|p| p.captures(line.trim()).map(|c| c[1].to_owned()))
}
fn command_heading(line: &str) -> bool {
    matches!(
        line.trim_end_matches(':').trim().to_lowercase().as_str(),
        "bash command" | "shell command"
    )
}
pub fn extract_command(question: &str, context: &str) -> String {
    let lines: Vec<_> = std::iter::once(question)
        .chain(context.split('\n'))
        .collect();
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();
    for (i, raw) in lines.iter().enumerate() {
        let line = raw.trim();
        let candidate = if let Some(c) = command_from_line(line) {
            Some(c)
        } else if command_heading(line) {
            lines[i + 1..]
                .iter()
                .map(|s| s.trim())
                .find(|s| !s.is_empty() && !command_heading(s))
                .map(|s| command_from_line(s).unwrap_or_else(|| s.to_owned()))
        } else {
            None
        };
        if let Some(candidate) = candidate {
            let c = compact(&candidate);
            if !c.is_empty() && seen.insert(c.clone()) {
                candidates.push(c);
            }
        }
    }
    if candidates.is_empty() {
        compact(question)
    } else {
        candidates.join("\n")
    }
}
pub fn summarize(question: &str, context: &str) -> ApprovalSummary {
    let command = extract_command(question, context);
    let risk = classify_risk(&command).into();
    let mut paths = Vec::new();
    let mut seen = HashSet::new();
    let text = format!("{command}\n{context}");
    for found in PATHS.find_iter(&text) {
        let path = found
            .as_str()
            .trim_end_matches(['.', ',', ';', ':', ')', ']', '}']);
        if !path.is_empty() && seen.insert(path.to_lowercase()) {
            paths.push(path.into());
        }
        if paths.len() == 4 {
            break;
        }
    }
    ApprovalSummary {
        command,
        paths,
        risk,
        raw: context.trim().into(),
    }
}
pub fn has_external_command_option(command: &str) -> bool {
    EXTERNAL.is_match(command)
}
pub fn classify_risk(command: &str) -> &'static str {
    let value = command.trim().to_lowercase();
    if value.is_empty() {
        return "mid";
    }
    let signals = [
        "rm -rf",
        "rmdir /s",
        "del /s",
        "remove-item -recurse",
        "git reset --hard",
        "git clean -",
        "push --force",
        "push -f",
        "sudo ",
        "doas ",
        "mkfs",
        "dd ",
        "shutdown",
        "reboot",
        "format ",
        "curl |",
        "wget |",
        "chmod -r",
        "chown -r",
    ];
    if HIGH.iter().any(|p| p.is_match(&value)) || signals.iter().any(|s| value.contains(s)) {
        return "high";
    }
    if has_write_redirect(&value)
        || is_git_branch_mutation(&value)
        || FIND_EFFECT.is_match(&value)
        || GIT_OUTPUT.is_match(&value)
        || EXTERNAL.is_match(&value)
        || ["$(", "`", "<(", "=(", "@("]
            .iter()
            .any(|s| value.contains(s))
    {
        return "mid";
    }
    let low = [
        "cat ",
        "head ",
        "tail ",
        "ls",
        "dir",
        "pwd",
        "git status",
        "git diff",
        "git log",
        "git show",
        "git branch",
        "find ",
        "rg ",
        "grep ",
        "type ",
    ];
    let mut saw_low = false;
    for segment in split_segments(&value) {
        let s = segment.trim();
        if s.is_empty() {
            continue;
        }
        if low.iter().any(|p| s == p.trim() || s.starts_with(p)) {
            saw_low = true;
        } else {
            return "mid";
        }
    }
    if saw_low { "low" } else { "mid" }
}
fn split_segments(value: &str) -> Vec<&str> {
    let bytes = value.as_bytes();
    let (mut start, mut quote, mut escaped) = (0, 0, false);
    let mut result = Vec::new();
    for (i, &c) in bytes.iter().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        if quote == b'\'' {
            if c == b'\'' {
                quote = 0;
            }
            continue;
        }
        if quote == b'"' {
            if c == b'\\' {
                escaped = true;
            } else if c == b'"' {
                quote = 0;
            }
            continue;
        }
        match c {
            b'\\' => {
                escaped = !bytes
                    .get(i + 1)
                    .is_some_and(|c| matches!(c, b'&' | b'|' | b';'));
            }
            b'\'' | b'"' => quote = c,
            b';' | b'\n' | b'|' | b'&' if !(c == b'&' && i > 0 && bytes[i - 1] == b'>') => {
                result.push(&value[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    result.push(&value[start..]);
    result
}
pub fn has_write_redirect(command: &str) -> bool {
    let bytes = command.as_bytes();
    let (mut quote, mut escaped) = (0, false);
    for (i, &c) in bytes.iter().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        if quote == b'\'' {
            if c == b'\'' {
                quote = 0;
            }
            continue;
        }
        if quote == b'"' {
            if c == b'\\' {
                escaped = true;
            } else if c == b'"' {
                quote = 0;
            }
            continue;
        }
        match c {
            b'\\' => {
                let path = i > 0
                    && !matches!(
                        bytes[i - 1],
                        b' ' | b'\t'
                            | b'\n'
                            | b'\r'
                            | b';'
                            | b'|'
                            | b'&'
                            | b'('
                            | b')'
                            | b'<'
                            | b'>'
                    );
                escaped = !(bytes.get(i + 1) == Some(&b'>') && path);
            }
            b'^' => escaped = true,
            b'\'' | b'"' => quote = c,
            b'>' if !safe_redirect_target(&command[i + 1..]) => return true,
            _ => {}
        }
    }
    false
}
fn safe_redirect_target(after: &str) -> bool {
    let target = after
        .strip_prefix('>')
        .unwrap_or(after)
        .trim_start_matches([' ', '\t']);
    if let Some(rest) = target.strip_prefix('&') {
        return rest.starts_with('-') || rest.as_bytes().first().is_some_and(u8::is_ascii_digit);
    }
    let mut target = target;
    if let Some(quote) = target.chars().next().filter(|c| *c == '\'' || *c == '"') {
        if let Some(end) = target[1..].find(quote) {
            target = &target[1..end + 1];
        } else {
            return false;
        }
    } else if let Some(end) = target.find([' ', '\t', '\r', '\n', ';', '|', '&', '>']) {
        target = &target[..end];
    }
    matches!(
        target.trim().to_lowercase().as_str(),
        "/dev/null" | "nul" | "$null"
    )
}
pub fn is_git_branch_mutation(command: &str) -> bool {
    let command = command.trim();
    let Some(found) = GIT_BRANCH.find(command) else {
        return false;
    };
    let mut words = command[found.end()..].split_whitespace();
    let mut allow = false;
    while let Some(token) = words.next() {
        if token == "--" {
            return !allow;
        }
        if token.starts_with("--") {
            let (name, value) = token
                .split_once('=')
                .map(|(a, _)| (a, true))
                .unwrap_or((token, false));
            match name {
                "--all" | "--remotes" | "--verbose" | "--show-current" | "--no-color"
                | "--omit-empty" => {}
                "--list" => allow = true,
                "--format" | "--column" | "--sort" | "--color" | "--contains" | "--no-contains"
                | "--merged" | "--no-merged" | "--points-at" => {
                    if !value && words.next().is_none() {
                        return true;
                    }
                }
                _ => return true,
            }
        } else if let Some(flags) = token.strip_prefix('-') {
            if flags.is_empty() || flags.chars().any(|c| !"aAvVrRlL".contains(c)) {
                return true;
            }
            allow = true;
        } else if !allow {
            return true;
        }
    }
    false
}

// Provider adapter masking differs from transcript/sessionlog masking. Keep its
// exact thresholds and replacement labels rather than silently sharing patterns.
static BEARER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?-u:\b)(Bearer(?-u:\s)+)[A-Za-z0-9_\-\.]{8,}").unwrap());
static SECRET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)(?-u:\b)((?:api[_-]?key|token|secret|password|auth|private[_-]?key)(?-u:\s)*[:=](?-u:\s)*["']?)([^"'\t\n\f\r ,;]{6,})(["']?)"#).unwrap()
});
static KNOWN_TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?-u:\b)(?:sk-[A-Za-z0-9_-]{16,}|ghp_[A-Za-z0-9_-]{16,}|xox[baprs]-[A-Za-z0-9_-]{16,})",
    )
    .unwrap()
});
pub fn mask_secrets(text: &str) -> String {
    let text = BEARER.replace_all(text, "${1}[REDACTED]");
    let text = SECRET.replace_all(&text, "${1}[REDACTED]${3}");
    KNOWN_TOKEN
        .replace_all(&text, "[REDACTED_TOKEN]")
        .into_owned()
}
