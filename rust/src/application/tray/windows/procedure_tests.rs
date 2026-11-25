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
    Minimize,
    Text(String),
}
struct WindowData {
    window: usize,
    status: usize,
    owner: isize,
    stop_answer: i32,
    calls: Arc<Mutex<Vec<NativeCall>>>,
}
thread_local! {
    static WINDOW: RefCell<Option<WindowData>> = const { RefCell::new(None) };
    static POSTS: RefCell<Vec<(isize, bool)>> = const { RefCell::new(Vec::new()) };
    static EXITS: RefCell<Vec<isize>> = const { RefCell::new(Vec::new()) };
}
pub(super) fn record_exit(hwnd: isize) {
    EXITS.with(|slot| slot.borrow_mut().push(hwnd));
}
fn take_exit_posts() -> Vec<isize> {
    EXITS.with(|slot| std::mem::take(&mut *slot.borrow_mut()))
}
pub(super) fn record_presence(hwnd: isize, running: bool) {
    POSTS.with(|slot| slot.borrow_mut().push((hwnd, running)));
}
pub(super) fn take_presence_posts() -> Vec<(isize, bool)> {
    POSTS.with(|slot| std::mem::take(&mut *slot.borrow_mut()))
}
struct WindowOwnerData {
    owner: Arc<TrayOwner>,
}
struct SyntheticWindow {
    _owner: Box<WindowOwnerData>,
    _presence: Option<Box<PresenceWindow>>,
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
                status: 0,
                owner: &owner.owner as *const Arc<TrayOwner> as isize,
                stop_answer: 0,
                calls: calls.clone(),
            }))
        });
        Self {
            _owner: owner,
            _presence: None,
            previous,
            calls,
            _same_thread: PhantomData,
        }
    }
    fn presence(owner: Arc<TrayOwner>, answer: i32) -> Self {
        owner
            .test_exit_answer
            .store(answer == native::IDYES, Ordering::Release);
        let presence = Box::new(presence_window(
            owner,
            std::ptr::without_provenance_mut(3),
            presence_copy(0x11),
        ));
        let calls = Arc::new(Mutex::new(Vec::new()));
        let previous = WINDOW.with(|slot| {
            slot.replace(Some(WindowData {
                window: 2,
                status: 3,
                owner: &*presence as *const PresenceWindow as isize,
                stop_answer: answer,
                calls: calls.clone(),
            }))
        });
        Self {
            _owner: Box::new(WindowOwnerData {
                owner: presence.owner.clone(),
            }),
            _presence: Some(presence),
            previous,
            calls,
            _same_thread: PhantomData,
        }
    }
    fn handle(&self) -> HWND {
        std::ptr::without_provenance_mut(1)
    }
    fn presence_handle(&self) -> HWND {
        std::ptr::without_provenance_mut(2)
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
pub(super) unsafe fn ShowWindow(window: HWND, command: i32) -> i32 {
    if let Some(calls) = window_calls(window) {
        if command == native::SW_MINIMIZE {
            calls.lock().unwrap().push(NativeCall::Minimize);
        }
        1
    } else {
        unsafe { native::ShowWindow(window, command) }
    }
}
pub(super) unsafe fn MessageBoxW(
    window: HWND,
    text: *const u16,
    caption: *const u16,
    kind: u32,
) -> i32 {
    let answer = WINDOW.with(|slot| {
        slot.borrow().as_ref().and_then(|data| {
            if data.window == window as usize {
                Some(if data.stop_answer == 0 {
                    native::IDNO
                } else {
                    data.stop_answer
                })
            } else {
                None
            }
        })
    });
    if let Some(answer) = answer {
        return answer;
    }
    unsafe { native::MessageBoxW(window, text, caption, kind) }
}
pub(super) unsafe fn SetWindowTextW(window: HWND, text: *const u16) -> i32 {
    if let Some(calls) = window_calls(window) {
        calls
            .lock()
            .unwrap()
            .push(NativeCall::Text(wide_to_string(text)));
        1
    } else {
        unsafe { native::SetWindowTextW(window, text) }
    }
}
fn wide_to_string(text: *const u16) -> String {
    if text.is_null() {
        return String::new();
    }
    let mut len = 0;
    unsafe {
        while *text.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(text, len))
    }
}
fn window_calls(window: HWND) -> Option<Arc<Mutex<Vec<NativeCall>>>> {
    WINDOW.with(|slot| {
        slot.borrow().as_ref().and_then(|data| {
            if data.window == window as usize
                || (data.status != 0 && data.status == window as usize)
            {
                Some(data.calls.clone())
            } else {
                None
            }
        })
    })
}

