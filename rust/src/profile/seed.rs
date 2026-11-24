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
    LocalPolicyRequired,
}
impl std::fmt::Display for SeedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::MalformedDefault => "default config.toml is not a TOML document",
            Self::MalformedProfile => "profile config.toml is not a TOML document",
            Self::LocalPolicyRequired => {
                "configure the default security policy in this profile before launching it"
            }
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
    // The current form starts with the quoted executable; the token stays in
    // the environment (U03). Earlier versions wrote the token in front, and
    // copies of that form are still recognized so they are not carried on.
    let executable = if prefix.starts_with('\'') {
        prefix
    } else {
        let Some((environment, executable)) = prefix.split_once(' ') else {
            return false;
        };
        let token = environment
            .strip_prefix("MANY_AI_CLI_HUB_TOKEN=")
            .or_else(|| environment.strip_prefix("ANY_AI_CLI_HUB_TOKEN="));
        if !token.is_some_and(|v| !v.is_empty() && !v.chars().any(char::is_whitespace)) {
            return false;
        }
        executable
    };
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

#[derive(Clone, Copy)]
pub(crate) enum SettingsFormat {
    Claude,
    Codex,
    Grok,
}

impl SettingsFormat {
    /// Only closed sets of scalar preferences may cross account boundaries.
    /// Environment blocks, server definitions and commands are profile-local.
    pub(crate) fn permits_string(self, key: &str, value: &str) -> bool {
        use std::sync::LazyLock;
        static MODEL: LazyLock<regex::Regex> = LazyLock::new(|| {
            regex::Regex::new(concat!(
                r"^(?:gpt-[0-9]{1,2}(?:\.[0-9]{1,2})?(?:-(?:mini|nano|pro|codex|codex-mini|codex-max))?",
                r"|o[0-9](?:-mini|-pro)?",
                r"|grok-(?:[0-9]{1,2}(?:\.[0-9]{1,2})?(?:-fast|-heavy)?|build|code-fast-1)",
                r"|claude-(?:sonnet|opus|haiku)-[0-9](?:-[0-9]{1,2})?(?:-[0-9]{8})?)$"
            )).expect("fixed model name grammar")
        });
        match (self, key) {
            (Self::Claude, "model") => {
                matches!(value, "sonnet" | "opus" | "haiku" | "opusplan")
                    || value.starts_with("claude-") && MODEL.is_match(value)
            }
            (Self::Codex, "model") => {
                (value.starts_with("gpt-") || value.starts_with('o')) && MODEL.is_match(value)
            }
            (Self::Grok, "model") => value.starts_with("grok-") && MODEL.is_match(value),
            (Self::Claude, "theme") => matches!(
                value,
                "light"
                    | "dark"
                    | "light-daltonized"
                    | "dark-daltonized"
                    | "light-ansi"
                    | "dark-ansi"
                    | "auto"
            ),
            (Self::Claude, "autoUpdatesChannel") => matches!(value, "stable" | "latest"),
            (Self::Claude, "effortLevel") => matches!(value, "low" | "medium" | "high" | "max"),
            (Self::Claude, "disableBypassPermissionsMode" | "disableAutoMode") => {
                value == "disable"
            }
            (Self::Codex, "model_reasoning_effort") => matches!(
                value,
                "none" | "minimal" | "low" | "medium" | "high" | "xhigh"
            ),
            (Self::Codex, "model_reasoning_summary") => {
                matches!(value, "auto" | "concise" | "detailed" | "none")
            }
            (Self::Codex, "model_verbosity") => matches!(value, "low" | "medium" | "high"),
            (Self::Codex, "approval_policy") => {
                matches!(value, "untrusted" | "on-failure" | "on-request" | "never")
            }
            (Self::Codex, "sandbox_mode") => matches!(
                value,
                "read-only" | "workspace-write" | "danger-full-access"
            ),
            _ => false,
        }
    }
    pub(crate) fn permits_bool(self, key: &str) -> bool {
        matches!(
            (self, key),
            (
                Self::Claude,
                "alwaysThinkingEnabled"
                    | "showTurnDuration"
                    | "voiceEnabled"
                    | "agentPushNotifEnabled"
                    | "allowManagedHooksOnly"
                    | "allowManagedPermissionRulesOnly"
                    | "allowManagedMcpServersOnly"
                    | "disableAllHooks"
            ) | (
                Self::Codex,
                "hide_agent_reasoning" | "show_raw_agent_reasoning"
            )
        )
    }
    /// Reconstruct selected values so source comments and TOML decorations
    /// cannot copy private text attached to an otherwise eligible setting.
    pub(crate) fn toml_defaults(self, source: &DocumentMut) -> DocumentMut {
        let mut defaults = DocumentMut::new();
        for (key, item) in source.iter() {
            if matches!(self, Self::Codex)
                && key == "windows"
                && let Some(table) = item.as_table_like().filter(|t| t.len() == 1)
                && let Some(mode @ ("elevated" | "unelevated")) =
                    table.get("sandbox").and_then(Item::as_str)
            {
                let mut windows = toml_edit::Table::new();
                windows.insert("sandbox", toml_edit::value(mode));
                defaults.insert(key, Item::Table(windows));
                continue;
            }
            if let Some(value) = item
                .as_str()
                .filter(|value| self.permits_string(key, value))
            {
                defaults.insert(key, toml_edit::value(value));
            } else if let Some(value) = item.as_bool().filter(|_| self.permits_bool(key)) {
                defaults.insert(key, toml_edit::value(value));
            }
        }
        defaults
    }
    /// Policy tables can contain commands, paths and credentials. If the
    /// permitted projection cannot carry one, require it in the profile.
    pub(crate) fn check_toml_policy(
        self,
        source: &DocumentMut,
        local: &DocumentMut,
        defaults: &DocumentMut,
        keep_local: impl Fn(&str) -> bool,
    ) -> Result<(), SeedError> {
        for (key, item) in source.iter() {
            if matches!(self, Self::Codex)
                && key == "hooks"
                && let Some(hooks) = item.as_table_like()
            {
                for event in ["PreToolUse", "PermissionRequest", "UserPromptSubmit"] {
                    if let Some(required) = hooks.get(event)
                        && !local
                            .get("hooks")
                            .and_then(Item::as_table_like)
                            .and_then(|h| h.get(event))
                            .is_some_and(|current| same_policy(current, required))
                    {
                        return Err(SeedError::LocalPolicyRequired);
                    }
                }
            }
            let policy = match self {
                Self::Codex => {
                    matches!(
                        key,
                        "sandbox_mode"
                            | "approval_policy"
                            | "sandbox_workspace_write"
                            | "sandbox_read_only"
                            | "permissions"
                            | "default_permissions"
                            | "shell_environment_policy"
                            | "profile"
                            | "windows"
                            | "approvals_reviewer"
                    ) || (key == "profiles" && source.contains_key("profile"))
                }
                Self::Grok => matches!(key, "permissions" | "sandbox" | "approval_policy"),
                Self::Claude => false,
            };
            if key == "windows"
                && defaults.contains_key(key)
                && local.get(key).is_some_and(|v| v.as_table_like().is_none())
            {
                return Err(SeedError::LocalPolicyRequired);
            }
            if policy
                && (!defaults.contains_key(key) || keep_local(key))
                && !local.get(key).is_some_and(|old| {
                    if let Some(preference) = defaults.get(key) {
                        same_preference(key, old, preference)
                    } else {
                        same_policy(old, item)
                    }
                })
            {
                return Err(SeedError::LocalPolicyRequired);
            }
        }
        Ok(())
    }
}

