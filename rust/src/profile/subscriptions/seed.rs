use super::{SeedNotice, check_path, default_dir, env_value, invalid, links};
use crate::{
    config::{Resource, RuntimePaths, SubscriptionProfile},
    files::safe_fs::Dir,
    profile::{persistence::seed_codex, seed::SyncOptions},
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
    Copy,
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
            ("trusted_folders.toml", Kind::Copy),
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
                (_, "trusted_folders.toml") => "信頼済みフォルダ（trusted_folders.toml）",
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
        let defaults = parse(&defaults)?;
        let current = parse(&current)?;
        for (key, value) in defaults.iter() {
            if owned.contains(key) {
                continue;
            }
            match current.get(key) {
                None => added.push(key.into()),
                Some(old) if diagnostic_toml(old) != diagnostic_toml(value) => {
                    changed.push(key.into())
                }
                _ => {}
            }
        }
    } else {
        let parse = |data: &[u8]| {
            serde_json::from_slice::<Option<BTreeMap<String, serde_json::Value>>>(data)
                .map(|v| v.unwrap_or_default())
                .map_err(|_| invalid("settings is not a JSON object"))
        };
        let defaults = parse(&defaults)?;
        let current = parse(&current)?;
        for (key, value) in defaults {
            if owned.contains(&key) {
                continue;
            }
            match current.get(&key) {
                None => added.push(key),
                Some(old) if diagnostic_json(old.clone()) != diagnostic_json(value) => {
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
fn diagnostic_json(value: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match value {
        Value::Number(n) => n
            .as_f64()
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        Value::Array(a) => Value::Array(a.into_iter().map(diagnostic_json).collect()),
        Value::Object(o) => Value::Object(
            o.into_iter()
                .map(|(k, v)| (k, diagnostic_json(v)))
                .collect(),
        ),
        other => other,
    }
}
#[derive(PartialEq)]
enum DiagnosticToml {
    None,
    String(String),
    Integer(i64),
    Float(f64),
    Bool(bool),
    Date(String),
    Array(Vec<DiagnosticToml>),
    Map(BTreeMap<String, DiagnosticToml>),
}
fn diagnostic_toml(item: &toml_edit::Item) -> DiagnosticToml {
    match item {
        toml_edit::Item::None => DiagnosticToml::None,
        toml_edit::Item::Value(v) => diagnostic_value(v),
        toml_edit::Item::Table(t) => DiagnosticToml::Map(
            t.iter()
                .map(|(k, v)| (k.into(), diagnostic_toml(v)))
                .collect(),
        ),
        toml_edit::Item::ArrayOfTables(t) => DiagnosticToml::Array(
            t.iter()
                .map(|t| {
                    DiagnosticToml::Map(
                        t.iter()
                            .map(|(k, v)| (k.into(), diagnostic_toml(v)))
                            .collect(),
                    )
                })
                .collect(),
        ),
    }
}
fn diagnostic_value(v: &toml_edit::Value) -> DiagnosticToml {
    use toml_edit::Value;
    match v {
        Value::String(v) => DiagnosticToml::String(v.value().clone()),
        Value::Integer(v) => DiagnosticToml::Integer(*v.value()),
        Value::Float(v) => DiagnosticToml::Float(*v.value()),
        Value::Boolean(v) => DiagnosticToml::Bool(*v.value()),
        Value::Datetime(v) => DiagnosticToml::Date(v.value().to_string()),
        Value::Array(v) => DiagnosticToml::Array(v.iter().map(diagnostic_value).collect()),
        Value::InlineTable(v) => DiagnosticToml::Map(
            v.iter()
                .map(|(k, v)| (k.into(), diagnostic_value(v)))
                .collect(),
        ),
    }
}
pub(super) fn run(
    paths: &RuntimePaths,
    home: &Path,
    hub_cwd: &Path,
    environment: &[String],
    provider: &str,
    destination: &Dir,
    profile: &SubscriptionProfile,
) -> io::Result<SeedNotice> {
    let mut notice = SeedNotice::default();
    for mut entry in entries(paths, home, hub_cwd, environment, provider)? {
        check_path(paths, &entry.source)?;
        if std::fs::symlink_metadata(&entry.source).is_err() {
            continue;
        }
        let exists = destination.metadata(entry.dest).is_ok()
            || std::fs::symlink_metadata(destination.path().join(entry.dest)).is_ok();
        if profile.settings_sync == Some(false) && matches!(entry.kind, Kind::Json | Kind::Toml) {
            entry.kind = Kind::Copy;
        }
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
            Err(error) if error.kind() == io::ErrorKind::Unsupported => return Err(error),
            Err(_) => notice.failed.push(entry.dest.into()),
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
        Kind::Copy => {
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
            root.retain(|key, _| {
                matches!(
                    key.as_str(),
                    "claudeInChromeDefaultEnabled" | "hasCompletedClaudeInChromeOnboarding"
                )
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
            if !exists {
                destination.create_new(entry.dest, &source, 0o600)?;
                return Ok(Some((false, vec![])));
            }
            let existing = destination.read(entry.dest, LIMIT + 1)?;
            if existing.len() > LIMIT {
                return Err(invalid("profile settings exceeds size limit"));
            }
            let (data, changed) = if matches!(entry.kind, Kind::Json) {
                merge_json(&source, &existing, &owned(profile, CLAUDE_STATE))?
            } else {
                merge_toml(&source, &existing, &owned(profile, &["cli", "ui"]))?
            };
            if !changed.is_empty() {
                destination.replace(entry.dest, &data, 0o600)?;
            }
            return Ok(Some((false, changed)));
        }
    }
    Ok(Some((false, vec![])))
}
fn merge_json(
    defaults: &[u8],
    existing: &[u8],
    owned: &BTreeSet<String>,
) -> io::Result<(Vec<u8>, Vec<String>)> {
    type Object = BTreeMap<String, Box<RawValue>>;
    let defaults = serde_json::from_slice::<Option<Object>>(defaults)
        .map_err(|_| invalid("default settings is not a JSON object"))?
        .unwrap_or_default();
    let mut profile = serde_json::from_slice::<Option<Object>>(existing)
        .map_err(|_| invalid("profile settings is not a JSON object"))?
        .unwrap_or_default();
    let mut changed = vec![];
    for (key, value) in defaults {
        if owned.contains(&key) {
            continue;
        }
        // Compare parsed values, while retaining each raw value for writes.
        let same = profile.get(&key).is_some_and(|old| {
            serde_json::from_str::<serde_json::Value>(old.get()).ok()
                == serde_json::from_str::<serde_json::Value>(value.get()).ok()
        });
        if !same {
            profile.insert(key.clone(), value);
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
) -> io::Result<(Vec<u8>, Vec<String>)> {
    let parse = |data: &[u8]| {
        std::str::from_utf8(data)
            .ok()
            .and_then(|s| s.parse::<DocumentMut>().ok())
            .ok_or_else(|| invalid("settings is not a TOML document"))
    };
    let defaults = parse(defaults)?;
    let mut profile = parse(existing)?;
    let mut changed = vec![];
    for (key, value) in defaults.iter() {
        if owned.contains(key) {
            continue;
        }
        if profile
            .get(key)
            .is_some_and(|old| old.to_string() == value.to_string())
        {
            continue;
        }
        profile.insert(key, value.clone());
        changed.push(key.into());
    }
    changed.sort();
    Ok((profile.to_string().into_bytes(), changed))
}
