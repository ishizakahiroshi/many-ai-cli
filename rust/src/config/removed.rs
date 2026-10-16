//! What a save of the Hub took out of `config.yaml`, kept until a person has seen it.
//!
//! The Hub rewrites `config.yaml` from the settings it holds in memory. Anything
//! it could not load (a `custom_providers` item that is not a mapping, has no
//! `id` or `command`, has a value of the wrong type; a second YAML document) is
//! not in memory, so a save leaves it out of the file. The save first copies the
//! whole file to `backups/config/` (see `ConfigStore::write_config`), so nothing
//! is lost. This module is the other half: it records **that and what was
//! left out**, so the person (and an AI) keep being told after the Hub restarts,
//! until they either bring the backup back (`config backup restore <id>`) or say
//! they have seen it (`config check --dismiss-removed`). Writing such items
//! back instead would change what they mean (a second document could become a
//! launchable provider).
//!
//! One record per backup: `config.yaml.bak_<id>.removed.json`, written once
//! when the backup is made and never changed afterwards. There is no shared
//! file to read and write back, so two Hubs (or a Hub and a command) cannot
//! lose each other's notices. A record lists at most `MAX_ITEMS` items and says
//! how many more the backup holds. It is "done" when it is renamed to
//! `...removed.json.dismissed`; nothing here deletes a record.
//!
//! A record that cannot be read (damaged, of the wrong shape, too large) is
//! never taken for "nothing to report": it is listed as a notice of its own and
//! is neither overwritten nor deleted.
//!
//! The record holds the same safe facts a problem holds (position, an id shown
//! the safe way, the kind of mistake) and the name of the backup. Never a line of
//! the file, never the Hub token.
use super::{
    backup::{self, BackupRecord, ConfigBackups, PREFIX},
    model::loadable_custom_provider,
    paths::RuntimePaths,
    safety::{
        CHECK_COMMAND, ConfigProblem, FILE_SECTION, ProblemCode, ProblemText, display_id,
        intern_field, parse_config_bytes,
    },
};
use crate::files::safe_fs::Dir;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeSet, io};

/// The single shared record older versions wrote. Still read (and moved to the
/// per-backup records the next time something is saved or settled).
pub const LEGACY_RECORD_NAME: &str = "removed-items.json";
/// The flag of `config check` that records "I have seen these".
pub const DISMISS_FLAG: &str = "--dismiss-removed";
/// The most items one record lists; the rest are only counted.
pub const MAX_ITEMS: usize = 50;
const RECORD_SUFFIX: &str = ".removed.json";
const DISMISSED_SUFFIX: &str = ".dismissed";
const MIGRATED_SUFFIX: &str = ".migrated";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeKind {
    /// One item a save took out.
    Item,
    /// "and N more", for a save that took out more than `MAX_ITEMS`.
    More,
    /// A record that cannot be read: what it said is not known.
    Unreadable,
}
impl NoticeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Item => "removed-by-save",
            Self::More => "removed-more",
            Self::Unreadable => "record-unreadable",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemovedNotice {
    pub kind: NoticeKind,
    /// The backup that holds the file as it was before the save (`YYYYMMDD_HHmmss[_N]`).
    /// Empty for a record whose name does not say.
    pub backup_id: String,
    pub section: String,
    pub index: Option<usize>,
    pub id: Option<String>,
    pub field: String,
    pub code: String,
    pub removed_at: String,
    /// `More`: how many further items the backup holds.
    pub more: usize,
    /// The record file this notice comes from (a name, never a path).
    pub record_file: String,
    /// `Unreadable`: why it cannot be read (fixed wording).
    pub detail: String,
}

