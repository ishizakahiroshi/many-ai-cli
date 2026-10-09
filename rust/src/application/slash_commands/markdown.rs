use super::*;
use regex::Regex;
use std::sync::OnceLock;
fn regex(pattern: &'static str) -> &'static Regex {
    static CACHE: OnceLock<Mutex<BTreeMap<&'static str, &'static Regex>>> = OnceLock::new();
    let mut cache = CACHE
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    cache
        .entry(pattern)
        .or_insert_with(|| Box::leak(Box::new(Regex::new(pattern).expect("fixed Go regex"))))
}
pub(super) fn clean(text: &str) -> String {
    let mut text = regex(r"!\[([^\]]*)\]\([^)]*\)")
        .replace_all(text, "$1")
        .into_owned();
    text = regex(r"\[([^\]]*)\]\([^)]*\)")
        .replace_all(&text, "$1")
        .into_owned();
    for _ in 0..3 {
        text = regex(r"\*\*([^*]+)\*\*|__([^_]+)__")
            .replace_all(&text, |caps: &regex::Captures<'_>| {
                caps.get(1)
                    .filter(|m| !m.as_str().is_empty())
                    .or_else(|| caps.get(2))
                    .unwrap()
                    .as_str()
                    .to_owned()
            })
            .into_owned();
    }
    text = regex(r"(?:\*([^*\n]+)\*|_([^_\n]+)_)")
        .replace_all(&text, |caps: &regex::Captures<'_>| {
            caps.get(1)
                .filter(|m| !m.as_str().is_empty())
                .or_else(|| caps.get(2))
                .unwrap()
                .as_str()
                .to_owned()
        })
        .into_owned();
    text = regex(r"`([^`]+)`").replace_all(&text, "$1").into_owned();
    text = regex(r"\[[^\]]*\]|\([^)]*\)")
        .replace_all(&text, "")
        .into_owned();
    text = text
        .replace("<br>", " ")
        .replace("<br/>", " ")
        .replace("<br />", " ");
    regex(r"[\t\n\f\r ]+").replace_all(&text, " ").trim().into()
}
pub(super) fn parse(text: &str) -> Vec<SlashCmd> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for pattern in [
        r"\|[\t\n\f\r ]*`?(/[a-z][a-z0-9_-]*)(?:[^`|\n]*)`?[\t\n\f\r ]*\|([^|\n]+)",
        r"(?m)^[ \t]*[-*][ \t]+`(/[a-z][a-z0-9_-]*)(?:[^`]*)`[ \t]*[-–—:]+[ \t]*(.+)",
        r"(?m)^[ \t]*[-*][ \t]+(/[a-z][a-z0-9_-]+)[ \t]+[-–—:]+[ \t]*(.+)",
        r"(?m)^[ \t]*`?(/[a-z][a-z0-9_-]*)(?:[\t\n\f\r ]+[^`\n]*)?`?[ \t]+([^\n]+)",
    ] {
        for captures in regex(pattern).captures_iter(text) {
            let cmd = captures[1].trim();
            let desc = clean(captures[2].trim().trim_end_matches([' ', '|']));
            let desc = desc.trim_end_matches([' ', '.']);
            if !cmd.is_empty() && seen.insert(cmd.to_owned()) {
                out.push(SlashCmd {
                    cmd: cmd.into(),
                    desc: desc.into(),
                    ..Default::default()
                });
            }
        }
    }
    out
}
