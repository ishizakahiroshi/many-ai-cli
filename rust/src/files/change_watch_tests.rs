use super::*;
use std::fs;

#[test]
fn only_paths_that_can_change_the_counts_are_kept() {
    use Kind::*;
    for (origin, path, expected) in [
        (Origin::WorkTree, "src/a.rs", Candidate("src/a.rs".into())),
        (Origin::WorkTree, "src/a.rs/", Candidate("src/a.rs".into())),
        (Origin::WorkTree, ".git/index", Relevant),
        (Origin::WorkTree, ".git/HEAD", Relevant),
        (Origin::WorkTree, ".git/packed-refs", Relevant),
        (Origin::WorkTree, ".git/refs/heads/main", Relevant),
        (Origin::WorkTree, ".git/info/exclude", Excludes),
        (Origin::WorkTree, ".git/index.lock", Skip),
        (Origin::WorkTree, ".git/objects/ab/cdef", Skip),
        (Origin::WorkTree, ".git/logs/HEAD", Skip),
        (Origin::WorkTree, ".git", Skip),
        (Origin::WorkTree, ".gitignore", Excludes),
        (Origin::WorkTree, "web/.gitignore", Excludes),
        (Origin::WorkTree, "", Skip),
        (Origin::GitDir, "index", Relevant),
        (Origin::GitDir, "HEAD", Relevant),
        (Origin::GitDir, "index.lock", Skip),
        (Origin::GitCommon, "refs/heads/side", Relevant),
        (Origin::GitCommon, "packed-refs", Relevant),
        (Origin::GitCommon, "index", Skip),
        (Origin::GitCommon, "HEAD", Skip),
        (Origin::GitCommon, "worktrees/w/index", Skip),
        (Origin::GitFull, "index", Relevant),
        (Origin::GitFull, "refs/tags/v1", Relevant),
        (Origin::GitFull, "objects/pack/x.pack", Skip),
    ] {
        assert_eq!(classify(origin, path), expected, "{origin:?} {path:?}");
    }
}

#[test]
fn an_ignored_directory_decides_everything_below_it() {
    assert_eq!(candidates("a/b/c.txt"), ["a/", "a/b/", "a/b/c.txt"]);
    assert_eq!(candidates("top.txt"), ["top.txt"]);
    let mut excluded = Excluded::default();
    assert_eq!(
        excluded.decide("target/debug/x.o"),
        Decision::Ask(vec![
            "target/".into(),
            "target/debug/".into(),
            "target/debug/x.o".into()
        ])
    );
    let asked: BTreeSet<String> = candidates("target/debug/x.o").into_iter().collect();
    excluded.learn(&asked, &asked.clone());
    assert_eq!(excluded.decide("target/release/y.o"), Decision::Ignored);
    let asked: BTreeSet<String> = ["src/".to_owned(), "src/a.rs".to_owned()].into();
    excluded.learn(&asked, &BTreeSet::new());
    assert_eq!(excluded.decide("src/a.rs"), Decision::Kept);
    // A new file in a known directory asks only about itself.
    assert_eq!(
        excluded.decide("src/b.rs"),
        Decision::Ask(vec!["src/b.rs".into()])
    );
}

#[test]
fn notification_lists_are_read_entry_by_entry() {
    fn entry(name: &str, last: bool) -> Vec<u8> {
        let units: Vec<u16> = name.encode_utf16().collect();
        let mut out = Vec::new();
        let length = units.len() * 2;
        let size = (12 + length).div_ceil(4) * 4;
        out.extend((if last { 0 } else { size } as u32).to_le_bytes());
        out.extend(3u32.to_le_bytes());
        out.extend((length as u32).to_le_bytes());
        for unit in units {
            out.extend(unit.to_le_bytes());
        }
        out.resize(size, 0);
        out
    }
    let raw = Raw::default();
    let mut bytes = entry(r"src\日本語.rs", false);
    bytes.extend(entry(r".git\index", true));
    parse_notifications(&bytes, Origin::WorkTree, &raw);
    let taken = raw.take();
    assert!(!taken.overflow);
    assert_eq!(
        taken.paths,
        BTreeSet::from([
            (Origin::WorkTree, ".git/index".to_owned()),
            (Origin::WorkTree, "src/日本語.rs".to_owned()),
        ])
    );
    // A cut-off list is a lost notification.
    parse_notifications(&bytes[..10], Origin::WorkTree, &raw);
    assert!(raw.take().overflow);
}

