//! What is wrong with `config.yaml`, said in a form people and tools can use.
//!
//! Two kinds of trouble are kept apart:
//!
//! * The file cannot be read as a whole (`ConfigUnreadable`). Nothing can be
//!   trusted, so the program stops and leaves the file untouched.
//! * One entry of `custom_providers` is wrong (`ConfigProblem`). Only that
//!   entry is left out; everything else keeps working.
//!
//! Nothing here ever repeats a line of the file or a value from it (a line can
//! be the Hub token). Positions are line/column numbers or entry numbers.
//! `ConfigProblem::text_en` is the only place the English wording of a problem
//! is made; the terminal, `config check --json` and `doctor` all read from it,
//! and the screen builds its own wording from the structured fields.
use super::{
    model::{
        CommandProblem, Config, CustomProvider, HeadlessProblem, IdProblem, headless_problem,
        normalize_node, normalize_subscription_id, split_command_typed,
    },
    paths::{Resource, RuntimePaths},
};
use crate::files::safe_fs::Dir;
use serde_json::{Value, json};
use std::{collections::BTreeMap, io, path::PathBuf};

pub const CHECK_COMMAND: &str = "many-ai-cli config check";
pub const RESET_COMMAND: &str = "many-ai-cli config reset";

const LIST_EXAMPLE: &str = "custom_providers:\n  - id: my-cli\n    label: My CLI\n    command: my-cli --flag \"value with spaces\"";
const HEADLESS_EXAMPLE: &str = "custom_providers:\n  - id: my-cli\n    command: my-cli\n    headless:\n      args: [exec]\n      format: text\n      prompt_via: arg";

// ---------------------------------------------------------------------------
// The whole file cannot be read
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigUnreadable {
    /// The settings file.
    pub path: PathBuf,
    /// Where numbered backups are kept.
    pub backups: PathBuf,
    /// 1-based, when the YAML parser could tell.
    pub line: Option<u32>,
    pub column: Option<u32>,
    /// Fixed wording only; never a piece of the file.
    pub reason: String,
    /// The file is valid YAML and still holds a Hub token, so `config reset`
    /// can keep it (the Hub URL does not change).
    pub token_readable: bool,
}
impl ConfigUnreadable {
    pub fn position_en(&self) -> Option<String> {
        match (self.line, self.column) {
            (Some(line), Some(column)) => Some(format!("line {line}, column {column}")),
            (Some(line), None) => Some(format!("line {line}")),
            _ => None,
        }
    }
    /// One line: the problem and where it is.
    pub fn summary_en(&self) -> String {
        match self.position_en() {
            Some(at) => format!("{} (at {at})", self.reason),
            None => self.reason.clone(),
        }
    }
    /// What the terminal shows when a command stops on this file.
    pub fn message_en(&self) -> String {
        let reset = self
            .backups
            .join("config.yaml.bak_<date>_<time>")
            .display()
            .to_string();
        let where_to_look = match self.position_en() {
            Some(at) => format!(" (look at {at})"),
            None => String::new(),
        };
        let token = if self.token_readable {
            "The Hub token in the file is kept, so the Hub URL does not change."
        } else {
            "The Hub token in the broken file cannot be read, so a new token is created and the Hub URL changes."
        };
        format!(
            "many-ai-cli cannot continue because its settings file cannot be read.\n\
             \x20 File:    {}\n\
             \x20 Problem: {}\n\
             \x20 The file has not been changed, and no other settings were used in its place.\n\
             \n\
             Fix it in one of two ways:\n\
             \x20 1. Edit the file and correct it{where_to_look}, then run the command again.\n\
             \x20    `{CHECK_COMMAND}` tells you whether it can be read now.\n\
             \x20 2. Run `{RESET_COMMAND} --yes` to start over from the default settings.\n\
             \x20    The current file is copied first to {reset} so nothing is lost;\n\
             \x20    bring it back with `many-ai-cli config backup restore <date>_<time>`.\n\
             \x20    {token}",
            self.path.display(),
            self.summary_en(),
        )
    }
    /// What a save by the Hub answers when the file on disk cannot be read:
    /// the save is refused, nothing is replaced.
    pub fn save_refused_en(&self) -> String {
        format!(
            "The settings were not saved: the settings file cannot be read ({}), and saving would replace what is written in it. The file has not been changed, and the Hub keeps running with the settings it holds. Fix the file (`{CHECK_COMMAND}` shows where) or run `{RESET_COMMAND} --yes` to start over from the defaults (the current file is saved as a backup), then try again.",
            self.summary_en()
        )
    }
    pub fn to_json(&self) -> Value {
        json!({
            "file": self.path,
            "line": self.line,
            "column": self.column,
            "reason": self.reason,
            "token_readable": self.token_readable,
            "message": self.message_en(),
            "reset_command": format!("{RESET_COMMAND} --yes"),
            "check_command": CHECK_COMMAND,
            "backup_directory": self.backups,
        })
    }
}

/// The whole-file failure inside an `io::Error` that wraps a `ConfigError`
/// (how `MainContext::load` reports it), if that is what it holds.
pub fn unreadable_from_io(error: &io::Error) -> Option<&ConfigUnreadable> {
    error
        .get_ref()?
        .downcast_ref::<super::ConfigError>()?
        .unreadable()
}

impl ConfigUnreadable {
    /// What the terminal shows after `config backup restore` put a copy back
    /// that cannot be read. (`message_en` says the file "has not been changed",
    /// which is not true here: the restore replaced it.)
    pub fn after_restore_en(&self) -> String {
        let where_to_look = match self.position_en() {
            Some(at) => format!(" (look at {at})"),
            None => String::new(),
        };
        format!(
            "The restored copy cannot be read: {}\n\
             \x20 File: {}\n\
             \x20 The file now holds that copy as it was; the settings that were in place before are saved as shown above.\n\
             \x20 Edit the file and correct it{where_to_look}, put another copy back (`many-ai-cli config backup list`),\n\
             \x20 or run `{RESET_COMMAND} --yes` to start over from the default settings.\n\
             \x20 `{CHECK_COMMAND}` tells you whether it can be read.\n",
            self.summary_en(),
            self.path.display(),
        )
    }
}

