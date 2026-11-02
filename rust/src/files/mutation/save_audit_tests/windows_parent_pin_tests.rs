//! Windows files-save transaction interleaving using the existing after-check seam.
use super::*;
use std::{
    fs, io,
    os::windows::io::AsRawHandle,
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, SystemTime},
};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
};

fn identity(path: &Path) -> (u32, u32, u32) {
    let file = fs::File::open(path).unwrap();
    // SAFETY: initialized output-only POD and a currently owned file handle.
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: the handle and valid output pointer stay live for the native call.
    assert_ne!(
        unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) },
        0
    );
    (
        info.dwVolumeSerialNumber,
        info.nFileIndexHigh,
        info.nFileIndexLow,
    )
}
fn refused(operation: &str, result: io::Result<()>) {
    let error = result.expect_err(operation);
    assert!(matches!(error.raw_os_error(), Some(5 | 32)), "{error:?}");
}

#[test]
fn windows_actual_save_refuses_pinned_path_replacement_and_competing_save_then_releases_handles() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    let parent = project.join("selected-parent");
    let moved_parent = project.join("moved-parent");
    let moved_project = root.path().join("moved-project");
    let runtime = root.path().join("runtime");
    let foreign = root.path().join("foreign");
    fs::create_dir_all(&parent).unwrap();
    fs::create_dir(&runtime).unwrap();
    fs::create_dir(&foreign).unwrap();
    let target = parent.join("fixture.txt");
    let moved_target = parent.join("moved-fixture.txt");
    let replacement = foreign.join("replacement.txt");
    let sentinel = foreign.join("sentinel.txt");
    fs::write(&target, b"initial selected bytes").unwrap();
    fs::write(&replacement, b"foreign replacement bytes").unwrap();
    fs::write(&sentinel, b"foreign sentinel").unwrap();
    // Give the original an old explicit timestamp so the post-save stale-base
    // control does not depend on timer or filesystem timestamp granularity.
    fs::OpenOptions::new()
        .write(true)
        .open(&target)
        .unwrap()
        .set_times(
            fs::FileTimes::new()
                .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000)),
        )
        .unwrap();
    let paths = crate::config::RuntimePaths::trial(&runtime, 49124, &root.path().join("installed"))
        .unwrap();
    let scope = Scope::new(project.clone(), paths);
    let base = time::format(fs::metadata(&target).unwrap().modified().unwrap()).unwrap();
    let original_identity = identity(&target);
    let original_mtime = fs::metadata(&target).unwrap().modified().unwrap();
    let request = |content: &str, base: &str| Mutation {
        path: target.to_string_lossy().into_owned(),
        base_mtime: base.into(),
        content: content.into(),
        ..Default::default()
    };
    let entered = AtomicUsize::new(0);
    let saved = save_file_after_check(request("actual first saved bytes", &base), &scope, || {
        entered.fetch_add(1, Ordering::SeqCst);
        // This closure runs after the real save acquired its own parent pin,
        // validated regular-file handle, OS lock and matching base mtime.
        refused(
            "rename parent after actual save check",
            fs::rename(&parent, &moved_parent),
        );
        refused(
            "rename ancestor after actual save check",
            fs::rename(&project, &moved_project),
        );
        refused(
            "rename leaf after actual save check",
            fs::rename(&target, &moved_target),
        );
        refused(
            "replace leaf after actual save check",
            fs::rename(&replacement, &target),
        );
        refused(
            "delete leaf after actual save check",
            fs::remove_file(&target),
        );
        assert_eq!(identity(&target), original_identity);
        assert_eq!(
            fs::metadata(&target).unwrap().modified().unwrap(),
            original_mtime
        );
        assert_eq!(
            fs::read(&replacement).unwrap(),
            b"foreign replacement bytes"
        );
        assert_eq!(fs::read(&sentinel).unwrap(), b"foreign sentinel");
        let competing = save_file(request("competing must not write", &base), &scope)
            .unwrap_or_else(|response| response);
        assert_eq!(competing.status, 409);
        let body: Value = serde_json::from_slice(&competing.body).unwrap();
        assert_eq!(
            body["detail"],
            "file is currently being saved by another process"
        );
        assert_eq!(identity(&target), original_identity);
        assert_eq!(
            fs::metadata(&target).unwrap().modified().unwrap(),
            original_mtime
        );
    })
    .unwrap();
    assert_eq!(saved.status, 200);
    assert_eq!(entered.load(Ordering::SeqCst), 1);
    assert_eq!(fs::read(&target).unwrap(), b"actual first saved bytes");
    assert_eq!(identity(&target), original_identity);
    assert_eq!(fs::read(&sentinel).unwrap(), b"foreign sentinel");
    assert_eq!(
        fs::read(&replacement).unwrap(),
        b"foreign replacement bytes"
    );
    assert!(!moved_parent.exists() && !moved_project.exists() && !moved_target.exists());

    let first_time = fs::metadata(&target).unwrap().modified().unwrap();
    let stale = save_file(request("stale request must not write", &base), &scope)
        .unwrap_or_else(|response| response);
    assert_eq!(stale.status, 409);
    assert_eq!(fs::read(&target).unwrap(), b"actual first saved bytes");
    assert_eq!(
        fs::metadata(&target).unwrap().modified().unwrap(),
        first_time
    );
    let fresh_base = time::format(first_time).unwrap();
    assert_eq!(
        save_file(request("valid fresh save", &fresh_base), &scope)
            .unwrap()
            .status,
        200
    );
    assert_eq!(fs::read(&target).unwrap(), b"valid fresh save");

    // No save capability remains after return. The exact previously denied
    // replacements now succeed, proving denial came from live pins/handles.
    fs::rename(&replacement, &target).unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"foreign replacement bytes");
    assert_ne!(identity(&target), original_identity);
    fs::rename(&parent, &moved_parent).unwrap();
    fs::rename(&project, &moved_project).unwrap();
    assert_eq!(
        fs::read(moved_project.join("moved-parent").join("fixture.txt")).unwrap(),
        b"foreign replacement bytes"
    );
    assert_eq!(fs::read(sentinel).unwrap(), b"foreign sentinel");
}
