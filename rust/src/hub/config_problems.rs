//! What `/api/info` tells the screen about mistakes in `config.yaml`.
//!
//! The Hub never refuses to start because one `custom_providers` entry is wrong:
//! that entry is left out and the Hub starts (see `config::ConfigProblem`). So
//! the person has to be told, and has to be able to see it go away once the
//! file is fixed. Every `/api/info` read looks at the file as it is now
//! (`config::inspect_file`, read only) and compares it with what this Hub
//! loaded at startup:
//!
//! * `problems`: mistakes in the file now, one JSON object each (the same
//!   `ConfigProblem::to_json` that `config check --json` prints, so the English
//!   wording is made in one place and the screen builds its own wording from
//!   `code` / `field` / `index` / `id`).
//! * `restart_needed`: the file now differs from what the running Hub holds
//!   (`ConfigStore::held_problems`: what it found at startup, or, once the Hub
//!   has written the file itself, the mistakes of its own settings). A running
//!   Hub does not re-read `config.yaml`, so a fixed entry is back in the spawn
//!   list only after a restart. A Hub that wrote the file holds what the file
//!   says, so nothing waits for a restart.
//!
//! A line of the file, a value from it and the Hub token are never put here: a
//! problem carries none, and an unreadable file is described by its position.
use crate::config::{
    CHECK_COMMAND, ConfigFileState, ConfigProblem, ConfigStore, RuntimePaths, inspect_file,
    removed::{self, RemovedNotice},
    safety::RESET_COMMAND,
};
use serde_json::{Value, json};
use std::sync::Arc;

type Signature = Vec<(Option<usize>, Option<String>, &'static str, &'static str)>;

/// What identifies a mistake: which entry, which field, which kind.
fn signature(problems: &[ConfigProblem]) -> Signature {
    problems
        .iter()
        .map(|p| (p.index, p.id.clone(), p.field, p.code.as_str()))
        .collect()
}

pub struct ConfigHealth {
    paths: RuntimePaths,
    config: Arc<ConfigStore>,
}
impl ConfigHealth {
    pub fn new(paths: RuntimePaths, config: Arc<ConfigStore>) -> Self {
        Self { paths, config }
    }
    /// The `config_problems` object of `/api/info`.
    pub fn view(&self) -> Value {
        view_of(
            inspect_file(&self.paths),
            &self.config.held_problems(),
            &removed::list(&self.paths).unwrap_or_default(),
        )
    }
}

