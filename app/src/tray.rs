//! The notification-area icon (Windows): one hidden window on a `tray` thread
//! owns it, and asks the main window for one thing at a time through
//! `mods::request`. The close decision is pure, so it tests without a window
//! or a real icon.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use crate::settings::SummonKey;

#[cfg(target_os = "windows")]
mod windows;

/// The system the app runs on; tests name another.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum System {
    Macos,
    Windows,
    Linux,
}

/// What the window's X (or Alt+F4) does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseAction {
    /// macOS: hide the app; the Dock brings it back.
    HideApp,
    /// Windows with the tray icon in place: hide the window, leaving the icon.
    HideToTray,
    /// Close Kompitum.
    Close,
}

pub fn system() -> System {
    if cfg!(target_os = "macos") {
        System::Macos
    } else if cfg!(target_os = "windows") {
        System::Windows
    } else {
        System::Linux
    }
}

/// The X hides to the tray only on Windows, and only while its icon is there.
pub fn close_action(system: System, tray: bool) -> CloseAction {
    match system {
        System::Macos => CloseAction::HideApp,
        System::Windows if tray => CloseAction::HideToTray,
        System::Windows | System::Linux => CloseAction::Close,
    }
}

static PRESENT: AtomicBool = AtomicBool::new(false);
static SUMMON_TAKEN: AtomicBool = AtomicBool::new(false);
static SUMMON_KEY: AtomicU8 = AtomicU8::new(0);

fn key_code(key: SummonKey) -> u8 {
    match key {
        SummonKey::CtrlAltK => 0,
        SummonKey::CtrlAltSpace => 1,
        SummonKey::Off => 2,
    }
}

/// The RegisterHotKey modifiers (Control, Alt, no repeat) and virtual key; None for off.
pub fn summon_hotkey(key: SummonKey) -> Option<(u32, u32)> {
    const MOD_ALT: u32 = 0x0001;
    const MOD_CONTROL: u32 = 0x0002;
    const MOD_NOREPEAT: u32 = 0x4000;
    const VK_SPACE: u32 = 0x20;
    const VK_K: u32 = 0x4B;
    let modifiers = MOD_CONTROL | MOD_ALT | MOD_NOREPEAT;
    match key {
        SummonKey::CtrlAltK => Some((modifiers, VK_K)),
        SummonKey::CtrlAltSpace => Some((modifiers, VK_SPACE)),
        SummonKey::Off => None,
    }
}

/// The key the tray registered, or should register now.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn summon_key() -> SummonKey {
    match SUMMON_KEY.load(Ordering::SeqCst) {
        1 => SummonKey::CtrlAltSpace,
        2 => SummonKey::Off,
        _ => SummonKey::CtrlAltK,
    }
}

/// A live settings change: the tray re-registers the key.
pub fn set_summon_key(key: SummonKey) {
    let code = key_code(key);
    if SUMMON_KEY.swap(code, Ordering::SeqCst) != code {
        #[cfg(target_os = "windows")]
        windows::summon_key_changed();
    }
}

/// Whether another app holds the summon key, so Settings can say so.
pub fn summon_taken() -> bool {
    SUMMON_TAKEN.load(Ordering::SeqCst)
}

#[cfg(target_os = "windows")]
fn note_summon_taken(taken: bool) {
    SUMMON_TAKEN.store(taken, Ordering::SeqCst);
}

/// Whether the tray icon is in place.
pub fn present() -> bool {
    PRESENT.load(Ordering::SeqCst)
}

/// Starts the tray once; true when its icon is in place.
pub fn start() -> bool {
    #[cfg(target_os = "windows")]
    {
        static STARTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *STARTED.get_or_init(|| {
            let key = crate::settings::load_person().map(|person| person.summon_key).unwrap_or_default();
            set_summon_key(key);
            windows::start(&class_name(&crate::mods::app_dir()), key)
        })
    }
    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

/// Removes the tray icon and ends its window; harmless without a tray.
pub fn stop() {
    #[cfg(target_os = "windows")]
    windows::stop(&class_name(&crate::mods::app_dir()));
}

/// Hides the main window, leaving it off the taskbar.
pub fn hide(window: &gpui::Window) {
    #[cfg(target_os = "windows")]
    windows::hide(window);
    #[cfg(not(target_os = "windows"))]
    let _ = window;
}

/// Shows the main window where it was and brings it to the front.
pub fn show(window: &gpui::Window) {
    #[cfg(target_os = "windows")]
    windows::show(window);
    #[cfg(not(target_os = "windows"))]
    let _ = window;
}

/// The tray window's class: one per app home, the same on every launch, so
/// quitting finds this home's tray and not another build's.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn class_name(home: &Path) -> String {
    use sha2::Digest;
    let digest = sha2::Sha256::digest(home.to_string_lossy().as_bytes());
    let short: String = digest
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("KompitumTray-{short}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_hides_to_the_tray_only_when_the_icon_is_there() {
        assert_eq!(close_action(System::Macos, false), CloseAction::HideApp);
        assert_eq!(close_action(System::Macos, true), CloseAction::HideApp);
        assert_eq!(close_action(System::Windows, true), CloseAction::HideToTray);
        assert_eq!(close_action(System::Windows, false), CloseAction::Close);
        assert_eq!(close_action(System::Linux, true), CloseAction::Close);
        assert_eq!(close_action(System::Linux, false), CloseAction::Close);
    }

    #[test]
    fn the_summon_key_carries_control_alt_and_the_key_for_each_choice() {
        const ALT_CONTROL_NO_REPEAT: u32 = 0x0001 | 0x0002 | 0x4000;
        assert_eq!(summon_hotkey(SummonKey::CtrlAltK), Some((ALT_CONTROL_NO_REPEAT, 0x4B)));
        assert_eq!(summon_hotkey(SummonKey::CtrlAltSpace), Some((ALT_CONTROL_NO_REPEAT, 0x20)));
        assert_eq!(summon_hotkey(SummonKey::Off), None);
    }

    #[test]
    fn the_tray_class_name_is_one_per_app_home() {
        let home = Path::new("C:/Users/person/.config/kompitum-app");
        let other = Path::new("D:/builds/kompitum-app-dev");
        assert_eq!(class_name(home), class_name(home));
        assert_ne!(class_name(home), class_name(other));
    }
}
