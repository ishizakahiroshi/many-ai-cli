//! `many-ai-cli config <check|reset|backup>`.
//!
//! These commands work from the runtime paths alone, so they run even when
//! `config.yaml` cannot be read (which stops every other command). They never
//! print the Hub token or any line of the file.
use crate::config::{
    ConfigBackups, ConfigFileState, ResetError, ResetOptions, RuntimePaths, TokenOutcome,
    inspect_file,
    removed::{self, RemovedNotice},
    reset_config_with,
};
use std::io::{self, Write};

pub const USAGE: &str = "\
Usage:
  many-ai-cli config check [--json] [--dismiss-removed]
      Read the settings file and list its mistakes, and the items a save by the Hub
      took out of it (they stay listed, even after a restart, until you bring the
      backup back with `config backup restore` or say you have seen them with
      --dismiss-removed, which renames the notice records and deletes nothing).
      A notice record that cannot be read is listed too, never taken for nothing.
      Changes nothing else.
      Exit code 0 when there is nothing to report, 1 when there is.
  many-ai-cli config reset [--new-token] [--yes]
      Save the current settings as a backup, then start over from the defaults.
      The Hub token is kept (the Hub URL does not change) unless --new-token is given.
      If the file cannot be read its token cannot be kept; --yes confirms that.
  many-ai-cli config backup list [--json]
      List saved copies of the settings (kept in ~/.many-ai-cli/backups/config/).
  many-ai-cli config backup restore <id>
      Put a saved copy back. The settings in place now are saved first.
";

#[derive(Default)]
struct Flags {
    json: bool,
    dismiss_removed: bool,
    new_token: bool,
    yes: bool,
    help: bool,
    positional: Vec<String>,
}
fn parse_flags(args: &[String], allowed: &[&str]) -> Result<Flags, String> {
    let mut flags = Flags::default();
    for arg in args {
        if arg == "-" || !arg.starts_with('-') {
            flags.positional.push(arg.clone());
            continue;
        }
        let name = arg
            .strip_prefix("--")
            .or_else(|| arg.strip_prefix('-'))
            .unwrap_or(arg);
        let name = match name {
            "h" => "help",
            "y" => "yes",
            other => other,
        };
        if name != "help" && !allowed.contains(&name) {
            return Err(format!("flag provided but not defined: -{name}"));
        }
        match name {
            "json" => flags.json = true,
            "dismiss-removed" => flags.dismiss_removed = true,
            "new-token" => flags.new_token = true,
            "yes" => flags.yes = true,
            "help" => flags.help = true,
            _ => return Err(format!("flag provided but not defined: -{name}")),
        }
    }
    Ok(flags)
}

/// Run one `config` command. Returns the process exit code (0 or 1); `Err` is
/// a failure to do the work at all.
pub fn run(paths: &RuntimePaths, args: &[String], out: &mut dyn Write) -> Result<i32, String> {
    run_with(
        paths,
        args,
        out,
        &ConfigBackups::new(paths),
        &mut crate::process::random_token,
    )
}

pub fn run_with(
    paths: &RuntimePaths,
    args: &[String],
    out: &mut dyn Write,
    backups: &ConfigBackups,
    new_token: &mut dyn FnMut() -> io::Result<String>,
) -> Result<i32, String> {
    let write_error = |error: io::Error| error.to_string();
    let Some(command) = args.first().map(String::as_str) else {
        return Err("config <check|reset|backup>".into());
    };
    let rest = &args[1..];
    match command {
        "help" | "--help" | "-h" => {
            out.write_all(USAGE.as_bytes()).map_err(write_error)?;
            Ok(0)
        }
        "check" => {
            let flags = parse_flags(rest, &["json", "dismiss-removed"])?;
            if flags.help {
                out.write_all(USAGE.as_bytes()).map_err(write_error)?;
                return Ok(0);
            }
            if !flags.positional.is_empty() {
                return Err("config check takes no arguments".into());
            }
            check(paths, &flags, out)
        }
        "reset" => {
            let flags = parse_flags(rest, &["new-token", "yes"])?;
            if flags.help {
                out.write_all(USAGE.as_bytes()).map_err(write_error)?;
                return Ok(0);
            }
            if !flags.positional.is_empty() {
                return Err("config reset takes no arguments".into());
            }
            reset(paths, &flags, out, backups, new_token)
        }
        "backup" => backup(paths, rest, out, backups),
        other => Err(format!(
            "unknown config command: {other} (try `many-ai-cli config help`)"
        )),
    }
}

