//! The worker loop, part A: a task is a record on top of a contained terminal.
//! One pump changes tasks one at a time; the terminal runs the runner, which
//! runs the worker, and the process's exit is what ends a turn.

pub mod decide;
pub mod git;
pub mod runner;
pub mod types;
pub mod worker;

use crate::api::{Label, RunParams};
use crate::rpc::RpcError;
use crate::runtime::{Core, now_ms};
use crate::task::decide::{Step, TerminalNow};
use crate::task::types::{Beat, Ending, Task, TaskLine, TaskState, Turn};
use crate::task::worker::TurnOutcome;
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

/// What a wake carries of a report; `result.json` has it whole.
pub const TEXT_LIMIT: usize = 1500;
/// A brief longer than this is refused (it travels as a file, but a record is not a document).
pub const MAX_BRIEF: usize = 200_000;

/// Every task this core knows, and the one lock that changes them.
pub struct Store {
    /// Tasks by name. Never held across an await.
    pub tasks: std::sync::Mutex<BTreeMap<String, Task>>,
    /// The next queue number.
    pub seq: std::sync::atomic::AtomicU64,
    /// Serializes everything that changes a task: the pump, a start, a cancel, a discard.
    pub lock: tokio::sync::Mutex<()>,
    /// Records that could not be read when the core started, one line each.
    pub problems: Vec<String>,
}

impl Store {
    pub fn load(state: &Path) -> Store {
        let dir = state.join("tasks");
        let mut tasks = BTreeMap::new();
        let mut problems = Vec::new();
        let mut highest = 0u64;
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let folder = entry.path();
            if !folder.is_dir() {
                continue;
            }
            let name = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let record = folder.join("record.json");
            let bytes = match std::fs::read(&record) {
                Ok(bytes) => bytes,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => {
                    problems.push(format!("{name}: {e}"));
                    continue;
                }
            };
            match serde_json::from_slice::<Task>(&bytes) {
                Ok(task) => {
                    highest = highest.max(task.seq);
                    tasks.insert(task.name.clone(), task);
                }
                Err(e) => {
                    // One damaged record costs one task, never the core.
                    eprintln!("core: task {name}: cannot read its record; moved to record.broken.json: {e}");
                    let broken = folder.join("record.broken.json");
                    let _ = std::fs::remove_file(&broken);
                    let _ = std::fs::rename(&record, &broken);
                    problems.push(format!("{name}: {e}"));
                }
            }
        }
        Store {
            tasks: std::sync::Mutex::new(tasks),
            seq: std::sync::atomic::AtomicU64::new(highest + 1),
            lock: tokio::sync::Mutex::new(()),
            problems,
        }
    }

    pub fn next_seq(&self) -> u64 {
        self.seq.fetch_add(1, Ordering::SeqCst)
    }

    pub fn snapshot(&self, name: &str) -> Option<Task> {
        self.tasks.lock().unwrap_or_else(|p| p.into_inner()).get(name).cloned()
    }

    pub fn all(&self) -> Vec<Task> {
        self.tasks.lock().unwrap_or_else(|p| p.into_inner()).values().cloned().collect()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.tasks.lock().unwrap_or_else(|p| p.into_inner()).contains_key(name)
    }
}

pub fn task_folder(state: &Path, name: &str) -> PathBuf {
    state.join("tasks").join(name)
}

pub fn worktree_path(state: &Path, name: &str) -> PathBuf {
    state.join("worktrees").join(name)
}

pub fn beat_path(state: &Path, name: &str, turn: u32) -> PathBuf {
    task_folder(state, name).join(format!("turn-{turn}.beat"))
}

pub fn read_beat(state: &Path, name: &str, turn: u32) -> Option<Beat> {
    serde_json::from_slice(&std::fs::read(beat_path(state, name, turn)).ok()?).ok()
}