/// What the fake watcher hands the test: its notification list and life flag.
type Slot = Arc<Mutex<Option<(Arc<Raw>, Arc<AtomicBool>)>>>;
type Push = Box<dyn Fn(&Raw)>;

/// A watcher that never runs: the test pushes notifications itself.
fn fake(slot: Slot) -> Start {
    Box::new(move |dirs, raw| {
        assert_eq!(dirs[0].origin, Origin::WorkTree);
        let alive = Arc::new(AtomicBool::new(true));
        *lock(&slot) = Some((raw, alive.clone()));
        Ok(Watching {
            alive,
            stop: Box::new(()),
        })
    })
}

fn repo() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("synthetic repo");
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    let cwd = root.join("src").to_string_lossy().into_owned();
    (dir, cwd)
}

#[test]
fn counts_are_reused_until_a_change_that_matters_arrives() {
    let (_dir, cwd) = repo();
    let slot = Arc::new(Mutex::new(None));
    let watch = ChangeWatch::new(Duration::ZERO, SAFETY, fake(slot.clone()));
    let Lease::Count(Some(ticket)) = watch.lease(&cwd) else {
        panic!("a new repository is counted");
    };
    let root = ticket.root.clone();
    watch.store(ticket, (1, 2, 3));
    assert_eq!(watch.lease(&cwd), Lease::Reuse((1, 2, 3)));
    let raw = lock(&slot).as_ref().unwrap().0.clone();

    // Skipped and ignored paths keep the counts.
    raw.push(Origin::WorkTree, ".git/index.lock".into());
    raw.push(Origin::WorkTree, "target/debug/x.o".into());
    let pending = watch.take();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].root(), root.as_path());
    assert_eq!(
        pending[0].asked(),
        ["target/", "target/debug/", "target/debug/x.o"]
    );
    let asked = pending[0].asked.clone();
    watch.learn(pending.into_iter().next().unwrap(), Some(asked));
    assert_eq!(watch.lease(&cwd), Lease::Reuse((1, 2, 3)));
    assert!(watch.due().is_empty());
    // The answer is remembered: no question the second time.
    raw.push(Origin::WorkTree, "target/debug/y.o".into());
    assert!(watch.take().is_empty());
    assert_eq!(watch.lease(&cwd), Lease::Reuse((1, 2, 3)));

    // A source file Git does not ignore changes them.
    raw.push(Origin::WorkTree, "src/a.rs".into());
    let pending = watch.take().into_iter().next().unwrap();
    watch.learn(pending, Some(BTreeSet::new()));
    assert!(matches!(watch.lease(&cwd), Lease::Count(Some(_))));
    assert_eq!(watch.due(), std::slice::from_ref(&root));
    assert!(watch.due().is_empty(), "a change is due once");

    // So do Git's own files, lost notifications, and an unanswered question.
    let pushes: [Push; 2] = [
        Box::new(|raw| raw.push(Origin::WorkTree, ".git/index".into())),
        Box::new(|raw| raw.overflow()),
    ];
    for push in pushes {
        let Lease::Count(Some(ticket)) = watch.lease(&cwd) else {
            panic!("counted");
        };
        watch.store(ticket, (4, 5, 6));
        assert_eq!(watch.lease(&cwd), Lease::Reuse((4, 5, 6)));
        push(&raw);
        assert!(watch.take().is_empty());
        assert!(matches!(watch.lease(&cwd), Lease::Count(Some(_))));
    }
    let Lease::Count(Some(ticket)) = watch.lease(&cwd) else {
        panic!("counted");
    };
    watch.store(ticket, (7, 8, 9));
    raw.push(Origin::WorkTree, "src/b.rs".into());
    let pending = watch.take().into_iter().next().unwrap();
    watch.learn(pending, None);
    assert!(matches!(watch.lease(&cwd), Lease::Count(Some(_))));

    // An exclude file voids the remembered answers.
    raw.push(Origin::WorkTree, ".gitignore".into());
    assert!(watch.take().is_empty());
    raw.push(Origin::WorkTree, "target/debug/z.o".into());
    assert_eq!(watch.take().len(), 1, "the ignore answers were forgotten");
}

