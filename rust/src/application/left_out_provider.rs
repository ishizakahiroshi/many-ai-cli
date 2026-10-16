//! Why a custom provider id cannot be launched.
//!
//! When one `custom_providers` entry is wrong, the settings loader leaves that
//! entry out and keeps going (see `config::ConfigProblem`). A launch request
//! for that id must then say why, not "invalid provider" or "not found". Every
//! place that refuses an unknown provider id asks this module for the reason.
//!
//! The wording is never made here: the reason is the problem's own
//! `ConfigProblem::warning_en` (built from `text_en`), with pointers to
//! `many-ai-cli config check` and, for child requests, the live valid ids. Nothing from the settings file
//! (no line, no token) can reach the text, because a problem carries none.
use crate::config::{CHECK_COMMAND, ConfigProblem, ConfigStore, normalize_subscription_id};

/// The id the way a problem shows it: trimmed, lower-cased, without control or
/// invisible formatting characters and cut at 64 characters (the same shape
/// `ConfigProblem::id` has; both come from `safety::display_id`).
fn shown_id(raw: &str) -> Option<String> {
    crate::config::safety::display_id(raw)
}

/// Why no new session can be started right now, when the settings file has
/// become unreadable while this Hub runs. The Hub keeps the settings it loaded
/// and running sessions are not affected, but every new session is started by a
/// child `many-ai-cli wrap`, which reads the file first and stops on it. Saying
/// "the Hub keeps running with its settings" would hide that. `None` when the
/// file can be read (or does not exist).
pub fn new_sessions_refusal(store: &ConfigStore) -> Option<String> {
    crate::config::safety::new_sessions_blocker(store.paths())
}

/// The reason `requested` is not available, when an entry with that id was left out.
pub fn reason_en(problems: &[ConfigProblem], requested: &str) -> Option<String> {
    let id = shown_id(requested)?;
    let found: Vec<String> = problems
        .iter()
        // An entry whose id another entry carries (`related_index`) did not make the id
        // unavailable: the other one is in use.
        .filter(|problem| {
            problem.dropped
                && problem.related_index.is_none()
                && problem.id.as_deref() == Some(id.as_str())
        })
        .map(ConfigProblem::warning_en)
        .collect();
    if found.is_empty() {
        return None;
    }
    Some(format!(
        "custom provider `{id}` is not available because its setting was left out. {} \
         Run `{CHECK_COMMAND}` (add --json for a list a program can read) to see how to write it correctly, fix the settings file, then run it again to check.",
        found.join(" ")
    ))
}

/// A short pointer for a refusal that did not name a left-out id: some other
/// entries were left out, and a mistyped id may have meant one of them.
pub fn hint_en(problems: &[ConfigProblem]) -> Option<String> {
    let dropped = problems.iter().filter(|problem| problem.dropped).count();
    (dropped > 0).then(|| {
        format!(
            "{dropped} custom_providers setting{} left out because of mistakes; run `{CHECK_COMMAND}` to see which",
            if dropped == 1 { " was" } else { "s were" }
        )
    })
}

/// `plain` is the message the caller would have given anyway. When `requested`
/// is a left-out id, its reason replaces it; otherwise `plain` stays, with a
/// pointer appended when any entry was left out.
pub fn explain(problems: &[ConfigProblem], requested: &str, plain: &str) -> String {
    if let Some(reason) = reason_en(problems, requested) {
        return reason;
    }
    // A built-in name (for example a disabled one) has nothing to do with a left-out entry.
    let id = normalize_subscription_id(requested);
    if crate::cli::BUILTIN_PROVIDERS.contains(&id.as_str()) || id == "shell" {
        return plain.to_owned();
    }
    match hint_en(problems) {
        Some(hint) => format!("{plain} ({hint})"),
        None => plain.to_owned(),
    }
}

/// The reason `requested` is not available when a save by the Hub took its
/// setting out of `config.yaml` (see `config::removed`). That outlives a restart.
fn removed_reason_en(store: &ConfigStore, requested: &str) -> Option<String> {
    let id = shown_id(requested)?;
    let notices = crate::config::removed::list(store.paths()).ok()?;
    let notice = notices.iter().find(|notice| {
        notice.kind == crate::config::removed::NoticeKind::Item
            && notice.id.as_deref() == Some(id.as_str())
    })?;
    Some(format!(
        "custom provider `{id}` is not available: a save by the Hub took its setting out of config.yaml because it could not be loaded ({}). The original is in the backup {}; `{}` brings it back. Run `{CHECK_COMMAND}` to see the details.",
        notice.text_en().reason,
        notice.backup_id,
        notice.restore_command().unwrap_or_default()
    ))
}

