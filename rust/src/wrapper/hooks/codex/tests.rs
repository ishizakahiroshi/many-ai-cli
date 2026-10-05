use super::*;
#[test]
fn pinned_go_27_quote_and_marker_transform_cases() {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct Case {
        name: String,
        content: String,
        exe: String,
    }
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct Golden {
        name: String,
        block: String,
        injected: String,
        removed: String,
    }
    let cases: Vec<Case> = serde_json::from_str(include_str!("cases.json")).unwrap();
    let golden: Vec<Golden> = serde_json::from_str(include_str!("golden.json")).unwrap();
    assert_eq!(cases.len(), 27);
    assert_eq!(cases.len(), golden.len());
    for (case, expected) in cases.iter().zip(golden) {
        assert_eq!(case.name, expected.name);
        let p = HookParams {
            executable: Path::new(&case.exe),
            ..params()
        };
        assert_eq!(block(&p), expected.block, "{} block", case.name);
        assert_eq!(
            inject_content(&case.content, &p),
            expected.injected,
            "{} inject",
            case.name
        );
        assert_eq!(
            remove_content(&case.content),
            expected.removed,
            "{} remove",
            case.name
        );
        assert_eq!(
            inject_bytes(case.content.as_bytes(), &p),
            expected.injected.as_bytes(),
            "{} byte inject",
            case.name
        );
        assert_eq!(
            remove_bytes(case.content.as_bytes()),
            expected.removed.as_bytes(),
            "{} byte remove",
            case.name
        );
    }
}
fn fixture() -> (tempfile::TempDir, CodexStopHooks) {
    let root = tempfile::tempdir().unwrap();
    let runtime = root.path().join("trial");
    std::fs::create_dir(&runtime).unwrap();
    let paths = RuntimePaths::trial(&runtime, 49686, &root.path().join("installed")).unwrap();
    let owner = CodexStopHooks::new(paths, runtime.join("profile/config.toml")).unwrap();
    (root, owner)
}
fn params() -> HookParams<'static> {
    HookParams {
        hub_url: "http://127.0.0.1:49686",
        token: "synthetic_test_token",
        session: 7,
        executable: Path::new("C:/trial/$portable/ai's app/many-ai-cli.exe"),
    }
}
#[test]
fn actual_rmw_preserves_unrelated_bytes_and_updates_literal_dollar_path() {
    let (_root, owner) = fixture();
    assert!(!owner.injected().unwrap());
    assert!(!owner.path.parent().unwrap().exists());
    let dir = owner.directory().unwrap();
    let original = b"model = \"example\"\n# invalid UTF8 retained: \xff\xfe\n";
    dir.create_new("config.toml", original, 0o600).unwrap();
    owner.inject(&params()).unwrap();
    assert!(owner.injected().unwrap());
    let first = read(&dir, "config.toml").unwrap();
    assert!(first.starts_with(original));
    assert!(String::from_utf8_lossy(&first).contains("$portable"));
    let mut p = params();
    p.session = 9;
    owner.inject(&p).unwrap();
    let current = read(&dir, "config.toml").unwrap();
    assert_eq!(
        current
            .windows(START.len())
            .filter(|v| *v == START.as_bytes())
            .count(),
        1
    );
    assert!(String::from_utf8_lossy(&current).contains("--session 9"));
    assert!(!String::from_utf8_lossy(&current).contains("--session 7"));
    owner.remove().unwrap();
    assert_eq!(read(&dir, "config.toml").unwrap(), original);
    assert!(!owner.injected().unwrap());
    assert!(dir.metadata("config.toml.many-ai-cli.lock").is_err());
}
#[test]
fn live_lock_is_not_stolen_and_dead_owner_can_be_recovered() {
    let (_root, owner) = fixture();
    let dir = owner.directory().unwrap();
    dir.create_new(
        "config.toml.many-ai-cli.lock",
        format!("{}\n", std::process::id()).as_bytes(),
        0o600,
    )
    .unwrap();
    assert!(!steal(&dir).unwrap());
    dir.remove_file("config.toml.many-ai-cli.lock").unwrap();
    dir.create_new("config.toml.many-ai-cli.lock", b"2147483647\n", 0o600)
        .unwrap();
    assert!(!pid_alive(2147483647));
    assert!(steal(&dir).unwrap());
    owner.inject(&params()).unwrap();
    assert!(owner.injected().unwrap());
}
#[test]
fn current_malformed_marker_keeps_original_and_legacy_block_removed() {
    let p = params();
    let malformed = format!("before\n{START}\nwithout end");
    assert_eq!(inject_content(&malformed, &p), malformed);
    assert_eq!(remove_content(&malformed), malformed);
    let legacy = format!("before\n{OLD_START}\nold secret\n{OLD_END}\nafter");
    let content = inject_content(&legacy, &p);
    assert!(!content.contains("old secret"));
    assert!(!content.contains(OLD_START));
    assert!(content.contains("beforeafter"));
}
#[test]
fn trial_rejects_foreign_selected_home() {
    let (root, owner) = fixture();
    let foreign = root.path().join("foreign/config.toml");
    assert!(CodexStopHooks::new(owner.paths.clone(), foreign).is_err());
}
#[cfg(unix)]
#[test]
fn stricter_owner_mode_survives_atomic_replacement() {
    use std::os::unix::fs::PermissionsExt;
    let (_root, owner) = fixture();
    let dir = owner.directory().unwrap();
    dir.create_new("config.toml", b"model = \"custom\"\n", 0o400)
        .unwrap();
    owner.inject(&params()).unwrap();
    assert_eq!(
        dir.metadata("config.toml").unwrap().permissions().mode() & 0o777,
        0o400
    );
    owner.remove().unwrap();
    assert_eq!(
        dir.metadata("config.toml").unwrap().permissions().mode() & 0o777,
        0o400
    );
}
