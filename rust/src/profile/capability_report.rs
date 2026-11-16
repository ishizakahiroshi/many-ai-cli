//! What a provider can and cannot do, computed from its declarations in one
//! place. `provider test`, the registration screen and the registry summary all
//! read this, so they never disagree.
//!
//! A feature is `available` when the declaration for it is present,
//! `unavailable` when it is not declared (with the reason, so the person or AI
//! registering the CLI knows what to add), and `unsupported` when it needs
//! vendor-specific code that a declaration cannot supply (an honest "no", not a
//! missing setting).
use crate::proto::provider::{CapabilitySummary, Definition};
use serde::Serialize;

pub const AVAILABLE: &str = "available";
pub const UNAVAILABLE: &str = "unavailable";
pub const UNSUPPORTED: &str = "unsupported";

/// One line of the list. `key` is stable (the screen translates it); `detail`
/// is an English reason or a short description of the declared value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CapabilityLine {
    pub key: &'static str,
    pub state: &'static str,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub detail: String,
}

fn line(key: &'static str, state: &'static str, detail: impl Into<String>) -> CapabilityLine {
    CapabilityLine {
        key,
        state,
        detail: detail.into(),
    }
}
fn available(s: &str) -> bool {
    !matches!(s, "" | "none" | "unsupported")
}
fn declared(on: bool, key: &'static str, yes: &str, no: &str) -> CapabilityLine {
    if on {
        line(key, AVAILABLE, yes)
    } else {
        line(key, UNAVAILABLE, no)
    }
}

/// The ten flags the registry summary carries (the dashboard's cache reads them).
pub fn summary(d: &Definition) -> CapabilitySummary {
    CapabilitySummary {
        launch: d.launch.is_some(),
        models: d.models.is_some(),
        effort: d.launch.as_ref().is_some_and(|l| !l.effort_args.is_empty()),
        headless: d.launch.as_ref().is_some_and(|l| l.headless.is_some()),
        approval: available(&d.adapters.approval),
        transcript: available(&d.adapters.transcript)
            || d.transcript.as_ref().is_some_and(|t| t.reader.is_some()),
        usage: available(&d.adapters.usage),
        subscription: available(&d.adapters.subscription),
        permissions: available(&d.adapters.permissions),
        subagents: available(&d.adapters.subagents),
    }
}

/// The list for `provider test` and the registration screen. `d` is the
/// definition with its declarations resolved (`profile::declared::resolve`).
pub fn report(d: &Definition) -> Vec<CapabilityLine> {
    let acp = !crate::profile::declared::automation_allowed(d);
    let launch = d.launch.as_ref();
    let mut out = vec![];
    out.push(declared(
        launch.is_some_and(|l| !l.executable.is_empty() || !l.executable_candidates.is_empty()),
        "launch",
        "starts an interactive session",
        "no launch command is declared",
    ));
    out.push(declared(
        launch.is_some_and(|l| l.headless.is_some()),
        "headless",
        "can run as a non-interactive child",
        "no headless arguments are declared (--headless-arg / --prompt-via)",
    ));
    let models = d.models.as_ref();
    out.push(declared(
        models.is_some_and(|m| !m.items.is_empty() || m.list.is_some() || m.allow_custom),
        "models",
        "a model list or free model name is declared",
        "no model list is declared (models.items or models.list)",
    ));
    out.push(declared(
        launch.is_some_and(|l| !l.model_args.is_empty()),
        "model_flag",
        "a chosen model is passed as a launch flag",
        "no model flag is declared (launch.model_args); a chosen model is not passed",
    ));
    out.push(declared(
        launch.is_some_and(|l| !l.effort_args.is_empty()),
        "effort",
        "effort levels are passed as a launch flag",
        "no effort flag is declared (launch.effort_args)",
    ));
    let approval_declared = d.approval.is_some()
        || !d.approval_pattern_source.is_empty()
        || available(&d.adapters.approval);
    out.push(declared(
        approval_declared,
        "approval",
        "approval prompts are detected from the declared wording",
        "no approval wording is declared; only the generic Yes/No/Allow/Deny prompts are detected",
    ));
    let transcript = d.transcript.as_ref();
    out.push(declared(
        transcript.is_some_and(|t| !t.root.is_empty() || !t.file.is_empty())
            || available(&d.adapters.transcript),
        "transcript",
        "the conversation file is located",
        "no conversation file location is declared (transcript.root)",
    ));
    out.push(declared(
        transcript.is_some_and(|t| t.reader.is_some()) || available(&d.adapters.transcript),
        "chat_view",
        "the conversation is shown as a chat",
        "no conversation file format is declared (transcript.reader)",
    ));
    let slash = d.slash_commands.as_ref();
    out.push(declared(
        slash.is_some_and(|s| !s.source.is_empty() || !s.items.is_empty()),
        "slash_commands",
        "slash command candidates are declared",
        "no slash command source is declared (slash_commands)",
    ));
    out.push(declared(
        d.permissions.as_ref().is_some_and(|p| !p.modes.is_empty()),
        "permissions",
        "permission modes map to launch flags",
        "no permission mode is declared (permissions.modes)",
    ));
    out.push(declared(
        d.links.as_ref().is_some_and(|l| !l.usage_url.is_empty()),
        "usage_link",
        "a usage page link is shown",
        "no usage page URL is declared (links.usage_url)",
    ));
    out.push(declared(
        d.login.as_ref().is_some_and(|l| !l.args.is_empty()),
        "login",
        "the login command can be started from the dashboard",
        "no login arguments are declared (login.args)",
    ));
    if acp {
        out.push(line(
            "automation",
            UNAVAILABLE,
            "reached over ACP: driven by a person only, so children, relay, routines, boards and automatic approval cannot use it",
        ));
    } else {
        out.push(declared(
            crate::profile::declared::routines_allowed(d),
            "automation",
            "children, relay and routines may use it",
            "routines are switched off for it (routines.enabled: false)",
        ));
    }
    // Needs vendor-specific code: a declaration cannot provide these.
    for (key, adapter, what) in [
        (
            "usage_meter",
            &d.adapters.usage,
            "reading the remaining quota needs code written for that vendor",
        ),
        (
            "subscription_profiles",
            &d.adapters.subscription,
            "switching subscription profiles needs code written for that vendor",
        ),
        (
            "subagent_tree",
            &d.adapters.subagents,
            "showing sub-agents needs code written for that vendor",
        ),
    ] {
        out.push(if available(adapter) {
            line(key, AVAILABLE, "provided by built-in support")
        } else {
            line(key, UNSUPPORTED, what)
        });
    }
    out
}

