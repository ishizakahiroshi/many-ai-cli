//! The checks `provider test` runs against what a provider declared, beyond
//! reading the declaration: can the slash command source be read, does the
//! conversation folder exist, does the model list command run. Each check
//! returns the line that replaces the declared one; a declaration that is not
//! there is left to the static list.
use crate::{
    application::slash_commands::{NativeSlashIo, SlashIo},
    config::RuntimePaths,
    process::execpath::{NativeFs, Platform, Resolver},
    profile::capability_report::{AVAILABLE, CapabilityLine, UNAVAILABLE},
    proto::provider::Definition,
};
use std::{path::Path, time::Duration};

fn checked(key: &'static str, ok: bool, detail: String) -> CapabilityLine {
    CapabilityLine {
        key,
        state: if ok { AVAILABLE } else { UNAVAILABLE },
        detail,
    }
}

/// `{home}` and `{env:NAME}` of a conversation folder pattern, up to the first
/// part that depends on a session (`{cwd_slug:...}`), which is not known here.
pub fn transcript_folder(
    pattern: &str,
    home: Option<&Path>,
    env: &dyn Fn(&str) -> Option<String>,
) -> Option<std::path::PathBuf> {
    let mut out = std::path::PathBuf::new();
    for segment in pattern.split(['/', '\\']).filter(|s| !s.is_empty()) {
        if segment.contains("{cwd_slug") {
            break;
        }
        let mut text = String::new();
        let mut rest = segment;
        while let Some(open) = rest.find('{') {
            text.push_str(&rest[..open]);
            let close = rest[open..].find('}')? + open;
            let token = &rest[open + 1..close];
            if token == "home" {
                text.push_str(home?.to_str()?);
            } else if let Some(name) = token.strip_prefix("env:") {
                text.push_str(&env(name).filter(|v| !v.is_empty())?);
            } else {
                return None;
            }
            rest = &rest[close + 1..];
        }
        text.push_str(rest);
        out.push(text);
    }
    out.is_absolute().then_some(out)
}

pub async fn live_checks(
    def: &Definition,
    paths: &RuntimePaths,
    environment: &[String],
    cwd: &Path,
) -> Vec<CapabilityLine> {
    let mut out = vec![];
    if let Some(slash) = def.slash_commands.as_ref()
        && slash.items.is_empty()
        && !slash.source.is_empty()
    {
        out.push(
            match NativeSlashIo::new(paths.clone(), environment.to_vec(), None) {
                Err(e) => checked(
                    "slash_commands",
                    false,
                    format!("source could not be read: {e}"),
                ),
                Ok(io) => match io.read(&slash.source).await {
                    Ok(bytes) if !bytes.is_empty() => checked(
                        "slash_commands",
                        true,
                        format!("checked: the source was read ({} bytes)", bytes.len()),
                    ),
                    Ok(_) => checked(
                        "slash_commands",
                        false,
                        "the source was read but is empty".into(),
                    ),
                    Err(e) => checked(
                        "slash_commands",
                        false,
                        format!("the source could not be read: {e}"),
                    ),
                },
            },
        );
    }
    if let Some(root) = def
        .transcript
        .as_ref()
        .map(|t| if t.root.is_empty() { &t.file } else { &t.root })
        .filter(|p| !p.is_empty())
    {
        let env = |name: &str| {
            environment
                .iter()
                .find_map(|e| e.strip_prefix(&format!("{name}=")).map(str::to_owned))
                .or_else(|| std::env::var(name).ok())
        };
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(std::path::PathBuf::from);
        out.push(match transcript_folder(root, home.as_deref(), &env) {
            None => checked(
                "transcript",
                false,
                "the conversation location could not be worked out here (a part of its pattern is missing)".into(),
            ),
            Some(folder) => {
                let found = if folder.extension().is_some() && def
                    .transcript
                    .as_ref()
                    .is_some_and(|t| t.root.is_empty())
                {
                    folder.is_file()
                } else {
                    folder.is_dir()
                };
                checked(
                    "transcript",
                    found,
                    if found {
                        "checked: the conversation location exists".into()
                    } else {
                        "the declared conversation location does not exist yet (it appears after the CLI has run once)".into()
                    },
                )
            }
        });
    }
    if let (Some(launch), Some(list)) = (
        def.launch.as_ref().filter(|l| !l.executable.is_empty()),
        def.models
            .as_ref()
            .and_then(|m| m.list.as_ref())
            .filter(|l| !l.args.is_empty()),
    ) {
        let seconds = u64::try_from(list.timeout_seconds)
            .ok()
            .filter(|s| *s > 0)
            .unwrap_or(15);
        let resolver = Resolver::new(Platform::native(), environment, cwd, &NativeFs);
        let executable = resolver
            .look_path(&launch.executable)
            .unwrap_or_else(|_| launch.executable.clone());
        let mut command = tokio::process::Command::new(&executable);
        command
            .args(&list.args)
            .current_dir(cwd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        for name in super::provider_registry::hub_session_variables(
            environment,
            std::env::vars_os().map(|(name, _)| name),
        )
        .into_keys()
        {
            command.env_remove(name);
        }
        let result = tokio::time::timeout(Duration::from_secs(seconds), command.output()).await;
        out.push(match result {
            Err(_) => checked(
                "models",
                false,
                format!("the model list command did not finish in {seconds} s"),
            ),
            Ok(Err(e)) => checked(
                "models",
                false,
                format!("the model list command could not start: {e}"),
            ),
            Ok(Ok(o)) if o.status.success() && !o.stdout.is_empty() => {
                checked("models", true, "checked: the model list command ran".into())
            }
            Ok(Ok(o)) => checked(
                "models",
                false,
                format!(
                    "the model list command failed (exit code {})",
                    o.status.code().map_or("none".into(), |c| c.to_string())
                ),
            ),
        });
    }
    out
}

/// Puts each checked line in place of the declared line with the same key.
pub fn merge(mut lines: Vec<CapabilityLine>, checked: Vec<CapabilityLine>) -> Vec<CapabilityLine> {
    for line in checked {
        match lines.iter_mut().find(|l| l.key == line.key) {
            Some(slot) => *slot = line,
            None => lines.push(line),
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_pattern_is_worked_out_up_to_the_first_session_part() {
        let env = |name: &str| (name == "FIXTURE_HOME").then(|| "/fx".to_owned());
        let abs = |p: &str| {
            if cfg!(windows) {
                format!("C:{p}")
            } else {
                p.to_owned()
            }
        };
        let home_str = abs("/h");
        let home_path = Path::new(&home_str);
        let got = transcript_folder(
            "{home}/.fixture/sessions/{cwd_slug:alnum}",
            Some(home_path),
            &env,
        );
        assert_eq!(
            got,
            Some(Path::new(&home_str).join(".fixture").join("sessions"))
        );
        assert!(transcript_folder("{env:MISSING}/x", Some(home_path), &env).is_none());
        assert!(transcript_folder("{home}/a", None, &env).is_none());
        assert!(transcript_folder("relative/a", Some(home_path), &env).is_none());
    }

    #[test]
    fn a_checked_line_replaces_the_declared_one() {
        let declared = vec![CapabilityLine {
            key: "models",
            state: AVAILABLE,
            detail: "declared".into(),
        }];
        let merged = merge(declared, vec![checked("models", false, "broke".into())]);
        assert_eq!(merged[0].state, UNAVAILABLE);
    }
}
