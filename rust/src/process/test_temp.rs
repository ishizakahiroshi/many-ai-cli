//! A temporary folder for tests that start a process inside it.
//!
//! Windows keeps a process's executable, working folder and inherited log file
//! locked until the process is really gone, which is a moment after it was
//! killed or reaped. `tempfile::TempDir` tries to remove its folder once and
//! ignores a failure, so such a test used to leave its folder behind on a busy
//! machine. This folder waits for the processes to let go, and fails the test
//! if they never do: a folder that cannot be removed means a process outlived
//! the test that started it.
use std::{
    path::Path,
    time::{Duration, Instant},
};

/// Far beyond the moment a killed process needs, on any machine load.
const RELEASE: Duration = Duration::from_secs(30);

pub(crate) struct ProcessTempDir {
    dir: Option<tempfile::TempDir>,
    release: Duration,
}

pub(crate) fn tempdir() -> ProcessTempDir {
    ProcessTempDir {
        dir: Some(tempfile::tempdir().unwrap()),
        release: RELEASE,
    }
}

impl ProcessTempDir {
    pub(crate) fn path(&self) -> &Path {
        self.dir.as_ref().unwrap().path()
    }
}

impl Drop for ProcessTempDir {
    fn drop(&mut self) {
        let Some(dir) = self.dir.take() else {
            return;
        };
        let path = dir.keep();
        let end = Instant::now() + self.release;
        loop {
            match std::fs::remove_dir_all(&path) {
                Ok(()) => return,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
                Err(error) if Instant::now() >= end => {
                    // A failing test already has its own message.
                    if !std::thread::panicking() {
                        panic!(
                            "a process started by this test still holds its temporary folder \
                             after {:?}: {error}",
                            self.release
                        );
                    }
                    return;
                }
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
    }
}

/// Only Windows refuses to remove a file that is held without sharing.
#[cfg(windows)]
#[test]
fn a_folder_that_is_never_let_go_fails_the_test() {
    use std::os::windows::fs::OpenOptionsExt;
    let mut dir = tempdir();
    dir.release = Duration::from_millis(200);
    let path = dir.path().to_owned();
    // Opened without sharing, the way a running executable or an open
    // database is held; a file std opens normally can still be removed.
    let held = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .share_mode(0)
        .open(path.join("held.txt"))
        .unwrap();
    let dropped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || drop(dir)));
    let message = dropped.unwrap_err();
    let message = message.downcast_ref::<String>().unwrap();
    assert!(
        message.contains("still holds its temporary folder"),
        "{message}"
    );
    // This test's own folder is not left behind either.
    drop(held);
    std::fs::remove_dir_all(&path).unwrap();
}
