//! New-input validation only. Loading an old store must not rewrite its records.
use super::{model::Definition, schedule};
use crate::proto::time::Timestamp;
use std::path::{Component, Path, PathBuf};

pub fn validate(item: &mut Definition, home: &Path, now: Timestamp) -> Result<(), String> {
    item.name = item.name.trim().into();
    item.cwd = item.cwd.trim().into();
    item.prompt = item.prompt.trim().into();
    if item.name.is_empty() || item.name.len() > 200 {
        return Err("name must contain 1 to 200 bytes".into());
    }
    if item.prompt.is_empty()
        || item.prompt.len() > 8192
        || item
            .prompt
            .chars()
            .any(|c| (c < ' ' && c != '\n' && c != '\t') || c == '\u{7f}')
    {
        return Err("prompt is empty, too long, or contains unsupported control characters".into());
    }
    if crate::profile::declared::is_acp(&item.provider) {
        return Err(crate::profile::declared::ACP_HUMAN_ONLY.into());
    }
    let Some(provider) = crate::profile::declared::declared(&item.provider)
        .filter(crate::profile::declared::routines_allowed)
    else {
        return Err("unsupported AI provider".into());
    };
    item.completion_mode = crate::profile::declared::completion_mode(&provider).into();
    if item.model.starts_with('-')
        || item
            .model
            .chars()
            .any(|c| c < ' ' || c == '\u{7f}' || " \"'|&><^%();`$".contains(c))
    {
        return Err("invalid model".into());
    }
    let cwd = Path::new(&item.cwd);
    if !cwd.is_absolute() || too_broad(cwd, home) {
        return Err("cwd must be an absolute project directory".into());
    }
    if !cwd.metadata().is_ok_and(|m| m.is_dir()) {
        return Err("cwd does not exist or is not a directory".into());
    }
    if item.schedule.kind.is_empty() {
        item.schedule.kind = "manual".into();
    }
    schedule::next(&item.schedule, now).map(|_| ())
}
fn clean(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            _ => out.push(c.as_os_str()),
        }
    }
    out
}
fn too_broad(cwd: &Path, home: &Path) -> bool {
    let cwd = clean(cwd);
    let home = clean(home);
    if cwd.parent().is_none() || cwd == home || home.parent() == Some(cwd.as_path()) {
        return true;
    }
    let text = cwd.to_string_lossy();
    if matches!(
        text.as_ref(),
        "/etc" | "/usr" | "/var" | "/bin" | "/sbin" | "/lib" | "/root" | "/home" | "/Users"
    ) {
        return true;
    }
    // The source also rejects these Windows names when seen on Unix.
    [
        r"C:\Windows",
        r"C:\Program Files",
        r"C:\Program Files (x86)",
        r"C:\Users",
    ]
    .iter()
    .any(|p| text.eq_ignore_ascii_case(p))
}

#[cfg(test)]
mod acp_tests {
    use super::*;
    use crate::{
        profile::{declared, registry},
        proto::provider::{self, Layers},
    };

    #[test]
    fn a_routine_cannot_use_an_acp_provider() {
        let acp = provider::Definition {
            schema_version: 1,
            id: "fixture-acp".into(),
            display_name: "Fixture ACP".into(),
            connection: "acp".into(),
            launch: Some(provider::LaunchDefinition {
                executable: "fixture-acp-bin".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let _guard = declared::install_test_registry(registry::Registry::build(
            Layers {
                embedded: Some(registry::embedded_definitions().unwrap()),
                user: Some(vec![acp]),
                ..Default::default()
            },
            &registry::default_adapters(),
        ));
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let mut item = Definition {
            name: "nightly".into(),
            cwd: project.to_string_lossy().into_owned(),
            provider: "fixture-acp".into(),
            prompt: "do it".into(),
            ..Default::default()
        };
        let home = dir.path().join("home");
        let error = validate(&mut item, &home, Timestamp::now()).unwrap_err();
        assert_eq!(error, declared::ACP_HUMAN_ONLY);
        // The same definition with a PTY provider passes this check.
        item.provider = "claude".into();
        assert!(validate(&mut item, &home, Timestamp::now()).is_ok());
    }
}