// What is written to disk. Every field is required: a record that lacks one is
// "unreadable", not "empty".
#[derive(Serialize, Deserialize)]
struct StoredItem {
    section: String,
    index: Option<usize>,
    id: Option<String>,
    field: String,
    code: String,
}
#[derive(Serialize, Deserialize)]
struct StoredRecord {
    backup_id: String,
    removed_at: String,
    items: Vec<StoredItem>,
    more: usize,
}
// The shared file of the first version.
#[derive(Deserialize)]
struct LegacyNotice {
    backup_id: String,
    section: String,
    index: Option<usize>,
    id: Option<String>,
    field: String,
    code: String,
    removed_at: String,
}
#[derive(Deserialize)]
struct LegacyRecord {
    notices: Vec<LegacyNotice>,
}

fn record_name(backup_id: &str) -> String {
    format!("{PREFIX}{backup_id}{RECORD_SUFFIX}")
}
/// The backup id a record file is for (`None`: not a record file name).
fn record_file_id(name: &str) -> Option<String> {
    let id = name.strip_prefix(PREFIX)?.strip_suffix(RECORD_SUFFIX)?;
    backup::valid_id(id).then(|| id.to_owned())
}
/// Every name that is a notice record, readable or not.
fn is_record_name(name: &str) -> bool {
    name == LEGACY_RECORD_NAME || (name.starts_with(PREFIX) && name.ends_with(RECORD_SUFFIX))
}

impl RemovedNotice {
    fn item(backup_id: &str, removed_at: &str, record_file: &str, stored: &StoredItem) -> Self {
        Self {
            kind: NoticeKind::Item,
            backup_id: backup_id.to_owned(),
            section: stored.section.clone(),
            index: stored.index.filter(|index| *index > 0),
            // A hand-edited record is not trusted to carry a safe id.
            id: stored.id.as_deref().and_then(display_id),
            field: intern_field(&stored.field).to_owned(),
            code: stored.code.clone(),
            removed_at: removed_at.to_owned(),
            more: 0,
            record_file: record_file.to_owned(),
            detail: String::new(),
        }
    }
    fn more(backup_id: &str, removed_at: &str, record_file: &str, count: usize) -> Self {
        Self {
            kind: NoticeKind::More,
            backup_id: backup_id.to_owned(),
            section: String::new(),
            index: None,
            id: None,
            field: String::new(),
            code: String::new(),
            removed_at: removed_at.to_owned(),
            more: count,
            record_file: record_file.to_owned(),
            detail: String::new(),
        }
    }
    fn unreadable(record_file: &str, backup_id: &str, detail: String) -> Self {
        Self {
            kind: NoticeKind::Unreadable,
            backup_id: backup_id.to_owned(),
            section: String::new(),
            index: None,
            id: None,
            field: String::new(),
            code: String::new(),
            removed_at: String::new(),
            more: 0,
            record_file: record_file.to_owned(),
            detail,
        }
    }
    fn problem(&self) -> ConfigProblem {
        let code = ProblemCode::parse(&self.code).unwrap_or(ProblemCode::IncompatibleType);
        if self.section == FILE_SECTION {
            return ConfigProblem::file_level(intern_field(&self.field), code);
        }
        ConfigProblem::new(
            self.index,
            &self.id,
            intern_field(&self.field),
            code,
            true,
            None,
        )
    }
    /// The English wording of what was wrong (for an `Item`, the same text a
    /// current mistake has; the other kinds carry their own wording).
    pub fn text_en(&self) -> ProblemText {
        match self.kind {
            NoticeKind::Item => self.problem().text_en(),
            NoticeKind::More => ProblemText {
                summary: format!(
                    "{} more item{} a save by the Hub took out of config.yaml",
                    self.more,
                    if self.more == 1 { "" } else { "s" }
                ),
                reason: format!(
                    "Each record lists at most {MAX_ITEMS} items; the rest are not named here but are in the backup {}.",
                    self.backup_id
                ),
                effect: String::new(),
                example: String::new(),
                fix_command: None,
                check_command: CHECK_COMMAND,
            },
            NoticeKind::Unreadable => ProblemText {
                summary: format!("the notice record {}", self.record_file),
                reason: self.detail.clone(),
                effect: String::new(),
                example: String::new(),
                fix_command: None,
                check_command: CHECK_COMMAND,
            },
        }
    }
    /// The command that brings the original file back (`None`: this notice does
    /// not name a backup).
    pub fn restore_command(&self) -> Option<String> {
        (self.kind != NoticeKind::Unreadable && !self.backup_id.is_empty())
            .then(|| format!("many-ai-cli config backup restore {}", self.backup_id))
    }
    pub fn dismiss_command() -> String {
        format!("{CHECK_COMMAND} {DISMISS_FLAG}")
    }
    /// A few lines for a terminal.
    pub fn render_en(&self) -> String {
        let text = self.text_en();
        if self.kind == NoticeKind::Unreadable {
            return format!(
                "{}: it cannot be read ({}).\n  What this means: what a save by the Hub took out of config.yaml cannot be listed from this record. The record file was not changed or deleted. The backups themselves are kept: `many-ai-cli config backup list`.\n  When you have looked at it: `{}` (it renames the record file; nothing is deleted).\n",
                text.summary,
                text.reason,
                Self::dismiss_command(),
            );
        }
        let restore = self.restore_command().unwrap_or_default();
        format!(
            "{}\n  What happened: a save by the Hub left this out of config.yaml, because it could not be loaded. It is no longer in the file.\n  Why it could not be loaded: {}\n  The original is in the backup {}. Bring it back with `{}` (that replaces config.yaml with the backup).\n  When you have seen this: `{}`\n",
            text.summary,
            text.reason,
            self.backup_id,
            restore,
            Self::dismiss_command(),
        )
    }
    /// One line, for places that list warnings.
    pub fn warning_en(&self) -> String {
        let text = self.text_en();
        if self.kind == NoticeKind::Unreadable {
            return format!(
                "{}: it cannot be read ({}); it was not changed. `{}` renames it once you have looked.",
                text.summary,
                text.reason,
                Self::dismiss_command()
            );
        }
        format!(
            "{}: removed by a save of the Hub ({}) The original is in the backup {}; `{}` brings it back.",
            text.summary,
            text.reason,
            self.backup_id,
            self.restore_command().unwrap_or_default()
        )
    }
    pub fn to_json(&self) -> Value {
        let text = self.text_en();
        json!({
            "kind": self.kind.as_str(),
            "section": self.section,
            "index": self.index,
            "id": self.id,
            "field": self.field,
            "code": self.code,
            "backup_id": self.backup_id,
            "removed_at": self.removed_at,
            "more": self.more,
            "record_file": self.record_file,
            "summary": text.summary,
            "reason": text.reason,
            "restore_command": self.restore_command(),
            "dismiss_command": Self::dismiss_command(),
            "check_command": CHECK_COMMAND,
        })
    }
}

