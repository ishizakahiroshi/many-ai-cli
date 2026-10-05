use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;
#[test]
fn pinned_go_91_marker_injection_removal_and_byte_preservation_cases() {
    #[derive(Deserialize)]
    struct Case {
        #[serde(rename = "Name")]
        name: String,
        #[serde(rename = "Provider")]
        provider: String,
        #[serde(rename = "Body")]
        body: String,
    }
    #[derive(Deserialize)]
    struct Golden {
        #[serde(rename = "Name")]
        name: String,
        #[serde(rename = "Injected")]
        injected: String,
        #[serde(rename = "Removed")]
        removed: String,
        #[serde(rename = "Stripped")]
        stripped: String,
        #[serde(rename = "Error")]
        error: bool,
    }
    let cases: Vec<Case> = serde_json::from_str(include_str!("cases.json")).unwrap();
    let golden: Vec<Golden> = serde_json::from_str(include_str!("golden.json")).unwrap();
    assert_eq!(cases.len(), 91);
    assert_eq!(cases.len(), golden.len());
    for (case, golden) in cases.into_iter().zip(golden) {
        assert_eq!(case.name, golden.name);
        let body = STANDARD.decode(case.body).unwrap();
        assert_eq!(
            strip_blocks(&body),
            STANDARD.decode(golden.stripped).unwrap(),
            "{}",
            case.name
        );
        let actual = inject_rules_content(&case.provider, &body, RULES.as_bytes());
        assert_eq!(actual.is_err(), golden.error, "{}", case.name);
        if let Ok(actual) = actual {
            assert_eq!(
                actual,
                STANDARD.decode(golden.injected).unwrap(),
                "{}",
                case.name
            );
            assert_eq!(
                remove_rules_content(&case.provider, &actual).unwrap(),
                STANDARD.decode(golden.removed).unwrap(),
                "{}",
                case.name
            );
        }
    }
}
#[test]
fn actual_private_trial_rmw_restores_bytes_and_central_version_is_source_authority() {
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49337, installed.path()).unwrap();
    let home = root.path().join("actor");
    std::fs::create_dir(&home).unwrap();
    let files = InstructionFiles::new(paths, home.clone()).unwrap();
    let target = root.path().join("project/AGENTS.md");
    let original = b"private original\xff\xfe";
    files.write(&target, original, false).unwrap();
    files.inject("codex", &target).unwrap();
    files.delegation("codex", &target, true).unwrap();
    assert!(
        files
            .read(&target)
            .unwrap()
            .windows(START.len())
            .any(|v| v == START.as_bytes())
    );
    files.remove("codex", &target, true).unwrap();
    assert_eq!(files.read(&target).unwrap(), original);
    let central = home.join(".many-ai-cli/approval-rules.md");
    files
        .write(&central, b"<!-- version: 24 -->\nuser central \xff", false)
        .unwrap();
    files.inject("codex", &target).unwrap();
    assert!(
        files
            .read(&target)
            .unwrap()
            .windows(b"user central \xff".len())
            .any(|v| v == b"user central \xff")
    );
    assert!(
        files
            .inject("codex", &installed.path().join("AGENTS.md"))
            .is_err()
    );
    assert!(
        files
            .write(&root.path().join("project/../escaped.md"), b"no", false)
            .is_err()
    );
}
#[test]
fn scanner_stops_on_first_found_marker_before_huge_later_line_and_rejects_prior_huge_line() {
    let mut content = format!("{START}\n").into_bytes();
    content.resize(content.len() + 8 * 1024 * 1024, b'x');
    assert!(inject_rules_content("codex", &content, RULES.as_bytes()).is_ok());
    let mut content = vec![b'x'; 8 * 1024 * 1024];
    content.extend_from_slice(format!("\n{START}\n").as_bytes());
    assert!(inject_rules_content("codex", &content, RULES.as_bytes()).is_err());
}
#[cfg(unix)]
#[test]
fn held_trial_parent_rejects_symlink_instruction_targets() {
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49337, installed.path()).unwrap();
    let home = root.path().join("actor");
    std::fs::create_dir(&home).unwrap();
    let files = InstructionFiles::new(paths, home).unwrap();
    let foreign = installed.path().join("AGENTS.md");
    std::fs::write(&foreign, b"foreign").unwrap();
    std::os::unix::fs::symlink(&foreign, root.path().join("AGENTS.md")).unwrap();
    assert!(
        files
            .inject("codex", &root.path().join("AGENTS.md"))
            .is_err()
    );
    assert_eq!(std::fs::read(&foreign).unwrap(), b"foreign");
}

