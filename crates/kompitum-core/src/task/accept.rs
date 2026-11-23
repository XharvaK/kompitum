//! `kom task accept` and `kom task redo`: bring a task's changes in, or send it back.

use crate::api::{TaskAcceptParams, TaskAcceptResult, TaskRedoParams, TaskRedoResult};
use crate::rpc::RpcError;
use crate::runtime::{Core, now_ms};
use crate::task::git;
use crate::task::parts::{self, PartState};
use crate::task::types::{Task, TaskState};
use crate::task::workshop;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

/// The two ways an accept can be told to go on: past a bad end, and past a scope violation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AcceptFlags {
    pub accept_unfinished: bool,
    pub accept_scope_violation: bool,
}

/// Why a task can't come in, in the order the refusals are checked.
pub fn refusal(task: &crate::task::types::Task, flags: &AcceptFlags) -> Option<RpcError> {
    let name = &task.name;
    if matches!(task.state, TaskState::Accepted | TaskState::Discarded) {
        return Some(RpcError::new(
            "finished",
            format!("{name} has already ended: {}", task.state.word()),
            "see `kom task show`",
        ));
    }
    if task.state == TaskState::Asking {
        return Some(RpcError::new(
            "asking",
            format!("cannot accept {name}: it is asking a question: answer it with kom task redo, or discard it"),
            format!("answer on stdin: kom task redo {name} --brief -"),
        ));
    }
    if !task.state.ended() {
        return Some(RpcError::new(
            "running",
            format!("{name} is still {}: cancel it first", task.state.word()),
            format!("kom task cancel {name}"),
        ));
    }
    if !flags.accept_unfinished {
        if task.state.ended_badly() {
            return Some(RpcError::new(
                "unfinished",
                format!("{name} ended {}: review it, then accept it with accept_unfinished", task.state.word()),
                "pass accept_unfinished=true after review",
            ));
        }
        if let Some(report) = task.checks.last().filter(|report| !report.ok) {
            return Some(RpcError::new(
                "unfinished",
                format!(
                    "{name}'s last checks failed ({}): review it, then accept it with accept_unfinished",
                    report.words()
                ),
                "pass accept_unfinished=true after review",
            ));
        }
    }
    if !flags.accept_scope_violation {
        let outside: Vec<&str> =
            task.changed.iter().filter(|file| !file.in_scope).map(|file| file.path.as_str()).collect();
        if !outside.is_empty() {
            return Some(RpcError::new(
                "scope_violation",
                format!("{name} changed files outside its scope: {}", outside.join(", ")),
                "pass accept_scope_violation=true after review",
            ));
        }
    }
    if task.worktree.as_deref().is_none_or(str::is_empty) {
        return Some(no_worktree(name));
    }
    None
}

fn no_worktree(name: &str) -> RpcError {
    RpcError::new(
        "no_worktree",
        format!("{name} has no worktree: its changes are gone"),
        format!("see `kom task show {name}`"),
    )
}

fn no_changes(name: &str) -> RpcError {
    RpcError::new("no_changes", format!("{name} changed nothing in its worktree"), format!("kom task discard {name}"))
}

fn commit_failed(name: &str, words: &str) -> RpcError {
    RpcError::new(
        "commit_failed",
        format!("{name}'s changes are in the working tree, but the commit failed: {words}"),
        format!("the changes stay in the working tree: commit them yourself, then kom task discard {name}"),
    )
}

fn part_commit_failed(name: &str, part: &str, words: &str) -> RpcError {
    RpcError::new(
        "commit_failed",
        format!(
            "{name}'s part {part} is in the working tree, but git add or git commit failed: {words}; the other parts' commits stay"
        ),
        format!("fix the commit, then accept part {part} again"),
    )
}

fn not_applied(name: &str, words: &str) -> RpcError {
    RpcError::new(
        "conflict",
        format!("{name}'s patch does not apply: {words}"),
        "the repository was not touched; see `kom task review`",
    )
}

/// None of the patch's files has changes in the repository: a failed three-way can be rolled back.
async fn unchanged(repo: &str, files: &[String]) -> Result<bool, RpcError> {
    let mut args = vec!["status", "--porcelain", "--"];
    args.extend(files.iter().map(String::as_str));
    let out = git::git(repo, &args).await?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().is_empty())
}

