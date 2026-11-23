//! The end of a turn: judge it, keep the diff and the result, and fail a start.

use super::{read_exit, replace, save, save_workers, stopping, task_folder, worker_list, write_result};
use crate::rpc::RpcError;
use crate::runtime::{Core, now_ms};
use crate::task::decide::{self, TerminalNow};
use crate::task::handoff;
use crate::task::opencode::RouteCheck;
use crate::task::types::{ChangedFile, Task, TaskState};
use crate::task::worker::{self, Cli, TurnOutcome};
use crate::task::{git, rate_wait, setup, workers, workshop};
use std::path::Path;
use std::sync::Arc;

/// The last bytes of a turn's stderr, which is where a CLI puts its limits.
fn read_tail(path: &Path, max: usize) -> String {
    let Ok(bytes) = std::fs::read(path) else { return String::new() };
    let start = bytes.len().saturating_sub(max);
    String::from_utf8_lossy(&bytes[start..]).into_owned()
}

/// What a blocked read's panic said, in words.
fn panic_words(error: tokio::task::JoinError) -> String {
    if error.is_cancelled() {
        return "the read was cancelled".into();
    }
    let payload = error.into_panic();
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_string()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "the read stopped unexpectedly".into()
    }
}

pub(super) fn unreadable_turn(error: tokio::task::JoinError) -> String {
    format!("the core could not read this turn's output: {}", panic_words(error))
}