pub fn read_exit(state: &Path, name: &str, turn: u32) -> Option<i32> {
    let bytes = std::fs::read(task_folder(state, name).join(format!("turn-{turn}.exit"))).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    value.get("code")?.as_i64().and_then(|code| i32::try_from(code).ok())
}

fn save(state: &Path, task: &Task) {
    let folder = task_folder(state, &task.name);
    let bytes = match serde_json::to_vec_pretty(task) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("core: task {}: cannot serialize its record: {e}", task.name);
            return;
        }
    };
    if let Err(e) =
        std::fs::create_dir_all(&folder).and_then(|_| crate::paths::write_atomic(&folder.join("record.json"), &bytes))
    {
        eprintln!("core: task {}: cannot save its record: {e}", task.name);
    }
}

fn write_result(state: &Path, task: &Task) {
    let line = TaskLine::of(task, now_ms(), usize::MAX);
    let Ok(bytes) = serde_json::to_vec_pretty(&line) else { return };
    if let Err(e) = crate::paths::write_atomic(&task_folder(state, &task.name).join("result.json"), &bytes) {
        eprintln!("core: task {}: cannot write its result: {e}", task.name);
    }
}

/// Replace a task under the map lock alone: callers hold the serializing lock.
fn replace(core: &Core, task: Task) {
    core.tasks.tasks.lock().unwrap_or_else(|p| p.into_inner()).insert(task.name.clone(), task);
}

fn update(core: &Core, name: &str, change: impl FnOnce(&mut Task)) -> Option<Task> {
    let mut tasks = core.tasks.tasks.lock().unwrap_or_else(|p| p.into_inner());
    let task = tasks.get_mut(name)?;
    change(task);
    let updated = task.clone();
    drop(tasks);
    save(&core.paths.state, &updated);
    Some(updated)
}

fn stopping(core: &Core) -> bool {
    core.lock().stopping || core.restart.load(Ordering::SeqCst)
}

/// The loop that moves the queue: on every change, at least every 2 s, and
/// never while the core is stopping or restarting.
pub async fn supervise(core: Arc<Core>) {
    loop {
        let notified = core.changed.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if stopping(&core) {
            return;
        }
        pump(core.clone()).await;
        if stopping(&core) {
            return;
        }
        tokio::select! {
            _ = notified => {}
            _ = tokio::time::sleep(Duration::from_secs(2)) => {}
        }
    }
}

/// One pass: end what ended, cut what ran too long, start what may start.
pub async fn pump(core: Arc<Core>) {
    let _serial = core.tasks.lock.lock().await;
    if stopping(&core) {
        return;
    }
    let running: Vec<Task> = core.tasks.all().into_iter().filter(|task| task.state == TaskState::Running).collect();
    for task in running {
        pump_running(&core, task).await;
        if stopping(&core) {
            return;
        }
    }
    let mut queued: Vec<Task> = core.tasks.all().into_iter().filter(|task| task.state == TaskState::Queued).collect();
    queued.sort_by_key(|task| task.seq);
    for task in queued {
        pump_queued(&core, task).await;
        if stopping(&core) {
            return;
        }
    }
}

async fn pump_running(core: &Arc<Core>, task: Task) {
    if stopping(core) {
        return;
    }
    let Some(turn) = task.turns.last().cloned() else {
        finish_with_error(core, &task, "its record says running, but it has no turn");
        return;
    };
    let terminal = terminal_now(core, turn.terminal.as_deref());
    let beat = read_beat(&core.paths.state, &task.name, turn.n);
    match decide::step(&task, terminal.as_ref(), beat.as_ref(), now_ms()) {
        Step::Wait => {}
        Step::Kill(ending) => {
            update(core, &task.name, |task| task.ending = Some(ending));
            kill_turn(core, &task.name, &turn).await;
        }
        Step::TurnOver => finish_turn(core, &task.name, turn.n, terminal.as_ref()).await,
    }
}

