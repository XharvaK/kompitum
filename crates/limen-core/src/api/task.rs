//! `task.*`: start, list, get, wait, cancel and discard the worker loop's
//! records. The deciding happens in `task::decide`; this file checks requests
//! and reports what the pump observed.

use super::*;
use crate::task::TEXT_LIMIT;
use crate::task::types::{Task, TaskLine, TaskState};
use crate::task::worker::Worker;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, JsonSchema)]
pub struct TaskStartParams {
    /// The task's name, using the terminal name rules (1-32 of a-z 0-9 - _ ., starting with a letter or digit).
    pub task: String,
    /// The brief the worker is given, at most 200000 characters.
    pub brief: String,
    /// The folder inside the repository (default: the core's own folder).
    #[serde(default)]
    pub folder: Option<String>,
    /// Paths the task may change; empty means the whole repository.
    #[serde(default)]
    pub scope: Vec<String>,
    /// The worker: go (the default) or free.
    #[serde(default)]
    pub worker: Option<String>,
    /// Seconds without output or CPU before the turn is cut (default 900).
    #[serde(default)]
    pub idle: Option<u64>,
    /// Seconds the turn may run (default 3600).
    #[serde(default)]
    pub timeout: Option<u64>,
    /// The caller's terminal.
    #[serde(default)]
    pub caller: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct TaskStartResult {
    /// The task's name.
    pub task: String,
    /// Its state right after the pump looked at it.
    pub state: TaskState,
    /// Where its worktree is, once there is one.
    pub worktree: Option<String>,
    /// The worker's terminal (`t7`), or null while it is still queued.
    pub terminal: Option<String>,
    /// The worker's label ("opencode Go").
    pub worker: String,
    /// Why it is still queued, in words.
    pub waiting: Option<String>,
    /// Why it failed, when the start already went wrong.
    pub error: Option<String>,
    /// Things worth knowing before the worker runs, like uncommitted files.
    pub warnings: Vec<String>,
    /// The command that waits for it.
    pub next: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct TaskListParams {
    /// Include discarded tasks too.
    #[serde(default)]
    pub all: bool,
}

#[derive(Serialize, JsonSchema)]
pub struct TaskListResult {
    /// One wake per task, its report cut as in `task.wait`.
    pub tasks: Vec<TaskLine>,
    /// How many tasks run now.
    pub running: usize,
    /// How many wait in the queue.
    pub queued: usize,
    /// How many may run at once.
    pub cap: usize,
    /// Records the core could not read when it started.
    pub problems: Vec<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct TaskParams {
    /// The task's name.
    pub task: String,
}

#[derive(Serialize, JsonSchema)]
pub struct TaskProgress {
    /// Steps finished so far.
    pub steps: u64,
    /// The last five tools, oldest first.
    pub tools: Vec<String>,
    /// What the turn has cost so far.
    pub cost_usd: f64,
    /// The largest request so far: input plus cache read plus cache write.
    pub peak: u64,
    /// Bytes the worker has printed.
    pub out_bytes: u64,
    /// How long the runner has been silent.
    pub seconds_since_event: f64,
}

#[derive(Serialize, JsonSchema)]
pub struct TaskView {
    /// The whole record.
    pub task: Task,
    /// The same task as a wake.
    pub line: TaskLine,
    /// Its folder: the brief, each turn's files, and result.json.
    pub folder: String,
    /// The current turn's heartbeat, when there is one.
    pub progress: Option<TaskProgress>,
}

#[derive(Deserialize, JsonSchema)]
pub struct TaskWaitParams {
    /// The tasks to wait for.
    pub tasks: Vec<String>,
    /// Return at the first task that ends, however it ended.
    #[serde(default)]
    pub first: bool,
    /// Seconds before it checks in (default 60, at most 300).
    #[serde(default)]
    pub timeout: Option<f64>,
}

#[derive(Serialize, JsonSchema)]
pub struct TaskWaitResult {
    /// Known tasks, in the order asked for.
    pub lines: Vec<TaskLine>,
    /// Known tasks that have not ended.
    pub unfinished: Vec<String>,
    /// Names no task here uses.
    pub unknown: Vec<String>,
    /// Every task has ended (or, with `first`, one has), and none is unknown.
    pub over: bool,
}

#[derive(Serialize, JsonSchema)]
pub struct TaskCancelResult {
    /// The task's name.
    pub task: String,
    /// Its state once it has ended.
    pub state: TaskState,
    /// Processes carrying the turn's scope that outlived the kill.
    pub survivors: u32,
}

#[derive(Deserialize, JsonSchema)]
pub struct TaskDiscardParams {
    /// The task's name.
    pub task: String,
    /// What the review decided, kept with the record.
    #[serde(default)]
    pub review: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct TaskDiscardResult {
    /// The task's name.
    pub task: String,
    /// Its state now: discarded.
    pub state: TaskState,
    /// Whether its worktree was removed.
    pub removed: bool,
    /// What could not be removed, in words.
    pub left: Vec<String>,
}

fn unavailable(message: impl std::fmt::Display) -> RpcError {
    RpcError::new("unavailable", message.to_string(), "check the state folder's permissions and free space")
}

pub(super) async fn task_start(core: Ctx<'_>, p: TaskStartParams) -> Result<TaskStartResult, RpcError> {
    crate::addr::check_name(&p.task)?;
    let length = p.brief.chars().count();
    if length == 0 || length > crate::task::MAX_BRIEF {
        return Err(RpcError::invalid_params(format!(
            "a brief is 1 to {} characters; this one has {length}",
            crate::task::MAX_BRIEF
        )));
    }
    if core.tasks.contains(&p.task)
        || core.lock().reg.terminals.values().any(|t| t.is_live() && t.name.as_deref() == Some(&p.task))
    {
        return Err(crate::task::name_taken(&p.task));
    }
    let key = p.worker.clone().unwrap_or_else(|| "go".into());
    let worker = Worker::from_key(&key)?;
    let config = crate::config::load(&core.paths.config)?;
    let prefix = crate::task::worker::opencode_prefix(&config)?;
    crate::task::worker::check_version(&prefix).await?;
    let folder = p.folder.clone().or_else(|| std::env::current_dir().ok().map(|p| p.display().to_string()));
    let Some(folder) = folder else { return Err(unavailable("cannot read the core's own folder")) };
    let (repo, branch, base) = crate::task::git::locate(&folder).await?;
    let idle = p.idle.unwrap_or(900);
    let wall = p.timeout.unwrap_or(3600);
    if idle == 0 || wall == 0 {
        return Err(RpcError::invalid_params("idle and timeout are at least 1 second"));
    }
    let dirty = crate::task::git::dirty_count(&repo).await?;
    let warnings = if dirty > 0 {
        vec![format!(
            "{dirty} uncommitted files in {repo} are not in the worktree: commit first if the worker needs them"
        )]
    } else {
        Vec::new()
    };
    let task = Task {
        name: p.task.clone(),
        seq: core.tasks.next_seq(),
        state: TaskState::Queued,
        repo,
        branch,
        base,
        task_branch: format!("task/{}", p.task),
        worktree: None,
        scope: p.scope,
        worker: key,
        idle_s: idle,
        wall_s: wall,
        started_by: p.caller,
        warnings,
        waiting: Some("waiting for the pump".into()),
        turns: Vec::new(),
        ending: None,
        error: None,
        changed: Vec::new(),
        review: None,
        created_ms: now_ms(),
        started_ms: None,
        ended_ms: None,
    };
    crate::task::create(core.core, task, &p.brief).await?;
    let task = core.tasks.snapshot(&p.task).ok_or_else(|| crate::task::not_found(&p.task))?;
    Ok(TaskStartResult {
        task: p.task.clone(),
        state: task.state,
        worktree: task.worktree.clone(),
        terminal: task.turns.first().and_then(|turn| turn.terminal.clone()),
        worker: worker.label().into(),
        waiting: task.waiting.clone(),
        error: task.error.clone(),
        warnings: task.warnings.clone(),
        next: format!("lc task wait {}", p.task),
    })
}

pub(super) async fn task_list(core: Ctx<'_>, p: TaskListParams) -> Result<TaskListResult, RpcError> {
    let now = now_ms();
    let mut all = core.tasks.all();
    all.sort_by_key(|task| task.seq);
    let running = all.iter().filter(|task| task.state == TaskState::Running).count();
    let queued = all.iter().filter(|task| task.state == TaskState::Queued).count();
    let tasks = all
        .iter()
        .filter(|task| p.all || task.state != TaskState::Discarded)
        .map(|task| TaskLine::of(task, now, TEXT_LIMIT))
        .collect();
    Ok(TaskListResult { tasks, running, queued, cap: core.settings().cap(), problems: core.tasks.problems.clone() })
}

pub(super) async fn task_get(core: Ctx<'_>, p: TaskParams) -> Result<TaskView, RpcError> {
    let task = core.tasks.snapshot(&p.task).ok_or_else(|| crate::task::not_found(&p.task))?;
    let now = now_ms();
    let line = TaskLine::of(&task, now, TEXT_LIMIT);
    let progress = task.turns.last().and_then(|turn| {
        crate::task::read_beat(&core.paths.state, &task.name, turn.n).map(|beat| TaskProgress {
            steps: beat.steps,
            tools: beat.tools,
            cost_usd: beat.cost_usd,
            peak: beat.peak,
            out_bytes: beat.out_bytes,
            seconds_since_event: now.saturating_sub(beat.active_ms) as f64 / 1000.0,
        })
    });
    Ok(TaskView {
        task: task.clone(),
        line,
        folder: crate::task::task_folder(&core.paths.state, &task.name).display().to_string(),
        progress,
    })
}

fn wait_snapshot(core: &Core, names: &[String], first: bool) -> TaskWaitResult {
    let now = now_ms();
    let mut lines = Vec::new();
    let mut unfinished = Vec::new();
    let mut unknown = Vec::new();
    for name in names {
        match core.tasks.snapshot(name) {
            Some(task) => {
                if !task.state.ended() {
                    unfinished.push(name.clone());
                }
                lines.push(TaskLine::of(&task, now, TEXT_LIMIT));
            }
            None => unknown.push(name.clone()),
        }
    }
    let over = unknown.is_empty() && crate::task::decide::wait_over(&lines, first);
    TaskWaitResult { lines, unfinished, unknown, over }
}

pub(super) async fn task_wait(core: Ctx<'_>, p: TaskWaitParams) -> Result<TaskWaitResult, RpcError> {
    if p.tasks.is_empty() {
        return Err(RpcError::invalid_params("give at least one task to wait for"));
    }
    let limit = p.timeout.unwrap_or(60.0);
    if !limit.is_finite() || !(0.0..=300.0).contains(&limit) {
        return Err(RpcError::invalid_params("timeout must be 0 to 300 seconds"));
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs_f64(limit);
    loop {
        // Armed before the snapshot, so a change between the two is not missed.
        let notified = core.changed.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        let result = wait_snapshot(core.core, &p.tasks, p.first);
        if result.over {
            return Ok(result);
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Ok(result);
        }
        let _ = tokio::time::timeout(remaining, notified).await;
    }
}

pub(super) async fn task_cancel(core: Ctx<'_>, p: TaskParams) -> Result<TaskCancelResult, RpcError> {
    let (state, survivors) = crate::task::cancel(core.core, &p.task).await?;
    Ok(TaskCancelResult { task: p.task, state, survivors })
}

pub(super) async fn task_discard(core: Ctx<'_>, p: TaskDiscardParams) -> Result<TaskDiscardResult, RpcError> {
    let (state, removed, left) = crate::task::discard(core.core, &p.task, p.review).await?;
    Ok(TaskDiscardResult { task: p.task, state, removed, left })
}
