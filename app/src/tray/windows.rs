//! The Win32 half of the tray: one hidden window on the `tray` thread, its
//! icon and its menu. Nothing here draws; the icon is the program file's own.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    ExtractIconExW, NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE,
    NIM_SETVERSION, NIN_SELECT, NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu,
    DestroyWindow, DispatchMessageW, FindWindowW, GetCursorPos, GetMessageW, HICON, IsIconic,
    MF_SEPARATOR, MF_STRING, MSG, PostMessageW, PostQuitMessage, RegisterClassW,
    RegisterWindowMessageW, SW_HIDE, SW_RESTORE, SW_SHOW, SendMessageW, SetForegroundWindow,
    ShowWindow, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage,
    WM_APP, WM_CLOSE, WM_CONTEXTMENU, WM_DESTROY, WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_NULL,
    WM_RBUTTONUP, WNDCLASSW, WS_OVERLAPPED,
};

const WM_TRAY_CALLBACK: u32 = WM_APP + 1;
const NIN_KEYSELECT: u32 = NIN_SELECT + 1;
const OPEN: usize = 1;
const EXIT: usize = 2;

static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

struct Tray {
    hwnd: HWND,
    icon: HICON,
    added: bool,
}

thread_local! {
    static TRAY: RefCell<Option<Tray>> = const { RefCell::new(None) };
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

pub(super) fn start(class: &str) -> bool {
    let (ready, answer) = std::sync::mpsc::channel();
    let class = class.to_owned();
    match std::thread::Builder::new()
        .name("tray".into())
        .spawn(move || run(&class, ready))
    {
        Ok(_) => answer.recv_timeout(Duration::from_secs(2)).unwrap_or(false),
        Err(error) => {
            tracing::warn!("cannot start the tray: {error}");
            false
        }
    }
}

fn run(class: &str, ready: std::sync::mpsc::Sender<bool>) {
    let taskbar = unsafe { RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()) };
    TASKBAR_CREATED.store(taskbar, Ordering::SeqCst);
    let instance: HINSTANCE = unsafe { GetModuleHandleW(std::ptr::null()) };
    let class = wide(class);
    let title = wide("Kompitum");
    let window_class = WNDCLASSW {
        style: 0,
        lpfnWndProc: Some(wndproc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: instance,
        hIcon: std::ptr::null_mut(),
        hCursor: std::ptr::null_mut(),
        hbrBackground: std::ptr::null_mut(),
        lpszMenuName: std::ptr::null(),
        lpszClassName: class.as_ptr(),
    };
    if unsafe { RegisterClassW(&window_class) } == 0 {
        tracing::warn!(
            "cannot register the tray window: {}",
            std::io::Error::last_os_error()
        );
        let _ = ready.send(false);
        return;
    }
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        )
    };
    if hwnd.is_null() {
        tracing::warn!(
            "cannot create the tray window: {}",
            std::io::Error::last_os_error()
        );
        let _ = ready.send(false);
        return;
    }
    let icon = program_icon();
    if icon.is_null() {
        tracing::warn!("cannot add the tray icon: the program's icon did not load");
        let _ = ready.send(false);
    } else {
        TRAY.with(|slot| {
            *slot.borrow_mut() = Some(Tray {
                hwnd,
                icon,
                added: false,
            });
        });
        let _ = ready.send(add_icon());
    }

    let mut message = MSG::default();
    loop {
        let result = unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) };
        if result == 0 || result == -1 {
            break;
        }
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    remove_icon();
}

/// The program file's own small icon, without a copy of the `.ico`.
fn program_icon() -> HICON {
    let Ok(program) = std::env::current_exe() else {
        return std::ptr::null_mut();
    };
    let path = wide(&program.to_string_lossy());
    let mut icon: HICON = std::ptr::null_mut();
    let count = unsafe { ExtractIconExW(path.as_ptr(), 0, std::ptr::null_mut(), &mut icon, 1) };
    if count == 0 {
        std::ptr::null_mut()
    } else {
        icon
    }
}

fn notify_data(hwnd: HWND, icon: HICON) -> NOTIFYICONDATAW {
    let mut data = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP,
        uCallbackMessage: WM_TRAY_CALLBACK,
        hIcon: icon,
        ..Default::default()
    };
    for (slot, unit) in data.szTip.iter_mut().zip(wide("Kompitum")) {
        *slot = unit;
    }
    data
}

