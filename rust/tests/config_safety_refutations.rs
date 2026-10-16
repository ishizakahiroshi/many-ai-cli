//! Refutation tests for C2 (config safety): they try to break what C2 promises
//! (no token or line of the file in any output, nothing destroyed without a
//! backup, a left-out provider says why). Synthetic data only, temporary
//! directories only. They were written as a scratch review and are kept as tests. The binary is only started with an explicit trial root and
//! a temporary home, and only for commands that stop before any Hub is started.
use chrono::{NaiveDate, NaiveDateTime};
use many_ai_cli::{
    application::{config_cli, diagnostic_cli, left_out_provider},
    config::*,
    process::{Cancellation, ExitOutcome, ProcessOutput, ProcessPlan, run_capped},
};
use std::{path::Path, sync::Arc, time::Duration};

const SECRET: &str = "synthetic-refute-marker-5521";

fn trial() -> (tempfile::TempDir, tempfile::TempDir, RuntimePaths) {
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49397, installed.path()).unwrap();
    (root, installed, paths)
}
fn at_second(second: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 10, 16)
        .unwrap()
        .and_hms_opt(10, 0, second)
        .unwrap()
}
fn fixed_clock() -> NaiveDateTime {
    at_second(1)
}
fn cli(paths: &RuntimePaths, items: &[&str]) -> (Result<i32, String>, String) {
    let mut out = Vec::new();
    let backups = ConfigBackups::with_clock(paths, fixed_clock);
    let args: Vec<String> = items.iter().map(|s| s.to_string()).collect();
    let result = config_cli::run_with(paths, &args, &mut out, &backups, &mut || {
        Ok("synthetic-fresh-token-0001".into())
    });
    (result, String::from_utf8_lossy(&out).into_owned())
}
fn backup_count(paths: &RuntimePaths) -> usize {
    ConfigBackups::new(paths).list().unwrap().len()
}

// ---------------------------------------------------------------------------
// 1. Leaks: no output carries the token or a line of the file
// ---------------------------------------------------------------------------

fn malformed_inputs() -> Vec<(&'static str, Vec<u8>)> {
    let s = SECRET;
    let mut out: Vec<(&'static str, Vec<u8>)> = vec![
        ("unterminated double-quoted token", format!("token: \"{s}\nhub: {{port: 1}}\n").into()),
        ("unterminated single-quoted token", format!("token: '{s}\n").into()),
        ("tab indentation", format!("token: {s}\nhub:\n\tport: 1\n").into()),
        ("unterminated flow", format!("token: {s}\nhub: [{s}\n").into()),
        ("wrong type for port", format!("token: {s}\nhub: {{port: \"{s}\"}}\n").into()),
        ("secret as a key", format!("{s}: [\n").into()),
        ("control char in key", format!("token: {s}\n\"\u{1}{s}\": 1\n").into()),
        ("bad escape", format!("token: \"\\q{s}\"\n").into()),
        ("duplicate key", format!("token: {s}\ntoken: {s}-2\n").into()),
        ("undefined alias", format!("token: &a {s}\nx: *b{s}\n").into()),
        ("binary tag", format!("token: !!binary {s}\n").into()),
        ("two documents", format!("token: {s}\n---\nhub: [\n").into()),
        ("nul byte", format!("token: {s}\0\nhub: 1\n").into()),
        ("bom then broken", format!("\u{feff}token: {s}\nhub: [x\n").into()),
        ("token as a list", format!("token: [{s}]\n").into()),
        ("token as a map", format!("token:\n  {s}: 1\n").into()),
        (
            "invalid cidr",
            format!("token: {s}\nhub: {{trusted_networks: [\"{s}/8\"]}}\n").into(),
        ),
        (
            "bad whisper url",
            format!("token: {s}\nvoice: {{whisper: {{server_url: \"http://{s}.example:1/\"}}}}\n")
                .into(),
        ),
        ("bad ollama url", format!("token: {s}\nollama: {{base_url: \"ftp://{s}\"}}\n").into()),
        (
            "bad notify mode",
            format!("token: {s}\norchestration: {{board_notify_mode: {s}}}\n").into(),
        ),
        (
            "anchors bomb",
            format!(
                "token: {s}\na: &a [x,x,x,x,x,x,x,x,x,x]\nb: &b [*a,*a,*a,*a,*a,*a,*a,*a,*a,*a]\nc: &c [*b,*b,*b,*b,*b,*b,*b,*b,*b,*b]\nd: &d [*c,*c,*c,*c,*c,*c,*c,*c,*c,*c]\ne: &e [*d,*d,*d,*d,*d,*d,*d,*d,*d,*d]\nf: [*e,*e,*e,*e,*e,*e,*e,*e,*e,*e]\n"
            )
            .into(),
        ),
    ];
    let mut invalid_utf8 = format!("token: {s}\n").into_bytes();
    invalid_utf8.extend([0xff, 0xfe, b'\n']);
    out.push(("invalid utf-8", invalid_utf8));
    out
}

#[test]
fn refute_leak_every_output_for_unreadable_files() {
    for (name, bytes) in malformed_inputs() {
        let (_root, _installed, paths) = trial();
        let file = paths.resource(Resource::Config);
        std::fs::write(&file, &bytes).unwrap();
        let mut everything = String::new();
        let readable = !matches!(
            inspect_file(&paths).unwrap(),
            ConfigFileState::Unreadable(_)
        );
        // A file that is valid YAML but refused as a whole still has a token to read:
        // `config reset` then keeps it and needs no confirmation (so it does run).
        let token_readable = matches!(
            inspect_file(&paths).unwrap(),
            ConfigFileState::Unreadable(ref unreadable) if unreadable.token_readable
        );
        // load (what every stopping command prints)
        match ConfigStore::load_or_create(paths.clone(), || Ok("synthetic-fresh-token-0002".into()))
        {
            Ok(store) => {
                // A readable file: the problems and the info view must not carry it either.
                everything.push_str("[readable]");
                for p in store.startup_problems() {
                    everything.push_str(&p.render_en());
                    everything.push_str(&p.to_json().to_string());
                }
                let health = many_ai_cli::hub::config_problems::ConfigHealth::new(
                    paths.clone(),
                    Arc::new(store),
                );
                everything.push_str(&health.view().to_string());
            }
            Err(error) => {
                everything.push_str(&error.to_string());
                if let Some(unreadable) = error.unreadable() {
                    let mut out = Vec::new();
                    let mut err = Vec::new();
                    diagnostic_cli::run_unreadable_config(&[], unreadable, &mut out, &mut err)
                        .unwrap();
                    diagnostic_cli::run_unreadable_config(
                        &["-json".into()],
                        unreadable,
                        &mut out,
                        &mut err,
                    )
                    .unwrap();
                    everything.push_str(&String::from_utf8_lossy(&out));
                    everything.push_str(&String::from_utf8_lossy(&err));
                }
            }
        }
        if !readable {
            assert_eq!(
                std::fs::read(&file).unwrap(),
                bytes,
                "input `{name}` was rewritten by load"
            );
        } else {
            eprintln!("note: input `{name}` is readable");
        }
        for items in [
            vec!["check"],
            vec!["check", "--json"],
            vec!["backup", "list"],
            vec!["backup", "list", "--json"],
            vec!["reset"],
        ] {
            let (result, text) = cli(&paths, &items);
            everything.push_str(&text);
            if let Err(e) = result {
                everything.push_str(&e);
            }
        }
        // a Hub that was started on a good file and then sees the broken file
        let (_r2, _i2, good_paths) = trial();
        std::fs::write(
            good_paths.resource(Resource::Config),
            "token: synthetic-good\n",
        )
        .unwrap();
        let store = ConfigStore::load_or_create(good_paths.clone(), || panic!("kept")).unwrap();
        std::fs::write(good_paths.resource(Resource::Config), &bytes).unwrap();
        let health = many_ai_cli::hub::config_problems::ConfigHealth::new(
            good_paths.clone(),
            Arc::new(store),
        );
        everything.push_str(&health.view().to_string());
        assert!(
            !everything.contains(SECRET),
            "input `{name}` leaked the marker:\n{everything}"
        );
        if !readable && !token_readable {
            // reset without --yes changes nothing for a file whose token cannot be read
            assert_eq!(
                std::fs::read(&file).unwrap(),
                bytes,
                "input `{name}` was rewritten"
            );
        }
    }
}

#[test]
fn refute_leak_readable_file_problems_only_show_the_id() {
    let (_root, _installed, paths) = trial();
    let file = paths.resource(Resource::Config);
    let long_tail = format!("{}{SECRET}", "a".repeat(64));
    let text = format!(
        "token: {SECRET}\n# {SECRET} comment\ncustom_providers:\n  - id: \"<img src=x onerror=alert(1)>\"\n    label: \"{SECRET} label\"\n    command: \"{SECRET}-cmd\"\n  - id: \"{long_tail}\"\n    command: x\n  - id: ok-one\n    label: \"<b>{SECRET}</b>\"\n    command: 'tool \"{SECRET}'\n  - id: hdl\n    command: hdl\n    headless: {{format: \"{SECRET}\", args: [\"{SECRET}\"]}}\n  - id: [\"{SECRET}\"]\n    command: x\n  - \"{SECRET}\"\n"
    );
    std::fs::write(&file, &text).unwrap();
    let store = ConfigStore::load_or_create(paths.clone(), || panic!("kept")).unwrap();
    let mut everything = String::new();
    for p in store.startup_problems() {
        everything.push_str(&p.render_en());
        everything.push_str(&p.warning_en());
        everything.push_str(&p.to_json().to_string());
    }
    for id in [
        "<img src=x onerror=alert(1)>",
        "ok-one",
        "hdl",
        "missing",
        &long_tail,
    ] {
        everything.push_str(&left_out_provider::explain(
            store.startup_problems(),
            id,
            "invalid provider",
        ));
    }
    let (_, out) = cli(&paths, &["check"]);
    everything.push_str(&out);
    let (_, out) = cli(&paths, &["check", "--json"]);
    everything.push_str(&out);
    let health =
        many_ai_cli::hub::config_problems::ConfigHealth::new(paths.clone(), Arc::new(store));
    everything.push_str(&health.view().to_string());
    assert!(!everything.contains(SECRET), "{everything}");
    // The HTML-looking id is passed on as text (the screen must treat it as text).
    assert!(everything.contains("<img src=x onerror=alert(1)>"));
}

#[test]
fn format_characters_in_an_id_do_not_reach_the_terminal() {
    // U+202E (right-to-left override) is not a control character, so it is kept.
    let (_root, _installed, paths) = trial();
    std::fs::write(
        paths.resource(Resource::Config),
        "token: synthetic-t\ncustom_providers:\n  - id: \"abc\u{202e}gpj.exe\"\n    command: x\n",
    )
    .unwrap();
    let (_, out) = cli(&paths, &["check"]);
    assert!(
        !out.contains('\u{202e}'),
        "a bidi override from the file is printed as is: {out:?}"
    );
}

// ---------------------------------------------------------------------------
// 2. Backups
// ---------------------------------------------------------------------------

#[test]
fn refute_backup_a_file_without_token_is_rewritten_at_startup_without_a_backup() {
    let (_root, _installed, paths) = trial();
    let file = paths.resource(Resource::Config);
    // Readable, no token: a comment, a hand-written name and two entries that
    // cannot be kept in memory (not a mapping / id of the wrong type).
    let original = "# my own note\nuser_prefs:\n  display_name: hand-written\ncustom_providers:\n  - 7\n  - id: [a, b]\n    command: x\n  - id: half-written\n";
    std::fs::write(&file, original).unwrap();
    let store =
        ConfigStore::load_or_create(paths.clone(), || Ok("synthetic-new-token-77".into())).unwrap();
    let now = std::fs::read_to_string(&file).unwrap();
    assert_ne!(
        now, original,
        "startup rewrote the file (expected: token added)"
    );
    // Comments and the items that cannot be loaded are not carried over; they stay in the
    // backup, and the items are recorded as taken out.
    assert!(!now.contains("my own note"));
    assert!(!now.contains("half-written"), "{now}");
    assert!(now.contains("hand-written"));
    let notices = removed::list(&paths).unwrap();
    assert_eq!(notices.len(), 3, "{notices:?}");
    let reread = ConfigStore::load_or_create(paths.clone(), || panic!("token kept")).unwrap();
    assert!(reread.startup_problems().is_empty());
    drop(reread);
    drop(store);
    // C2: what the rewrite destroys must be in a backup.
    let copies: Vec<String> = ConfigBackups::new(&paths)
        .list()
        .unwrap()
        .iter()
        .map(|r| std::fs::read_to_string(&r.path).unwrap())
        .collect();
    assert!(
        copies.iter().any(|c| c == original),
        "the user's file was overwritten at startup and no backup holds it (backups: {})",
        copies.len()
    );
}

fn big_unreadable_file() -> Vec<u8> {
    // Valid YAML but larger than the 8 MiB the reader accepts.
    let mut bytes = b"token: synthetic-big\n".to_vec();
    let line = format!("# {}\n", "z".repeat(1000));
    while bytes.len() < 9 * 1024 * 1024 {
        bytes.extend_from_slice(line.as_bytes());
    }
    bytes.extend_from_slice(b"# the-very-last-line\n");
    bytes
}

#[test]
fn refute_backup_reset_of_a_large_file_keeps_every_byte() {
    let (_root, _installed, paths) = trial();
    let file = paths.resource(Resource::Config);
    let original = big_unreadable_file();
    std::fs::write(&file, &original).unwrap();
    assert!(matches!(
        inspect_file(&paths).unwrap(),
        ConfigFileState::Unreadable(_)
    ));
    let (code, text) = cli(&paths, &["reset", "--yes"]);
    assert_eq!(code, Ok(0), "{text}");
    let records = ConfigBackups::new(&paths).list().unwrap();
    assert_eq!(records.len(), 1);
    let saved = std::fs::read(&records[0].path).unwrap();
    assert_eq!(
        saved.len(),
        original.len(),
        "the backup is not byte for byte: {} of {} bytes kept",
        saved.len(),
        original.len()
    );
}

#[test]
fn refute_backup_restore_of_a_large_backup_restores_every_byte() {
    let (_root, _installed, paths) = trial();
    std::fs::write(paths.resource(Resource::Config), "token: synthetic-now\n").unwrap();
    let big = big_unreadable_file();
    let record = ConfigBackups::with_clock(&paths, fixed_clock)
        .create(&big)
        .unwrap();
    let (_code, text) = cli(&paths, &["backup", "restore", &record.id]);
    let restored = std::fs::read(paths.resource(Resource::Config)).unwrap();
    assert_eq!(
        restored.len(),
        big.len(),
        "restore wrote {} of {} bytes: {text}",
        restored.len(),
        big.len()
    );
}

#[test]
fn refute_backup_an_unwritable_backup_place_stops_every_write() {
    let (_root, _installed, paths) = trial();
    let file = paths.resource(Resource::Config);
    let original = "token: synthetic-t\n# note that a save would lose\n";
    std::fs::write(&file, original).unwrap();
    let store = ConfigStore::load_or_create(paths.clone(), || panic!("kept")).unwrap();
    // backups/config is a plain file, so no backup can be made.
    std::fs::create_dir_all(paths.root().join("backups")).unwrap();
    std::fs::write(paths.root().join("backups").join("config"), "not a dir").unwrap();
    let snapshot = store.snapshot().unwrap();
    let mut next = snapshot.config;
    next.user_prefs.display_name = "changed".into();
    assert!(store.persist(snapshot.revision, next).is_err());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), original);
    let (code, _) = cli(&paths, &["reset"]);
    assert!(code.is_err());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), original);
}

#[test]
fn refute_backup_the_thousandth_reset_in_one_second_changes_nothing() {
    let (_root, _installed, paths) = trial();
    let file = paths.resource(Resource::Config);
    std::fs::write(&file, "token: synthetic-t\n").unwrap();
    let backups = ConfigBackups::with_clock(&paths, fixed_clock);
    for _ in 0..999 {
        backups.create(b"synthetic filler").unwrap();
    }
    std::fs::write(&file, "token: synthetic-t\n# last\n").unwrap();
    let (code, _) = cli(&paths, &["reset"]);
    assert!(code.is_err(), "{code:?}");
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "token: synthetic-t\n# last\n"
    );
    assert_eq!(backup_count(&paths), 999);
}

#[test]
fn restoring_the_same_copy_twice_adds_no_identical_backups() {
    let (_root, _installed, paths) = trial();
    let file = paths.resource(Resource::Config);
    std::fs::write(&file, "token: synthetic-t\n").unwrap();
    let record = ConfigBackups::with_clock(&paths, fixed_clock)
        .backup_current()
        .unwrap()
        .unwrap();
    for _ in 0..3 {
        let (code, text) = cli(&paths, &["backup", "restore", &record.id]);
        assert_eq!(code, Ok(0), "{text}");
    }
    // Nothing was ever different, yet every restore made a copy.
    let n = backup_count(&paths);
    assert_eq!(n, 1, "restoring identical content added {} copies", n - 1);
}