/// Same, for the settings the running process loaded (file positions), and for
/// settings a save of the Hub took out of the file.
pub fn explain_from(store: &ConfigStore, requested: &str, plain: &str) -> String {
    if reason_en(store.startup_problems(), requested).is_none()
        && let Some(reason) = removed_reason_en(store, requested)
    {
        return reason;
    }
    explain(store.startup_problems(), requested, plain)
}

fn is_used_custom_provider(store: &ConfigStore, requested: &str) -> bool {
    let id = normalize_subscription_id(requested);
    store
        .snapshot()
        .is_ok_and(|snapshot| snapshot.config.is_custom_provider_id(&id))
}

/// Child launch refusals keep the dropped-entry explanation and always list
/// ids that the caller verified against its live registry snapshot.
pub fn explain_for_children(
    store: &ConfigStore,
    requested: &str,
    plain: &str,
    allowed: &[String],
) -> String {
    format!(
        "{}; valid providers are: {}",
        explain_from(store, requested, plain),
        allowed.join(", ")
    )
}

/// The id as written by a person, as the one value every later step uses: a
/// custom provider id is compared trimmed and lower-cased everywhere that
/// decides (the refusal, the registry, the launch), so it is turned into that
/// form once, at the entrance. Built-in names and anything that is not a
/// custom id stay as they were written.
pub fn canonical_provider(store: &ConfigStore, provider: &str) -> String {
    let id = normalize_subscription_id(provider);
    let builtin = crate::cli::BUILTIN_PROVIDERS.contains(&provider) || provider == "shell";
    if !builtin && is_used_custom_provider(store, &id) {
        id
    } else {
        provider.to_owned()
    }
}

