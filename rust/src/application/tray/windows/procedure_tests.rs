//! CLI-N08 exercises the real procedure and TrayOwner with synthetic window data.
//! No HWND, icon, browser, Hub, OS message sender or native dialog is created.
#![allow(non_snake_case)]
use super::*;
use std::{cell::RefCell, marker::PhantomData, rc::Rc};
use windows_sys::Win32::UI::WindowsAndMessaging as native;

#[derive(Clone, Debug, PartialEq, Eq)]
enum NativeCall {
    Destroy,
    Quit(i32),
    Default(u32),
}
struct WindowData {
    window: usize,
    owner: isize,
    calls: Arc<Mutex<Vec<NativeCall>>>,
}
thread_local! {
    static WINDOW: RefCell<Option<WindowData>> = const { RefCell::new(None) };
}
struct WindowOwnerData {
    owner: Arc<TrayOwner>,
}
struct SyntheticWindow {
    _owner: Box<WindowOwnerData>,
    previous: Option<WindowData>,
    calls: Arc<Mutex<Vec<NativeCall>>>,
    // HWND data and its RAII guard must stay on the procedure's one OS thread.
    _same_thread: PhantomData<Rc<()>>,
}
impl SyntheticWindow {
    fn new(owner: Arc<TrayOwner>) -> Self {
        // The production procedure dereferences *const Arc<TrayOwner>, so the
        // Arc value itself needs a stable address, not Arc::as_ptr's TrayOwner.
        let owner = Box::new(WindowOwnerData { owner });
        let calls = Arc::new(Mutex::new(Vec::new()));
        let previous = WINDOW.with(|slot| {
            slot.replace(Some(WindowData {
                window: 1,
                owner: &owner.owner as *const Arc<TrayOwner> as isize,
                calls: calls.clone(),
            }))
        });
        Self {
            _owner: owner,
            previous,
            calls,
            _same_thread: PhantomData,
        }
    }
    fn handle(&self) -> HWND {
        std::ptr::without_provenance_mut(1)
    }
    fn calls(&self) -> Vec<NativeCall> {
        self.calls.lock().unwrap().clone()
    }
}
impl Drop for SyntheticWindow {
    fn drop(&mut self) {
        WINDOW.with(|slot| {
            slot.replace(self.previous.take());
        });
    }
}

// Only the thread-local synthetic window is intercepted. Other test owners
// and other OS threads keep the original native API behavior.
pub(super) unsafe fn GetWindowLongPtrW(window: HWND, index: i32) -> isize {
    if let Some(owner) = WINDOW.with(|slot| {
        slot.borrow()
            .as_ref()
            .filter(|data| data.window == window as usize)
            .map(|data| {
                assert_eq!(index, GWLP_USERDATA);
                data.owner
            })
    }) {
        owner
    } else {
        unsafe { native::GetWindowLongPtrW(window, index) }
    }
}
pub(super) unsafe fn DestroyWindow(window: HWND) -> i32 {
    if let Some(calls) = window_calls(window) {
        calls.lock().unwrap().push(NativeCall::Destroy);
        1
    } else {
        unsafe { native::DestroyWindow(window) }
    }
}
pub(super) unsafe fn PostQuitMessage(code: i32) {
    if let Some(calls) = WINDOW.with(|slot| slot.borrow().as_ref().map(|data| data.calls.clone())) {
        calls.lock().unwrap().push(NativeCall::Quit(code));
    } else {
        unsafe { native::PostQuitMessage(code) };
    }
}
pub(super) unsafe fn DefWindowProcW(window: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if let Some(calls) = window_calls(window) {
        calls.lock().unwrap().push(NativeCall::Default(message));
        277
    } else {
        unsafe { native::DefWindowProcW(window, message, w, l) }
    }
}
fn window_calls(window: HWND) -> Option<Arc<Mutex<Vec<NativeCall>>>> {
    WINDOW.with(|slot| {
        slot.borrow()
            .as_ref()
            .filter(|data| data.window == window as usize)
            .map(|data| data.calls.clone())
    })
}

