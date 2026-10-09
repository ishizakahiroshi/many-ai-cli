use super::*;
use std::fs;

#[test]
fn transcript_resolution_uses_registered_home_uuid_and_refuses_equal_time_tie() {
    let root = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(
        root.path(),
        49674,
        &root.path().join("../installed-transcript-fixture"),
    )
    .unwrap();
    let cwd = root.path().join("project");
    let vendor = root.path().join("owned-vendor");
    let mut identity = TranscriptSessionIdentity {
        provider: "claude".into(),
        cwd: cwd.to_string_lossy().into_owned(),
        claude_dir: vendor.to_string_lossy().into_owned(),
        started_at: "2026-10-05T00:00:00Z".into(),
        ..Default::default()
    };
    let dir = vendor
        .join("projects")
        .join(identity.cwd.replace(['\\', '/', ':'], "-"));
    fs::create_dir_all(&dir).unwrap();
    let id = "12345678-1234-1234-1234-123456789abc";
    let first = dir.join(format!("{id}.jsonl"));
    fs::write(
        &first,
        serde_json::json!({"cwd":identity.cwd,"timestamp":identity.started_at}).to_string() + "\n",
    )
    .unwrap();
    assert_eq!(
        transcript_path::resolve(&paths, &identity),
        Some(first.clone())
    );
    fs::copy(&first, dir.join("other.jsonl")).unwrap();
    assert!(transcript_path::resolve(&paths, &identity).is_none());
    identity.agent_session_id = id.into();
    assert_eq!(transcript_path::resolve(&paths, &identity), Some(first));
    identity.agent_session_id = "../../escape".into();
    assert!(transcript_path::resolve(&paths, &identity).is_none());
}
#[test]
fn trial_transcript_read_rejects_external_file_and_symlink_alias() {
    let root = tempfile::tempdir().unwrap();
    let trial = root.path().join("trial");
    fs::create_dir(&trial).unwrap();
    let paths = RuntimePaths::trial(&trial, 49673, &root.path().join("installed")).unwrap();
    let external = root.path().join("outside.jsonl");
    fs::write(&external, "synthetic\n").unwrap();
    assert!(transcript_path::check_trial_path(&paths, &external).is_err());
    let inside = trial.join("owned.jsonl");
    fs::write(&inside, "synthetic\n").unwrap();
    assert!(transcript_path::check_trial_path(&paths, &inside).is_ok());
    #[cfg(unix)]
    {
        let alias = trial.join("alias.jsonl");
        std::os::unix::fs::symlink(external, &alias).unwrap();
        assert!(transcript_path::check_trial_path(&paths, &alias).is_err());
    }
}

