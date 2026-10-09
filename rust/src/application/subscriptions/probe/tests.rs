use super::*;
#[test]
fn dialog_targets_cursor_nearest_and_never_guesses() {
    let lines = |rows: &[&str]| rows.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(
        dialog_key(&lines(&["❯ No, exit", "  Yes, I trust this folder"])),
        Some("\x1b[B")
    );
    assert_eq!(
        dialog_key(&lines(&[
            "  Yes, allow external imports",
            "› Disable external imports"
        ])),
        Some("\x1b[A")
    );
    assert_eq!(
        dialog_key(&lines(&[" > Yes, I trust this folder"])),
        Some("\r")
    );
    assert_eq!(dialog_key(&lines(&["Yes, I trust this folder"])), None);
    assert_eq!(
        dialog_key(&lines(&["❯ approve tool command", "enter to confirm"])),
        None
    );
    assert!(!confirm_dialog("enter to confirm tool permission"));
    assert!(confirm_dialog("Allow external CLAUDE.md"));
}
#[test]
fn transcript_cleanup_keeps_three_by_time_then_name_and_exact_project() {
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49276, installed.path()).unwrap();
    let profile = root.path().join("profile");
    let cwd = root.path().join("usage-probe");
    let name =
        crate::hub::preference_media::clean_native_path(&cwd.to_string_lossy(), cfg!(windows))
            .replace(['\\', '/', ':'], "-");
    let target = profile.join("projects").join(name);
    std::fs::create_dir_all(&target).unwrap();
    let other = profile.join("projects").join("user-project");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("keep.jsonl"), b"private synthetic transcript").unwrap();
    let stamp = std::time::UNIX_EPOCH + Duration::from_secs(1000);
    for name in ["a.jsonl", "b.JSONL", "c.jsonl", "d.jsonl", "e.jsonl"] {
        let path = target.join(name);
        std::fs::write(&path, b"synthetic").unwrap();
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(stamp))
            .unwrap();
    }
    std::fs::create_dir(target.join("directory.jsonl")).unwrap();
    std::fs::write(target.join("auth.json"), b"do not read").unwrap();
    assert_eq!(cleanup_transcripts(&paths, &profile, &cwd).unwrap(), 2);
    for name in [
        "c.jsonl",
        "d.jsonl",
        "e.jsonl",
        "auth.json",
        "directory.jsonl",
    ] {
        assert!(target.join(name).exists());
    }
    assert!(!target.join("a.jsonl").exists());
    assert!(!target.join("b.JSONL").exists());
    assert!(other.join("keep.jsonl").exists());
}
