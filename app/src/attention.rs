//! The taskbar button flashes when Kompitum isn't in front and something needs the Owner: a
//! blocked agent, or the lead asking. Quiet while another app fills the screen holds it back,
//! like it holds sounds and passing notes.

use gpui::Window;

/// Whether an alert should flash now: something needs the person, the window isn't the active
/// one, and no other app fills the screen.
pub fn should_flash(blocked: bool, window_active: bool, quiet: bool) -> bool {
    blocked && !window_active && !quiet
}

/// Flashes the window's taskbar button until it comes to the foreground. Nothing on systems
/// without a taskbar button.
pub fn request(window: &Window) {
    #[cfg(test)]
    {
        let _ = window;
        FLASHES.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    #[cfg(not(test))]
    flash(window);
}

#[cfg(test)]
static FLASHES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// How many times the taskbar button was asked to flash; a test-only observation point.
#[cfg(test)]
pub fn flashes() -> usize {
    FLASHES.load(std::sync::atomic::Ordering::SeqCst)
}

/// Starts counting flashes again.
#[cfg(test)]
pub fn reset_flashes() {
    FLASHES.store(0, std::sync::atomic::Ordering::SeqCst);
}

#[cfg(all(windows, not(test)))]
fn flash(window: &Window) {
    use raw_window_handle::RawWindowHandle;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FLASHW_TIMERNOFG, FLASHW_TRAY, FLASHWINFO, FlashWindowEx,
    };

    // The trait's method: GPUI's own `window_handle` names the window within the app.
    let Ok(handle) = raw_window_handle::HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::Win32(win) = handle.as_raw() else {
        return;
    };
    let info = FLASHWINFO {
        cbSize: std::mem::size_of::<FLASHWINFO>() as u32,
        hwnd: win.hwnd.get() as *mut std::ffi::c_void,
        // Flash the button and keep going until the window is in front.
        dwFlags: FLASHW_TRAY | FLASHW_TIMERNOFG,
        uCount: 0,
        dwTimeout: 0,
    };
    // SAFETY: the window is live for the call, and `info` has the size and handle it names.
    unsafe {
        FlashWindowEx(&info);
    }
}

#[cfg(all(not(windows), not(test)))]
fn flash(_window: &Window) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_blocked_alert_of_an_inactive_window_away_from_a_fullscreen_app_flashes() {
        for blocked in [false, true] {
            for active in [false, true] {
                for quiet in [false, true] {
                    let expected = blocked && !active && !quiet;
                    assert_eq!(
                        should_flash(blocked, active, quiet),
                        expected,
                        "blocked {blocked}, active {active}, quiet {quiet}"
                    );
                }
            }
        }
    }
}