/// The most this program reads as settings (the YAML reader's own limit).
const READ_LIMIT_BYTES: usize = 8 * 1024 * 1024;

/// What is at `config.yaml`.
pub(super) enum SettingsFile {
    Missing,
    Bytes(Vec<u8>),
    /// More than a backup copies (`backup::COPY_LIMIT`): not read at all. It is
    /// reported as a file that cannot be read, and `config reset` moves it into
    /// the backup folder instead of copying it.
    TooLarge,
}

/// Read the settings file whole (see `backup::read_whole`): a file over the
/// settings limit is still read in full so it can be reported and backed up as
/// it is.
pub(super) fn read_settings_file(root: &Dir) -> io::Result<SettingsFile> {
    match super::backup::read_whole(root, "config.yaml") {
        Ok(bytes) => Ok(SettingsFile::Bytes(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(SettingsFile::Missing),
        Err(error) if super::backup::is_too_large(&error) => Ok(SettingsFile::TooLarge),
        Err(error) => Err(error),
    }
}

impl ConfigUnreadable {
    /// The file is larger than a backup copies.
    pub(super) fn too_large(paths: &RuntimePaths) -> Self {
        Self {
            path: paths.resource(Resource::Config),
            backups: paths.resource(Resource::ConfigBackups),
            line: None,
            column: None,
            reason: format!(
                "the file is larger than {} MiB, far more than any settings file; `{RESET_COMMAND} --yes` moves it into the backup folder (it is not copied) and starts over, or move or delete it yourself",
                super::backup::COPY_LIMIT / (1024 * 1024)
            ),
            token_readable: false,
        }
    }
}

/// Hide an id that is, or contains, the Hub token (a YAML alias can make the
/// token the value of an `id`; a pasted token with a character added is the same
/// secret): a token is never shown, wherever it came from. Ids are compared in
/// the form they are shown in (lower-cased, without control and invisible
/// characters). An empty token hides nothing.
pub(super) fn hide_token_ids(problems: &mut [ConfigProblem], token: &str) {
    let Some(shown) = display_id(token) else {
        return;
    };
    for problem in problems {
        if problem
            .id
            .as_deref()
            .is_some_and(|id| id.contains(shown.as_str()))
        {
            problem.id = None;
        }
    }
}

/// Read settings from file bytes. A failure never carries file content.
pub(super) fn parse_config_bytes(
    bytes: &[u8],
    paths: &RuntimePaths,
) -> Result<(Config, Vec<ConfigProblem>), ConfigUnreadable> {
    let unreadable = |reason: String, location: Option<(usize, usize)>| ConfigUnreadable {
        path: paths.resource(Resource::Config),
        backups: paths.resource(Resource::ConfigBackups),
        line: location.map(|(line, _)| u32::try_from(line).unwrap_or(u32::MAX)),
        column: location.map(|(_, column)| u32::try_from(column).unwrap_or(u32::MAX)),
        reason,
        token_readable: salvage_token(bytes, paths).is_some(),
    };
    if bytes.len() > READ_LIMIT_BYTES {
        return Err(unreadable(
            format!(
                "the file is larger than {} MiB, which is more than is read as settings",
                READ_LIMIT_BYTES / (1024 * 1024)
            ),
            None,
        ));
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        return Err(unreadable("the file is not valid UTF-8 text".into(), None));
    };
    // Why the reader gave up, with a position when one can be found without
    // repeating anything from the file.
    let explain = |message: String| {
        if let Some(location) = super::yaml_nodes::syntax_error_location(text) {
            return unreadable("YAML syntax error".to_owned(), Some(location));
        }
        if let Some(location) = super::yaml_nodes::duplicate_key_location(text) {
            return unreadable(
                "a key is written twice in the same mapping".to_owned(),
                Some(location),
            );
        }
        unreadable(
            message.trim_end_matches(" (content redacted)").to_owned(),
            None,
        )
    };
    match Config::from_yaml_inspected(text, paths) {
        Ok((config, mut problems)) => {
            hide_token_ids(&mut problems, &config.token);
            // Like the Go reader, only the first YAML document is read. Later
            // ones are not checked and have no effect: say so rather than
            // letting someone think they were applied.
            if super::yaml_nodes::has_extra_documents(text) {
                problems.push(ConfigProblem::file_level(
                    "documents",
                    ProblemCode::ExtraDocuments,
                ));
            }
            Ok((config, problems))
        }
        Err(super::ConfigError::Parse(message)) => Err(explain(message)),
        Err(other) => Err(explain(other.to_string())),
    }
}

// ---------------------------------------------------------------------------
// One custom_providers entry is wrong
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProblemCode {
    NotAList,
    NotAMapping,
    IncompatibleType,
    MissingId,
    MissingCommand,
    IdTooLong,
    IdInvalidShape,
    ReservedId,
    DuplicateId,
    CommandUnterminatedQuote,
    CommandControlCharacter,
    HeadlessFormat,
    HeadlessPromptVia,
    HeadlessArgs,
    /// The file has more than one YAML document; only the first is read.
    ExtraDocuments,
}
impl ProblemCode {
    /// Stable machine name (also used in `config check --json`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotAList => "not-a-list",
            Self::NotAMapping => "not-a-mapping",
            Self::IncompatibleType => "incompatible-type",
            Self::MissingId => "missing-id",
            Self::MissingCommand => "missing-command",
            Self::IdTooLong => "id-too-long",
            Self::IdInvalidShape => "id-invalid-shape",
            Self::ReservedId => "reserved-id",
            Self::DuplicateId => "duplicate-id",
            Self::CommandUnterminatedQuote => "command-unterminated-quote",
            Self::CommandControlCharacter => "command-control-character",
            Self::HeadlessFormat => "headless-format",
            Self::HeadlessPromptVia => "headless-prompt-via",
            Self::HeadlessArgs => "headless-args",
            Self::ExtraDocuments => "extra-documents",
        }
    }
    /// The inverse of `as_str` (for records read back from disk).
    pub fn parse(name: &str) -> Option<Self> {
        [
            Self::NotAList,
            Self::NotAMapping,
            Self::IncompatibleType,
            Self::MissingId,
            Self::MissingCommand,
            Self::IdTooLong,
            Self::IdInvalidShape,
            Self::ReservedId,
            Self::DuplicateId,
            Self::CommandUnterminatedQuote,
            Self::CommandControlCharacter,
            Self::HeadlessFormat,
            Self::HeadlessPromptVia,
            Self::HeadlessArgs,
            Self::ExtraDocuments,
        ]
        .into_iter()
        .find(|code| code.as_str() == name)
    }
}