async fn pump_queued(core: &Arc<Core>, task: Task) {
    if stopping(core) {
        return;
    }
    let others = core.tasks.all();
    let others: Vec<&Task> = others.iter().filter(|other| other.name != task.name).collect();
    match decide::may_start(&task, &others, core.settings().cap()) {
        Err(waiting) => {
            if task.waiting.as_deref() != Some(waiting.as_str()) {
                update(core, &task.name, |task| task.waiting = Some(waiting));
                core.changed.notify_waiters();
            }
        }
        Ok(()) => {
            if let Err(error) = start_task(core, &task).await {
                fail_start(core, &task, error).await;
            }
        }
    }
}

fn terminal_now(core: &Core, address: Option<&str>) -> Option<TerminalNow> {
    let id = address.and_then(crate::terminals::parse_id)?;
    let st = core.lock();
    let terminal = st.reg.terminals.get(&id)?;
    Some(TerminalNow { live: terminal.is_live(), exit_code: terminal.exit_code, reason: terminal.reason.clone() })
}

async fn kill_turn(core: &Arc<Core>, name: &str, turn: &Turn) {
    if let Some(id) = turn.terminal.as_deref().and_then(crate::terminals::parse_id) {
        core.lock().killing.insert(id);
        core.kill(&[id]).await;
    }
    // Unix can leave a process behind: look for the token, sweep, look again.
    let survivors = sweep_survivors(turn.scope_token.clone()).await;
    update(core, name, |task| {
        if let Some(last) = task.turns.last_mut() {
            last.survivors = survivors;
        }
    });
}

async fn sweep_survivors(token: Option<String>) -> u32 {
    let Some(token) = token else { return 0 };
    let tokens = [token];
    if crate::sweep::members(&tokens).is_empty() {
        return 0;
    }
    let sweep = tokens.clone();
    let _ = tokio::task::spawn_blocking(move || crate::sweep::sweep(&sweep)).await;
    crate::sweep::members(&tokens).len() as u32
}

/// The turn ended: read what it wrote, judge it, keep the diff and the result.
async fn finish_turn(core: &Arc<Core>, name: &str, n: u32, terminal: Option<&TerminalNow>) {
    if stopping(core) {
        return;
    }
    let Some(mut task) = core.tasks.snapshot(name) else { return };
    let Some(index) = task.turns.iter().position(|turn| turn.n == n) else { return };
    let folder = task_folder(&core.paths.state, name);
    let path = folder.join(format!("turn-{n}.jsonl"));
    let outcome = tokio::task::spawn_blocking(move || match std::fs::read(&path) {
        Ok(bytes) => TurnOutcome::from_bytes(&bytes),
        Err(_) => TurnOutcome::default(),
    })
    .await
    .unwrap_or_default();
    let code = terminal.and_then(|terminal| terminal.exit_code).or_else(|| read_exit(&core.paths.state, name, n));
    let ended = now_ms();
    {
        let turn = &mut task.turns[index];
        turn.session = outcome.session.clone();
        turn.text = outcome.text.clone();
        turn.usage = outcome.usage.clone();
        turn.exit_code = code;
        turn.ended_ms = Some(ended);
    }
    let (state, error) = decide::turn_end(&task, terminal, code, &outcome);
    let changed = match task.worktree.as_deref() {
        Some(worktree) => match git::changed_files(worktree, &task.base, &task.scope).await {
            Ok(changed) => changed,
            Err(e) => {
                task.warnings.push(format!("cannot read the changed files: {}", e.message));
                Vec::new()
            }
        },
        None => Vec::new(),
    };
    task.state = state;
    task.error = error;
    task.changed = changed;
    task.ended_ms = Some(ended);
    save(&core.paths.state, &task);
    write_result(&core.paths.state, &task);
    replace(core, task);
    core.changed.notify_waiters();
}

