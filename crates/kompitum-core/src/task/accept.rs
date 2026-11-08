//! `kom task accept` and `kom task redo`: bring a task's changes in, or send it back.

use crate::api::{TaskAcceptParams, TaskAcceptResult, TaskRedoParams, TaskRedoResult};
use crate::rpc::RpcError;
use crate::runtime::{Core, now_ms};
use crate::task::git;
use crate::task::types::TaskState;
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
    git::git(&worktree, &["add", "-N", "."]).await?;
    let diff = git::git(&worktree, &["diff", "--binary", "--full-index", "--no-renames", &task.base]).await?;
    if diff.stdout.is_empty() {
        return Err(no_changes(&task.name));
    }
    let names = git::git(&worktree, &["diff", "--name-only", "-z", "--no-renames", &task.base]).await?;
    let files: Vec<String> = names
        .stdout
        .split(|byte| *byte == 0)
        .filter(|name| !name.is_empty())
        .map(|name| String::from_utf8_lossy(name).into_owned())
        .collect();
    if files.is_empty() {
        return Err(no_changes(&task.name));
    }
    let patch_path = core.paths.state.join("patches").join(format!("{}.patch", task.name));
    let patch = patch_path.display().to_string();
    let bytes = diff.stdout;
    let path = patch_path.clone();
    let written = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        crate::paths::write_atomic(&path, &bytes)
    })
    .await;
    match written {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(super::unavailable(format!("cannot write {}: {e}", patch_path.display()))),
        Err(e) => return Err(super::unavailable(format!("cannot write {}: {e}", patch_path.display()))),
    }
    if !p.no_commit {
        let mut args: Vec<&str> = vec!["status", "--porcelain", "--"];
        args.extend(files.iter().map(String::as_str));
        let status = git::git(&task.repo, &args).await?;
        let dirty: Vec<String> = String::from_utf8_lossy(&status.stdout)
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| line.get(3..).unwrap_or(line).trim().to_string())
            .filter(|line| !line.is_empty())
            .collect();
        if !dirty.is_empty() {
            return Err(RpcError::new(
                "files_dirty",
                format!("these files already have changes in {}: {}", task.repo, dirty.join(", ")),
                "commit or stash them, then accept again",
            ));
        }
    }
    if let Err(error) = git::git(&task.repo, &["apply", "--check", "--whitespace=nowarn", &patch]).await {
        return Err(RpcError::new(
            "conflict",
            format!("{}'s patch does not apply: {}", task.name, error.message),
            "the repository was not touched; see `kom task review`",
        ));
    }
    if let Err(error) = git::git(&task.repo, &["apply", "--whitespace=nowarn", &patch]).await {
        return Err(RpcError::new(
            "conflict",
            format!("{}'s patch could not be applied: {}", task.name, error.message),
            "check the repository; see `kom task review`",
        ));
    }
    let mut commit = None;
    if !p.no_commit {
        let text = p
            .message
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(String::from)
            .or_else(|| task.review.as_deref().map(str::trim).filter(|text| !text.is_empty()).map(String::from))
            .unwrap_or_else(|| format!("task {}: worker changes", task.name));
        let mut args: Vec<&str> = vec!["add", "--"];
        args.extend(files.iter().map(String::as_str));
        if let Err(error) = git::git(&task.repo, &args).await {
            return Err(commit_failed(&task.name, &error.message));
        }
        let mut args: Vec<&str> = vec!["commit", "-q", "-m", &text, "--"];
        args.extend(files.iter().map(String::as_str));
        if let Err(error) = git::git_for(&task.repo, &args, Duration::from_secs(600)).await {
            return Err(commit_failed(&task.name, &error.message));
        }
        let head = git::git(&task.repo, &["rev-parse", "HEAD"]).await?;
        commit = Some(String::from_utf8_lossy(&head.stdout).trim().to_string());
    }
    task.state = TaskState::Accepted;
    if let Err(words) = crate::task::ledger::record(&core.paths.state, &task, "accepted", now_ms()) {
        task.warnings.push(words);
    }
    let removal = git::remove_worktree(&task.repo, task.worktree.as_deref(), Some(&task.task_branch)).await;
    if removal.removed {
        task.worktree = None;
    }
    let name = task.name.clone();
    let state = task.state;
    super::save(&core.paths.state, &task);
    super::write_result(&core.paths.state, &task, &super::worker_list(core));
    super::replace(core, task);
    core.changed.notify_waiters();
    drop(serial);
    Ok(TaskAcceptResult { task: name, state, commit, files, patch, removed: removal.removed, left: removal.left })
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
