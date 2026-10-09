use super::*;
use std::ptr::{null, null_mut};
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, WPARAM},
    System::{Console::FreeConsole, LibraryLoader::GetModuleHandleW, Threading::CreateMutexW},
    UI::{
        Shell::{
            NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
        },
        WindowsAndMessaging::*,
    },
};
const CALLBACK: u32 = WM_USER + 1;
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
unsafe extern "system" fn procedure(window: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        match message {
            CALLBACK if matches!(l as u32, WM_LBUTTONUP | WM_RBUTTONUP) => {
                menu(window);
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
unsafe fn menu(window: HWND) {
    unsafe {
        let pointer = GetWindowLongPtrW(window, GWLP_USERDATA) as *const Arc<TrayOwner>;
        if pointer.is_null() {
            return;
        }
        let owner = &*pointer;
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return;
        }
        let running = owner
            .runtime
            .block_on(owner.hooks.running_url())
            .ok()
            .flatten()
            .is_some();
        AppendMenuW(menu, MF_STRING, 1, wide("Hub を開く").as_ptr());
        AppendMenuW(
            menu,
            MF_STRING | if running { 0 } else { MF_GRAYED },
            2,
            wide("Hub を停止").as_ptr(),
        );
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        AppendMenuW(menu, MF_STRING, 3, wide("終了").as_ptr());
        let mut point = std::mem::zeroed();
        GetCursorPos(&mut point);
        SetForegroundWindow(window);
        TrackPopupMenu(
            menu,
            TPM_LEFTALIGN | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            0,
            window,
            null(),
        );
        PostMessageW(window, WM_NULL, 0, 0);
        DestroyMenu(menu);
    }
}
pub(super) fn run(owner: Arc<TrayOwner>) -> io::Result<()> {
    // This complete function runs on one blocking OS thread: all window calls and
    // message dispatch share the same Win32 queue.
    unsafe {
        FreeConsole();
        let mutex = CreateMutexW(null(), 1, wide("Local\\ManyAICLITray").as_ptr());
        if mutex.is_null() {
            return Err(io::Error::last_os_error());
        }
        let duplicate = GetLastError() == ERROR_ALREADY_EXISTS;
        if duplicate {
            CloseHandle(mutex);
            return owner.runtime.block_on(owner.open_hub());
        }
        let result = window_loop(owner);
        CloseHandle(mutex);
        result
    }
}
unsafe fn window_loop(owner: Arc<TrayOwner>) -> io::Result<()> {
    unsafe {
        let instance = GetModuleHandleW(null());
        let name = wide("ManyAICLITrayWindow");
        let class = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: instance,
            lpszClassName: name.as_ptr(),
            ..std::mem::zeroed()
        };
        if RegisterClassW(&class) == 0 {
            return Err(io::Error::last_os_error());
        }
        let result = (|| {
            let window = CreateWindowExW(
                0,
                name.as_ptr(),
                name.as_ptr(),
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
            let state = Box::new(owner);
            SetWindowLongPtrW(
                window,
                GWLP_USERDATA,
                &*state as *const Arc<TrayOwner> as isize,
            );
            let icon = LoadImageW(
                instance,
                std::ptr::without_provenance::<u16>(1),
                IMAGE_ICON,
                GetSystemMetrics(SM_CXSMICON),
                GetSystemMetrics(SM_CYSMICON),
                LR_DEFAULTCOLOR,
            );
            let own_icon = !icon.is_null();
            let icon = if own_icon {
                icon
            } else {
                LoadIconW(null_mut(), IDI_APPLICATION)
            };
            let mut data: NOTIFYICONDATAW = std::mem::zeroed();
            data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            data.hWnd = window;
            data.uID = 1;
            data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
            data.uCallbackMessage = CALLBACK;
            data.hIcon = icon;
            for (slot, value) in data.szTip.iter_mut().zip(wide("many-ai-cli")) {
                *slot = value;
            }
            if Shell_NotifyIconW(NIM_ADD, &data) == 0 {
                let error = io::Error::last_os_error();
                DestroyWindow(window);
                if own_icon {
                    DestroyIcon(icon);
                }
                return Err(error);
            }
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
            Shell_NotifyIconW(NIM_DELETE, &data);
            // Exit/error paths remove the tray before releasing its private icon.
            if IsWindow(window) != 0 {
                DestroyWindow(window);
            }
            if own_icon {
                DestroyIcon(icon);
            }
            drop(state);
            result
        })();
        UnregisterClassW(name.as_ptr(), instance);
        result
    }
}
