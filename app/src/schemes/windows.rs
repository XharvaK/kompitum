use std::ffi::c_void;
use std::path::{Path, PathBuf};

use windows_sys::Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS},
    Graphics::Gdi::{
        BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC,
        GetDIBits, GetObjectW, HDC, ReleaseDC,
    },
    System::Registry::{
        HKEY, HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, KEY_READ, REG_EXPAND_SZ, REG_SZ, RegCloseKey,
        RegOpenKeyExW, RegQueryValueExW,
    },
    UI::{
        Shell::ExtractIconExW,
        WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO},
    },
};

use super::{Found, command_program};

pub(super) fn find(scheme: &str, want_icon: bool) -> Result<Option<Found>, String> {
    let Some(program) = scheme_program(scheme)? else {
        return Ok(None);
    };
    let icon = want_icon.then(|| icon_png(&program)).flatten();
    Ok(Some(Found { program, icon }))
}

fn scheme_program(scheme: &str) -> Result<Option<PathBuf>, String> {
    let roots = [
        (HKEY_CURRENT_USER, format!("Software\\Classes\\{scheme}")),
        (HKEY_CLASSES_ROOT, scheme.to_owned()),
    ];
    for (root, path) in roots {
        let Some(key) = open_key(root, &path, &path)? else {
            continue;
        };
        // A key without URL Protocol is someone else's file association.
        if read_string(key.0, Some("URL Protocol"))?.is_none() {
            continue;
        }
        // A user's key without a command falls through to the machine's.
        let command_path = format!("{path}\\shell\\open\\command");
        let Some(command) = open_key(key.0, "shell\\open\\command", &command_path)? else {
            continue;
        };
        let Some(command) = read_string(command.0, None)? else {
            continue;
        };
        return Ok(command_program(&command));
    }
    Ok(None)
}

struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        unsafe { RegCloseKey(self.0) };
    }
}

fn open_key(parent: HKEY, path: &str, label: &str) -> Result<Option<Key>, String> {
    let path = wide(path);
    let mut key: HKEY = std::ptr::null_mut();
    let result = unsafe { RegOpenKeyExW(parent, path.as_ptr(), 0, KEY_READ, &mut key) };
    match result {
        ERROR_SUCCESS => Ok(Some(Key(key))),
        ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => Ok(None),
        _ => Err(format!(
            "cannot read {label}: {}",
            std::io::Error::from_raw_os_error(result as i32)
        )),
    }
}

fn read_string(key: HKEY, name: Option<&str>) -> Result<Option<String>, String> {
    let name = name.map(wide);
    let name = name.as_ref().map_or(std::ptr::null(), |name| name.as_ptr());
    let mut length = 0u32;
    let result = unsafe {
        RegQueryValueExW(
            key,
            name,
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut length,
        )
    };
    if result == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if result != ERROR_SUCCESS {
        return Err(format!(
            "cannot read a registry value: {}",
            std::io::Error::from_raw_os_error(result as i32)
        ));
    }
    if length == 0 {
        return Ok(Some(String::new()));
    }
    let mut data = vec![0u8; length as usize];
    let mut kind = 0;
    let result = unsafe {
        RegQueryValueExW(
            key,
            name,
            std::ptr::null(),
            &mut kind,
            data.as_mut_ptr(),
            &mut length,
        )
    };
    if result != ERROR_SUCCESS {
        return Err(format!(
            "cannot read a registry value: {}",
            std::io::Error::from_raw_os_error(result as i32)
        ));
    }
    if kind != REG_SZ && kind != REG_EXPAND_SZ {
        return Ok(None);
    }
    let words: Vec<u16> = data
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .take_while(|c| *c != 0)
        .collect();
    Ok(Some(String::from_utf16_lossy(&words)))
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

pub(super) fn icon_png(program: &Path) -> Option<Vec<u8>> {
    let path = wide(&program.to_string_lossy());
    let mut icon: HICON = std::ptr::null_mut();
    let count = unsafe { ExtractIconExW(path.as_ptr(), 0, &mut icon, std::ptr::null_mut(), 1) };
    if count == 0 || icon.is_null() {
        return None;
    }
    let png = unsafe { icon_png_from_handle(icon) };
    unsafe { DestroyIcon(icon) };
    png
}

unsafe fn icon_png_from_handle(icon: HICON) -> Option<Vec<u8>> {
    let mut info = ICONINFO::default();
    if unsafe { GetIconInfo(icon, &mut info) } == 0 {
        return None;
    }
    let png = unsafe { icon_info_png(&info) };
    unsafe {
        if !info.hbmColor.is_null() {
            DeleteObject(info.hbmColor);
        }
        if !info.hbmMask.is_null() {
            DeleteObject(info.hbmMask);
        }
    }
    png
}

unsafe fn icon_info_png(info: &ICONINFO) -> Option<Vec<u8>> {
    // A monochrome icon has no color bitmap: the row draws its Lucide icon instead.
    let bitmap_handle = info.hbmColor;
    if bitmap_handle.is_null() {
        return None;
    }
    let mut bitmap = BITMAP::default();
    let got = unsafe {
        GetObjectW(
            bitmap_handle,
            std::mem::size_of::<BITMAP>() as i32,
            &mut bitmap as *mut BITMAP as *mut c_void,
        )
    };
    if got == 0 {
        return None;
    }
    let width = bitmap.bmWidth;
    let height = bitmap.bmHeight;
    if width <= 0 || height <= 0 {
        return None;
    }
    let (w, h) = (width as u32, height as u32);
    let mut info_header = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut pixels = vec![0u8; w as usize * h as usize * 4];
    let dc: HDC = unsafe { GetDC(std::ptr::null_mut()) };
    if dc.is_null() {
        return None;
    }
    let lines = unsafe {
        GetDIBits(
            dc,
            bitmap_handle,
            0,
            h,
            pixels.as_mut_ptr() as *mut c_void,
            &mut info_header,
            DIB_RGB_COLORS,
        )
    };
    if lines == 0 {
        unsafe { ReleaseDC(std::ptr::null_mut(), dc) };
        return None;
    }
    // Legacy icons keep transparency in the mask, not the color bitmap.
    if !info.hbmMask.is_null() && pixels.chunks_exact(4).all(|pixel| pixel[3] == 0) {
        let mut mask = vec![0u8; pixels.len()];
        let mut mask_header = info_header;
        mask_header.bmiHeader.biWidth = width;
        mask_header.bmiHeader.biHeight = -height;
        let lines = unsafe {
            GetDIBits(
                dc,
                info.hbmMask,
                0,
                h,
                mask.as_mut_ptr() as *mut c_void,
                &mut mask_header,
                DIB_RGB_COLORS,
            )
        };
        if lines != 0 {
            for (pixel, mask) in pixels.chunks_exact_mut(4).zip(mask.chunks_exact(4)) {
                pixel[3] = if mask[0] == 0 { 255 } else { 0 };
            }
        }
    }
    unsafe { ReleaseDC(std::ptr::null_mut(), dc) };
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    super::encode_png(w, h, &pixels)
}
