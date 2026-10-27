//! Atomic replacement for app-owned files.

use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

pub fn write_atomic(path: &Path, body: impl AsRef<[u8]>) -> std::io::Result<()> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(".tmp.{}.{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let tmp = std::path::PathBuf::from(name);
    let result = (|| {
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        file.write_all(body.as_ref())?;
        drop(file);
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}
