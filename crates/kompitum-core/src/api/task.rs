//! `task.*`: start, list, get, wait, cancel and discard the worker loop's
//! records. The deciding happens in `task::decide`; this file checks requests
//! and reports what the pump observed.

use super::*;
use crate::task::TEXT_LIMIT;
use crate::task::quota::Source;
use crate::task::types::{Asked, Task, TaskLine, TaskState};
use crate::task::updates::CliUpdate;
use crate::task::worker::{Cli, Worker};
use crate::task::workers::{self, Spend};
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
    /// The worker: auto, free, or a known worker key. With none, the mode `kom workers` shows.
    #[serde(default)]
    pub worker: Option<String>,
    /// Seconds without output, CPU or I/O before the turn is cut (default 900).
    #[serde(default)]
    pub idle: Option<u64>,
    /// Seconds the turn may run (default: none).
    #[serde(default)]
    pub timeout: Option<u64>,
    /// The test command the checks run when the worker ends, replacing the repository's.
    #[serde(default)]
    pub test: Option<String>,
    /// Regex or glob::regex: no added line may match (added to the repository's).
    #[serde(default)]
    pub forbid: Vec<String>,
    /// glob::regex: some line of the matching files must match after the run (added to the repository's).
    #[serde(default)]
    pub require: Vec<String>,
    /// Regex or glob::regex: a hit flags the line for the review (added to the repository's).
    #[serde(default)]
    pub warn: Vec<String>,
    /// How many times failed checks go back to the worker, 0 to 3 (default: the repository's, else 1).
    #[serde(default)]
    pub qa_rounds: Option<u8>,
    /// Let the worker use the web.
    #[serde(default)]
    pub web: bool,
    /// The caller's terminal.
    #[serde(default)]
    pub caller: Option<String>,
    /// Who starts it: opus, manager or owner (default: owner).
    #[serde(default)]
    pub by: Option<String>,
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
    /// The worker's label ("opencode Go"), once one is picked.
    pub worker: Option<String>,
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
    /// How many may run at once; null for no cap.
    pub cap: Option<usize>,
    /// Records the core could not read when it started.
    pub problems: Vec<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct TaskParams {
    /// The task's name.
    pub task: String,
    /// Who is asking: opus, manager or owner (default: owner).
    #[serde(default)]
    pub by: Option<String>,
    /// Cancel another caller's task.
    #[serde(default)]
    pub yes: bool,
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
    /// Return at the first task that ends or asks, however it did.
    #[serde(default)]
    pub first: bool,
    /// Keep waiting while every worker is out; the task goes on by itself when one comes back.
    #[serde(default)]
    pub while_stranded: bool,
    /// Seconds before it checks in (default 60, at most 300).
    #[serde(default)]
    pub timeout: Option<f64>,
}