/// The files a failed three-way names as conflicted, or every patch file when it names none.
fn conflicted_files(out: &std::process::Output, files: &[String]) -> Vec<String> {
    let text = format!("{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let mut found: Vec<String> = Vec::new();
    for line in text.lines() {
        let named = line.strip_prefix("U ").or_else(|| {
            line.strip_prefix("Applied patch to '").and_then(|rest| rest.strip_suffix("' with conflicts."))
        });
        let Some(file) = named.map(str::trim).filter(|file| !file.is_empty()) else { continue };
        if !found.iter().any(|seen| seen == file) {
            found.push(file.to_string());
        }
    }
    if found.is_empty() { files.to_vec() } else { found }
}

fn conflicted(name: &str, out: &std::process::Output, files: &[String]) -> RpcError {
    RpcError::new(
        "conflict",
        format!(
            "{name}'s patch conflicts in {} (a three-way apply was tried)",
            conflicted_files(out, files).join(", ")
        ),
        "the repository was not touched; see `kom task review`",
    )
}

/// Puts every patch file back as HEAD has it, and a file the patch added away from the index and
/// the disk, after a three-way apply that conflicted. A file that was on disk before the apply
/// (`before`) is never deleted: it is someone's, even when git ignores it.
async fn restore(repo: &str, rev: &str, files: &[String], before: &[bool]) -> Result<(), RpcError> {
    let mut tracked: Vec<&str> = Vec::new();
    let mut added: Vec<&str> = Vec::new();
    for (file, was_there) in files.iter().zip(before) {
        let listed = git::git(repo, &["ls-tree", "-z", rev, "--", file]).await?;
        if listed.stdout.is_empty() {
            if !was_there {
                added.push(file);
            }
        } else {
            tracked.push(file);
        }
    }
    if !tracked.is_empty() {
        let mut args = vec!["checkout", rev, "--"];
        args.extend(tracked);
        git::git(repo, &args).await?;
    }
    if !added.is_empty() {
        let mut args = vec!["rm", "-q", "--cached", "--ignore-unmatch", "--"];
        args.extend(added.iter().copied());
        git::git(repo, &args).await?;
        for file in added {
            let path = Path::new(repo).join(file);
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(RpcError::new("unavailable", format!("cannot remove {file}: {e}"), "")),
            }
        }
    }
    Ok(())
}

pub async fn accept(core: &Arc<Core>, p: TaskAcceptParams) -> Result<TaskAcceptResult, RpcError> {
    let serial = core.tasks.lock.lock().await;
    if super::stopping(core) {
        return Err(super::unavailable("the core is stopping".into()));
    }
    let Some(mut task) = core.tasks.snapshot(&p.task) else { return Err(super::not_found(&p.task)) };
    if let Some(error) = super::table::accept_refusal(&task, p.by.as_deref(), p.owner_said.as_deref()) {
        return Err(error);
    }
    let words = super::table::owner_said_words(p.owner_said.as_deref()).map(String::from);
    if p.review.is_some() || words.is_some() {
        if let Some(review) = p.review.clone() {
            task.review = Some(review);
        }
        if words.is_some() {
            task.owner_said = words;
        }
        super::save(&core.paths.state, &task);
        super::replace(core, task.clone());
    }
    let flags =
        AcceptFlags { accept_unfinished: p.accept_unfinished, accept_scope_violation: p.accept_scope_violation };
    if let Some(error) = refusal(&task, &flags) {
        return Err(error);
    }
    let Some(worktree) = task.worktree.clone().filter(|path| Path::new(path).exists()) else {
        return Err(no_worktree(&task.name));
    };
    let branch = git::git(&task.repo, &["rev-parse", "--abbrev-ref", "HEAD"]).await?;
    let branch = String::from_utf8_lossy(&branch.stdout).trim().to_string();
    if task.branch != "HEAD" && branch != task.branch {
        return Err(RpcError::new(
            "target_branch_changed",
            format!("the repository is on {branch}, but {} targets {}", task.name, task.branch),
            format!("check out {} and accept again", task.branch),
        ));
    }
    if p.part.is_some() && task.parts.is_empty() {
        return Err(RpcError::invalid_params(format!("{} has no parts: accept it without a part", task.name)));
    }
    if p.recorded && p.no_commit {
        return Err(RpcError::invalid_params("--recorded applies nothing, so --no-commit has no meaning"));
    }
    if !task.parts.is_empty() {
        if p.recorded {
            return record_parts(core, p, task, worktree).await;
        }
        return accept_parts(core, p, task, worktree).await;
    }
    if p.recorded {
        return record(core, task, worktree).await;
    }
    let (files, bytes) = taken(&task, &worktree).await?;
    if files.is_empty() || bytes.is_empty() {
        return Err(no_changes(&task.name));
    }
    // The repository's commit before the apply: a shared task's workshop settles against it.
    let before = if task.shared { Some(workshop::head(&task.repo).await?) } else { None };
    let patch_path = core.paths.state.join("patches").join(format!("{}.patch", task.name));
    let patch = patch_path.display().to_string();
    let commit = bring_in(&mut task, &files, &bytes, &patch_path, p.message.as_deref(), p.no_commit, None).await?;
    task.state = TaskState::Accepted;
    if let Err(words) = crate::task::ledger::record(&core.paths.state, &task, "accepted", now_ms()) {
        task.warnings.push(words);
    }
    let (removed, left) = match before {
        Some(before) => settle_files(&mut task, &worktree, &files, &before, commit.as_deref()).await,
        None => {
            let removal = git::remove_worktree(&task.repo, task.worktree.as_deref(), Some(&task.task_branch)).await;
            if removal.removed {
                task.worktree = None;
            }
            (removal.removed, removal.left)
        }
    };
    let name = task.name.clone();
    let state = task.state;
    super::save(&core.paths.state, &task);
    super::write_result(&core.paths.state, &task, &super::worker_list(core));
    super::replace(core, task);
    core.changed.notify_waiters();
    drop(serial);
    Ok(TaskAcceptResult { task: name, state, commit, files, patch, removed, left, note: None })
}

