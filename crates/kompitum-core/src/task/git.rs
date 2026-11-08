//! Git, one call at a time from the task pump: the repo a task starts in, its
//! worktree, the diff its turn leaves, and taking the worktree away again.

use crate::rpc::RpcError;
use crate::task::decide::in_scope;
use crate::task::types::ChangedFile;
use std::path::Path;
use std::process::{Output, Stdio};
use std::time::Duration;

const LIMIT: Duration = Duration::from_secs(120);

/// `git -C dir …`: stdin closed, no console, and a child that is killed when
/// it passes 120 s.
pub async fn git(dir: &str, args: &[&str]) -> Result<Output, RpcError> {
    git_for(dir, args, LIMIT).await
}

/// `git` with its own limit, for what can take longer: a commit runs the repo's hooks.
pub async fn git_for(dir: &str, args: &[&str], limit: Duration) -> Result<Output, RpcError> {
    let mut command = std::process::Command::new("git");
    command
        .arg("-C")
        .arg(dir)
        .args(["-c", "core.quotePath=false", "-c", "color.ui=false"])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::platform::no_console(&mut command);
    let mut command = tokio::process::Command::from(command);
    command.kill_on_drop(true);
    let out = tokio::time::timeout(limit, command.output()).await.map_err(|_| {
        RpcError::new(
            "unavailable",
            format!("git {} took longer than {} s", args.join(" "), limit.as_secs()),
            "check the repository for a lock or a hook waiting on input",
        )
    })?;
    let out = out
        .map_err(|e| RpcError::new("unavailable", format!("cannot run git: {e}"), "install git and put it on PATH"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let what = if stderr.is_empty() { format!("{}", out.status) } else { stderr };
        return Err(RpcError::new("unavailable", format!("git {} failed: {what}", args.join(" ")), ""));
    }
    Ok(out)
}

/// The repo a folder belongs to, the branch `HEAD` was on ("HEAD" when
/// detached), and its commit.
pub async fn locate(folder: &str) -> Result<(String, String, String), RpcError> {
    let top = git(folder, &["rev-parse", "--show-toplevel"]).await.map_err(|e| {
        RpcError::new(
            "not_a_repo",
            format!("{folder} is not inside a git repository: {}", e.message),
            "start the task in a repo, or give --in the folder that holds it",
        )
    })?;
    let repo = String::from_utf8_lossy(&top.stdout).trim().to_string();
    let branch = git(&repo, &["rev-parse", "--abbrev-ref", "HEAD"]).await?;
    let head = git(&repo, &["rev-parse", "HEAD"]).await?;
    Ok((
        repo,
        String::from_utf8_lossy(&branch.stdout).trim().to_string(),
        String::from_utf8_lossy(&head.stdout).trim().to_string(),
    ))
}

/// A file as a commit has it, or None when that commit doesn't have it. A bad commit is an error.
pub async fn show_file(repo: &str, rev: &str, path: &str) -> Result<Option<Vec<u8>>, RpcError> {
    let listed = git(repo, &["ls-tree", rev, "--", path]).await?;
    if listed.stdout.is_empty() {
        return Ok(None);
    }
    let spec = format!("{rev}:{path}");
    Ok(Some(git(repo, &["show", &spec]).await?.stdout))
}

/// Uncommitted or untracked entries: a worker's worktree starts without them.
pub async fn dirty_count(repo: &str) -> Result<usize, RpcError> {
    let out = git(repo, &["status", "--porcelain"]).await?;
    Ok(String::from_utf8_lossy(&out.stdout).lines().filter(|line| !line.trim().is_empty()).count())
}

pub async fn add_worktree(repo: &str, path: &str, branch: &str, base: &str) -> Result<(), RpcError> {
    git(repo, &["worktree", "add", "-b", branch, path, base]).await.map(|_| ())
}

/// What taking a worktree away managed, and what it left behind.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Removal {
    pub removed: bool,
    /// The paths or branches still there, in words.
    pub left: Vec<String>,
}

/// See `remove_in`: the allowed root is this core's `<state>/worktrees`.
pub async fn remove_worktree(repo: &str, path: Option<&str>, branch: Option<&str>) -> Removal {
    let root = crate::paths::Paths::from_env().ok().map(|paths| paths.state.join("worktrees"));
    remove_in(repo, path, branch, root.as_deref()).await
}