#[test]
fn counts_are_taken_again_after_the_safety_interval_or_without_a_watcher() {
    let (_dir, cwd) = repo();
    let slot = Arc::new(Mutex::new(None));
    let watch = ChangeWatch::new(Duration::ZERO, Duration::ZERO, fake(slot.clone()));
    let Lease::Count(Some(ticket)) = watch.lease(&cwd) else {
        panic!("counted");
    };
    watch.store(ticket, (1, 1, 1));
    assert!(matches!(watch.lease(&cwd), Lease::Count(Some(_))));

    let watch = ChangeWatch::new(Duration::ZERO, SAFETY, fake(slot.clone()));
    let Lease::Count(Some(ticket)) = watch.lease(&cwd) else {
        panic!("counted");
    };
    watch.store(ticket, (1, 1, 1));
    // The watcher ended (its directory went away): count every time.
    lock(&slot)
        .as_ref()
        .unwrap()
        .1
        .store(false, Ordering::Relaxed);
    assert_eq!(watch.lease(&cwd), Lease::Count(None));

    let failing = ChangeWatch::new(
        Duration::ZERO,
        SAFETY,
        Box::new(|_, _| Err(io::Error::other("synthetic"))),
    );
    assert_eq!(failing.lease(&cwd), Lease::Count(None));
    let outside = tempfile::tempdir().unwrap();
    assert_eq!(
        watch.lease(&outside.path().to_string_lossy()),
        Lease::Count(None)
    );
}

#[test]
fn a_change_is_due_after_it_settles() {
    let (_dir, cwd) = repo();
    let slot = Arc::new(Mutex::new(None));
    let watch = ChangeWatch::new(Duration::from_millis(80), SAFETY, fake(slot.clone()));
    let _ = watch.lease(&cwd);
    let raw = lock(&slot).as_ref().unwrap().0.clone();
    raw.push(Origin::WorkTree, ".git/HEAD".into());
    watch.take();
    assert!(watch.due().is_empty());
    std::thread::sleep(Duration::from_millis(120));
    assert_eq!(watch.due().len(), 1);
}

#[test]
fn a_linked_worktree_watches_its_own_and_the_common_git_directory() {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("main");
    let own = main.join(".git").join("worktrees").join("w");
    fs::create_dir_all(&own).unwrap();
    fs::write(own.join("commondir"), "../..\n").unwrap();
    let linked = dir.path().join("linked");
    fs::create_dir(&linked).unwrap();
    fs::write(linked.join(".git"), format!("gitdir: {}\n", own.display())).unwrap();
    let dirs = ChangeWatch::dirs(&linked).unwrap();
    let shape: Vec<_> = dirs.iter().map(|d| (d.origin, d.subtree)).collect();
    assert_eq!(
        shape,
        [
            (Origin::WorkTree, true),
            (Origin::GitDir, false),
            (Origin::GitCommon, true)
        ]
    );
    assert_eq!(dirs[1].path, own);
    // A Git directory without `commondir` (a submodule's) is watched whole.
    fs::remove_file(own.join("commondir")).unwrap();
    let dirs = ChangeWatch::dirs(&linked).unwrap();
    assert_eq!(dirs.len(), 2);
    assert_eq!(dirs[1].origin, Origin::GitFull);
    // A repository's own `.git` directory is inside its work tree.
    assert_eq!(ChangeWatch::dirs(&main).unwrap().len(), 1);
}

