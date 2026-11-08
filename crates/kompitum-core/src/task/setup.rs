//! What surrounds a worker's turn: its environment, the commit guard, denied tools and Python.

use crate::rpc::RpcError;
use crate::runtime::Core;
use crate::task::types::Task;
use crate::task::worker::Worker;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

/// What a turn gets from `prepare`.
#[derive(Debug, Default)]
pub struct Prepared {
    /// Set in the turn's terminal, on top of what the turn's command sets.
    pub env: BTreeMap<String, String>,
    /// A folder the runner puts first on the worker's PATH.
    pub path_first: Option<String>,
    /// Worth a line each in the task's warnings.
    pub warnings: Vec<String>,
}

/// The names of the variables a worker must not inherit: its launcher's
/// leftovers, another agent's settings, and this core's own, keyed the way the
/// core has them (Windows spells `Path` as it likes, so keys compare in upper case).
pub fn to_remove(vars: impl Iterator<Item = (String, String)>) -> Vec<String> {
    const KEEP: [&str; 8] = [
        "KOMPITUM_SCOPE",
        "KOMPITUM_TERMINAL_ID",
        "KOMPITUM_TERMINAL_SOCKET",
        "KOMPITUM_SESSION",
        "KOMPITUM_SOCKET",
        "KOMPITUM_COMSPEC",
        "KOMPITUM_WORKER",
        "KOMPITUM_PROTECTED",
    ];
    let vars: Vec<(String, String)> = vars.collect();
    // `scrub_env`'s rule: a uv launcher stub sets PYTHONHOME for the interpreter
    // it starts, and inherited further it makes every other Python load the
    // wrong standard library.
    let stub = vars.iter().any(|(key, value)| key.eq_ignore_ascii_case("UV_INTERNAL__PYTHONHOME") && !value.is_empty());
    vars.into_iter()
        .map(|(key, _)| key)
        .filter(|key| {
            let key = key.to_ascii_uppercase();
            key == "OLDPWD"
                || (stub && (key == "PYTHONHOME" || key == "UV_INTERNAL__PYTHONHOME"))
                || key.starts_with("CLAUDE")
                || key.starts_with("ANTHROPIC")
                || (key.starts_with("KOMPITUM_") && !KEEP.contains(&key.as_str()))
        })
        .collect()
}

/// Git hooks that refuse commit and push in the repository the worker was given.
/// Its worktree shares that repository's git dir, so `rev-parse` finds it from
/// either; a scratch repository a worker makes for itself is its own business.
/// LF on every OS, and executable: git runs it through sh.
const GUARD_HOOK: &str = "#!/bin/sh
dir=$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null | tr 'A-Z\\\\' 'a-z/')
case \"|$KOMPITUM_PROTECTED|\" in
  *\"|$dir|\"*) echo 'kom: git commit/push is blocked for workers' >&2; exit 1;;
esac
exit 0
";

