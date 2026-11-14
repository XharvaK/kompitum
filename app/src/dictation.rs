//! Voice input through the system's own dictation. On Windows that is voice typing, which Win+H opens for the
//! focused text box; on macOS it is the system's Dictation, started through AppKit's first responder. The
//! words land in the focused text box like typed ones. Linux's voice input is deferred.

#[cfg(target_os = "macos")]
mod macos {
    //! AppKit's half: the `startDictation:` action, sent to the first responder from the application's own
    //! responder chain. macOS opens Dictation for the focused text box, and the words land there like typed
    //! ones. GPUI dispatches commands on the main thread, where AppKit expects this call.

    use std::ffi::{CString, c_char, c_void};

    #[link(name = "objc")]
    unsafe extern "C" {
        fn objc_getClass(name: *const c_char) -> *mut c_void;
        fn sel_registerName(name: *const c_char) -> *mut c_void;
        fn objc_msgSend();
    }

    type Send0 = unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void;
    type SendAction = unsafe extern "C" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        *mut c_void,
        *mut c_void,
    ) -> i8;

    fn class(name: &str) -> *mut c_void {
        let name = CString::new(name).expect("an Objective-C class name has no NUL");
        // SAFETY: `name` is NUL-terminated and names a class loaded by AppKit.
        unsafe { objc_getClass(name.as_ptr()) }
    }

    fn selector(name: &str) -> *mut c_void {
        let name = CString::new(name).expect("an Objective-C selector has no NUL");
        // SAFETY: `name` is NUL-terminated.
        unsafe { sel_registerName(name.as_ptr()) }
    }

    /// Sends `startDictation:` to the first responder, as the menu's Start Dictation item does.
    pub fn start_dictation() -> Result<(), String> {
        let symbol: unsafe extern "C" fn() = objc_msgSend;
        // SAFETY: objc_msgSend's ABI is the platform's small-object method call; each cast matches the method.
        let send0 = unsafe { std::mem::transmute::<unsafe extern "C" fn(), Send0>(symbol) };
        let send_action =
            unsafe { std::mem::transmute::<unsafe extern "C" fn(), SendAction>(symbol) };
        let application = class("NSApplication");
        if application.is_null() {
            return Err("AppKit is not loaded".to_owned());
        }
        // SAFETY: sharedApplication takes no arguments; sendAction:to:from: takes the action, a nil target
        // (the first responder) and a nil sender.
        let app = unsafe { send0(application, selector("sharedApplication")) };
        if app.is_null() {
            return Err("there is no running application".to_owned());
        }
        unsafe {
            send_action(
                app,
                selector("sendAction:to:from:"),
                selector("startDictation:"),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
        }
        Ok(())
    }
}
#[cfg(target_os = "windows")]
mod windows;

/// Open the system's dictation for the focused text box, or say why it can't.
#[cfg(target_os = "windows")]
pub fn start() -> Result<(), String> {
    windows::press_win_h()
}

/// Open the system's dictation for the focused text box, or say why it can't.
#[cfg(target_os = "macos")]
pub fn start() -> Result<(), String> {
    macos::start_dictation()
}

/// Open the system's dictation for the focused text box, or say why it can't.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn start() -> Result<(), String> {
    Err("voice input isn't available on this system yet".to_owned())
}

/// Whether the system has dictation worth a mic, by the platform `window()` names.
pub fn available(platform: &str) -> bool {
    matches!(platform, "windows" | "macos")
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_mic_is_offered_where_dictation_works() {
        assert!(super::available("windows"));
        assert!(super::available("macos"));
        assert!(!super::available("linux"));
    }
}