fn read_failure(error: &io::Error) -> String {
    if error.kind() == io::ErrorKind::InvalidData {
        "it is larger than a record can be".to_owned()
    } else {
        format!("it could not be read ({})", error.kind())
    }
}

/// The notices in one per-backup record. A record that cannot be read becomes
/// one `Unreadable` notice.
fn read_record_file(backups: &ConfigBackups, name: &str, id: &str) -> Vec<RemovedNotice> {
    let bytes = match backups.record_read(name) {
        Ok(Some(bytes)) => bytes,
        // Gone between the listing and the read: nothing to say.
        Ok(None) => return Vec::new(),
        Err(error) => {
            return vec![RemovedNotice::unreadable(name, id, read_failure(&error))];
        }
    };
    let unreadable = |detail: &str| vec![RemovedNotice::unreadable(name, id, detail.to_owned())];
    let Ok(record) = serde_json::from_slice::<StoredRecord>(&bytes) else {
        return unreadable("it is damaged or is not a record this program wrote");
    };
    if record.backup_id != id {
        return unreadable("its content names another backup than its file name does");
    }
    let mut notices: Vec<_> = record
        .items
        .iter()
        .map(|item| RemovedNotice::item(id, &record.removed_at, name, item))
        .collect();
    if record.more > 0 {
        notices.push(RemovedNotice::more(
            id,
            &record.removed_at,
            name,
            record.more,
        ));
    }
    notices
}