#[test]
fn trial_discovery_thread_follow_and_parser_refuse_outside_directory_alias() {
    use crate::application::session_observations::subagents::open_artifact;
    let temp = tempfile::tempdir().unwrap();
    let trial = temp.path().join("trial");
    let outside = temp.path().join("outside");
    fs::create_dir(&trial).unwrap();
    fs::create_dir(&outside).unwrap();
    let paths = RuntimePaths::trial(&trial, 49675, &temp.path().join("installed")).unwrap();
    let cwd = trial.join("project");
    let id = "12345678-1234-1234-1234-123456789abc";
    let project = outside
        .join("projects")
        .join(cwd.to_string_lossy().replace(['\\', '/', ':'], "-"));
    fs::create_dir_all(&project).unwrap();
    let file = project.join(format!("{id}.jsonl"));
    let content =
        serde_json::json!({"cwd":cwd,"timestamp":"2026-10-05T00:00:00Z"}).to_string() + "\n";
    fs::write(&file, &content).unwrap();
    let alias = trial.join("vendor-alias");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, &alias).unwrap();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let status = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(&alias)
            .arg(&outside)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW)
            .status()
            .unwrap();
        assert!(
            status.success(),
            "synthetic fixture junction creation failed"
        );
    }
    let identity = TranscriptSessionIdentity {
        provider: "claude".into(),
        cwd: cwd.to_string_lossy().into(),
        claude_dir: alias.to_string_lossy().into(),
        agent_session_id: id.into(),
        started_at: "2026-10-05T00:00:00Z".into(),
        ..Default::default()
    };
    let linked = alias
        .join("projects")
        .join(cwd.to_string_lossy().replace(['\\', '/', ':'], "-"))
        .join(format!("{id}.jsonl"));
    assert!(transcript_path::resolve(&paths, &identity).is_none());
    assert!(open_artifact(&paths, &linked).is_err());
    let codex = TranscriptSessionIdentity {
        provider: "codex".into(),
        native_log_path: linked.to_string_lossy().into(),
        codex_home: alias.to_string_lossy().into(),
        ..identity
    };
    assert!(transcript_path::resolve(&paths, &codex).is_none());
    assert_eq!(
        transcript_path::thread_switch(
            &paths,
            &codex,
            &linked,
            &[],
            parse_rfc3339("2026-10-05T01:00:00Z").unwrap()
        ),
        (None, false)
    );
    assert_eq!(fs::read_to_string(&file).unwrap(), content);
}
#[test]
fn codex_thread_follow_rejects_subagent_and_parallel_peer_then_selects_latest_user_thread() {
    use chrono::{Datelike, Local};
    let root = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(
        root.path(),
        49674,
        &root.path().join("../installed-transcript-fixture"),
    )
    .unwrap();
    let start = parse_rfc3339("2026-10-05T00:00:00Z").unwrap();
    let day = crate::proto::time::utc(start)
        .unwrap()
        .with_timezone(&Local);
    let dir = root
        .path()
        .join("sessions")
        .join(format!("{:04}", day.year()))
        .join(format!("{:02}", day.month()))
        .join(format!("{:02}", day.day()));
    fs::create_dir_all(&dir).unwrap();
    let write = |name: &str, stamp: &str, parent: &str| {
        let path = dir.join(name);
        fs::write(&path,serde_json::json!({"type":"session_meta","payload":{"cwd":"synthetic-cwd","timestamp":stamp,"parent_thread_id":parent,"thread_source":"user"}}).to_string()+"\n").unwrap();
        let modified = parse_rfc3339(stamp)
            .unwrap()
            .to_system_time_exact()
            .unwrap();
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(modified)
            .unwrap();
        path
    };
    let current = write("current.jsonl", "2026-10-05T00:00:00Z", "");
    let identity = TranscriptSessionIdentity {
        provider: "codex".into(),
        cwd: "synthetic-cwd".into(),
        codex_home: root.path().to_string_lossy().into_owned(),
        started_at: "2026-10-05T00:00:00Z".into(),
        native_log_path: current.to_string_lossy().into_owned(),
        ..Default::default()
    };
    write("subagent.jsonl", "2026-10-05T01:00:00Z", "parent");
    let now = parse_rfc3339("2026-10-05T03:00:00Z").unwrap();
    assert_eq!(
        transcript_path::thread_switch(&paths, &identity, &current, &[], now),
        (None, false)
    );
    let next = write("next.jsonl", "2026-10-05T02:00:00Z", "");
    let peer = TranscriptSessionIdentity {
        provider: "codex".into(),
        cwd: identity.cwd.clone(),
        codex_home: identity.codex_home.clone(),
        started_at: "2026-10-04T00:00:00Z".into(),
        ..Default::default()
    };
    assert_eq!(
        transcript_path::thread_switch(&paths, &identity, &current, &[peer], now),
        (None, true)
    );
    assert_eq!(
        transcript_path::thread_switch(&paths, &identity, &current, &[], now),
        (Some(next), false)
    );
    fs::File::options()
        .write(true)
        .open(&current)
        .unwrap()
        .set_modified(now.to_system_time_exact().unwrap())
        .unwrap();
    assert_eq!(
        transcript_path::thread_switch(&paths, &identity, &current, &[], now),
        (None, false)
    );
}
