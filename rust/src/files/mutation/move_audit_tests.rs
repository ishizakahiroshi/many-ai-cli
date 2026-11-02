//! FG-D05: observe real exclusive renames and their partial rollback result.
//! The fault hook only changes synthetic filesystem entries between operations.
use super::*;
use std::{cell::RefCell, marker::PhantomData, rc::Rc};

type RenameHook = Box<dyn FnMut(usize, bool)>;
thread_local! {
    static BEFORE_RENAME: RefCell<Option<RenameHook>> = const { RefCell::new(None) };
}
struct HookGuard(PhantomData<Rc<()>>);
impl HookGuard {
    fn install(hook: impl FnMut(usize, bool) + 'static) -> Self {
        BEFORE_RENAME.with(|slot| {
            let mut slot = slot.borrow_mut();
            assert!(
                slot.is_none(),
                "a rename hook already belongs to this test thread"
            );
            *slot = Some(Box::new(hook));
        });
        Self(PhantomData)
    }
}
impl Drop for HookGuard {
    fn drop(&mut self) {
        BEFORE_RENAME.with(|slot| *slot.borrow_mut() = None);
    }
}
pub(super) fn before_rename(index: usize, rollback: bool) {
    BEFORE_RENAME.with(|slot| {
        if let Some(hook) = slot.borrow_mut().as_mut() {
            hook(index, rollback);
        }
    });
}

struct Fixture {
    _root: tempfile::TempDir,
    scope: Scope,
    src1: PathBuf,
    src2: PathBuf,
    dst1: PathBuf,
    dst2: PathBuf,
}
fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("synthetic-project");
    let source = project.join("source");
    let destination = project.join("destination");
    let runtime = root.path().join("runtime");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::create_dir(&destination).unwrap();
    std::fs::create_dir(&runtime).unwrap();
    let paths = crate::config::RuntimePaths::trial(&runtime, 49589, &root.path().join("installed"))
        .unwrap();
    let src1 = source.join("first.txt");
    let src2 = source.join("second.txt");
    std::fs::write(&src1, b"synthetic first source").unwrap();
    std::fs::write(&src2, b"synthetic second source").unwrap();
    Fixture {
        _root: root,
        scope: Scope::new(project, paths),
        src1,
        src2,
        dst1: destination.join("first.txt"),
        dst2: destination.join("second.txt"),
    }
}
fn call(f: &Fixture) -> Response {
    let request = Request {
        path: "/api/files-move".into(),
        method: "POST".into(),
        body: serde_json::to_vec(&json!({
            "srcs": [f.src1, f.src2],
            "dstDir": f.dst1.parent().unwrap(),
        }))
        .unwrap(),
        ..Default::default()
    };
    handle(&request, &f.scope).unwrap_or_else(|response| response)
}

#[test]
fn audit_fg_d05_second_rename_and_rollback_collision_report_remaining_destination() {
    let f = fixture();
    let src1 = f.src1.clone();
    let src2 = f.src2.clone();
    let dst1 = f.dst1.clone();
    let dst2 = f.dst2.clone();
    let steps = Rc::new(RefCell::new(Vec::new()));
    let observed = steps.clone();
    let guard = HookGuard::install(move |index, rollback| {
        observed.borrow_mut().push((index, rollback));
        if index == 1 && !rollback {
            // The first real rename has completed, and both paths were absent
            // during preflight. Create independent collisions before rename #2.
            assert!(!src1.exists());
            assert_eq!(std::fs::read(&dst1).unwrap(), b"synthetic first source");
            assert_eq!(std::fs::read(&src2).unwrap(), b"synthetic second source");
            assert!(!dst2.exists());
            std::fs::write(&src1, b"synthetic rollback collision").unwrap();
            std::fs::write(&dst2, b"synthetic second destination collision").unwrap();
        }
    });
    let response = call(&f);
    drop(guard);
    BEFORE_RENAME.with(|slot| assert!(slot.borrow().is_none()));
    assert_eq!(*steps.borrow(), [(0, false), (1, false), (0, true)]);
    assert_eq!(response.status, 400);
    let value: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(value["ok"], false);
    assert_eq!(value["error"], "rename_failed");
    assert_eq!(value["results"].as_array().unwrap().len(), 2);
    assert_eq!(value["results"][0]["src"], json!(f.src1));
    assert_eq!(value["results"][0]["newAbs"], json!(f.dst1));
    assert!(
        value["results"][0]["error"]
            .as_str()
            .unwrap()
            .contains("rollback failed; moved data remains at")
    );
    assert!(
        value["results"][1]["error"]
            .as_str()
            .unwrap()
            .starts_with("rename failed:")
    );
    assert!(value["results"][1].get("newAbs").is_none());
    assert!(
        value["detail"]
            .as_str()
            .unwrap()
            .contains("rollback failed")
    );
    assert_eq!(
        std::fs::read(&f.src1).unwrap(),
        b"synthetic rollback collision"
    );
    assert_eq!(std::fs::read(&f.dst1).unwrap(), b"synthetic first source");
    assert_eq!(std::fs::read(&f.src2).unwrap(), b"synthetic second source");
    assert_eq!(
        std::fs::read(&f.dst2).unwrap(),
        b"synthetic second destination collision"
    );
    // The same thread performs another complete operation after guard removal.
    // A stale hook would inspect or alter the prior fixture and fail its asserts.
    assert_successful_move(&fixture());
}

fn assert_successful_move(f: &Fixture) {
    let response = call(f);
    assert_eq!(response.status, 200);
    let value: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(value["ok"], true);
    assert_eq!(value["results"][0]["newAbs"], json!(f.dst1));
    assert_eq!(value["results"][1]["newAbs"], json!(f.dst2));
    assert!(!f.src1.exists());
    assert!(!f.src2.exists());
    assert_eq!(std::fs::read(&f.dst1).unwrap(), b"synthetic first source");
    assert_eq!(std::fs::read(&f.dst2).unwrap(), b"synthetic second source");
}

#[test]
fn audit_fg_d05_two_file_move_succeeds_without_collisions() {
    assert_successful_move(&fixture());
}
