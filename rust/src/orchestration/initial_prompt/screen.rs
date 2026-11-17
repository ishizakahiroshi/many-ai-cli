//! Screen predicates from orchestration.go at the fixed Go oracle.
use crate::proto::time::Timestamp;
use std::time::Duration;

pub(super) fn collapse_whitespace(text: &str) -> String {
    // Go unicode.IsSpace uses Unicode White_Space (as does char::is_whitespace).
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// What the screen shows once the CLI is ready for a prompt, with whitespace
/// removed like the screen text it is compared with (`screen.ready_markers`).
pub fn composer_signals(provider: &str) -> Vec<String> {
    crate::profile::declared::with_declared(provider, |def| {
        def.screen
            .as_ref()
            .map(|screen| {
                screen
                    .ready_markers
                    .iter()
                    .map(|marker| collapse_whitespace(marker))
                    .collect()
            })
            .unwrap_or_default()
    })
    .unwrap_or_default()
}

pub(super) fn blocker(provider: &str, collapsed: &str) -> Option<&'static str> {
    let signals: &[&str] = match provider {
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
    };
    signals
        .iter()
        .copied()
        .find(|signal| collapsed.contains(signal))
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

pub(super) fn contains_composer(provider: &str, collapsed: &str) -> bool {
    composer_signals(provider)
        .iter()
        .any(|signal| collapsed.contains(signal))
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

/// The text still sitting in the input line, read with the declared
/// `screen.composer_prefix` / `screen.composer_end_markers`. `None` when the
/// provider declares no prefix or the input line is not on the screen.
pub(super) fn composer_text(provider: &str, lines: &[String]) -> Option<String> {
    let (prefix, ends) = crate::profile::declared::with_declared(provider, |def| {
        def.screen.as_ref().map(|s| {
            (
                s.composer_prefix.clone(),
                s.composer_end_markers
                    .iter()
                    .map(|m| collapse_whitespace(m))
                    .collect::<Vec<_>>(),
            )
        })
    })
    .flatten()?;
    let prefix = prefix.trim();
    if prefix.is_empty() {
        return None;
    }
    let start = lines
        .iter()
        .rposition(|line| line.trim().starts_with(prefix))?;
    let mut text = String::new();
    for (index, line) in lines.iter().enumerate().skip(start) {
        let line = line.trim();
        if index > start && (rule_line(line) || composer_ends(line, &ends)) {
            break;
        }
        text.push_str(if index == start {
            line.strip_prefix(prefix).unwrap_or(line)
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
fn composer_ends(line: &str, ends: &[String]) -> bool {
    let collapsed = collapse_whitespace(line);
    ends.iter().any(|end| collapsed.contains(end.as_str()))
}

/// Source launchArgStartupScreen: callers use this only before transcript
/// acceptance/progress. A composer signal wins over quoted startup wording.
pub fn launch_arg_startup_screen(
    provider: &str,
    lines: &[String],
    spawned_at: Timestamp,
    last_output: Option<Timestamp>,
    now: Timestamp,
) -> Option<&'static str> {
    let screen = collapse_whitespace(&lines.concat());
    if composer_signals(provider).is_empty()
        || screen.is_empty()
        || contains_composer(provider, &screen)
        || last_output.is_some_and(|last| {
            now.duration_since(last).unwrap_or_default() < Duration::from_secs(10)
        })
    {
        return None;
    }
    if let Some(signal) = blocker(provider, &screen) {
        return Some(signal);
    }
    (now.duration_since(spawned_at).unwrap_or_default() >= Duration::from_secs(45)).then_some("")
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