fn finish_with_error(core: &Arc<Core>, task: &Task, message: &str) {
    if stopping(core) {
        return;
    }
    let mut task = task.clone();
    task.state = TaskState::Failed;
    task.error = Some(message.into());
    task.ended_ms = Some(now_ms());
    save(&core.paths.state, &task);
    write_result(&core.paths.state, &task);
    replace(core, task);
    core.changed.notify_waiters();
}

async fn fail_start(core: &Arc<Core>, task: &Task, error: RpcError) {
    if stopping(core) {
        return;
    }
    let mut task = core.tasks.snapshot(&task.name).unwrap_or_else(|| task.clone());
    if let Some(worktree) = task.worktree.clone() {
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
    write_result(&core.paths.state, &task);
    replace(core, task);
    core.changed.notify_waiters();
}

async fn start_task(core: &Arc<Core>, task: &Task) -> Result<(), RpcError> {
    let config = crate::config::load(&core.paths.config)?;
    let worker = worker::Worker::from_key(&task.worker)?;
    let prefix = worker::opencode_prefix(&config)?;
    worker::check_version(&prefix).await?;
    let folder = task_folder(&core.paths.state, &task.name);
    std::fs::create_dir_all(&folder).map_err(|e| unavailable(format!("cannot make {}: {e}", folder.display())))?;
    let brief = std::fs::read_to_string(folder.join("brief.md"))
        .map_err(|e| unavailable(format!("cannot read the task's brief: {e}")))?;
    let prompt = folder.join("turn-1.prompt.md");
    crate::paths::write_atomic(&prompt, brief.as_bytes())
        .map_err(|e| unavailable(format!("cannot write {}: {e}", prompt.display())))?;
    let worktree = worktree_path(&core.paths.state, &task.name).display().to_string();
    git::add_worktree(&task.repo, &worktree, &task.task_branch, &task.base).await?;
    update(core, &task.name, |task| task.worktree = Some(worktree.clone()));
    let command = worker::opencode_command(&prefix, worker, &task.name, &prompt.display().to_string(), None);
    let exe = std::env::current_exe().map_err(|e| unavailable(format!("cannot find this lc: {e}")))?;
    let mut argv = vec![
        exe.display().to_string(),
        "run-logged".into(),
        "--dir".into(),
        folder.display().to_string(),
        "--task".into(),
        task.name.clone(),
        "--turn".into(),
        "1".into(),
        "--worker".into(),
        task.worker.clone(),
        "--".into(),
    ];
    argv.extend(command);
    let params = RunParams {
        argv,
        cwd: Some(worktree),
        name: Some(task.name.clone()),
        labels: vec![Label("task".into(), task.name.clone()), Label("worker".into(), task.worker.clone())],
        contain: true,
        no_parent: true,
        no_notify: true,
        outlive_parent: true,
        ..Default::default()
    };
    let started = crate::api::start_terminal(core, params, None).await?;
    let now = now_ms();
    let turn = Turn {
        n: 1,
        worker: task.worker.clone(),
        terminal: Some(started.address.clone()),
        scope_token: started.scope,
        started_ms: now,
        ..Default::default()
    };
    update(core, &task.name, |task| {
        task.state = TaskState::Running;
        task.started_ms = Some(now);
        task.waiting = None;
        task.turns.push(turn);
    });
    core.changed.notify_waiters();
    Ok(())
}

fn unavailable(message: String) -> RpcError {
    RpcError::new("unavailable", message, "check the state folder's permissions and free space")
}

pub fn not_found(name: &str) -> RpcError {
    RpcError::new("not_found", format!("no task {name:?}"), "see `lc task ls --all`")
}

pub fn name_taken(name: &str) -> RpcError {
    RpcError::new(
        "name_taken",
        format!("{name:?} is already a task or a live terminal here"),
        "pick another name; a task keeps its record, discarded or not",
    )
}

/// Write a new task's record and brief, then let the pump look at it once.
pub async fn create(core: &Arc<Core>, task: Task, brief: &str) -> Result<(), RpcError> {
    let serial = core.tasks.lock.lock().await;
    if stopping(core) {
        return Err(unavailable("the core is stopping".into()));
    }
    if core.tasks.contains(&task.name)
        || core.lock().reg.terminals.values().any(|t| t.is_live() && t.name.as_deref() == Some(&task.name))
    {
        return Err(name_taken(&task.name));
    }
    let folder = task_folder(&core.paths.state, &task.name);
    std::fs::create_dir_all(&folder).map_err(|e| unavailable(format!("cannot make {}: {e}", folder.display())))?;
    crate::paths::write_atomic(&folder.join("brief.md"), brief.as_bytes())
        .map_err(|e| unavailable(format!("cannot write {}: {e}", folder.join("brief.md").display())))?;
    save(&core.paths.state, &task);
    core.tasks.tasks.lock().unwrap_or_else(|p| p.into_inner()).insert(task.name.clone(), task);
    drop(serial);
    pump(core.clone()).await;
    Ok(())
}

/// Cancel a queued task, or kill a running one and finish its turn as cancelled.
/// Returns its state and how many processes survived the kill.
pub async fn cancel(core: &Arc<Core>, name: &str) -> Result<(TaskState, u32), RpcError> {
    let _serial = core.tasks.lock.lock().await;
    if stopping(core) {
        return Err(unavailable("the core is stopping".into()));
    }
    let Some(task) = core.tasks.snapshot(name) else { return Err(not_found(name)) };
    match task.state {
        TaskState::Queued => {
            let mut task = task;
            task.state = TaskState::Cancelled;
            task.waiting = None;
            task.ended_ms = Some(now_ms());
            save(&core.paths.state, &task);
            write_result(&core.paths.state, &task);
            replace(core, task);
            core.changed.notify_waiters();
            Ok((TaskState::Cancelled, 0))
        }
        TaskState::Running => {
            update(core, name, |task| task.ending = Some(Ending::Cancel));
            let Some(turn) = task.turns.last().cloned() else {
                finish_with_error(core, &task, "its record says running, but it has no turn");
                return Ok((TaskState::Failed, 0));
            };
            kill_turn(core, name, &turn).await;
            let terminal = terminal_now(core, turn.terminal.as_deref());
            finish_turn(core, name, turn.n, terminal.as_ref()).await;
            let done = core.tasks.snapshot(name).unwrap_or(task);
            Ok((done.state, done.turns.last().map_or(0, |turn| turn.survivors)))
        }
        other => {
            Err(RpcError::new("finished", format!("{name} has already ended: {}", other.word()), "see `lc task show`"))
        }
    }
}

/// Remove a finished task's worktree and branch, keeping its folder and record.
pub async fn discard(
    core: &Arc<Core>,
    name: &str,
    review: Option<String>,
) -> Result<(TaskState, bool, Vec<String>), RpcError> {
    let _serial = core.tasks.lock.lock().await;
    if stopping(core) {
        return Err(unavailable("the core is stopping".into()));
    }
    let Some(mut task) = core.tasks.snapshot(name) else { return Err(not_found(name)) };
    if matches!(task.state, TaskState::Queued | TaskState::Running) {
        return Err(RpcError::new(
            "running",
            format!("{name} is still {}: cancel it first", task.state.word()),
            format!("lc task cancel {name}"),
        ));
    }
    let removal = git::remove_worktree(&task.repo, task.worktree.as_deref(), Some(&task.task_branch)).await;
    if removal.removed {
        task.worktree = None;
    }
    task.state = TaskState::Discarded;
    if review.is_some() {
        task.review = review;
    }
    save(&core.paths.state, &task);
    write_result(&core.paths.state, &task);
    replace(core, task);
    core.changed.notify_waiters();
    Ok((TaskState::Discarded, removal.removed, removal.left))
}