/// Plain text for the terminal: one line per feature.
pub fn render_text(lines: &[CapabilityLine]) -> String {
    let mut out = String::new();
    for l in lines {
        let mark = match l.state {
            AVAILABLE => "available  ",
            UNAVAILABLE => "unavailable",
            _ => "unsupported",
        };
        out.push_str(&format!("  {mark}  {}", l.key));
        if !l.detail.is_empty() {
            out.push_str(&format!(" - {}", l.detail));
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::provider::*;

    fn state(lines: &[CapabilityLine], key: &str) -> &'static str {
        lines.iter().find(|l| l.key == key).unwrap().state
    }

    #[test]
    fn a_bare_registered_cli_can_launch_and_nothing_else() {
        let def = Definition {
            id: "fixture".into(),
            launch: Some(LaunchDefinition {
                executable: "fixture-agent".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let lines = report(&def);
        assert_eq!(state(&lines, "launch"), AVAILABLE);
        for key in [
            "headless",
            "models",
            "approval",
            "transcript",
            "slash_commands",
            "permissions",
            "usage_link",
        ] {
            assert_eq!(state(&lines, key), UNAVAILABLE, "{key}");
        }
        assert_eq!(state(&lines, "automation"), AVAILABLE);
        for key in ["usage_meter", "subscription_profiles", "subagent_tree"] {
            assert_eq!(state(&lines, key), UNSUPPORTED, "{key}");
        }
    }

    #[test]
    fn declarations_turn_a_line_to_available() {
        let def = Definition {
            id: "fixture".into(),
            launch: Some(LaunchDefinition {
                executable: "fixture-agent".into(),
                headless: Some(HeadlessDefinition::default()),
                ..Default::default()
            }),
            slash_commands: Some(SlashCommandsDefinition {
                items: vec![SlashCommandItem {
                    cmd: "/help".into(),
                    desc: String::new(),
                }],
                ..Default::default()
            }),
            links: Some(LinksDefinition {
                usage_url: "https://example.test/usage".into(),
            }),
            ..Default::default()
        };
        let lines = report(&def);
        assert_eq!(state(&lines, "headless"), AVAILABLE);
        assert_eq!(state(&lines, "slash_commands"), AVAILABLE);
        assert_eq!(state(&lines, "usage_link"), AVAILABLE);
        assert_eq!(state(&lines, "approval"), UNAVAILABLE);
    }

    #[test]
    fn an_acp_provider_is_never_available_to_automation() {
        let def = Definition {
            id: "fixture".into(),
            connection: "acp".into(),
            launch: Some(LaunchDefinition {
                executable: "fixture-agent".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(state(&report(&def), "automation"), UNAVAILABLE);
    }

    #[test]
    fn built_in_support_makes_the_vendor_specific_lines_available() {
        let mut def = Definition::default();
        def.adapters.usage = "claude".into();
        assert_eq!(state(&report(&def), "usage_meter"), AVAILABLE);
    }

    #[test]
    fn every_built_in_reports_launch_and_the_summary_matches() {
        for id in BUILTIN_PROVIDER_IDS {
            let def = crate::profile::declared::builtin(id).unwrap();
            let lines = report(&def);
            assert_eq!(state(&lines, "launch"), AVAILABLE, "{id}");
            assert_eq!(
                summary(&def).headless,
                state(&lines, "headless") == AVAILABLE,
                "{id}"
            );
        }
    }

    #[test]
    fn text_has_one_line_per_feature() {
        let text = render_text(&report(&Definition::default()));
        assert_eq!(text.lines().count(), report(&Definition::default()).len());
        assert!(text.contains("unsupported  usage_meter"));
    }
}
