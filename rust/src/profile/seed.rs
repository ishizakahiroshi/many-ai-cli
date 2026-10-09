//! Named-entry profile seeding. The merge computation is separate from the
//! directory-handle writer so parsing errors cannot truncate an existing profile.
use std::collections::BTreeSet;
use toml_edit::{DocumentMut, Item, Value};

pub const CODEX_STATE_KEYS: &[&str] = &[
    "projects",
    "tui",
    "notice",
    "windows",
    "model",
    "model_reasoning_effort",
    "hooks",
];
#[derive(Clone, Debug)]
pub struct SyncOptions {
    pub enabled: bool,
    pub profile_owned_keys: Vec<String>,
    pub default_wins_keys: Vec<String>,
}
impl Default for SyncOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            profile_owned_keys: vec![],
            default_wins_keys: vec![],
        }
    }
}
#[derive(Debug, PartialEq, Eq)]
pub enum SeedError {
    MalformedDefault,
    MalformedProfile,
}
impl std::fmt::Display for SeedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::MalformedDefault => "default config.toml is not a TOML document",
            Self::MalformedProfile => "profile config.toml is not a TOML document",
        })
    }
}
impl std::error::Error for SeedError {}
pub struct MergeResult {
    pub text: String,
    pub changed_keys: Vec<String>,
    pub removed_temporary_hooks: usize,
    pub created: bool,
}
fn effective_owned(options: &SyncOptions) -> BTreeSet<String> {
    let wins: BTreeSet<_> = options
        .default_wins_keys
        .iter()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .collect();
    CODEX_STATE_KEYS
        .iter()
        .copied()
        .chain(options.profile_owned_keys.iter().map(String::as_str))
        .map(str::trim)
        .filter(|v| !v.is_empty() && !wins.contains(v))
        .map(Into::into)
        .collect()
}