/// Watching must not stand in the way of removing a worktree (the Hub's own
/// cleanup and `git worktree remove` delete it while sessions may still be
/// listed) or of renaming a folder above it.
#[cfg(windows)]
#[test]
fn windows_watcher_does_not_hold_up_removing_or_renaming_the_repository() {
    let dir = tempfile::tempdir().unwrap();
    let parent = dir.path().join("parent");
    let root = parent.join("synthetic removed repo");
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::create_dir_all(root.join("src").join("deep")).unwrap();
    fs::write(root.join("src").join("deep").join("a.txt"), "x").unwrap();
    let watch = ChangeWatch::new(Duration::ZERO, SAFETY, Box::new(platform::start));
    let cwd = root.to_string_lossy().into_owned();
    assert!(matches!(watch.lease(&cwd), Lease::Count(Some(_))));
    // While watched, a folder above cannot be renamed (measured 2026-10-18:
    // access denied), as with the session's own working directory. Once no
    // live session uses the repository the watcher lets go.
    let renamed = dir.path().join("renamed parent");
    assert!(fs::rename(&parent, &renamed).is_err());
    watch.keep(&BTreeSet::new());
    fs::rename(&parent, &renamed).expect("rename after the watcher let go");
    fs::rename(&renamed, &parent).unwrap();
    // Removing the whole work tree while it is watched.
    assert!(matches!(watch.lease(&cwd), Lease::Count(Some(_))));
    fs::remove_dir_all(&root).expect("remove a watched repository");
    let gone = (0..40).any(|_| {
        std::thread::sleep(Duration::from_millis(50));
        !root.exists()
    });
    assert!(gone, "the removed repository is still there");
    let ended = (0..40).any(|_| {
        std::thread::sleep(Duration::from_millis(50));
        lock(&watch.repos).values().all(|repo| !repo.watched())
    });
    assert!(
        ended,
        "the watcher kept running after its directory was removed"
    );
}

#[cfg(windows)]
async fn settle(watch: &ChangeWatch, service: &crate::files::FilesService) {
    use crate::files::branch::GitBranchSource;
    for pending in watch.take() {
        let asked = pending.asked();
        let ignored = service.ignored(pending.root(), &asked).await;
        watch.learn(pending, ignored);
    }
}

/// The real watcher and real Git: a source edit is seen, build output under
/// an ignored directory and Git's lock file are not.
#[cfg(windows)]
#[tokio::test]
async fn windows_watcher_sees_source_edits_and_not_ignored_output() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("synthetic watched repo");
    fs::create_dir(&root).unwrap();
    let empty = dir.path().join("empty-config");
    fs::write(&empty, b"").unwrap();
    // No commit is needed: check-ignore reads the work tree's .gitignore.
    let out = std::process::Command::new("git")
        .args(["init", "-b", "synthetic"])
        .arg(&root)
        .env("GIT_CONFIG_GLOBAL", &empty)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("tests require Git on PATH");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    fs::write(root.join(".gitignore"), "target/\n").unwrap();
    fs::write(root.join("a.txt"), "one\n").unwrap();
    fs::create_dir_all(root.join("target").join("debug")).unwrap();

    let runtime = dir.path().join("runtime");
    fs::create_dir(&runtime).unwrap();
    let paths =
        crate::config::RuntimePaths::trial(&runtime, 49224, &dir.path().join("installed")).unwrap();
    let mut service = crate::files::FilesService::new(dir.path().into(), paths);
    service.git_executable = "git".into();
    service.git_environment = std::collections::BTreeMap::from([
        (
            "GIT_CONFIG_GLOBAL".into(),
            Some(empty.clone().into_os_string()),
        ),
        ("GIT_CONFIG_NOSYSTEM".into(), Some("1".into())),
    ]);
    let watch = ChangeWatch::new(
        Duration::from_millis(100),
        SAFETY,
        Box::new(platform::start),
    );
    let cwd = root.to_string_lossy().into_owned();
    let Lease::Count(Some(ticket)) = watch.lease(&cwd) else {
        panic!("a watched repository is counted first");
    };
    watch.store(ticket, (0, 0, 0));
    // Let notifications from the setup above arrive and settle.
    tokio::time::sleep(Duration::from_millis(300)).await;
    settle(&watch, &service).await;
    if let Lease::Count(Some(ticket)) = watch.lease(&cwd) {
        watch.store(ticket, (0, 0, 0));
    }

    // Build output and Git's lock file: still reused after they arrive.
    fs::write(root.join("target").join("debug").join("x.o"), b"obj").unwrap();
    fs::write(root.join(".git").join("index.lock"), b"").unwrap();
    fs::remove_file(root.join(".git").join("index.lock")).unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    settle(&watch, &service).await;
    assert_eq!(watch.lease(&cwd), Lease::Reuse((0, 0, 0)));

    // A source edit: counted again, and due once it settles.
    fs::write(root.join("a.txt"), "two\n").unwrap();
    let mut seen = false;
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        settle(&watch, &service).await;
        if matches!(watch.lease(&cwd), Lease::Count(Some(_))) {
            seen = true;
            break;
        }
    }
    assert!(seen, "the watcher did not report the source edit");
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(watch.due().len(), 1);
}