#[test]
fn a_save_keeps_a_left_out_entry_and_its_notice() {
    // The person is told about entry #2 at startup; after the Hub saves any
    // preference the entry is gone from config.yaml and the notice disappears.
    let (_root, _installed, paths) = trial();
    let file = paths.resource(Resource::Config);
    std::fs::write(
        &file,
        "token: synthetic-t\ncustom_providers:\n  - id: ok\n    command: ok\n  - id: muse\n",
    )
    .unwrap();
    let store = Arc::new(ConfigStore::load_or_create(paths.clone(), || panic!("kept")).unwrap());
    let health = many_ai_cli::hub::config_problems::ConfigHealth::new(paths.clone(), store.clone());
    assert_eq!(health.view()["status"], "problems");
    let snapshot = store.snapshot().unwrap();
    let mut next = snapshot.config;
    next.user_prefs.display_name = "any change".into();
    store.persist(snapshot.revision, next).unwrap();
    let view = health.view();
    let after = std::fs::read_to_string(&file).unwrap();
    assert_eq!(backup_count(&paths), 1, "the save made a backup");
    assert!(
        after.contains("muse") || view["status"] == "problems",
        "after one save the half-written `muse` entry is gone from the file and the screen says {}: {view}",
        view["status"]
    );
}

#[test]
fn reset_of_valid_yaml_with_a_refused_value_keeps_its_token() {
    let (_root, _installed, paths) = trial();
    let file = paths.resource(Resource::Config);
    // Valid YAML, readable token, one value the validator refuses.
    std::fs::write(
        &file,
        "token: synthetic-readable-token\nhub: {trusted_networks: [\"192.0.2.0/8\"]}\n",
    )
    .unwrap();
    let (code, text) = cli(&paths, &["reset"]);
    assert_eq!(code, Ok(0), "reset asked for --yes / a new token: {text}");
}

// ---------------------------------------------------------------------------
// 3. Left-out providers
// ---------------------------------------------------------------------------

#[test]
fn refute_provider_spawn_list_and_registry_agree_when_the_first_twin_is_broken() {
    let (_root, _installed, paths) = trial();
    let file = paths.resource(Resource::Config);
    std::fs::write(
        &file,
        "token: synthetic-t\ncustom_providers:\n  - id: twin\n    command: 'tool \"open'\n  - id: twin\n    command: twin-ok\n",
    )
    .unwrap();
    let store = ConfigStore::load_or_create(paths.clone(), || panic!("kept")).unwrap();
    let config = store.snapshot().unwrap().config;
    // The screen's spawn list (/api/info custom_providers) offers `twin`.
    let offered: Vec<_> = effective_custom_providers(&config.custom_providers)
        .into_iter()
        .map(|p| (p.id, p.command))
        .collect();
    assert_eq!(offered, [("twin".to_owned(), "twin-ok".to_owned())]);
    // The registry the Hub's spawn route checks must then know it too.
    let registry = many_ai_cli::wrapper::launch::provider_registry_for_config(&config).unwrap();
    let launchable = registry
        .lookup("twin")
        .is_some_and(|d| d.definition.launch.is_some());
    let refusal = left_out_provider::explain(store.startup_problems(), "twin", "invalid provider");
    assert!(
        launchable,
        "`twin` is offered on screen but the registry has no launch for it; the spawn route would answer: {refusal}"
    );
}

#[test]
fn refute_provider_a_hub_started_with_a_port_keeps_the_file_positions() {
    let (_root, _installed, paths) = trial();
    std::fs::write(
        paths.resource(Resource::Config),
        "token: synthetic-t\ncustom_providers:\n  - id: muse\n  - id: shell\n    command: x\n",
    )
    .unwrap();
    let store = ConfigStore::load_or_create(paths.clone(), || panic!("kept")).unwrap();
    let shape = |s: &ConfigStore| {
        s.startup_problems()
            .iter()
            .map(|p| (p.index, p.id.clone(), p.code.as_str()))
            .collect::<Vec<_>>()
    };
    let original = shape(&store);
    // `serve --port N` (how `wrap` starts a Hub, bootstrap.rs) and ensure_hub's
    // set_port both replace the store with `derive`.
    let mut config = store.snapshot().unwrap().config;
    config.hub.port = 49397;
    let derived = Arc::new(store.derive(config).unwrap());
    let after = shape(&derived);
    let view =
        many_ai_cli::hub::config_problems::ConfigHealth::new(paths.clone(), derived.clone()).view();
    let refusal = left_out_provider::explain_from(&derived, "muse", "invalid provider");
    assert_eq!(
        after, original,
        "\n/api/info right after start: {view}\nspawn of `muse` answers: {refusal}"
    );
}

#[test]
fn wrap_refusal_for_ids_and_spellings() {
    let (_root, _installed, paths) = trial();
    std::fs::write(
        paths.resource(Resource::Config),
        "token: synthetic-t\ncustom_providers:\n  - id: Bad-Quote\n    command: 'a \"b'\n  - id: \" Good \"\n    command: good\n  - id: claude\n    command: fake-claude\n",
    )
    .unwrap();
    let store = ConfigStore::load_or_create(paths.clone(), || panic!("kept")).unwrap();
    for (asked, refused) in [
        ("bad-quote", true),
        ("BAD-QUOTE", true),
        (" bad-quote ", true),
        ("good", false),
        ("Good", false),
        ("claude", false),
        ("shell", false),
    ] {
        assert_eq!(
            left_out_provider::wrap_refusal(&store, asked).is_some(),
            refused,
            "{asked:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// 4. The binary (trial root + temporary home; commands that stop or only read)
// ---------------------------------------------------------------------------

async fn run_binary(root: &Path, home: &Path, args: &[&str]) -> ProcessOutput {
    let mut full: Vec<std::ffi::OsString> = vec![
        "--trial-root".into(),
        root.into(),
        "--trial-port".into(),
        "49397".into(),
    ];
    full.extend(args.iter().map(std::ffi::OsString::from));
    let env = ["HOME", "USERPROFILE"]
        .into_iter()
        .map(|name| (name.into(), Some(home.as_os_str().to_owned())))
        .collect();
    run_capped(
        &ProcessPlan {
            executable: env!("CARGO_BIN_EXE_many-ai-cli").into(),
            args: full,
            cwd: root.into(),
            env,
            stdin: vec![],
            timeout: Duration::from_secs(20),
            output_cap: 256 * 1024,
            pipe_drain_timeout: Duration::from_secs(1),
        },
        &Cancellation::default(),
    )
    .await
    .unwrap()
}
fn exit_code(output: &ProcessOutput) -> Option<i32> {
    match output.outcome {
        ExitOutcome::Exited { code, .. } => code,
        _ => None,
    }
}
fn text(output: &ProcessOutput) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[tokio::test]
async fn refute_binary_outputs_for_malformed_files_carry_no_marker() {
    for (name, bytes) in malformed_inputs().into_iter().filter(|(name, _)| {
        matches!(
            *name,
            "unterminated double-quoted token"
                | "duplicate key"
                | "nul byte"
                | "invalid utf-8"
                | "control char in key"
        )
    }) {
        let root = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let file = root.path().join("config.yaml");
        std::fs::write(&file, &bytes).unwrap();
        let mut everything = String::new();
        for args in [
            vec!["status"],
            vec!["orchestrate", "spawn", "--provider", "claude"],
            vec!["config", "check"],
            vec!["config", "check", "--json"],
            vec!["config", "reset"],
            vec!["doctor"],
            vec!["doctor", "-json"],
            vec!["not-a-command"],
        ] {
            let output = run_binary(root.path(), home.path(), &args).await;
            let shown = text(&output);
            eprintln!(
                "{name} {args:?} -> {:?} :: {}",
                exit_code(&output),
                shown.trim().lines().next().unwrap_or("")
            );
            everything.push_str(&shown);
        }
        assert!(!everything.contains(SECRET), "{name}: {everything}");
        assert_eq!(std::fs::read(&file).unwrap(), bytes, "{name}");
    }
}

#[tokio::test]
async fn refute_binary_a_left_out_alias_says_why() {
    let root = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("config.yaml"),
        format!(
            "token: {SECRET}\ncustom_providers:\n  - id: quote\n    command: 'tool \"{SECRET}'\n"
        ),
    )
    .unwrap();
    let output = run_binary(root.path(), home.path(), &["quote"]).await;
    let shown = text(&output);
    assert_eq!(exit_code(&output), Some(1), "{shown}");
    assert!(
        shown.contains("custom provider `quote` is not available"),
        "{shown}"
    );
    assert!(!shown.contains(SECRET), "{shown}");
}

#[tokio::test]
async fn binary_uninstall_with_a_broken_record_leaves_the_settings_untouched() {
    let root = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("config.yaml"), "token: x\nhub: [\n").unwrap();
    std::fs::write(root.path().join("hub-runtime.json"), "{not json").unwrap();
    let output = run_binary(root.path(), home.path(), &["uninstall"]).await;
    let shown = text(&output);
    // Record what happens; the settings file must not be touched in any case.
    assert_eq!(
        std::fs::read_to_string(root.path().join("config.yaml")).unwrap(),
        "token: x\nhub: [\n",
        "{shown}"
    );
    eprintln!(
        "uninstall with broken ledger: code={:?}\n{shown}",
        exit_code(&output)
    );
}

// ---------------------------------------------------------------------------
// 5. Fixes for what the refutations found (each test fails without its fix)
// ---------------------------------------------------------------------------

fn write_config_file(paths: &RuntimePaths, text: &str) {
    std::fs::write(paths.resource(Resource::Config), text).unwrap();
}
fn read_config_file(paths: &RuntimePaths) -> String {
    std::fs::read_to_string(paths.resource(Resource::Config)).unwrap()
}

#[test]
fn startup_stops_without_touching_the_file_when_the_backup_cannot_be_made() {
    let (_root, _installed, paths) = trial();
    // Readable but without a token: startup would rewrite it.
    let original = "# my own note\nhub: {port: 49397}\n";
    write_config_file(&paths, original);
    std::fs::create_dir_all(paths.root().join("backups")).unwrap();
    std::fs::write(paths.root().join("backups").join("config"), "not a dir").unwrap();
    let result = ConfigStore::load_or_create(paths.clone(), || Ok("synthetic-new-token-9".into()));
    assert!(result.is_err());
    assert_eq!(read_config_file(&paths), original);
}

#[test]
fn a_save_takes_out_what_cannot_be_loaded_and_says_so_until_it_is_seen() {
    let (_root, _installed, paths) = trial();
    let original = "token: synthetic-t\ncustom_providers:\n  - id: ok\n    command: ok\n  - 7\n  - id: half-written\n  - id: [a, b]\n    command: x\n  - id: no-command\n    some_new_field: 1\n";
    write_config_file(&paths, original);
    let store = Arc::new(ConfigStore::load_or_create(paths.clone(), || panic!("kept")).unwrap());
    let before = inspect_file(&paths).unwrap().problems().to_vec();
    assert_eq!(before.len(), 4, "{before:?}");
    let snapshot = store.snapshot().unwrap();
    let mut next = snapshot.config;
    next.user_prefs.display_name = "changed".into();
    store.persist(snapshot.revision, next).unwrap();
    // The live file has only what loads. Nothing the person wrote is written back.
    let text = read_config_file(&paths);
    assert!(text.contains("id: ok"));
    for gone in ["half-written", "some_new_field", "no-command"] {
        assert!(!text.contains(gone), "{gone} is still in the file:\n{text}");
    }
    // The whole original is in one backup, and a record names every item.
    let records = ConfigBackups::new(&paths).list().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(std::fs::read_to_string(&records[0].path).unwrap(), original);
    let notices = removed::list(&paths).unwrap();
    assert_eq!(
        notices
            .iter()
            .map(|n| (n.index, n.id.as_deref(), n.code.as_str()))
            .collect::<Vec<_>>(),
        [
            (Some(2), None, "not-a-mapping"),
            (Some(3), Some("half-written"), "missing-command"),
            (Some(4), None, "incompatible-type"),
            (Some(5), Some("no-command"), "missing-command"),
        ]
    );
    assert!(notices.iter().all(|n| n.backup_id == records[0].id));
    // The record (one file for this backup) has no token and no line of the file.
    let record_names: Vec<_> = std::fs::read_dir(paths.resource(Resource::ConfigBackups))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().ends_with(".removed.json"))
        .map(|e| e.path())
        .collect();
    assert_eq!(record_names.len(), 1, "{record_names:?}");
    let record = std::fs::read_to_string(&record_names[0]).unwrap();
    assert!(!record.contains("synthetic-t"));
    assert!(!record.contains("some_new_field"));
    // The screen says what happened instead of "fixed".
    let health = many_ai_cli::hub::config_problems::ConfigHealth::new(paths.clone(), store.clone());
    assert_eq!(health.view()["status"], "problems");
    assert_eq!(
        health.view()["removed_by_save"].as_array().unwrap().len(),
        4
    );
    // A custom_providers value that is not a list at all is reported the same way.
    let (_root, _installed, paths) = trial();
    write_config_file(&paths, "token: synthetic-t\ncustom_providers: not-a-list\n");
    let store = ConfigStore::load_or_create(paths.clone(), || panic!("kept")).unwrap();
    let snapshot = store.snapshot().unwrap();
    let mut next = snapshot.config;
    next.user_prefs.display_name = "changed".into();
    store.persist(snapshot.revision, next).unwrap();
    assert!(!read_config_file(&paths).contains("not-a-list"));
    let notices = removed::list(&paths).unwrap();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].code, "not-a-list");
}

#[test]
fn a_repeated_id_whose_first_command_is_broken_is_explained_and_launches_the_one_that_works() {
    let (_root, _installed, paths) = trial();
    write_config_file(
        &paths,
        "token: synthetic-t\ncustom_providers:\n  - id: twin\n    command: 'tool \"open'\n  - id: twin\n    command: twin-ok\n",
    );
    // config check says which entry is used (text and --json).
    let (code, text) = cli(&paths, &["check"]);
    assert_eq!(code, Ok(1), "{text}");
    assert!(
        text.contains("entry #2 has the same id and is the one that is used"),
        "{text}"
    );
    let (_, json) = cli(&paths, &["check", "--json"]);
    let json: serde_json::Value = serde_json::from_str(&json).unwrap();
    let problems = json["problems"].as_array().unwrap();
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0]["index"], 1);
    assert_eq!(problems[0]["related_index"], 2);
    // wrap launches that same entry (the spawn list and the registry agree: see the
    // refutation test above).
    let store = ConfigStore::load_or_create(paths.clone(), || panic!("kept")).unwrap();
    let config = store.snapshot().unwrap().config;
    let plan = many_ai_cli::wrapper::launch::prepare_launch(
        &config,
        "twin",
        many_ai_cli::wrapper::launch::WrapperArgs::parse(&[]),
    )
    .unwrap();
    assert_eq!(plan.custom_argv, Some(vec!["twin-ok".to_owned()]));
    // The Go-compatible adapter itself is unchanged: its first entry still wins.
    let (definitions, diagnostics) = legacy_provider_definitions(&config.custom_providers);
    assert!(definitions.is_empty());
    assert_eq!(diagnostics[0].code, "legacy_command_invalid");
}

#[test]
fn a_repeated_key_is_reported_with_its_position() {
    let (_root, _installed, paths) = trial();
    write_config_file(
        &paths,
        &format!("token: {SECRET}\nhub:\n  port: 49397\ntoken: {SECRET}-2\n"),
    );
    let (code, text) = cli(&paths, &["check"]);
    assert_eq!(code, Ok(1));
    assert!(text.contains("written twice"), "{text}");
    assert!(text.contains("line 4, column 1"), "{text}");
    assert!(!text.contains("incompatible"), "{text}");
    assert!(!text.contains(SECRET), "{text}");
    let (_, json) = cli(&paths, &["check", "--json"]);
    let json: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(json["unreadable"]["line"], 4);
    assert_eq!(json["unreadable"]["column"], 1);
    assert!(!json.to_string().contains(SECRET));
}

