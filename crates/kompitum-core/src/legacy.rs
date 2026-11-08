//! Names a renamed Kompitum still reads from the old installation.
//!
//! The old names live in this file on purpose (see `legacy name` below): every
//! other file uses only the new names.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::Path;

/// The environment pairs to add: `LIMEN_<rest>` becomes `KOMPITUM_<rest>` when
/// the new name is not already set. Pure, so the rule tests without touching
/// the process environment.
pub fn adoptable(vars: &[(OsString, OsString)]) -> Vec<(OsString, OsString)> {
    vars.iter()
        .filter_map(|(key, value)| {
            let rest = key.to_str()?.strip_prefix("LIMEN_")?;
            if rest.is_empty() {
                return None;
            }
            let new = OsString::from(format!("KOMPITUM_{rest}"));
            (!vars.iter().any(|(other, _)| other == &new)).then(|| (new, value.clone()))
        })
        .collect()
}

/// Sets the new names for every old environment setting.
///
/// Call it as the first thing a process does: the `unsafe` below is safe
/// because no thread has started yet.
pub fn adopt_env() {
    let vars: Vec<_> = std::env::vars_os().collect();
    for (key, value) in adoptable(&vars) {
        // SAFETY: the caller runs this before any thread starts, so no other
        // thread can read or write the environment concurrently.
        unsafe { std::env::set_var(key, value) };
    }
}

/// Copies the old folder to the new one once, leaving the old folder alone.
///
/// Returns whether anything was copied. Files ending in `.sock` or `.tmp` are
/// skipped, and so is a folder holding `.git` (a worktree, whose records point
/// at its old place).
pub fn adopt_folder(old: &Path, new: &Path) -> io::Result<bool> {
    if new.exists() || !old.is_dir() {
        return Ok(false);
    }
    if old.join(".git").exists() {
        return Ok(false);
    }
    copy_tree(old, new)?;
    eprintln!("kompitum: copied {} to {}; the old folder is left as it is", old.display(), new.display());
    Ok(true)
}

/// Copies one old file to the new place once, leaving the old file alone.
///
/// Used for the config file, which is read before any folder is made.
pub fn adopt_file(old: &Path, new: &Path) -> io::Result<bool> {
    if new.exists() || !old.is_file() {
        return Ok(false);
    }
    if let Some(parent) = new.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(old, new)?;
    eprintln!("kompitum: copied {} to {}; the old file is left as it is", old.display(), new.display());
    Ok(true)
}

fn copy_tree(old: &Path, new: &Path) -> io::Result<()> {
    fs::create_dir_all(new)?;
    for entry in fs::read_dir(old)? {
        let entry = entry?;
        let from = entry.path();
        let to = new.join(entry.file_name());
        if from.is_dir() {
            if from.join(".git").exists() {
                continue;
            }
            copy_tree(&from, &to)?;
        } else if from.is_file() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.ends_with(".sock") || name.ends_with(".tmp") {
                continue;
            }
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kompitum-legacy-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn old_settings_are_adopted_only_when_the_new_name_is_unset() {
        let vars = vec![
            (OsString::from("LIMEN_SESSION"), OsString::from("smoke")),
            (OsString::from("LIMEN_AGENT"), OsString::from("manager")),
            (OsString::from("KOMPITUM_AGENT"), OsString::from("opus")),
            (OsString::from("PATH"), OsString::from("/bin")),
        ];
        let pairs = adoptable(&vars);
        assert_eq!(pairs, vec![(OsString::from("KOMPITUM_SESSION"), OsString::from("smoke"))]);
    }

    #[test]
    fn a_folder_is_copied_once_skipping_sockets_temporaries_and_worktrees() {
        let root = temp("folder");
        let old = root.join("old");
        let new = root.join("new");
        fs::create_dir_all(old.join("work")).unwrap();
        fs::create_dir_all(old.join("sub")).unwrap();
        fs::write(old.join("keep.txt"), b"keep").unwrap();
        fs::write(old.join("drop.sock"), b"socket").unwrap();
        fs::write(old.join("drop.tmp"), b"temporary").unwrap();
        fs::write(old.join("work").join(".git"), b"gitdir: elsewhere").unwrap();
        fs::write(old.join("work").join("keep.txt"), b"worktree").unwrap();
        fs::write(old.join("sub").join("keep.txt"), b"sub").unwrap();

        assert!(adopt_folder(&old, &new).unwrap());
        assert!(old.join("keep.txt").exists(), "the old folder stays");
        assert_eq!(fs::read(new.join("keep.txt")).unwrap(), b"keep");
        assert_eq!(fs::read(new.join("sub").join("keep.txt")).unwrap(), b"sub");
        assert!(!new.join("drop.sock").exists());
        assert!(!new.join("drop.tmp").exists());
        assert!(!new.join("work").exists());

        assert!(!adopt_folder(&old, &new).unwrap(), "the new folder wins");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_file_is_copied_once_and_the_old_one_stays() {
        let root = temp("file");
        let old = root.join("old.toml");
        let new = root.join("nested").join("new.toml");
        fs::write(&old, b"settings").unwrap();

        assert!(adopt_file(&old, &new).unwrap());
        assert_eq!(fs::read(&new).unwrap(), b"settings");
        assert!(old.exists(), "the old file stays");
        assert!(!adopt_file(&old, &new).unwrap(), "the new file wins");
        fs::remove_dir_all(root).unwrap();
    }
}
