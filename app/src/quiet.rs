//! Quiet while another app fills the screen: no sounds, and passing notes
//! wait. The person's `quiet_while_fullscreen` setting turns it off.
//!
//! Windows answers two ways: the shell's own notification state, or a
//! foreground window with no title bar that covers its whole monitor and
//! belongs to neither this process nor the shell (borderless games). A
//! maximized window keeps its title bar, so it never counts. The answer is cached for a second:
//! every sound and every note asks, and the system is not asked twice a frame.

use std::time::{Duration, Instant};

/// A window or monitor rectangle, in screen pixels.
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// Whether `window` covers all of `monitor`. Equal and larger cover it; a
/// window inside it does not.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn covers_monitor(window: Rect, monitor: Rect) -> bool {
    window.left <= monitor.left
        && window.top <= monitor.top
        && window.right >= monitor.right
        && window.bottom >= monitor.bottom
}

const ASK_EVERY: Duration = Duration::from_secs(1);
static CACHE: std::sync::Mutex<Option<(Instant, bool)>> = std::sync::Mutex::new(None);

/// Whether another app fills the screen right now. At most one system query a
/// second; the answer in between is the last one.
pub fn fullscreen_elsewhere() -> bool {
    let now = Instant::now();
    let mut cache = CACHE.lock().unwrap();
    if let Some((at, answer)) = *cache
        && now.duration_since(at) < ASK_EVERY
    {
        return answer;
    }
    let answer = ask();
    *cache = Some((now, answer));
    answer
}

/// The person's setting on, and another app fills the screen.
pub fn quiet() -> bool {
    crate::settings::quiet_while_fullscreen() && fullscreen_elsewhere()
}

#[cfg(windows)]
fn ask() -> bool {
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
    };
    use windows_sys::Win32::UI::Shell::{
        QUNS_BUSY, QUNS_NOT_PRESENT, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN,
        SHQueryUserNotificationState,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_STYLE, GetDesktopWindow, GetForegroundWindow, GetShellWindow, GetWindowLongW,
        GetWindowRect, GetWindowThreadProcessId, WS_CAPTION,
    };

    let mut state = 0;
    if unsafe { SHQueryUserNotificationState(&mut state) } == 0
        && matches!(
            state,
            QUNS_NOT_PRESENT | QUNS_BUSY | QUNS_RUNNING_D3D_FULL_SCREEN | QUNS_PRESENTATION_MODE
        )
    {
        return true;
    }

    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_null()
        || hwnd == unsafe { GetShellWindow() }
        || hwnd == unsafe { GetDesktopWindow() }
    {
        return false;
    }
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    if pid == std::process::id() {
        return false;
    }
    // A window with a title bar is still a window, maximized or not: a borderless game has none.
    let style = unsafe { GetWindowLongW(hwnd, GWL_STYLE) } as u32;
    if style & WS_CAPTION == WS_CAPTION {
        return false;
    }
    let mut window = std::mem::MaybeUninit::<windows_sys::Win32::Foundation::RECT>::uninit();
    if unsafe { GetWindowRect(hwnd, window.as_mut_ptr()) } == 0 {
        return false;
    }
    let window = unsafe { window.assume_init() };
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    if monitor.is_null() {
        return false;
    }
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
        return false;
    }
    covers_monitor(
        Rect {
            left: window.left,
            top: window.top,
            right: window.right,
            bottom: window.bottom,
        },
        Rect {
            left: info.rcMonitor.left,
            top: info.rcMonitor.top,
            right: info.rcMonitor.right,
            bottom: info.rcMonitor.bottom,
        },
    )
}

#[cfg(not(windows))]
fn ask() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
        Rect {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn a_window_covers_its_monitor_only_when_it_reaches_every_edge() {
        let monitor = rect(0, 0, 1920, 1080);
        // Equal: a borderless window sized to the monitor.
        assert!(covers_monitor(rect(0, 0, 1920, 1080), monitor));
        // Larger: borderless games sometimes report beyond the monitor.
        assert!(covers_monitor(rect(-8, -8, 1928, 1088), monitor));
        // Inside: a normal window, even one nearly full.
        assert!(!covers_monitor(rect(10, 10, 1910, 1070), monitor));
        // Reaching only some edges doesn't cover it.
        assert!(!covers_monitor(rect(-8, 10, 1920, 1080), monitor));
    }

    #[cfg(not(windows))]
    #[test]
    fn other_systems_are_never_fullscreen_elsewhere() {
        assert!(!fullscreen_elsewhere());
    }
}