#[derive(Serialize, JsonSchema)]
pub struct TaskWaitResult {
    /// Known tasks, in the order asked for.
    pub lines: Vec<TaskLine>,
    /// Known tasks that have not ended or asked.
    pub unfinished: Vec<String>,
    /// Names no task here uses.
    pub unknown: Vec<String>,
    /// Every task has ended or is asking (or, with `first`, one has), and none is
    /// unknown; a stranded task ends it too unless `while_stranded` was asked for.
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
    /// Who is asking: opus, manager or owner (default: owner).
    #[serde(default)]
    pub by: Option<String>,
    /// Discard another caller's task.
    #[serde(default)]
    pub yes: bool,
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
    let file = core.tasks.workers();
    let mode = p.worker.as_deref().unwrap_or(&file.mode);
    let chain = workers::chain_for(&file, mode)?;
    let config = crate::config::load(&core.paths.config)?;
    let installed = |worker: &Worker| crate::task::worker::prefix(worker, &config).is_ok();
    if !chain.iter().any(installed) {
        let mut words: Vec<String> = chain
            .iter()
            .filter_map(|worker| crate::task::worker::prefix(worker, &config).err())
            .map(|e| format!("{}.", e.message.trim_end_matches('.')))
            .collect();
        words.dedup();
        return Err(RpcError::new(
            "worker_missing",
            words.join(" "),
            "install a worker CLI, or set [workers] codex, [workers] commandcode, [workers] opencode or [workers] claude in ~/.config/kompitum-core/config.toml",
        ));
    }
    let folder = p.folder.clone().or_else(|| std::env::current_dir().ok().map(|p| p.display().to_string()));
    let Some(folder) = folder else { return Err(unavailable("cannot read the core's own folder")) };
    let (repo, branch, base) = crate::task::git::locate(&folder).await?;
    let idle = p.idle.unwrap_or(900);
    let wall = p.timeout.unwrap_or(0);
    if idle == 0 || p.timeout == Some(0) {
        return Err(RpcError::invalid_params("idle and timeout are at least 1 second"));
    }
    if p.qa_rounds.is_some_and(|rounds| i64::from(rounds) > crate::task::checks::MAX_QA_ROUNDS) {
        return Err(RpcError::invalid_params(format!("qa_rounds is 0 to {}", crate::task::checks::MAX_QA_ROUNDS)));
    }
    let dirty = crate::task::git::dirty_count(&repo).await?;
    let mut warnings = if dirty > 0 {
        vec![format!(
            "{dirty} uncommitted files in {repo} are not in the worktree: commit first if the worker needs them"
        )]
    } else {
        Vec::new()
    };
    let asked =
        Asked { test: p.test, forbid: p.forbid, require: p.require, warn: p.warn, qa_rounds: p.qa_rounds, web: p.web };
    let (start_warnings, one_at_a_time) =
        crate::task::prompt::start_warnings(&repo, &base, &p.brief, &p.scope, &asked).await;
    warnings.extend(start_warnings);
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
        worker: String::new(),
        chain: chain.iter().map(|worker| worker.key().to_string()).collect(),
        handoffs: Vec::new(),
        idle_s: idle,
        wall_s: wall,
        started_by: p.caller,
        by: p.by.as_deref().map(str::trim).filter(|by| !by.is_empty()).map(String::from),
        owner_said: None,
        warnings,
        waiting: Some("waiting for the pump".into()),
        turns: Vec::new(),
        ending: None,
        error: None,
        question: None,
        changed: Vec::new(),
        review: None,
        asked,
        one_at_a_time,
        shared: false,
        one_writer: false,
        checks: Vec::new(),
        check: None,
        qa_used: 0,
        followup_role: None,
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
        terminal: task.turns.last().and_then(|turn| turn.terminal.clone()),
        worker: workers::find(&file, &task.worker).ok().map(|worker| worker.label),
        waiting: task.waiting.clone(),
        error: task.error.clone(),
        warnings: task.warnings.clone(),
        next: format!("kom task wait {}", p.task),
    })
}

pub(super) async fn task_list(core: Ctx<'_>, p: TaskListParams) -> Result<TaskListResult, RpcError> {
    let now = now_ms();
    let mut all = core.tasks.all();
    all.sort_by_key(|task| task.seq);
    let running = all.iter().filter(|task| matches!(task.state, TaskState::Running | TaskState::Checking)).count();
    let queued = all.iter().filter(|task| task.state == TaskState::Queued).count();
    let workers = workers::all(&core.tasks.workers());
    let tasks = all
        .iter()
        .filter(|task| p.all || task.state != TaskState::Discarded)
        .map(|task| TaskLine::of(task, &workers, now, TEXT_LIMIT))
        .collect();
    Ok(TaskListResult { tasks, running, queued, cap: core.settings().cap(), problems: core.tasks.problems.clone() })
}

