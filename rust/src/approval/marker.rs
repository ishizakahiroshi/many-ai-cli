//! Marker extraction, corruption guards and provider feature/source table.
use super::identity::{digest, normalize, strip_ansi};
use crate::terminal::vt::VtBuffer;
use regex::Regex;
use std::{collections::HashSet, sync::LazyLock};
pub const OPEN: &str = concat!("[", "MANY-AI-CLI", "]");
pub const CLOSE: &str = concat!("[/", "MANY-AI-CLI", "]");
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Marker {
    pub block: String,
    pub sig: String,
}
fn marker(block: String) -> Marker {
    let sig = digest(&normalize(&block));
    Marker { block, sig }
}
pub fn extract(text: &str) -> Option<Marker> {
    let end = text.rfind(CLOSE)?;
    let start = text[..end].rfind(OPEN)?;
    Some(marker(text[start..end + CLOSE.len()].into()))
}
pub fn extract_vt(vt: &VtBuffer) -> Option<Marker> {
    let lines = vt.tail_lines_with_scrollback(300);
    extract(&lines.join("\n")).or_else(|| reconstruct_openless(&lines, vt.rows()))
}
pub fn reconstruct_openless(lines: &[String], rows: usize) -> Option<Marker> {
    if rows == 0 || rows > lines.len() {
        return None;
    }
    let screen_start = lines.len() - rows;
    let close = lines
        .iter()
        .enumerate()
        .rev()
        .find(|(i, l)| *i >= screen_start && l.contains(CLOSE))
        .map(|(i, _)| i)?;
    if lines.len() - 1 - close > 16 || lines[close + 1..].iter().any(|s| s.contains(OPEN)) {
        return None;
    }
    let mut body = lines[screen_start..=close].to_vec();
    let last = body.last_mut()?;
    if let Some(i) = last.find(CLOSE) {
        last.truncate(i + CLOSE.len());
    }
    let first = body.iter().position(|s| !s.trim().is_empty())?;
    if body.len() - first < 8 {
        return None;
    }
    Some(marker(format!("{OPEN}\n{}", body[first..].join("\n"))))
}
static NUMBER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[ \t]*([0-9]{1,2})\.[ \t]+[^\t\n\f\r ]").unwrap());
static HEADING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[ \t]*[QＱ][ \t]*[0-9]{1,2}(?:[ \t]|$)").unwrap());
static YESNO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[（(](?-u:\s)*[YＹ](?-u:\s)*[:：](?-u:\s)*1(?-u:\s)*[/／](?-u:\s)*[NＮ](?-u:\s)*[:：](?-u:\s)*0(?-u:\s)*[）)]").unwrap()
});
static BOX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\x{2500}-\x{257F}]{3,}").unwrap());
pub fn classify(block: &str) -> &'static str {
    let clean = strip_ansi(block);
    if clean.matches(OPEN).count() > 1 || clean.matches(CLOSE).count() > 1 {
        return "marker_leak";
    }
    for line in clean.split('\n') {
        let only_rule = line
            .chars()
            .all(|c| matches!(c, '\u{2500}'..='\u{257f}' | ' ' | '\t' | '\r'))
            && line.chars().any(|c| matches!(c, '\u{2500}'..='\u{257f}'));
        if !only_rule && BOX.is_match(line) {
            return "box_rule";
        }
    }
    if YESNO.is_match(&clean) {
        return "";
    }
    let mut groups = Vec::new();
    let mut current = Vec::new();
    for line in clean.split('\n') {
        if HEADING.is_match(line) {
            if !current.is_empty() {
                groups.push(std::mem::take(&mut current));
            }
            continue;
        }
        if let Some(c) = NUMBER.captures(line) {
            current.push(c[1].parse::<u8>().unwrap());
        }
    }
    if !current.is_empty() {
        groups.push(current);
    }
    if groups.is_empty() {
        return "";
    }
    if !groups.iter().flatten().any(|n| *n == 1) {
        return "option_start";
    }
    for group in groups {
        if let Some(start) = group.iter().position(|n| *n == 1) {
            let mut seen = HashSet::new();
            for n in &group[start..] {
                if !seen.insert(n) {
                    return "duplicate_option";
                }
            }
        }
    }
    ""
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeatureSource {
    None,
    Native,
    Derived,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProviderFeatures {
    pub structured_transcript: FeatureSource,
    pub approval_marker: FeatureSource,
}
pub fn provider_features(provider: &str) -> ProviderFeatures {
    match provider {
        "claude" | "codex" => ProviderFeatures {
            structured_transcript: FeatureSource::Native,
            approval_marker: FeatureSource::Native,
        },
        "command-code" => ProviderFeatures {
            structured_transcript: FeatureSource::Native,
            approval_marker: FeatureSource::Derived,
        },
        _ => ProviderFeatures {
            structured_transcript: FeatureSource::None,
            approval_marker: FeatureSource::Derived,
        },
    }
}
#[derive(Default, Clone)]
pub struct TranscriptSource {
    pub resolved_path: std::path::PathBuf,
    pub miss_streak: usize,
    pub codex_thread_ambiguous: bool,
}
impl TranscriptSource {
    pub fn is_transcript(&self, provider: &str) -> bool {
        provider_features(provider).approval_marker == FeatureSource::Native
            && !self.resolved_path.as_os_str().is_empty()
            && self.miss_streak < 3
            && !self.codex_thread_ambiguous
    }
    /// Returns true only on transition into fallback, to avoid warning spam.
    pub fn miss(&mut self) -> bool {
        self.miss_streak = self.miss_streak.saturating_add(1);
        self.miss_streak == 3
    }
    pub fn resolved(&mut self, path: std::path::PathBuf) {
        self.resolved_path = path;
        self.miss_streak = 0;
    }
}