/// The turn ended: read what it wrote, judge it, keep the diff and the result.
pub(super) async fn finish_turn(core: &Arc<Core>, name: &str, n: u32, terminal: Option<&TerminalNow>) {
    if stopping(core) {
        return;
    }
    let Some(mut task) = core.tasks.snapshot(name) else { return };
    let Some(index) = task.turns.iter().position(|turn| turn.n == n) else { return };
    let folder = task_folder(&core.paths.state, name);
    let key = task.turns[index].worker.clone();
    let worker_file = core.tasks.workers();
    let all_workers = workers::all(&worker_file);
    let worker = workers::find(&worker_file, &key)
        .unwrap_or_else(|_| worker::builtin().into_iter().find(|worker| worker.key == "go").expect("go is built in"));
    let cli = task.turns[index].cli_or_lookup(&all_workers);
    let path = folder.join(format!("turn-{n}.jsonl"));
    let err_path = folder.join(format!("turn-{n}.err"));
    let route_path = folder.join(format!("turn-{n}.route.json"));
    let config = crate::config::load(&core.paths.config).unwrap_or_default();
    let children_folder = folder.clone();
    let earlier: Vec<(Option<String>, f64, crate::task::types::Usage)> = task.turns[..index]
        .iter()
        .map(|turn| (turn.session.clone(), turn.usage.cost_usd, turn.usage.clone()))
        .collect();
    let resumes = task.turns[index].resumes.clone();
    let read = tokio::task::spawn_blocking(move || {
        let mut outcome = match std::fs::read(&path) {
            Ok(bytes) => TurnOutcome::from_bytes(cli, &bytes),
            Err(_) => TurnOutcome::default(),
        };
        if cli == Cli::Codex && resumes.is_some() && outcome.session == resumes {
            // Codex reports the thread's whole total, so only what this turn
            // added since the earlier turns on that thread counts.
            let mut prior = crate::task::types::Usage::default();
            for (session, _, usage) in &earlier {
                if session.as_deref() == outcome.session.as_deref() {
                    prior.add(usage);
                }
            }
            outcome.usage = crate::task::codex::since_earlier(&outcome.usage, &prior);
        }
        let mut warning = None;
        if cli == Cli::CommandCode {
            match outcome.session.as_deref().and_then(crate::task::commandcode::session_cost) {
                Some(cost) => {
                    // The transcript holds every turn of a resumed session, so only
                    // what this turn added since the earlier ones counts.
                    let same: f64 = earlier
                        .iter()
                        .filter(|(session, _, _)| session.as_deref() == outcome.session.as_deref())
                        .map(|(_, cost, _)| *cost)
                        .sum();
                    outcome.usage.cost_usd = crate::task::checking::turn_cost(cost, same);
                }
                None => {
                    if let Some(session) = outcome.session.clone() {
                        warning = Some(format!(
                            "Command Code's transcript for session {session} was not found, so this turn's cost is unknown"
                        ));
                    }
                }
            }
        }
        let route = std::fs::read(&route_path).ok().and_then(|bytes| serde_json::from_slice::<RouteCheck>(&bytes).ok());
        let (children, child_warnings) = if cli == Cli::Opencode {
            crate::task::stats::children_usage(&config, &children_folder, &path)
        } else {
            (Vec::new(), Vec::new())
        };
        (outcome, warning, route, read_tail(&err_path, 8 * 1024), children, child_warnings)
    })
    .await;
    let (outcome, warning, route, stderr_tail, children, child_warnings) = match read {
        Ok(read) => read,
        Err(error) => {
            // A panic here is the core's failure to read, never the worker's
            // turn being empty: keep the turn's file and say what happened.
            finish_with_error(core, &task, &unreadable_turn(error));
            return;
        }
    };
    let code = terminal.and_then(|terminal| terminal.exit_code).or_else(|| read_exit(&core.paths.state, name, n));
    let ended = now_ms();
    let offset = crate::platform::utc_offset_minutes();
    {
        let turn = &mut task.turns[index];
        turn.session = outcome.session.clone();
        turn.text = outcome.text.clone();
        let mut usage = outcome.usage.clone();
        for child in &children {
            usage.add(&child.usage);
        }
        turn.usage = usage;
        turn.children = children;
        turn.exit_code = code;
        turn.ended_ms = Some(ended);
    }
    task.warnings.extend(child_warnings);
    if let Some(warning) = warning {
        task.warnings.push(warning);
    }
    let end =
        decide::turn_end(&task, &all_workers, terminal, code, &outcome, route.as_ref(), &stderr_tail, ended, offset);
    task.turns[index].refused = end.refusal.as_ref().map(|refusal| refusal.window.clone());
    let mut error = end.error;
    // A rate refusal waits on the same worker twice before it blocks like the rest.
    let mut retry_at = None;
    task.rate_wait_until_ms = None;
    if let Some(refusal) = &end.refusal {
        if refusal.window == "rate" {
            let refusals = task.rate_refusals.saturating_add(1);
            task.rate_refusals = refusals;
            retry_at = rate_wait::wait_ms(refusals).map(|wait| ended.saturating_add(wait));
            task.rate_wait_until_ms = retry_at;
        }
        if retry_at.is_none() {
            let before = core.tasks.workers();
            let mut file = before.clone();
            let current = file.blocks.get(&key).cloned();
            let new = workers::after_refusal(current.as_ref(), refusal, Some(&task.name), ended);
            if let Some(block) = &new {
                file.blocks.insert(key.clone(), block.clone());
            }
            let block = new.or_else(|| file.blocks.get(&key).filter(|block| block.until_ms > ended).cloned());
            if let Some(block) = block {
                error = Some(format!(
                    "QUOTA EXHAUSTED: {}",
                    workers::sentence(&worker::label_of(&all_workers, &key), &block, ended, offset)
                ));
            }
            if file != before {
                if let Err(e) = save_workers(&core.paths.state, &file) {
                    task.warnings.push(format!("cannot save workers.json: {e}"));
                }
                *core.tasks.workers.lock().unwrap_or_else(|p| p.into_inner()) = file;
            }
        }
    }
    let refused_words = error.clone().unwrap_or_default();
    if end.state == TaskState::Done {
        if let Some(words) = decide::route_unchecked(route.as_ref()) {
            task.warnings.push(words);
        }
        let before = core.tasks.workers();
        let mut file = before.clone();
        workers::after_good_turn(&mut file, &key);
        if file != before {
            if let Err(e) = save_workers(&core.paths.state, &file) {
                task.warnings.push(format!("cannot save workers.json: {e}"));
            }
            *core.tasks.workers.lock().unwrap_or_else(|p| p.into_inner()) = file;
        }
    }
    // Before the diff, so what the turn was given never counts as a change.
    if let Some(worktree) = task.worktree.as_deref()
        && let Err(words) = setup::after_turn(&folder, worktree, worker, n)
    {
        task.warnings.push(words);
    }
    let before_changed = task.changed.clone();
    let changed = match task.worktree.as_deref() {
        Some(worktree) if task.shared => match workshop::changed(worktree, &task.scope).await {
            Ok((mine, others)) => {
                let held: Vec<Vec<String>> = core
                    .tasks
                    .all()
                    .into_iter()
                    .filter(|other| other.name != task.name && other.repo == task.repo && workshop::holds(other))
                    .map(|other| other.scope)
                    .collect();
                let scopes: Vec<&[String]> = held.iter().map(Vec::as_slice).collect();
                let stray = workshop::strays(&others, &scopes);
                if !stray.is_empty() {
                    let text =
                        format!("files changed in the workshop outside every task's scope: {}", stray.join(", "));
                    if !task.warnings.contains(&text) {
                        task.warnings.push(text);
                    }
                }
                mine
            }
            Err(e) => {
                task.warnings.push(format!("cannot read the changed files: {}", e.message));
                Vec::new()
            }
        },
        Some(worktree) => match git::changed_files(worktree, &task.base, &task.scope).await {
            Ok(changed) => changed,
            Err(e) => {
                task.warnings.push(format!("cannot read the changed files: {}", e.message));
                Vec::new()
            }
        },
        None => Vec::new(),
    };
    let new_changed = changed.iter().filter(|file| !before_changed.iter().any(|old| old.path == file.path)).count();
    let (done_state, question) = decide::after_turn(end.state, &outcome.text, !changed.is_empty());
    let died = end.refusal.is_none() && handoff::died_at_start(&task, terminal, code, &outcome, ended);
    let mut restarting = false;
    if retry_at.is_some() {
        restarting = true;
    } else if end.refusal.is_some() {
        if let Some(words) = handoff::stopped_at_turn_limit(task.turns.len(), &refused_words) {
            error = Some(words);
        } else {
            restarting = true;
            let usage = &task.turns[index].usage;
            if handoff::did_nothing(usage.steps, new_changed) {
                if let Err(words) = carry_turn_file(&folder, n, "note") {
                    task.warnings.push(words);
                }
            } else {
                let from = worker::label_of(&all_workers, &key);
                let stopped = handoff::stopped_words(
                    task.turns[index].refused.as_deref().unwrap_or_default(),
                    usage.steps,
                    usage.cost_usd,
                );
                if let Err(words) = handoff_note(&task, &folder, n, &changed, &outcome, &from, &stopped).await {
                    task.warnings.push(words);
                }
            }
        }
    } else if died {
        task.turns[index].died_at_start = true;
        restarting = true;
        let line = handoff::first_line(&stderr_tail);
        let label = worker::label_of(&all_workers, &key);
        let warning = if line.is_empty() {
            format!("{label} died at its start, so it was started again")
        } else {
            format!("{label} died at its start, so it was started again: {line}")
        };
        task.warnings.push(warning);
        if let Err(words) = carry_turn_file(&folder, n, "note") {
            task.warnings.push(words);
        }
    }
    // A follow-up stays asked until a turn has really taken it.
    if restarting && let Err(words) = carry_turn_file(&folder, n, "followup") {
        task.warnings.push(words);
    }
    // A wall-limit end with changes leaves one report turn, the first time.
    let mut queue_report = !restarting
        && decide::report_turn_due(task.ending, !changed.is_empty(), task.followup_role.as_deref() == Some("report"));
    if queue_report {
        let path = folder.join(format!("turn-{}.followup.md", super::next_turn(&task)));
        match crate::paths::write_atomic(&path, decide::REPORT_PROMPT.as_bytes()) {
            Ok(()) => {
                task.ending = None;
                task.followup_role = Some("report".into());
                task.wall_s = 600;
            }
            Err(e) => {
                task.warnings.push(format!("cannot write {}: {e}", path.display()));
                queue_report = false;
            }
        }
    }
    // Whatever the report turn did, the task then ends timed_out with it.
    let report_done = !restarting && !queue_report && task.followup_role.as_deref() == Some("report");
    if report_done && error.is_none() {
        error = task.error.clone();
    }
    task.state = if restarting || queue_report {
        TaskState::Queued
    } else if report_done {
        TaskState::TimedOut
    } else {
        done_state
    };
    task.question = if task.state == TaskState::Asking { question } else { None };
    task.error = if restarting { None } else { error };
    task.waiting = if let Some(until) = retry_at {
        Some(rate_wait::sentence(&worker::label_of(&all_workers, &key), until, ended, offset))
    } else if restarting {
        if end.refusal.is_some() {
            Some(format!("handing off: {refused_words}"))
        } else {
            Some("starting again: it died at its start".into())
        }
    } else if queue_report {
        Some("starting the report turn: its time ran out".into())
    } else {
        None
    };
    task.changed = changed;
    task.ended_ms =
        if restarting || queue_report || (end.state == TaskState::Done && !report_done) { None } else { Some(ended) };
    save(&core.paths.state, &task);
    if !restarting && !queue_report && (end.state != TaskState::Done || report_done) {
        write_result(&core.paths.state, &task, &all_workers);
    }
    replace(core, task);
    core.changed.notify_waiters();
}

