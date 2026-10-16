//! A settings file that cannot be read stops the program instead of being
//! replaced; a wrong `custom_providers` entry is left out on its own; and no
//! command prints the Hub token or a line of the file. Everything runs on
//! synthetic data inside temporary directories. The actual binary is started
//! only with an explicit trial root and a temporary home.
use many_ai_cli::{
    config::*,
    process::{Cancellation, ExitOutcome, ProcessOutput, ProcessPlan, run_capped},
};
use std::{path::Path, time::Duration};

const SECRET: &str = "synthetic-private-marker-9137";

fn trial() -> (tempfile::TempDir, tempfile::TempDir, RuntimePaths) {
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49391, installed.path()).unwrap();
    (root, installed, paths)
}

async fn run_binary(root: &Path, home: &Path, args: &[&str]) -> ProcessOutput {
    let mut full: Vec<std::ffi::OsString> = vec![
        "--trial-root".into(),
        root.into(),
        "--trial-port".into(),
        "49391".into(),
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

#[test]
fn a_wrong_entry_is_left_out_alone_and_the_rest_keeps_working() {
    let (_root, _installed, paths) = trial();
    let file = paths.resource(Resource::Config);
    let original = format!(
        "token: {SECRET}\ncustom_providers:\n  - id: shell\n    command: shell-cli\n  - id: muse\n    command: muse\n  - id: muse\n    command: other\n"
    );
    std::fs::write(&file, &original).unwrap();
    let store =
        ConfigStore::load_or_create(paths.clone(), || panic!("token must be kept")).unwrap();
    let config = store.snapshot().unwrap().config;
    // The Hub starts; only the good entry can be launched.
    assert_eq!(config.token, SECRET);
    assert_eq!(
        effective_custom_providers(&config.custom_providers)
            .into_iter()
            .map(|p| p.id)
            .collect::<Vec<_>>(),
        ["muse"]
    );
    assert!(config.is_custom_provider_id("muse"));
    assert!(!config.is_custom_provider_id("shell"));
    // The file is exactly as the user wrote it.
    assert_eq!(std::fs::read_to_string(&file).unwrap(), original);
    // Each mistake names the entry, id, field and reason.
    let problems = store.startup_problems();
    assert_eq!(problems.len(), 2);
    assert_eq!(
        (
            problems[0].index,
            problems[0].id.as_deref(),
            problems[0].field
        ),
        (Some(1), Some("shell"), "id")
    );
    assert_eq!(problems[0].code, ProblemCode::ReservedId);
    assert_eq!(problems[1].code, ProblemCode::DuplicateId);
    assert_eq!(problems[1].related_index, Some(2));
    // The same findings come out of reading the file fresh.
    let state = inspect_file(&paths).unwrap();
    assert_eq!(state.problems(), problems);
    // Fix the file: the list empties without restarting anything.
    std::fs::write(
        &file,
        format!("token: {SECRET}\ncustom_providers:\n  - id: muse\n    command: muse\n"),
    )
    .unwrap();
    assert!(inspect_file(&paths).unwrap().is_ok());
    for problem in problems {
        for rendered in [
            problem.render_en(),
            problem.warning_en(),
            problem.to_json().to_string(),
        ] {
            assert!(!rendered.contains(SECRET));
            assert!(!rendered.contains("shell-cli"));
        }
    }
}

fn backup_files(paths: &RuntimePaths) -> Vec<std::path::PathBuf> {
    ConfigBackups::new(paths)
        .list()
        .unwrap()
        .into_iter()
        .map(|record| record.path)
        .collect()
}
fn save(store: &ConfigStore, edit: impl FnOnce(&mut Config)) {
    let snapshot = store.snapshot().unwrap();
    let mut next = snapshot.config;
    edit(&mut next);
    store.persist(snapshot.revision, next).unwrap();
}

#[test]
fn saving_copies_to_backups_what_it_would_destroy_and_nothing_else() {
    let (_root, _installed, paths) = trial();
    let file = paths.resource(Resource::Config);
    // A hand-written file: a comment, and an entry that cannot be kept.
    let original = "token: synthetic-t\n# my own note\nuser_prefs:\n  display_name: hand-written\ncustom_providers:\n  - id: half-written\n";
    std::fs::write(&file, original).unwrap();
    let store = ConfigStore::load_or_create(paths.clone(), || panic!("kept")).unwrap();
    let logged = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let sink = logged.clone();
    store.set_backup_observer(std::sync::Arc::new(move |backup| {
        sink.lock().unwrap().push(backup.file_name.clone())
    }));
    assert!(backup_files(&paths).is_empty());

    // First save: the comment, the layout and the half-written entry would be lost.
    save(&store, |c| c.user_prefs.display_name = "first".into());
    let backups = backup_files(&paths);
    assert_eq!(backups.len(), 1);
    assert_eq!(std::fs::read_to_string(&backups[0]).unwrap(), original);
    assert!(
        !std::fs::read_to_string(&file)
            .unwrap()
            .contains("my own note")
    );

    // More saves with nobody touching the file: no more backups.
    save(&store, |c| c.user_prefs.display_name = "second".into());
    save(&store, |c| c.user_prefs.display_name = "third".into());
    assert_eq!(backup_files(&paths).len(), 1);

    // The user edits the file by hand while the Hub runs: saving keeps that edit.
    let hand_edit = "token: synthetic-t\nuser_prefs: {display_name: edited-by-hand}\n";
    std::fs::write(&file, hand_edit).unwrap();
    save(&store, |c| c.user_prefs.display_name = "fourth".into());
    let backups = backup_files(&paths);
    assert_eq!(backups.len(), 2);
    assert!(
        backups
            .iter()
            .any(|p| std::fs::read_to_string(p).unwrap() == hand_edit)
    );

    // An edit that leaves the file unreadable is not replaced at all: the save is
    // refused (the Hub keeps its settings and keeps running), no copy is needed
    // because nothing is overwritten, and the file stays byte for byte.
    let broken = format!("token: {SECRET}\nhub: [broken");
    std::fs::write(&file, &broken).unwrap();
    let snapshot = store.snapshot().unwrap();
    let mut next = snapshot.config.clone();
    next.user_prefs.display_name = "fifth".into();
    let refused = store
        .persist(snapshot.revision, next)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("not saved") && refused.contains("config reset"));
    assert!(!refused.contains(SECRET));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), broken);
    assert_eq!(backup_files(&paths).len(), 2);
    assert_eq!(
        store.snapshot().unwrap().config.user_prefs.display_name,
        snapshot.config.user_prefs.display_name
    );
    // Once the person has fixed it, saving works again and keeps what they wrote.
    let fixed = format!("token: {SECRET}\nuser_prefs: {{display_name: fixed-by-hand}}\n");
    std::fs::write(&file, &fixed).unwrap();
    save(&store, |c| c.user_prefs.display_name = "sixth".into());
    let backups = backup_files(&paths);
    assert_eq!(backups.len(), 3);
    assert!(
        backups
            .iter()
            .any(|p| std::fs::read_to_string(p).unwrap() == fixed)
    );
    assert!(Config::from_yaml(&std::fs::read_to_string(&file).unwrap(), &paths).is_ok());

    // One log entry per backup, naming the copy and nothing from inside it.
    let logged = logged.lock().unwrap();
    assert_eq!(logged.len(), 3);
    assert!(
        logged
            .iter()
            .all(|name| name.starts_with("config.yaml.bak_") && !name.contains(SECRET))
    );
}