fn read_legacy(backups: &ConfigBackups) -> Vec<RemovedNotice> {
    let name = LEGACY_RECORD_NAME;
    let bytes = match backups.record_read(name) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return Vec::new(),
        Err(error) => return vec![RemovedNotice::unreadable(name, "", read_failure(&error))],
    };
    let Ok(record) = serde_json::from_slice::<LegacyRecord>(&bytes) else {
        return vec![RemovedNotice::unreadable(
            name,
            "",
            "it is damaged or is not a record this program wrote".to_owned(),
        )];
    };
    if record
        .notices
        .iter()
        .any(|notice| !backup::valid_id(&notice.backup_id))
    {
        return vec![RemovedNotice::unreadable(
            name,
            "",
            "it names a backup that cannot be right".to_owned(),
        )];
    }
    record
        .notices
        .iter()
        .map(|notice| {
            RemovedNotice::item(
                &notice.backup_id,
                &notice.removed_at,
                name,
                &StoredItem {
                    section: notice.section.clone(),
                    index: notice.index,
                    id: notice.id.clone(),
                    field: notice.field.clone(),
                    code: notice.code.clone(),
                },
            )
        })
        .collect()
}

/// The notices still standing, oldest first. Nothing here is an error: a
/// record or a folder that cannot be read is itself a notice.
pub fn list(paths: &RuntimePaths) -> io::Result<Vec<RemovedNotice>> {
    let backups = ConfigBackups::new(paths);
    let mut names = match backups.record_names() {
        Ok(names) => names,
        Err(error) => {
            return Ok(vec![RemovedNotice::unreadable(
                "backups/config",
                "",
                format!("the backup folder could not be listed ({})", error.kind()),
            )]);
        }
    };
    names.sort();
    let with_record: BTreeSet<String> = names.iter().filter_map(|n| record_file_id(n)).collect();
    let mut notices = Vec::new();
    for name in &names {
        if name == LEGACY_RECORD_NAME {
            // Items already moved to a per-backup record are not listed twice.
            notices.extend(
                read_legacy(&backups)
                    .into_iter()
                    .filter(|notice| !with_record.contains(&notice.backup_id)),
            );
        } else if let Some(id) = record_file_id(name) {
            notices.extend(read_record_file(&backups, name, &id));
        } else if name.starts_with(PREFIX) && name.ends_with(RECORD_SUFFIX) {
            // Looks like a record but its name does not say which backup.
            notices.push(RemovedNotice::unreadable(
                name,
                "",
                "its name does not name a backup".to_owned(),
            ));
        }
    }
    notices.sort_by_key(|notice| {
        (
            if notice.backup_id.is_empty() {
                (String::new(), 0)
            } else {
                backup::sort_key(&notice.backup_id)
            },
            notice.kind as u8,
        )
    });
    Ok(notices)
}

/// Record files moved aside (`<name>.dismissed`), so they can be put back if
/// the step they were moved for does not happen.
#[derive(Default)]
pub(super) struct Settled {
    moved: Vec<(String, String)>,
}
impl Settled {
    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.moved.is_empty()
    }
    /// Put every moved record back under its own name.
    pub(super) fn undo(&self, backups: &ConfigBackups, root: &Dir) -> io::Result<()> {
        let mut first = None;
        for (original, aside) in self.moved.iter().rev() {
            if let Err(error) = backups.record_rename(root, aside, original) {
                first.get_or_insert(error);
            }
        }
        first.map_or(Ok(()), Err)
    }
}