/// `config check`: the file as it is now, then what saves by the Hub took out of it.
fn check(paths: &RuntimePaths, flags: &Flags, out: &mut dyn Write) -> Result<i32, String> {
    let write_error = |error: io::Error| error.to_string();
    let state = inspect_file(paths).map_err(|e| format!("config check failed: {e}"))?;
    // "I have seen these" is recorded first, so the report below is what is
    // left and the exit code follows it.
    let dismissed = if flags.dismiss_removed {
        removed::dismiss_all(paths).map_err(|e| format!("config check failed: {e}"))?
    } else {
        Vec::new()
    };
    let standing = removed::list(paths).map_err(|e| format!("config check failed: {e}"))?;
    let ok = state.is_ok() && standing.is_empty();
    if flags.json {
        let mut json = state.to_json();
        json["ok"] = ok.into();
        if state.is_ok() && !standing.is_empty() {
            json["status"] = "problems".into();
        }
        json["removed_by_save"] = standing.iter().map(RemovedNotice::to_json).collect();
        json["dismissed_removed"] = dismissed.iter().map(RemovedNotice::to_json).collect();
        json["dismiss_command"] = RemovedNotice::dismiss_command().into();
        let json = serde_json::to_string_pretty(&json).map_err(|e| e.to_string())?;
        writeln!(out, "{json}").map_err(write_error)?;
        return Ok(if ok { 0 } else { 1 });
    }
    out.write_all(state.render_en().as_bytes())
        .map_err(write_error)?;
    if !dismissed.is_empty() {
        writeln!(
            out,
            "\nRecorded as seen: {} item{} that a save by the Hub had taken out of config.yaml. They are not listed again. The backups that hold them are kept.",
            dismissed.len(),
            if dismissed.len() == 1 { "" } else { "s" }
        )
        .map_err(write_error)?;
    }
    if !standing.is_empty() {
        writeln!(
            out,
            "\n{} notice{} about what a save by the Hub took out of config.yaml (what it could not load is not in the file now; the backups hold the originals):\n",
            standing.len(),
            if standing.len() == 1 { "" } else { "s" }
        )
        .map_err(write_error)?;
        for notice in &standing {
            writeln!(out, "{}", notice.render_en()).map_err(write_error)?;
        }
    }
    Ok(if ok { 0 } else { 1 })
}

