//! Scoped native assignment inputs for Windows unit tests only.
//!
//! The callback lives in the calling Tokio task and is explicitly carried into
//! the owned process task. There is no environment switch or mutable global
//! override, and this module is absent from non-test builds.
#![cfg(all(test, windows))]

use std::{future::Future, io, sync::Arc};
use windows_sys::Win32::Foundation::HANDLE;

pub(crate) type Assignment = Arc<dyn Fn(HANDLE, HANDLE) -> io::Result<()> + Send + Sync>;

tokio::task_local! {
    static ASSIGNMENT: Assignment;
}

pub(crate) fn current() -> Option<Assignment> {
    ASSIGNMENT.try_with(Arc::clone).ok()
}

pub(crate) async fn with_job_assignment<F: Future>(assignment: Assignment, future: F) -> F::Output {
    ASSIGNMENT.scope(assignment, future).await
}
