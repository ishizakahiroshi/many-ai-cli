//! Actual profile writes stay within a pinned directory capability supplied
//! from the runtime subscriptions root. Never open auth.json or copy a tree.
use super::seed::{SyncOptions, merge_codex};
use crate::files::safe_fs::Dir;
use std::{
    io::{self, Read},
    path::Path,
};
const SETTINGS_LIMIT: usize = 8 * 1024 * 1024;
#[derive(Debug)]
pub struct SeedReport {
    pub created: bool,
    pub changed_keys: Vec<String>,
    pub removed_temporary_hooks: usize,
}
/// The source is a named default settings file and may deliberately be a user
/// symlink. It is read-only; destination symlinks are always rejected by Dir.
pub fn seed_codex(
    default_config: &Path,
    profile: &Dir,
    options: &SyncOptions,
) -> io::Result<SeedReport> {
    if !options.enabled {
        match profile.metadata("config.toml") {
            Ok(_) => {
                return Ok(SeedReport {
                    created: false,
                    changed_keys: vec![],
                    removed_temporary_hooks: 0,
                });
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    let file = std::fs::File::open(default_config)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("default settings is not a regular file"));
    }
    let mut source = Vec::new();
    file.take((SETTINGS_LIMIT + 1) as u64)
        .read_to_end(&mut source)?;
    if source.len() > SETTINGS_LIMIT {
        return Err(io::Error::other("default settings exceeds size limit"));
    }
    let existing = match profile.read("config.toml", SETTINGS_LIMIT + 1) {
        Ok(b) if b.len() <= SETTINGS_LIMIT => Some(b),
        Ok(_) => return Err(io::Error::other("profile settings exceeds size limit")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    let default_text = std::str::from_utf8(&source)
        .map_err(|_| io::Error::other("default settings is not UTF-8"))?;
    let profile_text = existing
        .as_deref()
        .map(std::str::from_utf8)
        .transpose()
        .map_err(|_| io::Error::other("profile settings is not UTF-8"))?;
    let result = merge_codex(default_text, profile_text, options).map_err(io::Error::other)?;
    if result.created {
        profile.create_new("config.toml", result.text.as_bytes(), 0o600)?;
    } else if !result.changed_keys.is_empty() {
        profile.replace("config.toml", result.text.as_bytes(), 0o600)?;
    }
    Ok(SeedReport {
        created: result.created,
        changed_keys: result.changed_keys,
        removed_temporary_hooks: result.removed_temporary_hooks,
    })
}
