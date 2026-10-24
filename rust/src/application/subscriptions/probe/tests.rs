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

struct HeldDismiss {
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
impl SubscriptionProbeHooks for HeldDismiss {
    fn dismiss<'a>(&'a self, _: SessionBinding) -> CoreFuture<'a, Result<(), SessionError>> {
        Box::pin(async move {
            self.entered.notify_one();
            self.release.notified().await;
            Ok(())
        })
    }
    fn wrapper_pid(&self, _: SessionBinding) -> Option<u32> {
        None
    }
    fn recover_process(&self, _: u32) -> io::Result<()> {
        panic!("no native recovery in cleanup fixture")
    }
}

#[tokio::test(start_paused = true)]
async fn cleanup_waits_for_dismiss_beyond_work_budget_and_retains_claim() {
    use crate::hub::task_owner::HubTaskOwner;
    let root = tempfile::tempdir().unwrap();
    let installed = tempfile::tempdir().unwrap();
    let paths = RuntimePaths::trial(root.path(), 49468, installed.path()).unwrap();
    let cwd = root.path().join("usage-probe");
    let dir = active_dir(&cwd).unwrap();
    let name = marker_name("p1");
    dir.replace(&name, b"synthetic marker", 0o600).unwrap();
    let manager = Arc::new(ProbeManager::default());
    let cancel = TaskCancellation::default();
    manager.active.lock().unwrap().insert(
        "claude:p1".into(),
        Active {
            record: Record {
                version: 2,
                provider: "claude".into(),
                profile_id: "p1".into(),
                label: "usage-probe-synthetic".into(),
                session_id: 1,
                pid: 0,
                process_identity: String::new(),
                started_at: String::new(),
                cwd: cwd.clone(),
            },
            cancel: cancel.clone(),
        },
    );
    let tasks = HubTaskOwner::new(tokio::runtime::Handle::current());
    let hooks = Arc::new(HeldDismiss {
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    let mut cleanup = Cleanup {
        permit: Some(tasks.effect_permit().unwrap()),
        claim: Some(Claim {
            manager: manager.clone(),
            key: "claude:p1".into(),
            dir,
            name,
        }),
        binding: Arc::new(Mutex::new(Some(SessionBinding {
            session: LiveSessionId(1),
            incarnation: SessionIncarnation(1),
            wrapper: WrapperConnectionId(1),
        }))),
        hooks: hooks.clone(),
        paths,
        profile: root.path().join("profile"),
        cwd: cwd.clone(),
        warning: Arc::new(|_| panic!("unexpected cleanup warning")),
    };
    let waiter = cleanup.schedule().unwrap();
    hooks.entered.notified().await;
    assert!(cancel.token().is_cancelled());
    assert!(
        cleanup.schedule().is_none(),
        "cleanup must be scheduled only once"
    );
    drop(cleanup);
    tokio::time::advance(Duration::from_secs(61)).await;
    assert!(manager.running("claude", "p1"));
    assert!(cwd.join("active").join(marker_name("p1")).exists());
    assert_eq!(tasks.snapshot().effects.running, 1);
    hooks.release.notify_one();
    waiter.wait().await.unwrap();
    assert!(!manager.running("claude", "p1"));
    assert!(!cwd.join("active").join(marker_name("p1")).exists());
    assert_eq!(tasks.snapshot().effects.running, 0);
    eprintln!(
        "synthetic probe cleanup: retained_after_work_budget_ms=61000; released_after_dismiss=true"
    );
}