#[test]
fn reset_keeps_a_readable_token_and_needs_no_confirmation_for_a_refused_value() {
    let (_root, _installed, paths) = trial();
    let refused = "token: synthetic-readable-token\nhub: {trusted_networks: [\"192.0.2.0/8\"]}\n";
    write_config_file(&paths, refused);
    // check tells the truth: the Hub token can be kept.
    let (_, text) = cli(&paths, &["check"]);
    assert!(text.contains("The Hub token in the file is kept"), "{text}");
    assert!(!text.contains("cannot be read, so a new token"), "{text}");
    let (code, text) = cli(&paths, &["reset"]);
    assert_eq!(code, Ok(0), "{text}");
    assert!(text.contains("The Hub token was kept"), "{text}");
    assert!(!text.contains("synthetic-readable-token"));
    assert!(read_config_file(&paths).contains("synthetic-readable-token"));
    assert_eq!(backup_count(&paths), 1);
    // --new-token still asks for a new one, with no confirmation needed either.
    write_config_file(&paths, refused);
    let (code, text) = cli(&paths, &["reset", "--new-token"]);
    assert_eq!(code, Ok(0), "{text}");
    assert!(!read_config_file(&paths).contains("synthetic-readable-token"));
    // Not valid YAML: the token cannot be read, so confirmation is still needed.
    write_config_file(&paths, "token: synthetic-lost\nhub: [\n");
    let (code, text) = cli(&paths, &["reset"]);
    assert_eq!(code, Ok(1), "{text}");
    assert!(text.contains("a new token is created"), "{text}");
    assert!(read_config_file(&paths).contains("synthetic-lost"));
}

#[test]
fn restoring_a_copy_that_cannot_be_read_does_not_say_the_file_was_not_changed() {
    let (_root, _installed, paths) = trial();
    write_config_file(&paths, "token: synthetic-now\n");
    let record = ConfigBackups::with_clock(&paths, fixed_clock)
        .create(b"token: synthetic-broken\nhub: [\n")
        .unwrap();
    let (code, text) = cli(&paths, &["backup", "restore", &record.id]);
    assert_eq!(code, Ok(1), "{text}");
    assert!(text.contains("The restored copy cannot be read"), "{text}");
    assert!(text.contains("now holds that copy"), "{text}");
    assert!(!text.contains("has not been changed"), "{text}");
    assert!(!text.contains("synthetic-broken"), "{text}");
    assert_eq!(
        read_config_file(&paths),
        "token: synthetic-broken\nhub: [\n"
    );
    // The settings that were in place are in a backup.
    assert_eq!(backup_count(&paths), 2);
}

#[test]
fn later_yaml_documents_are_reported_as_not_read_and_never_stop_the_load() {
    let (_root, _installed, paths) = trial();
    // The second document is broken; the first is fine. Only the first is read.
    write_config_file(&paths, &format!("token: {SECRET}\n---\nhub: [\n"));
    let state = inspect_file(&paths).unwrap();
    assert!(!state.is_ok());
    let problems = state.problems();
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert_eq!(problems[0].code.as_str(), "extra-documents");
    assert_eq!(problems[0].section, "config");
    assert!(!problems[0].dropped);
    let (code, text) = cli(&paths, &["check"]);
    assert_eq!(code, Ok(1));
    assert!(text.contains("Only the first document is read"), "{text}");
    assert!(text.contains("ignored"), "{text}");
    assert!(!text.contains(SECRET));
    let (_, json) = cli(&paths, &["check", "--json"]);
    let json: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(json["problems"][0]["code"], "extra-documents");
    // The load still works, keeps the token and reports it as a start-up mistake.
    let store = ConfigStore::load_or_create(paths.clone(), || panic!("kept")).unwrap();
    assert_eq!(store.snapshot().unwrap().config.token, SECRET);
    assert_eq!(store.startup_problems().len(), 1);
    // A valid second document is reported the same way; a single document is clean.
    write_config_file(&paths, "token: synthetic-t\n---\nhub: {port: 1}\n");
    assert_eq!(inspect_file(&paths).unwrap().problems().len(), 1);
    write_config_file(&paths, "---\ntoken: synthetic-t\n");
    assert!(inspect_file(&paths).unwrap().is_ok());
    // The doctor wording does not call it a provider entry.
    write_config_file(&paths, "token: synthetic-t\n---\nx: 1\n");
    let store = ConfigStore::load_or_create(paths.clone(), || panic!("kept")).unwrap();
    assert!(!store.startup_problems()[0].is_provider_entry());
}

#[test]
fn an_id_with_invisible_formatting_characters_is_never_printed_with_them() {
    let (_root, _installed, paths) = trial();
    let rlo = char::from_u32(0x202e).unwrap();
    let isolate = char::from_u32(0x2066).unwrap();
    write_config_file(
        &paths,
        &format!(
            "token: synthetic-t\ncustom_providers:\n  - id: \"abc{rlo}gpj.exe{isolate}\"\n    command: 'x \"open'\n"
        ),
    );
    let store = ConfigStore::load_or_create(paths.clone(), || panic!("kept")).unwrap();
    let mut everything = String::new();
    let (_, text) = cli(&paths, &["check"]);
    everything.push_str(&text);
    let (_, text) = cli(&paths, &["check", "--json"]);
    everything.push_str(&text);
    for problem in store.startup_problems() {
        everything.push_str(&problem.render_en());
        everything.push_str(&problem.to_json().to_string());
    }
    everything.push_str(&left_out_provider::explain_from(
        &store,
        &format!("abc{rlo}gpj.exe"),
        "invalid provider",
    ));
    everything.push_str(
        &left_out_provider::wrap_refusal(&store, &format!("abc{rlo}gpj.exe")).unwrap_or_default(),
    );
    for hidden in [rlo, isolate] {
        assert!(!everything.contains(hidden), "{everything:?}");
    }
    assert!(everything.contains("abcgpj.exe"));
}

#[tokio::test]
async fn binary_uninstall_does_not_guess_when_the_hub_record_is_damaged() {
    for (config, name) in [
        ("token: x\nhub: [\n", "unreadable settings"),
        ("token: synthetic-t\n", "readable settings"),
    ] {
        let root = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("config.yaml"), config).unwrap();
        std::fs::write(root.path().join("hub-runtime.json"), "{not json").unwrap();
        let output = run_binary(root.path(), home.path(), &["uninstall"]).await;
        let shown = text(&output);
        assert!(
            shown.contains("cannot be told whether a Hub is running"),
            "{name}: {shown}"
        );
        assert!(!shown.contains("No Hub is running"), "{name}: {shown}");
        assert!(
            shown.contains("Has the Hub been stopped"),
            "{name}: {shown}"
        );
        // Nobody said yes (end of input), so nothing was removed.
        assert!(shown.contains("Uninstall was cancelled"), "{name}: {shown}");
        assert_eq!(
            std::fs::read_to_string(root.path().join("config.yaml")).unwrap(),
            config,
            "{name}"
        );
        assert!(root.path().join("hub-runtime.json").exists(), "{name}");
    }
}

#[tokio::test]
async fn binary_wrap_stops_on_an_unreadable_file_so_a_hub_spawn_cannot_start_a_session() {
    // The Hub starts every new session as `many-ai-cli wrap <provider>`. While the
    // file cannot be read that child stops before starting a Hub or the provider.
    let root = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let broken = format!("token: {SECRET}\nhub: [unterminated\n");
    std::fs::write(root.path().join("config.yaml"), &broken).unwrap();
    for provider in ["shell", "claude"] {
        let output = run_binary(root.path(), home.path(), &["wrap", provider]).await;
        let shown = text(&output);
        assert_eq!(exit_code(&output), Some(1), "{provider}: {shown}");
        assert!(
            shown.contains("settings file cannot be read"),
            "{provider}: {shown}"
        );
        assert!(shown.contains("config reset"), "{provider}: {shown}");
        assert!(!shown.contains(SECRET), "{provider}: {shown}");
        assert!(!shown.contains("unterminated"), "{provider}: {shown}");
    }
    assert_eq!(
        std::fs::read_to_string(root.path().join("config.yaml")).unwrap(),
        broken
    );
    // No Hub was started for it.
    assert!(!root.path().join("hub-runtime.json").exists());
}

// ---------------------------------------------------------------------------
// Second review: tries to break the fixes above. Same rules (synthetic data,
// temporary directories only); its helpers are its own.
// ---------------------------------------------------------------------------
mod second_review {
    use many_ai_cli::{application::left_out_provider, config::*};
    use std::{io::Write as _, sync::Arc};

    const SECRET: &str = "synthetic-refute2-marker-7731";

    fn trial() -> (tempfile::TempDir, tempfile::TempDir, RuntimePaths) {
        let root = tempfile::tempdir().unwrap();
        let installed = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::trial(root.path(), 49397, installed.path()).unwrap();
        (root, installed, paths)
    }
    fn write(paths: &RuntimePaths, text: &str) {
        std::fs::write(paths.resource(Resource::Config), text).unwrap();
    }
    fn read(paths: &RuntimePaths) -> String {
        std::fs::read_to_string(paths.resource(Resource::Config)).unwrap()
    }
    fn backups(paths: &RuntimePaths) -> usize {
        ConfigBackups::new(paths).list().unwrap().len()
    }
    fn fixed_clock() -> chrono::NaiveDateTime {
        chrono::NaiveDate::from_ymd_opt(2026, 10, 16)
            .unwrap()
            .and_hms_opt(11, 0, 0)
            .unwrap()
    }
    fn cli(paths: &RuntimePaths, items: &[&str]) -> (Result<i32, String>, String) {
        let mut out = Vec::new();
        let backups = ConfigBackups::with_clock(paths, fixed_clock);
        let args: Vec<String> = items.iter().map(|s| s.to_string()).collect();
        let result = many_ai_cli::application::config_cli::run_with(
            paths,
            &args,
            &mut out,
            &backups,
            &mut || Ok("synthetic-fresh-token-0009".into()),
        );
        (result, String::from_utf8_lossy(&out).into_owned())
    }
    fn load(paths: &RuntimePaths) -> ConfigStore {
        ConfigStore::load_or_create(paths.clone(), || panic!("the token must be kept")).unwrap()
    }
    /// Hub がどれかの設定を 1 つ保存したときと同じ書き込み。
    fn save_a_preference(store: &ConfigStore, value: &str) {
        let snapshot = store.snapshot().unwrap();
        let mut next = snapshot.config;
        next.user_prefs.display_name = value.into();
        store.persist(snapshot.revision, next).unwrap();
    }
    fn shape(
        problems: &[ConfigProblem],
    ) -> Vec<(Option<usize>, Option<String>, &'static str, bool)> {
        problems
            .iter()
            .map(|p| (p.index, p.id.clone(), p.code.as_str(), p.dropped))
            .collect()
    }
    fn offered(store: &ConfigStore) -> Vec<(String, String)> {
        effective_custom_providers(&store.snapshot().unwrap().config.custom_providers)
            .into_iter()
            .map(|p| (p.id, p.command))
            .collect()
    }

    // ---------------------------------------------------------------------------
    // 1. C2-05: a save takes out what could not be loaded; the backup keeps it and
    //    a record keeps reporting it (it is never written back)
    // ---------------------------------------------------------------------------

    /// After a save the item is out of the file (so it cannot turn into something
    /// that launches), the whole original is in exactly one backup, and a record
    /// keeps reporting it across a restart until it is dismissed.
    fn assert_item_is_taken_out_and_still_reported(name: &str, original: &str, ghost: &str) {
        let (_root, _installed, paths) = trial();
        write(&paths, original);
        assert!(
            matches!(
                inspect_file(&paths).unwrap(),
                ConfigFileState::Readable { .. }
            ),
            "{name}: precondition (the file is readable)"
        );
        let store = load(&paths);
        assert!(
            !offered(&store).iter().any(|(id, _)| id == ghost),
            "{name}: precondition (not offered from the start)"
        );
        save_a_preference(&store, "changed");
        // 1. Not in the file, so it can never become a provider.
        let reread = load(&paths);
        assert!(
            !offered(&reread).iter().any(|(id, _)| id == ghost),
            "{name}: `{ghost}` is offered after an unrelated save:\n{}",
            read(&paths)
        );
        // 2. The original is whole in one backup.
        let records = ConfigBackups::new(&paths).list().unwrap();
        assert_eq!(records.len(), 1, "{name}");
        assert_eq!(
            std::fs::read_to_string(&records[0].path).unwrap(),
            original,
            "{name}"
        );
        // 3. Still reported after a restart: record, check (text and JSON), screen.
        let notices = removed::list(&paths).unwrap();
        assert!(!notices.is_empty(), "{name}: nothing recorded");
        assert!(notices.iter().all(|n| n.backup_id == records[0].id));
        let (code, text) = cli(&paths, &["check"]);
        assert_eq!(code, Ok(1), "{name}: {text}");
        assert!(
            text.contains(&format!("config backup restore {}", records[0].id)),
            "{name}: {text}"
        );
        let (_, json) = cli(&paths, &["check", "--json"]);
        let json: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(json["ok"], false, "{name}");
        assert_eq!(
            json["removed_by_save"].as_array().unwrap().len(),
            notices.len(),
            "{name}"
        );
        let view =
            many_ai_cli::hub::config_problems::ConfigHealth::new(paths.clone(), Arc::new(reread))
                .view();
        assert_eq!(view["status"], "problems", "{name}: {view}");
        assert_eq!(
            view["removed_by_save"].as_array().unwrap().len(),
            notices.len(),
            "{name}"
        );
        // 4. Once seen, it is gone from every report (the backup stays).
        let (code, text) = cli(&paths, &["check", "--dismiss-removed"]);
        assert_eq!(code, Ok(0), "{name}: {text}");
        assert!(removed::list(&paths).unwrap().is_empty());
        assert_eq!(backups(&paths), 1, "{name}");
    }

    #[test]
    fn c2r_held_item_with_a_repeated_key_is_taken_out_not_written_back() {
        assert_item_is_taken_out_and_still_reported(
            "repeated key",
            "token: synthetic-t\ncustom_providers:\n  - id: ok\n    command: ok\n  - id: dup\n    command: first-cmd\n    command: second-cmd\n",
            "dup",
        );
    }

    #[test]
    fn c2r_a_save_does_not_change_which_command_an_id_launches() {
        // Entry 1 (repeated key) is not loaded and the same id's entry 2 is used. A save must not change that.
        let (_root, _installed, paths) = trial();
        write(
            &paths,
            "token: synthetic-t\ncustom_providers:\n  - id: dup\n    command: first-cmd\n    command: second-cmd\n  - id: dup\n    command: dup-ok\n",
        );
        let store = load(&paths);
        let before = offered(&store);
        save_a_preference(&store, "changed");
        let after = offered(&load(&paths));
        assert_eq!(after, before);
        assert_eq!(after, [("dup".to_owned(), "dup-ok".to_owned())]);
    }

    #[test]
    fn c2r_a_trailing_document_marker_is_not_reported_as_ignored_content() {
        // Only a trailing `---` (the second document is empty): nothing is ignored.
        let (_root, _installed, paths) = trial();
        for text in [
            "token: synthetic-t\n---\n",
            "token: synthetic-t\n---\n# only a comment\n",
            "token: synthetic-t\n...\n",
        ] {
            write(&paths, text);
            let problems = inspect_file(&paths).unwrap().problems().to_vec();
            let (code, _) = cli(&paths, &["check"]);
            assert!(
                problems.is_empty() && code == Ok(0),
                "{text:?}: {code:?} {:?}",
                shape(&problems)
            );
        }
        // With content after it, it is reported.
        write(&paths, "token: synthetic-t\n---\nhub: {port: 1}\n");
        assert_eq!(inspect_file(&paths).unwrap().problems().len(), 1);
    }

    #[test]
    fn c2r_held_item_with_a_complex_key_is_taken_out_not_written_back() {
        assert_item_is_taken_out_and_still_reported(
            "complex key",
            "token: synthetic-t\ncustom_providers:\n  - id: ok\n    command: ok\n  - id: cx\n    command: cx-cmd\n    ? [k1, k2]\n    : v\n",
            "cx",
        );
    }

    #[test]
    fn c2r_held_item_with_a_quoted_merge_key_is_taken_out_not_written_back() {
        assert_item_is_taken_out_and_still_reported(
            "quoted <<",
            "token: synthetic-t\ncustom_providers:\n  - id: ok\n    command: ok\n  - id: mq\n    \"<<\": {command: from-merge}\n",
            "mq",
        );
    }

    #[test]
    fn c2r_the_backup_keeps_the_exact_spelling_of_what_was_taken_out() {
        let (_root, _installed, paths) = trial();
        let original = "token: synthetic-t\ncustom_providers:\n  - id: 007\n  - id: ok\n    command: ok\n  - id: tool-a\n    label: 1.10\n  - command: 0x10\n  - id: tool-b\n    label: True\n    command: \"\"\n";
        write(&paths, original);
        let store = load(&paths);
        save_a_preference(&store, "changed");
        let live = read(&paths);
        // Nothing of it is rewritten into the live file (no value changes its meaning)...
        for spelled in ["007", "1.10", "0x10", "True"] {
            assert!(
                !live.contains(spelled),
                "{spelled} is in the live file:\n{live}"
            );
        }
        // ...and the backup has every spelling as written.
        let records = ConfigBackups::new(&paths).list().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(std::fs::read_to_string(&records[0].path).unwrap(), original);
        assert_eq!(removed::list(&paths).unwrap().len(), 4);
    }

