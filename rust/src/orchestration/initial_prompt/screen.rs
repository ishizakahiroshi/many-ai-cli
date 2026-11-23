//! Screen predicates from orchestration.go at the fixed Go oracle.
//!
//! A registered AI CLI gets the same predicates from its `screen` declaration.
use crate::proto::{provider::ScreenDefinition, time::Timestamp};
use std::time::Duration;

pub(super) fn collapse_whitespace(text: &str) -> String {
    // Go unicode.IsSpace uses Unicode White_Space (as does char::is_whitespace).
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// The two providers whose screens the fixed Go oracle knows. Their tables
/// stay as they are; a declaration only speaks for any other provider.
fn fixed(provider: &str) -> bool {
    matches!(provider, "claude" | "codex")
}

pub(super) fn composer_signals(provider: &str) -> &'static [&'static str] {
    match provider {
        "codex" => &["AskCodextodoanything"],
        "claude" => &["shift+tabtocycle"],
        _ => &[],
    }
}

fn fixed_blockers(provider: &str) -> &'static [&'static str] {
    match provider {
        "codex" => &[
            "Doyoutrustthecontentsofthisdirectory",
            "Updateavailable!",
            "Pressentertocontinue",
            "Trustthisfolder?Codexcanread",
        ],
        "claude" => &[
            "Isthisaprojectyoucreatedoroneyoutrust?",
            "Yes,Itrustthisfolder",
            "AllowexternalCLAUDE.mdfileimports?",
        ],
        _ => &[],
    }
}

pub(super) fn folder_trust(signal: &str) -> bool {
    matches!(
        signal,
        "Doyoutrustthecontentsofthisdirectory"
            | "Trustthisfolder?Codexcanread"
            | "Isthisaprojectyoucreatedoroneyoutrust?"
            | "Yes,Itrustthisfolder"
    )
}

/// What one provider's screen shows when it is ready for a prompt, when a
/// startup screen holds it, and where its input line is. Claude Code and Codex
/// use the fixed tables; every other provider (a registered AI CLI such as
/// Devin CLI) uses its definition's `screen` declaration, so the startup-wait
/// notice and the "prompt still in the input box" check work for it as well
/// instead of silently doing nothing. Markers are kept without whitespace,
/// exactly like the screen text they are compared with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScreenSignals {
    provider: String,
    ready: Vec<String>,
    blockers: Vec<String>,
    composer_prefix: String,
    composer_ends: Vec<String>,
}
impl ScreenSignals {
    /// Only the fixed tables (a provider without a declaration).
    pub fn builtin(provider: &str) -> Self {
        Self {
            provider: provider.into(),
            ..Self::default()
        }
    }
    /// The fixed tables for Claude Code and Codex, the declaration otherwise.
    pub fn declared(provider: &str, screen: Option<&ScreenDefinition>) -> Self {
        let mut out = Self::builtin(provider);
        let Some(screen) = screen.filter(|_| !fixed(provider)) else {
            return out;
        };
        let markers = |values: &[String]| {
            values
                .iter()
                .map(|value| collapse_whitespace(value))
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
        };
        out.ready = markers(&screen.ready_markers);
        out.blockers = markers(&screen.startup_blockers);
        out.composer_prefix = screen.composer_prefix.trim().to_owned();
        out.composer_ends = markers(&screen.composer_end_markers);
        out
    }
    /// The provider's declaration in the Hub's current registry. A provider
    /// the registry does not hold (or no registry) has only the fixed tables.
    pub fn from_registry(
        provider: &str,
        registry: Option<&crate::profile::registry::Registry>,
    ) -> Self {
        let definition = registry.and_then(|registry| registry.lookup(provider));
        Self::declared(
            provider,
            definition
                .as_ref()
                .and_then(|definition| definition.definition.screen.as_ref()),
        )
    }
    /// Whether anything on the screen can say the input box is there.
    pub(super) fn has_composer_signal(&self) -> bool {
        !composer_signals(&self.provider).is_empty() || !self.ready.is_empty()
    }
    pub(super) fn contains_composer(&self, collapsed: &str) -> bool {
        composer_signals(&self.provider)
            .iter()
            .any(|signal| collapsed.contains(signal))
            || self
                .ready
                .iter()
                .any(|signal| collapsed.contains(signal.as_str()))
    }
    pub(super) fn blocker(&self, collapsed: &str) -> Option<String> {
        fixed_blockers(&self.provider)
            .iter()
            .find(|signal| collapsed.contains(*signal))
            .map(|signal| (*signal).to_owned())
            .or_else(|| {
                self.blockers
                    .iter()
                    .find(|signal| collapsed.contains(signal.as_str()))
                    .cloned()
            })
    }
    /// The text still sitting in the input line. `None` when the provider has
    /// no input-line reader or the input line is not on the screen.
    pub(super) fn composer_text(&self, lines: &[String]) -> Option<String> {
        if self.provider == "claude" {
            return composer_lines(lines, "❯", claude_footer);
        }
        if fixed(&self.provider) || self.composer_prefix.is_empty() {
            return None;
        }
        composer_lines(lines, &self.composer_prefix, |line| {
            let collapsed = collapse_whitespace(line);
            self.composer_ends
                .iter()
                .any(|end| collapsed.contains(end.as_str()))
        })
    }
}