fn add_icon() -> bool {
    TRAY.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(tray) = slot.as_mut() else {
            return false;
        };
        let data = notify_data(tray.hwnd, tray.icon);
        if unsafe { Shell_NotifyIconW(NIM_ADD, &data) } == 0 {
            tray.added = false;
            super::PRESENT.store(false, Ordering::SeqCst);
            tracing::warn!(
                "cannot add the tray icon: {}",
                std::io::Error::last_os_error()
            );
            return false;
        }
        let mut version = data;
        version.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        unsafe { Shell_NotifyIconW(NIM_SETVERSION, &version) };
        tray.added = true;
        super::PRESENT.store(true, Ordering::SeqCst);
        true
    })
}

fn remove_icon() {
    TRAY.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(tray) = slot.as_mut() else {
            return;
        };
        if tray.added {
            let data = notify_data(tray.hwnd, tray.icon);
            unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
            tray.added = false;
        }
        if !tray.icon.is_null() {
            unsafe { DestroyIcon(tray.icon) };
            tray.icon = std::ptr::null_mut();
        }
        super::PRESENT.store(false, Ordering::SeqCst);
    });
}

unsafe extern "system" fn wndproc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let taskbar = TASKBAR_CREATED.load(Ordering::SeqCst);
    if taskbar != 0 && message == taskbar {
        add_icon();
        return 0;
    }
    match message {
        WM_TRAY_CALLBACK => {
            // NOTIFYICON_VERSION_4: the event is the low word of lParam.
            match (lparam as u32) & 0xFFFF {
                WM_LBUTTONUP | WM_LBUTTONDBLCLK | NIN_SELECT | NIN_KEYSELECT => {
                    crate::mods::request(crate::mods::Request::Show);
                }
                WM_RBUTTONUP | WM_CONTEXTMENU => menu(hwnd),
                _ => {}
            }
            0
        }
        WM_CLOSE => {
            unsafe { DestroyWindow(hwnd) };
            0
        }
        WM_DESTROY => {
            remove_icon();
            unsafe { PostQuitMessage(0) };
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn menu(hwnd: HWND) {
    unsafe {
        let items = CreatePopupMenu();
        if items.is_null() {
            return;
        }
        let open = wide("Open Kompitum");
        let exit = wide("Exit");
        AppendMenuW(items, MF_STRING, OPEN, open.as_ptr());
        AppendMenuW(items, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(items, MF_STRING, EXIT, exit.as_ptr());
        let mut at = POINT::default();
        GetCursorPos(&mut at);
        SetForegroundWindow(hwnd);
        let choice = TrackPopupMenu(
            items,
            TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY,
            at.x,
            at.y,
            0,
            hwnd,
            std::ptr::null(),
        );
        DestroyMenu(items);
        PostMessageW(hwnd, WM_NULL, 0, 0);
        match choice as usize {
            OPEN => crate::mods::request(crate::mods::Request::Show),
            EXIT => crate::mods::request(crate::mods::Request::Quit),
            _ => {}
        }
    }
}

pub(super) fn stop(class: &str) {
    let class = wide(class);
    let hwnd = unsafe { FindWindowW(class.as_ptr(), std::ptr::null()) };
    if hwnd.is_null() {
        return;
    }
    unsafe { SendMessageW(hwnd, WM_CLOSE, 0, 0) };
}

fn hwnd(window: &gpui::Window) -> Option<HWND> {
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as HWND),
        _ => None,
    }
}

pub(super) fn hide(window: &gpui::Window) {
    let Some(hwnd) = hwnd(window) else { return };
    unsafe { ShowWindow(hwnd, SW_HIDE) };
}

pub(super) fn show(window: &gpui::Window) {
    let Some(hwnd) = hwnd(window) else { return };
    unsafe {
        // A window minimized before it was hidden comes back restored.
        ShowWindow(
            hwnd,
            if IsIconic(hwnd) != 0 {
                SW_RESTORE
            } else {
                SW_SHOW
            },
        );
        SetForegroundWindow(hwnd);
    }
}