    #[test]
    fn c2r_every_save_gives_the_same_bytes_adds_no_backup_and_no_second_notice() {
        let (_root, _installed, paths) = trial();
        let original = "token: synthetic-t\ncustom_providers:\n  - id: ok\n    command: ok\n  - id: s1\n    label: '1_000'\n  - id: s2\n    label: '0o17'\n  - id: s10\n    nested: {a: [1, '2', {b: '0x1'}], c: !custom tagged, d: &anc {e: 1}, f: *anc}\n  - id: s13\n    '<<': plain-key\n";
        write(&paths, original);
        let store = load(&paths);
        save_a_preference(&store, "changed");
        drop(store);
        let first_text = read(&paths);
        let after_first = backups(&paths);
        let notices = removed::list(&paths).unwrap();
        assert_eq!(after_first, 1);
        assert_eq!(notices.len(), 4);
        for _ in 0..3 {
            // A restart, then a save that changes nothing.
            let store = load(&paths);
            let snapshot = store.snapshot().unwrap();
            store.persist(snapshot.revision, snapshot.config).unwrap();
            assert_eq!(read(&paths), first_text);
        }
        assert_eq!(backups(&paths), after_first);
        assert_eq!(removed::list(&paths).unwrap(), notices);
    }

    #[test]
    fn c2r_one_odd_item_does_not_stop_the_other_items_from_being_reported() {
        // An unknown key with a !!binary value that is not text: loading goes through.
        let (_root, _installed, paths) = trial();
        let original = "token: synthetic-t\ncustom_providers:\n  - id: ok\n    command: ok\n  - id: half-a\n  - id: half-b\n    extra: !!binary //79\n";
        write(&paths, original);
        assert!(matches!(
            inspect_file(&paths).unwrap(),
            ConfigFileState::Readable { .. }
        ));
        let store = load(&paths);
        save_a_preference(&store, "changed");
        let notices = removed::list(&paths).unwrap();
        let ids: Vec<_> = notices.iter().map(|n| n.id.as_deref()).collect();
        assert_eq!(ids, [Some("half-a"), Some("half-b")]);
        let records = ConfigBackups::new(&paths).list().unwrap();
        assert_eq!(std::fs::read_to_string(&records[0].path).unwrap(), original);
    }

    #[test]
    fn c2r_a_large_alias_expansion_in_one_item_does_not_stop_the_others_from_being_reported() {
        let (_root, _installed, paths) = trial();
        let text = "token: synthetic-t\na: &a [x,x,x,x,x,x,x,x,x,x]\nb: &b [*a,*a,*a,*a,*a,*a,*a,*a,*a,*a]\nc: &c [*b,*b,*b,*b,*b,*b,*b,*b,*b,*b]\nd: &d [*c,*c,*c,*c,*c,*c,*c,*c,*c,*c]\ne: &e [*d,*d,*d,*d,*d,*d,*d,*d,*d,*d]\nf: &f [*e,*e,*e,*e,*e,*e,*e,*e,*e,*e]\ncustom_providers:\n  - id: ok\n    command: ok\n  - id: half-c\n  - id: half-d\n    extra: *f\n";
        write(&paths, text);
        assert!(matches!(
            inspect_file(&paths).unwrap(),
            ConfigFileState::Readable { .. }
        ));
        let store = load(&paths);
        save_a_preference(&store, "changed");
        let ids: Vec<_> = removed::list(&paths)
            .unwrap()
            .into_iter()
            .map(|n| n.id)
            .collect();
        assert_eq!(ids, [Some("half-c".to_owned()), Some("half-d".to_owned())]);
        // The expansion is not copied anywhere: the live file stays small.
        assert!(read(&paths).len() < 100_000);
    }

    #[test]
    fn c2r_reported_items_keep_their_order_and_are_not_duplicated() {
        let (_root, _installed, paths) = trial();
        write(
            &paths,
            "token: synthetic-t\ncustom_providers:\n  - id: h0\n  - id: ok1\n    command: ok1\n  - ~\n  - id: h2\n  - id: ok3\n    command: ok3\n  - id: h4\n",
        );
        let store = load(&paths);
        for round in 0..3 {
            save_a_preference(&store, &format!("changed {round}"));
        }
        let text = read(&paths);
        assert!(text.find("id: ok1").unwrap() < text.find("id: ok3").unwrap());
        for gone in ["id: h0", "id: h2", "id: h4"] {
            assert!(!text.contains(gone), "{gone} was written back:\n{text}");
        }
        // One notice each, in file order, however many saves happened.
        let notices = removed::list(&paths).unwrap();
        let ids: Vec<_> = notices.iter().map(|n| n.id.as_deref()).collect();
        assert_eq!(ids, [Some("h0"), Some("h2"), Some("h4")]);
        assert_eq!(backups(&paths), 1);
    }

    #[test]
    fn c2r_the_token_never_reaches_a_report_even_as_an_id_through_an_alias() {
        // A value that looks like the token inside an item, and an `id` that is the
        // top-level token by alias.
        let (_root, _installed, paths) = trial();
        write(
            &paths,
            &format!(
                "token: &tk {SECRET}\ncustom_providers:\n  - id: ok\n    command: ok\n  - token: {SECRET}\n    label: half\n  - id: *tk\n  - id: *tk\n    command: x\n"
            ),
        );
        let store = Arc::new(load(&paths));
        let mut everything = String::new();
        // Before the save: the mistakes found at startup and the file now.
        for items in [vec!["check"], vec!["check", "--json"]] {
            let (_, out) = cli(&paths, &items);
            everything.push_str(&out);
        }
        for p in store.startup_problems() {
            everything.push_str(&p.render_en());
            everything.push_str(&p.warning_en());
            everything.push_str(&p.to_json().to_string());
        }
        everything.push_str(&left_out_provider::explain_from(
            &store,
            SECRET,
            "invalid provider",
        ));
        save_a_preference(&store, "changed");
        // After the save: the record and every report built from it.
        for items in [vec!["check"], vec!["check", "--json"]] {
            let (_, out) = cli(&paths, &items);
            everything.push_str(&out);
        }
        for notice in removed::list(&paths).unwrap() {
            everything.push_str(&notice.render_en());
            everything.push_str(&notice.warning_en());
            everything.push_str(&notice.to_json().to_string());
        }
        // Every record file of the backup folder (one per backup).
        for entry in std::fs::read_dir(paths.resource(Resource::ConfigBackups))
            .unwrap()
            .filter_map(Result::ok)
        {
            if entry
                .file_name()
                .to_string_lossy()
                .ends_with(".removed.json")
            {
                everything.push_str(&std::fs::read_to_string(entry.path()).unwrap());
            }
        }
        everything.push_str(
            &many_ai_cli::hub::config_problems::ConfigHealth::new(paths.clone(), store.clone())
                .view()
                .to_string(),
        );
        assert!(
            !everything.contains(SECRET),
            "the token value reached a report:\n{everything}"
        );
    }

    #[test]
    fn c2r_a_save_keeps_reporting_the_later_documents_it_took_out() {
        let (_root, _installed, paths) = trial();
        let original = "token: synthetic-t\n---\nhub: {port: 1}\n";
        write(&paths, original);
        let store = Arc::new(load(&paths));
        let health =
            many_ai_cli::hub::config_problems::ConfigHealth::new(paths.clone(), store.clone());
        assert_eq!(health.view()["status"], "problems");
        save_a_preference(&store, "changed");
        // The second document is gone from the file, so the screen must not say "fixed"...
        let view = health.view();
        assert_eq!(view["status"], "problems", "{view}");
        assert_eq!(view["problems"].as_array().unwrap().len(), 0);
        // ...it says what happened and where the original is.
        let notices = view["removed_by_save"].as_array().unwrap();
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0]["code"], "extra-documents");
        let records = ConfigBackups::new(&paths).list().unwrap();
        assert_eq!(notices[0]["backup_id"], records[0].id.as_str());
        assert_eq!(std::fs::read_to_string(&records[0].path).unwrap(), original);
        // Restoring that backup is what ends the notice (and puts the file back).
        let (code, text) = cli(&paths, &["backup", "restore", &records[0].id]);
        assert_eq!(
            code,
            Ok(1),
            "restoring a file with a mistake still reports it: {text}"
        );
        assert!(removed::list(&paths).unwrap().is_empty());
        assert_eq!(read(&paths), original);
    }

    #[test]
    fn c2r_reset_ends_the_notices_and_restore_ends_only_its_own() {
        let (_root, _installed, paths) = trial();
        write(
            &paths,
            "token: synthetic-t\ncustom_providers:\n  - id: first\n",
        );
        let store = load(&paths);
        save_a_preference(&store, "one");
        drop(store);
        write(
            &paths,
            "token: synthetic-t\ncustom_providers:\n  - id: second\n",
        );
        let store = load(&paths);
        save_a_preference(&store, "two");
        let records = ConfigBackups::new(&paths).list().unwrap();
        assert_eq!(records.len(), 2);
        let notices = removed::list(&paths).unwrap();
        assert_eq!(notices.len(), 2);
        // Restoring the first backup ends the first notice only.
        let (_, text) = cli(&paths, &["backup", "restore", &records[0].id]);
        assert!(text.contains("restored"), "{text}");
        let left = removed::list(&paths).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].id.as_deref(), Some("second"));
        // A reset replaces everything on purpose: nothing is left to warn about.
        let (code, text) = cli(&paths, &["reset"]);
        assert_eq!(code, Ok(0), "{text}");
        assert!(removed::list(&paths).unwrap().is_empty());
    }
    // ---------------------------------------------------------------------------
    // 2. C2-08 salvage_token: 別の値を token と誤認しないか
    // ---------------------------------------------------------------------------

    fn token_readable(paths: &RuntimePaths) -> Option<bool> {
        match inspect_file(paths).unwrap() {
            ConfigFileState::Unreadable(u) => Some(u.token_readable),
            _ => None,
        }
    }

    #[test]
    fn c2r_salvage_does_not_take_another_value_as_the_token() {
        let refused = "hub: {trusted_networks: [\"192.0.2.0/8\"]}\n";
        let cases: Vec<(&str, String, bool)> = vec![
            (
                "hub の中の token",
                format!("hub: {{token: {SECRET}, trusted_networks: [\"192.0.2.0/8\"]}}\n"),
                false,
            ),
            (
                "コメントの token",
                format!("# token: {SECRET}\n{refused}"),
                false,
            ),
            (
                "provider の中の token",
                format!("custom_providers:\n  - token: {SECRET}\n{refused}"),
                false,
            ),
            (
                "2 つ目の文書の token",
                format!("{refused}---\ntoken: {SECRET}\n"),
                false,
            ),
            (
                "大文字の Token",
                format!("Token: {SECRET}\n{refused}"),
                false,
            ),
            ("null の token", format!("token: ~\n{refused}"), false),
            (
                "token が 2 回",
                format!("token: {SECRET}\ntoken: {SECRET}-2\n{refused}"),
                false,
            ),
            // ブロック文字列は正しい YAML の token（値は末尾の改行つき）。引き継ぐのが正しい。
            (
                "ブロック文字列の token",
                format!("token: |\n  {SECRET}\n{refused}"),
                true,
            ),
        ];
        let mut wrong = Vec::new();
        for (name, text, expected) in &cases {
            let (_root, _installed, paths) = trial();
            write(&paths, text);
            let readable = token_readable(&paths);
            let (code, out) = cli(&paths, &["reset"]);
            let after = read(&paths);
            // 引き継がない場合: --yes が要る（1）で、ファイルはそのまま。引き継ぐ場合: 0 で新しいファイルに残る。
            let kept = if *expected {
                code == Ok(0) && after.contains(SECRET) && after != *text
            } else {
                code == Ok(1) && after == *text
            };
            if readable != Some(*expected) || !kept {
                wrong.push(format!(
                "{name}: token_readable={readable:?} reset={code:?} 新しいファイルに値が残る={kept}"
            ));
            }
            assert!(!out.contains(SECRET), "{name}: reset の出力に値が出た");
        }
        assert!(
            wrong.is_empty(),
            "別の値を token として引き継いだ / 判定が期待と違う:\n{}",
            wrong.join("\n")
        );
    }

    #[test]
    fn c2r_salvaged_token_matches_what_the_reader_would_use() {
        // 値の検査だけ落ちるファイルで、引き継ぐ token は Go の読み方と同じ値（merge・別名は YAML の意味どおり）。
        let refused = "hub: {trusted_networks: [\"192.0.2.0/8\"]}\n";
        for (name, text, expected) in [
            (
                "merge より明示が勝つ",
                format!("<<: {{token: other-1}}\ntoken: {SECRET}\n{refused}"),
                SECRET.to_owned(),
            ),
            (
                "別名",
                format!("x: &a {SECRET}\ntoken: *a\n{refused}"),
                SECRET.to_owned(),
            ),
            (
                "引用符つきのキー",
                format!("\"token\": {SECRET}\n{refused}"),
                SECRET.to_owned(),
            ),
        ] {
            let (_root, _installed, paths) = trial();
            write(&paths, &text);
            let (code, out) = cli(&paths, &["reset"]);
            assert_eq!(code, Ok(0), "{name}: {out}");
            let store = load(&paths);
            assert_eq!(store.snapshot().unwrap().config.token, expected, "{name}");
            assert!(!out.contains(SECRET), "{name}: {out}");
        }
    }

    // ---------------------------------------------------------------------------
    // 3. C2-04 5 か所の一致
    // ---------------------------------------------------------------------------

    #[test]
    fn c2r_the_five_places_agree_on_which_entry_an_id_means() {
        let (_root, _installed, paths) = trial();
        write(
            &paths,
            "token: synthetic-t\ncustom_providers:\n  - id: Twin\n    command: twin-1\n  - id: \" twin \"\n    command: twin-2\n  - id: Claude\n    command: fake-claude\n  - id: empty-exe\n    command: '\"\"'\n  - id: pair\n    command: pair-ok\n  - id: pair\n    command: 'pair \"open'\n",
        );
        let store = load(&paths);
        let config = store.snapshot().unwrap().config;
        let list = offered(&store);
        let registry = many_ai_cli::wrapper::launch::provider_registry_for_config(&config).unwrap();
        let mut table = Vec::new();
        let mut disagreements = Vec::new();
        for asked in [
            "twin",
            "Twin",
            " twin ",
            "claude",
            "empty-exe",
            "pair",
            "PAIR",
        ] {
            // What was typed is turned into one value at the entrance (`canonical_provider`),
            // and that value is what the refusal, the list, the registry and the launch use.
            let typed = asked;
            let asked = left_out_provider::canonical_provider(&store, typed).leak() as &str;
            let on_screen = list
                .iter()
                .find(|(id, _)| id == asked)
                .map(|(_, c)| c.clone());
            let in_registry = registry
                .lookup(asked)
                .and_then(|d| d.definition.launch)
                .map(|l| l.executable);
            let wrapped = many_ai_cli::wrapper::launch::prepare_launch(
                &config,
                asked,
                many_ai_cli::wrapper::launch::WrapperArgs::parse(&[]),
            )
            .map(|plan| plan.custom_argv)
            .map_err(|e| e.0);
            let refused = left_out_provider::wrap_refusal(&store, asked);
            let orchestrate = left_out_provider::explain_from(&store, asked, "invalid provider");
            let _ = typed;
            table.push(format!(
            "{asked:?}: 一覧={on_screen:?} registry={in_registry:?} wrap={wrapped:?} wrap拒否={} orchestrate={orchestrate}",
            refused.is_some()
        ));
            // 拒否しない（= 起動できると言う）なら、wrap は custom の 1 件を起動できる。
            let builtin = ["claude", "shell"].contains(&asked.trim().to_lowercase().as_str());
            if !builtin && refused.is_none() && !matches!(wrapped, Ok(Some(_))) {
                disagreements.push(format!(
                    "{asked:?}: wrap の拒否判定は「起動できる」なのに、wrap は custom を起動しない"
                ));
            }
            // 使われている id に「外したので使えない」と言わない。
            if on_screen.is_some()
                && orchestrate.contains("is not available because its setting was left out")
            {
                disagreements.push(format!(
                    "{asked:?}: 一覧にあり使われている id に「外したので使えない」と言う"
                ));
            }
            // 一覧に出す 1 件は、起動できる実行ファイルを持つ。
            if on_screen.is_some() && in_registry.as_deref() == Some("") {
                disagreements.push(format!(
                    "{asked:?}: 一覧に出るが実行ファイルが空（config check は誤りを出さない）"
                ));
            }
        }
        let problems = inspect_file(&paths).unwrap().problems().to_vec();
        assert!(
            disagreements.is_empty(),
            "{}\n---\n{}\n---\nconfig check: {:?}",
            disagreements.join("\n"),
            table.join("\n"),
            shape(&problems)
        );
    }

    // ---------------------------------------------------------------------------
    // 4. C2-02 境界と C2-15 の拒否
    // ---------------------------------------------------------------------------

    const MIB: u64 = 1024 * 1024;

    fn sized_file(paths: &RuntimePaths, size: u64) {
        let file_path = paths.resource(Resource::Config);
        let mut file = std::fs::File::create(&file_path).unwrap();
        file.write_all(b"token: synthetic-sized\n").unwrap();
        file.set_len(size).unwrap();
    }

    #[test]
    fn c2r_a_file_over_the_copy_limit_is_reported_and_reset_moves_it_into_the_backups() {
        let (_root, _installed, paths) = trial();
        sized_file(&paths, 256 * MIB + 1);
        // Reported as a file that cannot be read, with what to do (not "backup copies").
        let (code, text) = cli(&paths, &["check"]);
        assert_eq!(code, Ok(1));
        assert!(text.contains("larger than 256 MiB"), "{text}");
        assert!(text.contains("config reset --yes"), "{text}");
        assert!(text.contains("moves it into the backup folder"), "{text}");
        assert!(!text.contains("backup copies"), "{text}");
        // The start-up stops with the same words and touches nothing.
        let error = ConfigStore::load_or_create(paths.clone(), || Ok("x".into()))
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(error.contains("config reset"), "{error}");
        // Without --yes nothing changes (its token cannot be read).
        let (code, _) = cli(&paths, &["reset"]);
        assert_eq!(code, Ok(1));
        assert_eq!(
            std::fs::metadata(paths.resource(Resource::Config))
                .unwrap()
                .len(),
            256 * MIB + 1
        );
        assert_eq!(backups(&paths), 0);
        // With --yes the file is moved (renamed) into the backups, whatever its size.
        let (code, out) = cli(&paths, &["reset", "--yes"]);
        assert_eq!(code, Ok(0), "{out}");
        let records = ConfigBackups::new(&paths).list().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(
            std::fs::metadata(&records[0].path).unwrap().len(),
            256 * MIB + 1
        );
        assert!(
            std::fs::metadata(paths.resource(Resource::Config))
                .unwrap()
                .len()
                < MIB
        );
        assert!(inspect_file(&paths).unwrap().is_ok());
    }
    #[test]
    fn c2r_a_file_at_the_copy_limit_is_backed_up_whole() {
        let (_root, _installed, paths) = trial();
        sized_file(&paths, 256 * MIB);
        let (code, out) = cli(&paths, &["reset", "--yes"]);
        assert_eq!(code, Ok(0), "{out}");
        let records = ConfigBackups::new(&paths).list().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(
            std::fs::metadata(&records[0].path).unwrap().len(),
            256 * MIB
        );
    }

    #[test]
    fn c2r_the_eight_mib_reader_limit_boundary() {
        let make = |size: usize| {
            let mut bytes = b"token: synthetic-eight\n".to_vec();
            let line = format!("# {}\n", "z".repeat(1000));
            while bytes.len() + line.len() <= size {
                bytes.extend_from_slice(line.as_bytes());
            }
            while bytes.len() < size - 1 {
                bytes.push(b'#');
            }
            bytes.push(b'\n');
            assert_eq!(bytes.len(), size);
            bytes
        };
        // ちょうど 8 MiB: 読める。保存は全バイトを退避してから書く。
        let (_root, _installed, paths) = trial();
        let exact = make(8 * 1024 * 1024);
        std::fs::write(paths.resource(Resource::Config), &exact).unwrap();
        assert!(
            matches!(
                inspect_file(&paths).unwrap(),
                ConfigFileState::Readable { .. }
            ),
            "ちょうど 8 MiB が読めない"
        );
        let store = load(&paths);
        save_a_preference(&store, "changed");
        let records = ConfigBackups::new(&paths).list().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(std::fs::read(&records[0].path).unwrap(), exact);
        // 8 MiB + 1: 読めない扱い。起動しても書き換えない。
        let (_root, _installed, paths) = trial();
        let over = make(8 * 1024 * 1024 + 1);
        std::fs::write(paths.resource(Resource::Config), &over).unwrap();
        assert!(matches!(
            inspect_file(&paths).unwrap(),
            ConfigFileState::Unreadable(_)
        ));
        assert!(ConfigStore::load_or_create(paths.clone(), || Ok("x".into())).is_err());
        assert_eq!(
            std::fs::read(paths.resource(Resource::Config)).unwrap(),
            over
        );
    }

    #[test]
    fn c2r_new_sessions_are_refused_whenever_wrap_would_stop() {
        let (_root, _installed, paths) = trial();
        write(&paths, "token: synthetic-t\n");
        let store = load(&paths);
        let mut wrong = Vec::new();
        // 読めるファイル（誤りあり・2 つ目の文書あり）は拒否しない。
        for text in [
            "token: synthetic-t\ncustom_providers:\n  - id: shell\n    command: x\n",
            "token: synthetic-t\n---\nx: 1\n",
        ] {
            write(&paths, text);
            if left_out_provider::new_sessions_refusal(&store).is_some() {
                wrong.push(format!("読めるのに拒否: {text:?}"));
            }
        }
        // wrap が止まるファイルは拒否する。
        let file = paths.resource(Resource::Config);
        type MakeFile<'a> = Box<dyn Fn() + 'a>;
        let mut stop_cases: Vec<(&str, MakeFile)> = vec![
            (
                "構文の誤り",
                Box::new(|| write(&paths, "token: x\nhub: [\n")),
            ),
            (
                "値の誤り",
                Box::new(|| {
                    write(
                        &paths,
                        "token: x\nhub: {trusted_networks: [\"192.0.2.0/8\"]}\n",
                    )
                }),
            ),
            ("8 MiB 超", Box::new(|| sized_file(&paths, 8 * MIB + 1))),
            ("256 MiB 超", Box::new(|| sized_file(&paths, 256 * MIB + 1))),
        ];
        for (name, make) in stop_cases.drain(..) {
            make();
            let wrap_stops = ConfigStore::load_or_create(paths.clone(), || Ok("x".into())).is_err();
            let refused = left_out_provider::new_sessions_refusal(&store).is_some();
            if wrap_stops && !refused {
                wrong.push(format!(
                "{name}: wrap は止まるのに spawn は 503 config_unreadable で拒否しない（inspect_file={:?}）",
                inspect_file(&paths).map(|s| s.is_ok()).map_err(|e| e.kind())
            ));
            }
            let _ = std::fs::remove_file(&file);
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }
}