/// Recognize exactly the process-owned generated command shape, rather than
/// dropping an entire hooks table or any command mentioning the application.
fn owned_command(command: &str) -> bool {
    // The injector uses a POSIX-quoted executable. The custom-provider command
    // tokenizer intentionally uses different (Windows-style) quoting, so it
    // cannot safely recognize this fixed generated form with spaced paths.
    let Some((prefix, tail)) = command.split_once(" usage-relay --provider codex --hub ") else {
        return false;
    };
    let Some((target, session)) = tail.rsplit_once(" --session ") else {
        return false;
    };
    if session.is_empty()
        || !session.bytes().all(|c| c.is_ascii_digit())
        || session.parse::<u64>().is_err()
    {
        return false;
    }
    let Some((environment, executable)) = prefix.split_once(' ') else {
        return false;
    };
    let token = environment
        .strip_prefix("MANY_AI_CLI_HUB_TOKEN=")
        .or_else(|| environment.strip_prefix("ANY_AI_CLI_HUB_TOKEN="));
    if !token.is_some_and(|v| !v.is_empty() && !v.chars().any(char::is_whitespace)) {
        return false;
    }
    let Some(inner) = executable
        .strip_prefix('\'')
        .and_then(|v| v.strip_suffix('\''))
    else {
        return false;
    };
    if inner.replace("'\\''", "").contains('\'') {
        return false;
    }
    let Ok(url) = url::Url::parse(target) else {
        return false;
    };
    matches!(url.scheme(), "http" | "https") && url.host_str().is_some()
}
fn strip_temporary_hooks(default: &mut DocumentMut) -> usize {
    let Some(hooks) = default.get_mut("hooks").and_then(Item::as_table_like_mut) else {
        return 0;
    };
    let Some(stop) = hooks.get_mut("Stop") else {
        return 0;
    };
    let mut removed = 0;
    if let Some(array) = stop.as_array_of_tables_mut() {
        let mut i = 0;
        while i < array.len() {
            let table = array.get(i).expect("array entry");
            let command = table.get("command").and_then(Item::as_str).unwrap_or("");
            let marked = table
                .decor()
                .prefix()
                .and_then(|d| d.as_str())
                .is_some_and(|d| {
                    d.lines().any(|line| {
                        matches!(
                            line.trim(),
                            "# many-ai-cli:usage-hook-start" | "# any-ai-cli:usage-hook-start"
                        )
                    })
                });
            if marked || owned_command(command) {
                array.remove(i);
                removed += 1;
            } else {
                i += 1;
            }
        }
        if array.is_empty() {
            hooks.remove("Stop");
        }
    } else if let Some(array) = stop.as_array_mut() {
        let mut i = 0;
        while i < array.len() {
            let command = array
                .get(i)
                .and_then(Value::as_inline_table)
                .and_then(|t| t.get("command"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if owned_command(command) {
                array.remove(i);
                removed += 1;
            } else {
                i += 1;
            }
        }
        if array.is_empty() {
            hooks.remove("Stop");
        }
    }
    if hooks.is_empty() {
        default.remove("hooks");
    }
    removed
}
fn semantic(item: &Item) -> String {
    // Decor/comments/key ordering are not settings. Compare typed, canonical
    // representations while retaining the full TOML item for the actual write.
    if let Some(table) = item.as_table_like() {
        let mut values: Vec<_> = table.iter().map(|(k, v)| (k, semantic(v))).collect();
        values.sort_by(|a, b| a.0.cmp(b.0));
        return format!("table:{values:?}");
    }
    if let Some(array) = item.as_array_of_tables() {
        return format!(
            "tables:{:?}",
            array
                .iter()
                .map(|t| semantic(&Item::Table(t.clone())))
                .collect::<Vec<_>>()
        );
    }
    match item.as_value() {
        Some(value) => value_semantic(value),
        None => "none".into(),
    }
}
fn value_semantic(value: &Value) -> String {
    match value {
        Value::String(v) => format!("s:{:?}", v.value()),
        Value::Integer(v) => format!("i:{}", v.value()),
        Value::Float(v) => format!("f:{}", v.value()),
        Value::Boolean(v) => format!("b:{}", v.value()),
        Value::Datetime(v) => format!("d:{}", v.value()),
        Value::Array(v) => format!("a:{:?}", v.iter().map(value_semantic).collect::<Vec<_>>()),
        Value::InlineTable(v) => {
            let mut fields: Vec<_> = v.iter().map(|(k, v)| (k, value_semantic(v))).collect();
            fields.sort_by(|a, b| a.0.cmp(b.0));
            format!("inline:{fields:?}")
        }
    }
}

/// A missing destination inherits user settings, including user-maintained
/// hooks and profile-state preferences. Only generated temporary session hooks
/// are removed. Existing destination state keys remain owned by that profile.
pub fn merge_codex(
    default_text: &str,
    profile_text: Option<&str>,
    options: &SyncOptions,
) -> Result<MergeResult, SeedError> {
    if !options.enabled
        && let Some(existing) = profile_text
    {
        return Ok(MergeResult {
            text: existing.into(),
            changed_keys: vec![],
            removed_temporary_hooks: 0,
            created: false,
        });
    }
    let mut defaults = default_text
        .parse::<DocumentMut>()
        .map_err(|_| SeedError::MalformedDefault)?;
    let removed = strip_temporary_hooks(&mut defaults);
    let Some(profile_text) = profile_text else {
        return Ok(MergeResult {
            text: defaults.to_string(),
            changed_keys: defaults.iter().map(|(k, _)| k.to_string()).collect(),
            removed_temporary_hooks: removed,
            created: true,
        });
    };
    let mut profile = profile_text
        .parse::<DocumentMut>()
        .map_err(|_| SeedError::MalformedProfile)?;
    let owned = effective_owned(options);
    let mut changed = vec![];
    for (key, value) in defaults.iter() {
        if owned.contains(key) {
            continue;
        }
        if profile
            .get(key)
            .is_some_and(|v| semantic(v) == semantic(value))
        {
            continue;
        }
        profile.insert(key, value.clone());
        changed.push(key.to_string());
    }
    changed.sort();
    let text = if changed.is_empty() {
        profile_text.into()
    } else {
        profile.to_string()
    };
    Ok(MergeResult {
        text,
        changed_keys: changed,
        removed_temporary_hooks: removed,
        created: false,
    })
}
