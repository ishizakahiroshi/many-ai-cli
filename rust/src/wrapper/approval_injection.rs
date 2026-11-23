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
/// The rules handed to the AI CLIs. `approval_injection/rules.txt` beside it is
/// the frozen Go text (version 25) that the pinned oracle cases were generated
/// with: it is kept byte for byte for those cases and is not shipped.
pub const RULES: &str = include_str!("approval_injection/rules_current.txt");
/// First line of `RULES`. A central file or shared block without it is replaced.
const RULES_VERSION: &str = "<!-- version: 26 -->";
/// What an instruction file without imports receives instead of `RULES`: the
/// two forms the dashboard needs most and where the full rules are. An AI CLI
/// with a budget for always-on rules (Devin CLI: 32,768 bytes in total) drops
/// the project's own rules when a 13 KB block lands in the same file.
pub const RULES_SHORT: &str = include_str!("approval_injection/rules_short.txt");
/// First line of `RULES_SHORT`. A shared block without it — every block that
/// carries the full rules — is replaced on the next injection.
const SHORT_VERSION: &str = "<!-- short-version: 1 -->";
/// Stands in `RULES_SHORT` for the path of the central file. The path is
/// written out because `~` is not the same folder for every AI CLI the Hub
/// starts (a profile may run with its own home).
const CENTRAL_PLACEHOLDER: &str = "{{central}}";
/// A shared block may not exceed this, whatever the central path is.
pub const SHORT_LIMIT: usize = 1200;
/// The short rules pointing at `central`. A path too long for the limit is
/// replaced by the `~` spelling, which every AI CLI started with the default
/// home still resolves.
pub fn short_rules(central: &Path) -> String {
    let body = RULES_SHORT.replace(CENTRAL_PLACEHOLDER, &central.to_string_lossy());
    if body.len() <= SHORT_LIMIT {
        return body;
    }
    RULES_SHORT.replace(CENTRAL_PLACEHOLDER, "~/.many-ai-cli/approval-rules.md") // doxguard: allow approval-rules.md
}
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
/// Whether an instruction file already carries the rules (the same scan that
/// lets inject skip it). Doctor uses this to report targets the Hub missed.
pub fn rules_present(provider: &str, content: &[u8]) -> bool {
    let markers: &[&str] = if provider == "claude" {
        &[IMPORT]
    } else {
        &[START, OLD_START]
    };
    scan(content, markers).unwrap_or(false)
}
pub fn inject_rules_content(provider: &str, content: &[u8], central: &[u8]) -> io::Result<Vec<u8>> {
    inject_rules_at(provider, content, central, RULES_VERSION)
}
/// `version` is the line a shared block must carry to be left alone; the pinned
/// Go cases pass the version they were generated with.
fn inject_rules_at(
    provider: &str,
    content: &[u8],
    central: &[u8],
    version: &str,
) -> io::Result<Vec<u8>> {
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
    if already && (provider == "claude" || current(content, START, END, version)) {
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
/// The shared block of an import-less instruction file: `body` is the short
/// rules (`short_rules`). A block left by a version that wrote the full rules
/// does not carry `SHORT_VERSION` and is replaced.
pub fn inject_short_content(provider: &str, content: &[u8], body: &[u8]) -> io::Result<Vec<u8>> {
    if !shared(provider) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unknown instruction provider",
        ));
    }
    inject_rules_at(provider, content, body, SHORT_VERSION)
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
/// Where an instruction target comes from, which decides whether a link at the
/// file itself is followed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// A file in the user's own configuration folder (`CLAUDE.md` of a profile,
    /// `$CODEX_HOME/AGENTS.md`). The user placed any link there, so it is
    /// followed (a dotfiles link keeps working).
    User,
    /// `AGENTS.md` at the top of the project a session runs in. The folders
    /// down to the project are the user's and their links are followed (a
    /// worktree junction), but the file itself comes with the repository: when
    /// it is a link it is neither read nor written, so a cloned repository
    /// cannot point the write at a file outside it.
    Project,
}
/// What `inject` and `delegation` return for a project file that is a link.
const LINKED_PROJECT_FILE: &str = "project instruction file is a link";
fn is_link(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // FILE_ATTRIBUTE_REPARSE_POINT: symlinks and every other reparse kind.
        if meta.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    meta.file_type().is_symlink()
}
/// The mode a rewritten user file is created with. The replacement is a new
/// file, so the existing one's permission bits are carried over (a 0600 file
/// stays 0600, an executable keeps its bit); a new file gets 0644.
fn kept_mode(dir: &Dir, name: &str) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = dir.metadata(name) {
            return meta.permissions().mode() & 0o777;
        }
    }
    #[cfg(not(unix))]
    let _ = (dir, name);
    0o644
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
    /// Instruction targets are the user's own files, and users reach them
    /// through links they placed themselves (a junction for a worktree or a
    /// profile folder, a dotfiles symlink for CLAUDE.md). Follow those links
    /// once, as the Go version's plain writes did, then open the real location
    /// with the no-follow walk below. A trial keeps the lexical walk so a link
    /// can never lead it out of its root.
    ///
    /// A project's `AGENTS.md` follows the links of its folders only: `None`
    /// when the file itself is a link, which is then left untouched.
    fn resolve(&self, path: &Path, origin: Origin) -> io::Result<Option<PathBuf>> {
        let real = match origin {
            Origin::User if self.paths.is_trial() => return Ok(Some(path.to_path_buf())),
            Origin::User => return crate::files::scope::canonical(path).map(Some),
            Origin::Project if self.paths.is_trial() => path.to_path_buf(),
            Origin::Project => {
                let path = crate::files::scope::clean(path);
                let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
                    return Err(io::Error::other("instruction path unavailable"));
                };
                crate::files::scope::canonical(parent)?.join(name)
            }
        };
        match std::fs::symlink_metadata(&real) {
            Ok(meta) if is_link(&meta) => Ok(None),
            _ => Ok(Some(real)),
        }
    }
    /// `resolve` for a write: a linked project file is refused, so the Hub
    /// warns and doctor names the file that did not get the rules.
    fn writable(&self, path: &Path, origin: Origin) -> io::Result<PathBuf> {
        self.resolve(path, origin)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, LINKED_PROJECT_FILE))
    }
    /// The real file a target is written through, so two spellings of one file
    /// (two profiles linking the same CLAUDE.md) are known to be one target.
    pub fn real(&self, path: &Path, origin: Origin) -> PathBuf {
        self.resolve(path, origin)
            .ok()
            .flatten()
            .unwrap_or_else(|| path.to_path_buf())
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
            return dir.replace(&name, body, 0o600);
        }
        dir.replace(&name, body, kept_mode(&dir, &name))
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
    pub fn inject(&self, provider: &str, path: &Path, origin: Origin) -> io::Result<()> {
        let _guard = OPERATIONS
            .lock()
            .map_err(|_| io::Error::other("instruction lock failed"))?;
        let path = &self.writable(path, origin)?;
        let central = self.sync("approval-rules.md", RULES_VERSION, RULES.as_bytes())?; // doxguard: allow approval-rules.md
        let content = match self.read(path) {
            Ok(v) => v,
            Err(e) if e.kind() == io::ErrorKind::NotFound => vec![],
            Err(e) => return Err(e),
        };
        // Claude Code imports the central file. Every other AI CLI gets the short
        // rules in its own file and reads the central file when it needs more.
        let updated = if provider == "claude" {
            inject_rules_content(provider, &content, &central)?
        } else {
            let central_path = self.home.join(".many-ai-cli").join("approval-rules.md"); // doxguard: allow approval-rules.md
            inject_short_content(provider, &content, short_rules(&central_path).as_bytes())?
        };
        if content != updated {
            self.write(path, &updated, false)?;
        }
        Ok(())
    }
    pub fn delegation(
        &self,
        provider: &str,
        path: &Path,
        origin: Origin,
        enabled: bool,
    ) -> io::Result<()> {
        let _guard = OPERATIONS
            .lock()
            .map_err(|_| io::Error::other("instruction lock failed"))?;
        let path = &self.writable(path, origin)?;
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
    /// A linked project file was never written, so there is nothing to take out
    /// of it: it is left alone and reported as done.
    pub fn remove(
        &self,
        provider: &str,
        path: &Path,
        origin: Origin,
        delegation: bool,
    ) -> io::Result<()> {
        let _guard = OPERATIONS
            .lock()
            .map_err(|_| io::Error::other("instruction lock failed"))?;
        let Some(path) = &self.resolve(path, origin)? else {
            return Ok(());
        };
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