/// Writes the patch where git reads it and the review keeps it.
async fn write_patch(path: &Path, bytes: &[u8]) -> Result<(), RpcError> {
    let label = path.display().to_string();
    let path = path.to_path_buf();
    let bytes = bytes.to_vec();
    let written = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        crate::paths::write_atomic(&path, &bytes)
    })
    .await;
    match written {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(super::unavailable(format!("cannot write {label}: {e}"))),
        Err(e) => Err(super::unavailable(format!("cannot write {label}: {e}"))),
    }
}

/// The patch's files that have changes in the repository, as `git status` names them.
async fn dirty_files(repo: &str, files: &[String]) -> Result<Vec<String>, RpcError> {
    let mut args = vec!["status", "--porcelain", "--"];
    args.extend(files.iter().map(String::as_str));
    let status = git::git(repo, &args).await?;
    Ok(String::from_utf8_lossy(&status.stdout)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.get(3..).unwrap_or(line).trim().to_string())
        .filter(|line| !line.is_empty())
        .collect())
}

/// Refuses when one of the patch's files has uncommitted changes in the repository.
async fn clean_or_refuse(task: &Task, files: &[String]) -> Result<(), RpcError> {
    let dirty = dirty_files(&task.repo, files).await?;
    if dirty.is_empty() {
        return Ok(());
    }
    Err(RpcError::new(
        "files_dirty",
        format!("these files already have changes in {}: {}", task.repo, dirty.join(", ")),
        "commit or stash them, then accept again",
    ))
}

/// Writes the patch beside the task, applies it to the repository and commits the files.
/// `part` names the part in a failed add or commit; `apply_path` is where git reads it.
async fn bring_in(
    task: &mut Task,
    files: &[String],
    bytes: &[u8],
    apply_path: &Path,
    message: Option<&str>,
    no_commit: bool,
    part: Option<&str>,
) -> Result<Option<String>, RpcError> {
    write_patch(apply_path, bytes).await?;
    let patch = apply_path.display().to_string();
    if !no_commit {
        clean_or_refuse(task, files).await?;
    }
    if let Err(error) = git::git(&task.repo, &["apply", "--check", "--whitespace=nowarn", &patch]).await {
        // With no commit, a rollback would lose the Owner's edits: try three-way only when the
        // patch's files have none.
        if no_commit && !unchanged(&task.repo, files).await? {
            return Err(not_applied(&task.name, &error.message));
        }
        let before: Vec<bool> = files.iter().map(|file| Path::new(&task.repo).join(file).exists()).collect();
        let applied = git::git_raw(&task.repo, &["apply", "--3way", "--whitespace=nowarn", &patch]).await?;
        if !applied.status.success() {
            restore(&task.repo, "HEAD", files, &before).await?;
            return Err(conflicted(&task.name, &applied, files));
        }
        if no_commit {
            let mut args = vec!["reset", "-q", "--"];
            args.extend(files.iter().map(String::as_str));
            git::git(&task.repo, &args).await?;
        }
        task.warnings.push(format!("{}'s patch applied three-way: the repository had changed under it", task.name));
    } else if let Err(error) = git::git(&task.repo, &["apply", "--whitespace=nowarn", &patch]).await {
        return Err(RpcError::new(
            "conflict",
            format!("{}'s patch could not be applied: {}", task.name, error.message),
            "check the repository; see `kom task review`",
        ));
    }
    let mut commit = None;
    if !no_commit {
        let text = message
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(String::from)
            .or_else(|| task.review.as_deref().map(str::trim).filter(|text| !text.is_empty()).map(String::from))
            .unwrap_or_else(|| format!("task {}: worker changes", task.name));
        let failure = |words: &str| match part {
            Some(part) => part_commit_failed(&task.name, part, words),
            None => commit_failed(&task.name, words),
        };
        let mut args: Vec<&str> = vec!["add", "--"];
        args.extend(files.iter().map(String::as_str));
        if let Err(error) = git::git(&task.repo, &args).await {
            return Err(failure(&error.message));
        }
        let mut args: Vec<&str> = vec!["commit", "-q", "-m", &text, "--"];
        args.extend(files.iter().map(String::as_str));
        if let Err(error) = git::git_for(&task.repo, &args, Duration::from_secs(600)).await {
            return Err(failure(&error.message));
        }
        let head = git::git(&task.repo, &["rev-parse", "HEAD"]).await?;
        commit = Some(String::from_utf8_lossy(&head.stdout).trim().to_string());
    }
    Ok(commit)
}

