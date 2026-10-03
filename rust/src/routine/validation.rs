//! New-input validation only. Loading an old store must not rewrite its records.
use super::{model::Definition, schedule};
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

pub fn validate(item: &mut Definition, home: &Path, now: SystemTime) -> Result<(), String> {
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
    if !matches!(
        item.provider.as_str(),
        "claude" | "codex" | "copilot" | "cursor-agent" | "opencode" | "grok" | "command-code"
    ) {
        return Err("unsupported AI provider".into());
    }
    item.completion_mode = if item.provider == "codex" {
        "native_or_marker"
    } else {
        "marker"
    }
    .into();
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