/// `many-ai-cli wrap <id>` / `many-ai-cli <id>`: the reason when `provider` is
/// a left-out custom id and nothing else answers to that name.
pub fn wrap_refusal(store: &ConfigStore, provider: &str) -> Option<String> {
    let id = normalize_subscription_id(provider);
    if crate::cli::BUILTIN_PROVIDERS.contains(&id.as_str()) || id == "shell" {
        return None;
    }
    let snapshot = store.snapshot().ok()?;
    if snapshot.config.is_custom_provider_id(&id) {
        return None;
    }
    reason_en(store.startup_problems(), provider).or_else(|| removed_reason_en(store, provider))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, CustomProvider, RuntimePaths};

    fn provider(id: &str, command: &str) -> CustomProvider {
        CustomProvider {
            id: id.into(),
            command: command.into(),
            ..Default::default()
        }
    }
    fn config() -> Config {
        Config {
            token: "synthetic-token-marker".into(),
            custom_providers: vec![
                provider("shell", "synthetic-command-marker"),
                provider("good", "good"),
                provider("good", "synthetic-second-marker"),
                provider("quote", "a \"b"),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn a_left_out_id_gets_its_own_reason_and_the_check_command() {
        let problems = config().custom_provider_problems();
        let reason = reason_en(&problems, " Shell ").unwrap();
        assert!(reason.contains("custom provider `shell` is not available"));
        assert!(reason.contains("`id: shell` is reserved"));
        assert!(reason.contains("was left out"));
        assert!(reason.contains("many-ai-cli config check"));
        assert!(reason.contains("--json"));
        let quote = reason_en(&problems, "quote").unwrap();
        assert!(quote.contains("double quote"));
        // A repeated id: the second use was left out, but the first works, so the id
        // is not "unavailable".
        assert!(reason_en(&problems, "good").is_none());
    }

    #[test]
    fn nothing_from_the_settings_reaches_the_text() {
        let problems = config().custom_provider_problems();
        for id in ["shell", "good", "quote", "unknown"] {
            let text = explain(&problems, id, "invalid provider");
            assert!(!text.contains("synthetic-token-marker"), "{text}");
            assert!(!text.contains("synthetic-command-marker"), "{text}");
            assert!(!text.contains("synthetic-second-marker"), "{text}");
        }
    }

    #[test]
    fn an_id_that_was_not_left_out_keeps_the_plain_message() {
        let problems = config().custom_provider_problems();
        let text = explain(&problems, "mistyped", "invalid provider");
        assert!(text.starts_with("invalid provider ("), "{text}");
        assert!(text.contains("left out because of mistakes"));
        assert!(text.contains("many-ai-cli config check"));
        assert_eq!(
            explain(&[], "mistyped", "invalid provider"),
            "invalid provider"
        );
        assert_eq!(
            explain(&problems, "claude", "invalid provider"),
            "invalid provider"
        );
        assert!(reason_en(&problems, "").is_none());
    }

    #[test]
    fn an_id_with_invisible_formatting_characters_is_shown_without_them() {
        let direction_override = char::from_u32(0x202e).unwrap();
        let zero_width = char::from_u32(0x200b).unwrap();
        let id = format!("ab{direction_override}c{zero_width}d");
        let mut entry = provider(&id, "a \"b");
        entry.label = "synthetic".into();
        let problems = Config {
            custom_providers: vec![entry],
            ..Default::default()
        }
        .custom_provider_problems();
        assert_eq!(problems[0].id.as_deref(), Some("abcd"));
        let all = format!(
            "{}{}{}",
            reason_en(&problems, &id).unwrap(),
            explain(&problems, &id, "invalid provider"),
            problems[0].warning_en()
        );
        for hidden in [direction_override, zero_width] {
            assert!(!all.contains(hidden), "{all:?}");
        }
        assert!(all.contains("`abcd`"));
    }

    #[test]
    fn new_sessions_are_refused_with_the_reason_only_while_the_file_cannot_be_read() {
        let dir = tempfile::tempdir().unwrap();
        let trial = dir.path().join("trial");
        std::fs::create_dir(&trial).unwrap();
        let paths = RuntimePaths::trial(&trial, 48888, &dir.path().join("installed")).unwrap();
        let store = ConfigStore::new(paths.clone(), config()).unwrap();
        let file = paths.resource(crate::config::Resource::Config);
        // No file: nothing to refuse.
        assert!(new_sessions_refusal(&store).is_none());
        std::fs::write(&file, "token: synthetic-token-marker\nhub: [unterminated").unwrap();
        let reason = new_sessions_refusal(&store).unwrap();
        assert!(reason.contains("cannot be read"), "{reason}");
        assert!(reason.contains("a new session cannot be started"));
        assert!(reason.contains("already running are not affected"));
        assert!(reason.contains("many-ai-cli config reset"));
        assert!(reason.contains("many-ai-cli config check"));
        assert!(!reason.contains("synthetic-token-marker"));
        assert!(!reason.contains("unterminated"));
        // A file with a mistaken entry still reads: spawning is not blocked.
        std::fs::write(
            &file,
            "token: synthetic-token-marker\ncustom_providers:\n  - id: shell\n    command: x\n",
        )
        .unwrap();
        assert!(new_sessions_refusal(&store).is_none());
        std::fs::write(&file, "token: synthetic-token-marker\n").unwrap();
        assert!(new_sessions_refusal(&store).is_none());
    }

    #[test]
    fn child_refusals_keep_dropped_reasons_and_live_valid_ids_without_categorical_ban() {
        let dir = tempfile::tempdir().unwrap();
        let trial = dir.path().join("trial");
        std::fs::create_dir(&trial).unwrap();
        let paths = RuntimePaths::trial(&trial, 48888, &dir.path().join("installed")).unwrap();
        let store = ConfigStore::new(paths, config()).unwrap();
        let allowed = vec!["claude".into(), "codex".into(), "good".into()];
        for id in ["good", "quote", "nothing-like-it"] {
            let message = explain_for_children(&store, id, "invalid provider", &allowed);
            assert!(message.ends_with("valid providers are: claude, codex, good"));
            assert!(!message.contains("cannot be used for an orchestration child yet"));
            assert!(!message.contains("synthetic-token-marker"));
            assert!(!message.contains("synthetic-command-marker"));
            if id == "quote" {
                assert!(message.contains("double quote"));
            } else {
                assert!(message.starts_with("invalid provider"));
            }
        }
        assert_eq!(canonical_provider(&store, " GOOD "), "good");
        assert_eq!(canonical_provider(&store, "claude"), "claude");
        assert_eq!(canonical_provider(&store, "shell"), "shell");
        assert_eq!(canonical_provider(&store, " Quote "), " Quote ");
    }

    #[test]
    fn wrap_refuses_only_ids_that_nothing_else_answers_to() {
        let dir = tempfile::tempdir().unwrap();
        let trial = dir.path().join("trial");
        std::fs::create_dir(&trial).unwrap();
        let paths = RuntimePaths::trial(&trial, 48888, &dir.path().join("installed")).unwrap();
        let store = ConfigStore::new(paths, config()).unwrap();
        assert!(wrap_refusal(&store, "shell").is_none());
        assert!(wrap_refusal(&store, "claude").is_none());
        // `good` has one working entry, so it is launchable.
        assert!(wrap_refusal(&store, "good").is_none());
        assert!(wrap_refusal(&store, "nothing-like-it").is_none());
        let refused = wrap_refusal(&store, "quote").unwrap();
        assert!(refused.contains("custom provider `quote`"));
    }
}
