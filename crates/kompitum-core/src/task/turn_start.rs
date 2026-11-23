//! The start of a turn: read its note and follow-up and run the worker.

use super::turn_end::handoff_note;
use super::{next_turn, task_folder, unavailable, update, worker_list, worktree_path};
use crate::api::{Label, RunParams};
use crate::rpc::RpcError;
use crate::runtime::{Core, now_ms};
use crate::task::types::{Task, TaskState, Turn};
use crate::task::worker::{self, Cli, TurnOutcome, Worker};
use crate::task::{git, handoff, prompt, setup, workshop};
use std::io;
use std::path::Path;
use std::sync::Arc;

/// A turn's note or follow-up, when it has one.
fn read_turn_file(folder: &Path, n: u32, kind: &str) -> Result<Option<Vec<u8>>, RpcError> {
    let path = folder.join(format!("turn-{n}.{kind}.md"));
    match std::fs::read(&path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(unavailable(format!("cannot read {}: {e}", path.display()))),
    }
}

pub(super) async fn start_task(
    core: &Arc<Core>,
    task: &Task,
    worker: Worker,
    prefix: &[String],
) -> Result<(), RpcError> {
    let mut task = task.clone();
    let n = next_turn(&task);
    let all_workers = worker_list(core);
    let folder = task_folder(&core.paths.state, &task.name);
    std::fs::create_dir_all(&folder).map_err(|e| unavailable(format!("cannot make {}: {e}", folder.display())))?;
    let brief = std::fs::read_to_string(folder.join("brief.md"))
        .map_err(|e| unavailable(format!("cannot read the task's brief: {e}")))?;
    let mut note = read_turn_file(&folder, n, "note")?;
    let followup = read_turn_file(&folder, n, "followup")?;
    // A follow-up continues the session when the same worker is here to take it.
    let resume = prompt::resume_session(&task, &worker, followup.is_some());
    let mut warnings = Vec::new();
    // One that can't gets the story the way a hand-off does.
    if note.is_none()
        && followup.is_some()
        && resume.is_none()
        && let Some(last) = task.turns.last()
    {
        let stopped = match last.refused.as_deref() {
            Some(window) => handoff::stopped_words(window, last.usage.steps, last.usage.cost_usd),
            None => "finished its turn".to_string(),
        };
        let outcome = TurnOutcome { text: last.text.clone(), ..Default::default() };
        let from = worker::label_of(&all_workers, &last.worker);
        if let Err(words) = handoff_note(&task, &folder, last.n, &task.changed, &outcome, &from, &stopped).await {
            warnings.push(words);
        }
        note = read_turn_file(&folder, n, "note")?;
    }
    let built = prompt::turn_prompt(&task, &brief, note.as_deref(), followup.as_deref(), resume.is_some()).await?;
    warnings.extend(built.warnings);
    let prompt_path = prompt::prompt_path(&folder, n);
    crate::paths::write_atomic(&prompt_path, &built.text)
        .map_err(|e| unavailable(format!("cannot write {}: {e}", prompt_path.display())))?;
    if task.shared {
        let path = workshop::path(&core.paths.state, &task.repo).display().to_string();
        // Only a task of this repository that is mid-turn in the workshop keeps it where it is.
        let quiet = !core.tasks.all().iter().any(|other| {
            other.name != task.name
                && other.repo == task.repo
                && other.shared
                && matches!(other.state, TaskState::Running | TaskState::Checking)
        });
        if let Some(words) = workshop::enter(&task.repo, &path, quiet).await? {
            update(core, &task.name, |stored| {
                if !stored.warnings.contains(&words) {
                    stored.warnings.push(words.clone());
                }
            });
            if !task.warnings.contains(&words) {
                task.warnings.push(words);
            }
        }
        if task.worktree.is_none() {
            update(core, &task.name, |stored| stored.worktree = Some(path.clone()));
            task.worktree = Some(path);
        }
    } else if task.worktree.is_none() {
        let worktree = worktree_path(&core.paths.state, &task.name).display().to_string();
        git::add_worktree(&task.repo, &worktree, &task.task_branch, &task.base).await?;
        update(core, &task.name, |task| task.worktree = Some(worktree.clone()));
        task.worktree = Some(worktree);
    }
    let Some(worktree) = task.worktree.clone() else {
        return Err(unavailable("the task has no worktree".into()));
    };
    let prepared = setup::prepare(core, &task, &worker, &worktree, n).await?;
    let command =
        worker::turn_command(&worker, prefix, &task.name, &prompt_path.display().to_string(), resume.as_deref());
    let exe = std::env::current_exe().map_err(|e| unavailable(format!("cannot find this kom: {e}")))?;
    let mut argv = vec![
        exe.display().to_string(),
        "run-logged".into(),
        "--dir".into(),
        folder.display().to_string(),
        "--task".into(),
        task.name.clone(),
        "--turn".into(),
        n.to_string(),
        "--worker".into(),
        worker.key().into(),
        "--cli".into(),
        worker.cli.key().into(),
    ];
    if let Some(stdin) = &command.stdin {
        argv.push("--stdin".into());
        argv.push(stdin.clone());
    }
    if worker.cli == Cli::Opencode {
        argv.push("--route".into());
        argv.push(worker.route().into());
        if let Some(prefix_len) = command.prefix_len {
            argv.push("--prefix-len".into());
            argv.push(prefix_len.to_string());
        }
    }
    if let Some(first) = &prepared.path_first {
        argv.push("--path-first".into());
        argv.push(first.clone());
    }
    argv.push("--".into());
    argv.extend(command.argv);
    let mut env = command.env;
    env.extend(prepared.env);
    let params = RunParams {
        argv,
        cwd: Some(worktree.clone()),
        name: Some(task.name.clone()),
        labels: vec![Label("task".into(), task.name.clone()), Label("worker".into(), worker.key().into())],
        contain: true,
        no_parent: true,
        no_notify: true,
        outlive_parent: true,
        env,
        ..Default::default()
    };
    let started = match crate::api::start_terminal(core, params, None).await {
        Ok(started) => started,
        Err(error) => {
            // No turn will end to do this.
            if let Err(words) = setup::after_turn(&folder, &worktree, worker.clone(), n) {
                update(core, &task.name, |task| task.warnings.push(words));
            }
            return Err(error);
        }
    };
    let now = now_ms();
    let sentence = task.turns.last().and_then(|turn| {
        let window = turn.refused.as_deref()?;
        Some(handoff::sentence(
            &worker::label_of(&all_workers, &turn.worker),
            &handoff::stopped_words(window, turn.usage.steps, turn.usage.cost_usd),
            worker.label(),
            task.changed.len(),
            note.is_some(),
        ))
    });
    let role = if followup.is_some() {
        task.followup_role.clone().unwrap_or_else(|| "redo".into())
    } else {
        "worker".to_string()
    };
    let turn = Turn {
        n,
        worker: worker.key().into(),
        cli: Some(worker.cli),
        terminal: Some(started.address.clone()),
        scope_token: started.scope,
        started_ms: now,
        resumes: resume,
        role,
        path_first: prepared.path_first.clone(),
        ..Default::default()
    };
    update(core, &task.name, |task| {
        task.state = TaskState::Running;
        if task.started_ms.is_none() {
            task.started_ms = Some(now);
        }
        task.waiting = None;
        task.worker = worker.key().into();
        if let Some(sentence) = sentence {
            task.handoffs.push(sentence);
        }
        for warning in prepared.warnings.iter().chain(&warnings) {
            if !task.warnings.contains(warning) {
                task.warnings.push(warning.clone());
            }
        }
        task.turns.push(turn);
    });
    core.changed.notify_waiters();
    Ok(())
}
