use super::*;
use crate::proto::provider::LaunchDefinition;
use crate::{
    application::maintenance_cli::{
        ProviderCommand, parse_command, provider_args::DEFAULT_TEST_PROMPT, provider_hub::HubReply,
    },
    config::{ConfigStore, CustomProvider, Resource},
    hub::{http::Request, provider_routes},
};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

/// The real provider routes and a real registry store, called in process. The
/// token and Origin checks belong to the router and have their own tests; what
/// the CLI must get right here is which routes it calls and what it sends.
struct InProcessHub {
    store: Arc<ProviderRegistryStore>,
    paths: RuntimePaths,
    calls: Mutex<Vec<String>>,
    changes: Mutex<Vec<(String, &'static str)>>,
    refuse_create: AtomicBool,
}
impl HubApi for InProcessHub {
    async fn call(&self, method: &str, path: &str, body: Option<Vec<u8>>) -> io::Result<HubReply> {
        let (route, query) = path.split_once('?').unwrap_or((path, ""));
        self.calls.lock().unwrap().push(format!("{method} {route}"));
        if self.refuse_create.load(Ordering::SeqCst)
            && method == "POST"
            && route == "/api/providers"
        {
            return Ok(HubReply {
                status: 409,
                body: br#"{"ok":false,"error":"provider_id_exists","detail":"provider id already exists"}"#
                    .to_vec(),
            });
        }
        let request = Request {
            method: method.into(),
            path: route.into(),
            query: query.into(),
            body: body.unwrap_or_default(),
            ..Default::default()
        };
        // The CLI holds only the Hub token: it never has the dashboard's proof.
        let response = provider_routes::handle(
            &request,
            &self.store,
            &self.paths,
            None,
            &|_| true,
            &|_, _, _| {},
        )
        .ok_or_else(|| io::Error::other("unrouted"))?;
        if let Some(change) = provider_routes::change_of(&request, &response) {
            self.changes
                .lock()
                .unwrap()
                .push((change.id, change.action));
        }
        Ok(HubReply {
            status: response.status,
            body: response.body,
        })
    }
}
struct Fixture {
    paths: RuntimePaths,
    config: Config,
    cli_store: Arc<ProviderRegistryStore>,
    hub: InProcessHub,
    cwd: PathBuf,
    environment: Vec<String>,
    // Last: on Windows the folder cannot go while a field above holds a file.
    _temp: tempfile::TempDir,
}
fn fixture_with(mut config: Config) -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("runtime");
    let installed = temp.path().join("installed");
    let cwd = temp.path().join("work");
    for dir in [&root, &installed, &cwd] {
        std::fs::create_dir(dir).unwrap();
    }
    let paths = RuntimePaths::trial(&root, 49373, &installed).unwrap();
    config.hub.port = 49373;
    let store = Arc::new(ConfigStore::new(paths.clone(), config).unwrap());
    let config = store.snapshot().unwrap().config;
    let cli_store = Arc::new(ProviderRegistryStore::new(&paths, store.clone()).unwrap());
    let hub_store = Arc::new(ProviderRegistryStore::new(&paths, store).unwrap());
    Fixture {
        hub: InProcessHub {
            store: hub_store,
            paths: paths.clone(),
            calls: Mutex::new(vec![]),
            changes: Mutex::new(vec![]),
            refuse_create: AtomicBool::new(false),
        },
        paths,
        config,
        cli_store,
        cwd,
        environment: vec!["PATH=".to_owned()],
        _temp: temp,
    }
}
fn fixture() -> Fixture {
    fixture_with(Config::default())
}
fn reg<'a, H: HubApi>(f: &'a Fixture, hub: Option<&'a H>) -> Registry<'a, H> {
    Registry {
        hub,
        store: &f.cli_store,
        paths: &f.paths,
        config: &f.config,
        environment: &f.environment,
        cwd: &f.cwd,
    }
}
fn offline(f: &Fixture) -> Registry<'_, InProcessHub> {
    reg::<InProcessHub>(f, None)
}
fn online(f: &Fixture) -> Registry<'_, InProcessHub> {
    reg(f, Some(&f.hub))
}
/// A synthetic "AI CLI": the platform's echo, so `provider test` has a real
/// process to run and a known answer.
fn echo_tool() -> (String, Vec<String>) {
    if cfg!(windows) {
        let shell =
            std::env::var("ComSpec").unwrap_or_else(|_| r"C:\Windows\System32\cmd.exe".into());
        (shell, vec!["/c".into(), "echo".into()])
    } else {
        ("/bin/echo".into(), vec![])
    }
}
fn add_args(id: &str) -> AddArgs {
    let (command, headless_args) = echo_tool();
    AddArgs {
        id: id.into(),
        label: "Fixture CLI".into(),
        command,
        args: vec![],
        headless_args,
        headless: Some(("text".into(), "arg".into())),
        description: String::new(),
        approval_options: vec![],
        ..Default::default()
    }
}
fn user_files(f: &Fixture) -> usize {
    std::fs::read_dir(f.paths.resource(Resource::ProviderDefinitions))
        .map(|dir| dir.count())
        .unwrap_or(0)
}