#[derive(Default)]
struct Hooks {
    calls: Mutex<Vec<&'static str>>,
    active: std::sync::atomic::AtomicUsize,
    fail_count: AtomicBool,
    fail_exit: AtomicBool,
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
    fn active_sessions(&self) -> CoreFuture<'_, io::Result<ExitSnapshot>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push("count");
            if self.fail_count.load(Ordering::Acquire) {
                return Err(io::Error::other("synthetic count failure"));
            }
            Ok(ExitSnapshot {
                count: self.active.load(Ordering::Acquire),
                runtime: None,
            })
        })
    }
    fn exit_app(&self, snapshot: ExitSnapshot) -> CoreFuture<'_, io::Result<()>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push(if snapshot.count > 0 {
                "exit-confirmed"
            } else {
                "exit-empty"
            });
            if self.fail_exit.load(Ordering::Acquire) {
                return Err(io::Error::other("synthetic session stop failure"));
            }
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

#[tokio::test(flavor = "current_thread")]
async fn replace_request_migrates_on_the_forwarded_port() {
    struct Hooks {
        calls: Mutex<Vec<&'static str>>,
        port: Mutex<Option<u16>>,
    }
    impl TrayHooks for Hooks {
        fn accept_migration_port(&self, port: u16) {
            *self.port.lock().unwrap() = Some(port);
        }
        fn setup(&self) -> CoreFuture<'_, io::Result<()>> {
            Box::pin(async move {
                self.calls.lock().unwrap().push("setup");
                Ok(())
            })
        }
        fn running_url(&self) -> CoreFuture<'_, io::Result<Option<String>>> {
            Box::pin(async move {
                self.calls.lock().unwrap().push("url");
                Ok(Some("http://127.0.0.1:49001/".into()))
            })
        }
        fn start_hub(&self) -> CoreFuture<'_, io::Result<()>> {
            Box::pin(async move {
                self.calls.lock().unwrap().push("start");
                Ok(())
            })
        }
        fn open_url<'a>(&'a self, url: &'a str) -> CoreFuture<'a, io::Result<()>> {
            Box::pin(async move {
                assert_eq!(url, "http://127.0.0.1:49001/");
                self.calls.lock().unwrap().push("open");
                Ok(())
            })
        }
        fn stop_for_migration(&self) -> CoreFuture<'_, io::Result<()>> {
            Box::pin(async move {
                self.calls.lock().unwrap().push("migrate-stop");
                Ok(())
            })
        }
        fn stop_hub(&self) -> CoreFuture<'_, io::Result<()>> {
            Box::pin(async move {
                self.calls.lock().unwrap().push("menu-stop");
                Ok(())
            })
        }
    }
    let hooks = Arc::new(Hooks {
        calls: Mutex::new(vec![]),
        port: Mutex::new(None),
    });
    let owner = TrayOwner::new(hooks.clone());
    let window = SyntheticWindow::new(owner.clone());
    assert_eq!(
        unsafe { procedure(window.handle(), REPLACE_REQUEST, 49001, 0) },
        0
    );
    drain_commands(&owner).await;
    assert_eq!(*hooks.port.lock().unwrap(), Some(49001));
    assert_eq!(
        *hooks.calls.lock().unwrap(),
        vec!["setup", "migrate-stop", "start", "url", "open"]
    );
    owner.shutdown().await;
}