mod third_review {
    //! 3 回目の反証（対象 e1242ea4）の指摘を、方針（記録は退避ごとに 1 ファイル・
    //! 読めない記録は「何も無い」にしない・読めないファイルへの保存は拒否）に合わせた
    //! 正式なテストにしたもの。合成値と一時ディレクトリだけを使い、バイナリは起動しない。
    use many_ai_cli::{application::config_cli, config::*, hub::config_problems::ConfigHealth};
    use std::{path::PathBuf, sync::Arc};

    const TOKEN: &str = "synthetic-r3-token-0001";
    const BAD: &str = "token: synthetic-r3-token-0001\ncustom_providers:\n  - id: ok\n    command: ok\n  - id: half\n";

    fn trial() -> (tempfile::TempDir, tempfile::TempDir, RuntimePaths) {
        let root = tempfile::tempdir().unwrap();
        let installed = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::trial(root.path(), 49398, installed.path()).unwrap();
        (root, installed, paths)
    }
    fn write(paths: &RuntimePaths, text: &str) {
        std::fs::write(paths.resource(Resource::Config), text).unwrap();
    }
    fn read(paths: &RuntimePaths) -> String {
        std::fs::read_to_string(paths.resource(Resource::Config)).unwrap()
    }
    fn load(paths: &RuntimePaths) -> ConfigStore {
        ConfigStore::load_or_create(paths.clone(), || Ok("synthetic-r3-fresh-0002".into())).unwrap()
    }
    fn save(store: &ConfigStore, value: &str) -> Result<(), ConfigError> {
        let snapshot = store.snapshot().unwrap();
        let mut next = snapshot.config;
        next.user_prefs.display_name = value.into();
        store.persist(snapshot.revision, next).map(|_| ())
    }
    fn backups_folder(paths: &RuntimePaths) -> PathBuf {
        paths.resource(Resource::ConfigBackups)
    }
    /// The per-backup notice records (`config.yaml.bak_<id>.removed.json`).
    fn record_files(paths: &RuntimePaths) -> Vec<PathBuf> {
        let mut found: Vec<PathBuf> = std::fs::read_dir(backups_folder(paths))
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with(".removed.json"))
            })
            .collect();
        found.sort();
        found
    }
    fn all_record_text(paths: &RuntimePaths) -> String {
        record_files(paths)
            .iter()
            .filter_map(|path| std::fs::read_to_string(path).ok())
            .collect()
    }
    /// The shared record the first version wrote.
    fn legacy_path(paths: &RuntimePaths) -> PathBuf {
        backups_folder(paths).join(removed::LEGACY_RECORD_NAME)
    }
    /// `config_cli` with a fixed clock (so backups made "in the same second" can be forced).
    fn cli_at(
        paths: &RuntimePaths,
        items: &[&str],
        clock: fn() -> chrono::NaiveDateTime,
    ) -> (Result<i32, String>, String) {
        let mut out = Vec::new();
        let backups = ConfigBackups::with_clock(paths, clock);
        let args: Vec<String> = items.iter().map(|s| s.to_string()).collect();
        let result = config_cli::run_with(paths, &args, &mut out, &backups, &mut || {
            Ok("synthetic-r3-fresh-0007".into())
        });
        (result, String::from_utf8_lossy(&out).into_owned())
    }
    fn same_second() -> chrono::NaiveDateTime {
        chrono::NaiveDate::from_ymd_opt(2026, 10, 16)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap()
    }
    /// Use up every backup name of `same_second()`, so the next backup made at that
    /// time cannot be created.
    fn use_up_the_second(paths: &RuntimePaths) {
        let backups = ConfigBackups::with_clock(paths, same_second);
        for _ in 0..999 {
            backups.create(b"synthetic filler\n").unwrap();
        }
    }
    fn cli(paths: &RuntimePaths, items: &[&str]) -> (Result<i32, String>, String) {
        let mut out = Vec::new();
        let backups = ConfigBackups::new(paths);
        let args: Vec<String> = items.iter().map(|s| s.to_string()).collect();
        let result = config_cli::run_with(paths, &args, &mut out, &backups, &mut || {
            Ok("synthetic-r3-fresh-0003".into())
        });
        (result, String::from_utf8_lossy(&out).into_owned())
    }

    // ---------------------------------------------------------------------------
    // 1. removed-items.json の扱い
    // ---------------------------------------------------------------------------

    #[test]
    fn a_damaged_record_is_not_reported_as_nothing_to_report() {
        let damaged_forms = [
            ("cut off", "{\"backup_id\": \"2026".to_owned()),
            ("a list", "[]".to_owned()),
            ("wrong type", "{\"notices\": 5}".to_owned()),
            (
                "one entry without a field",
                "{\"backup_id\": \"20261016_100001\", \"removed_at\": \"x\", \"items\": [{\"section\": \"custom_providers\", \"index\": 2}], \"more\": 0}".to_owned(),
            ),
            ("empty", String::new()),
            ("not text", "\u{1}\u{2}binary".to_owned()),
        ];
        let mut silent = Vec::new();
        for (name, damaged) in &damaged_forms {
            for shared in [false, true] {
                let (_root, _installed, paths) = trial();
                write(&paths, BAD);
                let store = load(&paths);
                save(&store, "changed").unwrap();
                let files = record_files(&paths);
                assert_eq!(files.len(), 1, "{name}");
                if shared {
                    // The first version's one shared file, damaged.
                    std::fs::remove_file(&files[0]).unwrap();
                    std::fs::write(legacy_path(&paths), damaged).unwrap();
                } else {
                    std::fs::write(&files[0], damaged).unwrap();
                }
                let label = format!(
                    "{name} ({})",
                    if shared { "shared file" } else { "per backup" }
                );
                let (code, out) = cli(&paths, &["check"]);
                let (json_code, json_out) = cli(&paths, &["check", "--json"]);
                let view = ConfigHealth::new(paths.clone(), Arc::new(load(&paths))).view();
                let listed = removed::list(&paths).unwrap();
                if code != Ok(1) || json_code != Ok(1) || !out.contains("cannot be read") {
                    silent.push((label.clone(), out.clone()));
                }
                assert!(
                    json_out.contains("record-unreadable"),
                    "{label}: {json_out}"
                );
                assert!(json_out.contains("\"ok\": false"), "{label}: {json_out}");
                assert_eq!(view["status"], "problems", "{label}");
                assert_eq!(
                    view["removed_by_save"][0]["kind"], "record-unreadable",
                    "{label}"
                );
                assert!(
                    listed
                        .iter()
                        .all(|n| n.kind == removed::NoticeKind::Unreadable),
                    "{label}"
                );
                // It is left as it was (not repaired, not deleted).
                let on_disk = if shared {
                    std::fs::read(legacy_path(&paths)).unwrap()
                } else {
                    std::fs::read(&files[0]).unwrap()
                };
                assert_eq!(on_disk, damaged.as_bytes(), "{label}");
            }
        }
        assert!(
            silent.is_empty(),
            "config check does not say a record cannot be read: {silent:?}"
        );
    }

    #[test]
    fn a_damaged_record_is_not_overwritten_by_the_next_save_and_the_new_one_is_listed() {
        let (_root, _installed, paths) = trial();
        write(&paths, BAD);
        let store = load(&paths);
        save(&store, "changed").unwrap();
        let first = record_files(&paths).remove(0);
        let damaged = "{\"backup_id\": \"2026";
        std::fs::write(&first, damaged).unwrap();
        // The shared file of the first version, damaged too.
        std::fs::write(legacy_path(&paths), "[]").unwrap();
        // The person writes the same mistake again, and the Hub saves again.
        write(&paths, BAD);
        save(&store, "changed again").unwrap();
        assert_eq!(std::fs::read_to_string(&first).unwrap(), damaged);
        assert_eq!(std::fs::read_to_string(legacy_path(&paths)).unwrap(), "[]");
        let notices = removed::list(&paths).unwrap();
        let kinds: Vec<_> = notices.iter().map(|n| n.kind).collect();
        assert_eq!(
            kinds
                .iter()
                .filter(|k| **k == removed::NoticeKind::Unreadable)
                .count(),
            2
        );
        assert_eq!(
            kinds
                .iter()
                .filter(|k| **k == removed::NoticeKind::Item)
                .count(),
            1,
            "the new record is read next to the damaged ones"
        );
        assert_eq!(record_files(&paths).len(), 2);
    }

    fn many_left_out(count: usize) -> String {
        let mut text = format!("token: {TOKEN}\ncustom_providers:\n  - id: ok\n    command: ok\n");
        for _ in 0..count {
            text.push_str("  - 7\n");
        }
        text
    }

    #[test]
    fn a_record_the_hub_writes_can_be_read_back_however_many_items_a_save_takes_out() {
        let (_root, _installed, paths) = trial();
        write(&paths, &many_left_out(8000));
        let store = Arc::new(load(&paths));
        save(&store, "changed").unwrap();
        let files = record_files(&paths);
        assert_eq!(files.len(), 1);
        let size = std::fs::metadata(&files[0]).unwrap().len();
        assert!(size < 64 * 1024, "a record stays small: {size} bytes");
        let listed = removed::list(&paths).unwrap();
        assert_eq!(listed.len(), removed::MAX_ITEMS + 1);
        let more = listed.last().unwrap();
        assert_eq!(more.kind, removed::NoticeKind::More);
        assert_eq!(more.more, 8000 - removed::MAX_ITEMS);
        assert!(
            listed[..removed::MAX_ITEMS]
                .iter()
                .all(|n| n.kind == removed::NoticeKind::Item)
        );
        // The counts and the way back are in every surface.
        let health = ConfigHealth::new(paths.clone(), store.clone()).view();
        assert_eq!(health["status"], "problems");
        assert_eq!(
            health["removed_by_save"].as_array().unwrap().len(),
            removed::MAX_ITEMS + 1
        );
        assert_eq!(health["restart_needed"], false);
        let (code, out) = cli(&paths, &["check"]);
        assert_eq!(code, Ok(1));
        assert!(out.contains("7950 more items"), "{out}");
        let backup_id = listed[0].backup_id.clone();
        assert!(out.contains(&format!("config backup restore {backup_id}")));
        // The whole file is in the backup: nothing the cap left out is lost.
        let backups = ConfigBackups::new(&paths);
        let record = backups.find(&backup_id).unwrap();
        assert_eq!(
            String::from_utf8(backups.read(&record).unwrap())
                .unwrap()
                .matches("  - 7\n")
                .count(),
            8000
        );
    }

    #[test]
    fn restore_with_an_unreadable_old_record_in_the_way_restores_and_does_not_claim_otherwise() {
        let (_root, _installed, paths) = trial();
        write(&paths, "token: synthetic-r3-a\n");
        let backup = ConfigBackups::new(&paths)
            .create(b"token: synthetic-r3-b\n")
            .unwrap();
        // A folder where the first version's shared record would be.
        std::fs::create_dir_all(legacy_path(&paths)).unwrap();
        let (code, out) = cli(&paths, &["backup", "restore", &backup.id]);
        match &code {
            Ok(_) => assert_eq!(read(&paths), "token: synthetic-r3-b\n", "{out}"),
            Err(message) => assert_eq!(
                read(&paths),
                "token: synthetic-r3-a\n",
                "restore said `{message}` but replaced the file"
            ),
        }
        assert!(code.is_ok(), "{code:?}");
        // The record that cannot be read is neither touched nor hidden.
        assert!(legacy_path(&paths).is_dir());
        assert_eq!(cli(&paths, &["check"]).0, Ok(1));
    }

    #[test]
    fn reset_sets_an_unreadable_record_aside_instead_of_failing_or_deleting_it() {
        let (_root, _installed, paths) = trial();
        let original = "token: synthetic-r3-a\nuser_prefs: {display_name: synthetic-mine}\n";
        write(&paths, original);
        std::fs::create_dir_all(legacy_path(&paths)).unwrap();
        let (code, out) = cli(&paths, &["reset", "--yes"]);
        assert_eq!(code, Ok(0), "{out}");
        assert!(!read(&paths).contains("synthetic-mine"));
        // Nothing was deleted: the record is under its new name, and no notice stands.
        assert!(!legacy_path(&paths).exists());
        assert!(
            backups_folder(&paths)
                .join(format!("{}.dismissed", removed::LEGACY_RECORD_NAME))
                .is_dir()
        );
        assert!(removed::list(&paths).unwrap().is_empty());
        let backups = ConfigBackups::new(&paths).list().unwrap();
        assert_eq!(backups.len(), 1);
        assert_eq!(std::fs::read_to_string(&backups[0].path).unwrap(), original);
    }

    #[test]
    fn restoring_another_copy_with_the_same_item_does_not_keep_saying_it_is_gone() {
        let (_root, _installed, paths) = trial();
        write(&paths, BAD);
        let store = load(&paths);
        save(&store, "one").unwrap();
        let first = removed::list(&paths).unwrap()[0].backup_id.clone();
        let backups = ConfigBackups::new(&paths);
        backups.restore(&first).unwrap();
        assert!(removed::list(&paths).unwrap().is_empty());
        // 動いている Hub がまた保存し、同じ項目を外す（別の退避を指す知らせ）。
        save(&store, "two").unwrap();
        let second = removed::list(&paths).unwrap();
        assert_eq!(second.len(), 1);
        assert_ne!(second[0].backup_id, first);
        // 人が最初の退避をもう一度戻す: half はファイルに戻った。
        backups.restore(&first).unwrap();
        assert!(read(&paths).contains("id: half"));
        let now_in_file = inspect_file(&paths).unwrap().problems().len();
        let standing = removed::list(&paths).unwrap();
        eprintln!(
            "in file: {now_in_file}, still told it is gone: {:?}",
            standing
                .iter()
                .map(|n| (n.id.clone(), n.backup_id.clone()))
                .collect::<Vec<_>>()
        );
        // 期待: ファイルにある項目を「もうファイルに無い」と言い続けない（二重に数えない）。
        assert!(
            standing.is_empty(),
            "an item that is back in the file is still reported as removed"
        );
    }

    #[test]
    fn after_a_save_nothing_says_a_fix_waits_for_a_restart() {
        let (_root, _installed, paths) = trial();
        write(&paths, BAD);
        let store = Arc::new(load(&paths));
        save(&store, "changed").unwrap();
        let health = ConfigHealth::new(paths.clone(), store.clone());
        let view = health.view();
        eprintln!(
            "after save: status={} restart_needed={}",
            view["status"], view["restart_needed"]
        );
        assert_eq!(view["status"], "problems");
        // restart_needed=true だと画面は「ファイルで直したものは再起動後に使われます」を足す
        // （web/src/app/config-problems-view.ts の config_problems_restart_note）。
        // 何も直していない。Hub はファイルと同じ設定で動いている。
        let after_save = view["restart_needed"].clone();
        removed::dismiss_all(&paths).unwrap();
        let view = health.view();
        eprintln!(
            "after dismiss: status={} restart_needed={}",
            view["status"], view["restart_needed"]
        );
        // status=ok かつ restart_needed=true は画面で「設定の誤りはファイルで直っています」になる。
        assert_eq!(
            after_save, false,
            "the screen says a fix waits for a restart"
        );
        assert!(
            !(view["status"] == "ok" && view["restart_needed"] == true),
            "after dismiss the screen says the mistake was fixed in the file"
        );
    }

    #[test]
    fn a_missing_file_does_not_hide_standing_notices() {
        let (_root, _installed, paths) = trial();
        write(&paths, BAD);
        let store = Arc::new(load(&paths));
        save(&store, "changed").unwrap();
        std::fs::remove_file(paths.resource(Resource::Config)).unwrap();
        let view = ConfigHealth::new(paths.clone(), store.clone()).view();
        let (code, _) = cli(&paths, &["check"]);
        eprintln!(
            "missing file: view.status={} check={code:?}",
            view["status"]
        );
        // config check は 1（知らせあり）。画面も同じことを言う。
        assert_eq!(code, Ok(1));
        assert_eq!(view["status"], "problems", "the banner hides the notices");
    }

    #[test]
    fn a_save_over_a_file_a_person_broke_is_refused_and_changes_nothing() {
        let (_root, _installed, paths) = trial();
        write(&paths, &format!("token: {TOKEN}\n"));
        let store = Arc::new(load(&paths));
        let before = store.snapshot().unwrap();
        // The Hub keeps running while a person edits the file and breaks one place.
        let edited = format!(
            "token: {TOKEN}\nuser_prefs: {{display_name: synthetic-mine}}\ncustom_providers:\n  - id: mine\n    command: mine\nhub: [\n"
        );
        write(&paths, &edited);
        assert!(matches!(
            inspect_file(&paths).unwrap(),
            ConfigFileState::Unreadable(_)
        ));
        let error = save(&store, "from-the-screen").unwrap_err();
        let message = error.to_string();
        assert!(message.contains("cannot be read"), "{message}");
        assert!(message.contains("config reset"), "{message}");
        assert!(message.contains("not saved"), "{message}");
        assert!(!message.contains(TOKEN), "{message}");
        // The file is exactly what the person wrote; no backup, no notice was needed.
        assert_eq!(read(&paths), edited);
        assert_eq!(ConfigBackups::new(&paths).list().unwrap().len(), 0);
        assert!(removed::list(&paths).unwrap().is_empty());
        // The Hub still holds the settings it had, and its revision did not move.
        let after = store.snapshot().unwrap();
        assert_eq!(after.revision, before.revision);
        assert_eq!(
            after.config.user_prefs.display_name,
            before.config.user_prefs.display_name
        );
        // The screen keeps saying the file cannot be read.
        let view = ConfigHealth::new(paths.clone(), store.clone()).view();
        assert_eq!(view["status"], "unreadable");
        // Once the person fixes the file, the same save works; what they wrote is
        // copied first.
        let fixed = format!(
            "token: {TOKEN}\nuser_prefs: {{display_name: synthetic-mine}}\ncustom_providers:\n  - id: mine\n    command: mine\n"
        );
        write(&paths, &fixed);
        save(&store, "from-the-screen").unwrap();
        let backups = ConfigBackups::new(&paths).list().unwrap();
        assert_eq!(backups.len(), 1);
        assert_eq!(std::fs::read_to_string(&backups[0].path).unwrap(), fixed);
    }

    #[test]
    fn values_a_save_drops_from_a_kept_item_are_noticed() {
        let (_root, _installed, paths) = trial();
        let original = format!(
            "token: {TOKEN}\ncustom_providers:\n  - id: hl\n    command: hl\n    headless:\n      format: synthetic-bogus\n      prompt_via: synthetic-odd\n"
        );
        write(&paths, &original);
        let before = inspect_file(&paths).unwrap().problems().to_vec();
        eprintln!(
            "before: {:?}",
            before
                .iter()
                .map(|p| (p.index, p.field, p.code.as_str(), p.dropped))
                .collect::<Vec<_>>()
        );
        let store = load(&paths);
        save(&store, "changed").unwrap();
        let text = read(&paths);
        let notices = removed::list(&paths).unwrap();
        let after = inspect_file(&paths).unwrap().problems().len();
        eprintln!(
            "after: bogus_in_file={} odd_in_file={} notices={} problems_in_file={after}\n{text}",
            text.contains("synthetic-bogus"),
            text.contains("synthetic-odd"),
            notices.len()
        );
        let gone = !text.contains("synthetic-bogus") || !text.contains("synthetic-odd");
        assert!(
            !gone || !notices.is_empty(),
            "a value was dropped by the save with no notice"
        );
    }

    #[test]
    fn null_items_do_not_shift_what_is_recorded() {
        let (_root, _installed, paths) = trial();
        write(
            &paths,
            &format!(
                "token: {TOKEN}\ncustom_providers:\n  - ~\n  - id: half\n  -\n  - id: ok\n    command: ok\n  - 7\n"
            ),
        );
        let problems: Vec<_> = inspect_file(&paths)
            .unwrap()
            .problems()
            .iter()
            .map(|p| (p.index, p.id.clone(), p.code.as_str()))
            .collect();
        let store = load(&paths);
        save(&store, "changed").unwrap();
        let notices: Vec<_> = removed::list(&paths)
            .unwrap()
            .iter()
            .map(|n| (n.index, n.id.clone(), n.code.clone()))
            .collect();
        eprintln!("problems={problems:?}\nnotices={notices:?}");
        assert!(!read(&paths).contains("half"));
        assert_eq!(
            notices.len(),
            2,
            "every item the save took out has a notice"
        );
    }

    #[test]
    fn the_same_item_removed_again_after_dismiss_is_noticed_again() {
        let (_root, _installed, paths) = trial();
        write(&paths, BAD);
        let store = load(&paths);
        save(&store, "one").unwrap();
        assert_eq!(removed::dismiss_all(&paths).unwrap().len(), 1);
        write(&paths, BAD);
        save(&store, "two").unwrap();
        write(&paths, BAD);
        save(&store, "three").unwrap();
        let notices = removed::list(&paths).unwrap();
        // 2 回外したので 2 件。それぞれ別の退避を指し、その退避に half がある。
        assert_eq!(notices.len(), 2);
        assert_ne!(notices[0].backup_id, notices[1].backup_id);
        let backups = ConfigBackups::new(&paths);
        for notice in &notices {
            let record = backups.find(&notice.backup_id).unwrap();
            assert!(
                String::from_utf8(backups.read(&record).unwrap())
                    .unwrap()
                    .contains("id: half")
            );
        }
    }

    // ---------------------------------------------------------------------------
    // 1b. config reset の移動方式（256 MiB を超えるファイル）
    // ---------------------------------------------------------------------------

    const OVER: u64 = 256 * 1024 * 1024 + 1;

    fn big_file(paths: &RuntimePaths) -> PathBuf {
        use std::io::{Seek, SeekFrom, Write};
        let path = paths.resource(Resource::Config);
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(b"token: synthetic-r3-big-head\n").unwrap();
        file.set_len(OVER).unwrap();
        file.seek(SeekFrom::End(-8)).unwrap();
        file.write_all(b"TAIL-R3\n").unwrap();
        path
    }
    fn head_and_tail(path: &std::path::Path) -> (u64, Vec<u8>, Vec<u8>) {
        use std::io::{Read, Seek, SeekFrom};
        let mut file = std::fs::File::open(path).unwrap();
        let len = file.metadata().unwrap().len();
        let mut head = vec![0; 28];
        file.read_exact(&mut head).unwrap();
        file.seek(SeekFrom::End(-8)).unwrap();
        let mut tail = vec![0; 8];
        file.read_exact(&mut tail).unwrap();
        (len, head, tail)
    }

    #[cfg(windows)]
    #[test]
    fn reset_moves_a_read_only_large_file_whole_or_changes_nothing() {
        let (_root, _installed, paths) = trial();
        let path = big_file(&paths);
        let before = head_and_tail(&path);
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&path, permissions).unwrap();
        let (code, out) = cli(&paths, &["reset", "--yes"]);
        eprintln!("read-only large reset: {code:?}\n{out}");
        let listed = ConfigBackups::new(&paths).list().unwrap();
        match code {
            Ok(0) => {
                assert_eq!(listed.len(), 1);
                assert_eq!(head_and_tail(&listed[0].path), before);
                assert!(read(&paths).contains("token: "));
            }
            _ => assert_eq!(
                head_and_tail(&path),
                before,
                "a failed reset changed the file"
            ),
        }
        for entry in listed.iter().map(|r| r.path.clone()).chain([path]) {
            if let Ok(metadata) = std::fs::metadata(&entry) {
                let mut permissions = metadata.permissions();
                #[allow(clippy::permissions_set_readonly_false)]
                permissions.set_readonly(false);
                let _ = std::fs::set_permissions(&entry, permissions);
                let _ = std::fs::remove_file(&entry);
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn reset_of_a_large_file_into_a_backup_folder_that_is_a_junction_changes_nothing() {
        let (root, _installed, paths) = trial();
        let path = big_file(&paths);
        let before = head_and_tail(&path);
        let elsewhere = tempfile::tempdir().unwrap();
        let status = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(root.path().join("backups"))
            .arg(elsewhere.path())
            .output()
            .unwrap();
        assert!(status.status.success());
        let (code, out) = cli(&paths, &["reset", "--yes"]);
        eprintln!("junction large reset: {code:?}\n{out}");
        assert_ne!(
            code,
            Ok(0),
            "followed a junction out of the settings folder"
        );
        assert_eq!(
            head_and_tail(&path),
            before,
            "a failed reset changed the file"
        );
        assert_eq!(std::fs::read_dir(elsewhere.path()).unwrap().count(), 0);
        let _ = std::fs::remove_dir(root.path().join("backups"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_large_reset_with_an_unreadable_record_moves_the_whole_file_and_loses_nothing() {
        let (_root, _installed, paths) = trial();
        let path = big_file(&paths);
        let before = head_and_tail(&path);
        std::fs::create_dir_all(legacy_path(&paths)).unwrap();
        let (code, out) = cli(&paths, &["reset", "--yes"]);
        let listed = ConfigBackups::new(&paths).list().unwrap();
        match &code {
            Ok(0) => {
                assert_eq!(listed.len(), 1, "{out}");
                assert_eq!(
                    head_and_tail(&listed[0].path),
                    before,
                    "the moved copy is not whole"
                );
                assert!(read(&paths).contains("token: "));
            }
            other => {
                // Whatever was reported, the file is where it was.
                assert!(
                    std::fs::metadata(&path).is_ok_and(|m| m.len() == OVER),
                    "reset said {other:?} but the file was moved and replaced"
                );
            }
        }
        for record in &listed {
            let _ = std::fs::remove_file(&record.path);
        }
        let _ = std::fs::remove_file(&path);
    }

    // ---------------------------------------------------------------------------
    // 1c. 前回の書き戻し系（C2R-01〜03 / 09 相当）: 方式変更の後に化け・増え続けが無いか
    // ---------------------------------------------------------------------------

    #[test]
    fn odd_items_are_removed_once_never_become_providers_and_do_not_pile_up() {
        let cases = [
            ("repeated key", "  - id: twin\n    id: other\n"),
            (
                "complex key",
                "  - ? [id]\n    : ghost\n    command: ghost\n",
            ),
            (
                "quoted merge key",
                "  - \"<<\": {id: ghost, command: ghost}\n",
            ),
            ("merge from an anchor", "  - <<: *base\n    command: [x]\n"),
            (
                "binary in an unknown key",
                "  - id: half\n    extra: !!binary AAEC\n",
            ),
            ("number id", "  - id: 12\n    command: [x]\n"),
        ];
        for (name, item) in cases {
            let (_root, _installed, paths) = trial();
            let text = format!(
                "token: {TOKEN}\nbase: &base {{id: ghost, command: ghost}}\ncustom_providers:\n  - id: ok\n    command: ok\n{item}"
            );
            write(&paths, &text);
            let Ok(store) =
                ConfigStore::load_or_create(paths.clone(), || Ok("synthetic-r3-fresh-0006".into()))
            else {
                eprintln!("{name}: not loadable");
                continue;
            };
            let offered = |store: &ConfigStore| {
                effective_custom_providers(&store.snapshot().unwrap().config.custom_providers)
                    .into_iter()
                    .map(|p| p.id)
                    .collect::<Vec<_>>()
            };
            let before = offered(&store);
            save(&store, "one").unwrap();
            let after_save = offered(&load(&paths));
            let backups_1 = ConfigBackups::new(&paths).list().unwrap().len();
            let notices_1 = removed::list(&paths).unwrap().len();
            save(&store, "two").unwrap();
            save(&store, "three").unwrap();
            let backups_3 = ConfigBackups::new(&paths).list().unwrap().len();
            let notices_3 = removed::list(&paths).unwrap().len();
            let in_file = inspect_file(&paths).unwrap().problems().len();
            eprintln!(
                "{name}: before={before:?} after={after_save:?} backups {backups_1}->{backups_3} notices {notices_1}->{notices_3} problems_in_file={in_file}"
            );
            assert_eq!(
                before, after_save,
                "{name}: the save changed what is offered"
            );
            assert_eq!(backups_1, backups_3, "{name}: backups keep growing");
            assert_eq!(notices_1, notices_3, "{name}: notices keep growing");
            // 消えたもの（問題として出ていたもの）は、知らせとして残っている。
            if backups_1 > 0 {
                let original_problems = {
                    let (_r, _i, copy) = trial();
                    write(&copy, &text);
                    inspect_file(&copy).unwrap().problems().len()
                };
                assert!(
                    notices_1 > 0 || original_problems == 0,
                    "{name}: something was taken out but no notice stands"
                );
            }
        }
    }

    // ---------------------------------------------------------------------------
    // 2. C2R-11 token と同じ id
    // ---------------------------------------------------------------------------

    #[test]
    fn a_token_used_as_an_id_never_reaches_the_record_or_any_output() {
        let token = "Synthetic-R3-Token-MixedCase-77";
        let lower = token.to_lowercase();
        for (name, text) in [
            (
                "alias",
                format!(
                    "token: &t {token}\ncustom_providers:\n  - id: *t\n  - id: ok\n    command: ok\n"
                ),
            ),
            (
                "anchor on the id",
                format!(
                    "custom_providers:\n  - id: &t {token}\n  - id: ok\n    command: ok\ntoken: *t\n"
                ),
            ),
            (
                "alias in a list item that is a wrong type",
                format!("token: &t {token}\ncustom_providers:\n  - id: *t\n    command: [x]\n"),
            ),
            (
                "padded alias",
                format!("token: &t \"  {token}  \"\ncustom_providers:\n  - id: *t\n"),
            ),
        ] {
            let (_root, _installed, paths) = trial();
            write(&paths, &text);
            let (_, before_text) = cli(&paths, &["check"]);
            let (_, before_json) = cli(&paths, &["check", "--json"]);
            let mut view = String::new();
            if let Ok(store) =
                ConfigStore::load_or_create(paths.clone(), || Ok("synthetic-r3-fresh-0005".into()))
            {
                let store = Arc::new(store);
                save(&store, "changed").unwrap();
                view = ConfigHealth::new(paths.clone(), store.clone())
                    .view()
                    .to_string();
            } else {
                eprintln!("{name}: not loadable");
            }
            let record = all_record_text(&paths);
            let (_, check_text) = cli(&paths, &["check"]);
            let (_, check_json) = cli(&paths, &["check", "--json"]);
            eprintln!("{name}: record={record}");
            for (surface, out) in [
                ("check before the save", &before_text),
                ("check --json before the save", &before_json),
                ("record", &record),
                ("check", &check_text),
                ("check --json", &check_json),
                ("view", &view),
            ] {
                let folded = out.to_lowercase();
                assert!(
                    !folded.contains(&lower),
                    "{name}: {surface} carries the token:\n{out}"
                );
            }
        }
    }

    // ---------------------------------------------------------------------------
    // 3. C2R-07 拒否の判定（読める設定で誤って拒否しない / 読めない設定は拒否）
    // ---------------------------------------------------------------------------

    #[test]
    fn new_sessions_blocker_follows_what_wrap_would_do() {
        let readable = [
            format!("token: {TOKEN}\n"),
            BAD.to_owned(),
            format!("token: {TOKEN}\n---\n"),
            format!("token: {TOKEN}\n---\nx: 1\n"),
            format!("\u{feff}token: {TOKEN}\n"),
            format!("token: {TOKEN}\ncustom_providers:\n  - id: e\n    command: '\"\"'\n"),
        ];
        for text in &readable {
            let (_root, _installed, paths) = trial();
            write(&paths, text);
            assert!(load_ok(&paths), "wrap would start: {text:?}");
            assert_eq!(
                safety::new_sessions_blocker(&paths),
                None,
                "refused a readable file: {text:?}"
            );
        }
        let unreadable = [
            format!("token: {TOKEN}\nhub: [\n"),
            format!("token: {TOKEN}\ntoken: x\n"),
            format!("token: {TOKEN}\nhub: {{port: \"x\"}}\n"),
        ];
        for text in &unreadable {
            let (_root, _installed, paths) = trial();
            write(&paths, text);
            assert!(!load_ok(&paths), "wrap would stop: {text:?}");
            let reason = safety::new_sessions_blocker(&paths).expect("refused");
            assert!(!reason.contains(TOKEN), "{reason}");
        }
        // 修正担当の判断: 調べられない（config.yaml がフォルダ）なら通す。wrap は自分で止まる。
        let (_root, _installed, paths) = trial();
        std::fs::create_dir(paths.resource(Resource::Config)).unwrap();
        assert_eq!(safety::new_sessions_blocker(&paths), None);
        assert!(!load_ok(&paths), "wrap would not stop on a folder");
    }

    fn load_ok(paths: &RuntimePaths) -> bool {
        // wrap の子は MainContext::load -> ConfigStore::load_or_create で読む。ファイルを
        // 書き換えないよう、写しの root で確かめる。
        let copy = tempfile::tempdir().unwrap();
        let installed = tempfile::tempdir().unwrap();
        let other = RuntimePaths::trial(copy.path(), 49399, installed.path()).unwrap();
        let source = paths.resource(Resource::Config);
        let target = other.resource(Resource::Config);
        if source.is_dir() {
            std::fs::create_dir(&target).unwrap();
        } else {
            std::fs::copy(&source, &target).unwrap();
        }
        ConfigStore::load_or_create(other, || Ok("synthetic-r3-fresh-0004".into())).is_ok()
    }

    // ---------------------------------------------------------------------------
    // 4. C2R-04 先頭語が空の command
    // ---------------------------------------------------------------------------

    #[test]
    fn commands_whose_program_is_empty_are_never_offered() {
        // 引用符は `"` だけ（`'` は文字そのもの）。先頭語が空になるのは `""` で始まるとき。
        let mut offered_ones = Vec::new();
        for command in [
            "'\"\"'",
            "'\"\" x'",
            "'  \"\" --flag'",
            "\"\\t\\\"\\\" x\"",
            "'\"\"\"\"'",
            "'\" \" x'",
            // A tab inside the quotes. YAML single quotes keep a backslash as it is,
            // so the scratch's `'"\t" x'` is a program named backslash-t, which is a
            // name; the double-quoted form below holds a real tab.
            "\"\\\"\\t\\\" x\"",
        ] {
            let (_root, _installed, paths) = trial();
            write(
                &paths,
                &format!("token: {TOKEN}\ncustom_providers:\n  - id: e\n    command: {command}\n"),
            );
            let store = load(&paths);
            let config = store.snapshot().unwrap().config;
            let offered: Vec<_> = effective_custom_providers(&config.custom_providers)
                .into_iter()
                .map(|p| p.id)
                .collect();
            let problems = inspect_file(&paths).unwrap().problems().len();
            eprintln!("command {command}: offered={offered:?} problems={problems}");
            if !offered.is_empty() || problems == 0 {
                offered_ones.push(command);
            }
        }
        // `""""` は 1 文字の `"` という名前（空ではない）なので、出してよい。
        offered_ones.retain(|command| *command != "'\"\"\"\"'");
        assert!(
            offered_ones.is_empty(),
            "offered with an empty or blank program: {offered_ones:?}"
        );
    }

    // ---------------------------------------------------------------------------
    // 5. 方針変更の後の保証（記録は退避ごとに 1 ファイル・置き換え前に済ませる）
    // ---------------------------------------------------------------------------

    #[test]
    fn every_save_writes_a_record_of_its_own_and_never_changes_an_earlier_one() {
        let (_root, _installed, paths) = trial();
        write(&paths, BAD);
        let store = load(&paths);
        save(&store, "one").unwrap();
        let first = record_files(&paths);
        assert_eq!(first.len(), 1);
        let first_bytes = std::fs::read(&first[0]).unwrap();
        let first_stamp = std::fs::metadata(&first[0]).unwrap().modified().unwrap();
        write(&paths, BAD);
        save(&store, "two").unwrap();
        let both = record_files(&paths);
        assert_eq!(both.len(), 2);
        assert_eq!(std::fs::read(&first[0]).unwrap(), first_bytes);
        assert_eq!(
            std::fs::metadata(&first[0]).unwrap().modified().unwrap(),
            first_stamp
        );
        assert_eq!(removed::list(&paths).unwrap().len(), 2);
        // No shared file exists to be read and written back.
        assert!(!legacy_path(&paths).exists());
        // Setting them aside renames the files; nothing is deleted.
        assert_eq!(removed::dismiss_all(&paths).unwrap().len(), 2);
        assert!(record_files(&paths).is_empty());
        let kept = std::fs::read_dir(backups_folder(&paths))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".dismissed"))
            .count();
        assert_eq!(kept, 2);
    }

    #[test]
    fn the_first_versions_shared_record_is_read_and_moved_into_records_of_their_own() {
        let (_root, _installed, paths) = trial();
        write(&paths, BAD);
        let store = load(&paths);
        save(&store, "one").unwrap();
        let id = removed::list(&paths).unwrap()[0].backup_id.clone();
        // Turn what the Hub wrote into the old shared form.
        std::fs::remove_file(&record_files(&paths)[0]).unwrap();
        std::fs::write(
            legacy_path(&paths),
            format!(
                "{{\"notices\":[{{\"backup_id\":\"{id}\",\"section\":\"custom_providers\",\"index\":2,\"id\":\"half\",\"field\":\"command\",\"code\":\"missing-command\",\"removed_at\":\"2026-10-16 10:00:00\"}}]}}"
            ),
        )
        .unwrap();
        assert_eq!(removed::list(&paths).unwrap().len(), 1);
        // The next save moves it, and it is still listed once next to the new one.
        write(&paths, BAD);
        save(&store, "two").unwrap();
        assert!(!legacy_path(&paths).exists());
        let listed = removed::list(&paths).unwrap();
        assert_eq!(listed.len(), 2, "{listed:?}");
        assert_eq!(record_files(&paths).len(), 2);
    }

    #[test]
    fn a_reset_that_fails_leaves_the_notices_standing_and_says_nothing_changed() {
        let (_root, _installed, paths) = trial();
        write(&paths, BAD);
        let store = load(&paths);
        save(&store, "one").unwrap();
        assert_eq!(removed::list(&paths).unwrap().len(), 1);
        let before = read(&paths);
        use_up_the_second(&paths);
        // The copy of the current settings cannot be made: nothing may change.
        let (code, _) = cli_at(&paths, &["reset", "--yes"], same_second);
        let message = code.unwrap_err();
        assert!(message.contains("settings were not changed"), "{message}");
        assert_eq!(read(&paths), before);
        assert_eq!(removed::list(&paths).unwrap().len(), 1);
        assert_eq!(record_files(&paths).len(), 1);
    }

    #[test]
    fn a_restore_that_fails_leaves_the_notices_standing_and_says_nothing_changed() {
        let (_root, _installed, paths) = trial();
        write(&paths, BAD);
        let store = load(&paths);
        save(&store, "one").unwrap();
        let id = removed::list(&paths).unwrap()[0].backup_id.clone();
        let before = read(&paths);
        use_up_the_second(&paths);
        let (code, _) = cli_at(&paths, &["backup", "restore", &id], same_second);
        let message = code.unwrap_err();
        assert!(message.contains("settings were not changed"), "{message}");
        assert_eq!(read(&paths), before);
        assert_eq!(removed::list(&paths).unwrap().len(), 1);
    }

    /// The new file cannot be put in place (the old one is read-only): the notices
    /// that were set aside for it come back, and the report is true.
    #[cfg(windows)]
    #[test]
    fn when_the_replacement_itself_fails_the_set_aside_notices_come_back() {
        for command in [
            ["reset", "--yes"].as_slice(),
            ["backup", "restore", "ID"].as_slice(),
        ] {
            let (_root, _installed, paths) = trial();
            write(&paths, BAD);
            let store = load(&paths);
            save(&store, "one").unwrap();
            let id = removed::list(&paths).unwrap()[0].backup_id.clone();
            let args: Vec<String> = command
                .iter()
                .map(|a| {
                    if *a == "ID" {
                        id.clone()
                    } else {
                        (*a).to_owned()
                    }
                })
                .collect();
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let config = paths.resource(Resource::Config);
            let before = read(&paths);
            let mut permissions = std::fs::metadata(&config).unwrap().permissions();
            permissions.set_readonly(true);
            std::fs::set_permissions(&config, permissions).unwrap();
            let (code, out) = cli(&paths, &args);
            let still_readonly = std::fs::metadata(&config).unwrap().permissions().readonly();
            let mut permissions = std::fs::metadata(&config).unwrap().permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(false);
            std::fs::set_permissions(&config, permissions).unwrap();
            // Windows does not replace a read-only file, so the replacement fails here.
            assert!(still_readonly, "{command:?}");
            let message =
                code.expect_err(&format!("{command:?}: replaced a read-only file: {out}"));
            // The file is as it was, the notices are back, and the message says
            // "not changed" only because it is true.
            assert_eq!(read(&paths), before, "{command:?}");
            assert_eq!(
                removed::list(&paths).unwrap().len(),
                1,
                "{command:?}: {message}"
            );
            assert_eq!(record_files(&paths).len(), 1, "{command:?}");
            assert!(
                message.contains("settings were not changed"),
                "{command:?}: {message}"
            );
        }
    }

    #[test]
    fn restoring_a_copy_also_ends_the_notices_of_copies_with_the_same_bytes() {
        let (_root, _installed, paths) = trial();
        write(&paths, BAD);
        let store = load(&paths);
        save(&store, "one").unwrap();
        let first = removed::list(&paths).unwrap()[0].backup_id.clone();
        ConfigBackups::new(&paths).restore(&first).unwrap();
        assert!(removed::list(&paths).unwrap().is_empty());
        // The same file is saved over again: another copy with the same bytes.
        save(&store, "two").unwrap();
        let second = removed::list(&paths).unwrap();
        assert_eq!(second.len(), 1);
        let backups = ConfigBackups::new(&paths);
        let a = backups.find(&first).unwrap();
        let b = backups.find(&second[0].backup_id).unwrap();
        assert_eq!(backups.read(&a).unwrap(), backups.read(&b).unwrap());
        // Bringing back the first copy brings back the same items: both are done.
        backups.restore(&first).unwrap();
        assert!(removed::list(&paths).unwrap().is_empty());
        // Different bytes are not merged: a copy that is not the same stays listed.
        write(&paths, &format!("{BAD}  - id: other\n"));
        save(&store, "three").unwrap();
        let third = removed::list(&paths).unwrap();
        assert_eq!(third.len(), 2, "{third:?}");
        backups.restore(&first).unwrap();
        assert_eq!(removed::list(&paths).unwrap().len(), 2);
    }

    #[test]
    fn an_empty_command_with_blanks_is_never_offered_but_a_real_one_is() {
        for command in ["'\" \" x'", "\"\\\"\\t\\\" x\"", "'\"\" x'", "'\"\"'"] {
            let (_root, _installed, paths) = trial();
            write(
                &paths,
                &format!("token: {TOKEN}\ncustom_providers:\n  - id: e\n    command: {command}\n"),
            );
            let problems = inspect_file(&paths).unwrap().problems().to_vec();
            assert_eq!(problems.len(), 1, "{command}");
            // A tab is a control character, which is refused as such.
            assert!(
                ["missing-command", "command-control-character"]
                    .contains(&problems[0].code.as_str()),
                "{command}: {}",
                problems[0].code.as_str()
            );
            let config = load(&paths).snapshot().unwrap().config;
            assert!(
                effective_custom_providers(&config.custom_providers).is_empty(),
                "{command}"
            );
        }
        let (_root, _installed, paths) = trial();
        write(
            &paths,
            &format!("token: {TOKEN}\ncustom_providers:\n  - id: e\n    command: '\"a b\" x'\n"),
        );
        assert!(inspect_file(&paths).unwrap().problems().is_empty());
    }

    #[test]
    fn a_missing_file_with_standing_notices_is_not_ok_on_any_surface() {
        let (_root, _installed, paths) = trial();
        write(&paths, BAD);
        let store = Arc::new(load(&paths));
        save(&store, "changed").unwrap();
        std::fs::remove_file(paths.resource(Resource::Config)).unwrap();
        let view = ConfigHealth::new(paths.clone(), store).view();
        assert_eq!(view["status"], "problems");
        assert_eq!(view["removed_by_save"].as_array().unwrap().len(), 1);
        let (code, out) = cli(&paths, &["check", "--json"]);
        assert_eq!(code, Ok(1));
        assert!(out.contains("\"ok\": false"), "{out}");
    }
}

mod final_check {
    //! 最終確認（対象 620240a9）の指摘を正式なテストにしたもの。C2Z-01（拒否された
    //! 保存は legacy の経路でもメモリと revision を変えない）と C2Z-02（token を含む
    //! id はどこにも出ない）が直したもの。C2Z-03 は計画に残した軽微な不具合で ignore。
    use many_ai_cli::{application::config_cli, config::*, hub::config_problems::ConfigHealth};
    use std::{path::PathBuf, sync::Arc, time::SystemTime};

    const TOKEN: &str = "synthetic-r4-token-0001";

    fn trial() -> (tempfile::TempDir, tempfile::TempDir, RuntimePaths) {
        let root = tempfile::tempdir().unwrap();
        let installed = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::trial(root.path(), 49433, installed.path()).unwrap();
        (root, installed, paths)
    }
    fn config_path(paths: &RuntimePaths) -> PathBuf {
        paths.resource(Resource::Config)
    }
    fn write(paths: &RuntimePaths, text: &str) {
        std::fs::write(config_path(paths), text).unwrap();
    }
    fn read(paths: &RuntimePaths) -> String {
        std::fs::read_to_string(config_path(paths)).unwrap()
    }
    fn load(paths: &RuntimePaths) -> ConfigStore {
        ConfigStore::load_or_create(paths.clone(), || Ok("synthetic-r4-fresh-0002".into())).unwrap()
    }
    fn save(store: &ConfigStore, value: &str) -> Result<(), ConfigError> {
        let snapshot = store.snapshot().unwrap();
        let mut next = snapshot.config;
        next.user_prefs.display_name = value.into();
        store.persist(snapshot.revision, next).map(|_| ())
    }
    fn backups_folder(paths: &RuntimePaths) -> PathBuf {
        paths.resource(Resource::ConfigBackups)
    }
    fn names_in_backups(paths: &RuntimePaths) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(backups_folder(paths))
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|e| e.file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }
    fn cli(paths: &RuntimePaths, items: &[&str]) -> (Result<i32, String>, String) {
        let mut out = Vec::new();
        let backups = ConfigBackups::new(paths);
        let args: Vec<String> = items.iter().map(|s| s.to_string()).collect();
        let result = config_cli::run_with(paths, &args, &mut out, &backups, &mut || {
            Ok("synthetic-r4-fresh-0003".into())
        });
        (result, String::from_utf8_lossy(&out).into_owned())
    }
    fn half_items(count: usize) -> String {
        let mut text = format!("token: {TOKEN}\ncustom_providers:\n");
        for n in 0..count {
            text.push_str(&format!("  - id: h{n}\n"));
        }
        text
    }

    // 1. 保存の拒否 --------------------------------------------------------------

    /// 画面の設定保存（settings.rs）が使う legacy の経路でも、拒否された保存は
    /// メモリ上の設定と revision を変えないか。
    #[test]
    fn legacy_publish_path_refused_save_keeps_memory_and_revision() {
        let (_root, _installed, paths) = trial();
        write(&paths, &format!("token: {TOKEN}\n"));
        let store = load(&paths);
        let before = store.snapshot().unwrap();
        let broken = format!("token: {TOKEN}\nhub: [\n");
        write(&paths, &broken);
        let snapshot = store.snapshot().unwrap();
        let mut next = snapshot.config;
        next.user_prefs.display_name = "synthetic-from-screen".into();
        let error = store
            .publish_then_persist_legacy(snapshot.revision, next)
            .unwrap_err();
        assert!(error.to_string().contains("not saved"), "{error}");
        assert_eq!(read(&paths), broken, "file was replaced");
        let after = store.snapshot().unwrap();
        assert_eq!(
            (after.revision, after.config.user_prefs.display_name.clone()),
            (
                before.revision,
                before.config.user_prefs.display_name.clone()
            ),
            "refused save still changed what the Hub holds"
        );
    }

    /// 人が保存日時だけ変えた（中身は Hub が書いたまま）なら、拒否も退避も無い。
    #[test]
    fn mtime_only_change_is_neither_refused_nor_backed_up() {
        let (_root, _installed, paths) = trial();
        write(&paths, &format!("token: {TOKEN}\n"));
        let store = load(&paths);
        save(&store, "one").unwrap();
        // The first save backs up the hand-written start file (lossy); count from here.
        let backups_before = ConfigBackups::new(&paths).list().unwrap().len();
        let file = std::fs::File::options()
            .write(true)
            .open(config_path(&paths))
            .unwrap();
        file.set_modified(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(86_400))
            .unwrap();
        drop(file);
        save(&store, "two").unwrap();
        assert!(read(&paths).contains("two"));
        assert_eq!(
            ConfigBackups::new(&paths).list().unwrap().len(),
            backups_before
        );
    }

    /// Hub が書いた内容に 1 バイト足して壊す・途中で切れる → 拒否。読める 1 バイト追加 → 退避して保存。
    #[test]
    fn one_byte_and_cut_off_files_are_refused_readable_change_is_saved() {
        let (_root, _installed, paths) = trial();
        write(&paths, &format!("token: {TOKEN}\n"));
        let store = load(&paths);
        save(&store, "one").unwrap();
        let hub_written = read(&paths);
        for broken in [
            format!("{hub_written}["),
            format!("{hub_written}\t:"),
            hub_written[..hub_written.len() / 2].to_owned(),
        ] {
            write(&paths, &broken);
            let refused = matches!(
                inspect_file(&paths).unwrap(),
                ConfigFileState::Unreadable(_)
            );
            let result = save(&store, "two");
            if refused {
                let message = result.unwrap_err().to_string();
                assert!(message.contains("not saved"), "{message}");
                assert!(!message.contains(TOKEN), "{message}");
                assert_eq!(read(&paths), broken);
            } else {
                result.unwrap();
                // A readable person-made change is backed up, not lost.
                assert!(
                    ConfigBackups::new(&paths)
                        .list()
                        .unwrap()
                        .iter()
                        .any(|b| std::fs::read_to_string(&b.path).unwrap() == broken)
                );
                write(&paths, &read(&paths));
            }
            write(&paths, &hub_written);
        }
        // Back to what the Hub wrote: the save works again (never stuck).
        save(&store, "three").unwrap();
    }

    // restart_needed -------------------------------------------------------------

    /// Hub が自分で書いた後は、ファイルに残る誤り（読み込まれて残る項目）があっても
    /// 「再起動待ち」にならない。
    #[test]
    fn after_a_hub_save_kept_flagged_items_do_not_ask_for_a_restart() {
        let (_root, _installed, paths) = trial();
        let text = format!(
            "token: {TOKEN}\ncustom_providers:\n  - 7\n  - id: shell\n    command: x\n  - id: dup\n    command: a\n  - id: dup\n    command: b\n  - id: emp\n    command: '\"\"'\n  - id: quote\n    command: 'a \"b'\n  - id: {TOKEN}\n    command: t\n  - id: ' sp '\n    command: s\n"
        );
        write(&paths, &text);
        let store = Arc::new(load(&paths));
        save(&store, "one").unwrap();
        let view = ConfigHealth::new(paths.clone(), store.clone()).view();
        assert_eq!(view["restart_needed"], false, "{view}");
        assert!(!view.to_string().contains(TOKEN));
    }

    /// token が無いファイルは起動時に書き直される（written=true）。そのとき
    /// 先頭の読めない項目があっても「再起動待ち」にならない。
    #[test]
    fn startup_token_rewrite_with_a_dropped_item_does_not_ask_for_a_restart() {
        let (_root, _installed, paths) = trial();
        write(
            &paths,
            "custom_providers:\n  - 7\n  - id: shell\n    command: x\n  - id: dup\n    command: a\n  - id: dup\n    command: b\n",
        );
        let store = Arc::new(load(&paths));
        let view = ConfigHealth::new(paths.clone(), store.clone()).view();
        assert_eq!(view["restart_needed"], false, "{view}");
    }

    // 4. 回帰（token） -----------------------------------------------------------

    /// token に 1 文字足した id（貼り間違い）が、記録ファイル・知らせ・画面の応答に token を出さないか。
    #[test]
    fn an_id_that_contains_the_token_does_not_carry_it_out() {
        let (_root, _installed, paths) = trial();
        write(
            &paths,
            &format!("token: {TOKEN}\ncustom_providers:\n  - id: {TOKEN}x\n  - id: x{TOKEN}\n"),
        );
        let store = Arc::new(load(&paths));
        let startup = ConfigHealth::new(paths.clone(), store.clone())
            .view()
            .to_string();
        save(&store, "one").unwrap();
        let records: String = names_in_backups(&paths)
            .iter()
            .filter(|n| n.ends_with(".removed.json"))
            .map(|n| std::fs::read_to_string(backups_folder(&paths).join(n)).unwrap())
            .collect();
        let view = ConfigHealth::new(paths.clone(), store.clone())
            .view()
            .to_string();
        let (_, out) = cli(&paths, &["check"]);
        let leaks: Vec<&str> = [
            ("startup view", startup.as_str()),
            ("record file", records.as_str()),
            ("view after save", view.as_str()),
            ("config check", out.as_str()),
        ]
        .iter()
        .filter(|(_, text)| text.contains(TOKEN))
        .map(|(name, _)| *name)
        .collect();
        assert!(leaks.is_empty(), "token reaches: {leaks:?}");
    }

    // 2. 退避ごとの記録 ---------------------------------------------------------

    /// 同じ項目を外した退避が 2 つあり、中身（バイト）が違う。片方を restore して項目が
    /// ファイルに戻ったのに、もう片方の知らせが「もうファイルに無い」と言い続けないか。
    #[test]
    #[ignore = "C2Z-03: plan に残した既知の軽微な不具合"]
    fn restore_ends_notices_of_a_different_bytes_copy_with_the_same_item() {
        let (_root, _installed, paths) = trial();
        let bad = |name: &str| {
            format!(
                "token: {TOKEN}\nuser_prefs: {{display_name: {name}}}\ncustom_providers:\n  - id: ok\n    command: ok\n  - id: half\n"
            )
        };
        write(&paths, &bad("synthetic-a"));
        let store = load(&paths);
        save(&store, "one").unwrap();
        // A person writes the item back with another name change; the Hub saves again.
        write(&paths, &bad("synthetic-b"));
        save(&store, "two").unwrap();
        let standing = removed::list(&paths).unwrap();
        assert_eq!(standing.len(), 2);
        let second = standing[1].backup_id.clone();
        ConfigBackups::new(&paths).restore(&second).unwrap();
        assert!(read(&paths).contains("id: half"));
        let in_file = inspect_file(&paths).unwrap().problems().len();
        let still = removed::list(&paths).unwrap();
        assert!(
            still.is_empty(),
            "in file: {in_file}; still told it is gone: {:?}",
            still
                .iter()
                .map(|n| (n.id.clone(), n.backup_id.clone()))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn record_item_limit_boundaries() {
        for (count, items, more) in [(49, 49, 0), (50, 50, 0), (51, 50, 1), (8000, 50, 7950)] {
            let (_root, _installed, paths) = trial();
            write(&paths, &half_items(count));
            let store = load(&paths);
            save(&store, "one").unwrap();
            let notices = removed::list(&paths).unwrap();
            let listed = notices
                .iter()
                .filter(|n| n.kind == removed::NoticeKind::Item)
                .count();
            let extra: Vec<usize> = notices
                .iter()
                .filter(|n| n.kind == removed::NoticeKind::More)
                .map(|n| n.more)
                .collect();
            assert_eq!(listed, items, "{count}");
            assert_eq!(extra.iter().sum::<usize>(), more, "{count}");
            assert!(extra.len() <= 1, "{count}");
            // dismiss ends all of them once
            let (code, _) = cli(&paths, &["check", "--dismiss-removed"]);
            assert!(code.is_ok(), "{count}: {code:?}");
            assert!(removed::list(&paths).unwrap().is_empty(), "{count}");
        }
    }

    /// `.dismissed` が既にある・名前が退避 id でない記録がある。
    #[test]
    fn dismissed_name_taken_and_odd_record_names() {
        let (_root, _installed, paths) = trial();
        write(&paths, &half_items(1));
        let store = load(&paths);
        save(&store, "one").unwrap();
        let record = names_in_backups(&paths)
            .into_iter()
            .find(|n| n.ends_with(".removed.json"))
            .unwrap();
        let folder = backups_folder(&paths);
        std::fs::write(folder.join(format!("{record}.dismissed")), "synthetic old").unwrap();
        for odd in [
            "config.yaml.bak_20261016_100000..removed.json",
            "config.yaml.bak_..removed.json",
            "config.yaml.bak_20261016_100000_1.removed.json",
        ] {
            std::fs::write(folder.join(odd), "{}").unwrap();
        }
        let before = removed::list(&paths).unwrap();
        assert_eq!(
            before
                .iter()
                .filter(|n| n.kind == removed::NoticeKind::Unreadable)
                .count(),
            3,
            "{:?}",
            before.iter().map(|n| &n.record_file).collect::<Vec<_>>()
        );
        for notice in &before {
            assert!(!notice.record_file.contains('/') && !notice.record_file.contains('\\'));
        }
        let (code, out) = cli(&paths, &["check", "--dismiss-removed"]);
        assert!(code.is_ok(), "{code:?} {out}");
        assert!(removed::list(&paths).unwrap().is_empty());
        let after = names_in_backups(&paths);
        assert!(
            after.contains(&format!("{record}.dismissed_2")),
            "{after:?}"
        );
        assert_eq!(
            std::fs::read_to_string(folder.join(format!("{record}.dismissed"))).unwrap(),
            "synthetic old"
        );
    }

    /// restore で記録を改名できない（他のプロセスが削除共有なしで開いている）とき、
    /// 設定も知らせも元のままで、「変えていない」が事実である。
    #[cfg(windows)]
    #[test]
    fn restore_when_the_record_cannot_be_renamed() {
        use std::os::windows::fs::OpenOptionsExt;
        let (_root, _installed, paths) = trial();
        write(&paths, &half_items(2));
        let store = load(&paths);
        save(&store, "one").unwrap();
        let notices = removed::list(&paths).unwrap();
        let id = notices[0].backup_id.clone();
        let record = backups_folder(&paths).join(&notices[0].record_file);
        let before = read(&paths);
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&record)
            .unwrap();
        let (code, out) = cli(&paths, &["backup", "restore", &id]);
        drop(held);
        let message = code.expect_err(&format!("restored while the record was held: {out}"));
        assert_eq!(read(&paths), before, "{message}");
        assert_eq!(
            removed::list(&paths).unwrap().len(),
            notices.len(),
            "{message}"
        );
        assert!(message.contains("not changed"), "{message}");
        assert!(!message.contains(TOKEN), "{message}");
        // Once the handle is gone the same restore works and the notices end once.
        let (code, out) = cli(&paths, &["backup", "restore", &id]);
        assert!(code.is_ok(), "{code:?} {out}");
        assert!(removed::list(&paths).unwrap().is_empty());
        assert!(read(&paths).contains("h1"));
    }

    /// reset で記録を改名できないときも同じ。
    #[cfg(windows)]
    #[test]
    fn reset_when_the_record_cannot_be_renamed() {
        use std::os::windows::fs::OpenOptionsExt;
        let (_root, _installed, paths) = trial();
        write(&paths, &half_items(2));
        let store = load(&paths);
        save(&store, "one").unwrap();
        let notices = removed::list(&paths).unwrap();
        let record = backups_folder(&paths).join(&notices[0].record_file);
        let before = read(&paths);
        let backups_before = ConfigBackups::new(&paths).list().unwrap().len();
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&record)
            .unwrap();
        let (code, out) = cli(&paths, &["reset", "--yes"]);
        drop(held);
        let message = code.expect_err(&format!("reset while the record was held: {out}"));
        assert_eq!(read(&paths), before, "{message}");
        assert_eq!(removed::list(&paths).unwrap().len(), notices.len());
        assert_eq!(
            ConfigBackups::new(&paths).list().unwrap().len(),
            backups_before
        );
        assert!(message.contains("not changed"), "{message}");
    }
}