/// What an accept takes from a task's worktree: the changed files and the patch that applies them.
/// A shared task's are its scope's files in the workshop; any other task's are its worktree's.
async fn taken(task: &Task, worktree: &str) -> Result<(Vec<String>, Vec<u8>), RpcError> {
    taken_some(task, worktree, None).await
}

/// The same, only for the listed files: a part's patch must leave the other parts out.
async fn taken_some(task: &Task, worktree: &str, only: Option<&[String]>) -> Result<(Vec<String>, Vec<u8>), RpcError> {
    if only.is_some_and(|files| files.is_empty()) {
        return Ok((Vec::new(), Vec::new()));
    }
    let files: Vec<String> = if task.shared {
        let all = workshop::files_of(worktree, &task.scope).await?;
        match only {
            Some(only) => all.into_iter().filter(|file| only.contains(file)).collect(),
            None => all,
        }
    } else {
        git::git(worktree, &["add", "-N", "."]).await?;
        let mut args = vec!["diff", "--name-only", "-z", "--no-renames", task.base.as_str()];
        if let Some(only) = only {
            args.push("--");
            args.extend(only.iter().map(String::as_str));
        }
        let names = git::git(worktree, &args).await?;
        let mut files: Vec<String> = names
            .stdout
            .split(|byte| *byte == 0)
            .filter(|name| !name.is_empty())
            .map(|name| String::from_utf8_lossy(name).into_owned())
            .collect();
        if let Some(only) = only {
            files.retain(|file| only.contains(file));
        }
        files
    };
    if files.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let bytes = if task.shared {
        workshop::patch(worktree, &files, &["--binary", "--full-index"]).await?
    } else {
        let mut args = vec!["diff", "--binary", "--full-index", "--no-renames", task.base.as_str(), "--"];
        args.extend(files.iter().map(String::as_str));
        git::git(worktree, &args).await?.stdout
    };
    Ok((files, bytes))
}

/// Keeps a part's patch in the task's patch file, so a settled task's review still has every part.
async fn append_patch(path: &Path, bytes: &[u8]) -> Result<(), RpcError> {
    let path = path.to_path_buf();
    let label = path.display().to_string();
    let bytes = bytes.to_vec();
    let written = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        let mut text = std::fs::read(&path).unwrap_or_default();
        if !text.is_empty() && !text.ends_with(b"\n") {
            text.push(b'\n');
        }
        text.extend_from_slice(&bytes);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        crate::paths::write_atomic(&path, &text)
    })
    .await;
    match written {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(super::unavailable(format!("cannot write {label}: {e}"))),
        Err(e) => Err(super::unavailable(format!("cannot write the patch: {e}"))),
    }
}

/// The parts an accept takes: the one named, or every open one in order.
fn wanted_parts(task: &Task, single: Option<&str>) -> Result<Vec<usize>, RpcError> {
    let open: Vec<usize> =
        (0..task.parts.len()).filter(|index| parts::state(task, *index) == PartState::Open).collect();
    match single {
        Some(name) => {
            let index = parts::find(&task.parts, name)
                .ok_or_else(|| RpcError::invalid_params(format!("{} has no part {name:?}", task.name)))?;
            let state = parts::state(task, index);
            if state != PartState::Open {
                return Err(RpcError::new(
                    "finished",
                    format!("{}'s part {name} is already {}", task.name, state.word()),
                    "see `kom task review`",
                ));
            }
            Ok(vec![index])
        }
        None if open.is_empty() => {
            Err(RpcError::new("finished", format!("{} has no open part", task.name), "see `kom task review`"))
        }
        None => Ok(open),
    }
}