#[tokio::test]
async fn add_through_a_hub_validates_then_creates_and_reports_what_will_run() {
    let f = fixture();
    let report = online(&f).add(&add_args("fixture-cli")).await.unwrap();
    assert!(report.starts_with("added provider fixture-cli (Fixture CLI)\n"));
    assert!(report.contains("interactive: "));
    assert!(report.contains("headless: yes: "));
    assert!(
        report.contains("<prompt>"),
        "the argument prompt's place is shown"
    );
    assert!(report.contains("The running Hub reloaded the registry"));
    assert!(report.contains("next: many-ai-cli provider test fixture-cli"));
    let calls = f.hub.calls.lock().unwrap().clone();
    let validate = calls
        .iter()
        .position(|c| c == "POST /api/providers/validate")
        .unwrap();
    let create = calls
        .iter()
        .position(|c| c == "POST /api/providers")
        .unwrap();
    assert!(validate < create, "validate runs before create: {calls:?}");
    assert_eq!(
        *f.hub.changes.lock().unwrap(),
        [("fixture-cli".to_owned(), "created")]
    );
    let found = f
        .hub
        .store
        .snapshot()
        .unwrap()
        .registry
        .lookup("fixture-cli")
        .unwrap();
    assert_eq!(found.definition.display_name, "Fixture CLI");
    assert!(found.definition.launch.unwrap().headless.is_some());
    assert!(
        !report.contains("synthetic"),
        "no config values in the report"
    );
}
#[tokio::test]
async fn add_without_a_hub_writes_the_files_with_a_backup_and_says_the_hub_reads_it_later() {
    let f = fixture();
    let report = offline(&f).add(&add_args("fixture-cli")).await.unwrap();
    assert!(report.contains("No Hub is running; the Hub will read it on its next start."));
    assert!(f.hub.calls.lock().unwrap().is_empty());
    assert_eq!(user_files(&f), 1);
    let found = f
        .cli_store
        .snapshot()
        .unwrap()
        .registry
        .lookup("fixture-cli")
        .unwrap();
    assert_eq!(found.definition.id, "fixture-cli");
    let backups = f.cli_store.history.list_backups("fixture-cli").unwrap();
    assert_eq!(backups.len(), 1);
    assert_eq!(backups[0].reason, "create");
}
#[tokio::test]
async fn add_without_headless_options_says_headless_is_not_available() {
    let f = fixture();
    let mut args = add_args("fixture-cli");
    args.headless = None;
    args.headless_args = vec![];
    let report = offline(&f).add(&args).await.unwrap();
    assert!(report.contains("headless: no"));
    let list = offline(&f).list().await.unwrap();
    assert!(
        list.contains("fixture-cli\tFixture CLI\tuser\tenabled\tno\n"),
        "{list}"
    );
}
#[tokio::test]
async fn add_refuses_reserved_duplicate_missing_and_invalid_without_writing() {
    let f = fixture();
    for id in ["claude", "codex", "shell"] {
        let error = offline(&f)
            .add(&add_args(id))
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("reserved"), "{id}: {error}");
    }
    let error = offline(&f)
        .add(&add_args("Bad ID"))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("lowercase slug"), "{error}");
    let mut missing = add_args("fixture-cli");
    missing.command = "definitely-missing-agent-xyz".into();
    let error = offline(&f).add(&missing).await.unwrap_err().to_string();
    assert!(
        error.contains("not found on PATH") && error.contains("wrapper"),
        "{error}"
    );
    let mut broken = add_args("fixture-cli");
    broken.headless = Some(("text".into(), "pipe".into()));
    assert!(offline(&f).add(&broken).await.is_err());
    assert_eq!(user_files(&f), 0, "nothing written by any refusal");
    assert!(f.hub.calls.lock().unwrap().is_empty());

    offline(&f).add(&add_args("fixture-cli")).await.unwrap();
    let error = offline(&f)
        .add(&add_args("fixture-cli"))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("already exists"), "{error}");
    assert_eq!(user_files(&f), 1);
}
#[tokio::test]
async fn add_through_a_hub_stops_at_a_refusal_before_creating() {
    let f = fixture();
    let mut broken = add_args("fixture-cli");
    broken.headless = Some(("text".into(), "pipe".into()));
    let error = online(&f).add(&broken).await.unwrap_err().to_string();
    assert!(
        error.contains("prompt_via") || error.contains("refused"),
        "{error}"
    );
    let calls = f.hub.calls.lock().unwrap().clone();
    assert!(calls.contains(&"POST /api/providers/validate".to_owned()));
    assert!(
        !calls.contains(&"POST /api/providers".to_owned()),
        "{calls:?}"
    );
    assert_eq!(user_files(&f), 0);
    assert!(f.hub.changes.lock().unwrap().is_empty());
    // A duplicate is caught from the list, before any write.
    online(&f).add(&add_args("fixture-cli")).await.unwrap();
    let before = f.hub.calls.lock().unwrap().len();
    let error = online(&f)
        .add(&add_args("fixture-cli"))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("already exists (user)"), "{error}");
    assert_eq!(
        f.hub.calls.lock().unwrap()[before..],
        ["GET /api/providers".to_owned()]
    );
}
#[tokio::test]
async fn approval_options_become_a_phrase_file_the_definition_points_at() {
    let f = fixture();
    let mut args = add_args("fixture-cli");
    args.approval_options = vec!["Allow once".into(), "Deny".into()];
    let report = online(&f).add(&args).await.unwrap();
    let file = f
        .paths
        .root()
        .join("approval-patterns")
        .join("fixture-cli.md");
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("- `Allow once`\n- `Deny`\n"), "{text}");
    assert!(report.contains("approval phrases: 2 written to "));
    let source = f
        .hub
        .store
        .snapshot()
        .unwrap()
        .registry
        .lookup("fixture-cli")
        .unwrap()
        .definition
        .approval_pattern_source;
    assert_eq!(PathBuf::from(source), file);
}
#[tokio::test]
async fn a_failed_create_leaves_no_phrase_file_and_a_duplicate_keeps_the_old_one() {
    let f = fixture();
    let mut args = add_args("fixture-cli");
    args.approval_options = vec!["Allow once".into()];
    f.hub.refuse_create.store(true, Ordering::SeqCst);
    assert!(online(&f).add(&args).await.is_err());
    let file = f
        .paths
        .root()
        .join("approval-patterns")
        .join("fixture-cli.md");
    assert!(
        !file.exists(),
        "phrase file removed after the refused create"
    );
    f.hub.refuse_create.store(false, Ordering::SeqCst);
    online(&f).add(&args).await.unwrap();
    let original = std::fs::read_to_string(&file).unwrap();
    args.approval_options = vec!["Different".into()];
    assert!(online(&f).add(&args).await.is_err());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), original);
}
fn phrase_file(f: &Fixture, id: &str) -> PathBuf {
    f.paths
        .root()
        .join("approval-patterns")
        .join(format!("{id}.md"))
}
#[tokio::test]
async fn add_never_overwrites_or_deletes_a_phrase_file_it_did_not_create() {
    let f = fixture();
    let file = phrase_file(&f, "fixture-cli");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "someone else's phrases").unwrap();
    let mut args = add_args("fixture-cli");
    args.approval_options = vec!["Allow once".into()];
    for registry in [online(&f), offline(&f)] {
        let error = registry.add(&args).await.unwrap_err().to_string();
        assert!(error.contains("already exists"), "{error}");
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "someone else's phrases"
        );
    }
    assert_eq!(user_files(&f), 0);
    // A create the Hub refuses after this run wrote its own file removes only that one.
    std::fs::remove_file(&file).unwrap();
    f.hub.refuse_create.store(true, Ordering::SeqCst);
    assert!(online(&f).add(&args).await.is_err());
    assert!(!file.exists());
}
#[tokio::test]
async fn remove_deletes_the_phrase_file_add_created_and_only_that_one() {
    for through_hub in [true, false] {
        let f = fixture();
        let mut args = add_args("fixture-cli");
        args.approval_options = vec!["Allow once".into()];
        let registry = if through_hub { online(&f) } else { offline(&f) };
        registry.add(&args).await.unwrap();
        let file = phrase_file(&f, "fixture-cli");
        assert!(file.exists());
        registry.remove("fixture-cli").await.unwrap();
        assert!(!file.exists(), "hub: {through_hub}");
    }
    // A definition whose source is elsewhere keeps a same-named file in place.
    let f = fixture();
    let elsewhere = f._temp.path().join("work").join("their-phrases.md");
    std::fs::write(&elsewhere, "theirs").unwrap();
    let def = Definition {
        schema_version: CURRENT_SCHEMA_VERSION,
        id: "other-cli".into(),
        display_name: "Other".into(),
        launch: Some(LaunchDefinition {
            executable: echo_tool().0,
            ..Default::default()
        }),
        approval_pattern_source: elsewhere.to_string_lossy().into_owned(),
        ..Default::default()
    };
    f.cli_store.definitions.create_new(&def).unwrap();
    f.cli_store.reload().unwrap();
    let same_name = phrase_file(&f, "other-cli");
    std::fs::create_dir_all(same_name.parent().unwrap()).unwrap();
    std::fs::write(&same_name, "not ours").unwrap();
    offline(&f).remove("other-cli").await.unwrap();
    assert!(same_name.exists() && elsewhere.exists());
}
#[tokio::test]
async fn a_hub_with_another_data_folder_refuses_the_phrase_file_and_the_cli_says_why() {
    let mut f = fixture();
    let other = f._temp.path().join("other-runtime");
    std::fs::create_dir(&other).unwrap();
    f.hub.paths = RuntimePaths::trial(&other, 49373, &f._temp.path().join("installed")).unwrap();
    let mut args = add_args("fixture-cli");
    args.approval_options = vec!["Allow once".into()];
    let error = online(&f).add(&args).await.unwrap_err().to_string();
    assert!(
        error.contains("approval_pattern_source") && error.contains("hint:"),
        "{error}"
    );
    assert_eq!(user_files(&f), 0, "no provider was created");
    assert!(
        !phrase_file(&f, "fixture-cli").exists(),
        "no phrase file was written"
    );
}
#[test]
fn test_removes_every_inherited_hub_session_variable() {
    let environment = vec![
        "MANY_AI_CLI_HUB_TOKEN=synthetic".to_owned(),
        "aNy_Ai_Cli_HuB_tOkEn=synthetic".to_owned(),
        "MANY_AI_CLI_LOCAL_PROOF=synthetic".to_owned(),
        "Many_Ai_Cli_Future=synthetic".to_owned(),
        "PATH=synthetic".to_owned(),
    ];
    let inherited = [
        "MANY_AI_CLI_SOMETHING_NEW",
        "MANY_AI_CLI_SESSION_ID",
        "HOME",
    ]
    .map(std::ffi::OsString::from);
    let removed = hub_session_variables(&environment, inherited);
    let names: Vec<String> = removed
        .iter()
        .map(|(name, value)| {
            assert!(value.is_none(), "removed, never set");
            name.to_string_lossy().into_owned()
        })
        .collect();
    for needle in [
        "MANY_AI_CLI_HUB_TOKEN",
        "aNy_Ai_Cli_HuB_tOkEn",
        "ANY_AI_CLI_LOCAL_PROOF",
        "MANY_AI_CLI_HUB_PORT",
        "MANY_AI_CLI_SESSION_ID",
        "MANY_AI_CLI_LOCAL_PROOF",
        "MANY_AI_CLI_SOMETHING_NEW",
    ] {
        assert!(names.iter().any(|n| n == needle), "{needle} in {names:?}");
    }
    assert_eq!(
        names.iter().any(|n| n == "Many_Ai_Cli_Future"),
        cfg!(windows),
        "mixed case counts on Windows only: {names:?}"
    );
    assert!(!names.iter().any(|n| n == "PATH" || n == "HOME"));
}
#[tokio::test]
async fn test_output_carries_no_terminal_escape_sequence() {
    let f = fixture();
    offline(&f).add(&add_args("fixture-cli")).await.unwrap();
    let report = offline(&f)
        .test(&TestArgs {
            id: "fixture-cli".into(),
            prompt: "\u{1b}[31mred\u{1b}[0m \u{1b}]0;title\u{7}ok".into(),
            timeout_secs: 30,
        })
        .await
        .unwrap();
    assert!(
        !report.chars().any(|c| c == '\u{1b}' || c == '\u{7}'),
        "{report:?}"
    );
}
#[test]
fn printable_line_removes_ansi_and_osc_sequences_and_controls() {
    assert_eq!(
        printable_line("\u{1b}[31mred\u{1b}[0m \u{1b}]0;window title\u{7}ok"),
        "red ok"
    );
    let hyperlink =
        printable_line("a\u{1b}]8;;http://example.invalid\u{1b}\\link\u{1b}]8;;\u{1b}\\\tb\r");
    assert!(hyperlink.starts_with('a') && !hyperlink.chars().any(char::is_control));
    assert!(!hyperlink.contains("example.invalid"));
    assert!(
        !printable_line("x\u{0}\u{8}\u{1b}\u{9b}y")
            .chars()
            .any(char::is_control)
    );
}
#[tokio::test]
async fn list_shows_origin_enabled_and_headless_from_the_hub_and_from_disk() {
    let mut config = Config::default();
    config.custom_providers.push(CustomProvider {
        id: "legacy-cli".into(),
        label: "Legacy CLI".into(),
        command: "legacy-agent".into(),
        ..Default::default()
    });
    let f = fixture_with(config);
    online(&f).add(&add_args("fixture-cli")).await.unwrap();
    for list in [
        online(&f).list().await.unwrap(),
        // A fresh store sees the Hub's write on disk.
        {
            let fresh = Arc::new(
                ProviderRegistryStore::new(
                    &f.paths,
                    Arc::new(ConfigStore::new(f.paths.clone(), f.config.clone()).unwrap()),
                )
                .unwrap(),
            );
            Registry::<InProcessHub> {
                hub: None,
                store: &fresh,
                paths: &f.paths,
                config: &f.config,
                environment: &f.environment,
                cwd: &f.cwd,
            }
            .list()
            .await
            .unwrap()
        },
    ] {
        let line = |id: &str| {
            list.lines()
                .find(|l| l.starts_with(&format!("{id}\t")))
                .unwrap_or_else(|| panic!("no line for {id}: {list}"))
                .to_owned()
        };
        assert!(line("claude").contains("\tbuilt-in\tenabled\t"), "{list}");
        assert!(
            line("fixture-cli").ends_with("\tFixture CLI\tuser\tenabled\tyes")
                || line("fixture-cli").contains("\tuser\tenabled\tyes")
        );
        assert!(line("legacy-cli").contains("\tlegacy\tenabled\t"), "{list}");
    }
}
#[tokio::test]
async fn remove_through_a_hub_keeps_a_before_delete_backup_and_announces_it() {
    let f = fixture();
    online(&f).add(&add_args("fixture-cli")).await.unwrap();
    let report = online(&f).remove("fixture-cli").await.unwrap();
    assert!(report.starts_with("removed provider fixture-cli\n"));
    assert!(
        f.hub
            .calls
            .lock()
            .unwrap()
            .contains(&"DELETE /api/providers/fixture-cli".to_owned())
    );
    assert_eq!(
        *f.hub.changes.lock().unwrap(),
        [
            ("fixture-cli".to_owned(), "created"),
            ("fixture-cli".to_owned(), "removed")
        ]
    );
    assert!(
        f.hub
            .store
            .snapshot()
            .unwrap()
            .registry
            .lookup("fixture-cli")
            .is_none()
    );
    assert_eq!(user_files(&f), 0);
    let reasons: Vec<_> = f
        .hub
        .store
        .history
        .list_backups("fixture-cli")
        .unwrap()
        .into_iter()
        .map(|b| b.reason)
        .collect();
    assert!(reasons.contains(&"before-delete".to_owned()), "{reasons:?}");
}
#[tokio::test]
async fn remove_without_a_hub_deletes_the_file_and_backs_it_up() {
    let f = fixture();
    offline(&f).add(&add_args("fixture-cli")).await.unwrap();
    offline(&f).remove("fixture-cli").await.unwrap();
    assert_eq!(user_files(&f), 0);
    assert!(
        f.cli_store
            .snapshot()
            .unwrap()
            .registry
            .lookup("fixture-cli")
            .is_none()
    );
    let reasons: Vec<_> = f
        .cli_store
        .history
        .list_backups("fixture-cli")
        .unwrap()
        .into_iter()
        .map(|b| b.reason)
        .collect();
    assert!(reasons.contains(&"before-delete".to_owned()), "{reasons:?}");
}
#[tokio::test]
async fn remove_refuses_built_in_legacy_and_unknown_providers() {
    let mut config = Config::default();
    config.custom_providers.push(CustomProvider {
        id: "legacy-cli".into(),
        command: "legacy-agent".into(),
        ..Default::default()
    });
    let f = fixture_with(config);
    for (id, needle) in [
        ("claude", "built in"),
        ("legacy-cli", "config.yaml"),
        ("nothing-here", "was not found"),
    ] {
        for registry in [online(&f), offline(&f)] {
            let error = registry.remove(id).await.unwrap_err().to_string();
            assert!(error.contains(needle), "{id}: {error}");
        }
    }
    assert!(
        !f.hub
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|c| c.starts_with("DELETE")),
        "a refusal never reaches the delete route"
    );
}
#[tokio::test]
async fn test_runs_the_headless_definition_once_and_prints_exit_code_time_and_output() {
    let f = fixture();
    offline(&f).add(&add_args("fixture-cli")).await.unwrap();
    // The Hub's store loaded before the CLI wrote the file; a real Hub reloads on
    // its next start (or through the API), which this stands in for.
    f.hub.store.reload().unwrap();
    for registry in [offline(&f), online(&f)] {
        let report = registry
            .test(&TestArgs {
                id: "fixture-cli".into(),
                prompt: "42".into(),
                timeout_secs: 30,
            })
            .await
            .unwrap();
        assert!(report.starts_with("exit code: 0\nelapsed: "), "{report}");
        assert!(
            report.contains("output (first 10 lines):\n  42\n"),
            "{report}"
        );
    }
}
#[tokio::test]
async fn test_refuses_a_provider_without_headless_and_an_unknown_one() {
    let f = fixture();
    let mut args = add_args("fixture-cli");
    args.headless = None;
    args.headless_args = vec![];
    offline(&f).add(&args).await.unwrap();
    let test = |id: &str| TestArgs {
        id: id.into(),
        prompt: "x".into(),
        timeout_secs: 5,
    };
    let error = offline(&f)
        .test(&test("fixture-cli"))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("no headless definition"), "{error}");
    assert!(
        error.contains("unavailable  headless"),
        "the capability list follows the refusal: {error}"
    );
    let error = offline(&f)
        .test(&test("nothing-here"))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("was not found"), "{error}");
}
#[tokio::test]
async fn test_kills_a_run_that_outlives_the_timeout() {
    let f = fixture();
    let mut args = add_args("slow-cli");
    if cfg!(windows) {
        args.headless_args = ["/c", "ping", "-n", "30", "127.0.0.1"]
            .map(String::from)
            .to_vec();
    } else {
        args.command = "/bin/sleep".into();
        args.headless_args = vec!["30".into()];
    }
    args.headless = Some(("text".into(), "stdin".into()));
    offline(&f).add(&args).await.unwrap();
    let started = Instant::now();
    let error = offline(&f)
        .test(&TestArgs {
            id: "slow-cli".into(),
            prompt: "x".into(),
            timeout_secs: 1,
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("killed after the 1 s timeout"), "{error}");
    assert!(started.elapsed() < Duration::from_secs(20));
}
#[tokio::test]
async fn history_commands_go_through_the_hub_only_when_one_runs() {
    let f = fixture();
    let strings = |values: &[&str]| values.iter().map(|v| v.to_string()).collect::<Vec<_>>();
    let reset = strings(&[
        "reset",
        "--distributed",
        "claude",
        "--expected-revision",
        "",
    ]);
    assert_eq!(offline(&f).history_via_hub(&reset).await.unwrap(), None);
    // A running Hub changes a built-in only from the dashboard: the CLI holds
    // only the token, so it is told so and nothing changes.
    let refused = online(&f)
        .history_via_hub(&reset)
        .await
        .unwrap_err()
        .to_string();
    assert!(refused.contains("only from the dashboard"), "{refused}");
    assert!(
        f.hub
            .calls
            .lock()
            .unwrap()
            .contains(&"POST /api/providers/claude/reset".to_owned())
    );
    assert!(f.hub.changes.lock().unwrap().is_empty());
    // Read-only and unrelated forms stay in process.
    for args in [
        strings(&["recover", "claude", "--list"]),
        strings(&["backup", "list", "claude"]),
        strings(&["something-else"]),
    ] {
        assert_eq!(online(&f).history_via_hub(&args).await.unwrap(), None);
    }
    assert!(
        online(&f)
            .history_via_hub(&strings(&[
                "reset",
                "--distributed",
                "nope",
                "--expected-revision",
                ""
            ]))
            .await
            .is_err()
    );
    assert!(
        online(&f)
            .history_via_hub(&strings(&["reset", "--distributed", "claude"]))
            .await
            .is_err()
    );
}
#[test]
fn parse_command_serves_help_without_a_hub_and_routes_the_rest() {
    let strings = |values: &[&str]| values.iter().map(|v| v.to_string()).collect::<Vec<_>>();
    for (args, needle) in [
        (&["--help"][..], "provider add --id"),
        (&["add", "--help"][..], "IF YOU ARE AN AI AGENT"),
        (&["list", "-h"][..], "tab-separated"),
        (&["remove", "--help"][..], "Built-in CLIs"),
        (&["test", "--help"][..], "first 10 lines"),
    ] {
        let ProviderCommand::Text(text) = parse_command(&strings(args)).unwrap() else {
            panic!("{args:?} should print help")
        };
        assert!(text.contains(needle), "{args:?}");
    }
    assert!(parse_command(&strings(&[])).is_err());
    assert!(matches!(
        parse_command(&strings(&["backup", "list", "claude"])).unwrap(),
        ProviderCommand::History
    ));
    assert!(parse_command(&strings(&["add", "--id", "x"])).is_err());
    assert!(matches!(
        parse_command(&strings(&["test", "x", "--prompt", DEFAULT_TEST_PROMPT])).unwrap(),
        ProviderCommand::Test(_)
    ));
    assert!(matches!(
        parse_command(&strings(&[
            "add",
            "--id",
            "x",
            "--command",
            "c",
            "--arg",
            "-h"
        ]))
        .unwrap(),
        ProviderCommand::Add(_)
    ));
}

fn definition_file(f: &Fixture, extra: serde_json::Value) -> String {
    let (command, headless_args) = echo_tool();
    let mut value = serde_json::json!({
        "schema_version": 1,
        "id": "file-cli",
        "display_name": "File CLI",
        "launch": {"executable": command, "headless": {"args": headless_args, "format": "text", "prompt_via": "arg"}},
        "links": {"usage_url": "https://example.test/usage"},
        "slash_commands": {"items": [{"cmd": "/help", "desc": "help"}]},
    });
    for (key, entry) in extra.as_object().unwrap() {
        value[key] = entry.clone();
    }
    std::fs::write(f.cwd.join("def.json"), serde_json::to_vec(&value).unwrap()).unwrap();
    "def.json".into()
}
#[tokio::test]
async fn add_from_a_definition_file_registers_it_and_the_flags_win() {
    let f = fixture();
    let file = definition_file(&f, serde_json::json!({}));
    let report = online(&f)
        .add(&AddArgs {
            definition_file: Some(file),
            letter: "F".into(),
            models: vec!["m1".into(), "m2".into()],
            permission_modes: vec![("auto".into(), vec!["--yes".into()])],
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(
        report.starts_with(
            "added provider file-cli (File CLI)
"
        ),
        "{report}"
    );
    assert!(report.contains("available    slash_commands"), "{report}");
    assert!(report.contains("available    usage_link"), "{report}");
    assert!(report.contains("unavailable  approval"), "{report}");
    assert!(report.contains("unsupported  usage_meter"), "{report}");
    let found = f
        .hub
        .store
        .snapshot()
        .unwrap()
        .registry
        .lookup("file-cli")
        .unwrap()
        .definition;
    assert_eq!(found.presentation.unwrap().icon_text, "F");
    assert_eq!(found.models.unwrap().items.len(), 2);
    let permissions = found.permissions.unwrap();
    assert_eq!(permissions.modes["auto"], ["--yes"]);
    assert_eq!(permissions.choices, ["auto"]);
    assert_eq!(found.slash_commands.unwrap().items.len(), 1);
}
#[tokio::test]
async fn add_from_a_definition_file_refuses_bad_input_without_writing() {
    let f = fixture();
    let missing = offline(&f)
        .add(&AddArgs {
            definition_file: Some("nope.json".into()),
            ..Default::default()
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(
        missing.contains("could not read the definition file"),
        "{missing}"
    );
    std::fs::write(f.cwd.join("bad.json"), b"{ not json").unwrap();
    let bad = offline(&f)
        .add(&AddArgs {
            definition_file: Some("bad.json".into()),
            ..Default::default()
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(bad.contains("is not valid"), "{bad}");
    let reserved = definition_file(&f, serde_json::json!({"id": "claude"}));
    let error = offline(&f)
        .add(&AddArgs {
            definition_file: Some(reserved),
            ..Default::default()
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("reserved"), "{error}");
    let invalid = definition_file(&f, serde_json::json!({"connection": "carrier-pigeon"}));
    let error = offline(&f)
        .add(&AddArgs {
            definition_file: Some(invalid),
            ..Default::default()
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("not valid"), "{error}");
    assert_eq!(user_files(&f), 0);
}
#[tokio::test]
async fn test_prints_the_capability_list_and_checks_a_declared_conversation_folder() {
    let mut f = fixture();
    let sessions = f.cwd.join("sessions");
    std::fs::create_dir(&sessions).unwrap();
    f.environment
        .push(format!("FIXTURE_ROOT={}", f.cwd.display()));
    let file = definition_file(
        &f,
        serde_json::json!({"transcript": {"root": "{env:FIXTURE_ROOT}/sessions", "find": ["session_id"]}}),
    );
    offline(&f)
        .add(&AddArgs {
            definition_file: Some(file),
            ..Default::default()
        })
        .await
        .unwrap();
    let report = offline(&f)
        .test(&TestArgs {
            id: "file-cli".into(),
            prompt: "hi".into(),
            timeout_secs: 30,
        })
        .await
        .unwrap();
    assert!(report.contains("exit code: "), "{report}");
    assert!(report.contains("capabilities (available / unavailable / unsupported):"));
    assert!(report.contains("available    headless"), "{report}");
    assert!(
        report.contains("transcript - checked: the conversation location exists"),
        "{report}"
    );
    assert!(report.contains("unsupported  subagent_tree"), "{report}");
    std::fs::remove_dir(&sessions).unwrap();
    let report = offline(&f)
        .test(&TestArgs {
            id: "file-cli".into(),
            prompt: "hi".into(),
            timeout_secs: 30,
        })
        .await
        .unwrap();
    assert!(
        report.contains(
            "unavailable  transcript - the declared conversation location does not exist yet"
        ),
        "{report}"
    );
}

async fn provider_test(f: &Fixture) -> String {
    offline(f)
        .test(&TestArgs {
            id: "file-cli".into(),
            prompt: "hi".into(),
            timeout_secs: 30,
        })
        .await
        .unwrap()
}

/// End to end for a registered CLI: a definition file with declarations goes in
/// through `provider add --definition`, and every declared capability is then
/// read back through the same code paths a built-in provider uses (capability
/// list, launch flags, routines, approval detection, conversation folder).
/// Synthetic id and values only.
#[tokio::test]
async fn a_registered_cli_with_declarations_works_like_a_built_in_end_to_end() {
    use crate::profile::declared;
    let mut f = fixture();
    let sessions = f.cwd.join("sessions");
    std::fs::create_dir(&sessions).unwrap();
    f.environment
        .push(format!("FIXTURE_ROOT={}", f.cwd.display()));
    let file = definition_file(
        &f,
        serde_json::json!({
            "presentation": {"icon_text": "E"},
            "permissions": {"modes": {"auto": ["--yes"]}, "choices": ["auto"]},
            "input": {"slash_submit": "paste"},
            "transcript": {"root": "{env:FIXTURE_ROOT}/sessions", "find": ["session_id"]},
            "approval": {"arrow_send": true, "hint_phrases": ["trust this folder"]},
        }),
    );
    // 1. Add it from the definition file (written under the data folder).
    let report = offline(&f)
        .add(&AddArgs {
            definition_file: Some(file),
            ..Default::default()
        })
        .await
        .unwrap();
    for line in [
        "available    slash_commands",
        "available    permissions",
        "available    approval",
        "available    transcript",
        "available    usage_link",
        "unsupported  usage_meter",
        "unsupported  subscription_profiles",
        "unsupported  subagent_tree",
    ] {
        assert!(report.contains(line), "{line}\n{report}");
    }
    // 2. The registry the Hub holds now resolves the declarations.
    let registry = f.cli_store.snapshot().unwrap().registry;
    let _installed = declared::install_test_registry((*registry).clone());
    let def = declared::declared("file-cli").expect("registered definition");
    assert_eq!(def.slash_commands.as_ref().unwrap().items[0].cmd, "/help");
    assert_eq!(def.input.as_ref().unwrap().slash_submit, "paste");
    assert_eq!(def.presentation.as_ref().unwrap().icon_text, "E");
    assert!(declared::routines_allowed(&def));
    assert_eq!(declared::completion_mode(&def), "marker");
    assert_eq!(
        declared::permission_args(Some(&def), "auto"),
        (vec!["--yes".to_string()], "auto".to_string())
    );
    assert!(declared::permission_args(Some(&def), "other").0.is_empty());
    assert!(declared::automation_refusal("file-cli").is_none());
    // 3. Approval detection follows the declared wording and nothing else.
    let screen: Vec<String> = ["Trust this folder?", "  1. Yes", "❯ 2. No", "  3. Never"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let found = crate::approval::detect::detect_native("file-cli", &screen, &[])
        .expect("a declared approval screen is detected");
    assert_eq!(found.options.len(), 3);
    assert!(
        crate::approval::detect::detect_native("an-unregistered-cli", &screen, &[]).is_none(),
        "the same screen is no approval for a CLI that declares nothing"
    );
    // 4. `provider test` checks the declared conversation folder for real.
    assert!(
        provider_test(&f)
            .await
            .contains("transcript - checked: the conversation location exists")
    );
    std::fs::remove_dir(&sessions).unwrap();
    assert!(provider_test(&f).await.contains(
        "unavailable  transcript - the declared conversation location does not exist yet"
    ));
}
