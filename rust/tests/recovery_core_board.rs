//! Trial orchestration board mutations stay under the selected synthetic root.
use many_ai_cli::{
    config::{Resource, RuntimePaths},
    orchestration::child_launch::board::BoardStore,
    proto::{core::SessionSnapshot, time::Timestamp},
};
use std::{fs, path::PathBuf};

struct Fixture {
    _root: tempfile::TempDir,
    paths: RuntimePaths,
    outside: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let trial = root.path().join("trial");
        let outside = root.path().join("outside");
        fs::create_dir(&trial).unwrap();
        fs::create_dir(&outside).unwrap();
        let paths = RuntimePaths::trial(&trial, 49680, &root.path().join("installed")).unwrap();
        Self {
            _root: root,
            paths,
            outside,
        }
    }
}

#[test]
fn trial_board_owner_refuses_outside_append_without_modifying_sentinel() {
    let f = Fixture::new();
    let boards = BoardStore::new(&f.paths);
    let sentinel = f.outside.join("board.md");
    fs::write(&sentinel, b"outside synthetic content").unwrap();
    assert!(
        boards
            .append(&sentinel, "hub", "must not escape", Timestamp::UNIX_EPOCH)
            .is_err()
    );
    assert_eq!(fs::read(&sentinel).unwrap(), b"outside synthetic content");
}

#[test]
fn trial_board_owner_refuses_parent_traversal_without_creating_file() {
    let f = Fixture::new();
    let boards = BoardStore::new(&f.paths);
    fs::create_dir_all(f.paths.resource(Resource::Orchestration)).unwrap();
    let relative_escape = f
        .paths
        .resource(Resource::Orchestration)
        .join("../escape.md");
    assert!(
        boards
            .append(
                &relative_escape,
                "hub",
                "must not escape",
                Timestamp::UNIX_EPOCH,
            )
            .is_err()
    );
    assert!(!f.paths.root().join("escape.md").exists());
}

#[cfg(unix)]
#[test]
fn trial_board_creation_and_append_reject_external_directory_and_file_aliases() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let boards = BoardStore::new(&f.paths);
    let root = f.paths.resource(Resource::Orchestration);
    symlink(&f.outside, &root).unwrap();
    assert!(
        boards
            .ensure(
                "alias",
                &SessionSnapshot::default(),
                "task",
                Timestamp::UNIX_EPOCH
            )
            .is_err()
    );
    assert_eq!(fs::read_dir(&f.outside).unwrap().count(), 0);
    fs::remove_file(&root).unwrap();
    fs::create_dir_all(root.join("alias")).unwrap();
    let sentinel = f.outside.join("sentinel.md");
    fs::write(&sentinel, b"outside synthetic content").unwrap();
    let alias = boards.path("alias");
    symlink(&sentinel, &alias).unwrap();
    assert!(
        boards
            .ensure(
                "alias",
                &SessionSnapshot::default(),
                "task",
                Timestamp::UNIX_EPOCH
            )
            .is_err()
    );
    assert!(
        boards
            .append(&alias, "hub", "must not escape", Timestamp::UNIX_EPOCH)
            .is_err()
    );
    assert_eq!(fs::read(&sentinel).unwrap(), b"outside synthetic content");
}

#[cfg(unix)]
#[test]
fn replaced_board_parent_cannot_redirect_a_later_append() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let boards = BoardStore::new(&f.paths);
    let board = boards
        .ensure(
            "owned",
            &SessionSnapshot::default(),
            "task",
            Timestamp::UNIX_EPOCH,
        )
        .unwrap();
    let before = fs::read(&board).unwrap();
    let parent = board.parent().unwrap();
    let moved = parent.with_file_name("moved");
    fs::rename(parent, &moved).unwrap();
    let outside = f.outside.join("board.md");
    fs::write(&outside, b"outside synthetic content").unwrap();
    symlink(&f.outside, parent).unwrap();
    assert!(
        boards
            .append(&board, "hub", "must not escape", Timestamp::UNIX_EPOCH)
            .is_err()
    );
    assert_eq!(fs::read(moved.join("board.md")).unwrap(), before);
    assert_eq!(fs::read(outside).unwrap(), b"outside synthetic content");
}