/// Accepts one part, or every open part in order, one commit each; the last one settles the task.
async fn accept_parts(
    core: &Arc<Core>,
    p: TaskAcceptParams,
    mut task: Task,
    worktree: String,
) -> Result<TaskAcceptResult, RpcError> {
    if p.no_commit {
        return Err(RpcError::invalid_params("a part is accepted with a commit: leave out no_commit"));
    }
    let single = p.part.clone();
    if single.is_some() && p.message.as_deref().is_none_or(|text| text.trim().is_empty()) {
        return Err(RpcError::invalid_params("accepting one part needs a message: pass message, or accept every part"));
    }
    let wanted = wanted_parts(&task, single.as_deref())?;
    let base_message = p
        .message
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(String::from)
        .or_else(|| task.review.as_deref().map(str::trim).filter(|text| !text.is_empty()).map(String::from))
        .unwrap_or_else(|| format!("task {}: worker changes", task.name));
    let patch_path = core.paths.state.join("patches").join(format!("{}.patch", task.name));
    let mut files_all: Vec<String> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let mut commit: Option<String> = None;
    for index in wanted {
        let part = task.parts[index].name.clone();
        let all = taken(&task, &worktree).await?.0;
        let (groups, _) = parts::group(&task.parts, &task.scope, &all);
        let files = groups[index].clone();
        if files.is_empty() {
            notes.push(format!("part {part} has no changes: accepted with no commit"));
            parts::set_state(&mut task, index, PartState::Accepted);
            super::save(&core.paths.state, &task);
            super::replace(core, task.clone());
            core.changed.notify_waiters();
            continue;
        }
        let (files, bytes) = taken_some(&task, &worktree, Some(&files)).await?;
        let text = if single.is_some() { base_message.clone() } else { format!("{base_message} (part {part})") };
        let apply_path = core.paths.state.join("patches").join(format!("{}.{part}.apply.patch", task.name));
        let made = bring_in(&mut task, &files, &bytes, &apply_path, Some(&text), false, Some(&part)).await;
        let _ = std::fs::remove_file(&apply_path);
        let made = made?;
        append_patch(&patch_path, &bytes).await?;
        files_all.extend(files);
        if made.is_some() {
            commit = made;
        }
        parts::set_state(&mut task, index, PartState::Accepted);
        super::save(&core.paths.state, &task);
        super::replace(core, task.clone());
        core.changed.notify_waiters();
    }
    let name = task.name.clone();
    if parts::open_count(&task) > 0 {
        super::write_result(&core.paths.state, &task, &super::worker_list(core));
        return Ok(TaskAcceptResult {
            task: name,
            state: task.state,
            commit,
            files: files_all,
            patch: patch_path.display().to_string(),
            removed: false,
            left: Vec::new(),
            note: (!notes.is_empty()).then(|| notes.join("; ")),
        });
    }
    let before = if task.shared { Some(workshop::head(&task.repo).await?) } else { None };
    let settle_list = if task.shared { taken(&task, &worktree).await?.0 } else { files_all.clone() };
    task.state = TaskState::Accepted;
    if let Err(words) = crate::task::ledger::record(&core.paths.state, &task, "accepted", now_ms()) {
        task.warnings.push(words);
    }
    let (removed, left) = match before {
        Some(before) => settle_files(&mut task, &worktree, &settle_list, &before, commit.as_deref()).await,
        None => {
            let removal = git::remove_worktree(&task.repo, task.worktree.as_deref(), Some(&task.task_branch)).await;
            if removal.removed {
                task.worktree = None;
            }
            (removal.removed, removal.left)
        }
    };
    let state = task.state;
    super::save(&core.paths.state, &task);
    super::write_result(&core.paths.state, &task, &super::worker_list(core));
    super::replace(core, task);
    core.changed.notify_waiters();
    Ok(TaskAcceptResult {
        task: name,
        state,
        commit,
        files: files_all,
        patch: patch_path.display().to_string(),
        removed,
        left,
        note: (!notes.is_empty()).then(|| notes.join("; ")),
    })
}

/// Records a task whose work was brought in by hand: accepted with no apply and no commit, once
/// every file its patch changes is committed in the repository.
async fn record(core: &Arc<Core>, mut task: Task, worktree: String) -> Result<TaskAcceptResult, RpcError> {
    let (files, bytes) = taken(&task, &worktree).await?;
    if files.is_empty() || bytes.is_empty() {
        return Err(no_changes(&task.name));
    }
    clean_or_refuse(&task, &files).await?;
    let patch_path = core.paths.state.join("patches").join(format!("{}.patch", task.name));
    write_patch(&patch_path, &bytes).await?;
    let before = if task.shared { Some(workshop::head(&task.repo).await?) } else { None };
    task.state = TaskState::Accepted;
    if let Err(words) = crate::task::ledger::record(&core.paths.state, &task, "accepted", now_ms()) {
        task.warnings.push(words);
    }
    let (removed, left) = match before {
        Some(before) => settle_files(&mut task, &worktree, &files, &before, None).await,
        None => {
            let removal = git::remove_worktree(&task.repo, task.worktree.as_deref(), Some(&task.task_branch)).await;
            if removal.removed {
                task.worktree = None;
            }
            (removal.removed, removal.left)
        }
    };
    let name = task.name.clone();
    let state = task.state;
    super::save(&core.paths.state, &task);
    super::write_result(&core.paths.state, &task, &super::worker_list(core));
    super::replace(core, task);
    core.changed.notify_waiters();
    Ok(TaskAcceptResult {
        task: name,
        state,
        commit: None,
        files,
        patch: patch_path.display().to_string(),
        removed,
        left,
        note: None,
    })
}

