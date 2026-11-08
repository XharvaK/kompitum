use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::{Found, run};

pub(super) fn find(scheme: &str, want_icon: bool) -> Result<Option<Found>, String> {
    let scheme = scheme.replace('\\', "\\\\").replace('\'', "\\'");
    let script = format!(
        "ObjC.import('AppKit');const a=$.NSWorkspace.sharedWorkspace.\
URLForApplicationToOpenURL($.NSURL.URLWithString('{scheme}://'));a?a.path.js:'';"
    );
    let answer = run(
        "osascript",
        &["-l".into(), "JavaScript".into(), "-e".into(), script],
    )?;
    let path = answer.trim();
    if path.is_empty() {
        return Ok(None);
    }
    let program = PathBuf::from(path);
    let icon = want_icon.then(|| icon_png(&program)).flatten();
    Ok(Some(Found { program, icon }))
}

fn icon_png(app: &Path) -> Option<Vec<u8>> {
    let bundle = bundle(app)?;
    let plist = bundle.join("Contents").join("Info.plist");
    let name = run(
        "plutil",
        &[
            "-extract".into(),
            "CFBundleIconFile".into(),
            "raw".into(),
            plist.to_string_lossy().into_owned(),
        ],
    )
    .ok()?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let icon = {
        let icon = Path::new(name);
        if icon.is_absolute() {
            icon.to_path_buf()
        } else {
            let icon = bundle.join("Contents").join("Resources").join(icon);
            if icon.extension().is_none() {
                icon.with_extension("icns")
            } else {
                icon
            }
        }
    };
    if !icon.is_file() {
        return None;
    }
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let png = std::env::temp_dir().join(format!(
        "kompitum-icon-{}-{}.png",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&png);
    let converted = run(
        "sips",
        &[
            "-s".into(),
            "format".into(),
            "png".into(),
            icon.to_string_lossy().into_owned(),
            "--out".into(),
            png.to_string_lossy().into_owned(),
        ],
    );
    let bytes = converted.ok().and_then(|_| std::fs::read(&png).ok());
    let _ = std::fs::remove_file(&png);
    bytes.filter(|bytes| bytes.starts_with(b"\x89PNG\r\n\x1a\n"))
}

/// The `.app` folder, also when LaunchServices answers with a helper inside it.
fn bundle(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|path| path.extension().is_some_and(|ext| ext == "app"))
        .map(Path::to_path_buf)
}