pub(super) fn echo_marker(prompt: &str) -> String {
    let text = collapse_whitespace(prompt);
    let skip = text.chars().count().saturating_sub(16);
    text.chars().skip(skip).collect()
}

/// The "[Pasted text #N ...]" stand-in a TUI shows for a long paste.
pub(super) fn has_paste_placeholder(collapsed: &str) -> bool {
    collapsed.match_indices("[Pastedtext#").any(|(at, prefix)| {
        collapsed.as_bytes()[at + prefix.len()..]
            .first()
            .is_some_and(u8::is_ascii_digit)
    })
}

pub(super) fn echo_visible(collapsed: &str, marker: &str) -> bool {
    marker.is_empty() || collapsed.contains(marker) || has_paste_placeholder(collapsed)
}

fn composer_lines(lines: &[String], prefix: &str, ends: impl Fn(&str) -> bool) -> Option<String> {
    let start = lines
        .iter()
        .rposition(|line| line.trim().starts_with(prefix))?;
    let mut text = String::new();
    for (index, line) in lines.iter().enumerate().skip(start) {
        let line = line.trim();
        if index > start && (rule_line(line) || ends(line)) {
            break;
        }
        text.push_str(if index == start {
            line.strip_prefix(prefix).expect("selected composer prefix")
        } else {
            line
        });
    }
    Some(collapse_whitespace(&text))
}

fn rule_line(line: &str) -> bool {
    line.chars().all(|c| matches!(c, '─' | '━' | '═' | ' '))
        && line.chars().filter(|c| *c != ' ').count() >= 8
}
fn claude_footer(line: &str) -> bool {
    line.starts_with('⏵') || collapse_whitespace(line).contains("shift+tabtocycle")
}

/// Source launchArgStartupScreen: callers use this only before transcript
/// acceptance/progress. A composer signal wins over quoted startup wording.
pub fn launch_arg_startup_screen(
    signals: &ScreenSignals,
    lines: &[String],
    spawned_at: Timestamp,
    last_output: Option<Timestamp>,
    now: Timestamp,
) -> Option<String> {
    let screen = collapse_whitespace(&lines.concat());
    if !signals.has_composer_signal()
        || screen.is_empty()
        || signals.contains_composer(&screen)
        || last_output.is_some_and(|last| {
            now.duration_since(last).unwrap_or_default() < Duration::from_secs(10)
        })
    {
        return None;
    }
    if let Some(signal) = signals.blocker(&screen) {
        return Some(signal);
    }
    (now.duration_since(spawned_at).unwrap_or_default() >= Duration::from_secs(45))
        .then(String::new)
}

pub(super) fn blocked_detail(signal: &str) -> String {
    if folder_trust(signal) {
        format!(
            "child TUI is asking whether to trust its working folder ({signal}); the initial prompt was NOT delivered. Ask the user to open the child session and choose Yes (Codex: Trust and continue), then send the instructions again with orchestrate send"
        )
    } else {
        format!(
            "child TUI is waiting on a modal ({signal}) and never reached its composer; the initial prompt was NOT delivered"
        )
    }
}