/// Records the parts whose work was brought in by hand, one open part or every open part; the
/// last one settles the task.
async fn record_parts(
    core: &Arc<Core>,
    p: TaskAcceptParams,
    mut task: Task,
    worktree: String,
) -> Result<TaskAcceptResult, RpcError> {
    let wanted = wanted_parts(&task, p.part.as_deref())?;
    let patch_path = core.paths.state.join("patches").join(format!("{}.patch", task.name));
    let mut files_all: Vec<String> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    for index in wanted {
        let part = task.parts[index].name.clone();
        let all = taken(&task, &worktree).await?.0;
        let (groups, _) = parts::group(&task.parts, &task.scope, &all);
        let files = groups[index].clone();
        if files.is_empty() {
            notes.push(format!("part {part} has no changes: accepted with no commit"));
            parts::set_state(&mut task, index, PartState::Accepted);
            super::save(&core.paths.state, &task);
            super::replace(core, task.clone());
            core.changed.notify_waiters();
            continue;
        }
        let (files, bytes) = taken_some(&task, &worktree, Some(&files)).await?;
        clean_or_refuse(&task, &files).await?;
        append_patch(&patch_path, &bytes).await?;
        files_all.extend(files);
        parts::set_state(&mut task, index, PartState::Accepted);
        super::save(&core.paths.state, &task);
        super::replace(core, task.clone());
        core.changed.notify_waiters();
    }
    let name = task.name.clone();
    if parts::open_count(&task) > 0 {
        super::write_result(&core.paths.state, &task, &super::worker_list(core));
        return Ok(TaskAcceptResult {
            task: name,
            state: task.state,
            commit: None,
            files: files_all,
            patch: patch_path.display().to_string(),
            removed: false,
            left: Vec::new(),
            note: (!notes.is_empty()).then(|| notes.join("; ")),
        });
    }
    let before = if task.shared { Some(workshop::head(&task.repo).await?) } else { None };
    let settle_list = if task.shared { taken(&task, &worktree).await?.0 } else { files_all.clone() };
    task.state = TaskState::Accepted;
    if let Err(words) = crate::task::ledger::record(&core.paths.state, &task, "accepted", now_ms()) {
        task.warnings.push(words);
    }
    let (removed, left) = match before {
        Some(before) => settle_files(&mut task, &worktree, &settle_list, &before, None).await,
        None => {
            let removal = git::remove_worktree(&task.repo, task.worktree.as_deref(), Some(&task.task_branch)).await;
            if removal.removed {
                task.worktree = None;
            }
            (removal.removed, removal.left)
        }
    };
    let state = task.state;
    super::save(&core.paths.state, &task);
    super::write_result(&core.paths.state, &task, &super::worker_list(core));
    super::replace(core, task);
    core.changed.notify_waiters();
    Ok(TaskAcceptResult {
        task: name,
        state,
        commit: None,
        files: files_all,
        patch: patch_path.display().to_string(),
        removed,
        left,
        note: (!notes.is_empty()).then(|| notes.join("; ")),
    })
}

/// Discards one part: its files go back to the base, the task waits for the other parts.
pub(crate) async fn discard_part(
    core: &Arc<Core>,
    name: &str,
    part: &str,
    review: Option<String>,
    by: Option<&str>,
    yes: bool,
) -> Result<(TaskState, bool, Vec<String>), RpcError> {
    let _serial = core.tasks.lock.lock().await;
    if super::stopping(core) {
        return Err(super::unavailable("the core is stopping".into()));
    }
    let Some(mut task) = core.tasks.snapshot(name) else { return Err(super::not_found(name)) };
    if let Some(error) = super::table::change_refusal(&task, by, yes) {
        return Err(error);
    }
    if !task.state.ended() && task.state != TaskState::Asking {
        return Err(RpcError::new(
            "running",
            format!("{name} is still {}: cancel it first", task.state.word()),
            format!("kom task cancel {name}"),
        ));
    }
    if task.state == TaskState::Accepted {
        return Err(RpcError::new(
            "finished",
            format!("{name} was accepted: its changes are in the repository"),
            "see `kom task show`",
        ));
    }
    if task.parts.is_empty() {
        return Err(RpcError::invalid_params(format!("{name} has no parts: discard the whole task")));
    }
    let index = parts::find(&task.parts, part)
        .ok_or_else(|| RpcError::invalid_params(format!("{name} has no part {part:?}")))?;
    let state = parts::state(&task, index);
    if state != PartState::Open {
        return Err(RpcError::new(
            "finished",
            format!("{name}'s part {part} is already {}", state.word()),
            "see `kom task review`",
        ));
    }
    let Some(worktree) = task.worktree.clone().filter(|path| Path::new(path).exists()) else {
        return Err(no_worktree(name));
    };
    let all = taken(&task, &worktree).await?.0;
    let (groups, _) = parts::group(&task.parts, &task.scope, &all);
    let files = groups[index].clone();
    if task.shared {
        if !files.is_empty() {
            workshop::restore(&worktree, &files).await.map_err(|e| {
                RpcError::new(
                    "unavailable",
                    format!("{name}'s part {part} could not be put back in the workshop: {}", e.message),
                    format!("try again: kom task discard {name} --part {part}"),
                )
            })?;
        }
    } else if !files.is_empty() {
        // Every file the part changed goes back to the base, added files included.
        let before = vec![false; files.len()];
        restore(&worktree, &task.base, &files, &before).await?;
    }
    parts::set_state(&mut task, index, PartState::Discarded);
    if review.is_some() {
        task.review = review;
    }
    let (state, removed, left) = if parts::open_count(&task) > 0 {
        (task.state, false, Vec::new())
    } else {
        let accepted = (0..task.parts.len()).any(|index| parts::state(&task, index) == PartState::Accepted);
        task.state = if accepted { TaskState::Accepted } else { TaskState::Discarded };
        if let Err(words) = crate::task::ledger::record(&core.paths.state, &task, task.state.word(), now_ms()) {
            task.warnings.push(words);
        }
        if task.shared {
            let before = workshop::head(&task.repo).await?;
            let after = if accepted {
                Some(
                    String::from_utf8_lossy(&git::git(&task.repo, &["rev-parse", "HEAD"]).await?.stdout)
                        .trim()
                        .to_string(),
                )
            } else {
                None
            };
            let remaining = taken(&task, &worktree).await?.0;
            let settled = settle_files(&mut task, &worktree, &remaining, &before, after.as_deref()).await;
            (task.state, settled.0, settled.1)
        } else {
            let removal = git::remove_worktree(&task.repo, task.worktree.as_deref(), Some(&task.task_branch)).await;
            if removal.removed {
                task.worktree = None;
            }
            (task.state, removal.removed, removal.left)
        }
    };
    super::save(&core.paths.state, &task);
    super::write_result(&core.paths.state, &task, &super::worker_list(core));
    super::replace(core, task);
    core.changed.notify_waiters();
    Ok((state, removed, left))
}