pub(super) async fn task_get(core: Ctx<'_>, p: TaskParams) -> Result<TaskView, RpcError> {
    let task = core.tasks.snapshot(&p.task).ok_or_else(|| crate::task::not_found(&p.task))?;
    let now = now_ms();
    let workers = workers::all(&core.tasks.workers());
    let line = TaskLine::of(&task, &workers, now, TEXT_LIMIT);
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

fn wait_snapshot(core: &Core, names: &[String], first: bool, while_stranded: bool) -> TaskWaitResult {
    let now = now_ms();
    let workers = workers::all(&core.tasks.workers());
    let mut lines = Vec::new();
    let mut unfinished = Vec::new();
    let mut unknown = Vec::new();
    for name in names {
        match core.tasks.snapshot(name) {
            Some(task) => {
                if !task.state.ended() && task.state != TaskState::Asking {
                    unfinished.push(name.clone());
                }
                lines.push(TaskLine::of(&task, &workers, now, TEXT_LIMIT));
            }
            None => unknown.push(name.clone()),
        }
    }
    let over = unknown.is_empty() && crate::task::decide::wait_over(&lines, first, while_stranded);
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
        let result = wait_snapshot(core.core, &p.tasks, p.first, p.while_stranded);
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
    let (state, survivors) = crate::task::cancel(core.core, &p.task, p.by.as_deref(), p.yes).await?;
    Ok(TaskCancelResult { task: p.task, state, survivors })
}

pub(super) async fn task_discard(core: Ctx<'_>, p: TaskDiscardParams) -> Result<TaskDiscardResult, RpcError> {
    let (state, removed, left) = crate::task::discard(core.core, &p.task, p.review, p.by.as_deref(), p.yes).await?;
    Ok(TaskDiscardResult { task: p.task, state, removed, left })
}

#[derive(Deserialize, JsonSchema)]
pub struct WorkersBlockParam {
    /// The worker to mark: a known worker key.
    pub worker: String,
    /// When it is back: a date, a date and clock, or a span such as 3h.
    #[serde(default)]
    pub until: Option<String>,
    /// Why, in your words.
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct WorkersAddParam {
    /// The new worker's key, using the terminal name rules (1-32 of a-z 0-9 - _ ., starting with a letter or digit).
    pub key: String,
    /// Its label; the key when absent.
    #[serde(default)]
    pub label: Option<String>,
    /// Its CLI: codex, command_code, opencode or claude.
    pub cli: Cli,
    /// The model route it runs; not empty.
    pub route: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct WorkersSetParams {
    /// The mode new tasks use: auto or a known worker key.
    #[serde(default)]
    pub mode: Option<String>,
    /// The new auto chain: known keys, no repeats, at least one.
    #[serde(default)]
    pub order: Option<Vec<String>>,
    /// Add a worker, at the end of the auto chain.
    #[serde(default)]
    pub add: Option<WorkersAddParam>,
    /// Remove an added worker, its block and its place in the order.
    #[serde(default)]
    pub remove: Option<String>,
    /// Mark a worker out until a time.
    #[serde(default)]
    pub block: Option<WorkersBlockParam>,
    /// Say this worker is back now.
    #[serde(default)]
    pub clear: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct WorkersView {
    /// The mode new tasks use.
    pub mode: String,
    /// The auto chain, with the default filled in.
    pub order: Vec<String>,
    /// The worker keys this mode tries, in order.
    pub chain: Vec<String>,
    /// The key of the worker the next task would use, or null when all are out.
    pub next: Option<String>,
    /// Why every worker is out, when every one is.
    pub all_out: Option<String>,
    /// Things worth knowing, like a damaged workers.json.
    pub warnings: Vec<String>,
    /// The CLIs the core keeps up to date, and where each one stands.
    pub clis: Vec<CliUpdate>,
    /// Tasks whose worker asked a question and waits for the lead's answer.
    pub asking: Vec<AskingView>,
    pub workers: Vec<WorkerView>,
}

#[derive(Serialize, JsonSchema)]
pub struct AskingView {
    pub task: String,
    pub question: String,
}

#[derive(Serialize, JsonSchema)]
pub struct WorkerView {
    pub key: String,
    pub label: String,
    /// codex, command_code, opencode or claude.
    pub cli: Cli,
    /// The model route it runs.
    pub route: String,
    /// ready, out or not_installed.
    pub state: String,
    /// When its block ends, or 0.
    pub until_ms: u64,
    /// Who set the block: person, named or estimate.
    pub source: Option<Source>,
    pub window: String,
    pub message: String,
    /// The block in words, as `kom workers` shows it.
    pub sentence: String,
    /// Credits refusals in a row.
    pub refusals: u32,
    pub spend: Spend,
}

fn workers_view(core: &Core) -> WorkersView {
    let file = core.tasks.workers();
    let mut warnings = Vec::new();
    let config = crate::config::load(&core.paths.config).unwrap_or_else(|e| {
        warnings.push(format!("cannot read the config: {}", e.message));
        Default::default()
    });
    let now = now_ms();
    let offset = crate::platform::utc_offset_minutes();
    let installed = |worker: &Worker| crate::task::worker::prefix(worker, &config).is_ok();
    let chain = workers::chain(&file);
    let (next, all_out) = match workers::pick(&chain, &file, installed, now, offset) {
        Ok(worker) => (Some(worker.key), None),
        Err(words) => (None, Some(words)),
    };
    let spend = workers::spend(&core.tasks.all(), now);
    let workers = workers::all(&file)
        .into_iter()
        .map(|worker| {
            let block = file.blocks.get(&worker.key).filter(|block| block.until_ms > now);
            let missing = crate::task::worker::prefix(&worker, &config).err();
            let state = if missing.is_some() {
                "not_installed"
            } else if block.is_some() {
                "out"
            } else {
                "ready"
            };
            WorkerView {
                key: worker.key.clone(),
                label: worker.label.clone(),
                cli: worker.cli,
                route: worker.route.clone(),
                state: state.into(),
                until_ms: block.map_or(0, |block| block.until_ms),
                source: block.map(|block| block.source),
                window: block.map_or(String::new(), |block| block.window.clone()),
                message: match (&missing, block) {
                    (Some(missing), _) => missing.message.clone(),
                    (None, Some(block)) => block.message.clone(),
                    (None, None) => String::new(),
                },
                sentence: block.map_or(String::new(), |block| workers::sentence(&worker.label, block, now, offset)),
                refusals: block.map_or(0, |block| block.refusals),
                spend: spend.get(&worker.key).cloned().unwrap_or_default(),
            }
        })
        .collect();
    if let Some(problem) = &core.tasks.worker_problem {
        warnings.push(problem.clone());
    }
    let order = workers::order(&file);
    let clis = crate::task::updates::views(core, &config);
    let asking = core
        .tasks
        .all()
        .into_iter()
        .filter(|task| task.state == TaskState::Asking)
        .filter_map(|task| Some(AskingView { question: task.question.clone()?, task: task.name }))
        .collect();
    WorkersView {
        mode: file.mode,
        order,
        chain: chain.iter().map(|worker| worker.key.clone()).collect(),
        next,
        all_out,
        warnings,
        clis,
        asking,
        workers,
    }
}

pub(super) async fn workers_show(core: Ctx<'_>, _: Empty) -> Result<WorkersView, RpcError> {
    Ok(workers_view(core.core))
}

pub(super) async fn workers_set(core: Ctx<'_>, p: WorkersSetParams) -> Result<WorkersView, RpcError> {
    let serial = core.tasks.lock.lock().await;
    let now = now_ms();
    let offset = crate::platform::utc_offset_minutes();
    let mut file = core.tasks.workers();
    if let Some(add) = p.add {
        let key = add.key;
        let label = add.label.filter(|label| !label.trim().is_empty()).unwrap_or_else(|| key.clone());
        workers::add_worker(&mut file, Worker { key, label, cli: add.cli, route: add.route })?;
    }
    if let Some(order) = p.order {
        workers::set_order(&mut file, order)?;
    }
    if let Some(remove) = p.remove.as_deref() {
        workers::remove_worker(&mut file, remove)?;
    }
    if let Some(mode) = p.mode.as_deref() {
        workers::set_mode(&mut file, mode)?;
    }
    if let Some(block) = p.block {
        let worker = workers::find(&file, &block.worker)?;
        let Some(until) = block.until else {
            return Err(RpcError::invalid_params("a block needs --until"));
        };
        let until_ms = crate::task::clock::parse_when(&until, now, offset).map_err(RpcError::invalid_params)?;
        file.blocks.insert(
            worker.key,
            workers::Block {
                until_ms,
                window: String::new(),
                source: Source::Person,
                message: block.reason.unwrap_or_default(),
                task: None,
                refusals: 0,
                set_ms: now,
            },
        );
    }
    if let Some(clear) = p.clear.as_deref() {
        let worker = workers::find(&file, clear)?;
        file.blocks.remove(&worker.key);
    }
    crate::task::save_workers(&core.paths.state, &file)
        .map_err(|e| unavailable(format!("cannot save workers.json: {e}")))?;
    *core.tasks.workers.lock().unwrap_or_else(|p| p.into_inner()) = file;
    drop(serial);
    // A stranded task must hear about a cleared block at once, not at the next pass.
    core.changed.notify_waiters();
    Ok(workers_view(core.core))
}