/// The field names a problem can carry, as the `&'static str` it holds.
pub(super) fn intern_field(name: &str) -> &'static str {
    [
        "id",
        "label",
        "command",
        "approval_pattern_source",
        "headless",
        "headless.format",
        "headless.prompt_via",
        "headless.args",
        "entry",
        "custom_providers",
        "documents",
    ]
    .into_iter()
    .find(|known| *known == name)
    .unwrap_or("entry")
}

/// One mistake in one `custom_providers` entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigProblem {
    pub section: &'static str,
    /// 1-based position among the entries of the list (None: the whole list).
    pub index: Option<usize>,
    /// The entry's id (lower-cased and trimmed, at most 64 characters) when it could be read.
    pub id: Option<String>,
    /// The field that is wrong: `id`, `command`, `headless.format`, ... or `entry`.
    pub field: &'static str,
    pub code: ProblemCode,
    /// True when the entry was left out; false when it still works in a reduced way.
    pub dropped: bool,
    /// For a repeated id: the 1-based position of the entry that keeps the id.
    /// For an entry left out because of its `command` while a later entry has
    /// the same id: the position of that later entry, which is the one used.
    pub related_index: Option<usize>,
}

/// The `section` of a problem about the file as a whole (not one provider entry).
pub const FILE_SECTION: &str = "config";

/// The English wording of one problem. Built in exactly one place
/// (`ConfigProblem::text_en`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProblemText {
    /// Which entry and field, e.g. "custom_providers entry #2 (id: shell), field `id`".
    pub summary: String,
    /// What is wrong.
    pub reason: String,
    /// What it means now (for example, that the provider is not a spawn option).
    pub effect: String,
    /// A correct way to write it.
    pub example: String,
    /// A command that fixes it, when one exists.
    pub fix_command: Option<String>,
    pub check_command: &'static str,
}