/// Takes a shared task's files out of the workshop once they are in the repository: they settle to
/// the commit that now holds them, or are left behind in words. Returns (removed, left).
async fn settle_files(
    task: &mut Task,
    worktree: &str,
    files: &[String],
    before: &str,
    commit: Option<&str>,
) -> (bool, Vec<String>) {
    let settled = workshop::settle(worktree, files, before, commit).await;
    task.worktree = None;
    match settled {
        Ok(None) => (true, Vec::new()),
        Ok(Some(words)) => {
            let text = format!("the workshop stayed behind: {words}");
            task.warnings.push(text.clone());
            (true, vec![text])
        }
        Err(e) => {
            let text = format!("the workshop still has {}'s files: {}", task.name, e.message);
            task.warnings.push(text.clone());
            (false, vec![text])
        }
    }
}

/// What a redo sends as the next turn: an answer carries its own words in front.
pub fn redo_brief(state: TaskState, brief: &str) -> String {
    if state == TaskState::Asking {
        format!("The lead's answer to your question:\n\n{brief}")
    } else {
        brief.to_string()
    }
}

pub async fn redo(core: &Arc<Core>, p: TaskRedoParams) -> Result<TaskRedoResult, RpcError> {
    let length = p.brief.chars().count();
    if length == 0 || length > super::MAX_BRIEF {
        return Err(RpcError::invalid_params(format!(
            "a brief is 1 to {} characters; this one has {length}",
            super::MAX_BRIEF
        )));
    }
    let serial = core.tasks.lock.lock().await;
    if super::stopping(core) {
        return Err(super::unavailable("the core is stopping".into()));
    }
    let Some(mut task) = core.tasks.snapshot(&p.task) else { return Err(super::not_found(&p.task)) };
    if !task.state.ended() && task.state != TaskState::Asking {
        return Err(RpcError::new(
            "running",
            format!("{} is still {}: cancel it first", task.name, task.state.word()),
            format!("kom task cancel {}", task.name),
        ));
    }
    if matches!(task.state, TaskState::Accepted | TaskState::Discarded) {
        return Err(RpcError::new(
            "finished",
            format!("{} has already ended: {}", task.name, task.state.word()),
            "see `kom task show`",
        ));
    }
    if task.worktree.as_deref().is_none_or(|path| !Path::new(path).exists()) {
        return Err(no_worktree(&task.name));
    }
    let n = super::next_turn(&task);
    let path = super::task_folder(&core.paths.state, &task.name).join(format!("turn-{n}.followup.md"));
    let brief = redo_brief(task.state, &p.brief);
    let written = tokio::task::spawn_blocking(move || crate::paths::write_atomic(&path, brief.as_bytes())).await;
    match written {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(super::unavailable(format!("cannot write the follow-up: {e}"))),
        Err(e) => return Err(super::unavailable(format!("cannot write the follow-up: {e}"))),
    }
    task.followup_role = Some("redo".into());
    task.state = TaskState::Queued;
    task.ended_ms = None;
    task.error = None;
    task.question = None;
    task.ending = None;
    task.qa_used = 0;
    task.waiting = Some("redo: waiting for its worker".into());
    super::save(&core.paths.state, &task);
    super::replace(core, task);
    core.changed.notify_waiters();
    drop(serial);
    super::pump(core.clone()).await;
    let after = core.tasks.snapshot(&p.task).ok_or_else(|| super::not_found(&p.task))?;
    let name = after.name.clone();
    Ok(TaskRedoResult {
        task: name.clone(),
        state: after.state,
        turn: n,
        waiting: after.waiting,
        next: format!("kom task wait {name}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::types::{ChangedFile, CheckReport, Task, TestRun};

    fn task(state: TaskState) -> Task {
        Task { name: "fix-1".into(), state, worktree: Some("C:/state/worktrees/fix-1".into()), ..Default::default() }
    }

    fn failed() -> CheckReport {
        CheckReport {
            ok: false,
            seconds: 1.0,
            test: Some(TestRun { failed: vec!["a_test".into()], ..Default::default() }),
            ..Default::default()
        }
    }

    fn passed() -> CheckReport {
        CheckReport { ok: true, seconds: 1.0, forbid_checked: 1, ..Default::default() }
    }

    fn kind(task: &Task, flags: AcceptFlags) -> Option<String> {
        refusal(task, &flags).map(|error| error.kind)
    }

    #[test]
    fn refusal_checks_its_cases_in_order() {
        let flags = AcceptFlags::default();
        assert_eq!(kind(&task(TaskState::Accepted), flags).as_deref(), Some("finished"));
        assert_eq!(kind(&task(TaskState::Discarded), flags).as_deref(), Some("finished"));
        for state in [TaskState::Queued, TaskState::Running, TaskState::Stranded, TaskState::Checking] {
            assert_eq!(kind(&task(state), flags).as_deref(), Some("running"), "{state:?}");
        }
        let asking = refusal(&task(TaskState::Asking), &flags).unwrap();
        assert_eq!(asking.kind, "asking");
        assert!(asking.message.contains("it is asking a question: answer it with kom task redo, or discard it"));
        assert!(asking.hint.contains("kom task redo"), "{}", asking.hint);
        for state in [TaskState::Failed, TaskState::TimedOut, TaskState::Cancelled] {
            assert_eq!(kind(&task(state), flags).as_deref(), Some("unfinished"), "{state:?}");
        }
        let mut bad = task(TaskState::Done);
        bad.checks = vec![failed()];
        assert_eq!(kind(&bad, flags).as_deref(), Some("unfinished"));
        let allow = AcceptFlags { accept_unfinished: true, ..Default::default() };
        assert_eq!(kind(&bad, allow), None, "the flag accepts a failed last report");
        let mut fixed = task(TaskState::Done);
        fixed.checks = vec![failed(), passed()];
        assert_eq!(kind(&fixed, flags), None, "a pass after an earlier failure is finished");
        let mut gone = task(TaskState::Done);
        gone.worktree = None;
        assert_eq!(kind(&gone, flags).as_deref(), Some("no_worktree"));
        let mut outside = task(TaskState::Done);
        outside.changed = vec![ChangedFile { path: "outside.txt".into(), added: 1, removed: 0, in_scope: false }];
        assert_eq!(kind(&outside, flags).as_deref(), Some("scope_violation"));
        let allow = AcceptFlags { accept_scope_violation: true, ..Default::default() };
        assert_eq!(kind(&outside, allow), None);

        let mut bad_and_outside = outside;
        bad_and_outside.state = TaskState::Failed;
        assert_eq!(kind(&bad_and_outside, flags).as_deref(), Some("unfinished"), "the bad end comes first");
        let allow = AcceptFlags { accept_unfinished: true, ..Default::default() };
        assert_eq!(kind(&bad_and_outside, allow).as_deref(), Some("scope_violation"), "then the scope");
        let both = AcceptFlags { accept_unfinished: true, accept_scope_violation: true };
        assert_eq!(kind(&bad_and_outside, both), None);
    }

    #[test]
    fn a_redo_from_asking_prefixes_the_answer_and_any_other_redo_does_not() {
        assert_eq!(
            redo_brief(TaskState::Asking, "Take option A."),
            "The lead's answer to your question:\n\nTake option A."
        );
        assert_eq!(redo_brief(TaskState::Done, "Try again."), "Try again.");
        assert_eq!(redo_brief(TaskState::Failed, "Fix it."), "Fix it.");
    }

    #[test]
    fn an_unfinished_task_needs_its_flag_even_with_a_worktree() {
        let mut failed_task = task(TaskState::Failed);
        failed_task.changed = vec![ChangedFile { path: "made.txt".into(), added: 1, removed: 0, in_scope: true }];
        let flags = AcceptFlags { accept_unfinished: true, ..Default::default() };
        assert_eq!(kind(&failed_task, flags), None);
        let error = refusal(&failed_task, &AcceptFlags::default()).unwrap();
        assert!(error.message.contains("accept_unfinished"), "{}", error.message);
        assert!(error.hint.contains("accept_unfinished"), "{}", error.hint);
    }
}
