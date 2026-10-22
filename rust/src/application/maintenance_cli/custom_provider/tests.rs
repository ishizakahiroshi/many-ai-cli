use super::*;
use crate::config::{ConfigBackups, Resource, RuntimePaths};

const TOKEN: &str = "synthetic-private-token";
struct Fixture {
    _temp: tempfile::TempDir,
    paths: RuntimePaths,
}
impl Fixture {
    fn new(body: Option<&str>) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("runtime");
        let installed = temp.path().join("installed");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&installed).unwrap();
        let paths = RuntimePaths::trial(&root, 49372, &installed).unwrap();
        if let Some(body) = body {
            std::fs::write(paths.resource(Resource::Config), body).unwrap();
        }
        Self { _temp: temp, paths }
    }
    fn store(&self) -> Arc<ConfigStore> {
        Arc::new(ConfigStore::load_for_provider(self.paths.clone()).unwrap())
    }
    fn run(&self, args: &[&str]) -> io::Result<String> {
        run_with(&self.store(), &strings(args), &|name| {
            if name == "fake-cli" { Ok(name.into()) } else { Err(io::ErrorKind::NotFound.into()) }
        })
    }
    fn bytes(&self) -> Vec<u8> {
        std::fs::read(self.paths.resource(Resource::Config)).unwrap()
    }
    fn backups(&self) -> usize {
        ConfigBackups::new(&self.paths).list().unwrap().len()
    }
}
fn strings(args: &[&str]) -> Vec<String> { args.iter().map(|s| (*s).into()).collect() }
fn fixture() -> Fixture { Fixture::new(Some(&format!("token: {TOKEN}\n"))) }
fn add() -> Vec<&'static str> { vec!["add", "--id", "muse", "--label", "Muse Code", "--command", "fake-cli", "--headless-args", "exec", "--headless-format", "text", "--prompt-via", "arg"] }

#[test]
fn successful_add_remove_have_one_backup_each_and_no_token_in_output() {
    let f = fixture();
    let original = f.bytes();
    let out = f.run(&add()).unwrap();
    assert!(!out.contains(TOKEN));
    assert!(out.contains("Restart") || out.contains("restart"));
    assert_eq!(f.backups(), 1);
    let records = ConfigBackups::new(&f.paths).list().unwrap();
    assert_eq!(ConfigBackups::new(&f.paths).read(&records[0]).unwrap(), original);
    assert!(f.store().snapshot().unwrap().config.is_custom_provider_id("muse"));
    let before = f.bytes();
    assert!(f.run(&add()).is_err());
    assert_eq!(f.bytes(), before);
    assert_eq!(f.backups(), 1);
    for args in [vec!["list"], vec!["list", "--json"]] {
        let out = f.run(&args).unwrap();
        assert!(out.contains("muse"));
        assert!(!out.contains(TOKEN));
    }
    let out = f.run(&["remove", "muse"]).unwrap();
    assert!(!out.contains(TOKEN));
    assert!(out.contains("approval-patterns"));
    assert_eq!(f.backups(), 2);
    assert!(!f.store().snapshot().unwrap().config.is_custom_provider_id("muse"));
}

#[test]
fn invalid_flags_are_reported_together_without_changing_file_or_backups() {
    let f = fixture();
    let original = f.bytes();
    let cases = [
        vec!["add", "--id", "claude", "--command", "fake-cli"],
        vec!["add", "--id", "serve", "--command", "fake-cli"],
        vec!["add", "--id", "wrap", "--command", "fake-cli"],
        vec!["add", "--id", "muse", "--command", "missing-cli"],
        vec!["add", "--id", "muse", "--command", "\"fake-cli"],
        vec!["add", "--id", "muse", "--command", "fake-cli\narg"],
        vec!["add", "--id", "muse", "--command", "fake-cli", "--headless-format", "json", "--prompt-via", "arg"],
        vec!["remove", "claude"], vec!["remove", "missing"],
        vec!["add", "--id", "common", "--command", "fake-cli", "--approval-option", "Allow"],
    ];
    for args in cases {
        assert!(f.run(&args).is_err(), "{args:?}");
        assert_eq!(f.bytes(), original);
        assert_eq!(f.backups(), 0);
    }
    let error = f.run(&["add", "--id", "provider", "--command", "missing-cli", "--headless-format", "json", "--prompt-via", "socket", "--headless-args", "\n"]).unwrap_err().to_string();
    for flag in ["--id", "--command", "--headless-format", "--prompt-via", "--headless-args"] { assert!(error.contains(flag), "{error}"); }
    assert_eq!(f.bytes(), original);
    assert_eq!(f.backups(), 0);
}

