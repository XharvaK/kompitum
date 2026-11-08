//! What `kom run --contain` adds on Windows: a COMSPEC that keeps shells hidden.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

/// What a contained terminal adds to its program's environment: on Windows, a
/// COMSPEC that keeps shells hidden. An error means the terminal must not start.
pub fn contain_env(state: &Path, env: &mut BTreeMap<String, String>) -> io::Result<()> {
    let source = std::env::current_exe()?.with_file_name("kompitum-shell.exe");
    let bytes = match std::fs::read(&source) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Err(io::Error::other(
                "kompitum-shell.exe is missing beside kom, and a contained terminal needs it",
            ));
        }
        Err(e) => return Err(e),
    };
    // A copy per build of the shell: a copy in use is never replaced, and a
    // new build gets a folder of its own.
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    let folder = state.join(format!("shell-{:016x}", hasher.finish()));
    // Its name must be cmd.exe: Node and others pass cmd's /d /s /c only to a
    // shell of that name.
    let shell = folder.join("cmd.exe");
    if !shell.is_file() {
        std::fs::create_dir_all(&folder)?;
        let staged = folder.join(format!("cmd.{}.exe", std::process::id()));
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
