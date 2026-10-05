//! Fixed-Go approval/delegation instruction transformations. No environment discovery.
use crate::{config::RuntimePaths, files::safe_fs::Dir};
use std::{
    io,
    path::{Component, Path, PathBuf},
    sync::Mutex,
};
const IMPORT: &str = "@~/.many-ai-cli/approval-rules.md";
const START: &str = "<!-- many-ai-cli:approval-rules -->";
const END: &str = "<!-- /many-ai-cli:approval-rules -->";
const OLD_START: &str = "<!-- any-ai-cli:approval-rules -->";
const OLD_END: &str = "<!-- /any-ai-cli:approval-rules -->";
const DELEGATE_START: &str = "<!-- many-ai-cli:delegation -->";
const DELEGATE_END: &str = "<!-- /many-ai-cli:delegation -->";
pub const RULES: &str = include_str!("approval_injection/rules.txt");
static OPERATIONS: Mutex<()> = Mutex::new(());
const CAP: usize = 32 * 1024 * 1024;
fn pattern(pairs: &[(&str, &str)]) -> regex::bytes::Regex {
    let alternatives = pairs
        .iter()
        .map(|(start, end)| format!("{}.*?{}", regex::escape(start), regex::escape(end)))
        .collect::<Vec<_>>()
        .join("|");
    regex::bytes::Regex::new(&format!("(?s-u)\\n?(?:{alternatives})\\n?")).expect("fixed markers")
}
pub fn strip_blocks(content: &[u8]) -> Vec<u8> {
    pattern(&[
        (START, END),
        (OLD_START, OLD_END),
        (DELEGATE_START, DELEGATE_END),
    ])
    .replace_all(content, &b""[..])
    .into_owned()
}
pub fn remove_rules_content(provider: &str, content: &[u8]) -> io::Result<Vec<u8>> {
    if provider == "claude" {
        let injected = format!("\n{IMPORT}\n");
        if content.ends_with(injected.as_bytes())
            && content
                .windows(IMPORT.len())
                .filter(|v| *v == IMPORT.as_bytes())
                .count()
                == 1
        {
            return Ok(content[..content.len() - injected.len()].to_vec());
        }
        let mut lines: Vec<&[u8]> = Vec::new();
        for line in content.split(|c| *c == b'\n') {
            if String::from_utf8_lossy(line).trim() != IMPORT {
                lines.push(line);
                continue;
            }
            if lines
                .last()
                .is_some_and(|line| String::from_utf8_lossy(line).trim().is_empty())
            {
                lines.pop();
            }
        }
        return Ok(lines.join(&b'\n'));
    }
    if !shared(provider) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unknown instruction provider",
        ));
    }
    Ok(pattern(&[(START, END), (OLD_START, OLD_END)])
        .replace_all(content, &b""[..])
        .into_owned())
}
pub fn remove_delegation_content(content: &[u8]) -> Vec<u8> {
    pattern(&[(DELEGATE_START, DELEGATE_END)])
        .replace_all(content, &b""[..])
        .into_owned()
}
fn shared(provider: &str) -> bool {
    matches!(provider, "codex" | "copilot" | "cursor-agent" | "opencode")
}
fn scan(content: &[u8], markers: &[&str]) -> io::Result<bool> {
    let mut offset = 0;
    for line in content.split_inclusive(|c| *c == b'\n') {
        // bufio.Scanner's exact boundary accepts an included newline at max.
        if line.len() > 8 * 1024 * 1024 || (line.len() == 8 * 1024 * 1024 && !line.ends_with(b"\n"))
        {
            return Err(io::Error::other("instruction scan line exceeds limit"));
        }
        if markers.contains(&String::from_utf8_lossy(line).trim()) {
            return Ok(true);
        }
        offset += line.len();
    }
    debug_assert_eq!(offset, content.len());
    Ok(false)
}
fn current(content: &[u8], start: &str, end: &str, version: &str) -> bool {
    let re = regex::bytes::Regex::new(&format!(
        "(?s-u){}(.*?){}",
        regex::escape(start),
        regex::escape(end)
    ))
    .expect("fixed markers");
    re.captures(content)
        .is_some_and(|c| c[1].windows(version.len()).any(|v| v == version.as_bytes()))
}
fn append(content: &mut Vec<u8>, start: &str, end: &str, body: &[u8]) {
    content.extend_from_slice(format!("\n{start}\n").as_bytes());
    let trimmed = regex::bytes::Regex::new(r"^\s+|\s+$")
        .expect("fixed whitespace")
        .replace_all(body, &b""[..]);
    content.extend_from_slice(&trimmed);
    content.extend_from_slice(format!("\n{end}\n").as_bytes());
}
pub fn inject_rules_content(provider: &str, content: &[u8], central: &[u8]) -> io::Result<Vec<u8>> {
    let markers = if provider == "claude" {
        vec![IMPORT]
    } else if shared(provider) {
        vec![START, OLD_START]
    } else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unknown instruction provider",
        ));
    };
    let already = scan(content, &markers)?;
    if already && (provider == "claude" || current(content, START, END, "<!-- version: 24 -->")) {
        return Ok(content.to_vec());
    }
    let mut result = if already {
        remove_rules_content(provider, content)?
    } else {
        content.to_vec()
    };
    if provider == "claude" {
        result.extend_from_slice(format!("\n{IMPORT}\n").as_bytes());
    } else {
        append(&mut result, START, END, central);
    }
    Ok(result)
}
pub fn inject_delegation_content(
    provider: &str,
    content: &[u8],
    central: &[u8],
) -> io::Result<Vec<u8>> {
    if !shared(provider) {
        return Ok(content.to_vec());
    }
    let already = scan(content, &[DELEGATE_START])?;
    if already && current(content, DELEGATE_START, DELEGATE_END, "<!-- version: 2 -->") {
        return Ok(content.to_vec());
    }
    let mut result = if already {
        remove_delegation_content(content)
    } else {
        content.to_vec()
    };
    append(&mut result, DELEGATE_START, DELEGATE_END, central);
    Ok(result)
}
pub struct InstructionFiles {
    paths: RuntimePaths,
    home: PathBuf,
}
impl InstructionFiles {
    pub fn new(paths: RuntimePaths, home: PathBuf) -> io::Result<Self> {
        if !home.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "explicit instruction home required",
            ));
        }
        let result = Self { paths, home };
        result.directory(&result.home, false)?;
        Ok(result)
    }
    fn directory(&self, path: &Path, create: bool) -> io::Result<Dir> {
        let (mut dir, relative) = if self.paths.is_trial() {
            // Keep the caller's components intact until the shared boundary has
            // checked the selected lexical/canonical root spellings. Descendants
            // still open through no-follow directory capabilities below.
            let relative = self.paths.relative_to_selected_root(path)?;
            (Dir::open(self.paths.root())?, relative)
        } else {
            let path = crate::files::scope::clean(path);
            let root = path
                .ancestors()
                .last()
                .ok_or_else(|| io::Error::other("instruction root unavailable"))?;
            (
                Dir::open(root)?,
                path.strip_prefix(root)
                    .map_err(|_| io::Error::other("instruction path unavailable"))?
                    .to_path_buf(),
            )
        };
        for part in relative.components() {
            let Component::Normal(name) = part else {
                return Err(io::Error::other("unclean instruction directory"));
            };
            dir = dir.child_dir(
                name.to_str()
                    .ok_or_else(|| io::Error::other("instruction component invalid"))?,
                create,
            )?;
        }
        Ok(dir)
    }
    fn selected(&self, path: &Path, create: bool) -> io::Result<(Dir, String)> {
        if !path.is_absolute()
            || path
                .components()
                .any(|p| matches!(p, Component::ParentDir | Component::CurDir))
        {
            return Err(io::Error::other(
                "instruction target must be absolute clean",
            ));
        }
        let name = path
            .file_name()
            .and_then(|v| v.to_str())
            .ok_or_else(|| io::Error::other("instruction filename invalid"))?;
        Ok((
            self.directory(
                path.parent()
                    .ok_or_else(|| io::Error::other("instruction parent invalid"))?,
                create,
            )?,
            name.into(),
        ))
    }
    pub fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        let (dir, name) = self.selected(path, false)?;
        let bytes = dir.read(&name, CAP + 1)?;
        if bytes.len() > CAP {
            return Err(io::Error::other(
                "instruction file exceeds safe mutation limit",
            ));
        }
        Ok(bytes)
    }
    pub fn write(&self, path: &Path, body: &[u8], private: bool) -> io::Result<()> {
        let (dir, name) = self.selected(path, true)?;
        if private {
            dir.restrict_private()?;
        }
        dir.replace(&name, body, if private { 0o600 } else { 0o644 })
    }
    pub fn remove_file(&self, path: &Path) -> io::Result<()> {
        match self
            .selected(path, false)
            .and_then(|(dir, name)| dir.remove_file(&name))
        {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            v => v,
        }
    }
    fn sync(&self, name: &str, version: &str, body: &[u8]) -> io::Result<Vec<u8>> {
        let path = self.home.join(".many-ai-cli").join(name);
        let content = match self.read(&path) {
            Ok(v) => v,
            Err(e) if e.kind() == io::ErrorKind::NotFound => vec![],
            Err(e) => return Err(e),
        };
        let first = content.split(|c| *c == b'\n').next().unwrap_or_default();
        if String::from_utf8_lossy(first).trim() == version {
            return Ok(content);
        }
        let (dir, file) = self.selected(&path, true)?;
        dir.restrict_private()?;
        dir.replace(&file, body, 0o644)?;
        Ok(body.to_vec())
    }
    pub fn inject(&self, provider: &str, path: &Path) -> io::Result<()> {
        let _guard = OPERATIONS
            .lock()
            .map_err(|_| io::Error::other("instruction lock failed"))?;
        let central = self.sync(
            "approval-rules.md",
            "<!-- version: 24 -->",
            RULES.as_bytes(),
        )?;
        let content = match self.read(path) {
            Ok(v) => v,
            Err(e) if e.kind() == io::ErrorKind::NotFound => vec![],
            Err(e) => return Err(e),
        };
        let updated = inject_rules_content(provider, &content, &central)?;
        if content != updated {
            self.write(path, &updated, false)?;
        }
        Ok(())
    }
    pub fn delegation(&self, provider: &str, path: &Path, enabled: bool) -> io::Result<()> {
        let _guard = OPERATIONS
            .lock()
            .map_err(|_| io::Error::other("instruction lock failed"))?;
        let content = match self.read(path) {
            Ok(v) => v,
            Err(e) if e.kind() == io::ErrorKind::NotFound => vec![],
            Err(e) => return Err(e),
        };
        let body = if enabled {
            if !shared(provider) {
                return Ok(());
            }
            let body = format!(
                "<!-- version: 2 -->\n## many-ai-cli Delegation\n\n{}",
                crate::wrapper::hooks::DELEGATION_PROMPT
            );
            let central = self.sync("delegation.md", "<!-- version: 2 -->", body.as_bytes())?;
            inject_delegation_content(provider, &content, &central)?
        } else {
            remove_delegation_content(&content)
        };
        if content != body {
            self.write(path, &body, false)?;
        }
        Ok(())
    }
    pub fn remove(&self, provider: &str, path: &Path, delegation: bool) -> io::Result<()> {
        let _guard = OPERATIONS
            .lock()
            .map_err(|_| io::Error::other("instruction lock failed"))?;
        let content = match self.read(path) {
            Ok(v) => v,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };
        let mut body = remove_rules_content(provider, &content)?;
        if delegation {
            body = remove_delegation_content(&body);
        }
        if body != content {
            self.write(path, &body, false)?;
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests;