/// `_remove_worktree`: three tries at 1, 2 and 4 s, then the folder itself.
/// A path outside the allowed root is never force-removed, and what stays is
/// reported.
async fn remove_in(repo: &str, path: Option<&str>, branch: Option<&str>, root: Option<&Path>) -> Removal {
    let mut removal = Removal { removed: true, left: Vec::new() };
    if let Some(path) = path.filter(|path| Path::new(path).exists()) {
        if !allowed(path, root) {
            removal.removed = false;
            removal.left.push(format!(
                "worktree {path} (outside {}/worktrees)",
                root.map_or("the state folder".into(), |r| r.display().to_string())
            ));
        } else {
            let mut gone = false;
            for delay in [1u64, 2, 4] {
                let _ = git(repo, &["worktree", "remove", "--force", path]).await;
                if !Path::new(path).exists() {
                    gone = true;
                    break;
                }
                tokio::time::sleep(Duration::from_secs(delay)).await;
            }
            if !gone
                && Path::new(path).exists()
                && let Ok(()) = std::fs::remove_dir_all(path)
                && !Path::new(path).exists()
            {
                gone = true;
            }
            if !gone {
                removal.removed = false;
                removal.left.push(format!("worktree {path}"));
            }
        }
    }
    let _ = git(repo, &["worktree", "prune"]).await;
    if let Some(branch) = branch {
        let exists = git(repo, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")]).await;
        if exists.is_ok() && git(repo, &["branch", "-D", branch]).await.is_err() {
            removal.removed = false;
            removal.left.push(format!("branch {branch}"));
        }
    }
    removal
}

fn allowed(path: &str, root: Option<&Path>) -> bool {
    let Some(root) = root else { return false };
    let real = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    real(Path::new(path)).starts_with(real(root))
}

/// `_changed_files`: tracked changes, plus untracked files made visible with
/// `add -N`, as numstat against the commit the worktree was cut from. A binary
/// file's `-` counts as zero. Each path is marked against the task's scope.
pub async fn changed_files(worktree: &str, base: &str, scope: &[String]) -> Result<Vec<ChangedFile>, RpcError> {
    git(worktree, &["add", "-N", "."]).await?;
    let out = git(worktree, &["diff", "--numstat", "--no-renames", base]).await?;
    let number = |value: &str| value.parse::<u64>().unwrap_or(0);
    let mut files: Vec<ChangedFile> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(3, '\t');
            let added = fields.next()?;
            let removed = fields.next()?;
            let path = fields.next()?.to_string();
            Some(ChangedFile { in_scope: in_scope(&path, scope), path, added: number(added), removed: number(removed) })
        })
        .collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kom-git-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn run(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git").current_dir(dir).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }

    fn repo(name: &str) -> PathBuf {
        let dir = scratch(name);
        run(&dir, &["init", "-q"]);
        run(&dir, &["config", "user.name", "t"]);
        run(&dir, &["config", "user.email", "t@t"]);
        std::fs::write(dir.join("a.txt"), "one\n").unwrap();
        run(&dir, &["add", "."]);
        run(&dir, &["commit", "-q", "-m", "first"]);
        dir
    }

    #[tokio::test]
    async fn a_worktree_is_cut_from_head_and_its_changes_are_marked() {
        let dir = repo("changed");
        let repo = dir.display().to_string();
        let (top, branch, base) = locate(&repo).await.unwrap();
        assert_eq!(Path::new(&top).canonicalize().unwrap(), dir.canonicalize().unwrap());
        assert!(!branch.is_empty() && base.len() >= 40);
        let worktrees = dir.join("state").join("worktrees");
        let wt = worktrees.join("fix-1");
        let wt_text = wt.display().to_string();
        add_worktree(&repo, &wt_text, "task/fix-1", &base).await.unwrap();
        std::fs::write(wt.join("a.txt"), "one\ntwo\n").unwrap();
        std::fs::create_dir_all(wt.join("src")).unwrap();
        std::fs::write(wt.join("src").join("new.rs"), "x\n").unwrap();
        let changed = changed_files(&wt_text, &base, &[String::from("src/**")]).await.unwrap();
        assert_eq!(changed.len(), 2, "{changed:?}");
        let a = changed.iter().find(|f| f.path == "a.txt").unwrap();
        assert_eq!((a.added, a.removed, a.in_scope), (1, 0, false));
        let new = changed.iter().find(|f| f.path == "src/new.rs").unwrap();
        assert_eq!((new.added, new.removed, new.in_scope), (1, 0, true));

        let removal = remove_in(&repo, Some(&wt_text), Some("task/fix-1"), Some(&worktrees)).await;
        assert!(removal.removed, "{removal:?}");
        assert!(removal.left.is_empty());
        assert!(!wt.exists());
        assert!(git(&repo, &["rev-parse", "--verify", "--quiet", "refs/heads/task/fix-1"]).await.is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn a_path_outside_the_worktrees_folder_is_never_removed() {
        let dir = repo("outside");
        let repo = dir.display().to_string();
        let (_, _, base) = locate(&repo).await.unwrap();
        let outside = dir.join("elsewhere").join("wt");
        let text = outside.display().to_string();
        add_worktree(&repo, &text, "task/outside", &base).await.unwrap();
        let removal =
            remove_in(&repo, Some(&text), Some("task/outside"), Some(&dir.join("state").join("worktrees"))).await;
        assert!(!removal.removed);
        assert!(outside.exists(), "it stays for the person to look at");
        assert!(removal.left.iter().any(|left| left.contains("outside")), "{removal:?}");
        assert!(
            removal.left.iter().any(|left| left.contains("task/outside")),
            "a branch still checked out there is reported too: {removal:?}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_folder_that_is_not_a_repo_is_not_a_repo() {
        let dir = scratch("plain");
        let text = dir.display().to_string();
        let error = locate(&text).await.unwrap_err();
        assert_eq!(error.kind, "not_a_repo");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
