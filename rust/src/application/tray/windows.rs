use super::*;
use std::ffi::c_void;
use std::ptr::{null, null_mut};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, RECT,
        SetLastError, WPARAM,
    },
    Graphics::Gdi::{COLOR_BTNFACE, CreateFontIndirectW, DeleteObject, GetSysColorBrush, HFONT},
    System::{Console::FreeConsole, LibraryLoader::GetModuleHandleW, Threading::CreateMutexW},
    UI::WindowsAndMessaging::*,
};
#[cfg(test)]
#[path = "windows/procedure_tests.rs"]
mod procedure_tests;
#[cfg(test)]
use procedure_tests::{
    DefWindowProcW, DestroyWindow, GetWindowLongPtrW, MessageBoxW, PostQuitMessage, SetWindowTextW,
    ShowWindow,
};
const OPEN_REQUEST: u32 = WM_APP + 1;
const REPLACE_REQUEST: u32 = WM_APP + 2;
const PRESENCE_STATE: u32 = WM_APP + 3;
const PRESENCE_WIDTH: i32 = 360;
const PRESENCE_HEIGHT: i32 = 160;
const PRESENCE_EDGE: i32 = 16;
const SS_LEFT: u32 = 0;
const SS_NOPREFIX: u32 = 128;
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetUserDefaultUILanguage() -> u16;
}
pub(crate) fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
unsafe extern "system" fn procedure(window: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        match message {
            OPEN_REQUEST => {
                let pointer = GetWindowLongPtrW(window, GWLP_USERDATA) as *const Arc<TrayOwner>;
                if !pointer.is_null() {
                    let owner = &*pointer;
                    let mismatch = w != 0
                        && owner
                            .hooks
                            .instance_port()
                            .is_some_and(|port| usize::from(port) != w);
                    owner.command(if mismatch { 4 } else { 1 });
                }
                0
            }
            REPLACE_REQUEST => {
                let pointer = GetWindowLongPtrW(window, GWLP_USERDATA) as *const Arc<TrayOwner>;
                if !pointer.is_null() {
                    // The sender is the console Hub's port. This owner may have
                    // been started for a different one, and sessions stay on the
                    // port they already use.
                    let port = u16::try_from(w).ok();
                    (*pointer).command_replace(port);
                }
                0
            }
            WM_COMMAND => {
                let pointer = GetWindowLongPtrW(window, GWLP_USERDATA) as *const Arc<TrayOwner>;
                if !pointer.is_null() {
                    match w as u32 & 0xffff {
                        3 => {
                            DestroyWindow(window);
                        }
                        id => {
                            (*pointer).command(id);
                        }
                    }
                }
                0
            }
            WM_CLOSE => {
                DestroyWindow(window);
                0
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(window, message, w, l),
        }
    }
}
struct PresenceCopy {
    running: &'static str,
    stopped: &'static str,
    open: &'static str,
    stop: &'static str,
    confirm_title: &'static str,
    confirm_body: &'static str,
}
struct PresenceWindow {
    owner: Arc<TrayOwner>,
    status: HWND,
    text: PresenceCopy,
}
/// Primary language of the Windows display language.
/// Japanese `0x11`, Vietnamese `0x2a`, English `0x09`. Anything else is Japanese.
fn presence_copy(langid: u16) -> PresenceCopy {
    match langid & 0x3ff {
        0x09 => PresenceCopy {
            running: "Running",
            stopped: "Stopped",
            open: "Open browser",
            stop: "Stop",
            confirm_title: "Stop the Hub",
            confirm_body: "Stop the Hub? Running sessions wait until the Hub comes back.",
        },
        0x2a => PresenceCopy {
            running: "Đang chạy",
            stopped: "Đã dừng",
            open: "Mở trình duyệt",
            stop: "Dừng",
            confirm_title: "Dừng Hub",
            confirm_body: "Dừng Hub? Phiên đang chạy sẽ chờ Hub quay lại.",
        },
        _ => PresenceCopy {
            running: "動いています",
            stopped: "止まっています",
            open: "ブラウザを開く",
            stop: "停止",
            confirm_title: "Hub の停止",
            confirm_body: "Hub を停止しますか？ 実行中のセッションは、Hub が戻るまで待ちます。",
        },
    }
}
fn presence_origin(work: Option<(i32, i32, i32, i32)>) -> (i32, i32) {
    match work {
        Some((_, _, right, bottom)) => (
            right - PRESENCE_EDGE - PRESENCE_WIDTH,
            bottom - PRESENCE_EDGE - PRESENCE_HEIGHT,
        ),
        None => (100, 100),
    }
}
fn current_presence_copy() -> PresenceCopy {
    presence_copy(unsafe { GetUserDefaultUILanguage() })
}
unsafe extern "system" fn presence_procedure(
    window: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
) -> LRESULT {
    unsafe {
        let pointer = GetWindowLongPtrW(window, GWLP_USERDATA) as *const PresenceWindow;
        match message {
            WM_COMMAND => {
                if !pointer.is_null() {
                    let data = &*pointer;
                    match w as u32 & 0xffff {
                        1 => data.owner.command(1),
                        2 => {
                            if confirm_stop(window, data) {
                                data.owner.command(2);
                            }
                        }
                        _ => {}
                    }
                }
                0
            }
            WM_CLOSE => {
                ShowWindow(window, SW_MINIMIZE);
                0
            }
            WM_SYSCOMMAND if (w & 0xFFF0) == SC_CLOSE as usize => {
                ShowWindow(window, SW_MINIMIZE);
                0
            }
            PRESENCE_STATE => {
                if !pointer.is_null() {
                    let data = &*pointer;
                    let label = if w == 0 {
                        data.text.stopped
                    } else {
                        data.text.running
                    };
                    let label = wide(label);
                    SetWindowTextW(data.status, label.as_ptr());
                }
                0
            }
            _ => DefWindowProcW(window, message, w, l),
        }
    }
}
fn confirm_stop(window: HWND, data: &PresenceWindow) -> bool {
    let body = wide(data.text.confirm_body);
    let title = wide(data.text.confirm_title);
    let answer = unsafe {
        MessageBoxW(
            window,
            body.as_ptr(),
            title.as_ptr(),
            MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2,
        )
    };
    answer == IDYES
}
pub(super) fn post_presence_state(hwnd: isize, running: bool) {
    if hwnd == 0 {
        return;
    }
    #[cfg(test)]
    procedure_tests::record_presence(hwnd, running);
    #[cfg(not(test))]
    unsafe {
        PostMessageW(
            std::ptr::without_provenance_mut(hwnd as usize),
            PRESENCE_STATE,
            usize::from(running),
            0,
        );
    }
}
pub(super) fn run(owner: Arc<TrayOwner>, open: bool, replace: bool) -> io::Result<()> {
    // This complete function runs on one blocking OS thread: all window calls and
    // message dispatch share the same Win32 queue.
    unsafe {
        FreeConsole();
        // An older tray has no per-home identity or forwarding protocol. Ask
        // the user to close it instead of starting a conflicting second owner.
        let class = wide("ManyAICLITrayWindow");
        if !FindWindowW(class.as_ptr(), class.as_ptr()).is_null() {
            return Err(io::Error::other(
                "旧版のトレイが起動しています。旧版のメニューからトレイだけを終了し、アイコンを開き直してください。Hub を停止する必要はありません。",
            ));
        }
        let key = owner.hooks.instance_key();
        let mutex = CreateMutexW(null(), 0, wide(&format!("Local\\{key}")).as_ptr());
        if mutex.is_null() {
            return Err(io::Error::last_os_error());
        }
        let duplicate = GetLastError() == ERROR_ALREADY_EXISTS;
        if duplicate {
            let result = if replace {
                forward_replace(&key, owner.hooks.requested_port())
            } else if open {
                notify_existing(&key, owner.hooks.requested_port())
            } else {
                Ok(())
            };
            CloseHandle(mutex);
            return result;
        }
        let result = window_loop(owner, &key, open, replace);
        CloseHandle(mutex);
        result
    }
}
fn notify_existing(key: &str, port: Option<u16>) -> io::Result<()> {
    let class = wide("ManyAICLITrayWindow");
    let title = wide(key);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        // A simultaneous first click may still be creating the first owner's
        // window. Only that owner performs Hub startup/browser actions.
        let window = unsafe { FindWindowW(class.as_ptr(), title.as_ptr()) };
        if !window.is_null() {
            if unsafe { PostMessageW(window, OPEN_REQUEST, port.map(usize::from).unwrap_or(0), 0) }
                != 0
            {
                return Ok(());
            }
            return Err(io::Error::last_os_error());
        }
        if std::time::Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "起動済みの小さい窓へ接続できませんでした。窓の状態を確認して開き直してください。",
            ));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}