impl ConfigProblem {
    pub(super) fn new(
        number: Option<usize>,
        id: &Option<String>,
        field: &'static str,
        code: ProblemCode,
        dropped: bool,
        related_index: Option<usize>,
    ) -> Self {
        Self {
            section: "custom_providers",
            index: number,
            id: id.clone(),
            field,
            code,
            dropped,
            related_index,
        }
    }
    /// A mistake in the file as a whole, not in one `custom_providers` entry.
    pub(super) fn file_level(field: &'static str, code: ProblemCode) -> Self {
        Self {
            section: FILE_SECTION,
            index: None,
            id: None,
            field,
            code,
            dropped: false,
            related_index: None,
        }
    }
    /// True for a problem about one `custom_providers` entry (or the whole list).
    pub fn is_provider_entry(&self) -> bool {
        self.section == "custom_providers"
    }
    pub fn text_en(&self) -> ProblemText {
        if self.code == ProblemCode::ExtraDocuments {
            return ProblemText {
                summary: "config.yaml, everything after the first YAML document".to_owned(),
                reason: "The file has more than one YAML document (a line with `---` followed by more). Only the first document is read.".to_owned(),
                effect: "Everything written after the first document is ignored and is not checked for mistakes.".to_owned(),
                example: "token: your-token\ncustom_providers:\n  - id: my-cli\n    command: my-cli".to_owned(),
                fix_command: None,
                check_command: CHECK_COMMAND,
            };
        }
        let id = self.id.as_deref();
        let entry = match (self.index, id) {
            (Some(index), Some(id)) => format!("entry #{index} (id: {id})"),
            (Some(index), None) => format!("entry #{index}"),
            (None, _) => "list".to_owned(),
        };
        let summary = if self.index.is_some() {
            format!("{} {entry}, field `{}`", self.section, self.field)
        } else {
            self.section.to_owned()
        };
        let subject = match (id, self.index) {
            (Some(id), _) => format!("`{id}`"),
            (None, Some(index)) => format!("Entry #{index}"),
            (None, None) => "The list".to_owned(),
        };
        let field = self.field;
        let reserved = {
            let mut ids = super::model::BUILTIN_PROVIDER_IDS.to_vec();
            ids.push("shell");
            ids.join(", ")
        };
        let reason = match self.code {
            ProblemCode::NotAList => {
                "`custom_providers` must be a list of entries, but something else is written there."
                    .to_owned()
            }
            ProblemCode::NotAMapping => {
                "This entry is not a set of `key: value` lines.".to_owned()
            }
            ProblemCode::IncompatibleType => {
                format!("The value of `{field}` has the wrong type (for example a list where text is expected).")
            }
            ProblemCode::MissingId => "`id` is missing or empty.".to_owned(),
            ProblemCode::MissingCommand => {
                "`command` is missing or empty, or its first word (the program to run) is empty."
                    .to_owned()
            }
            ProblemCode::IdTooLong => "`id` is longer than 64 characters.".to_owned(),
            ProblemCode::IdInvalidShape => "`id` may only contain letters, digits, dot, underscore and hyphen, and must start with a letter or digit.".to_owned(),
            ProblemCode::ReservedId => format!(
                "`id: {}` is reserved (built-in providers and `shell`: {reserved}), so it cannot be used as a custom provider id.",
                id.unwrap_or("")
            ),
            ProblemCode::DuplicateId => format!(
                "`id: {}` is already used by entry #{}; every id must be different.",
                id.unwrap_or(""),
                self.related_index.map_or_else(String::new, |n| n.to_string())
            ),
            ProblemCode::CommandUnterminatedQuote => {
                "`command` has a double quote (\") that is never closed.".to_owned()
            }
            ProblemCode::CommandControlCharacter => {
                "`command` contains a control character (such as a line break or a tab inside quotes).".to_owned()
            }
            ProblemCode::HeadlessFormat => {
                "`headless.format` is missing or is not one of: text, claude-stream-json.".to_owned()
            }
            ProblemCode::HeadlessPromptVia => {
                "`headless.prompt_via` must be `stdin` or `arg` (or left out).".to_owned()
            }
            ProblemCode::HeadlessArgs => {
                "`headless.args` has too many entries, an empty entry, a very long entry or a control character.".to_owned()
            }
            // Returned early above.
            ProblemCode::ExtraDocuments => String::new(),
        };
        let effect = if self.dropped {
            match self.code {
                ProblemCode::NotAList => {
                    "No custom providers were loaded from this setting.".to_owned()
                }
                ProblemCode::DuplicateId => format!(
                    "This second use was left out; entry #{} is the one that is used.",
                    self.related_index
                        .map_or_else(String::new, |n| n.to_string())
                ),
                ProblemCode::CommandUnterminatedQuote | ProblemCode::CommandControlCharacter
                    if self.related_index.is_some() =>
                {
                    format!(
                        "{subject} entry #{} was left out; entry #{} has the same id and is the one that is used.",
                        self.index.map_or_else(String::new, |n| n.to_string()),
                        self.related_index
                            .map_or_else(String::new, |n| n.to_string())
                    )
                }
                _ => format!(
                    "{subject} was left out: it is not offered as a spawn option and cannot be launched."
                ),
            }
        } else {
            format!(
                "{subject} still works for interactive sessions, but headless (non-interactive) runs are not available for it."
            )
        };
        let example = match self.code {
            ProblemCode::HeadlessFormat
            | ProblemCode::HeadlessPromptVia
            | ProblemCode::HeadlessArgs => HEADLESS_EXAMPLE,
            _ => LIST_EXAMPLE,
        }
        .to_owned();
        ProblemText {
            summary,
            reason,
            effect,
            example,
            // The `provider add/remove` commands that could fix an entry do not exist yet.
            fix_command: None,
            check_command: CHECK_COMMAND,
        }
    }
    /// A few lines for a terminal.
    pub fn render_en(&self) -> String {
        let text = self.text_en();
        let example = text
            .example
            .lines()
            .map(|line| format!("      {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut out = format!(
            "{}\n  Problem: {}\n  Effect:  {}\n  Example:\n{example}\n",
            text.summary, text.reason, text.effect
        );
        if let Some(command) = &text.fix_command {
            out.push_str(&format!("  Fix:     {command}\n"));
        }
        out.push_str(&format!(
            "  Check:   {} (after editing the file)\n",
            text.check_command
        ));
        out
    }
    /// One line, for places that list warnings.
    pub fn warning_en(&self) -> String {
        let text = self.text_en();
        format!("{}: {} {}", text.summary, text.reason, text.effect)
    }
    pub fn to_json(&self) -> Value {
        let text = self.text_en();
        json!({
            "section": self.section,
            "index": self.index,
            "id": self.id,
            "field": self.field,
            "code": self.code.as_str(),
            "dropped": self.dropped,
            "related_index": self.related_index,
            "summary": text.summary,
            "reason": text.reason,
            "effect": text.effect,
            "example": text.example,
            "fix_command": text.fix_command,
            "check_command": text.check_command,
        })
    }
}

/// Characters that are not "control" characters but still change how text is
/// drawn or read: the invisible formatting characters (Unicode category Cf: zero
/// width, left-to-right/right-to-left marks and overrides, isolates, BOM, tag
/// characters) and the line and paragraph separators. An id taken from the
/// settings file could use them to make a message read differently from what
/// it says, so they are never shown.
fn is_invisible_format(c: char) -> bool {
    matches!(
        c as u32,
        0x00AD
            | 0x0600..=0x0605
            | 0x061C
            | 0x06DD
            | 0x070F
            | 0x0890..=0x0891
            | 0x08E2
            | 0x180E
            | 0x200B..=0x200F
            | 0x2028..=0x202E
            | 0x2060..=0x206F
            | 0xFEFF
            | 0xFFF9..=0xFFFB
            | 0x110BD
            | 0x110CD
            | 0x13430..=0x1343F
            | 0x1BCA0..=0x1BCA3
            | 0x1D173..=0x1D17A
            | 0xE0001
            | 0xE0020..=0xE007F
    )
}

/// An id safe to show: trimmed, lower-cased, without control or invisible
/// formatting characters, short. The one place that makes it: the problems, the
/// refusal texts (`left_out_provider`) and the screen all show this form.
pub fn display_id(raw: &str) -> Option<String> {
    let id: String = normalize_subscription_id(raw)
        .chars()
        .filter(|c| !c.is_control() && !is_invisible_format(*c))
        .collect();
    if id.is_empty() {
        None
    } else if id.chars().count() > 64 {
        Some(format!("{}...", id.chars().take(64).collect::<String>()))
    } else {
        Some(id)
    }
}

/// Decide, entry by entry, which `custom_providers` can be launched. The one
/// place that decision is made: the spawn list, the problems shown to users
/// and `warnings()` all follow it. Entries come with their 0-based position.
/// `strict_command` also requires the command to be splittable.
pub(super) fn analyze_custom_providers<'a>(
    entries: impl IntoIterator<Item = (usize, &'a CustomProvider)>,
    strict_command: bool,
) -> (Vec<CustomProvider>, Vec<ConfigProblem>) {
    let mut kept = Vec::new();
    let mut problems = Vec::new();
    let mut first_use: BTreeMap<String, usize> = BTreeMap::new();
    // Entries left out for their `command` (index into `problems`, normalized
    // id): when another entry with the same id is the one used, say which.
    let mut left_out_by_command: Vec<(usize, String)> = Vec::new();
    for (position, entry) in entries {
        let number = Some(position + 1);
        let id = normalize_subscription_id(&entry.id);
        let shown = display_id(&entry.id);
        let mut usable = true;
        if let Some(found) = super::model::custom_provider_id_problem(&id) {
            usable = false;
            let code = match found {
                IdProblem::Empty => ProblemCode::MissingId,
                IdProblem::TooLong => ProblemCode::IdTooLong,
                IdProblem::BadShape => ProblemCode::IdInvalidShape,
                IdProblem::Reserved => ProblemCode::ReservedId,
            };
            problems.push(ConfigProblem::new(number, &shown, "id", code, true, None));
        }
        if entry.command.trim().is_empty() {
            usable = false;
            problems.push(ConfigProblem::new(
                number,
                &shown,
                "command",
                ProblemCode::MissingCommand,
                true,
                None,
            ));
        } else if strict_command
            && let Some(found) = match split_command_typed(&entry.command) {
                Err(found) => Some(found),
                // `command: '""'` (or `'" " x'`) splits into a first word that is
                // empty or only blanks: there is no program to run. (The
                // Go-compatible adapter still accepts it; it is the program that
                // must not offer or launch it.)
                Ok(words) if words.first().is_none_or(|word| word.trim().is_empty()) => {
                    Some(CommandProblem::Empty)
                }
                Ok(_) => None,
            }
        {
            usable = false;
            let code = match found {
                CommandProblem::Unterminated => ProblemCode::CommandUnterminatedQuote,
                CommandProblem::Control(_) => ProblemCode::CommandControlCharacter,
                CommandProblem::Empty => ProblemCode::MissingCommand,
            };
            left_out_by_command.push((problems.len(), id.clone()));
            problems.push(ConfigProblem::new(
                number, &shown, "command", code, true, None,
            ));
        }
        if usable {
            if let Some(first) = first_use.get(&id) {
                usable = false;
                problems.push(ConfigProblem::new(
                    number,
                    &shown,
                    "id",
                    ProblemCode::DuplicateId,
                    true,
                    Some(*first),
                ));
            } else {
                first_use.insert(id.clone(), position + 1);
            }
        }
        if usable {
            if let Some(def) = &entry.headless
                && let Some(found) = headless_problem(def)
            {
                let (field, code) = match found {
                    HeadlessProblem::FormatRequired | HeadlessProblem::FormatUnknown => {
                        ("headless.format", ProblemCode::HeadlessFormat)
                    }
                    HeadlessProblem::PromptViaUnknown => {
                        ("headless.prompt_via", ProblemCode::HeadlessPromptVia)
                    }
                    HeadlessProblem::TooManyArgs
                    | HeadlessProblem::ArgEmpty
                    | HeadlessProblem::ArgTooLong
                    | HeadlessProblem::ArgControl => ("headless.args", ProblemCode::HeadlessArgs),
                };
                problems.push(ConfigProblem::new(number, &shown, field, code, false, None));
            }
            let mut normalized = entry.clone();
            normalized.id = id;
            kept.push(normalized);
        }
    }
    for (at, id) in left_out_by_command {
        if let Some(used) = first_use.get(&id) {
            problems[at].related_index = Some(*used);
        }
    }
    (kept, problems)
}

/// Problems with the `custom_providers` entries exactly as written in the
/// file, before entries that cannot be represented are dropped on load. This
/// is what gives each problem its position in the file.
pub(super) fn analyze_raw_custom_providers(raw: Option<&Value>) -> Vec<ConfigProblem> {
    let list = match raw {
        None | Some(Value::Null) => return Vec::new(),
        Some(Value::Array(list)) => list,
        Some(_) => {
            return vec![ConfigProblem::new(
                None,
                &None,
                "custom_providers",
                ProblemCode::NotAList,
                true,
                None,
            )];
        }
    };
    const FIELDS: [(&str, &str); 5] = [
        ("id", "string"),
        ("label", "string"),
        ("command", "string"),
        ("approval_pattern_source", "string"),
        ("headless", "*HeadlessDef"),
    ];
    let mut problems = Vec::new();
    let mut loadable = Vec::new();
    for (position, entry) in list.iter().enumerate() {
        let number = Some(position + 1);
        let Value::Object(map) = entry else {
            problems.push(ConfigProblem::new(
                number,
                &None,
                "entry",
                ProblemCode::NotAMapping,
                true,
                None,
            ));
            continue;
        };
        let shown = map.get("id").and_then(Value::as_str).and_then(display_id);
        match normalize_node(
            entry.clone(),
            "CustomProvider",
            "custom_providers.entry",
            false,
        ) {
            Ok(value) => match serde_json::from_value::<CustomProvider>(value) {
                Ok(provider) => loadable.push((position, provider)),
                Err(_) => problems.push(ConfigProblem::new(
                    number,
                    &shown,
                    "entry",
                    ProblemCode::IncompatibleType,
                    true,
                    None,
                )),
            },
            Err(_) => {
                let field = FIELDS
                    .iter()
                    .find(|(name, kind)| {
                        map.get(*name).is_some_and(|value| {
                            normalize_node(value.clone(), kind, "custom_providers.entry", false)
                                .is_err()
                        })
                    })
                    .map_or("entry", |(name, _)| *name);
                problems.push(ConfigProblem::new(
                    number,
                    &shown,
                    field,
                    ProblemCode::IncompatibleType,
                    true,
                    None,
                ));
            }
        }
    }
    problems.extend(
        analyze_custom_providers(loadable.iter().map(|(position, p)| (*position, p)), true).1,
    );
    problems.sort_by_key(|problem| problem.index);
    problems
}

// ---------------------------------------------------------------------------
// Looking at the file as it is now (read only)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigFileState {
    /// There is no settings file yet.
    Missing { path: PathBuf },
    /// The file cannot be read as a whole.
    Unreadable(ConfigUnreadable),
    /// The file is readable. `problems` lists entries that are left out or reduced.
    Readable {
        path: PathBuf,
        problems: Vec<ConfigProblem>,
    },
}
impl ConfigFileState {
    /// True when nothing needs the user's attention.
    pub fn is_ok(&self) -> bool {
        match self {
            Self::Missing { .. } => true,
            Self::Unreadable(_) => false,
            Self::Readable { problems, .. } => problems.is_empty(),
        }
    }
    pub fn problems(&self) -> &[ConfigProblem] {
        match self {
            Self::Readable { problems, .. } => problems,
            _ => &[],
        }
    }
    pub fn path(&self) -> &PathBuf {
        match self {
            Self::Missing { path } | Self::Readable { path, .. } => path,
            Self::Unreadable(unreadable) => &unreadable.path,
        }
    }
    pub fn render_en(&self) -> String {
        match self {
            Self::Missing { path } => format!(
                "There is no settings file yet ({}). Defaults are used, and the file is created the first time many-ai-cli starts.\n",
                path.display()
            ),
            Self::Unreadable(unreadable) => format!("{}\n", unreadable.message_en()),
            Self::Readable { path, problems } if problems.is_empty() => {
                format!("OK: {} can be read and has no mistakes.\n", path.display())
            }
            Self::Readable { path, problems } => {
                let mut out = format!(
                    "{} mistake{} found in {}. The Hub still starts; the entries named below are affected.\n\n",
                    problems.len(),
                    if problems.len() == 1 { "" } else { "s" },
                    path.display()
                );
                for problem in problems {
                    out.push_str(&problem.render_en());
                    out.push('\n');
                }
                out.push_str(&format!(
                    "After editing the file, run `{CHECK_COMMAND}` again.\n"
                ));
                out
            }
        }
    }
    pub fn to_json(&self) -> Value {
        let status = match self {
            Self::Missing { .. } => "missing",
            Self::Unreadable(_) => "unreadable",
            Self::Readable { problems, .. } if problems.is_empty() => "ok",
            Self::Readable { .. } => "problems",
        };
        json!({
            "ok": self.is_ok(),
            "status": status,
            "file": self.path(),
            "unreadable": match self {
                Self::Unreadable(unreadable) => unreadable.to_json(),
                _ => Value::Null,
            },
            "problems": self.problems().iter().map(ConfigProblem::to_json).collect::<Vec<_>>(),
            "check_command": CHECK_COMMAND,
        })
    }
}