/// Move `name` aside under `<name>.dismissed` (`_2`, `_3` ... if taken).
fn move_aside(backups: &ConfigBackups, root: &Dir, name: &str, suffix: &str) -> io::Result<String> {
    for attempt in 1..=99 {
        let aside = if attempt == 1 {
            format!("{name}{suffix}")
        } else {
            format!("{name}{suffix}_{attempt}")
        };
        if backups.record_exists(root, &aside) {
            continue;
        }
        match backups.record_rename(root, name, &aside) {
            Ok(()) => return Ok(aside),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::other(
        "no free name to move a notice record aside",
    ))
}

/// Move all of `names` aside; if one cannot be, the ones already moved are put back.
fn settle(backups: &ConfigBackups, root: &Dir, names: &[String]) -> io::Result<Settled> {
    let mut done = Settled::default();
    for name in names {
        match move_aside(backups, root, name, DISMISSED_SUFFIX) {
            Ok(aside) => done.moved.push((name.clone(), aside)),
            Err(error) => {
                return Err(match done.undo(backups, root) {
                    Ok(()) => error,
                    Err(undo) => backup::partial_failure(format!(
                        "a notice record ({name}) could not be set aside ({error}), and the ones already set aside could not be put back ({undo}); they are the `*.dismissed` files in the backup folder"
                    )),
                });
            }
        }
    }
    Ok(done)
}

fn all_record_names(backups: &ConfigBackups) -> io::Result<Vec<String>> {
    let mut names: Vec<String> = backups
        .record_names()?
        .into_iter()
        .filter(|name| is_record_name(name))
        .collect();
    names.sort();
    Ok(names)
}

/// "I have seen these": set every record aside (readable or not). Returns what
/// was listed. Nothing is deleted: `<name>.dismissed` stays in the backup folder.
pub fn dismiss_all(paths: &RuntimePaths) -> io::Result<Vec<RemovedNotice>> {
    let backups = ConfigBackups::new(paths);
    let listed = list(paths)?;
    let names = all_record_names(&backups)?;
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let root = Dir::open(paths.root())?;
    settle(&backups, &root, &names)?;
    Ok(listed)
}

/// `config reset` replaces everything on purpose: no notice is left standing.
/// Settled before the file is replaced; the caller puts them back (`undo`) if
/// the reset does not happen.
pub(super) fn settle_for_reset(backups: &ConfigBackups, root: &Dir) -> io::Result<Settled> {
    settle(backups, root, &all_record_names(backups)?)
}

/// A restore brings back `restored` (whose bytes are `bytes`). The notices
/// about it are done, and so are the notices about any other backup with the
/// very same bytes: they hold the same items. Settled before the file is
/// replaced; the caller puts them back (`undo`) if the replacement fails.
pub(super) fn settle_for_restore(
    backups: &ConfigBackups,
    root: &Dir,
    restored: &BackupRecord,
    bytes: &[u8],
) -> io::Result<Settled> {
    // Best effort: an old shared record that cannot be moved stays as it is.
    let _ = migrate_legacy(backups, root);
    let all = backups.list()?;
    let mut chosen = Vec::new();
    for name in backups.record_names()? {
        let Some(id) = record_file_id(&name) else {
            continue;
        };
        let same = id == restored.id
            || all.iter().any(|other| {
                other.id == id
                    && other.bytes == bytes.len() as u64
                    && backups.read(other).is_ok_and(|other| other == bytes)
            });
        if same {
            chosen.push(name);
        }
    }
    chosen.sort();
    settle(backups, root, &chosen)
}

/// Move the readable items of an old shared record into per-backup records. A
/// record that cannot be read stays where it is (and is listed as unreadable).
fn migrate_legacy(backups: &ConfigBackups, root: &Dir) -> io::Result<()> {
    let Some(bytes) = backups.record_read(LEGACY_RECORD_NAME)? else {
        return Ok(());
    };
    let record: LegacyRecord = serde_json::from_slice(&bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "damaged old record"))?;
    if record
        .notices
        .iter()
        .any(|notice| !backup::valid_id(&notice.backup_id))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "old record names no backup",
        ));
    }
    let mut ids: Vec<&str> = Vec::new();
    for notice in &record.notices {
        if !ids.contains(&notice.backup_id.as_str()) {
            ids.push(&notice.backup_id);
        }
    }
    for id in ids {
        let name = record_name(id);
        if backups.record_exists(root, &name) {
            continue;
        }
        let own: Vec<&LegacyNotice> = record
            .notices
            .iter()
            .filter(|notice| notice.backup_id == id)
            .collect();
        let stored = StoredRecord {
            backup_id: id.to_owned(),
            removed_at: own[0].removed_at.clone(),
            items: own
                .iter()
                .take(MAX_ITEMS)
                .map(|notice| StoredItem {
                    section: notice.section.clone(),
                    index: notice.index,
                    id: notice.id.clone(),
                    field: notice.field.clone(),
                    code: notice.code.clone(),
                })
                .collect(),
            more: own.len().saturating_sub(MAX_ITEMS),
        };
        let bytes = serde_json::to_vec_pretty(&stored).map_err(io::Error::other)?;
        backups.record_create_new(root, &name, &bytes)?;
    }
    move_aside(backups, root, LEGACY_RECORD_NAME, MIGRATED_SUFFIX)?;
    Ok(())
}

