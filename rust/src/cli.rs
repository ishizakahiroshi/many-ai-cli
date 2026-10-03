//! Dispatch mirrors cmd/many-ai-cli/main.go. Execution is wired by the integration owner.
use std::path::PathBuf;

pub const BUILTIN_PROVIDERS: &[&str] = &[
    "claude",
    "codex",
    "copilot",
    "cursor-agent",
    "opencode",
    "grok",
    "command-code",
];
pub const USAGE: &str = "many-ai-cli <serve|connect|setup|doctor|issue|wrap|claude|codex|copilot|cursor-agent|opencode|grok|command-code|shell-init|stop|status|tray|profile-export|log-clean|uninstall|version>";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Default,
    Version,
    Help,
    Serve(Vec<String>),
    Connect(Vec<String>),
    Setup(Vec<String>),
    Doctor(Vec<String>),
    Issue(Vec<String>),
    Provider(Vec<String>),
    Wrap { provider: String, args: Vec<String> },
    ShellInit,
    Stop,
    Status,
    Tray,
    ProfileExport(Vec<String>),
    LogClean(Vec<String>),
    Uninstall(Vec<String>),
    UsageRelay(Vec<String>),
    Orchestrate(Vec<String>),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrialOptions {
    pub root: PathBuf,
    pub port: u16,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invocation {
    pub trial: Option<TrialOptions>,
    pub command: Command,
}

pub fn parse(args: &[String], custom_providers: &[String]) -> Result<Invocation, String> {
    // Migration-only flags are accepted only as a leading pair. Provider argv is opaque.
    let mut args = args;
    let mut root = None;
    let mut port = None;
    while args
        .first()
        .is_some_and(|x| x == "--trial-root" || x == "--trial-port")
    {
        let value = args.get(1).ok_or("trial flag requires a value")?;
        match args[0].as_str() {
            "--trial-root" => {
                if root.is_some() {
                    return Err("duplicate --trial-root".into());
                }
                root = Some(PathBuf::from(value));
            }
            "--trial-port" => {
                if port.is_some() {
                    return Err("duplicate --trial-port".into());
                }
                port = Some(value.parse::<u16>().map_err(|_| "invalid trial port")?);
            }
            _ => unreachable!(),
        }
        args = &args[2..];
    }
    let trial = match (root, port) {
        (None, None) => None,
        (Some(root), Some(port)) if root.is_absolute() && port > 0 && port != 47777 => {
            Some(TrialOptions { root, port })
        }
        _ => {
            return Err(
                "trial mode requires an absolute --trial-root and a non-default --trial-port"
                    .into(),
            );
        }
    };
    let Some(cmd) = args.first() else {
        return Ok(Invocation {
            trial,
            command: Command::Default,
        });
    };
    let rest = args[1..].to_vec();
    let command = match cmd.as_str() {
        "version" | "--version" | "-v" => Command::Version,
        "help" | "--help" | "-h" => Command::Help,
        "serve" => Command::Serve(rest),
        "connect" => Command::Connect(rest),
        "setup" => Command::Setup(rest),
        "doctor" => Command::Doctor(rest),
        "issue" => Command::Issue(rest),
        "provider" => Command::Provider(rest),
        "shell-init" => Command::ShellInit,
        "stop" => Command::Stop,
        "status" => Command::Status,
        "tray" => Command::Tray,
        "profile-export" => Command::ProfileExport(rest),
        "log-clean" => Command::LogClean(rest),
        "uninstall" => Command::Uninstall(rest),
        "usage-relay" => Command::UsageRelay(rest),
        "orchestrate" if rest.is_empty() => return Err("orchestrate <spawn|send>".into()),
        "orchestrate" => Command::Orchestrate(rest),
        "wrap" if rest.is_empty() => return Err("wrap <provider>".into()),
        "wrap" => Command::Wrap {
            provider: rest[0].clone(),
            args: rest[1..].to_vec(),
        },
        _ if BUILTIN_PROVIDERS.contains(&cmd.as_str()) || custom_providers.contains(cmd) => {
            Command::Wrap {
                provider: cmd.clone(),
                args: rest,
            }
        }
        _ => return Err(format!("unknown command: {cmd}")),
    };
    Ok(Invocation { trial, command })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }
    #[test]
    fn all_aliases_and_opaque_provider_arguments() {
        for p in BUILTIN_PROVIDERS {
            assert_eq!(
                parse(&args(&[p, "--trial-root", "not-a-hub-flag", "日本語"]), &[])
                    .unwrap()
                    .command,
                Command::Wrap {
                    provider: p.to_string(),
                    args: args(&["--trial-root", "not-a-hub-flag", "日本語"])
                }
            );
        }
        for a in ["version", "--version", "-v"] {
            assert_eq!(parse(&args(&[a]), &[]).unwrap().command, Command::Version);
        }
        assert!(matches!(
            parse(&args(&["custom", "--x"]), &args(&["custom"]))
                .unwrap()
                .command,
            Command::Wrap { .. }
        ));
    }
    #[test]
    fn hidden_commands_and_missing_arguments() {
        assert!(matches!(
            parse(&args(&["usage-relay", "codex"]), &[])
                .unwrap()
                .command,
            Command::UsageRelay(_)
        ));
        assert!(parse(&args(&["orchestrate"]), &[]).is_err());
        assert!(parse(&args(&["wrap"]), &[]).is_err());
        assert!(parse(&args(&["gemini"]), &[]).is_err());
    }
    #[test]
    fn trial_is_explicit_and_complete() {
        assert!(
            parse(
                &args(&["--trial-root", "relative", "--trial-port", "49000", "serve"]),
                &[]
            )
            .is_err()
        );
        assert!(parse(&args(&["--trial-port", "49000", "serve"]), &[]).is_err());
        assert!(
            parse(
                &args(&[
                    "--trial-root",
                    "/synthetic",
                    "--trial-port",
                    "47777",
                    "serve"
                ]),
                &[]
            )
            .is_err()
        );
        assert!(parse(&args(&["serve"]), &[]).unwrap().trial.is_none());
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LauncherInvocation {
    pub profile: String,
    pub use_last: bool,
    pub open_ui: bool,
    pub help: bool,
}
/// Go flag.FlagSet-compatible lexical behavior: one/two dashes, =value,
/// boolean literals, and stop at the first positional value or `--`.
pub fn parse_launcher(args: &[String]) -> Result<LauncherInvocation, String> {
    let mut invocation = LauncherInvocation::default();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--" || arg == "-" || !arg.starts_with('-') {
            break;
        }
        let flag = arg
            .strip_prefix("--")
            .or_else(|| arg.strip_prefix('-'))
            .unwrap();
        if flag.starts_with(['-', '=']) {
            return Err(format!("bad flag syntax: {arg}"));
        }
        let (name, inline) = flag
            .split_once('=')
            .map_or((flag, None), |(n, v)| (n, Some(v)));
        match name {
            "h" | "help" => {
                invocation.help = true;
                return Ok(invocation);
            }
            "profile" => {
                let value = if let Some(v) = inline {
                    v
                } else {
                    i += 1;
                    args.get(i).ok_or("flag needs an argument: -profile")?
                };
                invocation.profile = value.into();
            }
            "last" | "ui" => {
                let value = match inline.unwrap_or("true") {
                    "1" | "t" | "T" | "true" | "TRUE" | "True" => true,
                    "0" | "f" | "F" | "false" | "FALSE" | "False" => false,
                    _ => {
                        return Err(format!(
                            "invalid boolean value {} for -{name}: parse error",
                            go_quote(inline.unwrap_or("true"))
                        ));
                    }
                };
                if name == "last" {
                    invocation.use_last = value
                } else {
                    invocation.open_ui = value
                }
            }
            _ => return Err(format!("flag provided but not defined: -{name}")),
        }
        i += 1;
    }
    Ok(invocation)
}
#[cfg(test)]
mod launcher_tests {
    use super::*;
    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }
    #[test]
    fn launcher_defaults_and_go_flags() {
        assert_eq!(parse_launcher(&[]).unwrap(), LauncherInvocation::default());
        assert_eq!(
            parse_launcher(&args(&["-profile=two words", "--last=false", "--ui=1"])).unwrap(),
            LauncherInvocation {
                profile: "two words".into(),
                use_last: false,
                open_ui: true,
                help: false
            }
        );
        assert!(
            parse_launcher(&args(&["--last", "false", "--ui"]))
                .unwrap()
                .use_last
        );
        assert!(
            !parse_launcher(&args(&["--last", "false", "--ui"]))
                .unwrap()
                .open_ui
        );
        assert!(parse_launcher(&args(&["--profile"])).is_err());
        assert!(parse_launcher(&args(&["--ui=bad"])).is_err());
        assert!(parse_launcher(&args(&["--help"])).unwrap().help);
    }
}

fn go_quote(value: &str) -> String {
    use std::fmt::Write;
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '\"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{7}' => out.push_str("\\a"),
            '\u{8}' => out.push_str("\\b"),
            '\u{b}' => out.push_str("\\v"),
            '\u{c}' => out.push_str("\\f"),
            c if c.is_ascii_control() => {
                write!(&mut out, "\\x{:02x}", c as u32).unwrap();
            }
            c if c.is_control()
                || (c.is_whitespace() && c != ' ')
                || matches!(c, '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{feff}') =>
            {
                if (c as u32) <= 0xffff {
                    write!(&mut out, "\\u{:04x}", c as u32).unwrap();
                } else {
                    write!(&mut out, "\\U{:08x}", c as u32).unwrap();
                }
            }
            c => out.push(c),
        }
    }
    out.push('\"');
    out
}
