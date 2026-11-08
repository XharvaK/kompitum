use std::path::{Path, PathBuf};

use super::{Found, command_program, desktop_entry, run};

pub(super) fn find(scheme: &str, want_icon: bool) -> Result<Option<Found>, String> {
    let entry = run(
        "xdg-mime",
        &[
            "query".into(),
            "default".into(),
            format!("x-scheme-handler/{scheme}"),
        ],
    )?;
    let entry = entry.trim();
    if entry.is_empty() {
        return Ok(None);
    }
    let Some(text) = desktop_text(entry) else {
        return Ok(None);
    };
    let (exec, icon) = desktop_entry(&text);
    let Some(program) = exec.as_deref().and_then(command_program) else {
        return Ok(None);
    };
    let icon = want_icon
        .then(|| icon_png(scheme, icon.as_deref()))
        .flatten();
    Ok(Some(Found { program, icon }))
}

fn desktop_text(entry: &str) -> Option<String> {
    let path = Path::new(entry);
    if path.is_absolute() && path.is_file() {
        return std::fs::read_to_string(path).ok();
    }
    data_dirs()
        .into_iter()
        .find_map(|dir| std::fs::read_to_string(dir.join("applications").join(entry)).ok())
}

fn data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    match std::env::var_os("XDG_DATA_HOME").filter(|value| !value.is_empty()) {
        Some(home) => dirs.push(PathBuf::from(home)),
        None => {
            if let Some(home) = std::env::home_dir() {
                dirs.push(home.join(".local").join("share"));
            }
        }
    }
    let list = std::env::var_os("XDG_DATA_DIRS")
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    for dir in list.split(':').filter(|dir| !dir.is_empty()) {
        dirs.push(PathBuf::from(dir));
    }
    dirs
}

fn icon_png(scheme: &str, icon: Option<&str>) -> Option<Vec<u8>> {
    let name = icon.unwrap_or(scheme);
    let path = Path::new(name);
    if path.is_absolute()
        && let Ok(bytes) = std::fs::read(path)
        && bytes.starts_with(b"\x89PNG\r\n\x1a\n")
    {
        return Some(bytes);
    }
    let file = path.file_name()?.to_string_lossy().into_owned();
    let file = match file.strip_suffix(".png") {
        Some(stem) => format!("{stem}.png"),
        None => format!("{file}.png"),
    };
    // The largest size the hicolor theme has is the sharpest one to cache.
    let mut best: Option<(u32, PathBuf)> = None;
    for dir in data_dirs() {
        let root = dir.join("icons").join("hicolor");
        let Ok(sizes) = std::fs::read_dir(root) else {
            continue;
        };
        for size in sizes.flatten() {
            let size_path = size.path().join("apps").join(&file);
            if !size_path.is_file() {
                continue;
            }
            let pixels = size
                .file_name()
                .to_string_lossy()
                .split('x')
                .next()
                .and_then(|pixels| pixels.parse().ok())
                .unwrap_or(0);
            if best.as_ref().is_none_or(|(best, _)| pixels > *best) {
                best = Some((pixels, size_path));
            }
        }
    }
    if let Some((_, path)) = best
        && let Ok(bytes) = std::fs::read(path)
        && bytes.starts_with(b"\x89PNG\r\n\x1a\n")
    {
        return Some(bytes);
    }
    let mut dirs = data_dirs();
    dirs.push(PathBuf::from("/usr/share"));
    let scheme_file = format!("{scheme}.png");
    for dir in dirs {
        for file in [&file, &scheme_file] {
            if let Ok(bytes) = std::fs::read(dir.join("pixmaps").join(file))
                && bytes.starts_with(b"\x89PNG\r\n\x1a\n")
            {
                return Some(bytes);
            }
        }
    }
    None
}