#[test]
fn missing_or_tokenless_config_is_not_written_on_invalid_input() {
    let absent = Fixture::new(None);
    assert!(absent.run(&["add", "--id", "provider"]).is_err());
    assert!(!absent.paths.resource(Resource::Config).exists());
    let out = absent.run(&add()).unwrap();
    assert!(out.contains("no backup was needed"));
    assert_eq!(absent.backups(), 0);
    let tokenless = Fixture::new(Some("# retain this\ncustom_providers: []\n"));
    let original = tokenless.bytes();
    assert!(tokenless.run(&["add", "--id", "serve"]).is_err());
    assert_eq!(tokenless.bytes(), original);
    assert_eq!(tokenless.backups(), 0);
    assert!(tokenless.run(&add()).unwrap().contains("Comments were not kept"));
    assert_eq!(tokenless.backups(), 1);
}

#[test]
fn comment_notice_matches_original_file_and_dropped_id_can_be_removed() {
    assert!(!fixture().run(&add()).unwrap().contains("Comments were not kept"));
    let f = Fixture::new(Some(&format!("# previous settings\ntoken: {TOKEN}\ncustom_providers:\n  - id: broken\n    command: '\"unterminated'\n")));
    let list = f.run(&["list", "--json"]).unwrap();
    assert!(list.contains("dropped"));
    let reason = f.store().startup_problems()[0].text_en().reason.clone();
    let values: Vec<Value> = serde_json::from_str(&list).unwrap();
    assert!(values.iter().any(|row| row["reasons"].as_array().unwrap().iter().any(|value| value == &reason)));
    let out = f.run(&["remove", "broken"]).unwrap();
    assert!(out.contains("Comments were not kept"));
    assert_eq!(f.backups(), 1);
    assert!(!f.bytes().windows(b"broken".len()).any(|w| w == b"broken"));
}

#[test]
fn approval_options_load_and_survive_remove_and_readd() {
    let f = fixture();
    let mut args = add();
    args.extend(["--approval-option", "Allow once", "--approval-option", "Reject"]);
    f.run(&args).unwrap();
    let path = custom_pattern_path(&f.paths, "muse");
    let before = std::fs::read(&path).unwrap();
    let strings: Vec<String> = serde_json::from_slice(&before).unwrap();
    assert_eq!(strings, ["Allow once", "Reject"]);
    let patterns = ApprovalPatterns::new(&f.paths, f.store(), Weak::new(), Arc::new(|_| {})).unwrap();
    assert_eq!(patterns.active().unwrap()["muse"], strings);
    f.run(&["remove", "muse"]).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), before);
    f.run(&add()).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn displayed_custom_fields_and_errors_redact_a_token_copied_into_them() {
    let f = Fixture::new(Some(&format!("token: {TOKEN}\ncustom_providers:\n  - id: redaction\n    label: {TOKEN}\n    command: fake-cli {TOKEN}\n")));
    for args in [vec!["list"], vec!["list", "--json"]] { assert!(!f.run(&args).unwrap().contains(TOKEN)); }
    let out = f.run(&["add", "--id", "second", "--label", TOKEN, "--command", "fake-cli"]).unwrap();
    assert!(!out.contains(TOKEN));
}

#[cfg(windows)]
#[test]
fn windows_path_finds_muse_cmd_without_running_it() {
    let f = fixture();
    let shim = f.paths.root().join("muse.cmd");
    std::fs::write(&shim, "@echo synthetic\r\n").unwrap();
    let environment = vec![format!("PATH={}", f.paths.root().display()), "PATHEXT=.COM;.EXE;.BAT;.CMD".into()];
    let resolver = Resolver::new(Platform::Windows, &environment, f.paths.root(), &NativeFs);
    let result = run_with(&f.store(), &strings(&["add", "--id", "muse", "--command", "muse"]), &|name| resolver.look_path(name));
    assert!(result.is_ok());
    assert_eq!(f.backups(), 1);
}

#[test]
fn expanded_approval_file_limit_is_checked_before_config_or_backup_write() {
    let f = fixture();
    let original = f.bytes();
    for character in ['<', '>', '&', '\u{2028}', '\u{2029}'] {
        let option = character.to_string().repeat(350_000);
        let mut args = add();
        args.extend(["--approval-option", option.as_str()]);
        let error = f.run(&args).unwrap_err().to_string();
        assert!(error.contains("--approval-option") && error.contains("2 MiB"));
        assert_eq!(f.bytes(), original);
        assert_eq!(f.backups(), 0);
        assert!(!custom_pattern_path(&f.paths, "muse").exists());
    }
}

#[test]
fn list_json_redacts_data_without_rewriting_json_structural_characters() {
    for token in ["\"", "\n", "claude"] {
        let body = format!("token: {}\ncustom_providers:\n  - id: muse\n    label: {}\n    command: fake-cli\n", serde_json::to_string(token).unwrap(), serde_json::to_string(&format!("Label {token}")).unwrap());
        let f = Fixture::new(Some(&body));
        let output = f.run(&["list", "--json"]).unwrap();
        let rows: Vec<Value> = serde_json::from_str(&output).unwrap();
        let custom = rows.iter().find(|row| row["id"] == "muse").unwrap();
        assert_eq!(custom["label"], "Label [redacted]");
        for line in f.run(&["list"]).unwrap().lines() {
            let _: Value = serde_json::from_str(line).unwrap();
        }
    }
}