fn reset(
    paths: &RuntimePaths,
    flags: &Flags,
    out: &mut dyn Write,
    backups: &ConfigBackups,
    new_token: &mut dyn FnMut() -> io::Result<String>,
) -> Result<i32, String> {
    let write_error = |error: io::Error| error.to_string();
    let options = ResetOptions {
        new_token: flags.new_token,
        confirmed: flags.yes,
    };
    match reset_config_with(paths, options, new_token, backups) {
        Ok(outcome) => {
            writeln!(
                out,
                "The settings were reset to the defaults: {}",
                paths.resource(crate::config::Resource::Config).display()
            )
            .map_err(write_error)?;
            match &outcome.backup {
                Some(backup) => {
                    writeln!(
                        out,
                        "The previous settings were saved to {}\n  Bring them back with: many-ai-cli config backup restore {}",
                        backup.path.display(),
                        backup.id
                    )
                    .map_err(write_error)?;
                }
                None => {
                    writeln!(out, "There was no settings file, so nothing needed saving.")
                        .map_err(write_error)?;
                }
            }
            let token = match outcome.token {
                TokenOutcome::Kept => {
                    "The Hub token was kept, so the Hub URL and any bookmarks still work."
                }
                TokenOutcome::NewRequested => {
                    "A new Hub token was created (as asked), so the Hub URL has changed."
                }
                TokenOutcome::NewNoneBefore => {
                    "The old settings had no Hub token, so a new one was created."
                }
                TokenOutcome::NewUnreadable => {
                    "A new Hub token was created because the old file could not be read, so the Hub URL has changed."
                }
                TokenOutcome::NewNoFile => "A new Hub token was created.",
            };
            writeln!(out, "{token}").map_err(write_error)?;
            writeln!(
                out,
                "If the Hub is running, stop it (`many-ai-cli stop`) and start it again so it uses these settings."
            )
            .map_err(write_error)?;
            Ok(0)
        }
        Err(ResetError::NeedsConfirmation(unreadable)) => {
            writeln!(
                out,
                "The settings file cannot be read: {}\n  File: {}\n\n\
                 Resetting it starts over from the default settings:\n\
                 \x20 - The current file is copied to {} first, so nothing is lost.\n\
                 \x20 - Its Hub token cannot be read, so a new token is created and the Hub URL changes.\n\n\
                 Nothing has been changed. To go ahead, run:\n\
                 \x20 many-ai-cli config reset --yes",
                unreadable.summary_en(),
                unreadable.path.display(),
                unreadable.backups.display(),
            )
            .map_err(write_error)?;
            Ok(1)
        }
        // A plain failure changed nothing. A failure that says something was
        // already changed carries its own account of what.
        Err(ResetError::Io(error)) if crate::config::is_partial_failure(&error) => {
            Err(format!("config reset failed: {error}"))
        }
        Err(ResetError::Io(error)) => Err(format!(
            "config reset failed; the settings were not changed: {error}"
        )),
    }
}