#[test]
fn oversize_instruction_rmw_rejects_before_mutation_and_preserves_actual_tail() {
    use std::io::{Read, Seek, SeekFrom, Write};
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49337, installed.path()).unwrap();
    let home = root.path().join("actor");
    std::fs::create_dir(&home).unwrap();
    let files = InstructionFiles::new(paths, home).unwrap();
    let target = root.path().join("AGENTS.md");
    let prefix = b"original\n<!-- many-ai-cli:approval-rules -->\n<!-- version: 1 -->\nstale\n<!-- /many-ai-cli:approval-rules -->\n";
    let tail = b"PRIVATE_SYNTHETIC_USER_TAIL_MUST_SURVIVE\n";
    {
        let mut file = std::fs::File::create(&target).unwrap();
        file.write_all(prefix).unwrap();
        let chunk = b"synthetic short instruction line\n".repeat(32_768);
        while file.stream_position().unwrap() <= CAP as u64 {
            file.write_all(&chunk).unwrap();
        }
        file.write_all(tail).unwrap();
    }
    let length = std::fs::metadata(&target).unwrap().len();
    for operation in 0..3 {
        let result = match operation {
            0 => files.inject("codex", &target),
            1 => files.remove("codex", &target, true),
            _ => files.delegation("codex", &target, true),
        };
        assert!(result.is_err());
        let mut held = std::fs::File::open(&target).unwrap();
        assert_eq!(held.metadata().unwrap().len(), length);
        let mut before = vec![0; prefix.len()];
        held.read_exact(&mut before).unwrap();
        assert_eq!(before, prefix);
        held.seek(SeekFrom::End(-(tail.len() as i64))).unwrap();
        let mut after = vec![0; tail.len()];
        held.read_exact(&mut after).unwrap();
        assert_eq!(after, tail);
    }
}

#[cfg(unix)]
#[test]
fn selected_trial_alias_stays_bound_to_canonical_instruction_tree() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("physical");
    let installed = temp.path().join("installed");
    std::fs::create_dir_all(root.join("actor")).unwrap();
    std::fs::create_dir(&installed).unwrap();
    let selected = temp.path().join("selected");
    let unselected = temp.path().join("unselected");
    symlink(&root, &selected).unwrap();
    symlink(&root, &unselected).unwrap();
    let paths = RuntimePaths::trial(&selected, 49337, &installed).unwrap();
    let files = InstructionFiles::new(paths.clone(), selected.join("actor")).unwrap();
    let target = selected.join("project/AGENTS.md");
    files.write(&target, b"original", false).unwrap();
    assert_eq!(
        files.read(&paths.root().join("project/AGENTS.md")).unwrap(),
        b"original"
    );
    assert!(files.read(&unselected.join("project/AGENTS.md")).is_err());
    assert!(InstructionFiles::new(paths, selected.join("actor/../actor")).is_err());
    symlink(root.join("project"), root.join("descendant-alias")).unwrap();
    assert!(
        files
            .read(&selected.join("descendant-alias/AGENTS.md"))
            .is_err()
    );
    symlink(&installed, root.join("outside-alias")).unwrap();
    assert!(
        files
            .write(&selected.join("outside-alias/AGENTS.md"), b"no", false)
            .is_err()
    );
    // An already-selected alias is only a spelling. Replacing it must not
    // redirect a later instruction write away from the validated root.
    std::fs::remove_file(&selected).unwrap();
    symlink(&installed, &selected).unwrap();
    files.inject("codex", &target).unwrap();
    files.remove("codex", &target, false).unwrap();
    assert_eq!(files.read(&target).unwrap(), b"original");
    assert_eq!(
        std::fs::read(root.join("project/AGENTS.md")).unwrap(),
        b"original"
    );
    assert_eq!(std::fs::read_dir(&installed).unwrap().count(), 0);
}
