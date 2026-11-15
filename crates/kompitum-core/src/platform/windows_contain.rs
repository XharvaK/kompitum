//! What `kom run --contain` adds on Windows: a COMSPEC that keeps shells hidden.

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

type ShellKey = (PathBuf, u64, std::time::SystemTime);
type ShellHashes = Mutex<HashMap<ShellKey, u64>>;

/// The shell's hash by path, size and modification time: a kept terminal asks
/// every second, and the file must not be read and hashed each time.
static HASHED: OnceLock<ShellHashes> = OnceLock::new();
#[cfg(test)]
static READS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn missing_shell() -> io::Error {
    io::Error::other("kompitum-shell.exe is missing beside kom, and a contained terminal needs it")
}

fn shell_hash(source: &Path) -> io::Result<u64> {
    let missing = |e: io::Error| if e.kind() == io::ErrorKind::NotFound { missing_shell() } else { e };
    let meta = std::fs::metadata(source).map_err(missing)?;
    let key = (source.to_path_buf(), meta.len(), meta.modified()?);
    let hashes = HASHED.get_or_init(Default::default);
    if let Some(hash) = hashes.lock().unwrap_or_else(|p| p.into_inner()).get(&key) {
        return Ok(*hash);
    }
    let bytes = std::fs::read(source).map_err(missing)?;
    #[cfg(test)]
    READS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    let hash = hasher.finish();
    hashes.lock().unwrap_or_else(|p| p.into_inner()).insert(key, hash);
    Ok(hash)
}

/// What a contained terminal adds to its program's environment: on Windows, a
/// COMSPEC that keeps shells hidden. An error means the terminal must not start.
pub fn contain_env(state: &Path, env: &mut BTreeMap<String, String>) -> io::Result<()> {
    let source = std::env::current_exe()?.with_file_name("kompitum-shell.exe");
    // A copy per build of the shell: a copy in use is never replaced, and a
    // new build gets a folder of its own.
    let folder = state.join(format!("shell-{:016x}", shell_hash(&source)?));
    // Its name must be cmd.exe: Node and others pass cmd's /d /s /c only to a
    // shell of that name.
    let shell = folder.join("cmd.exe");
    if !shell.is_file() {
        std::fs::create_dir_all(&folder)?;
        let staged = folder.join(format!("cmd.{}.exe", std::process::id()));
        let bytes = std::fs::read(&source)
            .map_err(|e| if e.kind() == io::ErrorKind::NotFound { missing_shell() } else { e })?;
        std::fs::write(&staged, &bytes)?;
        if let Err(e) = std::fs::rename(&staged, &shell) {
            let _ = std::fs::remove_file(&staged);
            // Another terminal may have finished the same copy first.
            if !shell.is_file() {
                return Err(e);
            }
        }
    }
    point_comspec(env, &shell);
    Ok(())
}

/// Points `COMSPEC` at `shell`, keeping the real one in `KOMPITUM_COMSPEC` so a
/// shell line still starts Windows' own cmd.
fn point_comspec(env: &mut BTreeMap<String, String>, shell: &Path) {
    let find = |key: &str| env.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.clone());
    let old = find("COMSPEC").unwrap_or_else(|| {
        let root = find("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
        format!(r"{root}\System32\cmd.exe")
    });
    env.retain(|k, _| !k.eq_ignore_ascii_case("COMSPEC"));
    // A terminal inside another contained one: the real shell it already has.
    let contained = Path::new(&old)
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("shell-"));
    if !contained {
        env.insert("KOMPITUM_COMSPEC".into(), old);
    }
    env.insert("COMSPEC".into(), shell.display().to_string());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_comspec_key_in_any_case_becomes_one_comspec_and_keeps_the_real_shell() {
        let real = r"C:\Windows\System32\cmd.exe";
        let mut env = BTreeMap::from([
            ("ComSpec".to_string(), real.to_string()),
            ("PATH".to_string(), r"C:\Windows".to_string()),
        ]);
        let shell = Path::new(r"C:\state\shell-0123456789abcdef\cmd.exe");
        point_comspec(&mut env, shell);
        let keys: Vec<&String> = env.keys().filter(|k| k.eq_ignore_ascii_case("comspec")).collect();
        assert_eq!(keys, ["COMSPEC"]);
        assert_eq!(env["COMSPEC"], shell.display().to_string());
        assert_eq!(env["KOMPITUM_COMSPEC"], real);
        // A terminal started inside a contained one keeps the real shell.
        let inner = Path::new(r"C:\state\shell-fedcba9876543210\cmd.exe");
        point_comspec(&mut env, inner);
        assert_eq!(env["COMSPEC"], inner.display().to_string());
        assert_eq!(env["KOMPITUM_COMSPEC"], real);
    }

    #[test]
    fn the_shell_is_hashed_once_per_file_version() {
        let dir = std::env::temp_dir().join(format!("kom-shell-hash-{}", crate::platform::random_hex()));
        std::fs::create_dir_all(&dir).unwrap();
        let shell = dir.join("kompitum-shell.exe");
        std::fs::write(&shell, b"first build").unwrap();
        let first = shell_hash(&shell).unwrap();
        let reads = READS.load(std::sync::atomic::Ordering::SeqCst);
        assert_eq!(shell_hash(&shell).unwrap(), first);
        assert_eq!(
            READS.load(std::sync::atomic::Ordering::SeqCst),
            reads,
            "the same file and version is not read again"
        );
        std::fs::write(&shell, b"a different build").unwrap();
        let second = shell_hash(&shell).unwrap();
        assert_ne!(second, first, "a new build has a new hash");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_missing_comspec_names_windows_own_shell_as_the_real_one() {
        let shell = Path::new(r"C:\state\shell-0123456789abcdef\cmd.exe");
        let mut env = BTreeMap::from([("SYSTEMROOT".to_string(), r"D:\Win".to_string())]);
        point_comspec(&mut env, shell);
        assert_eq!(env["KOMPITUM_COMSPEC"], r"D:\Win\System32\cmd.exe");
        assert_eq!(env["COMSPEC"], shell.display().to_string());
        let mut env = BTreeMap::new();
        point_comspec(&mut env, shell);
        assert_eq!(env["KOMPITUM_COMSPEC"], r"C:\Windows\System32\cmd.exe");
    }
}