fn backup(
    paths: &RuntimePaths,
    args: &[String],
    out: &mut dyn Write,
    backups: &ConfigBackups,
) -> Result<i32, String> {
    let write_error = |error: io::Error| error.to_string();
    let Some(sub) = args.first().map(String::as_str) else {
        return Err("config backup <list|restore>".into());
    };
    let rest = &args[1..];
    match sub {
        "list" => {
            let flags = parse_flags(rest, &["json"])?;
            if flags.help {
                out.write_all(USAGE.as_bytes()).map_err(write_error)?;
                return Ok(0);
            }
            if !flags.positional.is_empty() {
                return Err("config backup list takes no arguments".into());
            }
            let records = backups
                .list()
                .map_err(|e| format!("config backup list failed: {e}"))?;
            if flags.json {
                let json = serde_json::json!({
                    "directory": backups.directory(),
                    "backups": records.iter().map(|record| serde_json::json!({
                        "id": record.id,
                        "created": record.created(),
                        "file": record.path,
                        "bytes": record.bytes,
                    })).collect::<Vec<_>>(),
                });
                writeln!(
                    out,
                    "{}",
                    serde_json::to_string_pretty(&json).map_err(|e| e.to_string())?
                )
                .map_err(write_error)?;
            } else if records.is_empty() {
                writeln!(
                    out,
                    "There are no saved copies of the settings yet ({}).",
                    backups.directory().display()
                )
                .map_err(write_error)?;
            } else {
                writeln!(
                    out,
                    "Saved copies of the settings in {} (oldest first):",
                    backups.directory().display()
                )
                .map_err(write_error)?;
                for record in &records {
                    writeln!(
                        out,
                        "  {}  {}  {} bytes",
                        record.id,
                        record.created(),
                        record.bytes
                    )
                    .map_err(write_error)?;
                }
                writeln!(
                    out,
                    "Put one back with: many-ai-cli config backup restore <id>"
                )
                .map_err(write_error)?;
            }
            Ok(0)
        }
        "restore" => {
            let flags = parse_flags(rest, &[])?;
            if flags.help {
                out.write_all(USAGE.as_bytes()).map_err(write_error)?;
                return Ok(0);
            }
            let [id] = flags.positional.as_slice() else {
                return Err(
                    "config backup restore <id> (see `many-ai-cli config backup list`)".into(),
                );
            };
            let outcome = backups.restore(id).map_err(|e| {
                if crate::config::is_partial_failure(&e) {
                    format!("config backup restore failed: {e}")
                } else {
                    format!("config backup restore failed; the settings were not changed: {e}")
                }
            })?;
            writeln!(
                out,
                "The settings were restored from the copy of {} ({}).",
                outcome.restored.created(),
                outcome.restored.id
            )
            .map_err(write_error)?;
            match &outcome.previous {
                Some(previous) => writeln!(
                    out,
                    "The settings that were in place before were saved to {}\n  Bring them back with: many-ai-cli config backup restore {}",
                    previous.path.display(),
                    previous.id
                ),
                None if outcome.already_identical => writeln!(
                    out,
                    "The settings in place were already identical to this copy, so no new copy was saved."
                ),
                None => writeln!(out, "There were no settings in place before."),
            }
            .map_err(write_error)?;
            let state = inspect_file(paths).map_err(|e| e.to_string())?;
            let mut code = 0;
            if !state.is_ok() {
                // `render_en` of an unreadable file says the file "has not been
                // changed", which is true for the commands that stop on it but
                // not here: the restore just replaced the file.
                let described = match &state {
                    ConfigFileState::Unreadable(unreadable) => unreadable.after_restore_en(),
                    other => other.render_en(),
                };
                writeln!(
                    out,
                    "\nNote: the restored settings have a problem:\n{described}"
                )
                .map_err(write_error)?;
                code = 1;
            }
            writeln!(
                out,
                "If the Hub is running, stop it (`many-ai-cli stop`) and start it again so it uses these settings."
            )
            .map_err(write_error)?;
            Ok(code)
        }
        other => Err(format!(
            "unknown config backup command: {other} (try `many-ai-cli config help`)"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Resource;
    use chrono::{NaiveDate, NaiveDateTime};

    fn trial() -> (tempfile::TempDir, tempfile::TempDir, RuntimePaths) {
        let root = tempfile::tempdir().unwrap();
        let installed = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::trial(root.path(), 49213, installed.path()).unwrap();
        (root, installed, paths)
    }
    fn at(second: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, 16)
            .unwrap()
            .and_hms_opt(9, 0, second)
            .unwrap()
    }
    fn second_1() -> NaiveDateTime {
        at(1)
    }
    fn second_2() -> NaiveDateTime {
        at(2)
    }
    fn second_3() -> NaiveDateTime {
        at(3)
    }
    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }
    fn go(
        paths: &RuntimePaths,
        clock: fn() -> NaiveDateTime,
        items: &[&str],
    ) -> (Result<i32, String>, String) {
        let mut out = Vec::new();
        let backups = ConfigBackups::with_clock(paths, clock);
        let result = run_with(paths, &args(items), &mut out, &backups, &mut || {
            Ok("synthetic-new-token".into())
        });
        (result, String::from_utf8(out).unwrap())
    }

    #[test]
    fn check_is_read_only_and_exit_codes_follow_the_state() {
        let (_root, _installed, paths) = trial();
        let config = paths.resource(Resource::Config);
        // No file: fine, and nothing is created.
        let (code, text) = go(&paths, second_1, &["check"]);
        assert_eq!(code, Ok(0), "{text}");
        assert!(text.contains("no settings file yet"));
        assert!(!config.exists());
        // A mistake in one entry.
        let original = "token: synthetic-t\ncustom_providers:\n  - id: shell\n    command: x\n  - id: ok\n    command: ok\n";
        std::fs::write(&config, original).unwrap();
        let (code, text) = go(&paths, second_1, &["check"]);
        assert_eq!(code, Ok(1));
        assert!(text.contains("entry #1 (id: shell)"));
        assert!(text.contains("1 mistake found"));
        let (code, json) = go(&paths, second_1, &["check", "--json"]);
        assert_eq!(code, Ok(1));
        let json: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(json["ok"], false);
        assert_eq!(json["status"], "problems");
        assert_eq!(json["problems"].as_array().unwrap().len(), 1);
        assert_eq!(json["problems"][0]["id"], "shell");
        assert_eq!(json["problems"][0]["index"], 1);
        // Fixed: an empty list and exit 0.
        std::fs::write(
            &config,
            "token: synthetic-t\ncustom_providers:\n  - id: ok\n    command: ok\n",
        )
        .unwrap();
        let (code, json) = go(&paths, second_1, &["check", "--json"]);
        assert_eq!(code, Ok(0));
        let json: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(json["ok"], true);
        assert!(json["problems"].as_array().unwrap().is_empty());
        // Nothing was written by any of the above.
        assert!(!paths.resource(Resource::ConfigBackups).exists());
    }

    #[test]
    fn nothing_printed_by_any_command_contains_the_token_or_a_line_of_the_file() {
        let (_root, _installed, paths) = trial();
        let config = paths.resource(Resource::Config);
        let secret = "synthetic-private-marker-4242";
        let broken = format!("token: {secret}\nhub: [unterminated\nnote: {secret}-line\n");
        std::fs::write(&config, &broken).unwrap();
        let mut everything = String::new();
        for (clock, items) in [
            (second_1 as fn() -> NaiveDateTime, vec!["check"]),
            (second_1, vec!["check", "--json"]),
            (second_1, vec!["reset"]),
            (second_1, vec!["backup", "list"]),
            (second_1, vec!["backup", "list", "--json"]),
            (second_1, vec!["help"]),
        ] {
            let (_, text) = go(&paths, clock, &items);
            everything.push_str(&text);
        }
        let (code, text) = go(&paths, second_2, &["reset", "--yes"]);
        assert_eq!(code, Ok(0), "{text}");
        everything.push_str(&text);
        let (_, text) = go(&paths, second_3, &["backup", "list"]);
        everything.push_str(&text);
        let (_, text) = go(&paths, second_3, &["backup", "restore", "20261016_090002"]);
        everything.push_str(&text);
        assert!(!everything.contains(secret), "{everything}");
        assert!(!everything.contains("synthetic-new-token"), "{everything}");
        assert!(!everything.contains("unterminated"), "{everything}");
    }

    #[test]
    fn resetting_an_unreadable_file_needs_yes_and_keeps_every_byte_in_a_backup() {
        let (_root, _installed, paths) = trial();
        let config = paths.resource(Resource::Config);
        let broken = "token: synthetic-old\nhub: [unterminated";
        std::fs::write(&config, broken).unwrap();
        let (code, text) = go(&paths, second_1, &["reset"]);
        assert_eq!(code, Ok(1));
        assert!(text.contains("Nothing has been changed"));
        assert!(text.contains("config reset --yes"));
        assert_eq!(std::fs::read_to_string(&config).unwrap(), broken);
        assert!(!paths.resource(Resource::ConfigBackups).exists());
        let (code, text) = go(&paths, second_1, &["reset", "--yes"]);
        assert_eq!(code, Ok(0), "{text}");
        assert!(text.contains("a new Hub token") || text.contains("A new Hub token"));
        let backup = paths
            .resource(Resource::ConfigBackups)
            .join("config.yaml.bak_20261016_090001");
        assert_eq!(std::fs::read_to_string(backup).unwrap(), broken);
        let fresh = std::fs::read_to_string(&config).unwrap();
        assert!(fresh.contains("synthetic-new-token"));
        assert!(crate::config::Config::from_yaml(&fresh, &paths).is_ok());
    }

    #[test]
    fn resetting_a_readable_file_keeps_the_token_and_two_resets_leave_two_backups() {
        let (_root, _installed, paths) = trial();
        let config = paths.resource(Resource::Config);
        std::fs::write(
            &config,
            "token: synthetic-kept\nuser_prefs:\n  display_name: synthetic-name\n",
        )
        .unwrap();
        let (code, text) = go(&paths, second_1, &["reset"]);
        assert_eq!(code, Ok(0), "{text}");
        assert!(text.contains("The Hub token was kept"));
        let after_first =
            crate::config::Config::from_yaml(&std::fs::read_to_string(&config).unwrap(), &paths)
                .unwrap();
        assert_eq!(after_first.token, "synthetic-kept");
        assert_eq!(after_first.user_prefs.display_name, "");
        let (code, _) = go(&paths, second_2, &["reset"]);
        assert_eq!(code, Ok(0));
        let (_, listing) = go(&paths, second_3, &["backup", "list"]);
        assert!(listing.contains("20261016_090001"), "{listing}");
        assert!(listing.contains("20261016_090002"), "{listing}");
        // The first backup is the user's original file, byte for byte.
        let first = paths
            .resource(Resource::ConfigBackups)
            .join("config.yaml.bak_20261016_090001");
        assert!(
            std::fs::read_to_string(first)
                .unwrap()
                .contains("synthetic-name")
        );
        // --new-token replaces the token.
        let (code, text) = go(&paths, second_3, &["reset", "--new-token"]);
        assert_eq!(code, Ok(0));
        assert!(text.contains("A new Hub token was created (as asked)"));
        assert!(
            std::fs::read_to_string(&config)
                .unwrap()
                .contains("synthetic-new-token")
        );
    }

    #[test]
    fn resetting_twice_in_one_second_keeps_both_copies() {
        let (_root, _installed, paths) = trial();
        let config = paths.resource(Resource::Config);
        std::fs::write(&config, "token: synthetic-first\n").unwrap();
        let (code, _) = go(&paths, second_1, &["reset"]);
        assert_eq!(code, Ok(0));
        let (code, text) = go(&paths, second_1, &["reset"]);
        assert_eq!(code, Ok(0), "{text}");
        let (_, listing) = go(&paths, second_1, &["backup", "list"]);
        assert!(listing.contains("20261016_090001 "), "{listing}");
        assert!(listing.contains("20261016_090001_2"), "{listing}");
        let dir = paths.resource(Resource::ConfigBackups);
        assert_eq!(
            std::fs::read_to_string(dir.join("config.yaml.bak_20261016_090001")).unwrap(),
            "token: synthetic-first\n"
        );
    }

    #[test]
    fn restore_brings_the_old_settings_back_and_saves_the_current_ones() {
        let (_root, _installed, paths) = trial();
        let config = paths.resource(Resource::Config);
        std::fs::write(&config, "token: synthetic-old\n").unwrap();
        let _ = go(&paths, second_1, &["reset", "--new-token"]);
        let (code, text) = go(&paths, second_2, &["backup", "restore", "20261016_090001"]);
        assert_eq!(code, Ok(0), "{text}");
        assert_eq!(
            std::fs::read_to_string(&config).unwrap(),
            "token: synthetic-old\n"
        );
        // The settings in place before the restore were saved as well.
        let saved = std::fs::read_to_string(
            paths
                .resource(Resource::ConfigBackups)
                .join("config.yaml.bak_20261016_090002"),
        )
        .unwrap();
        assert!(saved.contains("synthetic-new-token"));
        // An unknown id changes nothing.
        let (code, _) = go(&paths, second_3, &["backup", "restore", "20250101_000000"]);
        assert!(code.unwrap_err().contains("no backup with that id"));
        assert_eq!(
            std::fs::read_to_string(&config).unwrap(),
            "token: synthetic-old\n"
        );
    }

    #[test]
    fn bad_arguments_are_refused_before_anything_changes() {
        let (_root, _installed, paths) = trial();
        let config = paths.resource(Resource::Config);
        std::fs::write(&config, "token: synthetic-t\n").unwrap();
        for items in [
            vec!["reset", "--bogus"],
            vec!["check", "extra"],
            vec!["backup"],
            vec!["backup", "restore"],
            vec!["nope"],
            vec![],
        ] {
            let (result, _) = go(&paths, second_1, &items);
            assert!(result.is_err(), "{items:?}");
        }
        assert_eq!(
            std::fs::read_to_string(&config).unwrap(),
            "token: synthetic-t\n"
        );
        assert!(!paths.resource(Resource::ConfigBackups).exists());
    }
}
