//! The shared workshop: one long-lived checkout per repository that several tasks edit at the same
//! time, each inside its own scope. No worktree and no cold build per task. A task's work is its
//! scope's files there; accept takes those to the repository and discard puts them back.

use crate::rpc::RpcError;
use crate::task::decide::{in_scope, scopes_overlap};
use crate::task::git::git;
use crate::task::types::{ChangedFile, Task, TaskState};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Where a repository's workshop lives: `<state>/workshops/<folder>-<id>`, the id from the
/// repository's path so two repositories with one folder name never share.
pub fn path(state: &Path, repo: &str) -> PathBuf {
    let normal = repo.replace('\\', "/");
    let trimmed = normal.trim_end_matches('/');
    let folder = match trimmed.rsplit('/').next() {
        Some(last) if !last.is_empty() => last,
        _ => repo,
    };
    // The id reads the path in lower case: Windows paths differ only in case, and they are one folder.
    let id = fnv1a(&trimmed.to_lowercase());
    state.join("workshops").join(format!("{folder}-{id:016x}"))
}

/// FNV-1a, 64 bit: a short, stable hash, so the workshop's name never changes between runs.
fn fnv1a(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Its files are in the workshop now: it has started there and is neither accepted nor discarded.
pub fn holds(task: &Task) -> bool {
    task.shared && task.worktree.is_some() && !matches!(task.state, TaskState::Accepted | TaskState::Discarded)
}

/// Whether a scope touches the files only one task at a time may write.
pub fn needs_one_writer(scope: &[String], one_writer: &[String]) -> bool {
    !one_writer.is_empty() && scopes_overlap(scope, one_writer)
}

/// Why a task that shares the workshop may not start yet, beyond the queue's own rules.
pub fn may_start(task: &Task, others: &[&Task]) -> Result<(), String> {
    if !task.shared {
        return Ok(());
    }
    for other in others {
        if other.name == task.name || other.repo != task.repo || !other.shared {
            continue;
        }
        if holds(other) && scopes_overlap(&task.scope, &other.scope) {
            return Err(format!(
                "waiting for {}: its files in the workshop are not accepted or discarded yet",
                other.name
            ));
        }
        if !(task.one_writer && other.one_writer) {
            continue;
        }
        // A finished writer's work is whole, so it no longer holds the place; a queued one goes by its number.
        let holding = matches!(
            other.state,
            TaskState::Running
                | TaskState::Checking
                | TaskState::Asking
                | TaskState::Failed
                | TaskState::TimedOut
                | TaskState::Cancelled
        );
        if holds(other) && holding {
            return Err(format!("waiting for {}: the workshop takes one writer at a time for these files", other.name));
        }
        if matches!(other.state, TaskState::Queued | TaskState::Stranded) && other.seq < task.seq {
            return Err(format!(
                "queued behind {}: the workshop takes one writer at a time for these files",
                other.name
            ));
        }
    }
    Ok(())
}

/// The changed paths no task's scope admits: someone wrote outside their files.
pub fn strays(changed: &[String], scopes: &[&[String]]) -> Vec<String> {
    changed.iter().filter(|path| !scopes.iter().any(|scope| in_scope(path.as_str(), scope))).cloned().collect()
}

/// `git` in the workshop, tried again when another git held the index for a moment.
async fn git_retry(dir: &str, args: &[&str]) -> Result<std::process::Output, RpcError> {
    let mut tries = 1;
    loop {
        match git(dir, args).await {
            Err(e) if tries < 3 && e.message.contains("index.lock") => {
                tries += 1;
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
            other => return other,
        }
    }
}

/// The commit a checkout is on.
pub async fn head(dir: &str) -> Result<String, RpcError> {
    let out = git_retry(dir, &["rev-parse", "HEAD"]).await?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Makes the workshop when it isn't there: a detached worktree of the repository at its HEAD.
pub async fn ensure(repo: &str, path: &str) -> Result<(), RpcError> {
    if Path::new(path).join(".git").exists() {
        return Ok(());
    }
    if let Some(parent) = Path::new(path).parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| RpcError::new("unavailable", format!("cannot make {}: {e}", parent.display()), ""))?;
    }
    let _ = git(repo, &["worktree", "prune"]).await;
    git(repo, &["worktree", "add", "--detach", path, "HEAD"]).await.map(|_| ())
}

/// Before a turn: the workshop exists, and follows the repository when no other task is mid-turn
/// there. A refused follow is a warning in words, never a failed start.
pub async fn enter(repo: &str, path: &str, quiet: bool) -> Result<Option<String>, RpcError> {
    ensure(repo, path).await?;
    if !quiet {
        return Ok(None);
    }
    match sync(path, &head(repo).await?).await {
        Ok(()) => Ok(None),
        Err(e) => Ok(Some(e.message)),
    }
}

/// Moves the workshop to the repository's commit, keeping every task's uncommitted files. Refused,
/// with git's own words, when that commit changes a file a task is editing.
pub async fn sync(path: &str, to: &str) -> Result<(), RpcError> {
    if head(path).await? == to {
        return Ok(());
    }
    git_retry(path, &["checkout", "--detach", "-q", to]).await.map(|_| ()).map_err(|e| {
        let short: String = to.chars().take(8).collect();
        RpcError::new(
            "workshop_conflict",
            format!("the workshop can't follow the repository to {short}: {}", e.message),
            "accept or discard the task that holds those files, or commit them yourself",
        )
    })
}

/// After an accept: the workshop's commit becomes the repository's new one, files untouched, so the
/// accepted files read as unchanged and every other task's edits stay.
pub async fn follow(path: &str, to: &str) -> Result<(), RpcError> {
    git_retry(path, &["reset", "-q", to]).await.map(|_| ())
}

/// What a task changed in the workshop: the changed files its scope admits, with their line counts,
/// and every other changed path. Reads only: the index is not touched.
pub async fn changed(path: &str, scope: &[String]) -> Result<(Vec<ChangedFile>, Vec<String>), RpcError> {
    let tracked = git_retry(path, &["diff", "--numstat", "--no-renames", "HEAD"]).await?;
    let mut files = numstat(&String::from_utf8_lossy(&tracked.stdout));
    let untracked = git_retry(path, &["ls-files", "--others", "--exclude-standard", "-z"]).await?;
    for file in split_z(&untracked.stdout) {
        let added = line_count(&Path::new(path).join(&file));
        files.push((file, added, 0));
    }
    let (mut mine, mut others) = (Vec::new(), Vec::new());
    for (file, added, removed) in files {
        if in_scope(&file, scope) {
            mine.push(ChangedFile { path: file, added, removed, in_scope: true });
        } else {
            others.push(file);
        }
    }
    mine.sort_by(|a, b| a.path.cmp(&b.path));
    others.sort();
    Ok((mine, others))
}

/// The files a task has changed in the workshop right now, by path.
pub async fn files_of(path: &str, scope: &[String]) -> Result<Vec<String>, RpcError> {
    Ok(changed(path, scope).await?.0.into_iter().map(|file| file.path).collect())
}

/// After an accept. `before` is the repository's commit before the accept, `after` the new commit
/// (None when nothing was committed). The accepted files end up reading as unchanged.
pub async fn settle(
    path: &str,
    files: &[String],
    before: &str,
    after: Option<&str>,
) -> Result<Option<String>, RpcError> {
    let Some(after) = after else {
        restore(path, files).await?;
        return Ok(None);
    };
    if head(path).await? != before {
        if let Err(e) = sync(path, before).await {
            restore(path, files).await?;
            return Ok(Some(e.message));
        }
    }
    follow(path, after).await?;
    Ok(None)
}

/// `git diff --numstat` lines as (path, added, removed); a binary file's `-` counts as zero.
fn numstat(text: &str) -> Vec<(String, u64, u64)> {
    let number = |value: &str| value.parse::<u64>().unwrap_or(0);
    text.lines()
        .filter_map(|line| {
            let mut fields = line.splitn(3, '\t');
            let added = fields.next()?;
            let removed = fields.next()?;
            let path = fields.next()?.to_string();
            Some((path, number(added), number(removed)))
        })
        .collect()
}

/// The lines of a file on disk, for an untracked file: a last line without `\n` counts, and a file
/// that can't be read or holds a NUL byte counts none.
fn line_count(file: &Path) -> u64 {
    let Ok(bytes) = std::fs::read(file) else { return 0 };
    if bytes.contains(&0) {
        return 0;
    }
    let newlines = bytes.iter().filter(|byte| **byte == b'\n').count() as u64;
    newlines + u64::from(!bytes.is_empty() && bytes.last() != Some(&b'\n'))
}

/// `-z` output: NUL-ended paths, the empty ones dropped.
fn split_z(bytes: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(bytes).split('\0').filter(|item| !item.is_empty()).map(str::to_string).collect()
}

/// The untracked files among `files`, as `ls-files --others` names them.
async fn untracked_among(path: &str, files: &[String]) -> Result<Vec<String>, RpcError> {
    let mut args = vec!["ls-files", "--others", "--exclude-standard", "-z", "--"];
    args.extend(files.iter().map(String::as_str));
    let out = git_retry(path, &args).await?;
    Ok(split_z(&out.stdout))
}

/// A task's files as one patch against the workshop's commit. `flags` go right after `diff`: an
/// accept passes `--binary --full-index` so `git apply` can take it, the checks `-U0 --no-color`,
/// reading passes none.
pub async fn patch(path: &str, files: &[String], flags: &[&str]) -> Result<Vec<u8>, RpcError> {
    if files.is_empty() {
        return Ok(Vec::new());
    }
    let added = untracked_among(path, files).await?;
    if !added.is_empty() {
        let mut args = vec!["add", "-N", "--"];
        args.extend(added.iter().map(String::as_str));
        git_retry(path, &args).await?;
    }
    let mut args = vec!["diff"];
    args.extend(flags.iter().copied());
    args.extend(["--no-renames", "HEAD", "--"]);
    args.extend(files.iter().map(String::as_str));
    let diff = git_retry(path, &args).await;
    if added.is_empty() {
        return diff.map(|out| out.stdout);
    }
    // The index goes back as it was, even when the diff failed; the diff's own error is the one said.
    let mut undo = vec!["reset", "-q", "--"];
    undo.extend(added.iter().map(String::as_str));
    let undone = git_retry(path, &undo).await;
    let out = diff?;
    undone?;
    Ok(out.stdout)
}

/// Puts a task's files back as the workshop's commit has them: a discard. A file the commit doesn't
/// have is deleted, and folders it leaves empty go with it.
pub async fn restore(path: &str, files: &[String]) -> Result<(), RpcError> {
    if files.is_empty() {
        return Ok(());
    }
    // A worker's `git add` would keep a file in the index: unstage first, so the listing below sees it.
    let mut unstage = vec!["reset", "-q", "--"];
    unstage.extend(files.iter().map(String::as_str));
    git_retry(path, &unstage).await?;
    let added = untracked_among(path, files).await?;
    let root = Path::new(path);
    for file in &added {
        let full = root.join(file);
        match std::fs::remove_file(&full) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(RpcError::new("unavailable", format!("cannot remove {file}: {e}"), "")),
        }
        // Folders the file leaves empty go too, up to the workshop's root, which is never removed.
        let mut dir = full.parent();
        while let Some(folder) = dir.filter(|folder| *folder != root) {
            if std::fs::remove_dir(folder).is_err() {
                break;
            }
            dir = folder.parent();
        }
    }
    let tracked: Vec<&String> = files.iter().filter(|file| !added.contains(*file)).collect();
    if !tracked.is_empty() {
        let mut args = vec!["checkout", "-q", "HEAD", "--"];
        args.extend(tracked.iter().map(|file| file.as_str()));
        git_retry(path, &args).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn scopes(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn task(name: &str, state: TaskState, seq: u64, scope: &[&str]) -> Task {
        Task { name: name.into(), seq, state, repo: "/repo".into(), scope: scopes(scope), ..Default::default() }
    }

    /// A task that shares the workshop: shared, with a worktree of its own only once it has started.
    fn shared(name: &str, state: TaskState, seq: u64, scope: &[&str]) -> Task {
        Task { shared: true, worktree: Some("/ws".into()), ..task(name, state, seq, scope) }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kom-workshop-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn run(dir: &Path, args: &[&str]) {
        let out = Command::new("git").current_dir(dir).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }

    fn repo(name: &str) -> PathBuf {
        let dir = scratch(name);
        run(&dir, &["init", "-q"]);
        run(&dir, &["config", "user.name", "t"]);
        run(&dir, &["config", "user.email", "t@t"]);
        run(&dir, &["config", "core.autocrlf", "false"]);
        std::fs::write(dir.join("a.txt"), "one\n").unwrap();
        run(&dir, &["add", "."]);
        run(&dir, &["commit", "-q", "-m", "first"]);
        dir
    }

    /// The scratch repository `main`: `a.txt` and `c.txt`, both committed.
    fn main_repo(name: &str) -> PathBuf {
        let dir = repo(name);
        std::fs::write(dir.join("c.txt"), "c one\n").unwrap();
        run(&dir, &["add", "."]);
        run(&dir, &["commit", "-q", "-m", "second"]);
        dir
    }

    /// `git status --porcelain` in a folder, as text.
    fn status_of(dir: &str) -> String {
        let out = Command::new("git").current_dir(dir).args(["status", "--porcelain"]).output().unwrap();
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    /// The edits the git tests share: `a.txt` changed, a new source file and a new note.
    fn edit_workshop(ws: &Path) {
        std::fs::write(ws.join("a.txt"), "one\ntwo\n").unwrap();
        std::fs::create_dir_all(ws.join("src")).unwrap();
        std::fs::write(ws.join("src").join("new.rs"), "x\ny\n").unwrap();
        std::fs::create_dir_all(ws.join("notes")).unwrap();
        std::fs::write(ws.join("notes").join("x.md"), "note\n").unwrap();
    }

    #[test]
    fn a_workshop_lives_under_the_state_folder_by_the_repositorys_name_and_path() {
        let state = Path::new("state");
        let mine = path(state, "C:\\Work\\App");
        let name = mine.file_name().unwrap().to_str().unwrap().to_string();
        let (folder, id) = name.rsplit_once('-').unwrap();
        assert_eq!(folder, "App", "the last path component names it");
        assert_eq!(id.len(), 16, "{id}");
        assert!(id.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)), "lower-case hex: {id}");
        assert_eq!(mine.parent().unwrap(), state.join("workshops"));
        assert_eq!(path(state, "c:/Work/App/"), mine, "separators, case and a trailing slash change nothing");
        // The brief's pair spells the folder in two cases: the id still agrees, the folder keeps its case.
        let other_case = path(state, "c:/work/app/");
        assert_eq!(other_case.file_name().unwrap().to_str().unwrap().rsplit_once('-').unwrap().1, id);
        assert_ne!(path(state, "D:\\Other\\App"), mine, "another repository never shares a workshop");
    }

    #[test]
    fn a_started_shared_task_holds_its_files_until_accepted_or_discarded() {
        let mut task = shared("a", TaskState::Running, 1, &["src/**"]);
        assert!(holds(&task));
        task.shared = false;
        assert!(!holds(&task), "a task that does not share holds nothing");
        task.shared = true;
        task.worktree = None;
        assert!(!holds(&task), "no worktree, no files in the workshop");
        for (state, expected) in [
            (TaskState::Done, true),
            (TaskState::Failed, true),
            (TaskState::Accepted, false),
            (TaskState::Discarded, false),
        ] {
            let held = shared("a", state, 1, &["src/**"]);
            assert_eq!(holds(&held), expected, "{state:?}");
        }
    }

    #[test]
    fn a_shared_task_waits_for_unaccepted_files_its_scope_overlaps() {
        let finished = shared("held", TaskState::Done, 1, &["src/a/**"]);
        let queued = shared("next", TaskState::Queued, 2, &["src/a/x.rs"]);
        assert_eq!(
            may_start(&queued, &[&finished]),
            Err("waiting for held: its files in the workshop are not accepted or discarded yet".into())
        );
        let disjoint = shared("docs", TaskState::Queued, 3, &["docs/**"]);
        assert_eq!(may_start(&disjoint, &[&finished]), Ok(()), "disjoint scopes start");
        let elsewhere = Task { repo: "/elsewhere".into(), ..shared("far", TaskState::Done, 1, &["src/a/**"]) };
        assert_eq!(may_start(&queued, &[&elsewhere]), Ok(()), "a holder in another repository is ignored");
        let plain = task("plain", TaskState::Queued, 4, &["src/a/x.rs"]);
        assert_eq!(may_start(&plain, &[&finished]), Ok(()), "a task that is not shared always starts");
        let accepted = shared("gone", TaskState::Accepted, 1, &["src/a/**"]);
        assert_eq!(may_start(&queued, &[&accepted]), Ok(()), "accepted files are in the repository");
    }

    #[test]
    fn one_writer_tasks_run_one_at_a_time_and_a_done_one_frees_the_place() {
        // The writer under test: one that may write under the one-writer globs, in src/, starting now.
        let writer = |name: &str, state: TaskState, seq: u64, scope: &[&str]| Task {
            one_writer: true,
            ..shared(name, state, seq, scope)
        };
        let next = writer("next", TaskState::Queued, 5, &["src/**"]);
        let text = "the workshop takes one writer at a time for these files";
        // Their scopes are in lib/, away from src/: only the one-writer rule can hold this task.
        for state in [TaskState::Running, TaskState::Asking, TaskState::Failed] {
            let holder = writer("h", state, 1, &["lib/**"]);
            assert_eq!(may_start(&next, &[&holder]), Err(format!("waiting for h: {text}")), "{state:?}");
        }
        let done = writer("d", TaskState::Done, 1, &["lib/**"]);
        assert_eq!(may_start(&next, &[&done]), Ok(()), "a done writer's work is whole");
        let queued_earlier = Task { worktree: None, ..writer("q", TaskState::Queued, 2, &["lib/**"]) };
        assert_eq!(may_start(&next, &[&queued_earlier]), Err(format!("queued behind q: {text}")));
        let queued_later = Task { worktree: None, ..writer("l", TaskState::Queued, 9, &["lib/**"]) };
        assert_eq!(may_start(&next, &[&queued_later]), Ok(()), "a later writer waits for no one");
        let not_a_writer = shared("n", TaskState::Running, 1, &["lib/**"]);
        assert_eq!(may_start(&next, &[&not_a_writer]), Ok(()), "a holder that is not a writer is not held");
    }

    #[test]
    fn needs_one_writer_and_strays_are_about_the_scope_and_the_globs() {
        let globs = scopes(&["src/core/**"]);
        assert!(needs_one_writer(&scopes(&["src/core/x.rs"]), &globs));
        assert!(needs_one_writer(&scopes(&["src/**"]), &globs), "a wider scope overlaps too");
        assert!(!needs_one_writer(&scopes(&["docs/a.md"]), &globs));
        assert!(!needs_one_writer(&scopes(&["src/core/x.rs"]), &[]), "no globs, no place to wait for");
        let src = scopes(&["src/**"]);
        let notes = scopes(&["notes/**"]);
        let changed = scopes(&["a.txt", "src/x.rs", "notes/y.md"]);
        assert_eq!(strays(&changed, &[src.as_slice(), notes.as_slice()]), ["a.txt"]);
        assert_eq!(strays(&changed, &[]), changed, "with no scope every path is a stray");
        assert!(in_scope("src/x.rs", &src));
    }

    #[tokio::test]
    async fn the_workshop_is_a_detached_checkout_made_once() {
        let dir = main_repo("made-once");
        let main = dir.display().to_string();
        let ws = dir.join("state").join("workshops").join("x").display().to_string();
        ensure(&main, &ws).await.unwrap();
        ensure(&main, &ws).await.unwrap();
        assert_eq!(head(&ws).await.unwrap(), head(&main).await.unwrap());
        assert!(Path::new(&ws).join("a.txt").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_tasks_changes_are_its_scopes_files_and_the_rest_are_named() {
        let dir = main_repo("changes");
        let main = dir.display().to_string();
        let ws = dir.join("state").join("workshops").join("x");
        ensure(&main, &ws.display().to_string()).await.unwrap();
        edit_workshop(&ws);
        let ws = ws.display().to_string();
        let before = status_of(&ws);
        let (mine, others) = changed(&ws, &scopes(&["src/**"])).await.unwrap();
        assert_eq!(mine, [ChangedFile { path: "src/new.rs".into(), added: 2, removed: 0, in_scope: true }]);
        assert_eq!(others, ["a.txt", "notes/x.md"]);
        assert_eq!(status_of(&ws), before, "the index was not touched");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_patch_holds_only_the_tasks_files_and_applies_to_the_repository() {
        let dir = main_repo("patch");
        let main = dir.display().to_string();
        let ws = dir.join("state").join("workshops").join("x");
        ensure(&main, &ws.display().to_string()).await.unwrap();
        edit_workshop(&ws);
        let ws = ws.display().to_string();
        let before = status_of(&ws);
        let bytes = patch(&ws, &scopes(&["src/new.rs"]), &["--binary", "--full-index"]).await.unwrap();
        assert_eq!(status_of(&ws), before, "patch leaves the workshop's index as it was");
        let file = dir.join("state").join("task.patch");
        std::fs::write(&file, &bytes).unwrap();
        let file_text = file.display().to_string();
        run(&dir, &["apply", file_text.as_str()]);
        assert_eq!(std::fs::read_to_string(dir.join("src").join("new.rs")).unwrap(), "x\ny\n");
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "one\n", "a.txt is left alone there");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn after_an_accept_the_workshop_follows_and_other_edits_stay() {
        let dir = main_repo("follow");
        let main = dir.display().to_string();
        let ws = dir.join("state").join("workshops").join("x");
        ensure(&main, &ws.display().to_string()).await.unwrap();
        edit_workshop(&ws);
        let ws = ws.display().to_string();
        let bytes = patch(&ws, &scopes(&["src/new.rs"]), &["--binary", "--full-index"]).await.unwrap();
        let file = dir.join("state").join("task.patch");
        std::fs::write(&file, &bytes).unwrap();
        let file_text = file.display().to_string();
        run(&dir, &["apply", file_text.as_str()]);
        run(&dir, &["add", "src/new.rs"]);
        run(&dir, &["commit", "-q", "-m", "accepted"]);
        let new_head = head(&main).await.unwrap();
        follow(&ws, &new_head).await.unwrap();
        let (mine, others) = changed(&ws, &scopes(&["**"])).await.unwrap();
        let listed: Vec<String> = mine.iter().map(|f| f.path.clone()).chain(others).collect();
        assert_eq!(listed, ["a.txt", "notes/x.md"], "the accepted file reads as unchanged");
        assert_eq!(head(&ws).await.unwrap(), new_head);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn the_workshop_syncs_to_a_new_commit_and_keeps_edits_but_refuses_a_conflict() {
        let dir = main_repo("sync");
        let main = dir.display().to_string();
        let ws = dir.join("state").join("workshops").join("x");
        ensure(&main, &ws.display().to_string()).await.unwrap();
        let ws = ws.display().to_string();
        std::fs::write(Path::new(&ws).join("a.txt"), "one\nedit\n").unwrap();
        std::fs::write(dir.join("c.txt"), "c two\n").unwrap();
        run(&dir, &["commit", "-q", "-am", "c moves"]);
        let first = head(&main).await.unwrap();
        sync(&ws, &first).await.unwrap();
        assert_eq!(std::fs::read_to_string(Path::new(&ws).join("c.txt")).unwrap(), "c two\n");
        assert_eq!(std::fs::read_to_string(Path::new(&ws).join("a.txt")).unwrap(), "one\nedit\n");
        assert_eq!(head(&ws).await.unwrap(), first);

        std::fs::write(dir.join("a.txt"), "one\nmain\n").unwrap();
        run(&dir, &["commit", "-q", "-am", "a moves"]);
        let second = head(&main).await.unwrap();
        let error = sync(&ws, &second).await.unwrap_err();
        assert_eq!(error.kind, "workshop_conflict");
        assert!(error.message.contains("a.txt"), "{}", error.message);
        assert_eq!(std::fs::read_to_string(Path::new(&ws).join("a.txt")).unwrap(), "one\nedit\n");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn restore_puts_files_back_and_removes_new_ones() {
        let dir = main_repo("restore");
        let main = dir.display().to_string();
        let ws = dir.join("state").join("workshops").join("x");
        ensure(&main, &ws.display().to_string()).await.unwrap();
        let ws_text = ws.display().to_string();
        std::fs::write(ws.join("a.txt"), "one\ntwo\n").unwrap();
        std::fs::create_dir_all(ws.join("src").join("deep")).unwrap();
        std::fs::write(ws.join("src").join("deep").join("new.rs"), "x\n").unwrap();
        run(&ws, &["add", "src/deep/new.rs"]);
        restore(&ws_text, &scopes(&["a.txt", "src/deep/new.rs"])).await.unwrap();
        assert_eq!(std::fs::read_to_string(ws.join("a.txt")).unwrap(), "one\n");
        assert!(!ws.join("src").exists(), "the folders it left empty go too");
        assert_eq!(status_of(&ws_text), "", "the workshop is clean again");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Accepts `files` the way the core does: the workshop's patch applied to the repository and
    /// committed. Returns the repository's commit before the accept and the one after.
    async fn accept_by_hand(dir: &Path, ws: &str, files: &[String]) -> (String, String) {
        let before = head(&dir.display().to_string()).await.unwrap();
        let bytes = patch(ws, files, &["--binary", "--full-index"]).await.unwrap();
        let file = dir.join("state").join("task.patch");
        std::fs::write(&file, &bytes).unwrap();
        let file_text = file.display().to_string();
        run(dir, &["apply", file_text.as_str()]);
        for accepted in files {
            run(dir, &["add", accepted.as_str()]);
        }
        run(dir, &["commit", "-q", "-m", "accepted"]);
        (before, head(&dir.display().to_string()).await.unwrap())
    }

    #[tokio::test]
    async fn settle_leaves_the_accepted_file_clean_and_another_edit_changed() {
        let dir = main_repo("settle");
        let main = dir.display().to_string();
        let ws = dir.join("state").join("workshops").join("x");
        ensure(&main, &ws.display().to_string()).await.unwrap();
        edit_workshop(&ws);
        let ws = ws.display().to_string();
        let accepted = scopes(&["src/new.rs"]);
        let (before, after) = accept_by_hand(&dir, &ws, &accepted).await;
        assert_eq!(settle(&ws, &accepted, &before, Some(after.as_str())).await.unwrap(), None);
        let (mine, others) = changed(&ws, &scopes(&["**"])).await.unwrap();
        let listed: Vec<String> = mine.iter().map(|f| f.path.clone()).chain(others).collect();
        assert_eq!(listed, ["a.txt", "notes/x.md"], "the accepted file is clean, the rest stay changed");
        assert_eq!(head(&ws).await.unwrap(), after);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn settle_refuses_a_repository_commit_that_meets_an_edit_and_keeps_the_other_edit() {
        let dir = main_repo("settle-refused");
        let main = dir.display().to_string();
        let ws = dir.join("state").join("workshops").join("x");
        ensure(&main, &ws.display().to_string()).await.unwrap();
        edit_workshop(&ws);
        let ws = ws.display().to_string();
        let workshop_head = head(&ws).await.unwrap();
        // The repository moves on by a commit of its own that changes a.txt, which the workshop edited.
        std::fs::write(dir.join("a.txt"), "one\nmain\n").unwrap();
        run(&dir, &["commit", "-q", "-am", "a moves"]);
        let accepted = scopes(&["src/new.rs"]);
        let (before, after) = accept_by_hand(&dir, &ws, &accepted).await;
        let warning = settle(&ws, &accepted, &before, Some(after.as_str())).await.unwrap();
        assert!(warning.as_deref().is_some_and(|text| text.contains("a.txt")), "{warning:?}");
        assert_eq!(head(&ws).await.unwrap(), workshop_head, "the workshop stays behind");
        assert!(!Path::new(&ws).join("src").join("new.rs").exists(), "the accepted file is back as HEAD has it");
        assert_eq!(std::fs::read_to_string(Path::new(&ws).join("a.txt")).unwrap(), "one\ntwo\n");
        assert!(Path::new(&ws).join("notes").join("x.md").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn settle_without_a_commit_restores_only_the_accepted_files() {
        let dir = main_repo("settle-none");
        let main = dir.display().to_string();
        let ws = dir.join("state").join("workshops").join("x");
        ensure(&main, &ws.display().to_string()).await.unwrap();
        edit_workshop(&ws);
        let ws = ws.display().to_string();
        let main_head = head(&main).await.unwrap();
        assert_eq!(settle(&ws, &scopes(&["src/new.rs"]), &main_head, None).await.unwrap(), None);
        assert!(!Path::new(&ws).join("src").exists(), "the accepted file is restored away");
        assert_eq!(std::fs::read_to_string(Path::new(&ws).join("a.txt")).unwrap(), "one\ntwo\n", "other edits stay");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn enter_makes_the_workshop_once_and_follows_the_repository_when_quiet() {
        let dir = main_repo("enter");
        let main = dir.display().to_string();
        let ws = dir.join("state").join("workshops").join("x").display().to_string();
        assert_eq!(enter(&main, &ws, false).await.unwrap(), None);
        assert_eq!(enter(&main, &ws, false).await.unwrap(), None, "a second start is fine");
        std::fs::write(dir.join("c.txt"), "c two\n").unwrap();
        run(&dir, &["commit", "-q", "-am", "c moves"]);
        let moved = head(&main).await.unwrap();
        assert_eq!(enter(&main, &ws, true).await.unwrap(), None);
        assert_eq!(head(&ws).await.unwrap(), moved);
        let _ = std::fs::remove_dir_all(dir);
    }
}
