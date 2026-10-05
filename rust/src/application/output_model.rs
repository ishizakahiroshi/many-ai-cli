//! Fixed Go PTY model detection. Session owns scan budget and update admission.
use regex::Regex;
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DetectedModel {
    pub model: String,
    pub effort: String,
}
fn re(pattern: &'static str) -> &'static Regex {
    static CACHE: OnceLock<Mutex<HashMap<&'static str, &'static Regex>>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(Default::default).lock().unwrap();
    cache
        .entry(pattern)
        .or_insert_with(|| Box::leak(Box::new(Regex::new(pattern).expect("fixed Go pattern"))))
}
fn trim(s: &str) -> &str {
    s.trim_matches(|c:char|matches!(c,'\t'..='\r'|' '|'\u{85}'|'\u{a0}'|'\u{1680}'|'\u{2000}'..='\u{200a}'|'\u{2028}'|'\u{2029}'|'\u{202f}'|'\u{205f}'|'\u{3000}'))
}
fn detected(model: &str, effort: &str) -> DetectedModel {
    DetectedModel {
        model: model.into(),
        effort: effort.into(),
    }
}
pub const INITIAL_SCAN_MAX_BYTES: usize = 256 * 1024;
pub fn initial_provider(provider: &str) -> bool {
    matches!(provider, "claude" | "codex" | "copilot" | "cursor-agent")
}
pub fn split_claude(line: &str) -> DetectedModel {
    let mut rest = trim(line);
    if let Some((before, _)) = rest.split_once('·') {
        rest = trim(before);
    }
    let pattern = re(r"[ \t\n\f\r]+with[ \t\n\f\r]+([^ \t\n\f\r]+)[ \t\n\f\r]+effort$");
    if let Some(m) = pattern.captures(rest) {
        return detected(trim(&pattern.replace_all(rest, "")), &m[1]);
    }
    detected(rest, "")
}
pub fn change(provider: &str, chunk: &[u8], clean: &str) -> Option<DetectedModel> {
    if ![b"Set model to ".as_slice(), b"Model changed to ".as_slice()]
        .iter()
        .any(|token| chunk.windows(token.len()).any(|w| w == *token))
    {
        return None;
    }
    let pattern = match provider {
        "claude" => r"Set model to ([^\r\n]+)",
        "codex" => r"Model changed to ([^\r\n]+)",
        _ => return None,
    };
    let m = re(pattern).captures(clean)?;
    let value = trim(&m[1]);
    if value.is_empty() {
        return None;
    }
    Some(if provider == "claude" {
        split_claude(value)
    } else {
        detected(value, "")
    })
}
pub fn banner(provider: &str, cwd: &str, lines: &[String]) -> DetectedModel {
    match provider {
        "claude" => {
            for (i, line) in lines.iter().enumerate() {
                if !re(r"Claude Code[ \t\n\f\r]+v[0-9]").is_match(line) {
                    continue;
                }
                for next in lines.iter().skip(i + 1).take(2) {
                    let without = re(r"^[ \t\n\f\r\x{2580}-\x{259F}]+").replace_all(next, "");
                    let rest = trim(&without);
                    if rest.is_empty() {
                        continue;
                    }
                    let result = split_claude(rest);
                    if !result.model.is_empty() {
                        return result;
                    }
                }
            }
        }
        "codex" => {
            for line in lines {
                if let Some(m) =
                    re(r"model:[ \t\n\f\r]+(.+?)[ \t\n\f\r]+/model to change").captures(line)
                {
                    let name = trim(&m[1]);
                    if !name.is_empty() && !name.eq_ignore_ascii_case("loading") {
                        return detected(name, "");
                    }
                }
            }
        }
        "copilot" => {
            for line in lines.iter().rev() {
                let line = trim(line);
                if line.is_empty() {
                    continue;
                }
                let split: Vec<_> = re(r"[ \t\n\f\r]{3,}").split(line).collect();
                let mut seg = trim(split.last().unwrap());
                let mut effort = "";
                let suffix = re(r"[ \t\n\f\r]+·[ \t\n\f\r]+(low|medium|high|xhigh)$");
                let without;
                let m = suffix.captures(seg);
                if let Some(ref m) = m {
                    effort = m.get(1).unwrap().as_str();
                    without = suffix.replace_all(seg, "");
                    seg = trim(&without);
                }
                if seg == "Auto"
                    || (seg.len() <= 40 && re(r"^[A-Za-z][A-Za-z0-9_.\- ()]*[0-9]").is_match(seg))
                {
                    return detected(seg, effort);
                }
                return DetectedModel::default();
            }
        }
        "cursor-agent" => {
            if cwd.is_empty() {
                return DetectedModel::default();
            }
            for (i, line) in lines.iter().enumerate() {
                let t = trim(line);
                if !t.starts_with(&format!("{cwd} · ")) && t != cwd {
                    continue;
                }
                for above in lines[..i].iter().rev() {
                    let above = trim(above);
                    if above.is_empty() {
                        continue;
                    }
                    let above = re(r"[ \t\n\f\r]+·[ \t\n\f\r]+[0-9]+(?:\.[0-9]+)?%$")
                        .replace_all(above, "");
                    if !above.is_empty() && !above.contains(['→', '❯']) && above.len() <= 60 {
                        return detected(&above, "");
                    }
                    break;
                }
            }
        }
        _ => {}
    }
    DetectedModel::default()
}
#[cfg(test)]
#[path = "output_model/tests.rs"]
mod tests;