#[test]
fn presence_copy_follows_the_four_primary_languages() {
    let japanese = presence_copy(0x0411);
    assert_eq!(japanese.running, "動いています");
    assert_eq!(japanese.minimize, "しまう");
    assert_eq!(japanese.stopped, "止まっています");
    assert_eq!(japanese.open, "ブラウザを開く");
    assert_eq!(japanese.stop, "終了");
    assert_eq!(japanese.confirm_title, "many-ai-cli を終了");
    assert_eq!(japanese.cancel, "戻る");
    assert_eq!(japanese.confirm_accept, "すべて終了");
    assert_eq!(exit_heading(&japanese, 3), "実行中のAIが3件あります");
    assert_eq!(
        japanese.confirm_body,
        "すべてのAIの作業を終了し、many-ai-cliを閉じます。"
    );
    let english = presence_copy(0x0409);
    assert_eq!(english.minimize, "Minimize");
    assert_eq!(english.running, "Running");
    assert_eq!(english.stopped, "Stopped");
    assert_eq!(english.open, "Open browser");
    assert_eq!(english.stop, "Exit");
    assert_eq!(english.confirm_title, "Exit many-ai-cli");
    assert_eq!(
        english.confirm_body,
        "All AI sessions will end and many-ai-cli will close."
    );
    let vietnamese = presence_copy(0x042a);
    assert_eq!(vietnamese.minimize, "Thu nhỏ");
    assert_eq!(vietnamese.running, "Đang chạy");
    assert_eq!(vietnamese.stopped, "Đã dừng");
    assert_eq!(vietnamese.open, "Mở trình duyệt");
    assert_eq!(vietnamese.stop, "Thoát");
    assert_eq!(vietnamese.confirm_title, "Thoát many-ai-cli");
    assert_eq!(
        vietnamese.confirm_body,
        "Kết thúc tất cả phiên AI và đóng many-ai-cli."
    );
    let other = presence_copy(0x0407);
    assert_eq!(other.running, japanese.running);
    assert_eq!(other.confirm_body, japanese.confirm_body);
    assert_eq!(presence_origin(None, 16, 360, 160), (100, 100));
    assert_eq!(
        presence_origin(Some((0, 0, 1920, 1080)), 16, 360, 160),
        (1544, 904)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn presence_window_button_and_close_share_confirmed_exit() {
    let _ = take_presence_posts();
    let _ = take_exit_posts();
    let hooks = Arc::new(Hooks::default());
    hooks.active.store(3, Ordering::Release);
    let owner = TrayOwner::new(hooks.clone());
    let window = SyntheticWindow::presence(owner.clone(), native::IDNO);
    let handle = window.presence_handle();
    assert_eq!(unsafe { presence_procedure(handle, WM_COMMAND, 1, 0) }, 0);
    drain_commands(&owner).await;
    assert_eq!(*hooks.calls.lock().unwrap(), ["running", "open"]);
    assert!(window.calls().is_empty());

    hooks.calls.lock().unwrap().clear();
    owner.bind_presence_window(2);
    assert_eq!(unsafe { presence_procedure(handle, WM_COMMAND, 2, 0) }, 0);
    drain_commands(&owner).await;
    assert_eq!(*hooks.calls.lock().unwrap(), ["count"]);
    assert!(take_exit_posts().is_empty());

    drop(window);
    let window = SyntheticWindow::presence(owner.clone(), native::IDYES);
    let handle = window.presence_handle();
    hooks.calls.lock().unwrap().clear();
    assert_eq!(unsafe { presence_procedure(handle, WM_COMMAND, 2, 0) }, 0);
    drain_commands(&owner).await;
    assert_eq!(*hooks.calls.lock().unwrap(), ["count", "exit-confirmed"]);
    assert_eq!(take_exit_posts(), [2]);
    assert!(owner.exiting.load(Ordering::Acquire));

    // The real message loop now exits; a fresh synthetic owner checks titlebar close.
    owner.shutdown().await;
    drop(window);
    let owner = TrayOwner::new(hooks.clone());
    let window = SyntheticWindow::presence(owner.clone(), native::IDYES);
    owner.bind_presence_window(2);
    let handle = window.presence_handle();

    hooks.calls.lock().unwrap().clear();
    assert_eq!(unsafe { presence_procedure(handle, WM_COMMAND, 3, 0) }, 0);
    assert_eq!(unsafe { presence_procedure(handle, WM_CLOSE, 0, 0) }, 0);
    assert_eq!(
        unsafe { presence_procedure(handle, WM_SYSCOMMAND, SC_CLOSE as usize, 0) },
        0
    );
    // A second close while exit is pending is coalesced.
    drain_commands(&owner).await;
    assert_eq!(*hooks.calls.lock().unwrap(), ["count", "exit-confirmed"]);
    assert_eq!(take_exit_posts(), [2]);
    assert_eq!(
        unsafe { presence_procedure(handle, WM_SYSCOMMAND, SC_MINIMIZE as usize, 0) },
        277
    );
    assert_eq!(window.calls(), [NativeCall::Default(WM_SYSCOMMAND)]);
    assert!(!owner.cancel.is_cancelled());

    assert_eq!(
        unsafe { presence_procedure(handle, PRESENCE_STATE, 1, 0) },
        0
    );
    assert_eq!(
        unsafe { presence_procedure(handle, PRESENCE_STATE, 0, 0) },
        0
    );
    assert_eq!(
        window.calls(),
        [
            NativeCall::Default(WM_SYSCOMMAND),
            NativeCall::Text("動いています".into()),
            NativeCall::Text("止まっています".into()),
        ]
    );
    hooks.calls.lock().unwrap().clear();
    assert_eq!(
        unsafe { presence_procedure(handle, WM_COMMAND, MINIMIZE_COMMAND as usize, 0) },
        0
    );
    assert_eq!(window.calls().last(), Some(&NativeCall::Minimize));
    assert!(hooks.calls.lock().unwrap().is_empty());
    owner.shutdown().await;
}

#[test]
fn presence_card_stays_narrow_and_follows_the_theme_inputs() {
    assert!(PRESENCE_WIDTH < 360);
    assert!(PRESENCE_HEIGHT < 160);
    let layout = card_layout(PRESENCE_WIDTH, PRESENCE_HEIGHT, 96);
    assert!(layout.mark.x < layout.title.x);
    assert!(layout.mark.x + layout.mark.w <= layout.title.x);
    assert_eq!(layout.mark.w, layout.mark.h);
    assert!(layout.mark.w < layout.minimize.w);
    assert!(layout.minimize.x > layout.title.x);
    assert_eq!(layout.minimize.y, layout.title.y);
    assert!(layout.open.w > layout.exit.w);
    assert!(layout.open.x + layout.open.w <= layout.exit.x);
    assert!(layout.exit.x + layout.exit.w <= PRESENCE_WIDTH);
    assert!(layout.open.y > layout.status.y);
    let wide = card_layout(PRESENCE_WIDTH * 2, PRESENCE_HEIGHT * 2, 192);
    assert_eq!(wide.mark.w, layout.mark.w * 2);
    assert_eq!(wide.open.w, layout.open.w * 2);
    assert_eq!(wide.exit.w, layout.exit.w * 2);
    let light = palette_for(Some(1), None);
    let missing = palette_for(None, None);
    let dark = palette_for(Some(0), None);
    assert_eq!(light.background, missing.background);
    assert_ne!(light.background, dark.background);
    assert!(color_is_dark(dark.background));
    assert!(!color_is_dark(light.background));
    let contrast = palette_for(
        Some(0),
        Some(SystemColors {
            window: 1,
            text: 2,
            highlight: 3,
            highlight_text: 4,
            face: 5,
            button_text: 6,
            frame: 7,
        }),
    );
    assert_eq!(contrast.background, 1);
    assert_eq!(contrast.title, 2);
    assert_eq!(contrast.primary_fill, 3);
    assert_eq!(contrast.primary_text, 4);
    assert_eq!(contrast.secondary_fill, 5);
    assert_eq!(contrast.quiet_mark, 6);
    assert_eq!(contrast.border, 7);
}

#[tokio::test(flavor = "current_thread")]
async fn presence_empty_exits_without_confirmation_and_failures_keep_window() {
    let _ = take_exit_posts();
    let hooks = Arc::new(Hooks::default());
    let errors = Arc::new(Mutex::new(Vec::new()));
    let recorded = errors.clone();
    let mut owner = TrayOwner::new(hooks.clone());
    Arc::get_mut(&mut owner).unwrap().test_error_reporter = Some(Arc::new(move |message| {
        recorded.lock().unwrap().push(message.to_owned());
    }));
    let window = SyntheticWindow::presence(owner.clone(), native::IDNO);
    let handle = window.presence_handle();
    owner.bind_presence_window(2);
    unsafe {
        presence_procedure(handle, WM_CLOSE, 0, 0);
    }
    drain_commands(&owner).await;
    assert_eq!(*hooks.calls.lock().unwrap(), ["count", "exit-empty"]);
    assert_eq!(take_exit_posts(), [2]);
    assert!(window.calls().is_empty());
    owner.shutdown().await;
    drop(window);

    let mut owner = TrayOwner::new(hooks.clone());
    let recorded = errors.clone();
    Arc::get_mut(&mut owner).unwrap().test_error_reporter = Some(Arc::new(move |message| {
        recorded.lock().unwrap().push(message.to_owned());
    }));
    let window = SyntheticWindow::presence(owner.clone(), native::IDNO);
    owner.bind_presence_window(2);
    let handle = window.presence_handle();

    hooks.fail_count.store(true, Ordering::Release);
    hooks.calls.lock().unwrap().clear();
    unsafe {
        presence_procedure(handle, WM_COMMAND, 2, 0);
    }
    drain_commands(&owner).await;
    assert_eq!(*hooks.calls.lock().unwrap(), ["count"]);
    assert!(take_exit_posts().is_empty());
    hooks.fail_count.store(false, Ordering::Release);
    hooks.fail_exit.store(true, Ordering::Release);
    unsafe {
        presence_procedure(handle, WM_CLOSE, 0, 0);
    }
    drain_commands(&owner).await;
    assert!(take_exit_posts().is_empty());
    assert_eq!(errors.lock().unwrap().len(), 2);
    assert!(!owner.exiting.load(Ordering::Acquire));
    assert!(window.calls().is_empty());
    owner.shutdown().await;
}

#[tokio::test(flavor = "current_thread")]
async fn presence_publish_posts_state_without_a_url_and_skips_a_missing_window() {
    let _ = take_presence_posts();
    let hooks = Arc::new(Hooks::default());
    let owner = TrayOwner::new(hooks.clone());
    owner.command(1);
    drain_commands(&owner).await;
    assert!(take_presence_posts().is_empty());
    assert_eq!(*hooks.calls.lock().unwrap(), ["running", "open"]);

    hooks.calls.lock().unwrap().clear();
    owner.bind_presence_window(2);
    owner.command(2);
    drain_commands(&owner).await;
    assert_eq!(take_presence_posts(), vec![(2, true)]);
    assert_eq!(*hooks.calls.lock().unwrap(), ["running", "stop", "running"]);
    owner.shutdown().await;

    let migrate_errors = Arc::new(Mutex::new(Vec::new()));
    let recorded_migrate = migrate_errors.clone();
    let hooks = Arc::new(Hooks::default());
    let mut owner = TrayOwner::new(hooks.clone());
    Arc::get_mut(&mut owner).unwrap().test_error_reporter = Some(Arc::new(move |message| {
        recorded_migrate.lock().unwrap().push(message.to_owned());
    }));
    owner.bind_presence_window(2);
    owner.command(5);
    drain_commands(&owner).await;
    assert_eq!(take_presence_posts(), vec![(2, true)]);
    assert!(
        !migrate_errors
            .lock()
            .unwrap()
            .iter()
            .any(|message| message.contains("http"))
    );
    owner.shutdown().await;

    let errors = Arc::new(Mutex::new(Vec::new()));
    let recorded = errors.clone();
    let hooks = Arc::new(Hooks::default());
    let mut owner = TrayOwner::new(hooks);
    Arc::get_mut(&mut owner).unwrap().test_error_reporter = Some(Arc::new(move |message| {
        recorded.lock().unwrap().push(message.to_owned());
    }));
    owner.command(4);
    drain_commands(&owner).await;
    assert_eq!(
        errors.lock().unwrap().as_slice(),
        [
            "同じ保存先の小さい窓が別のポートで開いています。その窓のプロセスだけをタスクマネージャーで終了し、開き直してください。Hub は自動では停止しません。"
        ]
    );
    owner.shutdown().await;
}