#[test]
fn a_file_the_hub_wrote_is_saved_again_and_again_without_backups() {
    let (_root, _installed, paths) = trial();
    let store = ConfigStore::load_or_create(paths.clone(), || Ok("synthetic-t".into())).unwrap();
    save(&store, |c| c.user_prefs.display_name = "one".into());
    save(&store, |c| c.user_prefs.display_name = "two".into());
    // Saving the very same content again changes nothing.
    save(&store, |_| {});
    assert!(backup_files(&paths).is_empty());
    // A restart reads the Hub's own file back: saving it loses nothing either.
    let again = ConfigStore::load_or_create(paths.clone(), || panic!("kept")).unwrap();
    save(&again, |c| c.user_prefs.display_name = "three".into());
    // A store continued from another (an in-memory override) knows the file too.
    let derived = again.derive(again.snapshot().unwrap().config).unwrap();
    save(&derived, |c| c.user_prefs.display_name = "four".into());
    assert!(backup_files(&paths).is_empty());
}

#[test]
fn several_backups_in_the_same_second_all_stay() {
    let (_root, _installed, paths) = trial();
    let file = paths.resource(Resource::Config);
    std::fs::write(&file, "token: synthetic-t\n").unwrap();
    let store = ConfigStore::load_or_create(paths.clone(), || panic!("kept")).unwrap();
    // Hand edits followed by saves, as fast as they can run.
    for edit in ["one", "two", "three"] {
        std::fs::write(
            &file,
            format!("token: synthetic-t\nuser_prefs: {{display_name: {edit}}}\n"),
        )
        .unwrap();
        save(&store, |c| c.user_prefs.display_name = "saved".into());
    }
    let backups = backup_files(&paths);
    assert_eq!(backups.len(), 3);
    let mut contents: Vec<_> = backups
        .iter()
        .map(|p| std::fs::read_to_string(p).unwrap())
        .collect();
    contents.sort();
    assert!(contents[0].contains("one"));
    assert!(contents[1].contains("three"));
    assert!(contents[2].contains("two"));
}

