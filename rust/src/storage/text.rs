//! Equivalent sessionlog masking and visible/noise text used by the repository.
use super::*;
use regex::Regex;
use std::sync::LazyLock;
static SECRETS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
    r"(?i)([A-Z_]*API_KEY=)[^\t\n\f\r ]+",
    r#"(?i)((?:[A-Z0-9]+_)?(?:PASSWORD|PASSWD|PWD|SECRET|ACCESS_KEY|SECRET_KEY|AUTH_TOKEN|API_TOKEN|ACCESS_TOKEN)(?:\\?["'])?[\t\n\f\r ]*[=:][\t\n\f\r ]*(?:\\?["'])?)[^\t\n\f\r ]{6,}"#,
    r"sk-(?:ant-)?[A-Za-z0-9_\-]{20,}", r"xai-[A-Za-z0-9_\-]{20,}", r"gsk_[A-Za-z0-9_\-]{20,}",
    r"(?:cohere|mistral)[_-](?:api[_-])?key[_-][A-Za-z0-9_\-]{16,}",
    r"(?:ghp_|gho_|ghu_|ghs_|ghr_|github_pat_)[A-Za-z0-9_]{20,}",
    r"glpat-[A-Za-z0-9_\-]{20,}", r"xox[abprs]-[A-Za-z0-9\-]{20,}",
    r"AIza[A-Za-z0-9_\-]{20,}", r"hf_[A-Za-z0-9]{20,}", r"(?i)(Bearer )[^\t\n\f\r ]{8,}",
    r"(?:AKIA|ASIA|AROA)[A-Z0-9]{16}",
    r"(?s)-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----.*?-----END [A-Z0-9 ]*PRIVATE KEY-----",
].iter().map(|p| Regex::new(p).expect("fixed secret regex")).collect()
});
static URL_SECRET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"([a-zA-Z][a-zA-Z0-9+.\-]*://[^\t\n\f\r :/@]+:)([^\t\n\f\r :/@]+)(@)").unwrap()
});
static LETTER_OR_NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\p{L}\p{N}]").unwrap());
static ANSI: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"\x1b\][^\x07]*(?:\x07|\x1b\\)",
        r"\x1b\[[0-?]*[ -/]*[@-~]",
        r"\x1b[@-_]",
    ]
    .iter()
    .map(|p| Regex::new(p).unwrap())
    .collect()
});
/// Exact sessionlog.MaskSecrets thresholds and replacement convention.
pub fn mask_secrets(value: &str) -> String {
    let mut out = value.to_owned();
    for regex in SECRETS.iter() {
        out = regex
            .replace_all(&out, |captures: &regex::Captures<'_>| {
                if let Some(prefix) = captures.get(1) {
                    let complete = captures.get(0).unwrap();
                    format!(
                        "{}***",
                        &complete.as_str()[..prefix.end() - complete.start()]
                    )
                } else {
                    "***".to_owned()
                }
            })
            .into_owned();
    }
    URL_SECRET.replace_all(&out, "${1}***${3}").into_owned()
}
/// Byte-preserving sessionlog.MaskSecrets. Go regexp treats each invalid UTF-8
/// byte as one U+FFFD rune while capture offsets still address the original
/// bytes. Rebuild the projection/index map after each sequential regex pass;
/// never substitute sentinels or duplicate the pattern table.
pub fn mask_secret_bytes(value: &[u8]) -> Vec<u8> {
    let mut out = value.to_vec();
    for regex in SECRETS.iter() {
        out = replace_raw_spans(&out, regex, false);
    }
    replace_raw_spans(&out, &URL_SECRET, true)
}
fn raw_span_map(raw: &[u8]) -> Vec<usize> {
    let mut map = vec![0];
    let mut offset = 0;
    while offset < raw.len() {
        let valid = match std::str::from_utf8(&raw[offset..]) {
            Ok(text) => text.len(),
            Err(error) => error.valid_up_to(),
        };
        if valid > 0 {
            map.extend(offset + 1..=offset + valid);
            offset += valid;
        } else {
            // The Go projection emits three UTF-8 bytes for exactly one invalid
            // source byte. Regex Unicode capture boundaries never split them.
            map.extend([offset, offset, offset + 1]);
            offset += 1;
        }
    }
    map
}
fn replace_raw_spans(raw: &[u8], regex: &Regex, url_password: bool) -> Vec<u8> {
    let projected = crate::proto::wire::go_utf8_lossy(raw);
    let map = raw_span_map(raw);
    debug_assert_eq!(map.len(), projected.len() + 1);
    let mut result = Vec::with_capacity(raw.len());
    let mut cursor = 0;
    for captures in regex.captures_iter(&projected) {
        let whole = captures.get(0).expect("matched regex has capture zero");
        let start = map[whole.start()];
        let end = map[whole.end()];
        result.extend_from_slice(&raw[cursor..start]);
        if let Some(prefix) = captures.get(1) {
            result.extend_from_slice(&raw[start..map[prefix.end()]]);
        }
        result.extend_from_slice(b"***");
        if url_password {
            let suffix = captures.get(3).expect("URL password pattern has suffix");
            result.extend_from_slice(&raw[map[suffix.start()]..map[suffix.end()]]);
        }
        cursor = end;
    }
    result.extend_from_slice(&raw[cursor..]);
    result
}
pub(super) fn visible(value: &str) -> String {
    let mut out = value.to_owned();
    for re in ANSI.iter() {
        out = re.replace_all(&out, "").into_owned();
    }
    out.replace("\r\n", "\n").replace('\r', "\n").chars().filter(|c| !matches!(*c, '\u{0}'..='\u{8}' | '\u{b}' | '\u{c}' | '\u{e}'..='\u{1f}' | '\u{7f}')).collect()
}
pub(super) fn noise(value: &str) -> bool {
    let t = value.trim();
    if t.is_empty() || matches!(t, "Boot" | "Boo" | "Bo" | "Thinking" | "Working") {
        return true;
    }
    if t.chars().count() <= 2 && !LETTER_OR_NUMBER.is_match(t) {
        return true;
    }
    t.lines().filter(|l| !l.trim().is_empty()).all(|line| {
        let lower = line.to_lowercase();
        let spinner_count = line
            .chars()
            .filter(|c| matches!(*c, '\u{2722}'..='\u{273f}' | '\u{2800}'..='\u{28ff}'))
            .count();
        lower.contains("esc to interrupt")
            || lower.contains("shift+tab to cycle")
            || lower.contains("shift + tab to cycle")
            || (line.contains('↑') && line.contains('↓'))
            || (lower.contains("thinking") && spinner_count > 0)
            || spinner_count >= 2
    })
}
pub(super) fn value(event: &HistoryEvent, key: &str) -> String {
    match event.0.get(key) {
        None | Some(serde_json::Value::Null) => String::new(),
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(v) => go_display(v),
    }
}
fn go_display(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "<nil>".to_owned(),
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(a) => format!(
            "[{}]",
            a.iter().map(go_display).collect::<Vec<_>>().join(" ")
        ),
        serde_json::Value::Object(m) => format!(
            "map[{}]",
            m.iter()
                .map(|(k, v)| format!("{k}:{}", go_display(v)))
                .collect::<Vec<_>>()
                .join(" ")
        ),
        _ => value.to_string(),
    }
}
pub(super) fn trim(value: &str, n: usize) -> String {
    value.chars().take(n).collect()
}
pub(super) fn tags(values: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for v in values {
        let v = trim(v.trim(), 32);
        if !v.is_empty() && !out.contains(&v) {
            out.push(v);
        }
        if out.len() == 12 {
            break;
        }
    }
    out
}
pub(super) fn coalesce(messages: Vec<ChatMessage>) -> Vec<ChatMessage> {
    let mut out = Vec::<ChatMessage>::new();
    for msg in messages {
        if msg.role == "ai"
            && msg.kind == "text"
            && let Some(last) = out
                .last_mut()
                .filter(|m| m.role == "ai" && m.kind == "text")
        {
            if !last.raw_text.is_empty() {
                last.raw_text.push('\n');
            }
            last.raw_text.push_str(&msg.raw_text);
            if !last.normalized_text.is_empty() {
                last.normalized_text.push('\n');
            }
            last.normalized_text.push_str(&msg.normalized_text);
            continue;
        }
        out.push(msg);
    }
    out.retain_mut(|m| {
        if m.role == "ai" && m.kind == "text" {
            m.raw_text = m.raw_text.trim().to_owned();
            m.normalized_text = m.normalized_text.trim().to_owned();
            !m.raw_text.is_empty()
        } else {
            true
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sessionlog_masking_preserves_exact_thresholds_and_context() {
        let token = "A".repeat(25);
        for prefix in [
            "sk-",
            "sk-ant-",
            "xai-",
            "gsk_",
            "cohere-api-key-",
            "mistral_api_key_",
            "ghp_",
            "gho_",
            "ghu_",
            "ghs_",
            "ghr_",
            "github_pat_",
            "glpat-",
            "xoxb-",
            "AIza",
            "hf_",
        ] {
            assert_eq!(mask_secrets(&format!("{prefix}{token}")), "***", "{prefix}");
        }
        for (input, expected) in [
            ("DB_PASSWORD=synthetic-value", "DB_PASSWORD=***"),
            ("PASSWORD=abcdef", "PASSWORD=***"),
            ("PWD=abc", "PWD=abc"),
            ("client_SECRET: abcdef123456", "client_SECRET: ***"),
            ("ACCESS_TOKEN=abcdefghijklmno", "ACCESS_TOKEN=***"),
            (
                r#"{"PASSWORD":"syntheticQuotedValue"}"#,
                r#"{"PASSWORD":"***"#,
            ),
            (
                "postgres://user:synthetic@host:5432/db",
                "postgres://user:***@host:5432/db",
            ),
            ("Bearer abcdefgh", "Bearer ***"),
            ("Bearer abcdefg", "Bearer abcdefg"),
            ("OPENAI_API_KEY=value", "OPENAI_API_KEY=***"),
            ("PASSWORD=abcdef\u{a0}suffix", "PASSWORD=***"),
            (
                "https://example.com/path?q=1",
                "https://example.com/path?q=1",
            ),
            (
                "please reset your password soon",
                "please reset your password soon",
            ),
        ] {
            assert_eq!(mask_secrets(input), expected);
        }
        // Delimiters are built at runtime so scanner fixtures are not mistaken
        // for embedded PEM material. The body is deliberately not key data.
        let pem = format!(
            "-----{} RSA PRIVATE {}-----\nsynthetic-only-not-a-key\n-----{} RSA PRIVATE {}-----",
            "BEGIN", "KEY", "END", "KEY"
        );
        assert_eq!(mask_secrets(&pem), "***");
    }
}
