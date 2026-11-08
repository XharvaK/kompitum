//! The app that opens a link scheme on this computer, and its icon.

use std::path::PathBuf;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
use windows as os;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as os;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as os;

/// The app that opens `<scheme>://` links on this computer.
pub struct Handler {
    pub program: PathBuf,
    pub icon: Option<PathBuf>,
}

/// What one OS found: the program, and its icon already as PNG bytes.
struct Found {
    program: PathBuf,
    icon: Option<Vec<u8>>,
}

/// Ok(None): nothing handles the scheme. Err: the question itself failed, in words.
pub fn handler(scheme: &str) -> Result<Option<Handler>, String> {
    let cached = cache_path(scheme);
    let found = os::find(scheme, !cached.is_file())?;
    let Some(found) = found else {
        return Ok(None);
    };
    let icon = if cached.is_file() {
        Some(cached)
    } else {
        found.icon.and_then(|png| cache_icon(scheme, &png))
    };
    Ok(Some(Handler {
        program: found.program,
        icon,
    }))
}

/// `<app home>/cache/links/<scheme>.png`.
fn cache_path(scheme: &str) -> PathBuf {
    let safe: String = scheme
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    crate::mods::app_dir()
        .join("cache")
        .join("links")
        .join(format!("{safe}.png"))
}

/// The icon as `image()` names it: mods read assets by their path under the app home.
pub fn icon_asset(icon: &std::path::Path) -> String {
    let home = crate::mods::app_dir();
    icon.strip_prefix(&home).unwrap_or(icon).to_string_lossy().replace('\\', "/")
}

/// Writes the icon where `image()` can read it. No icon is never an error.
fn cache_icon(scheme: &str, png: &[u8]) -> Option<PathBuf> {
    if !png.starts_with(b"\x89PNG\r\n\x1a\n") {
        return None;
    }
    let path = cache_path(scheme);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&path, png).ok()?;
    Some(path)
}

/// The program of a `shell\open\command` value or an `Exec=` line, quoted or
/// not, with `%1`, `%u` and their kin dropped.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn command_program(command: &str) -> Option<PathBuf> {
    let mut rest = command.trim();
    let mut tokens: Vec<String> = Vec::new();
    while !rest.is_empty() {
        if let Some(quoted) = rest.strip_prefix('"') {
            let end = quoted.find('"')?;
            tokens.push(quoted[..end].to_owned());
            rest = quoted[end + 1..].trim_start();
        } else {
            let end = rest
                .char_indices()
                .find(|(_, c)| c.is_whitespace())
                .map_or(rest.len(), |(i, _)| i);
            tokens.push(rest[..end].to_owned());
            rest = rest[end..].trim_start();
        }
    }
    tokens
        .into_iter()
        .find(|token| !is_field_code(token))
        .map(PathBuf::from)
}

/// A desktop entry's `Exec=` and `Icon=`, from its `[Desktop Entry]` group only.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn desktop_entry(text: &str) -> (Option<String>, Option<String>) {
    let mut in_entry = false;
    let mut exec = None;
    let mut icon = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry || line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "Exec" if exec.is_none() => exec = Some(value.trim().to_owned()),
            "Icon" if icon.is_none() => icon = Some(value.trim().to_owned()),
            _ => {}
        }
    }
    (exec, icon)
}

/// `%1`, `%u`, `%U`, `%f` and their kin, but never `%SystemRoot%\...`.
fn is_field_code(token: &str) -> bool {
    token == "%%" || (token.len() == 2 && token.starts_with('%') && !token[1..].starts_with('%'))
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn run(program: &str, args: &[String]) -> Result<String, String> {
    let output = crate::spawn::command(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("cannot run {program}: {e}"))?;
    if !output.status.success() {
        let words = String::from_utf8_lossy(&output.stderr);
        let words = words.trim();
        return Err(if words.is_empty() {
            format!("{program} failed")
        } else {
            format!("{program} failed: {words}")
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(target_os = "windows")]
fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(rgba).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cached_icon_goes_by_its_path_under_the_app_home() {
        assert_eq!(icon_asset(&cache_path("claude")), "cache/links/claude.png");
    }

    #[test]
    fn command_program_reads_quoted_and_bare_commands() {
        assert_eq!(
            command_program(r#""C:\Program Files\Grok Bot\grokbot.exe" "%1""#),
            Some(PathBuf::from(r"C:\Program Files\Grok Bot\grokbot.exe"))
        );
        assert_eq!(
            command_program(r"C:\Windows\notepad.exe %1"),
            Some(PathBuf::from(r"C:\Windows\notepad.exe"))
        );
        assert_eq!(
            command_program("/usr/bin/firefox %u"),
            Some(PathBuf::from("/usr/bin/firefox"))
        );
        assert_eq!(
            command_program(r#""/opt/My App/app" --flag %U"#),
            Some(PathBuf::from("/opt/My App/app"))
        );
        assert_eq!(
            command_program("%u /opt/app"),
            Some(PathBuf::from("/opt/app"))
        );
        assert_eq!(
            command_program(r"%SystemRoot%\explorer.exe"),
            Some(PathBuf::from(r"%SystemRoot%\explorer.exe"))
        );
        assert_eq!(command_program(""), None);
        assert_eq!(command_program("%1 %u"), None);
    }

    #[test]
    fn desktop_entry_reads_only_the_desktop_entry_group() {
        let entry = "[Desktop Entry]\n\
Name=Grok\n\
Exec=\"/opt/My App/grok\" %U\n\
Icon=/opt/My App/icon.png\n\
\n\
[Desktop Action new]\n\
Exec=/opt/Other/other %u\n\
Icon=/opt/other.png\n";
        assert_eq!(
            desktop_entry(entry),
            (
                Some("\"/opt/My App/grok\" %U".into()),
                Some("/opt/My App/icon.png".into())
            )
        );
        assert_eq!(
            desktop_entry("[Desktop Action new]\nExec=/bin/other\nIcon=other\n"),
            (None, None)
        );
        assert_eq!(
            desktop_entry("[Desktop Entry]\nExec=/usr/bin/firefox %u\n"),
            (Some("/usr/bin/firefox %u".into()), None)
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn unknown_scheme_has_no_handler() {
        let scheme = format!("kompitum-none-{}", std::process::id());
        assert!(handler(&scheme).unwrap().is_none());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn explorer_icon_is_png() {
        let png = windows::icon_png(std::path::Path::new(r"C:\Windows\explorer.exe"))
            .expect("explorer.exe has an icon");
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    }
}
