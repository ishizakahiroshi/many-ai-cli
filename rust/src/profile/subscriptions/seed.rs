use super::{SeedNotice, check_path, default_dir, env_value, invalid, links};
use crate::{
    config::{Resource, RuntimePaths, SubscriptionProfile},
    files::safe_fs::Dir,
    profile::{
        persistence::seed_codex,
        seed::{SettingsFormat, SyncOptions, merge_toml_preferences},
    },
};
use serde_json::value::RawValue;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, Read},
    path::{Path, PathBuf},
};
use toml_edit::DocumentMut;
const LIMIT: usize = 8 * 1024 * 1024;
const CLAUDE_STATE: &[&str] = &[
    "autoMode",
    "modelSettings",
    "tui",
    "theme",
    "effortLevel",
    "skipDangerousModePermissionPrompt",
    "skipAutoPermissionPrompt",
    "skipWorkflowUsageWarning",
    "agentPushNotifEnabled",
    "voice",
    "voiceEnabled",
    "autoUpdatesChannel",
    "switchModelsOnFlag",
];
#[derive(Clone, Copy)]
enum Kind {
    Mirror,
    Directory,
    ClaudeState,
    Json,
    Toml,
    Codex,
}
struct Entry {
    source: PathBuf,
    dest: &'static str,
    kind: Kind,
}
fn entries(
    paths: &RuntimePaths,
    home: &Path,
    hub_cwd: &Path,
    environment: &[String],
    provider: &str,
) -> io::Result<Vec<Entry>> {
    if provider == "opencode" {
        return Ok(vec![]);
    }
    let base = default_dir(paths, home, hub_cwd, environment, provider)?;
    let specs: &[(&str, Kind)] = match provider {
        "claude" => &[
            ("CLAUDE.md", Kind::Mirror),
            ("settings.json", Kind::Json),
            ("commands", Kind::Directory),
            ("skills", Kind::Directory),
            ("agents", Kind::Directory),
        ],
        "codex" => &[
            ("AGENTS.md", Kind::Mirror),
            ("config.toml", Kind::Codex),
            ("prompts", Kind::Directory),
        ],
        "grok" => &[
            ("AGENTS.md", Kind::Mirror),
            ("config.toml", Kind::Toml),
            ("skills", Kind::Directory),
        ],
        _ => return Err(invalid("unsupported profile seeding")),
    };
    let mut entries = specs
        .iter()
        .map(|(dest, kind)| Entry {
            source: base.join(dest),
            dest,
            kind: *kind,
        })
        .collect::<Vec<_>>();
    if provider == "claude" {
        let explicit = env_value(environment, "CLAUDE_CONFIG_DIR")
            .unwrap_or("")
            .trim();
        let source = if !explicit.is_empty()
            && !super::clean(Path::new(explicit))
                .starts_with(paths.resource(Resource::Subscriptions))
        {
            base.join(".claude.json")
        } else {
            home.join(".claude.json")
        };
        entries.push(Entry {
            source,
            dest: ".claude.json",
            kind: Kind::ClaudeState,
        });
    }
    Ok(entries)
}
pub struct DiagnosticEntry {
    pub(super) format: Option<SettingsFormat>,
    pub(crate) trial_root: Option<PathBuf>,
    pub source: PathBuf,
    pub dest: &'static str,
    pub label: String,
    pub mirror: bool,
    pub sync_owned: Option<BTreeSet<String>>,
}
impl DiagnosticEntry {
    /// Trial reads retain a no-follow directory handle for every component.
    /// Production diagnostics retain the source vendor configuration link semantics.
    pub fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        let Some(root) = &self.trial_root else {
            return read(path);
        };
        let relative = path
            .strip_prefix(root)
            .map_err(|_| invalid("diagnostic path escapes trial root"))?;
        let mut dir = Dir::open(root)?;
        let parts = relative.components().collect::<Vec<_>>();
        for part in parts.iter().take(parts.len().saturating_sub(1)) {
            let std::path::Component::Normal(name) = part else {
                return Err(invalid("invalid diagnostic path"));
            };
            dir = dir.child_dir(
                name.to_str()
                    .ok_or_else(|| invalid("invalid diagnostic path"))?,
                false,
            )?;
        }
        let Some(std::path::Component::Normal(name)) = parts.last() else {
            return Err(invalid("invalid diagnostic file"));
        };
        let data = dir.read(
            name.to_str()
                .ok_or_else(|| invalid("invalid diagnostic file"))?,
            LIMIT + 1,
        )?;
        if data.len() > LIMIT {
            return Err(invalid("diagnostic file exceeds size limit"));
        }
        Ok(data)
    }
}
pub fn diagnostic_entries(
    paths: &RuntimePaths,
    home: &Path,
    cwd: &Path,
    environment: &[String],
    provider: &str,
    profile: &SubscriptionProfile,
) -> io::Result<Vec<DiagnosticEntry>> {
    entries(paths, home, cwd, environment, provider)?
        .into_iter()
        .map(|entry| {
            check_path(paths, &entry.source)?;
            let label = match (provider, entry.dest) {
                (_, "CLAUDE.md") => "共通ルール（CLAUDE.md）",
                (_, "AGENTS.md") => "共通ルール（AGENTS.md）",
                (_, "settings.json") => "ユーザー設定（settings.json・起動時に既定と同期）",
                ("codex", "config.toml") => {
                    "設定（config.toml・起動時に既定と同期。信頼済みフォルダ等は profile が持つ）"
                }
                ("grok", "config.toml") => {
                    "設定（config.toml・起動時に既定と同期。画面の切替は profile が持つ）"
                }
                (_, "commands") => "スラッシュコマンド（commands/）",
                (_, "skills") => "スキル（skills/）",
                (_, "agents") => "サブエージェント（agents/）",
                (_, "prompts") => "カスタム スラッシュコマンド（prompts/）",
                (_, ".claude.json") => "ブラウザ操作の既定（.claude.json の名指しキーのみ）",
                _ => entry.dest,
            };
            let sync_owned = if profile.settings_sync == Some(false) {
                None
            } else {
                match entry.kind {
                    Kind::Json => Some(owned(profile, CLAUDE_STATE)),
                    Kind::Toml => Some(owned(profile, &["cli", "ui"])),
                    Kind::Codex => Some(owned(profile, crate::profile::seed::CODEX_STATE_KEYS)),
                    _ => None,
                }
            };
            Ok(DiagnosticEntry {
                format: match entry.kind {
                    Kind::Codex => Some(SettingsFormat::Codex),
                    Kind::Toml => Some(SettingsFormat::Grok),
                    Kind::Json => Some(SettingsFormat::Claude),
                    _ => None,
                },
                trial_root: paths.is_trial().then(|| paths.root().to_path_buf()),
                source: entry.source,
                dest: entry.dest,
                label: label.into(),
                mirror: matches!(entry.kind, Kind::Mirror),
                sync_owned,
            })
        })
        .collect()
}
pub fn sync_drift(
    entry: &DiagnosticEntry,
    destination: &Path,
) -> io::Result<(Vec<String>, Vec<String>)> {
    let Some(owned) = &entry.sync_owned else {
        return Ok((vec![], vec![]));
    };
    let defaults = entry.read(&entry.source)?;
    let current = entry.read(destination)?;
    let (mut added, mut changed) = (vec![], vec![]);
    if entry.dest.ends_with(".toml") {
        let parse = |data: &[u8]| {
            std::str::from_utf8(data)
                .ok()
                .and_then(|v| v.parse::<DocumentMut>().ok())
                .ok_or_else(|| invalid("settings is not a TOML document"))
        };
        let Some(format) = entry.format else {
            return Ok((added, changed));
        };
        let source = parse(&defaults)?;
        let defaults = format.toml_defaults(&source);
        let current = parse(&current)?;
        format
            .check_toml_policy(&source, &current, &defaults, |key| owned.contains(key))
            .map_err(io::Error::other)?;
        for (key, value) in defaults.iter() {
            if owned.contains(key) {
                continue;
            }
            match current.get(key) {
                None => added.push(key.into()),
                Some(old) if !crate::profile::seed::same_preference(key, old, value) => {
                    changed.push(key.into())
                }
                _ => {}
            }
        }
    } else {
        let parse = |data: &[u8]| {
            serde_json::from_slice::<Option<BTreeMap<String, Box<RawValue>>>>(data)
                .map(|v| v.unwrap_or_default())
                .map_err(|_| invalid("settings is not a JSON object"))
        };
        let current = parse(&current)?;
        let defaults = claude_defaults(&defaults, &current, |key| owned.contains(key))?;
        for (key, value) in defaults {
            match current.get(&key) {
                None => added.push(key),
                Some(old)
                    if serde_json::from_str::<serde_json::Value>(old.get())
                        .ok()
                        .as_ref()
                        != Some(&value) =>
                {
                    changed.push(key)
                }
                _ => {}
            }
        }
    }
    added.sort();
    changed.sort();
    Ok((added, changed))
}
// Both callers hand over the launcher's fields one by one; a parameter struct
// would only rename them.
#[allow(clippy::too_many_arguments)]
pub(super) fn run(
    paths: &RuntimePaths,
    home: &Path,
    hub_cwd: &Path,
    environment: &[String],
    provider: &str,
    destination: &Dir,
    profile: &SubscriptionProfile,
    codex_plugin_installer: &std::sync::Arc<super::codex_browser::Installer>,
) -> io::Result<SeedNotice> {
    let mut notice = SeedNotice::default();
    for entry in entries(paths, home, hub_cwd, environment, provider)? {
        check_path(paths, &entry.source)?;
        match std::fs::symlink_metadata(&entry.source) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        }
        let exists = destination.metadata(entry.dest).is_ok()
            || std::fs::symlink_metadata(destination.path().join(entry.dest)).is_ok();
        if exists && !matches!(entry.kind, Kind::Json | Kind::Toml | Kind::Codex) {
            continue;
        }
        let result = apply(&entry, destination, profile, exists);
        match result {
            Ok(None) => {}
            Ok(Some((degraded, changed))) => {
                if exists {
                    notice.synced.extend(changed);
                } else {
                    notice.applied.push(entry.dest.into());
                    if degraded {
                        notice.degraded.push(entry.dest.into());
                    }
                }
            }
            // A missing implementation is different from Go's ordinary per-file
            // seed I/O failure. Do not pretend an unsupported seed succeeded.
            Err(error)
                if error.kind() == io::ErrorKind::Unsupported
                    || matches!(entry.kind, Kind::Json | Kind::Codex | Kind::Toml) =>
            {
                return Err(error);
            }
            Err(_) => notice.failed.push(entry.dest.into()),
        }
    }
    if provider == "codex" {
        // Codex Desktop's browser plugins live only in the default home.
        let bundled = default_dir(paths, home, hub_cwd, environment, provider)?
            .join(".tmp")
            .join("bundled-marketplaces");
        if check_path(paths, &bundled).is_ok() {
            super::codex_browser::ensure(
                &bundled,
                destination.path(),
                environment,
                hub_cwd,
                codex_plugin_installer,
                &mut notice,
            );
        }
    }
    Ok(notice)
}
fn read(path: &Path) -> io::Result<Vec<u8>> {
    let file = std::fs::File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(invalid("seed source is not a regular file"));
    }
    let mut data = vec![];
    file.take((LIMIT + 1) as u64).read_to_end(&mut data)?;
    if data.len() > LIMIT {
        return Err(invalid("seed source exceeds size limit"));
    }
    Ok(data)
}
fn owned(profile: &SubscriptionProfile, defaults: &[&str]) -> BTreeSet<String> {
    let wins: BTreeSet<_> = profile.default_wins_keys.iter().map(|s| s.trim()).collect();
    defaults
        .iter()
        .copied()
        .chain(profile.profile_owned_keys.iter().map(String::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty() && !wins.contains(s))
        .map(str::to_owned)
        .collect()
}
fn apply(
    entry: &Entry,
    destination: &Dir,
    profile: &SubscriptionProfile,
    exists: bool,
) -> io::Result<Option<(bool, Vec<String>)>> {
    let target = destination.path().join(entry.dest);
    match entry.kind {
        Kind::Directory => {
            links::directory(&entry.source, &target)?;
        }
        Kind::Mirror => {
            if std::fs::symlink_metadata(&entry.source)?
                .file_type()
                .is_symlink()
            {
                match entry
                    .source
                    .canonicalize()
                    .and_then(|source| links::file(&source, &target))
                {
                    Ok(()) => return Ok(Some((false, vec![]))),
                    Err(_) => {
                        destination.create_new(entry.dest, &read(&entry.source)?, 0o600)?;
                        return Ok(Some((true, vec![])));
                    }
                }
            }
            destination.create_new(entry.dest, &read(&entry.source)?, 0o600)?;
        }
        Kind::Codex => {
            let options = SyncOptions {
                enabled: profile.settings_sync.unwrap_or(true),
                profile_owned_keys: profile.profile_owned_keys.clone(),
                default_wins_keys: profile.default_wins_keys.clone(),
            };
            let result = seed_codex(&entry.source, destination, &options)?;
            return Ok(Some((false, result.changed_keys)));
        }
        Kind::ClaudeState => {
            let data = read(&entry.source)?;
            let Ok(mut root) = serde_json::from_slice::<BTreeMap<String, Box<RawValue>>>(&data)
            else {
                return Ok(None);
            };
            root.retain(|key, value| {
                matches!(
                    key.as_str(),
                    "claudeInChromeDefaultEnabled" | "hasCompletedClaudeInChromeOnboarding"
                ) && matches!(value.get(), "true" | "false")
            });
            if root.is_empty() {
                return Ok(None);
            }
            let mut bytes = serde_json::to_vec_pretty(&root)
                .map_err(|_| invalid("cannot encode named seed settings"))?;
            bytes.push(b'\n');
            destination.create_new(entry.dest, &bytes, 0o600)?;
        }
        Kind::Json | Kind::Toml => {
            let source = read(&entry.source)?;
            let existing = if exists {
                destination.read(entry.dest, LIMIT + 1)?
            } else if matches!(entry.kind, Kind::Json) {
                b"{}".to_vec()
            } else {
                vec![]
            };
            if existing.len() > LIMIT {
                return Err(invalid("profile settings exceeds size limit"));
            }
            let profile_owned = if !exists {
                BTreeSet::new()
            } else if matches!(entry.kind, Kind::Json) {
                owned(profile, CLAUDE_STATE)
            } else {
                owned(profile, &["cli", "ui"])
            };
            let (data, changed) = if matches!(entry.kind, Kind::Json) {
                merge_json(
                    &source,
                    &existing,
                    &profile_owned,
                    !exists || profile.settings_sync != Some(false),
                )?
            } else {
                merge_toml(
                    &source,
                    &existing,
                    &profile_owned,
                    !exists || profile.settings_sync != Some(false),
                )?
            };
            if !exists {
                destination.create_new(entry.dest, &data, 0o600)?;
            } else if !changed.is_empty() {
                destination.replace(entry.dest, &data, 0o600)?;
            }
            return Ok(Some((false, changed)));
        }
    }
    Ok(Some((false, vec![])))
}
fn claude_defaults(
    data: &[u8],
    existing: &BTreeMap<String, Box<RawValue>>,
    keep_local: impl Fn(&str) -> bool,
) -> io::Result<BTreeMap<String, serde_json::Value>> {
    let input = serde_json::from_slice::<Option<BTreeMap<String, Box<RawValue>>>>(data)
        .map_err(|_| invalid("default settings is not a JSON object"))?
        .unwrap_or_default();
    let required_local = || io::Error::other(crate::profile::seed::SeedError::LocalPolicyRequired);
    let mut defaults = BTreeMap::new();
    for (key, raw) in input {
        use serde_json::Value;
        let value = || serde_json::from_str::<Value>(raw.get()).map_err(|_| required_local());
        if matches!(raw.get().as_bytes().first(), Some(b'"' | b't' | b'f')) {
            let value = value()?;
            if value
                .as_str()
                .is_some_and(|s| SettingsFormat::Claude.permits_string(&key, s))
                || value.is_boolean() && SettingsFormat::Claude.permits_bool(&key)
            {
                defaults.insert(key, value);
                continue;
            }
        }
        let mut required = match key.as_str() {
            "permissions"
            | "hooks"
            | "sandbox"
            | "allowedMcpServers"
            | "deniedMcpServers"
            | "blockedMarketplaces"
            | "strictKnownMarketplaces"
            | "allowManagedHooksOnly"
            | "allowManagedPermissionRulesOnly"
            | "allowManagedMcpServersOnly"
            | "disableAllHooks"
            | "disableBypassPermissionsMode"
            | "disableAutoMode" => value()?,
            _ => continue,
        };
        let mut portable = false;
        match key.as_str() {
            "permissions" => {
                let rules = required.as_object_mut().ok_or_else(required_local)?;
                // Omitting a grant cannot broaden the new profile's permissions.
                rules.remove("allow");
                rules.remove("additionalDirectories");
                if rules.is_empty() {
                    continue;
                }
                portable = rules.iter().all(|(k, v)| match k.as_str() {
                    "disableBypassPermissionsMode" | "disableAutoMode" => {
                        v.as_str() == Some("disable")
                    }
                    "defaultMode" => matches!(
                        v.as_str(),
                        Some("default" | "plan" | "acceptEdits" | "dontAsk" | "auto")
                    ),
                    "blockWritesOutsideWorkingDirectories" => v.is_boolean(),
                    "ask" | "deny" => v.as_array().is_some_and(|a| {
                        a.iter().all(|v| {
                            matches!(
                                v.as_str(),
                                Some(
                                    "Read"
                                        | "Write"
                                        | "Edit"
                                        | "Bash"
                                        | "WebFetch"
                                        | "WebSearch"
                                        | "Glob"
                                        | "Grep"
                                        | "NotebookEdit"
                                        | "Agent"
                                        | "Task"
                                        | "Skill"
                                )
                            )
                        })
                    }),
                    _ => false,
                });
            }
            "sandbox" => {
                portable = required.as_object().is_some_and(|fields| {
                    fields.iter().all(|(k, v)| {
                        matches!(
                            k.as_str(),
                            "enabled"
                                | "autoAllowBashIfSandboxed"
                                | "allowUnsandboxedCommands"
                                | "failIfUnavailable"
                        ) && v.is_boolean()
                    })
                });
            }
            "hooks" => {
                let hooks = required.as_object_mut().ok_or_else(required_local)?;
                hooks.retain(|event, _| {
                    matches!(
                        event.as_str(),
                        "PreToolUse" | "PermissionRequest" | "UserPromptSubmit"
                    )
                });
                if hooks.is_empty() {
                    continue;
                }
            }
            _ => {}
        }
        if let Some(local) = existing.get(&key) {
            let local: Value = serde_json::from_str(local.get()).map_err(|_| required_local())?;
            let matches = match key.as_str() {
                // Allow additional profile-local controls; every required member
                // must still match before permitting a launch.
                "permissions" | "hooks" => required
                    .as_object()
                    .is_some_and(|fields| fields.iter().all(|(k, v)| local.get(k) == Some(v))),
                _ => local == required,
            };
            if !matches {
                return Err(required_local());
            }
        } else if portable {
            defaults.insert(key, required);
        } else {
            return Err(required_local());
        }
    }
    for (key, value) in &defaults {
        if keep_local(key)
            && matches!(
                key.as_str(),
                "permissions"
                    | "sandbox"
                    | "disableBypassPermissionsMode"
                    | "disableAutoMode"
                    | "allowManagedHooksOnly"
                    | "allowManagedPermissionRulesOnly"
                    | "allowManagedMcpServersOnly"
                    | "disableAllHooks"
            )
            && !existing.get(key).is_some_and(|raw| {
                serde_json::from_str::<serde_json::Value>(raw.get())
                    .ok()
                    .as_ref()
                    == Some(value)
            })
        {
            return Err(required_local());
        }
    }
    defaults.retain(|key, _| !keep_local(key));
    Ok(defaults)
}
fn merge_json(
    defaults: &[u8],
    existing: &[u8],
    owned: &BTreeSet<String>,
    sync: bool,
) -> io::Result<(Vec<u8>, Vec<String>)> {
    type Object = BTreeMap<String, Box<RawValue>>;
    let mut profile = serde_json::from_slice::<Option<Object>>(existing)
        .map_err(|_| invalid("profile settings is not a JSON object"))?
        .unwrap_or_default();
    let defaults = claude_defaults(defaults, &profile, |key| !sync || owned.contains(key))?;
    let mut changed = vec![];
    for (key, value) in defaults {
        // Compare parsed values, while retaining each raw value for writes.
        let same = profile.get(&key).is_some_and(|old| {
            serde_json::from_str::<serde_json::Value>(old.get())
                .ok()
                .as_ref()
                == Some(&value)
        });
        if !same {
            profile.insert(
                key.clone(),
                serde_json::value::to_raw_value(&value)
                    .map_err(|_| invalid("cannot encode profile setting"))?,
            );
            changed.push(key);
        }
    }
    let mut bytes = serde_json::to_vec_pretty(&profile)
        .map_err(|_| invalid("cannot encode profile settings"))?;
    bytes.push(b'\n');
    Ok((bytes, changed))
}
fn merge_toml(
    defaults: &[u8],
    existing: &[u8],
    owned: &BTreeSet<String>,
    sync: bool,
) -> io::Result<(Vec<u8>, Vec<String>)> {
    let parse = |data: &[u8]| {
        std::str::from_utf8(data)
            .ok()
            .and_then(|s| s.parse::<DocumentMut>().ok())
            .ok_or_else(|| invalid("settings is not a TOML document"))
    };
    let source = parse(defaults)?;
    let defaults = SettingsFormat::Grok.toml_defaults(&source);
    let mut profile = parse(existing)?;
    SettingsFormat::Grok
        .check_toml_policy(&source, &profile, &defaults, |k| !sync || owned.contains(k))
        .map_err(io::Error::other)?;
    let changed = if sync {
        merge_toml_preferences(&defaults, &mut profile, owned)
    } else {
        Vec::new()
    };
    Ok((profile.to_string().into_bytes(), changed))
}
