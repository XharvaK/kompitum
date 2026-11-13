//! The Win32 half of voice input: Win+H, sent as four key presses to the focused window. Windows opens voice typing
//! for the focused text box, and a second Win+H closes it.

use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VK_LWIN,
};

const VK_H: u16 = 0x48;

fn key(vk: u16, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: if up { KEYEVENTF_KEYUP } else { 0 },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// Presses Win+H: Windows opens voice typing for the focused text box, or closes it when it is open.
pub fn press_win_h() -> Result<(), String> {
    let inputs = [key(VK_LWIN, false), key(VK_H, false), key(VK_H, true), key(VK_LWIN, true)];
    // SAFETY: `inputs` holds four initialised INPUT values, and the size passed is the size of one of them.
    let sent = unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
    if sent < 4 {
        return Err(format!("Windows took {sent} of the 4 key presses for Win+H"));
    }
    Ok(())
}