/// `tray --replace` against an already-running tray. `port` is the console Hub
/// the new process was given; the owner records command 5 before this returns.
fn forward_replace(key: &str, port: Option<u16>) -> io::Result<()> {
    let class = wide("ManyAICLITrayWindow");
    let title = wide(key);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let window = unsafe { FindWindowW(class.as_ptr(), title.as_ptr()) };
        if !window.is_null() {
            unsafe {
                SetLastError(0);
                let parameter = port.map(usize::from).unwrap_or(0);
                SendMessageW(window, REPLACE_REQUEST, parameter, 0);
            }
            let error = unsafe { GetLastError() };
            return if error == 0 {
                Ok(())
            } else {
                Err(io::Error::from_raw_os_error(error as i32))
            };
        }
        if std::time::Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "起動済みの小さい窓へ接続できませんでした。窓の状態を確認して開き直してください。",
            ));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}
pub(super) fn show_error(message: &str) {
    let message = wide(&format!(
        "操作を完了できませんでした。\n\n{message}\n\n同じ保存先でターミナルから many-ai-cli doctor または serve を実行すると詳細を確認できます。\nこの案内では Hub を停止していません。"
    ));
    unsafe {
        MessageBoxW(
            null_mut(),
            message.as_ptr(),
            wide("many-ai-cli").as_ptr(),
            MB_OK | MB_ICONERROR | MB_SETFOREGROUND | MB_TASKMODAL,
        );
    }
}
struct ShownPresence {
    window: HWND,
    state: Box<PresenceWindow>,
    owned_icons: Vec<HICON>,
    font: HFONT,
}
fn presence_position() -> (i32, i32) {
    let mut area = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    let ok = unsafe {
        SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut area as *mut RECT as *mut c_void, 0)
    };
    presence_origin(if ok != 0 {
        Some((area.left, area.top, area.right, area.bottom))
    } else {
        None
    })
}
fn ui_font() -> HFONT {
    unsafe {
        let mut metrics: NONCLIENTMETRICSW = std::mem::zeroed();
        metrics.cbSize = std::mem::size_of::<NONCLIENTMETRICSW>() as u32;
        if SystemParametersInfoW(
            SPI_GETNONCLIENTMETRICS,
            metrics.cbSize,
            &mut metrics as *mut NONCLIENTMETRICSW as *mut c_void,
            0,
        ) == 0
        {
            return null_mut();
        }
        CreateFontIndirectW(&metrics.lfMessageFont)
    }
}
fn load_owned_icon(instance: HINSTANCE, width: i32, height: i32) -> HICON {
    unsafe {
        LoadImageW(
            instance,
            std::ptr::without_provenance::<u16>(1),
            IMAGE_ICON,
            width,
            height,
            LR_DEFAULTCOLOR,
        )
    }
}
/// Resource icons are owned. A missing resource falls back to the shared
/// application icon and does not fail startup.
fn load_presence_icons(instance: HINSTANCE) -> (Vec<HICON>, HICON, HICON) {
    let big = load_owned_icon(instance, unsafe { GetSystemMetrics(SM_CXICON) }, unsafe {
        GetSystemMetrics(SM_CYICON)
    });
    let small = load_owned_icon(instance, unsafe { GetSystemMetrics(SM_CXSMICON) }, unsafe {
        GetSystemMetrics(SM_CYSMICON)
    });
    if big.is_null() && small.is_null() {
        let shared = unsafe { LoadIconW(null_mut(), IDI_APPLICATION) };
        return (Vec::new(), shared, shared);
    }
    let mut owned = Vec::new();
    if !big.is_null() {
        owned.push(big);
    }
    if !small.is_null() {
        owned.push(small);
    }
    let big = if big.is_null() { small } else { big };
    let small = if small.is_null() { big } else { small };
    (owned, big, small)
}
unsafe fn create_presence(instance: HINSTANCE, owner: Arc<TrayOwner>) -> io::Result<ShownPresence> {
    unsafe {
        let class = wide("ManyAICLIPresenceWindow");
        let (x, y) = presence_position();
        let window = CreateWindowExW(
            WS_EX_APPWINDOW,
            class.as_ptr(),
            wide("MANY-AI-CLI").as_ptr(),
            WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
            x,
            y,
            PRESENCE_WIDTH,
            PRESENCE_HEIGHT,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if window.is_null() {
            return Err(io::Error::last_os_error());
        }
        let text = current_presence_copy();
        let font = ui_font();
        let mut client = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        GetClientRect(window, &mut client);
        let width = (client.right - client.left).max(160);
        let margin = 12;
        let gap = 8;
        let inner = (width - margin * 2).max(80);
        let button_w = (inner - gap) / 2;
        let status = CreateWindowExW(
            0,
            wide("STATIC").as_ptr(),
            wide(text.stopped).as_ptr(),
            WS_CHILD | WS_VISIBLE | SS_LEFT | SS_NOPREFIX,
            margin,
            margin,
            inner,
            24,
            window,
            std::ptr::without_provenance_mut(100),
            instance,
            null(),
        );
        let open = CreateWindowExW(
            0,
            wide("BUTTON").as_ptr(),
            wide(text.open).as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON as u32,
            margin,
            margin + 24 + gap,
            button_w,
            28,
            window,
            std::ptr::without_provenance_mut(1),
            instance,
            null(),
        );
        let stop = CreateWindowExW(
            0,
            wide("BUTTON").as_ptr(),
            wide(text.stop).as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON as u32,
            margin + button_w + gap,
            margin + 24 + gap,
            button_w,
            28,
            window,
            std::ptr::without_provenance_mut(2),
            instance,
            null(),
        );
        if status.is_null() || open.is_null() || stop.is_null() {
            let error = io::Error::last_os_error();
            DestroyWindow(window);
            if !font.is_null() {
                DeleteObject(font);
            }
            return Err(error);
        }
        if !font.is_null() {
            for control in [status, open, stop] {
                SendMessageW(control, WM_SETFONT, font as usize, 1);
            }
        }
        let (owned_icons, big, small) = load_presence_icons(instance);
        if !big.is_null() {
            SendMessageW(window, WM_SETICON, ICON_BIG as usize, big as isize);
        }
        if !small.is_null() {
            SendMessageW(window, WM_SETICON, ICON_SMALL as usize, small as isize);
        }
        let state = Box::new(PresenceWindow {
            owner,
            status,
            text,
        });
        SetWindowLongPtrW(
            window,
            GWLP_USERDATA,
            &*state as *const PresenceWindow as isize,
        );
        ShowWindow(window, SW_SHOWNOACTIVATE);
        Ok(ShownPresence {
            window,
            state,
            owned_icons,
            font,
        })
    }
}
unsafe fn window_loop(
    owner: Arc<TrayOwner>,
    key: &str,
    open: bool,
    replace: bool,
) -> io::Result<()> {
    unsafe {
        let instance = GetModuleHandleW(null());
        let name = wide("ManyAICLITrayWindow");
        let presence_name = wide("ManyAICLIPresenceWindow");
        let class = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: instance,
            lpszClassName: name.as_ptr(),
            ..std::mem::zeroed()
        };
        if RegisterClassW(&class) == 0 {
            return Err(io::Error::last_os_error());
        }
        let presence_class = WNDCLASSW {
            lpfnWndProc: Some(presence_procedure),
            hInstance: instance,
            lpszClassName: presence_name.as_ptr(),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: GetSysColorBrush(COLOR_BTNFACE),
            ..std::mem::zeroed()
        };
        if RegisterClassW(&presence_class) == 0 {
            let error = io::Error::last_os_error();
            UnregisterClassW(name.as_ptr(), instance);
            return Err(error);
        }
        let result = (|| {
            let window = CreateWindowExW(
                0,
                name.as_ptr(),
                wide(key).as_ptr(),
                0,
                0,
                0,
                0,
                0,
                null_mut(),
                null_mut(),
                instance,
                null(),
            );
            if window.is_null() {
                return Err(io::Error::last_os_error());
            }
            let state = Box::new(owner.clone());
            SetWindowLongPtrW(
                window,
                GWLP_USERDATA,
                &*state as *const Arc<TrayOwner> as isize,
            );
            let presence = match create_presence(instance, owner.clone()) {
                Ok(presence) => presence,
                Err(error) => {
                    DestroyWindow(window);
                    return Err(error);
                }
            };
            owner.bind_presence_window(presence.window as isize);
            if replace {
                state.command(5);
            } else if open {
                state.command(1);
            }
            owner.kick_presence_refresh();
            let mut message: MSG = std::mem::zeroed();
            let result = loop {
                let receipt = GetMessageW(&mut message, null_mut(), 0, 0);
                if receipt < 0 {
                    break Err(io::Error::last_os_error());
                }
                if receipt == 0 {
                    break Ok(());
                }
                TranslateMessage(&message);
                DispatchMessageW(&message);
            };
            owner.bind_presence_window(0);
            if IsWindow(presence.window) != 0 {
                DestroyWindow(presence.window);
            }
            if IsWindow(window) != 0 {
                DestroyWindow(window);
            }
            if !presence.font.is_null() {
                DeleteObject(presence.font);
            }
            for icon in presence.owned_icons {
                DestroyIcon(icon);
            }
            drop(presence.state);
            drop(state);
            result
        })();
        UnregisterClassW(presence_name.as_ptr(), instance);
        UnregisterClassW(name.as_ptr(), instance);
        result
    }
}