/// What a save that replaces `replaced` (the file as it is now) leaves out:
/// the `custom_providers` items the loader cannot keep and later YAML
/// documents. Empty when the file cannot be read (the whole file is then
/// reported as unreadable instead).
fn left_out_by_a_save(replaced: &[u8], paths: &RuntimePaths) -> Vec<ConfigProblem> {
    let Ok(text) = std::str::from_utf8(replaced) else {
        return Vec::new();
    };
    let Ok((_, problems)) = parse_config_bytes(replaced, paths) else {
        return Vec::new();
    };
    // Positions (0-based, in the list the loader sees) of the items it drops.
    let dropped: Vec<usize> = super::yaml_nodes::decode_config(text)
        .ok()
        .and_then(|decoded| match decoded.get("custom_providers") {
            Some(Value::Array(items)) => Some(
                items
                    .iter()
                    .enumerate()
                    .filter(|(_, item)| loadable_custom_provider((*item).clone()).is_none())
                    .map(|(position, _)| position)
                    .collect(),
            ),
            _ => None,
        })
        .unwrap_or_default();
    problems
        .into_iter()
        .filter(|problem| match problem.code {
            ProblemCode::ExtraDocuments | ProblemCode::NotAList => true,
            _ => {
                problem.is_provider_entry()
                    && problem
                        .index
                        .is_some_and(|index| dropped.contains(&(index - 1)))
            }
        })
        .collect()
}

