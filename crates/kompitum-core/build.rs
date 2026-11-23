//! Build metadata: the commit `kom -V` names, and macOS's Info.plist.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let commit = match git_dir(&root) {
        Some(git) => {
            println!("cargo:rerun-if-changed={}", git.join("HEAD").display());
            if let Some(reference) = head_ref(&git) {
                println!("cargo:rerun-if-changed={}", common_dir(&git).join(reference).display());
            }
            commit(&root)
        }
        None => String::new(),
    };
    println!("cargo:rustc-env=KOM_COMMIT={commit}");

    println!("cargo:rerun-if-changed=../../distribution/macos-Info.plist");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let plist = include_str!("../../distribution/macos-Info.plist")
        .replace("@VERSION@", &std::env::var("CARGO_PKG_VERSION").unwrap());
    let path = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("Info.plist");
    std::fs::write(&path, plist).unwrap();
    // Separate arguments preserve paths containing spaces.
    for arg in ["-sectcreate", "__TEXT", "__info_plist"] {
        println!("cargo:rustc-link-arg-bin=kompitum-core=-Wl,{arg}");
    }
    println!("cargo:rustc-link-arg-bin=kompitum-core={}", path.display());
}

/// The repository's `.git`, following the `gitdir:` file a worktree has.
fn git_dir(root: &Path) -> Option<PathBuf> {
    let dot = root.join(".git");
    if dot.is_dir() {
        return Some(dot);
    }
    let text = std::fs::read_to_string(&dot).ok()?;
    let path = Path::new(text.trim().strip_prefix("gitdir:")?.trim());
    Some(if path.is_absolute() { path.to_path_buf() } else { root.join(path) })
}

/// The branch `HEAD` points to, or nothing when it holds a commit (detached).
fn head_ref(git: &Path) -> Option<String> {
    let head = std::fs::read_to_string(git.join("HEAD")).ok()?;
    head.trim().strip_prefix("ref: ").map(str::to_string)
}

/// Where a worktree keeps its branches; its own git folder when it has no `commondir`.
fn common_dir(git: &Path) -> PathBuf {
    let Ok(text) = std::fs::read_to_string(git.join("commondir")) else {
        return git.to_path_buf();
    };
    let path = Path::new(text.trim());
    if path.is_absolute() { path.to_path_buf() } else { git.join(path) }
}

/// The short hash of `HEAD`, or an empty string without git or outside a repository.
fn commit(root: &Path) -> String {
    let mut cmd = Command::new("git");
    cmd.args(["rev-parse", "--short", "HEAD"]).current_dir(root);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let Ok(out) = cmd.output() else {
        return String::new();
    };
    if !out.status.success() {
        return String::new();
    }
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}
