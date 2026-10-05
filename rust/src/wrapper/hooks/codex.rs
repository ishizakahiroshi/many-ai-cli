//! Fixed-Go Codex Stop hook RMW. Explicit selected path; no home/environment discovery.
use crate::{config::RuntimePaths, files::safe_fs::Dir, process::pid_alive};
use std::{
    io::{self, Read},
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, SystemTime},
};
const START: &str = "# many-ai-cli:usage-hook-start";
const END: &str = "# many-ai-cli:usage-hook-end";
const OLD_START: &str = "# any-ai-cli:usage-hook-start";
const OLD_END: &str = "# any-ai-cli:usage-hook-end";
static CONFIG: Mutex<()> = Mutex::new(());
pub struct HookParams<'a> {
    pub hub_url: &'a str,
    pub token: &'a str,
    pub session: i64,
    pub executable: &'a Path,
}
pub fn block(p: &HookParams<'_>) -> String {
    let command = format!(
        "MANY_AI_CLI_HUB_TOKEN={} {} usage-relay --provider codex --hub {} --session {}",
        p.token,
        super::quote(&p.executable.to_string_lossy().replace('\\', "/")),
        p.hub_url,
        p.session
    );
    format!(
        "{START}\n[[hooks.Stop]]\ncommand = {}\n{END}\n",
        crate::proto::go_quote::quote(&command)
    )
}
fn pattern(start: &str, end: &str, prefix: bool) -> regex::Regex {
    regex::Regex::new(&format!(
        "(?s){}{}.*?{}\\n?",
        if prefix { "\\n?" } else { "" },
        regex::escape(start),
        regex::escape(end)
    ))
    .expect("fixed marker regex")
}
pub fn inject_content(content: &str, p: &HookParams<'_>) -> String {
    let content = pattern(OLD_START, OLD_END, true).replace_all(content, "");
    let new = block(p);
    if content.contains(START) {
        pattern(START, END, false)
            .replace_all(&content, regex::NoExpand(&new))
            .into_owned()
    } else {
        let mut content = content.into_owned();
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push('\n');
        content.push_str(&new);
        content
    }
}
pub fn remove_content(content: &str) -> String {
    let re = regex::Regex::new(&format!(
        "(?s)\\n?(?:{}.*?{}|{}.*?{})\\n?",
        regex::escape(START),
        regex::escape(END),
        regex::escape(OLD_START),
        regex::escape(OLD_END)
    ))
    .expect("fixed markers");
    re.replace_all(content, "").into_owned()
}
fn bytes_pattern(start: &str, end: &str, prefix: bool) -> regex::bytes::Regex {
    regex::bytes::Regex::new(&format!(
        "(?s-u){}{}.*?{}\\n?",
        if prefix { "\\n?" } else { "" },
        regex::escape(start),
        regex::escape(end)
    ))
    .expect("fixed bytes marker regex")
}
fn inject_bytes(content: &[u8], p: &HookParams<'_>) -> Vec<u8> {
    let content = bytes_pattern(OLD_START, OLD_END, true).replace_all(content, &b""[..]);
    let new = block(p);
    if content.windows(START.len()).any(|v| v == START.as_bytes()) {
        bytes_pattern(START, END, false)
            .replace_all(&content, |_: &regex::bytes::Captures<'_>| {
                new.as_bytes().to_vec()
            })
            .into_owned()
    } else {
        let mut content = content.into_owned();
        if !content.is_empty() && !content.ends_with(b"\n") {
            content.push(b'\n');
        }
        content.push(b'\n');
        content.extend_from_slice(new.as_bytes());
        content
    }
}
fn remove_bytes(content: &[u8]) -> Vec<u8> {
    let re = regex::bytes::Regex::new(&format!(
        "(?s-u)\\n?(?:{}.*?{}|{}.*?{})\\n?",
        regex::escape(START),
        regex::escape(END),
        regex::escape(OLD_START),
        regex::escape(OLD_END)
    ))
    .expect("fixed byte markers");
    re.replace_all(content, &b""[..]).into_owned()
}
pub struct CodexStopHooks {
    paths: RuntimePaths,
    path: PathBuf,
}
impl CodexStopHooks {
    pub fn new(paths: RuntimePaths, path: PathBuf) -> io::Result<Self> {
        crate::profile::subscriptions::check_path(&paths, &path)?;
        if !path.is_absolute() || path.file_name().and_then(|v| v.to_str()) != Some("config.toml") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Codex config path must be explicit absolute",
            ));
        }
        Ok(Self { paths, path })
    }
    /// Resolve the same selected CODEX_HOME/default root as Go against explicit actor cwd.
    pub fn for_context(
        paths: RuntimePaths,
        home: &Path,
        environment: &[String],
        cwd: &Path,
    ) -> io::Result<Self> {
        let override_root = environment
            .iter()
            .rev()
            .find_map(|v| {
                v.split_once('=')
                    .filter(|(k, _)| *k == "CODEX_HOME")
                    .map(|(_, v)| v)
            })
            .unwrap_or("");
        let root = if override_root.is_empty() {
            home.join(".codex")
        } else {
            PathBuf::from(override_root)
        };
        let root = if root.is_absolute() {
            root
        } else {
            cwd.join(root)
        };
        Self::new(paths, crate::files::scope::clean(&root.join("config.toml")))
    }
    fn directory(&self) -> io::Result<Dir> {
        crate::profile::subscriptions::check_path(&self.paths, &self.path)?;
        let parent = self
            .path
            .parent()
            .ok_or_else(|| io::Error::other("Codex config parent unavailable"))?;
        if parent.exists() {
            Dir::open(parent)
        } else {
            Dir::open_or_create_private(parent)
        }
    }
    pub fn injected(&self) -> io::Result<bool> {
        let dir = match self
            .path
            .parent()
            .ok_or_else(|| io::Error::other("Codex config parent unavailable"))
            .and_then(Dir::open)
        {
            Ok(v) => v,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e),
        };
        let content = match read(&dir, "config.toml") {
            Ok(v) => v,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e),
        };
        let content = String::from_utf8_lossy(&content);
        Ok(content.contains(START) || content.contains(OLD_START))
    }
    pub fn inject(&self, p: &HookParams<'_>) -> io::Result<()> {
        let _serial = CONFIG.lock().unwrap_or_else(|p| p.into_inner());
        let dir = self.directory()?;
        let _lock = Lock::acquire(&dir)?;
        let bytes = match read(&dir, "config.toml") {
            Ok(v) => v,
            Err(e) if e.kind() == io::ErrorKind::NotFound => vec![],
            Err(e) => return Err(e),
        };
        let content = inject_bytes(&bytes, p);
        write(&dir, &content)
    }
    pub fn remove(&self) -> io::Result<()> {
        let _serial = CONFIG.lock().unwrap_or_else(|p| p.into_inner());
        let dir = self.directory()?;
        let _lock = Lock::acquire(&dir)?;
        let bytes = match read(&dir, "config.toml") {
            Ok(v) => v,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };
        let new = remove_bytes(&bytes);
        if new == bytes {
            return Ok(());
        }
        write(&dir, &new)
    }
}
fn read(dir: &Dir, name: &str) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    dir.open_file(name, false)?.read_to_end(&mut bytes)?;
    Ok(bytes)
}
fn write(dir: &Dir, content: &[u8]) -> io::Result<()> {
    #[cfg(not(unix))]
    let mode = 0o600;
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        let mut mode = 0o600;
        if let Ok(meta) = dir.metadata("config.toml") {
            mode = meta.permissions().mode() & 0o600;
            if mode == 0 {
                mode = 0o600;
            }
        }
        mode
    };
    dir.replace("config.toml", content, mode)
}
struct Lock<'a> {
    dir: &'a Dir,
    owned: Vec<u8>,
}
impl<'a> Lock<'a> {
    fn acquire(dir: &'a Dir) -> io::Result<Self> {
        let owned = format!("{}\n", std::process::id()).into_bytes();
        for _ in 0..50 {
            match dir.create_new("config.toml.many-ai-cli.lock", &owned, 0o600) {
                Ok(()) => return Ok(Self { dir, owned }),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    if !steal(dir)? {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                }
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "Codex config lock busy",
        ))
    }
}
impl Drop for Lock<'_> {
    fn drop(&mut self) {
        if read(self.dir, "config.toml.many-ai-cli.lock").is_ok_and(|v| v == self.owned) {
            let _ = self.dir.remove_file("config.toml.many-ai-cli.lock");
        }
    }
}
fn steal(dir: &Dir) -> io::Result<bool> {
    let prior = read(dir, "config.toml.many-ai-cli.lock").ok();
    if let Some(bytes) = &prior
        && let Ok(pid) = crate::proto::wire::go_utf8_lossy(bytes)
            .trim()
            .parse::<i64>()
        && pid > 0
    {
        if pid_alive(pid) {
            return Ok(false);
        }
        if read(dir, "config.toml.many-ai-cli.lock").ok().as_ref() == prior.as_ref() {
            let _ = dir.remove_file("config.toml.many-ai-cli.lock");
            return Ok(true);
        }
        return Ok(false);
    }
    let meta = match dir.metadata("config.toml.many-ai-cli.lock") {
        Ok(v) => v,
        Err(_) => return Ok(false),
    };
    if meta
        .modified()
        .ok()
        .and_then(|v| SystemTime::now().duration_since(v).ok())
        .is_none_or(|age| age <= Duration::from_secs(30 * 60))
    {
        return Ok(false);
    }
    if read(dir, "config.toml.many-ai-cli.lock").ok().as_ref() == prior.as_ref() {
        let _ = dir.remove_file("config.toml.many-ai-cli.lock");
        return Ok(true);
    }
    Ok(false)
}
#[cfg(test)]
mod tests;