#[derive(Default)]
struct Hooks {
    calls: Mutex<Vec<&'static str>>,
}
impl TrayHooks for Hooks {
    fn instance_port(&self) -> Option<u16> {
        Some(49351)
    }
    fn running_url(&self) -> CoreFuture<'_, io::Result<Option<String>>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push("running");
            Ok(Some("http://127.0.0.1:49351/".into()))
        })
    }
    fn start_hub(&self) -> CoreFuture<'_, io::Result<()>> {
        panic!("synthetic message fixture already has its fake running Hub")
    }
    fn open_url<'a>(&'a self, url: &'a str) -> CoreFuture<'a, io::Result<()>> {
        Box::pin(async move {
            assert_eq!(url, "http://127.0.0.1:49351/");
            self.calls.lock().unwrap().push("open");
            Ok(())
        })
    }
    fn stop_hub(&self) -> CoreFuture<'_, io::Result<()>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push("stop");
            Ok(())
        })
    }
}
async fn drain_commands(owner: &TrayOwner) {
    let tasks = std::mem::take(&mut *owner.runs.lock().unwrap());
    for task in tasks {
        task.await.unwrap();
    }
}

#[tokio::test(flavor = "current_thread")]
async fn windows_message_procedure_dispatches_only_the_selected_synthetic_effect() {
    for (message, w, hooks_expected, errors, native_expected) in [
        (OPEN_REQUEST, 0, vec!["running", "open"], 0, vec![]),
        (OPEN_REQUEST, 49351, vec!["running", "open"], 0, vec![]),
        (OPEN_REQUEST, 49352, vec![], 1, vec![]),
        (WM_COMMAND, 1, vec!["running", "open"], 0, vec![]),
        (WM_COMMAND, 2, vec!["running", "stop"], 0, vec![]),
        (WM_COMMAND, 3, vec![], 0, vec![NativeCall::Destroy]),
        (WM_COMMAND, 4, vec![], 1, vec![]),
        (WM_COMMAND, 0xffff, vec![], 0, vec![]),
        (WM_COMMAND, 0x1_0001, vec!["running", "open"], 0, vec![]),
    ] {
        let hooks = Arc::new(Hooks::default());
        let error_messages = Arc::new(Mutex::new(Vec::new()));
        let recorded_errors = error_messages.clone();
        let mut owner = TrayOwner::new(hooks.clone());
        Arc::get_mut(&mut owner).unwrap().test_error_reporter = Some(Arc::new(move |message| {
            recorded_errors.lock().unwrap().push(message.to_owned());
        }));
        let window = SyntheticWindow::new(owner.clone());
        assert_eq!(unsafe { procedure(window.handle(), message, w, 0) }, 0);
        drain_commands(&owner).await;
        assert_eq!(
            *hooks.calls.lock().unwrap(),
            hooks_expected,
            "{message}/{w}"
        );
        assert_eq!(
            error_messages.lock().unwrap().len(),
            errors,
            "{message}/{w}"
        );
        assert_eq!(window.calls(), native_expected, "{message}/{w}");
        assert!(!owner.cancel.is_cancelled());
        owner.shutdown().await;
        // Closing the TrayOwner does not introduce any extra Hub action.
        assert_eq!(*hooks.calls.lock().unwrap(), hooks_expected);
    }

    let hooks = Arc::new(Hooks::default());
    let owner = TrayOwner::new(hooks.clone());
    let window = SyntheticWindow::new(owner.clone());
    assert_eq!(unsafe { procedure(window.handle(), WM_CLOSE, 0, 0) }, 0);
    assert_eq!(window.calls(), [NativeCall::Destroy]);
    // Native DestroyWindow delivery is outside this synthetic test: explicitly
    // inject its following WM_DESTROY to inspect the actual quit-message arm.
    assert_eq!(unsafe { procedure(window.handle(), WM_DESTROY, 0, 0) }, 0);
    assert_eq!(window.calls(), [NativeCall::Destroy, NativeCall::Quit(0)]);
    assert_eq!(unsafe { procedure(window.handle(), WM_NULL, 0, 0) }, 277);
    assert_eq!(
        window.calls(),
        [
            NativeCall::Destroy,
            NativeCall::Quit(0),
            NativeCall::Default(WM_NULL)
        ]
    );
    assert!(hooks.calls.lock().unwrap().is_empty());
    owner.shutdown().await;
}