fn view_of(
    file: std::io::Result<ConfigFileState>,
    loaded: &[ConfigProblem],
    removed: &[RemovedNotice],
) -> Value {
    let shared = |status: &str, problems: &[ConfigProblem], restart_needed: bool| {
        json!({
            "status": status,
            "problems": problems.iter().map(ConfigProblem::to_json).collect::<Vec<_>>(),
            // Items a save by the Hub took out of the file: kept until the person
            // restores the backup or says they have seen them. While any stand,
            // the screen does not say the file is "fixed".
            "removed_by_save": removed.iter().map(RemovedNotice::to_json).collect::<Vec<_>>(),
            "restart_needed": restart_needed,
            "check_command": CHECK_COMMAND,
        })
    };
    match file {
        Ok(ConfigFileState::Readable { problems, .. }) => {
            let restart_needed = signature(&problems) != signature(loaded);
            shared(
                if problems.is_empty() && removed.is_empty() {
                    "ok"
                } else {
                    "problems"
                },
                &problems,
                restart_needed,
            )
        }
        // No file now: no mistake in it. What a save took out of it earlier is
        // still owed to the person, so the notices are not hidden.
        Ok(ConfigFileState::Missing { .. }) => shared(
            if removed.is_empty() { "ok" } else { "problems" },
            &[],
            false,
        ),
        // The running Hub keeps the settings it loaded and running sessions go
        // on, but no new session can start: each is started by a child `wrap`
        // that reads the file first and stops on it (see
        // `left_out_provider::new_sessions_refusal`). The file needs fixing.
        Ok(ConfigFileState::Unreadable(unreadable)) => {
            let mut view = shared("unreadable", &[], false);
            view["unreadable"] = json!({
                "line": unreadable.line,
                "column": unreadable.column,
                "reason": unreadable.reason,
                "reset_command": format!("{RESET_COMMAND} --yes"),
            });
            view
        }
        // Could not look: say nothing rather than a guess.
        Err(_) => shared("unavailable", &[], false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, Resource};

    fn trial() -> (tempfile::TempDir, RuntimePaths) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("trial");
        std::fs::create_dir(&root).unwrap();
        let paths = RuntimePaths::trial(&root, 48888, &dir.path().join("installed")).unwrap();
        (dir, paths)
    }
    fn write(paths: &RuntimePaths, text: &str) {
        std::fs::write(paths.resource(Resource::Config), text).unwrap();
    }
    const BAD: &str = "token: synthetic-token-marker\ncustom_providers:\n  - id: shell\n    command: synthetic-command-marker\n  - id: good\n    command: good\n";
    const FIXED: &str =
        "token: synthetic-token-marker\ncustom_providers:\n  - id: good\n    command: good\n";

    fn loaded(paths: &RuntimePaths, text: &str) -> Vec<ConfigProblem> {
        Config::from_yaml_inspected(text, paths).unwrap().1
    }

    #[test]
    fn a_mistake_in_the_file_is_listed_and_a_fixed_file_clears_it() {
        let (_dir, paths) = trial();
        write(&paths, BAD);
        let startup = loaded(&paths, BAD);
        let view = view_of(inspect_file(&paths), &startup, &[]);
        assert_eq!(view["status"], "problems");
        assert_eq!(view["restart_needed"], false);
        let problems = view["problems"].as_array().unwrap();
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0]["index"], 1);
        assert_eq!(problems[0]["id"], "shell");
        assert_eq!(problems[0]["field"], "id");
        assert_eq!(problems[0]["code"], "reserved-id");
        assert_eq!(problems[0]["dropped"], true);
        assert_eq!(view["check_command"], "many-ai-cli config check");
        // The person fixes the file while this Hub keeps running: the mistakes
        // are gone from the list, and the Hub says it has to restart to use it.
        write(&paths, FIXED);
        let view = view_of(inspect_file(&paths), &startup, &[]);
        assert_eq!(view["status"], "ok");
        assert_eq!(view["problems"], json!([]));
        assert_eq!(view["restart_needed"], true);
        // A Hub started after the fix has nothing to report at all.
        let view = view_of(inspect_file(&paths), &loaded(&paths, FIXED), &[]);
        assert_eq!(view["status"], "ok");
        assert_eq!(view["restart_needed"], false);
    }

    #[test]
    fn nothing_from_the_file_reaches_the_view() {
        let (_dir, paths) = trial();
        write(&paths, BAD);
        let startup = loaded(&paths, BAD);
        let text = view_of(inspect_file(&paths), &startup, &[]).to_string();
        assert!(!text.contains("synthetic-token-marker"), "{text}");
        assert!(!text.contains("synthetic-command-marker"), "{text}");
        write(&paths, "token: synthetic-token-marker\nhub: [unterminated");
        let text = view_of(inspect_file(&paths), &startup, &[]).to_string();
        assert!(!text.contains("synthetic-token-marker"), "{text}");
        assert!(!text.contains("unterminated"), "{text}");
    }

    #[test]
    fn a_file_broken_while_running_is_reported_by_position_only() {
        let (_dir, paths) = trial();
        write(&paths, "token: synthetic-token-marker\nhub: [unterminated");
        let view = view_of(inspect_file(&paths), &[], &[]);
        assert_eq!(view["status"], "unreadable");
        assert_eq!(view["restart_needed"], false);
        assert!(view["unreadable"]["line"].is_number());
        assert_eq!(view["unreadable"]["reason"], "YAML syntax error");
        assert_eq!(
            view["unreadable"]["reset_command"],
            "many-ai-cli config reset --yes"
        );
    }

    #[test]
    fn no_file_is_not_a_mistake() {
        let (_dir, paths) = trial();
        let view = view_of(inspect_file(&paths), &[], &[]);
        assert_eq!(view["status"], "ok");
        assert_eq!(view["problems"], json!([]));
    }
}
