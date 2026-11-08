//! Session-owned files. Persistent shell profiles and shared Claude settings are
//! never changed. OpenCode's required project overlay retains rollback evidence.
pub mod codex;
use crate::{
    config::{Resource, RuntimePaths},
    files::safe_fs::Dir,
    process::pid_alive,
    proto::Message,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::{self, Read},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

pub const DELEGATION_PROMPT: &str = include_str!("delegation_prompt.txt");
pub const OPENCODE_CONFIG: &str = "opencode.json";
pub const OPENCODE_LOCK: &str = "opencode.json.many-ai-cli.lock";
struct OwnedFile {
    directory: Dir,
    name: String,
}
impl Drop for OwnedFile {
    fn drop(&mut self) {
        let _ = self.directory.remove_file(&self.name);
    }
}
/// A session's own folder, removed with what is in it, without following a
/// link placed there.
struct OwnedFolder {
    directory: Dir,
    name: String,
}
impl Drop for OwnedFolder {
    fn drop(&mut self) {
        let _ = self.directory.remove_tree(&self.name);
    }
}
#[derive(Default)]
pub struct SessionHooks {
    files: Vec<OwnedFile>,
    folders: Vec<OwnedFolder>,
    opencode: Option<OpenCodeOverlay>,
}
/// What `claude_settings` adds to Claude's launch: its arguments, and the
/// variables its process gets.
#[derive(Default)]
pub struct ClaudeLaunch {
    pub args: Vec<String>,
    pub env: Vec<(String, std::ffi::OsString)>,
}
fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
/// Largest project settings file read when checking a bounded launch.
const PROJECT_SETTINGS_CAP: u64 = 1024 * 1024;
/// Why a bounded Claude child must not start in `cwd`: a project settings file
/// between `cwd` and its Git root widens the sandbox. Claude Code adds the
/// project's sandbox lists to the session's own, so a repository could let
/// the child's commands write outside the folder, reach the network or skip
/// the sandbox (owner's decision B1). `None` when nothing widens it.
pub fn project_widens_sandbox(cwd: &Path) -> Option<String> {
    fn widening(value: &Value, path: &str) -> Option<String> {
        let sandbox = value.get("sandbox")?;
        let listed =
            |v: &Value| v.as_array().is_some_and(|a| !a.is_empty()) || v == &Value::Bool(true);
        if sandbox.get("excludedCommands").is_some_and(listed) {
            return Some(format!("{path}: sandbox.excludedCommands"));
        }
        for group in ["filesystem", "network"] {
            if let Some(map) = sandbox.get(group).and_then(Value::as_object) {
                for (key, v) in map {
                    if key.starts_with("allow") && listed(v) {
                        return Some(format!("{path}: sandbox.{group}.{key}"));
                    }
                }
            }
        }
        None
    }
    let mut dir = Some(cwd);
    while let Some(current) = dir {
        for name in ["settings.json", "settings.local.json"] {
            let file = current.join(".claude").join(name);
            let shown = file.display().to_string();
            let bytes = match std::fs::File::open(&file) {
                Ok(f) => {
                    let mut bytes = Vec::new();
                    if f.take(PROJECT_SETTINGS_CAP + 1)
                        .read_to_end(&mut bytes)
                        .is_err()
                        || bytes.len() as u64 > PROJECT_SETTINGS_CAP
                    {
                        return Some(format!("{shown}: could not be read"));
                    }
                    bytes
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(_) => return Some(format!("{shown}: could not be read")),
            };
            match serde_json::from_slice::<Value>(&bytes) {
                Ok(value) => {
                    if let Some(reason) = widening(&value, &shown) {
                        return Some(reason);
                    }
                }
                Err(_) => return Some(format!("{shown}: is not valid JSON")),
            }
        }
        if current.join(".git").exists() {
            break;
        }
        dir = current.parent();
    }
    None
}
fn read(dir: &Dir, name: &str) -> io::Result<Vec<u8>> {
    let mut bytes = vec![];
    dir.open_file(name, false)?.read_to_end(&mut bytes)?;
    Ok(bytes)
}
/// Removes the files and folders a session of this user left under `prefix`
/// when its process is gone and a day has passed. This is housekeeping: a
/// scan that fails does not stop the new file.
fn remove_stale(directory: &Dir, prefix: &str, suffix: &str) {
    for name in directory.entries().unwrap_or_default() {
        let Some(body) = name
            .strip_prefix(prefix)
            .and_then(|v| v.strip_suffix(suffix))
        else {
            continue;
        };
        let Some((session, pid)) = body.split_once("-p") else {
            continue;
        };
        if session.parse::<u64>().is_err() {
            continue;
        }
        // The PID, followed by the random part of newer names.
        let pid = pid.split_once('-').map_or(pid, |(pid, _)| pid);
        let Some(pid) = pid.parse::<i64>().ok().filter(|pid| *pid > 0) else {
            continue;
        };
        if pid_alive(pid) {
            continue;
        }
        let old = |modified: io::Result<SystemTime>| {
            modified
                .ok()
                .and_then(|mtime| SystemTime::now().duration_since(mtime).ok())
                .is_some_and(|age| age > Duration::from_secs(24 * 3600))
        };
        // Opened without following a link, so only a real folder is emptied.
        if let Ok(folder) = directory.child_dir(&name, false) {
            if folder
                .own_metadata()
                .is_ok_and(|metadata| old(metadata.modified()))
            {
                drop(folder);
                let _ = directory.remove_tree(&name);
            }
            continue;
        }
        if directory
            .metadata(&name)
            .is_ok_and(|metadata| metadata.is_file() && old(metadata.modified()))
        {
            let _ = directory.remove_file(&name);
        }
    }
}
impl SessionHooks {
    /// The user's own folder, never the shared system temporary folder, where
    /// another user could take the name first or leave a name that stops the
    /// scan, so the session started without these files (U60). Also returns
    /// the start of this user's names there.
    fn own_folder(paths: &RuntimePaths, kind: &str) -> io::Result<(PathBuf, Dir, String)> {
        let root = paths.resource(Resource::UsageHooks);
        let directory = Dir::open_or_create_private(&root)?;
        let home = if paths.is_trial() {
            paths.root()
        } else {
            paths.root().parent().unwrap_or(paths.root())
        };
        let owner = Sha256::digest(home.to_string_lossy().as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        Ok((root, directory, format!("aac-{kind}-u{}-s", &owner[..16])))
    }
    /// A name with a random part, so it cannot be predicted; `create` is
    /// retried with a new one when the name is taken.
    fn fresh_name(
        prefix: &str,
        session_id: i64,
        suffix: &str,
        mut create: impl FnMut(&str) -> io::Result<()>,
    ) -> io::Result<String> {
        let mut attempts = 0;
        loop {
            let random = crate::process::random_token()?;
            let name = format!(
                "{prefix}{session_id}-p{}-{}{suffix}",
                std::process::id(),
                &random[..16]
            );
            match create(&name) {
                Ok(()) => return Ok(name),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists && attempts < 8 => {
                    attempts += 1;
                }
                Err(error) => return Err(error),
            }
        }
    }
    fn write(
        &mut self,
        paths: &RuntimePaths,
        kind: &str,
        session_id: i64,
        extension: &str,
        bytes: &[u8],
    ) -> io::Result<PathBuf> {
        let (root, directory, prefix) = Self::own_folder(paths, kind)?;
        let suffix = format!(".{extension}");
        remove_stale(&directory, &prefix, &suffix);
        let name = Self::fresh_name(&prefix, session_id, &suffix, |name| {
            directory.create_new(name, bytes, 0o600)
        })?;
        let path = root.join(&name);
        self.files.push(OwnedFile { directory, name });
        Ok(path)
    }
    /// A private folder for this session, removed with its contents when the
    /// session ends.
    fn folder(&mut self, paths: &RuntimePaths, kind: &str, session_id: i64) -> io::Result<PathBuf> {
        let (root, directory, prefix) = Self::own_folder(paths, kind)?;
        remove_stale(&directory, &prefix, "");
        let name = Self::fresh_name(&prefix, session_id, "", |name| directory.mkdir_new(name))?;
        let path = root.join(&name);
        self.folders.push(OwnedFolder { directory, name });
        Ok(path)
    }
    /// A headless session gets only the bounded settings; the status line and
    /// cross-session messages are for an interactive one.
    #[allow(clippy::too_many_arguments)]
    pub fn claude_settings(
        &mut self,
        paths: &RuntimePaths,
        registered: &Message,
        executable: &Path,
        hub_port: u16,
        native_windows: bool,
        headless: bool,
        bounded: bool,
    ) -> io::Result<ClaudeLaunch> {
        let statusbar = !headless && registered.token_statusbar;
        let cross = !headless
            && !native_windows
            && registered.auto
            && !registered.orchestration_id.is_empty();
        if !statusbar && !cross && !bounded {
            return Ok(ClaudeLaunch::default());
        }
        let mut doc = serde_json::Map::new();
        let mut env = Vec::new();
        if statusbar {
            // No token here: Claude runs this through a shell, whose command
            // line other users of the machine can read. The relay reads
            // MANY_AI_CLI_HUB_TOKEN from the environment Claude inherits from
            // this wrapper (U03).
            let command = format!(
                "{} usage-relay --provider claude --hub http://127.0.0.1:{hub_port} --session {}",
                quote(&executable.to_string_lossy().replace('\\', "/")),
                registered.session_id
            );
            doc.insert(
                "statusLine".into(),
                json!({"type":"command","command":command,"padding":0}),
            );
        }
        if cross {
            doc.insert("crossSessionInbound".into(), Value::String("accept".into()));
        }
        if bounded {
            // The bounded tier allows editing and a list of commands. Claude's
            // file tools then stay in the working folders, and where Claude
            // Code has a sandbox, what an allowed command runs (a test or a
            // hook the child edited) writes only there and reaches no network
            // (U49). Without the sandbox, Claude Code refuses to start rather
            // than run unsandboxed; a command is never retried outside it; and
            // a sandboxed command still needs a rule, so the tier's list stays
            // the limit. Claude Code versions that predate a key ignore it.
            doc.insert(
                "permissions".into(),
                json!({"blockWritesOutsideWorkingDirectories": true}),
            );
            if !native_windows {
                // Go's build cache outside the working folder is not
                // writable in the sandbox, and must not be shared: a cached
                // result the child wrote would be used by the user's own
                // builds. This session gets its own, removed at its end.
                let cache = self.folder(paths, "gocache", registered.session_id)?;
                doc.insert(
                    "sandbox".into(),
                    json!({
                        "enabled": true,
                        "failIfUnavailable": true,
                        "allowUnsandboxedCommands": false,
                        "autoAllowBashIfSandboxed": false,
                        "filesystem": {"allowWrite": [cache.to_string_lossy()]},
                    }),
                );
                env.push(("GOCACHE".into(), cache.into_os_string()));
            }
        }
        let mut bytes = serde_json::to_vec_pretty(&doc).map_err(io::Error::other)?;
        bytes.push(b'\n');
        let path = self.write(
            paths,
            "claude-settings",
            registered.session_id,
            "json",
            &bytes,
        )?;
        Ok(ClaudeLaunch {
            args: vec!["--settings".into(), path.to_string_lossy().into_owned()],
            env,
        })
    }
    pub fn delegation(
        &mut self,
        paths: &RuntimePaths,
        provider: &str,
        session_id: i64,
    ) -> io::Result<Vec<String>> {
        match provider {
            "claude" => {
                let path = self.write(
                    paths,
                    "delegation",
                    session_id,
                    "md",
                    DELEGATION_PROMPT.as_bytes(),
                )?;
                Ok(vec![
                    "--append-system-prompt-file".into(),
                    path.to_string_lossy().into_owned(),
                ])
            }
            "grok" => Ok(vec!["--rules".into(), DELEGATION_PROMPT.into()]),
            _ => Ok(vec![]),
        }
    }
    pub fn opencode(
        &mut self,
        cwd: &Path,
        permission: &str,
        bash_deny: &[String],
    ) -> io::Result<()> {
        self.opencode = Some(OpenCodeOverlay::prepare(cwd, permission, bash_deny)?);
        Ok(())
    }
    /// Return errors to the caller instead of concealing an un-restored overlay.
    pub fn finish(&mut self) -> io::Result<()> {
        if let Some(mut overlay) = self.opencode.take() {
            overlay.restore()?;
        }
        self.files.clear();
        self.folders.clear();
        Ok(())
    }
}
struct OpenCodeOverlay {
    dir: Dir,
    original: Option<Vec<u8>>,
    written: Vec<u8>,
    lock: Vec<u8>,
    restored: bool,
}
impl OpenCodeOverlay {
    fn prepare(cwd: &Path, permission: &str, deny: &[String]) -> io::Result<Self> {
        let dir = Dir::open(cwd)?;
        let initial =
            serde_json::to_vec(&json!({"pid":std::process::id()})).map_err(io::Error::other)?;
        if let Err(error) = dir.create_new(OPENCODE_LOCK, &initial, 0o600) {
            if error.kind() != io::ErrorKind::AlreadyExists {
                return Err(error);
            }
            let prior = read(&dir, OPENCODE_LOCK)?;
            let state = serde_json::from_slice::<Value>(&prior).unwrap_or(Value::Null);
            let pid = state
                .get("pid")
                .and_then(Value::as_i64)
                .filter(|pid| *pid > 0)
                .or_else(|| {
                    std::str::from_utf8(&prior)
                        .ok()?
                        .trim()
                        .parse::<i64>()
                        .ok()
                        .filter(|pid| *pid > 0)
                });
            let stale = match pid {
                Some(pid) => !pid_alive(pid),
                None => {
                    SystemTime::now()
                        .duration_since(dir.metadata(OPENCODE_LOCK)?.modified()?)
                        .unwrap_or_default()
                        > Duration::from_secs(30 * 60)
                }
            };
            if !stale {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "opencode.json locked by another session",
                ));
            }
            // Compare again before removing a stale pathname. The directory
            // capability prevents symlink traversal; never recover a live owner.
            if read(&dir, OPENCODE_LOCK)? != prior {
                return Err(io::Error::other("OpenCode lock changed while recovering"));
            }
            if let Some(rollback) = state.get("rollback") {
                let old = rollback
                    .get("original")
                    .and_then(Value::as_str)
                    .and_then(|s| STANDARD.decode(s).ok())
                    .unwrap_or_default();
                let written = rollback
                    .get("written")
                    .and_then(Value::as_str)
                    .and_then(|s| STANDARD.decode(s).ok());
                if let (Some(written), Ok(current)) = (written, read(&dir, OPENCODE_CONFIG))
                    && current == written
                {
                    if rollback
                        .get("config_existed")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                    {
                        dir.replace(OPENCODE_CONFIG, &old, 0o600)?;
                    } else {
                        dir.remove_file(OPENCODE_CONFIG)?;
                    }
                }
            }
            dir.remove_file(OPENCODE_LOCK)?;
            dir.create_new(OPENCODE_LOCK, &initial, 0o600)?;
        }
        let prepared = (|| {
            let original = match read(&dir, OPENCODE_CONFIG) {
                Ok(bytes) => Some(bytes),
                Err(e) if e.kind() == io::ErrorKind::NotFound => None,
                Err(e) => return Err(e),
            };
            let mut doc = match &original {
                Some(bytes) => serde_json::from_slice::<Value>(bytes).map_err(io::Error::other)?,
                None => json!({}),
            };
            if doc.is_null() {
                doc = json!({});
            }
            let object = doc.as_object_mut().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "opencode.json must be an object",
                )
            })?;
            let perms = object.entry("permission").or_insert(json!({}));
            if !perms.is_object() {
                *perms = json!({});
            }
            perms["*"] = permission.into();
            if !deny.is_empty() {
                if !perms.get("bash").is_some_and(Value::is_object) {
                    perms["bash"] = json!({});
                }
                perms["bash"]["*"] = permission.into();
                for pattern in deny {
                    perms["bash"][pattern] = "deny".into();
                }
            }
            let written = serde_json::to_vec_pretty(&doc).map_err(io::Error::other)?;
            let mut rollback =
                json!({"config_existed":original.is_some(),"written":STANDARD.encode(&written)});
            if let Some(original) = &original {
                rollback["original"] = STANDARD.encode(original).into();
            }
            let lock = serde_json::to_vec(&json!({"pid":std::process::id(),"rollback":rollback}))
                .map_err(io::Error::other)?;
            dir.replace(OPENCODE_LOCK, &lock, 0o600)?;
            dir.replace(OPENCODE_CONFIG, &written, 0o600)?;
            Ok::<_, io::Error>((original, written, lock))
        })();
        match prepared {
            Ok((original, written, lock)) => Ok(Self {
                dir,
                original,
                written,
                lock,
                restored: false,
            }),
            Err(error) => {
                let _ = dir.remove_file(OPENCODE_LOCK);
                Err(error)
            }
        }
    }
    fn restore(&mut self) -> io::Result<()> {
        if self.restored {
            return Ok(());
        }
        // An externally replaced lock never authorizes us to restore/remove.
        if read(&self.dir, OPENCODE_LOCK)? != self.lock {
            return Err(io::Error::other("OpenCode lock ownership changed"));
        }
        match read(&self.dir, OPENCODE_CONFIG) {
            Ok(current) if current == self.written => match &self.original {
                Some(original) => self.dir.replace(OPENCODE_CONFIG, original, 0o600)?,
                None => self.dir.remove_file(OPENCODE_CONFIG)?,
            },
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        self.dir.remove_file(OPENCODE_LOCK)?;
        self.restored = true;
        Ok(())
    }
}
impl Drop for OpenCodeOverlay {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn paths(root: &Path) -> RuntimePaths {
        RuntimePaths::trial(
            root,
            49011,
            &root.parent().unwrap().join(format!(
                "installed-{}",
                root.file_name().unwrap().to_string_lossy()
            )),
        )
        .unwrap()
    }
    #[test]
    fn claude_settings_and_delegation_are_private_owned_and_removed() {
        let root = tempfile::tempdir().unwrap();
        let paths = paths(root.path());
        let mut hooks = SessionHooks::default();
        let reg = Message {
            session_id: 7,
            token_statusbar: true,
            auto: true,
            orchestration_id: "synthetic".into(),
            ..Default::default()
        };
        let launch = hooks
            .claude_settings(
                &paths,
                &reg,
                Path::new("/synthetic/space dir/many-ai-cli"),
                49011,
                false,
                false,
                false,
            )
            .unwrap();
        assert!(launch.env.is_empty());
        let args = launch.args;
        let file = Path::new(&args[1]);
        assert!(file.starts_with(paths.root()));
        let body: Value = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
        assert_eq!(body["crossSessionInbound"], "accept");
        // The token stays in the environment Claude inherits (U03).
        assert_eq!(
            body["statusLine"]["command"],
            "'/synthetic/space dir/many-ai-cli' usage-relay --provider claude --hub http://127.0.0.1:49011 --session 7"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(file).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let delegation = hooks.delegation(&paths, "claude", 7).unwrap();
        assert_eq!(
            std::fs::read(&delegation[1]).unwrap(),
            DELEGATION_PROMPT.as_bytes()
        );
        hooks.finish().unwrap();
        assert!(!file.exists());
        assert!(!Path::new(&delegation[1]).exists());
    }
    #[test]
    fn audit_u60_production_files_stay_in_the_users_folder_under_unpredictable_names() {
        let home = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::production(home.path()).unwrap();
        let folder = paths.resource(Resource::UsageHooks);
        let mut hooks = SessionHooks::default();
        let reg = Message {
            session_id: 7,
            token_statusbar: true,
            ..Default::default()
        };
        let mut settings = || {
            hooks
                .claude_settings(
                    &paths,
                    &reg,
                    Path::new("/synthetic/many-ai-cli"),
                    49011,
                    false,
                    false,
                    false,
                )
                .unwrap()
                .args[1]
                .clone()
        };
        let first = settings();
        // The same session and process get a different name each time.
        let second = settings();
        assert_ne!(first, second);
        for file in [&first, &second] {
            assert!(Path::new(file).starts_with(&folder), "{file}");
        }
        // A name that is not UTF-8 in the folder does not stop the next file.
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            std::fs::write(
                folder.join(std::ffi::OsStr::from_bytes(b"synthetic-\xff")),
                b"",
            )
            .unwrap();
            assert!(hooks.delegation(&paths, "claude", 7).is_ok());
        }
        hooks.finish().unwrap();
        assert!(!Path::new(&first).exists() && !Path::new(&second).exists());
    }
    #[test]
    fn audit_u49_bounded_settings_confine_file_tools_and_sandbox_commands() {
        let home = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::production(home.path()).unwrap();
        let reg = Message {
            session_id: 7,
            token_statusbar: true,
            ..Default::default()
        };
        let exe = Path::new("/synthetic/many-ai-cli");
        let body = |launch: &ClaudeLaunch| -> Value {
            serde_json::from_slice(&std::fs::read(&launch.args[1]).unwrap()).unwrap()
        };
        // A headless bounded session where Claude Code has its sandbox.
        let mut hooks = SessionHooks::default();
        let launch = hooks
            .claude_settings(&paths, &reg, exe, 49011, false, true, true)
            .unwrap();
        let settings = body(&launch);
        assert!(settings.get("statusLine").is_none());
        assert_eq!(
            settings["permissions"]["blockWritesOutsideWorkingDirectories"],
            true
        );
        let sandbox = &settings["sandbox"];
        assert_eq!(sandbox["enabled"], true);
        assert_eq!(sandbox["failIfUnavailable"], true);
        assert_eq!(sandbox["allowUnsandboxedCommands"], false);
        assert_eq!(sandbox["autoAllowBashIfSandboxed"], false);
        // Go's build cache is the session's own folder, the one place
        // outside the working folder its commands may write.
        let [(name, cache)] = launch.env.as_slice() else {
            panic!("{:?}", launch.env)
        };
        assert_eq!(name, "GOCACHE");
        let cache = Path::new(cache);
        assert_eq!(
            sandbox["filesystem"]["allowWrite"],
            json!([cache.to_string_lossy()])
        );
        assert!(cache.is_dir());
        assert!(cache.starts_with(paths.resource(Resource::UsageHooks)));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(cache).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        std::fs::create_dir(cache.join("00")).unwrap();
        std::fs::write(cache.join("00").join("synthetic-d"), b"synthetic").unwrap();
        hooks.finish().unwrap();
        assert!(!cache.exists());
        assert!(!Path::new(&launch.args[1]).exists());
        // Without the sandbox (native Windows) the file tools are still kept
        // in the working folders.
        let mut hooks = SessionHooks::default();
        let launch = hooks
            .claude_settings(&paths, &reg, exe, 49011, true, false, true)
            .unwrap();
        let settings = body(&launch);
        assert!(settings.get("sandbox").is_none());
        assert!(launch.env.is_empty());
        assert_eq!(
            settings["permissions"]["blockWritesOutsideWorkingDirectories"],
            true
        );
        assert!(settings["statusLine"].is_object());
        hooks.finish().unwrap();
        // A headless session outside the bounded tier gets no file at all,
        // and an interactive one only its status line.
        let mut hooks = SessionHooks::default();
        let none = hooks
            .claude_settings(&paths, &reg, exe, 49011, false, true, false)
            .unwrap();
        assert!(none.args.is_empty() && none.env.is_empty());
        let plain = hooks
            .claude_settings(&paths, &reg, exe, 49011, false, false, false)
            .unwrap();
        let settings = body(&plain);
        assert!(settings.get("permissions").is_none() && settings.get("sandbox").is_none());
        hooks.finish().unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn audit_u49_stale_session_folders_are_removed_without_following_links() {
        let home = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::production(home.path()).unwrap();
        let (root, _, prefix) = SessionHooks::own_folder(&paths, "gocache").unwrap();
        // A process number no system gives out, so the owner is gone.
        let stale = root.join(format!("{prefix}9-p3000000000-0123456789abcdef"));
        std::fs::create_dir_all(stale.join("ff")).unwrap();
        std::fs::write(stale.join("ff").join("synthetic-a"), b"synthetic").unwrap();
        let outside = home.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("keep"), b"synthetic").unwrap();
        let link = root.join(format!("{prefix}9-p3000000000-fedcba9876543210"));
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        let two_days_ago = SystemTime::now() - Duration::from_secs(48 * 3600);
        std::fs::File::open(&stale)
            .unwrap()
            .set_modified(two_days_ago)
            .unwrap();
        let mut hooks = SessionHooks::default();
        let reg = Message {
            session_id: 7,
            ..Default::default()
        };
        hooks
            .claude_settings(
                &paths,
                &reg,
                Path::new("/synthetic/many-ai-cli"),
                49011,
                false,
                true,
                true,
            )
            .unwrap();
        assert!(!stale.exists());
        assert_eq!(std::fs::read(outside.join("keep")).unwrap(), b"synthetic");
        hooks.finish().unwrap();
    }
    #[test]
    fn opencode_overlay_restores_original_and_preserves_concurrent_edit() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join(OPENCODE_CONFIG);
        let original = b"{\"theme\":\"synthetic\",\"permission\":{\"read\":\"allow\"}}";
        std::fs::write(&file, original).unwrap();
        {
            let mut hooks = SessionHooks::default();
            hooks
                .opencode(root.path(), "allow", &["git push*".into()])
                .unwrap();
            let current: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
            assert_eq!(current["permission"]["read"], "allow");
            assert_eq!(current["permission"]["bash"]["git push*"], "deny");
            assert!(
                SessionHooks::default()
                    .opencode(root.path(), "ask", &[])
                    .is_err()
            );
            hooks.finish().unwrap();
        }
        assert_eq!(std::fs::read(&file).unwrap(), original);
        {
            let mut hooks = SessionHooks::default();
            hooks.opencode(root.path(), "allow", &[]).unwrap();
            std::fs::write(&file, b"{\"user_edit\":true}").unwrap();
            hooks.finish().unwrap();
        }
        assert_eq!(std::fs::read(&file).unwrap(), b"{\"user_edit\":true}");
        assert!(!root.path().join(OPENCODE_LOCK).exists());
    }
    #[test]
    fn opencode_recovers_stale_rollback_before_new_overlay() {
        let root = tempfile::tempdir().unwrap();
        let original = b"{\"original\":true}";
        let written = b"{\"permission\":{\"*\":\"allow\"}}";
        std::fs::write(root.path().join(OPENCODE_CONFIG), written).unwrap();
        let lock = json!({"pid":i64::MAX,"rollback":{"config_existed":true,"original":STANDARD.encode(original),"written":STANDARD.encode(written)}});
        std::fs::write(
            root.path().join(OPENCODE_LOCK),
            serde_json::to_vec(&lock).unwrap(),
        )
        .unwrap();
        let mut hooks = SessionHooks::default();
        hooks.opencode(root.path(), "ask", &[]).unwrap();
        hooks.finish().unwrap();
        assert_eq!(
            std::fs::read(root.path().join(OPENCODE_CONFIG)).unwrap(),
            original
        );
    }
    #[cfg(unix)]
    #[test]
    fn opencode_symlink_configuration_is_not_followed() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("unrelated.json");
        std::fs::write(&target, b"synthetic-owned-but-unrelated").unwrap();
        symlink(&target, root.path().join(OPENCODE_CONFIG)).unwrap();
        assert!(
            SessionHooks::default()
                .opencode(root.path(), "allow", &[])
                .is_err()
        );
        assert_eq!(
            std::fs::read(target).unwrap(),
            b"synthetic-owned-but-unrelated"
        );
        assert!(!root.path().join(OPENCODE_LOCK).exists());
    }

    #[test]
    fn audit_b1_project_settings_that_widen_the_sandbox_are_found_up_to_the_git_root() {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("repo");
        let work = repo.join("sub").join("dir");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let write = |dir: &Path, name: &str, body: &str| {
            std::fs::create_dir_all(dir.join(".claude")).unwrap();
            std::fs::write(dir.join(".claude").join(name), body).unwrap();
        };
        // Nothing, or settings that do not widen the sandbox.
        assert_eq!(project_widens_sandbox(&work), None);
        write(
            &repo,
            "settings.json",
            r#"{"permissions":{"allow":["Read"]},"sandbox":{"excludedCommands":[],"network":{"allowedDomains":[]}},"env":{"A":"1"}}"#,
        );
        assert_eq!(project_widens_sandbox(&work), None);
        // Each widening list is found, in the folder or above it up to the Git root.
        for (body, key) in [
            (
                r#"{"sandbox":{"excludedCommands":["docker"]}}"#,
                "sandbox.excludedCommands",
            ),
            (
                r#"{"sandbox":{"filesystem":{"allowWrite":["~/"]}}}"#,
                "sandbox.filesystem.allowWrite",
            ),
            (
                r#"{"sandbox":{"network":{"allowedDomains":["example.com"]}}}"#,
                "sandbox.network.allowedDomains",
            ),
        ] {
            write(&repo, "settings.local.json", body);
            let reason = project_widens_sandbox(&work).unwrap();
            assert!(reason.ends_with(key), "{reason}");
            assert!(reason.contains("settings.local.json"), "{reason}");
        }
        std::fs::remove_file(repo.join(".claude").join("settings.local.json")).unwrap();
        write(&work, "settings.json", "{ not json");
        assert!(
            project_widens_sandbox(&work)
                .unwrap()
                .ends_with("is not valid JSON")
        );
        std::fs::remove_file(work.join(".claude").join("settings.json")).unwrap();
        // Above the Git root is not the project's.
        write(
            root.path(),
            "settings.json",
            r#"{"sandbox":{"excludedCommands":["docker"]}}"#,
        );
        assert_eq!(project_widens_sandbox(&work), None);
    }
}