/// Read `config.yaml` and say what is wrong with it. Never writes, never
/// creates a directory, never starts anything.
pub fn inspect_file(paths: &RuntimePaths) -> io::Result<ConfigFileState> {
    let path = paths.resource(Resource::Config);
    let root = match Dir::open(paths.root()) {
        Ok(root) => root,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(ConfigFileState::Missing { path });
        }
        Err(error) => return Err(error),
    };
    let bytes = match read_settings_file(&root)? {
        SettingsFile::Missing => return Ok(ConfigFileState::Missing { path }),
        SettingsFile::TooLarge => {
            return Ok(ConfigFileState::Unreadable(ConfigUnreadable::too_large(
                paths,
            )));
        }
        SettingsFile::Bytes(bytes) => bytes,
    };
    Ok(match parse_config_bytes(&bytes, paths) {
        Ok((_, problems)) => ConfigFileState::Readable { path, problems },
        Err(unreadable) => ConfigFileState::Unreadable(unreadable),
    })
}

/// Why no new session can be started right now, or `None` when one can. Every
/// new session is started by a child `many-ai-cli wrap`, which reads the
/// settings file first and stops on a file it cannot read (too large, not
/// text, not YAML, a value refused). A Hub that keeps running with the settings
/// it loaded still runs its sessions, but cannot start new ones. The text
/// never repeats anything from the file.
pub fn new_sessions_blocker(paths: &RuntimePaths) -> Option<String> {
    let because = match inspect_file(paths) {
        Ok(ConfigFileState::Unreadable(unreadable)) => unreadable.summary_en(),
        // Could not even look at it (a folder where the file should be, a
        // lock): that is not "the content is unreadable". It is left to `wrap`,
        // which reports its own I/O error, so a Hub whose save failed for such a
        // reason still starts children from the settings it holds.
        Err(_) | Ok(_) => return None,
    };
    Some(format!(
        "The settings file cannot be read ({because}), so a new session cannot be started: every new session is started by `many-ai-cli wrap`, which reads the settings file first. Sessions that are already running are not affected. Fix the file (`{CHECK_COMMAND}` shows where), or run `{RESET_COMMAND} --yes` to start over from the defaults (the current file is saved as a backup)."
    ))
}
// ---------------------------------------------------------------------------
// Starting over from the default settings
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default)]
pub struct ResetOptions {
    /// Create a new Hub token even when the current one could be kept.
    pub new_token: bool,
    /// The user agreed to the consequences of resetting an unreadable file.
    pub confirmed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenOutcome {
    /// The Hub token was carried over; the Hub URL does not change.
    Kept,
    /// A new token was made because it was asked for.
    NewRequested,
    /// A new token was made because the old file had none.
    NewNoneBefore,
    /// A new token was made because the old file cannot be read.
    NewUnreadable,
    /// A new token was made because there was no settings file.
    NewNoFile,
}
impl TokenOutcome {
    pub fn is_kept(self) -> bool {
        self == Self::Kept
    }
}

#[derive(Debug)]
pub struct ResetOutcome {
    /// The copy of the previous settings (None when there was no file).
    pub backup: Option<super::backup::BackupRecord>,
    pub token: TokenOutcome,
}

#[derive(Debug)]
pub enum ResetError {
    /// Nothing was changed: the file cannot be read, so its token is lost by
    /// resetting. The caller must ask the user first (`confirmed`).
    NeedsConfirmation(ConfigUnreadable),
    Io(io::Error),
}
impl From<io::Error> for ResetError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// The Hub token of a file that is valid YAML but whose settings are refused as
/// a whole. `None` when the file is not valid YAML, is not text, has no token,
/// or the token would not make a usable settings file.
fn salvage_token(bytes: &[u8], paths: &RuntimePaths) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    let value = super::yaml_nodes::decode_config(text).ok()?;
    let token = value.get("token")?.as_str()?.to_owned();
    if token.is_empty() {
        return None;
    }
    let mut fresh = Config::defaults(paths);
    fresh.token = token.clone();
    let payload = fresh.to_private_yaml().ok()?;
    Config::from_yaml(&payload, paths).ok()?;
    Some(token)
}