#[test]
fn the_old_single_backup_moves_into_the_numbered_backups_at_startup() {
    let (_root, _installed, paths) = trial();
    std::fs::write(
        paths.root().join("config.yaml.bak"),
        "token: synthetic-legacy\n",
    )
    .unwrap();
    std::fs::write(
        paths.resource(Resource::Config),
        "token: synthetic-current\n",
    )
    .unwrap();
    ConfigStore::load_or_create(paths.clone(), || panic!("token must be kept")).unwrap();
    assert!(!paths.root().join("config.yaml.bak").exists());
    let backups = ConfigBackups::new(&paths).list().unwrap();
    assert_eq!(backups.len(), 1);
    assert_eq!(
        std::fs::read_to_string(&backups[0].path).unwrap(),
        "token: synthetic-legacy\n"
    );
}

#[tokio::test]
async fn the_binary_stops_on_an_unreadable_file_but_diagnosis_and_repair_still_run() {
    let root = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let file = root.path().join("config.yaml");
    let broken = format!("token: {SECRET}\nhub: [unterminated\nnote: {SECRET}\n");
    std::fs::write(&file, &broken).unwrap();
    let mut everything = String::new();

    // Commands that need the settings stop with the same message and a non-zero code.
    for args in [
        vec!["serve"],
        vec!["status"],
        vec!["stop"],
        vec!["shell-init"],
        vec!["log-clean", "missing.jsonl"],
        vec!["provider", "backup", "list"],
        vec!["orchestrate", "send"],
    ] {
        let output = run_binary(root.path(), home.path(), &args).await;
        let shown = text(&output);
        assert_eq!(exit_code(&output), Some(1), "{args:?}: {shown}");
        assert!(shown.contains("cannot be read"), "{args:?}: {shown}");
        assert!(
            shown.contains(&file.display().to_string()),
            "{args:?}: {shown}"
        );
        assert!(shown.contains("config reset"), "{args:?}: {shown}");
        assert!(shown.contains("at line "), "{args:?}: {shown}");
        everything.push_str(&shown);
    }
    // Nothing was rewritten, moved or replaced by any of them.
    assert_eq!(std::fs::read_to_string(&file).unwrap(), broken);
    assert!(!root.path().join("backups").exists());

    // Uninstall needs no settings: it runs (here it is cancelled at the prompt,
    // because no answer is given) and leaves the file alone.
    let output = run_binary(root.path(), home.path(), &["uninstall"]).await;
    let shown = text(&output);
    assert_eq!(exit_code(&output), Some(0), "{shown}");
    assert!(shown.contains("continuing without it"), "{shown}");
    everything.push_str(&shown);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), broken);
    // ... unless a Hub looks alive: without the token it cannot be asked to stop.
    let ledger = root.path().join("hub-runtime.json");
    std::fs::write(
        &ledger,
        format!(
            "{{\"version\":1,\"pid\":{},\"port\":49391,\"started_at\":\"2026-10-16T00:00:00Z\"}}",
            std::process::id()
        ),
    )
    .unwrap();
    let output = run_binary(root.path(), home.path(), &["uninstall"]).await;
    let shown = text(&output);
    assert_eq!(exit_code(&output), Some(1), "{shown}");
    assert!(shown.contains("A Hub appears to be running"), "{shown}");
    everything.push_str(&shown);
    std::fs::remove_file(&ledger).unwrap();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), broken);

    // These still work.
    for args in [
        vec!["version"],
        vec!["help"],
        vec!["config", "help"],
        vec!["config", "backup", "list"],
        vec!["doctor"],
        vec!["doctor", "-json"],
    ] {
        let output = run_binary(root.path(), home.path(), &args).await;
        let shown = text(&output);
        assert_eq!(exit_code(&output), Some(0), "{args:?}: {shown}");
        everything.push_str(&shown);
    }
    let output = run_binary(root.path(), home.path(), &["doctor"]).await;
    let shown = text(&output);
    assert!(shown.contains("[FAIL] config"), "{shown}");
    assert!(shown.contains("cannot be read"), "{shown}");
    let output = run_binary(root.path(), home.path(), &["help"]).await;
    assert!(text(&output).contains("many-ai-cli config"));

    // `config check` reports the problem and exits 1, in both forms.
    let output = run_binary(root.path(), home.path(), &["config", "check"]).await;
    assert_eq!(exit_code(&output), Some(1));
    everything.push_str(&text(&output));
    let output = run_binary(root.path(), home.path(), &["config", "check", "--json"]).await;
    assert_eq!(exit_code(&output), Some(1));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["status"], "unreadable");
    assert!(json["unreadable"]["line"].as_u64().unwrap() >= 1);
    everything.push_str(&text(&output));

    // Reset needs --yes for a file whose token cannot be read.
    let output = run_binary(root.path(), home.path(), &["config", "reset"]).await;
    assert_eq!(exit_code(&output), Some(1));
    everything.push_str(&text(&output));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), broken);
    let output = run_binary(root.path(), home.path(), &["config", "reset", "--yes"]).await;
    assert_eq!(exit_code(&output), Some(0), "{}", text(&output));
    everything.push_str(&text(&output));
    let backups = ConfigBackups::new(
        &RuntimePaths::trial(
            root.path(),
            49391,
            home.path().join(".many-ai-cli").as_path(),
        )
        .unwrap(),
    )
    .list()
    .unwrap();
    assert_eq!(backups.len(), 1);
    assert_eq!(std::fs::read_to_string(&backups[0].path).unwrap(), broken);

    // After the reset everything reads again.
    let output = run_binary(root.path(), home.path(), &["config", "check"]).await;
    assert_eq!(exit_code(&output), Some(0), "{}", text(&output));
    let output = run_binary(
        root.path(),
        home.path(),
        &["config", "backup", "list", "--json"],
    )
    .await;
    assert_eq!(exit_code(&output), Some(0));
    everything.push_str(&text(&output));

    assert!(!everything.contains(SECRET), "{everything}");
    assert!(!everything.contains("unterminated"), "{everything}");
    // The fresh token is also never printed.
    let fresh = std::fs::read_to_string(&file).unwrap();
    let token = fresh
        .lines()
        .find_map(|line| line.strip_prefix("token: "))
        .unwrap()
        .trim()
        .trim_matches('\'')
        .trim_matches('"')
        .to_owned();
    assert!(token.len() >= 16);
    assert!(!everything.contains(&token));
}

#[tokio::test]
async fn check_names_the_wrong_entry_in_a_file_that_otherwise_reads_fine() {
    let root = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let file = root.path().join("config.yaml");
    let original = format!(
        "token: {SECRET}\ncustom_providers:\n  - id: shell\n    command: x\n  - id: ok\n    command: ok\n"
    );
    std::fs::write(&file, &original).unwrap();
    let output = run_binary(root.path(), home.path(), &["config", "check", "--json"]).await;
    assert_eq!(exit_code(&output), Some(1));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["status"], "problems");
    let problems = json["problems"].as_array().unwrap();
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0]["index"], 1);
    assert_eq!(problems[0]["id"], "shell");
    assert_eq!(problems[0]["field"], "id");
    assert_eq!(problems[0]["code"], "reserved-id");
    assert!(
        problems[0]["effect"]
            .as_str()
            .unwrap()
            .contains("spawn option")
    );
    assert!(!text(&output).contains(SECRET));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), original);
}
