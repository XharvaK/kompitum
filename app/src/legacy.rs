//! The old names the app still reads after the rename.
//!
//! A file whose name holds `legacy` may name them freely: the rest of the app
//! uses only Kompitum's names.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The environment pairs to add: `LIMEN_<rest>` becomes `KOMPITUM_<rest>` when
/// the new name is not already set. Pure, so it tests without the process
/// environment.
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
/// Call it first in `main`, before any thread starts.
pub fn adopt_env() {
    let vars: Vec<_> = std::env::vars_os().collect();
    for (key, value) in adoptable(&vars) {
        // SAFETY: this is the first thing main does, before any thread starts.
        unsafe { std::env::set_var(key, value) };
    }
}

/// Copies the old app home once, leaving the old folder alone.
pub fn adopt_home() {
    let env = |key: &str| {
        std::env::var_os(key)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    if env("KOMPITUM_APP_HOME").is_some() {
        return;
    }
    let config = env("XDG_CONFIG_HOME").unwrap_or_else(|| crate::mods::home().join(".config"));
    adopt_home_at(&config);
}

/// The work of [`adopt_home`], on a config folder the tests choose.
pub fn adopt_home_at(config: &Path) {
    let old = config.join("limen-app");
    let new = config.join("kompitum-app");
    if new.exists() || !old.is_dir() {
        return;
    }
    match adopt_folder(&old, &new) {
        Ok(true) => {
            if let Err(e) = rename_copied_names(&new) {
                eprintln!(
                    "kompitum-app: cannot rename the copied settings in {}: {e}",
                    new.display()
                );
            }
        }
        Ok(false) => {}
        Err(e) => eprintln!(
            "kompitum-app: cannot copy {} to {}: {e}",
            old.display(),
            new.display()
        ),
    }
}

/// Copies the old folder once, skipping sockets, temporaries and worktrees.
pub fn adopt_folder(old: &Path, new: &Path) -> io::Result<bool> {
    if new.exists() || !old.is_dir() || old.join(".git").exists() {
        return Ok(false);
    }
    copy_tree(old, new)?;
    eprintln!(
        "kompitum-app: copied {} to {}; the old folder is left as it is",
        old.display(),
        new.display()
    );
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

/// In the copied home, an old theme or look name becomes the new one.
fn rename_copied_names(home: &Path) -> io::Result<()> {
    rename_entries(home)?;
    for folder in ["settings.json", "layouts", "themes"] {
        rename_looks(&home.join(folder))?;
    }
    Ok(())
}

fn rename_entries(folder: &Path) -> io::Result<()> {
    for entry in fs::read_dir(folder)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            rename_entries(&path)?;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.contains("limen98") {
            fs::rename(
                &path,
                path.with_file_name(name.replace("limen98", "kompitum98")),
            )?;
        }
    }
    Ok(())
}

fn rename_looks(path: &Path) -> io::Result<()> {
    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            if entry.path().is_dir() {
                rename_looks(&entry.path())?;
            } else {
                rewrite_look(&entry.path())?;
            }
        }
    } else if path.is_file() {
        rewrite_look(path)?;
    }
    Ok(())
}

fn rewrite_look(path: &Path) -> io::Result<()> {
    if path.extension().is_none_or(|ext| ext != "json") {
        return Ok(());
    }
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::InvalidData => return Ok(()),
        Err(e) => return Err(e),
    };
    let new = text.replace("limen98", "kompitum98");
    if new != text {
        fs::write(path, new)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("kompitum-app-legacy-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn old_settings_are_adopted_only_when_the_new_name_is_unset() {
        let vars = vec![
            (OsString::from("LIMEN_AGENT"), OsString::from("manager")),
            (OsString::from("KOMPITUM_AGENT"), OsString::from("opus")),
            (OsString::from("PATH"), OsString::from("/bin")),
        ];
        assert!(adoptable(&vars).is_empty(), "the new name is already set");
        let vars = vec![(OsString::from("LIMEN_AGENT"), OsString::from("manager"))];
        assert_eq!(
            adoptable(&vars),
            vec![(OsString::from("KOMPITUM_AGENT"), OsString::from("manager"))]
        );
    }

    #[test]
    fn the_old_home_is_copied_once_with_the_old_theme_renamed() {
        let root = temp("home");
        let old = root.join("limen-app");
        let new = root.join("kompitum-app");
        fs::create_dir_all(old.join("layouts")).unwrap();
        fs::create_dir_all(old.join("themes")).unwrap();
        fs::create_dir_all(old.join("work")).unwrap();
        fs::write(old.join("settings.json"), r#"{"theme":"limen98"}"#).unwrap();
        fs::write(
            old.join("layouts").join("desk.json"),
            r#"{"look":{"theme":"limen98"}}"#,
        )
        .unwrap();
        fs::write(old.join("themes").join("limen98.json"), "{}").unwrap();
        fs::write(old.join("drop.sock"), "socket").unwrap();
        fs::write(old.join("drop.tmp"), "temporary").unwrap();
        fs::write(old.join("work").join(".git"), "gitdir: elsewhere").unwrap();
        fs::write(old.join("work").join("keep.json"), "{}").unwrap();

        adopt_home_at(&root);
        assert!(old.join("settings.json").exists(), "the old home stays");
        assert_eq!(
            fs::read_to_string(new.join("settings.json")).unwrap(),
            r#"{"theme":"kompitum98"}"#
        );
        assert_eq!(
            fs::read_to_string(new.join("layouts").join("desk.json")).unwrap(),
            r#"{"look":{"theme":"kompitum98"}}"#
        );
        assert!(new.join("themes").join("kompitum98.json").exists());
        assert!(!new.join("drop.sock").exists());
        assert!(!new.join("drop.tmp").exists());
        assert!(!new.join("work").exists());

        fs::write(new.join("settings.json"), "new wins").unwrap();
        adopt_home_at(&root);
        assert_eq!(
            fs::read_to_string(new.join("settings.json")).unwrap(),
            "new wins"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