fn same_policy(a: &Item, b: &Item) -> bool {
    if let (Some(a), Some(b)) = (a.as_table_like(), b.as_table_like()) {
        return a.len() == b.len()
            && a.iter()
                .all(|(k, v)| b.get(k).is_some_and(|w| same_policy(v, w)));
    }
    if let (Some(a), Some(b)) = (a.as_array(), b.as_array()) {
        return a.len() == b.len()
            && a.iter()
                .zip(b)
                .all(|(v, w)| same_policy(&Item::Value(v.clone()), &Item::Value(w.clone())));
    }
    if let (Some(a), Some(b)) = (a.as_array_of_tables(), b.as_array_of_tables()) {
        return a.len() == b.len()
            && a.iter()
                .zip(b)
                .all(|(v, w)| same_policy(&Item::Table(v.clone()), &Item::Table(w.clone())));
    }
    match (a.as_value(), b.as_value()) {
        (Some(Value::String(x)), Some(Value::String(y))) => x.value() == y.value(),
        (Some(Value::Integer(x)), Some(Value::Integer(y))) => x.value() == y.value(),
        (Some(Value::Boolean(x)), Some(Value::Boolean(y))) => x.value() == y.value(),
        (Some(Value::Float(x)), Some(Value::Float(y))) => x.value() == y.value(),
        (Some(Value::Datetime(x)), Some(Value::Datetime(y))) => x.value() == y.value(),
        _ => false,
    }
}

pub(crate) fn same_preference(key: &str, old: &Item, projected: &Item) -> bool {
    if key == "windows" {
        return old.get("sandbox").and_then(Item::as_str)
            == projected.get("sandbox").and_then(Item::as_str);
    }
    same_policy(old, projected)
}

pub(crate) fn merge_toml_preferences(
    defaults: &DocumentMut,
    profile: &mut DocumentMut,
    owned: &BTreeSet<String>,
) -> Vec<String> {
    let mut changed = vec![];
    for (key, value) in defaults.iter() {
        if owned.contains(key)
            || profile
                .get(key)
                .is_some_and(|old| same_preference(key, old, value))
        {
            continue;
        }
        if key == "windows"
            && let Some(local) = profile.get_mut(key).and_then(Item::as_table_like_mut)
            && let Some(sandbox) = value.get("sandbox")
        {
            local.insert("sandbox", sandbox.clone());
        } else {
            profile.insert(key, value.clone());
        }
        changed.push(key.to_string());
    }
    changed.sort();
    changed
}

pub fn merge_codex(
    default_text: &str,
    profile_text: Option<&str>,
    options: &SyncOptions,
) -> Result<MergeResult, SeedError> {
    let mut source = default_text
        .parse::<DocumentMut>()
        .map_err(|_| SeedError::MalformedDefault)?;
    let removed = strip_temporary_hooks(&mut source);
    let defaults = SettingsFormat::Codex.toml_defaults(&source);
    let mut profile = profile_text
        .unwrap_or("")
        .parse::<DocumentMut>()
        .map_err(|_| SeedError::MalformedProfile)?;
    let owned = effective_owned(options);
    SettingsFormat::Codex.check_toml_policy(&source, &profile, &defaults, |key| {
        profile_text.is_some() && (!options.enabled || owned.contains(key))
    })?;
    let Some(existing) = profile_text else {
        return Ok(MergeResult {
            text: defaults.to_string(),
            changed_keys: defaults.iter().map(|(k, _)| k.to_owned()).collect(),
            removed_temporary_hooks: removed,
            created: true,
        });
    };
    let changed = if options.enabled {
        merge_toml_preferences(&defaults, &mut profile, &owned)
    } else {
        Vec::new()
    };
    Ok(MergeResult {
        text: if changed.is_empty() {
            existing.to_owned()
        } else {
            profile.to_string()
        },
        changed_keys: changed,
        removed_temporary_hooks: removed,
        created: false,
    })
}