/// A save is about to replace `replaced` after it was copied to `backup_id`:
/// record what it will leave out, in a record of its own. Nothing to record is
/// not an error. If it cannot be recorded the caller does not replace the file.
pub(super) fn record_replaced(
    backups: &ConfigBackups,
    root: &Dir,
    paths: &RuntimePaths,
    replaced: &[u8],
    backup_id: &str,
) -> io::Result<()> {
    let left_out = left_out_by_a_save(replaced, paths);
    if left_out.is_empty() {
        return Ok(());
    }
    // Best effort: an old shared record moves to the new form now; one that
    // cannot be read stays and keeps being reported.
    let _ = migrate_legacy(backups, root);
    let more = left_out.len().saturating_sub(MAX_ITEMS);
    let record = StoredRecord {
        backup_id: backup_id.to_owned(),
        removed_at: backups.now_text(),
        items: left_out
            .iter()
            .take(MAX_ITEMS)
            .map(|problem| StoredItem {
                section: problem.section.to_owned(),
                index: problem.index,
                id: problem.id.clone(),
                field: problem.field.to_owned(),
                code: problem.code.as_str().to_owned(),
            })
            .collect(),
        more,
    };
    let bytes = serde_json::to_vec_pretty(&record).map_err(io::Error::other)?;
    backups.record_create_new(root, &record_name(backup_id), &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Resource;

    fn trial() -> (tempfile::TempDir, tempfile::TempDir, RuntimePaths) {
        let root = tempfile::tempdir().unwrap();
        let installed = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::trial(root.path(), 49214, installed.path()).unwrap();
        (root, installed, paths)
    }
    fn folder(paths: &RuntimePaths) -> std::path::PathBuf {
        paths.resource(Resource::ConfigBackups)
    }

    #[test]
    fn only_what_the_loader_drops_is_recorded() {
        let (_root, _installed, paths) = trial();
        let text = "token: synthetic-t\ncustom_providers:\n  - id: shell\n    command: x\n  - id: half\n  - id: good\n    command: good\n  - 7\n  - id: quote\n    command: 'a \"b'\n---\nx: 1\n";
        let found = left_out_by_a_save(text.as_bytes(), &paths);
        let summary: Vec<_> = found
            .iter()
            .map(|p| (p.index, p.id.as_deref(), p.code.as_str()))
            .collect();
        // `shell` (reserved) and `quote` (bad command) load, so a save keeps them.
        assert_eq!(
            summary,
            [
                (Some(2), Some("half"), "missing-command"),
                (Some(4), None, "not-a-mapping"),
                (None, None, "extra-documents"),
            ]
        );
        // A file that cannot be read has nothing recorded here.
        assert!(left_out_by_a_save(b"token: x\nhub: [\n", &paths).is_empty());
    }

    #[test]
    fn a_record_per_backup_is_listed_dismissed_per_backup_and_never_deleted() {
        let (_root, _installed, paths) = trial();
        let text = b"token: synthetic-t\ncustom_providers:\n  - id: half\n---\nx: 1\n";
        let backups = ConfigBackups::new(&paths);
        let root = Dir::open_or_create_private(paths.root()).unwrap();
        assert!(list(&paths).unwrap().is_empty());
        record_replaced(&backups, &root, &paths, text, "20261016_100000").unwrap();
        record_replaced(&backups, &root, &paths, text, "20261016_110000").unwrap();
        let notices = list(&paths).unwrap();
        assert_eq!(notices.len(), 4);
        assert!(
            notices[0]
                .render_en()
                .contains("config backup restore 20261016_100000")
        );
        assert_eq!(notices[0].to_json()["kind"], "removed-by-save");
        // Each backup has its own file; nothing shared is written back.
        let names: Vec<_> = std::fs::read_dir(folder(&paths))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names.len(), 2, "{names:?}");
        assert!(
            names
                .iter()
                .all(|n| n.starts_with("config.yaml.bak_") && n.ends_with(".removed.json"))
        );
        // Settling one backup leaves the other.
        let first = BackupRecord {
            id: "20261016_100000".into(),
            file_name: String::new(),
            path: folder(&paths),
            bytes: text.len() as u64,
        };
        let settled = settle_for_restore(&backups, &root, &first, text).unwrap();
        assert!(!settled.is_empty());
        // The other backup does not exist as a file, so it is not "the same bytes".
        let left = list(&paths).unwrap();
        assert_eq!(left.len(), 2);
        assert!(left.iter().all(|n| n.backup_id == "20261016_110000"));
        // Undo puts it back.
        settled.undo(&backups, &root).unwrap();
        assert_eq!(list(&paths).unwrap().len(), 4);
        assert_eq!(dismiss_all(&paths).unwrap().len(), 4);
        assert!(list(&paths).unwrap().is_empty());
        assert!(dismiss_all(&paths).unwrap().is_empty());
        // Nothing was deleted: the records are still there under `.dismissed`.
        let kept = std::fs::read_dir(folder(&paths))
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".dismissed")
            })
            .count();
        assert_eq!(kept, 2);
        // Nothing to record leaves no record file behind.
        record_replaced(&backups, &root, &paths, b"token: x\n", "20261016_120000").unwrap();
        assert_eq!(std::fs::read_dir(folder(&paths)).unwrap().count(), 2);
    }

    #[test]
    fn a_record_is_capped_and_still_readable() {
        let (_root, _installed, paths) = trial();
        let mut text = String::from("token: synthetic-t\ncustom_providers:\n");
        for _ in 0..(MAX_ITEMS + 30) {
            text.push_str("  - 7\n");
        }
        let backups = ConfigBackups::new(&paths);
        let root = Dir::open_or_create_private(paths.root()).unwrap();
        record_replaced(&backups, &root, &paths, text.as_bytes(), "20261016_100000").unwrap();
        let notices = list(&paths).unwrap();
        assert_eq!(notices.len(), MAX_ITEMS + 1);
        let more = notices.last().unwrap();
        assert_eq!((more.kind, more.more), (NoticeKind::More, 30));
        assert!(
            more.render_en().contains("30 more items"),
            "{}",
            more.render_en()
        );
        let size = std::fs::metadata(folder(&paths).join(record_name("20261016_100000")))
            .unwrap()
            .len();
        assert!(size < 64 * 1024, "{size}");
    }

    #[test]
    fn a_record_that_cannot_be_read_is_a_notice_and_is_left_alone() {
        let (_root, _installed, paths) = trial();
        std::fs::create_dir_all(folder(&paths)).unwrap();
        let cases = [
            ("20261016_100000", "{\"backup_id\": \"2026".to_owned()),
            ("20261016_100001", "[]".to_owned()),
            (
                "20261016_100002",
                "{\"backup_id\":\"20261016_100002\",\"removed_at\":\"x\",\"items\":5,\"more\":0}"
                    .to_owned(),
            ),
            (
                "20261016_100003",
                "{\"backup_id\":\"20261016_999999\",\"removed_at\":\"x\",\"items\":[],\"more\":0}"
                    .to_owned(),
            ),
        ];
        for (id, damaged) in &cases {
            std::fs::write(folder(&paths).join(record_name(id)), damaged).unwrap();
        }
        // A folder where a record should be, and an oversized record.
        std::fs::create_dir(folder(&paths).join(LEGACY_RECORD_NAME)).unwrap();
        std::fs::write(
            folder(&paths).join(record_name("20261016_100004")),
            vec![b' '; 2 * 1024 * 1024],
        )
        .unwrap();
        let notices = list(&paths).unwrap();
        assert_eq!(notices.len(), 6, "{notices:?}");
        assert!(notices.iter().all(|n| n.kind == NoticeKind::Unreadable));
        assert!(notices[0].render_en().contains("cannot be read"));
        assert_eq!(notices[0].to_json()["kind"], "record-unreadable");
        for (id, damaged) in &cases {
            assert_eq!(
                std::fs::read_to_string(folder(&paths).join(record_name(id))).unwrap(),
                *damaged,
                "left as it was"
            );
        }
        // They are set aside, not deleted, when the person says they have looked.
        assert_eq!(dismiss_all(&paths).unwrap().len(), 6);
        assert!(list(&paths).unwrap().is_empty());
        assert!(
            folder(&paths)
                .join(format!("{}.dismissed", record_name("20261016_100000")))
                .is_file()
        );
    }

    #[test]
    fn an_old_shared_record_is_read_and_moved_to_records_of_their_own() {
        let (_root, _installed, paths) = trial();
        std::fs::create_dir_all(folder(&paths)).unwrap();
        std::fs::write(
            folder(&paths).join(LEGACY_RECORD_NAME),
            "{\"notices\":[{\"backup_id\":\"20261016_100000\",\"section\":\"custom_providers\",\"index\":2,\"id\":\"half\",\"field\":\"command\",\"code\":\"missing-command\",\"removed_at\":\"2026-10-16 10:00:00\"}]}",
        )
        .unwrap();
        let notices = list(&paths).unwrap();
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].id.as_deref(), Some("half"));
        // Settling moves it to the new form, and it is still listed once.
        let backups = ConfigBackups::new(&paths);
        let root = Dir::open(paths.root()).unwrap();
        migrate_legacy(&backups, &root).unwrap();
        let after = list(&paths).unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].record_file, record_name("20261016_100000"));
        assert!(!folder(&paths).join(LEGACY_RECORD_NAME).exists());
        assert!(
            folder(&paths)
                .join(format!("{LEGACY_RECORD_NAME}.migrated"))
                .is_file()
        );
    }
}