/// Back up `config.yaml`, then replace it with the default settings. The Hub
/// token is carried over when the file can be read (unless `new_token`), so
/// the Hub URL stays the same. If the backup cannot be made, nothing changes.
pub fn reset_config(
    paths: &RuntimePaths,
    options: ResetOptions,
    new_token: impl FnMut() -> io::Result<String>,
) -> Result<ResetOutcome, ResetError> {
    reset_config_with(
        paths,
        options,
        new_token,
        &super::backup::ConfigBackups::new(paths),
    )
}

/// `reset_config` with the backup writer (and so its clock) supplied.
pub fn reset_config_with(
    paths: &RuntimePaths,
    options: ResetOptions,
    mut new_token: impl FnMut() -> io::Result<String>,
    backups: &super::backup::ConfigBackups,
) -> Result<ResetOutcome, ResetError> {
    let root = Dir::open_or_create_private(paths.root())?;
    let existing = read_settings_file(&root)?;
    let too_large = matches!(existing, SettingsFile::TooLarge);
    let (token, outcome) = match &existing {
        SettingsFile::Missing => (new_token()?, TokenOutcome::NewNoFile),
        // Not read: its token is lost, which needs the person's say-so.
        SettingsFile::TooLarge if !options.confirmed => {
            return Err(ResetError::NeedsConfirmation(ConfigUnreadable::too_large(
                paths,
            )));
        }
        SettingsFile::TooLarge => (new_token()?, TokenOutcome::NewUnreadable),
        SettingsFile::Bytes(bytes) => match parse_config_bytes(bytes, paths) {
            Ok((config, _)) if !options.new_token && !config.token.is_empty() => {
                (config.token, TokenOutcome::Kept)
            }
            Ok((config, _)) => (
                new_token()?,
                if config.token.is_empty() {
                    TokenOutcome::NewNoneBefore
                } else {
                    TokenOutcome::NewRequested
                },
            ),
            // The file is valid YAML but a value is refused (for example a bad
            // `trusted_networks` entry): the token is still there to read, so
            // it is not lost and nothing needs confirming.
            Err(unreadable) => match salvage_token(bytes, paths) {
                Some(token) if !options.new_token => (token, TokenOutcome::Kept),
                Some(_) => (new_token()?, TokenOutcome::NewRequested),
                None if !options.confirmed => {
                    return Err(ResetError::NeedsConfirmation(unreadable));
                }
                None => (new_token()?, TokenOutcome::NewUnreadable),
            },
        },
    };
    if token.is_empty() {
        return Err(io::Error::other("token generator returned an empty token").into());
    }
    let mut fresh = Config::defaults(paths);
    fresh.token = token;
    let payload = fresh
        .to_private_yaml()
        .map_err(|error| io::Error::other(error.to_string()))?;
    // Never write a file this program could not read back.
    Config::from_yaml(&payload, paths).map_err(|error| io::Error::other(error.to_string()))?;
    // Everything is replaced on purpose, so no earlier "removed by a save"
    // notice is left standing. That is settled BEFORE the file is replaced: a
    // failure after the replacement could then not be told apart from a reset
    // that did nothing. If the reset does not happen the notices are put back.
    let settled = super::removed::settle_for_reset(backups, &root)?;
    let put_records_back = |error: io::Error| -> ResetError {
        match settled.undo(backups, &root) {
            Ok(()) => error.into(),
            Err(undo) => super::backup::partial_failure(format!(
                "the settings were not changed ({error}), but the notice records could not be put back ({undo}); they are the `*.dismissed` files in {}",
                backups.directory().display()
            ))
            .into(),
        }
    };
    let backup = if too_large {
        // A rename inside the same folder tree, so the size does not matter.
        let moved = match backups.move_config_in(&root) {
            Ok(moved) => moved,
            Err(error) => return Err(put_records_back(error)),
        };
        if let Err(error) = root.replace("config.yaml", payload.as_bytes(), 0o600) {
            // Put the file back: nothing may change when the reset fails.
            return Err(match backups.move_back_in(&root, &moved) {
                Ok(()) => put_records_back(error),
                Err(back) => super::backup::partial_failure(format!(
                    "the new settings could not be written ({error}), and the previous settings, which were moved to {}, could not be moved back ({back}). Move that file back to {} yourself, or run `{RESET_COMMAND} --yes` again",
                    moved.path.display(),
                    paths.resource(Resource::Config).display()
                ))
                .into(),
            });
        }
        Some(moved)
    } else {
        match backups.replace_config_in(&root, payload.as_bytes()) {
            Ok(backup) => backup,
            Err(error) => return Err(put_records_back(error)),
        }
    };
    Ok(ResetOutcome {
        backup,
        token: outcome,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trial() -> (tempfile::TempDir, tempfile::TempDir, RuntimePaths) {
        let root = tempfile::tempdir().unwrap();
        let installed = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::trial(root.path(), 49212, installed.path()).unwrap();
        (root, installed, paths)
    }
    fn problems_of(yaml: &str) -> Vec<ConfigProblem> {
        let (_root, _installed, paths) = trial();
        Config::from_yaml_inspected(yaml, &paths).unwrap().1
    }

    #[test]
    fn a_correct_list_has_no_problems() {
        assert!(
            problems_of(
                "custom_providers:\n  - id: my-cli\n    command: my-cli --flag \"two words\"\n"
            )
            .is_empty()
        );
        assert!(problems_of("hub: {port: 49000}\n").is_empty());
    }

    #[test]
    fn each_kind_of_mistake_names_entry_id_field_and_reason() {
        let found = problems_of(
            "custom_providers:\n\
             \x20 - id: shell\n    command: shell-cli\n\
             \x20 - id: good-one\n    command: good-one\n\
             \x20 - id: good-one\n    command: other\n\
             \x20 - id: quote-bad\n    command: 'tool \"open'\n\
             \x20 - id: no-command\n\
             \x20 - command: no-id\n\
             \x20 - 7\n\
             \x20 - id: [a, b]\n    command: x\n\
             \x20 - id: bad-headless\n    command: x\n    headless: {format: yaml}\n",
        );
        let summary: Vec<_> = found
            .iter()
            .map(|p| (p.index.unwrap(), p.id.as_deref(), p.field, p.code.as_str()))
            .collect();
        assert_eq!(
            summary,
            [
                (1, Some("shell"), "id", "reserved-id"),
                (3, Some("good-one"), "id", "duplicate-id"),
                (
                    4,
                    Some("quote-bad"),
                    "command",
                    "command-unterminated-quote"
                ),
                (5, Some("no-command"), "command", "missing-command"),
                (6, None, "id", "missing-id"),
                (7, None, "entry", "not-a-mapping"),
                (8, None, "id", "incompatible-type"),
                (
                    9,
                    Some("bad-headless"),
                    "headless.format",
                    "headless-format"
                ),
            ]
        );
        assert_eq!(found[1].related_index, Some(2));
        assert!(found[..7].iter().all(|p| p.dropped));
        assert!(!found[7].dropped);
    }

    #[test]
    fn a_list_written_as_something_else_is_one_problem() {
        let found = problems_of("custom_providers: not-a-list\n");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].code, ProblemCode::NotAList);
        assert_eq!(found[0].index, None);
    }

    #[test]
    fn the_loaded_view_and_the_spawn_list_follow_the_same_decision() {
        let (_root, _installed, paths) = trial();
        let (config, file_problems) = Config::from_yaml_inspected(
            "custom_providers:\n  - id: shell\n    command: x\n  - id: ok\n    command: ok\n  - id: bad-quote\n    command: 'a \"b'\n",
            &paths,
        )
        .unwrap();
        let spawnable: Vec<_> = super::super::effective_custom_providers(&config.custom_providers)
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(spawnable, ["ok"]);
        assert_eq!(
            file_problems.iter().map(|p| p.index).collect::<Vec<_>>(),
            [Some(1), Some(3)]
        );
        // The in-memory view reports the same entries (positions in the loaded list).
        assert_eq!(config.custom_provider_problems().len(), 2);
        assert_eq!(config.warnings().len(), 2);
    }

    #[test]
    fn wording_comes_from_one_function_and_never_carries_values() {
        let found =
            problems_of("custom_providers:\n  - id: shell\n    command: synthetic-secret-marker\n");
        let problem = &found[0];
        let text = problem.text_en();
        assert!(text.reason.contains("`id: shell` is reserved"));
        assert!(text.effect.contains("`shell` was left out"));
        assert!(text.effect.contains("spawn option"));
        assert_eq!(text.check_command, "many-ai-cli config check");
        let json = problem.to_json();
        assert_eq!(json["index"], 1);
        assert_eq!(json["id"], "shell");
        assert_eq!(json["field"], "id");
        assert_eq!(json["code"], "reserved-id");
        assert_eq!(json["reason"], text.reason);
        assert_eq!(json["effect"], text.effect);
        assert_eq!(json["dropped"], true);
        assert!(json["fix_command"].is_null());
        assert_eq!(json["check_command"], "many-ai-cli config check");
        for rendered in [problem.render_en(), problem.warning_en(), json.to_string()] {
            assert!(!rendered.contains("synthetic-secret-marker"));
        }
    }

    #[test]
    fn unreadable_files_report_a_position_and_never_a_line() {
        let (_root, _installed, paths) = trial();
        let bytes = b"token: synthetic-private-marker\nhub: [unterminated";
        let unreadable = parse_config_bytes(bytes, &paths).unwrap_err();
        assert_eq!(unreadable.reason, "YAML syntax error");
        assert!(unreadable.line.unwrap() >= 2, "{unreadable:?}");
        assert!(unreadable.column.is_some());
        let message = unreadable.message_en();
        assert!(message.contains(&paths.resource(Resource::Config).display().to_string()));
        assert!(message.contains("config reset"));
        assert!(message.contains("config.yaml.bak_<date>_<time>"));
        assert!(!message.contains("synthetic-private-marker"));
        assert!(
            !unreadable
                .to_json()
                .to_string()
                .contains("synthetic-private-marker")
        );
    }

    #[test]
    fn inspect_file_reads_without_writing() {
        let (_root, _installed, paths) = trial();
        assert!(matches!(
            inspect_file(&paths).unwrap(),
            ConfigFileState::Missing { .. }
        ));
        let config = paths.resource(Resource::Config);
        std::fs::write(&config, "token: synthetic-t\nhub: [broken").unwrap();
        let state = inspect_file(&paths).unwrap();
        assert!(matches!(state, ConfigFileState::Unreadable(_)));
        assert!(!state.is_ok());
        assert_eq!(
            std::fs::read_to_string(&config).unwrap(),
            "token: synthetic-t\nhub: [broken"
        );
        assert!(!paths.resource(Resource::ConfigBackups).exists());
        std::fs::write(&config, "token: synthetic-t\n").unwrap();
        assert!(inspect_file(&paths).unwrap().is_ok());
    }
}
