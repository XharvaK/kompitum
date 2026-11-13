//! Voice input through the system's own dictation. On Windows that is voice typing, which Win+H opens for the
//! focused text box; the words land there like typed ones. Other systems have none here yet.

#[cfg(target_os = "windows")]
mod windows;

/// Open the system's dictation for the focused text box, or say why it can't.
#[cfg(target_os = "windows")]
pub fn start() -> Result<(), String> {
    windows::press_win_h()
}

/// Open the system's dictation for the focused text box, or say why it can't.
#[cfg(not(target_os = "windows"))]
pub fn start() -> Result<(), String> {
    Err("voice input works only on Windows for now".to_owned())
}