/// A path as git names one: forward slashes, which git config entries accept on
/// both systems.
fn forward_slashes(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

/// A path as `KOMPITUM_PROTECTED` holds it and the hook compares it: forward
/// slashes and lower case, since a Windows path has no fixed case.
fn guard_path(path: &Path) -> String {
    forward_slashes(path).to_ascii_lowercase()
}

fn guard_failed(message: String) -> RpcError {
    RpcError::new(
        "unavailable",
        format!("{message}; a worker does not start without the commit guard"),
        "check the state folder's permissions",
    )
}

/// `<state>/guard-hooks/`: the three hooks, made where they are not already right.
fn guard_hooks(state: &Path) -> Result<PathBuf, RpcError> {
    let folder = state.join("guard-hooks");
    std::fs::create_dir_all(&folder).map_err(|e| guard_failed(format!("cannot make {}: {e}", folder.display())))?;
    for name in ["pre-commit", "pre-push", "commit-msg"] {
        let hook = folder.join(name);
        if std::fs::read(&hook).ok().as_deref() != Some(GUARD_HOOK.as_bytes()) {
            std::fs::write(&hook, GUARD_HOOK)
                .map_err(|e| guard_failed(format!("cannot write {}: {e}", hook.display())))?;
        }
        crate::platform::make_executable(&hook)
            .map_err(|e| guard_failed(format!("cannot make {} executable: {e}", hook.display())))?;
    }
    Ok(folder)
}

/// The repository's common git dir, as the hook compares it: absolute, lower
/// case, forward slashes. A `git` that cannot say fails the start, loudly.
async fn protected_git_dir(repo: &str) -> Result<String, RpcError> {
    let out = super::git::git(repo, &["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .await
        .map_err(|e| guard_failed(format!("cannot read the git dir of {repo}: {}", e.message)))?;
    let dir = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if dir.is_empty() {
        return Err(guard_failed(format!("git could not say where the git dir of {repo} is")));
    }
    // The hook asks git the same question, so a repo path spelled another way still matches.
    Ok(guard_path(Path::new(&dir)))
}

/// The files a worker must not read: `worker_config`'s globs, and the same
/// names behind any folder.
const SECRET_GLOBS: [&str; 7] = ["*.env", "*.env.*", ".env", "*.pem", "*/.ssh/*", "*.ssh*", "*auth.json*"];
const SECRET_DEEP: [&str; 5] = ["**/.env", "**/.env.*", "**/*.pem", "**/.ssh/**", "**/auth.json"];
/// The shell commands a worker must not run: a commit or a push, and a secret in a shell line.
const BASH_DENY: [&str; 9] = [
    "git push*",
    "*git push*",
    "git commit*",
    "*git commit*",
    "*git -c*commit*",
    "*.env*",
    "*.pem*",
    "*.ssh*",
    "*auth.json*",
];

/// `worker_config`: OpenCode's permission map, so a tool that would read a
/// secret or commit is not a tool this worker has at all. Denied by
/// permission, never by asking: a worker routes around a tool that fails, but
/// cannot use one it doesn't have. The tools nobody calls are gone too; a task
/// that asked for the web keeps its two.
fn opencode_config(web: bool) -> String {
    let mut read = serde_json::Map::new();
    read.insert("*".into(), serde_json::json!("allow"));
    for glob in SECRET_GLOBS.iter().chain(SECRET_DEEP.iter()) {
        read.insert((*glob).to_string(), serde_json::json!("deny"));
    }
    let mut bash = serde_json::Map::new();
    bash.insert("*".into(), serde_json::json!("allow"));
    for glob in BASH_DENY {
        bash.insert(glob.to_string(), serde_json::json!("deny"));
    }
    let edit = serde_json::json!({"*": "allow", "*.env": "deny", "**/.env": "deny", "*.pem": "deny"});
    let mut permission = serde_json::Map::new();
    permission.insert("read".into(), read.into());
    permission.insert("bash".into(), bash.into());
    permission.insert("edit".into(), edit);
    for tool in ["skill", "subagent", "question", "execute"] {
        permission.insert(tool.into(), serde_json::json!("deny"));
    }
    if !web {
        for tool in ["webfetch", "websearch"] {
            permission.insert(tool.into(), serde_json::json!("deny"));
        }
    }
    serde_json::json!({"permission": permission}).to_string()
}

/// `CC_DENY`: Command Code's own spellings of the same refusals.
const CC_DENY: [&str; 15] = [
    "Read(.env*)",
    "Read(*.env)",
    "Read(*.pem)",
    "Read(~/.ssh/**)",
    "Read(auth.json)",
    "Read(~/.commandcode/**)",
    "Read(~/.local/share/opencode/**)",
    "Edit(.env*)",
    "Edit(*.pem)",
    "Shell(*.env*)",
    "Shell(*.pem*)",
    "Shell(*.ssh*)",
    "Shell(*auth.json*)",
    "Shell(*git commit*)",
    "Shell(*git push*)",
];

/// `cc_settings`'s JSON, as Command Code reads its per-workspace deny rules.
fn cc_settings() -> Vec<u8> {
    let settings = serde_json::json!({
        "tasteLearning": false,
        "attribution": {"commit": ""},
        "permissions": {"deny": CC_DENY},
    });
    serde_json::to_vec_pretty(&settings).unwrap_or_default()
}

/// A settings file's marker, read back however the turn ended.
fn settings_marker(folder: &Path, n: u32, kind: &str) -> PathBuf {
    folder.join(format!("turn-{n}.settings-{kind}"))
}

/// Command Code reads deny rules from `<worktree>/.commandcode/settings.local.json`.
/// A file that was there keeps its bytes beside the turn; one this made leaves
/// a marker, so `after_turn` knows how to put the worktree back.
fn command_code_settings(folder: &Path, worktree: &str, n: u32) -> Result<(), RpcError> {
    let dir = Path::new(worktree).join(".commandcode");
    let path = dir.join("settings.local.json");
    std::fs::create_dir_all(&dir).map_err(|e| settings_failed(format!("cannot make {}: {e}", dir.display())))?;
    if path.is_file() {
        let kept = settings_marker(folder, n, "kept");
        let bytes =
            std::fs::read(&path).map_err(|e| settings_failed(format!("cannot read {}: {e}", path.display())))?;
        std::fs::write(&kept, bytes).map_err(|e| {
            settings_failed(format!("cannot keep {}'s bytes in {}: {e}", path.display(), kept.display()))
        })?;
    } else {
        let made = settings_marker(folder, n, "made");
        std::fs::write(&made, b"").map_err(|e| settings_failed(format!("cannot write {}: {e}", made.display())))?;
    }
    std::fs::write(&path, cc_settings()).map_err(|e| settings_failed(format!("cannot write {}: {e}", path.display())))
}

fn settings_failed(message: String) -> RpcError {
    RpcError::new("unavailable", message, "check the worktree's and the state folder's permissions")
}

/// The question a candidate Python answers in one line: its version, its
/// release level, and the real interpreter behind a launcher stub.
const PYTHON_PROBE: &str = concat!(
    "import sys; print(sys.version.split()[0], sys.version_info.releaselevel, ",
    "getattr(sys, '_base_executable', None) or sys.executable)"
);

/// The one real Python this core found, kept for its life; a miss is kept too.
static REAL_PYTHON: tokio::sync::OnceCell<Option<String>> = tokio::sync::OnceCell::const_new();

/// What a probe printed, when it is a Python a worker may use: 3.9 or newer, a
/// final release, and a path that is a file now. Anything else is refused by
/// what it said, not by the folder it was found in.
fn usable_python(text: &str) -> Option<String> {
    let line = text.lines().next().unwrap_or_default().trim();
    let mut fields = line.splitn(3, ' ');
    let version = fields.next()?;
    let level = fields.next()?;
    let path = fields.next()?.trim();
    let mut numbers = version.split('.').map(str::parse::<u32>);
    let (Some(major), Some(minor)) = (numbers.next().and_then(Result::ok), numbers.next().and_then(Result::ok)) else {
        return None;
    };
    if (major, minor) < (3, 9) || level != "final" || !Path::new(path).is_file() {
        return None;
    }
    Some(path.to_string())
}

/// Every Python worth asking, best first: those on PATH (never the Windows
/// Store's alias, whose `python` only prints an advert), then uv's own, newest
/// first by its folder's number, so 3.12 comes before 3.9.
fn python_candidates(path: &std::ffi::OsStr, uv: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::env::split_paths(path)
        .filter(|dir| !dir.components().any(|part| part.as_os_str().eq_ignore_ascii_case("WindowsApps")))
        .flat_map(|dir| ["python.exe", "python3.exe"].map(|name| dir.join(name)))
        .collect();
    let number = |exe: &PathBuf| -> Vec<u32> {
        exe.parent()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
            .split(|c: char| !c.is_ascii_digit())
            .filter_map(|part| part.parse().ok())
            .collect()
    };
    let mut uv: Vec<(Vec<u32>, PathBuf)> = uv.into_iter().map(|exe| (number(&exe), exe)).collect();
    uv.sort_by(|a, b| b.0.cmp(&a.0));
    found.extend(uv.into_iter().map(|(_, exe)| exe));
    found
}

/// uv's Pythons: `%APPDATA%\uv\python\cpython-3.*\python.exe`.
fn uv_entries(appdata: Option<std::ffi::OsString>) -> Vec<PathBuf> {
    let Some(appdata) = appdata else { return Vec::new() };
    let folder = Path::new(&appdata).join("uv").join("python");
    std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|dir| dir.file_name().is_some_and(|name| name.to_string_lossy().starts_with("cpython-3.")))
        .map(|dir| dir.join("python.exe"))
        .collect()
}

/// One candidate, asked with `-I -c` under 10 s, without a window.
async fn probe_python(exe: &Path) -> Option<String> {
    let mut command = std::process::Command::new(exe);
    command.args(["-I", "-c", PYTHON_PROBE]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    crate::platform::no_console(&mut command);
    let mut command = tokio::process::Command::from(command);
    command.kill_on_drop(true);
    let out = tokio::time::timeout(Duration::from_secs(10), command.output()).await.ok()?.ok()?;
    usable_python(&String::from_utf8_lossy(&out.stdout))
}

async fn real_python() -> Option<String> {
    REAL_PYTHON
        .get_or_init(|| async {
            let path = std::env::var_os("PATH").unwrap_or_default();
            for candidate in python_candidates(&path, uv_entries(std::env::var_os("APPDATA"))) {
                if !candidate.is_file() {
                    continue;
                }
                if let Some(found) = probe_python(&candidate).await {
                    return Some(found);
                }
            }
            None
        })
        .await
        .clone()
}

fn shim_failed(message: String) -> RpcError {
    RpcError::new("unavailable", message, "check the state folder's permissions and free space")
}

/// `<state>/bin/`: `python` and `python3` for the real interpreter, so a worker's
/// python is a Python and not the Store's advert. The `.cmd` files serve cmd and
/// PowerShell; the extensionless sh scripts serve Git Bash, the shell OpenCode
/// runs on Windows, which never resolves a bare name to a `.cmd` file.
fn python_shims(state: &Path, exe: &str) -> Result<PathBuf, RpcError> {
    let folder = state.join("bin");
    std::fs::create_dir_all(&folder).map_err(|e| shim_failed(format!("cannot make {}: {e}", folder.display())))?;
    let cmd = format!("@\"{exe}\" %*\r\n");
    let sh = format!("#!/bin/sh\nexec \"{}\" \"$@\"\n", exe.replace('\\', "/"));
    let files = [("python.cmd", cmd.clone()), ("python", sh.clone()), ("python3.cmd", cmd), ("python3", sh)];
    for (name, text) in files {
        let path = folder.join(name);
        if std::fs::read(&path).ok().as_deref() != Some(text.as_bytes()) {
            std::fs::write(&path, text).map_err(|e| shim_failed(format!("cannot write {}: {e}", path.display())))?;
        }
        if !name.ends_with(".cmd") {
            crate::platform::make_executable(&path)
                .map_err(|e| shim_failed(format!("cannot make {} executable: {e}", path.display())))?;
        }
    }
    Ok(folder)
}

/// The line a worker's python question leaves when none was found.
const NO_PYTHON: &str = "no real Python found, so this worker's python is whatever its PATH has";

/// Everything a turn needs around it, made just before its terminal starts.
pub async fn prepare(
    core: &Arc<Core>,
    task: &Task,
    worker: Worker,
    worktree: &str,
    n: u32,
) -> Result<Prepared, RpcError> {
    let hooks = guard_hooks(&core.paths.state)?;
    let protected = protected_git_dir(&task.repo).await?;
    let mut prepared = Prepared::default();
    // The mark a worker's own kom reads: it may look, and nothing more.
    prepared.env.insert("KOMPITUM_WORKER".into(), "1".into());
    prepared.env.insert("GIT_CONFIG_COUNT".into(), "1".into());
    prepared.env.insert("GIT_CONFIG_KEY_0".into(), "core.hooksPath".into());
    prepared.env.insert("GIT_CONFIG_VALUE_0".into(), forward_slashes(&hooks));
    prepared.env.insert("KOMPITUM_PROTECTED".into(), protected);
    prepared.env.insert("OPENCODE_CONFIG_CONTENT".into(), opencode_config(task.asked.web));
    // OpenCode would read the repository's and the home folder's AGENTS.md by itself; the prompt
    // carries the committed one instead. Command Code's settings stay as they are: what it
    // carries is unmeasured until its credits return.
    if worker != Worker::Cc {
        prepared.env.insert("OPENCODE_DISABLE_PROJECT_CONFIG".into(), "1".into());
    }
    if crate::platform::PYTHON_SHIMS {
        match real_python().await {
            Some(exe) => prepared.path_first = Some(python_shims(&core.paths.state, &exe)?.display().to_string()),
            None if !task.warnings.iter().any(|warning| warning == NO_PYTHON) => {
                prepared.warnings.push(NO_PYTHON.into());
            }
            None => {}
        }
    }
    let folder = crate::task::task_folder(&core.paths.state, &task.name);
    if worker == Worker::Cc
        && let Err(error) = command_code_settings(&folder, worktree, n)
    {
        // A start that fails leaves the worktree as it found it.
        let _ = after_turn(&folder, worktree, worker, n);
        return Err(error);
    }
    Ok(prepared)
}

/// Puts back what `prepare` put in the worktree, however the turn ended and after a core restart too.
pub fn after_turn(folder: &Path, worktree: &str, _worker: Worker, n: u32) -> Result<(), String> {
    let path = Path::new(worktree).join(".commandcode").join("settings.local.json");
    let kept = settings_marker(folder, n, "kept");
    let made = settings_marker(folder, n, "made");
    if kept.is_file() {
        let bytes = std::fs::read(&kept).map_err(|e| format!("cannot read {}: {e}", kept.display()))?;
        // The worker may have removed the folder along with the file.
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("cannot make {}: {e}", dir.display()))?;
        }
        std::fs::write(&path, bytes).map_err(|e| format!("cannot put {} back: {e}", path.display()))?;
        std::fs::remove_file(&kept).map_err(|e| format!("cannot remove {}: {e}", kept.display()))?;
    } else if made.is_file() {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("cannot remove {}: {e}", path.display())),
        }
        if let Some(dir) = path.parent()
            && dir.is_dir()
            && std::fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_none())
        {
            // Meant to fail when anything else is in it.
            let _ = std::fs::remove_dir(dir);
        }
        std::fs::remove_file(&made).map_err(|e| format!("cannot remove {}: {e}", made.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kept_settings_go_back_even_when_the_worker_removed_their_folder() {
        let scratch = std::env::temp_dir().join(format!("kom-after-turn-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        let folder = scratch.join("task");
        let worktree = scratch.join("worktree");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::write(settings_marker(&folder, 1, "kept"), b"mine").unwrap();

        after_turn(&folder, &worktree.display().to_string(), Worker::Cc, 1).unwrap();

        let back = std::fs::read(worktree.join(".commandcode").join("settings.local.json")).unwrap();
        assert_eq!(back, b"mine");
        assert!(!settings_marker(&folder, 1, "kept").exists(), "the kept copy is gone");
        let _ = std::fs::remove_dir_all(&scratch);
    }

    fn vars(pairs: &[(&str, &str)]) -> impl Iterator<Item = (String, String)> {
        let pairs: Vec<(String, String)> = pairs.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect();
        pairs.into_iter()
    }

    #[test]
    fn what_a_worker_must_not_inherit_goes_and_the_rest_stays() {
        let removed = to_remove(vars(&[
            ("OLDPWD", "/before"),
            ("PWD", "/worktree"),
            ("Path", "/c/Windows"),
            ("path", "/c/Other"),
            ("UV_INTERNAL__PYTHONHOME", "/uv/python"),
            ("PYTHONHOME", "/uv/python"),
            ("CLAUDE_CODE_ENTRYPOINT", "cli"),
            ("anthropic_api_key", "set"),
            ("KOMPITUM_POLL_SECONDS", "5"),
            ("KOMPITUM_WORKER", "1"),
            ("KOMPITUM_SCOPE", "abc"),
            ("HOME", "/home"),
        ]));
        assert_eq!(
            removed,
            [
                "OLDPWD",
                "UV_INTERNAL__PYTHONHOME",
                "PYTHONHOME",
                "CLAUDE_CODE_ENTRYPOINT",
                "anthropic_api_key",
                "KOMPITUM_POLL_SECONDS",
            ]
        );
        assert!(
            !removed.iter().any(|key| key.eq_ignore_ascii_case("path")),
            "one spelling each, and neither goes: {removed:?}"
        );

        let kept = to_remove(vars(&[
            ("PYTHONHOME", "/py"),
            ("UV_INTERNAL__PYTHONHOME", ""),
            ("KOMPITUM_TERMINAL_SOCKET", "\\\\.\\pipe\\x"),
            ("KOMPITUM_SESSION", "s"),
            ("KOMPITUM_PROTECTED", "/repo/.git"),
            ("KOMPITUM_COMSPEC", "kompitum-shell"),
        ]));
        assert!(kept.is_empty(), "{kept:?}");
    }

    #[test]
    fn the_probe_takes_a_final_python_and_refuses_the_rest() {
        let scratch = std::env::temp_dir().join(format!("kom-python-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        let folder = scratch.join("a folder").join("with spaces");
        std::fs::create_dir_all(&folder).unwrap();
        let exe = folder.join("python.exe");
        std::fs::write(&exe, "").unwrap();
        let path = exe.display().to_string();

        assert_eq!(usable_python(&format!("3.12.4 final {path}\r\n")), Some(path.clone()));
        assert_eq!(usable_python(&format!("3.9.0 final {path}")), Some(path.clone()), "3.9 is new enough");
        assert_eq!(usable_python(&format!("3.8.10 final {path}")), None, "an old one");
        assert_eq!(usable_python(&format!("3.13.0 alpha {path}")), None, "an alpha, whatever its folder is called");
        assert_eq!(usable_python("garbage"), None);
        assert_eq!(usable_python(""), None);
        assert_eq!(usable_python(&format!("3.12.4 final {path}.gone")), None, "a path that is not a file");
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn python_candidates_skip_the_store_and_go_newest_first() {
        let path = std::env::join_paths([
            PathBuf::from("user").join("bin"),
            PathBuf::from("store").join("WindowsApps"),
            PathBuf::from("windows").join("system32"),
        ])
        .unwrap();
        let uv = ["cpython-3.9.0", "cpython-3.12.4", "cpython-3.10.1"]
            .map(|name| PathBuf::from("roaming").join("uv").join("python").join(name).join("python.exe"))
            .to_vec();
        let found: Vec<String> =
            python_candidates(&path, uv).iter().map(|exe| exe.display().to_string().replace('\\', "/")).collect();
        assert_eq!(
            found,
            [
                "user/bin/python.exe",
                "user/bin/python3.exe",
                "windows/system32/python.exe",
                "windows/system32/python3.exe",
                "roaming/uv/python/cpython-3.12.4/python.exe",
                "roaming/uv/python/cpython-3.10.1/python.exe",
                "roaming/uv/python/cpython-3.9.0/python.exe",
            ],
            "the Store's alias is no candidate, and 3.12 comes before 3.9"
        );
        assert!(!found.iter().any(|exe| exe.contains("WindowsApps")), "{found:?}");
    }
}