/// A turn that didn't take what it was given carries it into the next turn.
fn carry_turn_file(folder: &Path, n: u32, kind: &str) -> Result<(), String> {
    let from = folder.join(format!("turn-{n}.{kind}.md"));
    if !from.is_file() {
        return Ok(());
    }
    let to = folder.join(format!("turn-{}.{kind}.md", n.saturating_add(1)));
    std::fs::copy(&from, &to).map(|_| ()).map_err(|e| format!("cannot copy {}: {e}", from.display()))
}

/// The hand-off note for the turn after this one, with each changed file's size
/// read off the disk away from the core's thread.
pub(super) async fn handoff_note(
    task: &Task,
    folder: &Path,
    n: u32,
    changed: &[ChangedFile],
    outcome: &TurnOutcome,
    from: &str,
    stopped: &str,
) -> Result<(), String> {
    let files = changed.to_vec();
    let worktree = task.worktree.clone();
    let changes = tokio::task::spawn_blocking(move || {
        let root = worktree.as_deref().map(Path::new);
        let base = worktree.as_deref().unwrap_or_default();
        files
            .into_iter()
            .map(|file| handoff::FileChange {
                bytes: root.and_then(|root| std::fs::metadata(root.join(&file.path)).ok()).map(|meta| meta.len()),
                path: handoff::relative(&file.path, base),
                added: file.added,
                removed: file.removed,
            })
            .collect::<Vec<_>>()
    })
    .await
    .unwrap_or_default();
    let worktree = task.worktree.as_deref().unwrap_or_default();
    let read: Vec<String> = outcome.files_read.iter().map(|path| handoff::relative(path, worktree)).collect();
    let text = handoff::note(from, stopped, &changes, &read, &outcome.reasoning_tail, &outcome.text);
    let path = folder.join(format!("turn-{}.note.md", n.saturating_add(1)));
    crate::paths::write_atomic(&path, text.as_bytes()).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

pub(super) fn finish_with_error(core: &Arc<Core>, task: &Task, message: &str) {
    if stopping(core) {
        return;
    }
    let mut task = task.clone();
    task.state = TaskState::Failed;
    task.error = Some(message.into());
    task.ended_ms = Some(now_ms());
    save(&core.paths.state, &task);
    write_result(&core.paths.state, &task, &worker_list(core));
    replace(core, task);
    core.changed.notify_waiters();
}

pub(super) async fn fail_start(core: &Arc<Core>, task: &Task, error: RpcError) {
    if stopping(core) {
        return;
    }
    let mut task = core.tasks.snapshot(&task.name).unwrap_or_else(|| task.clone());
    // A later turn's failed start keeps the first worker's work.
    if task.turns.is_empty() && task.shared {
        // The workshop stays; a task holds nothing in it before its first turn.
        task.worktree = None;
    } else if task.turns.is_empty()
        && let Some(worktree) = task.worktree.clone()
    {
        let removal = git::remove_worktree(&task.repo, Some(&worktree), Some(&task.task_branch)).await;
        if removal.removed {
            task.worktree = None;
        }
        if !removal.left.is_empty() {
            task.warnings.push(format!("the failed start left {}", removal.left.join(", ")));
        }
    }
    task.state = TaskState::Failed;
    task.error = Some(error.message);
    task.waiting = None;
    task.ended_ms = Some(now_ms());
    save(&core.paths.state, &task);
    write_result(&core.paths.state, &task, &worker_list(core));
    replace(core, task);
    core.changed.notify_waiters();
}
