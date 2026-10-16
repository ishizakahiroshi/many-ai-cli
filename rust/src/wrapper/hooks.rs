//! Session-owned files. Persistent shell profiles and shared Claude settings are
//! never changed. OpenCode's required project overlay retains rollback evidence.
pub mod codex;
use crate::{config::RuntimePaths, files::safe_fs::Dir, process::pid_alive, proto::Message};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::{self, Read},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

pub const DELEGATION_PROMPT: &str = include_str!("delegation_prompt.txt");
/// The one line added to a child AI's delegation prompt while many-ai-cli's own
/// settings have mistakes. It carries a count only: neither what the mistakes
/// are nor any setting value, because the prompt goes to the AI vendor.
pub fn config_problems_notice(count: usize) -> Option<String> {
    (count > 0).then(|| {
        format!(
            "many-ai-cli: its settings have {count} mistake{}. Run `many-ai-cli config check --json` to see them, tell the user, and fix them only after that.",
            if count == 1 { "" } else { "s" }
        )
    })
}
/// The same one line for the whole file state: a settings file that cannot be
/// read at all is not "0 mistakes", it is its own message (still a fixed
/// sentence, with no position or value from the file).
pub fn config_state_notice(
    state: &crate::config::ConfigFileState,
    removed_by_save: usize,
) -> Option<String> {
    match state {
        crate::config::ConfigFileState::Unreadable(_) => Some(
            "many-ai-cli: its settings file cannot be read, so no new session can be started. Run `many-ai-cli config check --json` to see why, tell the user, and fix it only after that."
                .to_owned(),
        ),
        // Items a save by the Hub took out of the file count as mistakes too: they
        // are still waiting for someone to look at them.
        other => config_problems_notice(other.problems().len() + removed_by_save),
    }
}
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
#[derive(Default)]
pub struct SessionHooks {
    files: Vec<OwnedFile>,
    opencode: Option<OpenCodeOverlay>,
}
fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
fn read(dir: &Dir, name: &str) -> io::Result<Vec<u8>> {
    let mut bytes = vec![];
    dir.open_file(name, false)?.read_to_end(&mut bytes)?;
    Ok(bytes)
}
impl SessionHooks {
    fn write(
        &mut self,
        paths: &RuntimePaths,
        kind: &str,
        session_id: i64,
        extension: &str,
        bytes: &[u8],
    ) -> io::Result<PathBuf> {
        let root = paths.usage_hook_temporary_dir(&std::env::temp_dir());
        // Never restrict the shared system temporary directory. Only the
        // selected trial-owned directory is created/restricted here.
        let directory = if paths.is_trial() {
            Dir::open_or_create_private(&root)?
        } else {
            Dir::open(&root)?
        };
        let home = if paths.is_trial() {
            paths.root()
        } else {
            paths.root().parent().unwrap_or(paths.root())
        };
        let owner = Sha256::digest(home.to_string_lossy().as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let owner = &owner[..16];
        let prefix = format!("aac-{kind}-u{owner}-s");
        let suffix = format!(".{extension}");
        for name in directory.entries()? {
            let Some(body) = name
                .strip_prefix(&prefix)
                .and_then(|v| v.strip_suffix(&suffix))
            else {
                continue;
            };
            let Some((session, pid)) = body.split_once("-p") else {
                continue;
            };
            if session.parse::<u64>().is_err() {
                continue;
            }
            let Some(pid) = pid.parse::<i64>().ok().filter(|pid| *pid > 0) else {
                continue;
            };
            if pid_alive(pid) {
                continue;
            }
            let Ok(metadata) = directory.metadata(&name) else {
                continue;
            };
            if metadata.is_file()
                && metadata
                    .modified()
                    .ok()
                    .and_then(|mtime| SystemTime::now().duration_since(mtime).ok())
                    .is_some_and(|age| age > Duration::from_secs(24 * 3600))
            {
                let _ = directory.remove_file(&name);
            }
        }
        let name = format!(
            "aac-{kind}-u{owner}-s{session_id}-p{}.{extension}",
            std::process::id()
        );
        directory.create_new(&name, bytes, 0o600)?;
        let path = root.join(&name);
        self.files.push(OwnedFile { directory, name });
        Ok(path)
    }
    pub fn claude_settings(
        &mut self,
        paths: &RuntimePaths,
        registered: &Message,
        executable: &Path,
        hub_port: u16,
        hub_token: &str,
        native_windows: bool,
    ) -> io::Result<Vec<String>> {
        let cross = !native_windows && registered.auto && !registered.orchestration_id.is_empty();
        if !registered.token_statusbar && !cross {
            return Ok(vec![]);
        }
        let mut doc = serde_json::Map::new();
        if registered.token_statusbar {
            let command = format!(
                "MANY_AI_CLI_HUB_TOKEN={} {} usage-relay --provider claude --hub http://127.0.0.1:{hub_port} --session {}",
                quote(hub_token),
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
        let mut bytes = serde_json::to_vec_pretty(&doc).map_err(io::Error::other)?;
        bytes.push(b'\n');
        let path = self.write(
            paths,
            "claude-settings",
            registered.session_id,
            "json",
            &bytes,
        )?;
        Ok(vec![
            "--settings".into(),
            path.to_string_lossy().into_owned(),
        ])
    }
    pub fn delegation(
        &mut self,
        paths: &RuntimePaths,
        provider: &str,
        session_id: i64,
    ) -> io::Result<Vec<String>> {
        self.delegation_with_notice(paths, provider, session_id, None)
    }
    /// `notice` is one extra line appended to the delegation prompt (see
    /// `config_problems_notice`).
    pub fn delegation_with_notice(
        &mut self,
        paths: &RuntimePaths,
        provider: &str,
        session_id: i64,
        notice: Option<&str>,
    ) -> io::Result<Vec<String>> {
        let prompt = match notice {
            Some(line) => format!("{DELEGATION_PROMPT}\n{line}\n"),
            None => DELEGATION_PROMPT.to_owned(),
        };
        match provider {
            "claude" => {
                let path = self.write(paths, "delegation", session_id, "md", prompt.as_bytes())?;
                Ok(vec![
                    "--append-system-prompt-file".into(),
                    path.to_string_lossy().into_owned(),
                ])
            }
            "grok" => Ok(vec!["--rules".into(), prompt]),
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
        let args = hooks
            .claude_settings(
                &paths,
                &reg,
                Path::new("/synthetic/space dir/many-ai-cli"),
                49011,
                "synthetic-token",
                false,
            )
            .unwrap();
        let file = Path::new(&args[1]);
        assert!(file.starts_with(paths.root()));
        let body: Value = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
        assert_eq!(body["crossSessionInbound"], "accept");
        assert!(
            body["statusLine"]["command"]
                .as_str()
                .unwrap()
                .contains("'/synthetic/space dir/many-ai-cli'")
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
    fn delegation_prompt_gets_one_count_only_line_while_settings_have_mistakes() {
        assert_eq!(config_problems_notice(0), None);
        let one = config_problems_notice(1).unwrap();
        assert!(one.contains("1 mistake."));
        assert!(config_problems_notice(3).unwrap().contains("3 mistakes."));
        assert!(one.contains("many-ai-cli config check --json"));
        assert!(!one.contains('\n'));
        let root = tempfile::tempdir().unwrap();
        let paths = paths(root.path());
        let mut hooks = SessionHooks::default();
        let with = hooks
            .delegation_with_notice(&paths, "claude", 8, Some(&one))
            .unwrap();
        let text = String::from_utf8(std::fs::read(&with[1]).unwrap()).unwrap();
        assert!(text.starts_with(DELEGATION_PROMPT));
        assert_eq!(text.lines().filter(|l| l.contains(&one)).count(), 1);
        let grok = hooks
            .delegation_with_notice(&paths, "grok", 8, Some(&one))
            .unwrap();
        assert!(grok[1].contains(&one));
        // Without mistakes the prompt is exactly the fixed text.
        let plain = hooks.delegation(&paths, "grok", 8).unwrap();
        assert_eq!(plain[1], DELEGATION_PROMPT);
        hooks.finish().unwrap();
    }
    #[test]
    fn an_unreadable_settings_file_is_not_reported_as_zero_mistakes() {
        use crate::config::{ConfigFileState, ConfigUnreadable};
        let unreadable = ConfigFileState::Unreadable(ConfigUnreadable {
            path: PathBuf::from("synthetic-config.yaml"),
            backups: PathBuf::from("synthetic-backups"),
            line: Some(3),
            column: Some(1),
            reason: "YAML syntax error".into(),
            token_readable: false,
        });
        // The count is 0 (an unreadable file lists no entries), yet the child AI is told.
        assert_eq!(unreadable.problems().len(), 0);
        let notice = config_state_notice(&unreadable, 0).unwrap();
        assert!(notice.contains("cannot be read"), "{notice}");
        assert!(notice.contains("many-ai-cli config check --json"));
        assert!(!notice.contains('\n'));
        // No position, path or value from the file reaches the prompt.
        assert!(!notice.contains("synthetic-config.yaml"));
        assert!(!notice.contains("line 3"));
        // Entries with mistakes keep the count line; a clean file adds nothing.
        let clean = ConfigFileState::Readable {
            path: PathBuf::from("c.yaml"),
            problems: vec![],
        };
        assert_eq!(config_state_notice(&clean, 0), None);
        // Two items a save took out of the file: counted, though the file reads cleanly.
        assert!(
            config_state_notice(&clean, 2)
                .unwrap()
                .contains("2 mistakes.")
        );
        assert_eq!(
            config_state_notice(
                &ConfigFileState::Missing {
                    path: PathBuf::from("c.yaml")
                },
                0
            ),
            None
        );
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
}
