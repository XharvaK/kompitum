//! The worker loop, part A: a task is a record on top of a contained terminal.
//! One pump changes tasks one at a time; the terminal runs the runner, which
//! runs the worker, and the process's exit is what ends a turn.

pub mod accept;
pub mod brief;
pub mod checking;
pub mod checks;
pub mod claude;
pub mod clock;
pub mod codex;
pub mod commandcode;
pub mod decide;
pub mod git;
pub mod handoff;
pub mod ledger;
pub mod names;
pub mod opencode;
pub mod parent_usage;
pub mod parts;
pub mod prompt;
pub mod quota;
pub mod rate_wait;
pub mod review;
pub mod runner;
pub mod setup;
pub mod stats;
pub mod sweep;
pub mod table;
mod turn_end;
mod turn_start;
pub mod types;
pub mod updates;
pub mod worker;
pub mod workers;
pub mod workshop;

use crate::rpc::RpcError;
use crate::runtime::{Core, now_ms};
use crate::task::decide::{Step, TerminalNow};
use crate::task::turn_end::{fail_start, finish_turn, finish_with_error};
use crate::task::turn_start::start_task;
use crate::task::types::{Beat, Ending, Task, TaskLine, TaskState, Turn};
use crate::task::worker::{Cli, Worker};
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

/// What a wake carries of a report; `result.json` has it whole.
pub const TEXT_LIMIT: usize = 1500;
/// A brief longer than this is refused (it travels as a file, but a record is not a document).
pub const MAX_BRIEF: usize = 200_000;
/// A task moves to its next worker at most this many times before it stops.
pub const MAX_TURNS: usize = 8;

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
    /// The worker chain's mode and blocks, written on each change.
    pub workers: std::sync::Mutex<workers::WorkersFile>,
    /// Why `workers.json` could not be read, when it could not.
    pub worker_problem: Option<String>,
    /// The last worker start, kept in memory only, for the 8 s start slot.
    pub last_start_ms: std::sync::Mutex<Option<u64>>,
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
        let workers_path = state.join("workers.json");
        let mut worker_file = workers::WorkersFile::default();
        let mut worker_problem = None;
        match std::fs::read(&workers_path) {
            Ok(bytes) => match serde_json::from_slice::<workers::WorkersFile>(&bytes) {
                Ok(file) => worker_file = file,
                Err(e) => {
                    // A damaged file is a warning, never a reason to stop the core.
                    eprintln!("core: workers.json: cannot read it; moved to workers.broken.json: {e}");
                    let broken = state.join("workers.broken.json");
                    let _ = std::fs::remove_file(&broken);
                    let _ = std::fs::rename(&workers_path, &broken);
                    worker_problem = Some(format!("workers.json: {e}"));
                }
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => worker_problem = Some(format!("workers.json: {e}")),
        }
        Store {
            tasks: std::sync::Mutex::new(tasks),
            seq: std::sync::atomic::AtomicU64::new(highest + 1),
            lock: tokio::sync::Mutex::new(()),
            problems,
            workers: std::sync::Mutex::new(worker_file),
            worker_problem,
            last_start_ms: std::sync::Mutex::new(None),
        }
    }

    /// The mode and blocks as the pump last saw them.
    pub fn workers(&self) -> workers::WorkersFile {
        self.workers.lock().unwrap_or_else(|p| p.into_inner()).clone()
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

    /// Whether `name` holds a record that may not be reused. A discarded record
    /// is not taken: `create` moves it aside first.
    pub fn contains(&self, name: &str) -> bool {
        self.snapshot(name).is_some_and(|task| task.state != TaskState::Discarded)
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

/// The worker file changes rarely; each change lands whole.
pub fn save_workers(state: &Path, file: &workers::WorkersFile) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(file).map_err(std::io::Error::other)?;
    crate::paths::write_atomic(&state.join("workers.json"), &bytes)
}

fn write_result(state: &Path, task: &Task, workers: &[Worker]) {
    let line = TaskLine::of(task, workers, now_ms(), usize::MAX);
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

fn worker_list(core: &Core) -> Vec<Worker> {
    workers::all(&core.tasks.workers())
}

/// The loop that moves the queue: on every change, at least every 2 s, and
/// never while the core is stopping or restarting.
pub async fn supervise(core: Arc<Core>) {
    let mut last_sweep = None;
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
        sweep::when_due(&core, &mut last_sweep).await;
        updates::when_due(&core).await;
        tokio::select! {
            _ = notified => {}
            _ = tokio::time::sleep(Duration::from_secs(2)) => {}
        }
    }
}

/// One pump pass's inputs, read once: the tasks as the pass began, the config,
/// and each worker's prefix. Nothing here is re-read per task.
struct Pass<'a> {
    tasks: &'a [Task],
    config: crate::config::Config,
    config_error: Option<RpcError>,
    prefixes: std::sync::Mutex<BTreeMap<String, Result<Vec<String>, RpcError>>>,
    #[cfg(test)]
    prefix_calls: std::sync::atomic::AtomicUsize,
}

impl<'a> Pass<'a> {
    fn new(tasks: &'a [Task], loaded: Result<crate::config::Config, RpcError>) -> Pass<'a> {
        let (config, config_error) = match loaded {
            Ok(config) => (config, None),
            Err(error) => (crate::config::Config::default(), Some(error)),
        };
        Pass {
            tasks,
            config,
            config_error,
            prefixes: std::sync::Mutex::new(BTreeMap::new()),
            #[cfg(test)]
            prefix_calls: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// The same config and cached prefixes, with the snapshot the pass will use.
    fn rebind<'b>(self, tasks: &'b [Task]) -> Pass<'b> {
        Pass {
            tasks,
            config: self.config,
            config_error: self.config_error,
            prefixes: self.prefixes,
            #[cfg(test)]
            prefix_calls: self.prefix_calls,
        }
    }

    fn tasks_in(&self, state: TaskState) -> Vec<Task> {
        self.tasks.iter().filter(|task| task.state == state).cloned().collect()
    }

    fn prefix(&self, worker: &Worker) -> Result<Vec<String>, RpcError> {
        let prefixes = self.prefixes.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(prefix) = prefixes.get(worker.key()) {
            return prefix.clone();
        }
        drop(prefixes);
        #[cfg(test)]
        self.prefix_calls.fetch_add(1, Ordering::SeqCst);
        let prefix = worker::prefix(worker, &self.config);
        self.prefixes.lock().unwrap_or_else(|p| p.into_inner()).insert(worker.key().into(), prefix.clone());
        prefix
    }

    fn installed(&self, worker: &Worker) -> bool {
        self.prefix(worker).is_ok()
    }
}

type VersionChecks = BTreeMap<String, Result<(), RpcError>>;

/// OpenCode's version check, before the serializing lock: a slow `--version`
/// delays this pass, never another task's changes.
async fn check_opencode_versions(core: &Core, pass: &Pass<'_>) -> VersionChecks {
    let mut checks = VersionChecks::new();
    if pass.config_error.is_some() {
        return checks;
    }
    let file = core.tasks.workers();
    let now = now_ms();
    // Only a worker a queued task could start this pass is worth checking: a
    // blocked one is not picked, and its turn must not wait on a version.
    let mut wanted = BTreeSet::new();
    for task in pass.tasks.iter().filter(|task| task.state == TaskState::Queued) {
        let chain: Vec<Worker> = task.chain.iter().filter_map(|key| workers::find(&file, key).ok()).collect();
        let chain = if chain.is_empty() { workers::chain_for(&file, "auto").unwrap_or_default() } else { chain };
        for worker in chain {
            let blocked = file.blocks.get(worker.key()).is_some_and(|block| block.until_ms > now);
            if worker.cli == Cli::Opencode && !blocked {
                wanted.insert(worker.key().to_string());
            }
        }
    }
    for key in wanted {
        let Ok(worker) = workers::find(&file, &key) else { continue };
        let Ok(prefix) = pass.prefix(&worker) else { continue };
        checks.insert(key, worker::check_version(&prefix).await);
    }
    checks
}

/// One pass: end what ended, cut what ran too long, start what may start.
pub async fn pump(core: Arc<Core>) {
    // Load the pass's config and preflight a planned snapshot before the
    // serializing lock; the pass itself reads the tasks once it holds it.
    let planning = core.tasks.all();
    let loaded = if planning.iter().any(|task| matches!(task.state, TaskState::Queued | TaskState::Stranded)) {
        crate::config::load(&core.paths.config)
    } else {
        Ok(crate::config::Config::default())
    };
    let pass = Pass::new(&planning, loaded);
    let checks = if planning.iter().any(|task| task.state == TaskState::Queued) {
        check_opencode_versions(&core, &pass).await
    } else {
        VersionChecks::new()
    };
    let _serial = core.tasks.lock.lock().await;
    if stopping(&core) {
        return;
    }
    let tasks = core.tasks.all();
    let pass = pass.rebind(&tasks);
    for task in pass.tasks_in(TaskState::Running) {
        pump_running(&core, task).await;
        if stopping(&core) {
            return;
        }
    }
    for task in pass.tasks_in(TaskState::Checking) {
        checking::pump_checking(&core, task).await;
        if stopping(&core) {
            return;
        }
    }
    let mut stranded = pass.tasks_in(TaskState::Stranded);
    stranded.sort_by_key(|task| task.seq);
    for task in stranded {
        pump_stranded(&core, task, &pass).await;
        if stopping(&core) {
            return;
        }
    }
    let mut queued = pass.tasks_in(TaskState::Queued);
    queued.sort_by_key(|task| task.seq);
    for task in queued {
        pump_queued(&core, task, &pass, &checks).await;
        if stopping(&core) {
            return;
        }
    }
}

/// A stranded task asks again on every pass: a worker whose block lapsed, or
/// whose block a person cleared, puts it back in the queue.
async fn pump_stranded(core: &Arc<Core>, task: Task, pass: &Pass<'_>) {
    if stopping(core) || pass.config_error.is_some() {
        return;
    }
    let file = core.tasks.workers();
    let installed = |worker: &Worker| pass.installed(worker);
    let chain: Vec<Worker> = task.chain.iter().filter_map(|key| workers::find(&file, key).ok()).collect();
    let chain = if chain.is_empty() { workers::chain_for(&file, "auto").unwrap_or_default() } else { chain };
    let now = now_ms();
    match workers::pick(&chain, &file, installed, now, crate::platform::utc_offset_minutes()) {
        Err(words) if workers::may_come_back(&chain, &file, now) => {
            if task.error.as_deref() != Some(words.as_str()) {
                update(core, &task.name, |task| task.error = Some(words));
                core.changed.notify_waiters();
            }
        }
        // A worker is back, or nothing can come back any more: the queue starts
        // it or fails it, in words.
        _ => {
            update(core, &task.name, |task| {
                task.state = TaskState::Queued;
                task.error = None;
            });
            core.changed.notify_waiters();
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

async fn pump_queued(core: &Arc<Core>, task: Task, pass: &Pass<'_>, checks: &VersionChecks) {
    if stopping(core) {
        return;
    }
    // A rate wait leaves the task, and its waiting sentence, alone until its time.
    if task.rate_wait_until_ms.is_some_and(|until| now_ms() < until) {
        return;
    }
    let others: Vec<&Task> = pass.tasks.iter().filter(|other| other.name != task.name).collect();
    match decide::may_start(&task, &others, pass.config.cap()) {
        Err(waiting) => {
            if task.waiting.as_deref() != Some(waiting.as_str()) {
                update(core, &task.name, |task| task.waiting = Some(waiting));
                core.changed.notify_waiters();
            }
        }
        Ok(()) => {
            if let Some(error) = &pass.config_error {
                return fail_start(core, &task, error.clone()).await;
            }
            let now = now_ms();
            let file = core.tasks.workers();
            let installed = |worker: &Worker| pass.installed(worker);
            let chain: Vec<Worker> = task.chain.iter().filter_map(|key| workers::find(&file, key).ok()).collect();
            let chain = if chain.is_empty() { workers::chain_for(&file, "auto").unwrap_or_default() } else { chain };
            // A follow-up stays with the worker that did the work, while it can.
            let followup =
                task_folder(&core.paths.state, &task.name).join(format!("turn-{}.followup.md", next_turn(&task)));
            let chain = match task.turns.last() {
                Some(last) if followup.is_file() => workers::stay_with(chain, &last.worker),
                _ => chain,
            };
            // A rate wait's next turn goes back to the worker that was refused.
            let chain = if task.rate_wait_until_ms.is_some() { workers::stay_with(chain, &task.worker) } else { chain };
            let worker = match workers::pick(&chain, &file, installed, now, crate::platform::utc_offset_minutes()) {
                Ok(worker) => worker,
                Err(words) if workers::may_come_back(&chain, &file, now) => {
                    // A block lapses, or a person clears it: the task waits, and
                    // goes on by itself when a worker is back.
                    update(core, &task.name, |task| {
                        task.state = TaskState::Stranded;
                        task.error = Some(words);
                        task.waiting = None;
                    });
                    core.changed.notify_waiters();
                    return;
                }
                Err(words) => {
                    let error = RpcError::new("worker_out", words, "see `kom workers`");
                    return fail_start(core, &task, error).await;
                }
            };
            let prefix = match pass.prefix(&worker) {
                Ok(prefix) => prefix,
                Err(error) => return fail_start(core, &task, error).await,
            };
            if worker.cli == Cli::Opencode {
                match checks.get(worker.key()) {
                    Some(Ok(())) => {}
                    Some(Err(error)) => return fail_start(core, &task, error.clone()).await,
                    // A worker added after the pass's check: fail rather than hold the queue.
                    None => {
                        if let Err(error) = worker::check_version(&prefix).await {
                            return fail_start(core, &task, error).await;
                        }
                    }
                }
            }
            let last = *core.tasks.last_start_ms.lock().unwrap_or_else(|p| p.into_inner());
            if let Err(waiting) = decide::start_slot(last, now) {
                if task.waiting.as_deref() != Some(waiting.as_str()) {
                    update(core, &task.name, |task| task.waiting = Some(waiting));
                    core.changed.notify_waiters();
                }
                return;
            }
            match start_task(core, &task, worker, &prefix).await {
                Ok(()) => {
                    *core.tasks.last_start_ms.lock().unwrap_or_else(|p| p.into_inner()) = Some(now_ms());
                    if task.rate_wait_until_ms.is_some() {
                        update(core, &task.name, |task| task.rate_wait_until_ms = None);
                    }
                }
                Err(error) => fail_start(core, &task, error).await,
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

async fn kill_terminal(core: &Arc<Core>, address: Option<&str>) {
    if let Some(id) = address.and_then(crate::terminals::parse_id) {
        core.lock().killing.insert(id);
        core.kill(&[id]).await;
    }
}

async fn kill_turn(core: &Arc<Core>, name: &str, turn: &Turn) {
    kill_terminal(core, turn.terminal.as_deref()).await;
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

/// The number the next turn gets.
fn next_turn(task: &Task) -> u32 {
    u32::try_from(task.turns.len()).unwrap_or(u32::MAX - 1).saturating_add(1)
}

fn unavailable(message: String) -> RpcError {
    RpcError::new("unavailable", message, "check the state folder's permissions and free space")
}

pub fn not_found(name: &str) -> RpcError {
    RpcError::new("not_found", format!("no task {name:?}"), "see `kom task ls --all`")
}

pub fn name_taken(name: &str) -> RpcError {
    RpcError::new(
        "name_taken",
        format!("{name:?} is already a task or a live terminal here"),
        "pick another name; an accepted or unfinished task keeps its name",
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
    names::move_aside(core, &task.name)?;
    let folder = task_folder(&core.paths.state, &task.name);
    std::fs::create_dir_all(&folder).map_err(|e| unavailable(format!("cannot make {}: {e}", folder.display())))?;
    crate::paths::write_atomic(&folder.join("brief.md"), brief.as_bytes())
        .map_err(|e| unavailable(format!("cannot write {}: {e}", folder.join("brief.md").display())))?;
    crate::task::parts::write_briefs(&folder, &task.parts)?;
    save(&core.paths.state, &task);
    core.tasks.tasks.lock().unwrap_or_else(|p| p.into_inner()).insert(task.name.clone(), task);
    drop(serial);
    pump(core.clone()).await;
    Ok(())
}

/// Cancel a queued task, or kill a running one and finish its turn as cancelled.
/// Returns its state and how many processes survived the kill.
pub async fn cancel(core: &Arc<Core>, name: &str, by: Option<&str>, yes: bool) -> Result<(TaskState, u32), RpcError> {
    let _serial = core.tasks.lock.lock().await;
    if stopping(core) {
        return Err(unavailable("the core is stopping".into()));
    }
    let Some(task) = core.tasks.snapshot(name) else { return Err(not_found(name)) };
    if let Some(error) = table::change_refusal(&task, by, yes) {
        return Err(error);
    }
    match task.state {
        TaskState::Queued | TaskState::Stranded => {
            let mut task = task;
            task.state = TaskState::Cancelled;
            task.waiting = None;
            task.error = None;
            task.ended_ms = Some(now_ms());
            save(&core.paths.state, &task);
            write_result(&core.paths.state, &task, &worker_list(core));
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
        TaskState::Checking => {
            if let Some(check) = task.check.as_ref() {
                kill_terminal(core, check.terminal.as_deref()).await;
            }
            let mut task = task;
            task.state = TaskState::Cancelled;
            task.ended_ms = Some(now_ms());
            task.check = None;
            save(&core.paths.state, &task);
            write_result(&core.paths.state, &task, &worker_list(core));
            replace(core, task);
            core.changed.notify_waiters();
            Ok((TaskState::Cancelled, 0))
        }
        other => {
            Err(RpcError::new("finished", format!("{name} has already ended: {}", other.word()), "see `kom task show`"))
        }
    }
}

/// End a rate wait now: the worker that was refused runs its turn again at once.
pub async fn retry(core: &Arc<Core>, name: &str) -> Result<Task, RpcError> {
    let serial = core.tasks.lock.lock().await;
    if stopping(core) {
        return Err(unavailable("the core is stopping".into()));
    }
    let Some(mut task) = core.tasks.snapshot(name) else { return Err(not_found(name)) };
    if task.state != TaskState::Queued || task.rate_wait_until_ms.is_none() {
        return Err(RpcError::new("invalid_params", format!("{name} isn't waiting to retry"), "see `kom task show`"));
    }
    // The wait is served, so the next rate refusal counts as the next one.
    task.rate_wait_until_ms = Some(now_ms());
    save(&core.paths.state, &task);
    replace(core, task);
    core.changed.notify_waiters();
    drop(serial);
    pump(core.clone()).await;
    core.tasks.snapshot(name).ok_or_else(|| not_found(name))
}

/// Remove a finished task's worktree and branch, keeping its folder and record.
pub async fn discard(
    core: &Arc<Core>,
    name: &str,
    review: Option<String>,
    by: Option<&str>,
    yes: bool,
) -> Result<(TaskState, bool, Vec<String>), RpcError> {
    let _serial = core.tasks.lock.lock().await;
    if stopping(core) {
        return Err(unavailable("the core is stopping".into()));
    }
    let Some(mut task) = core.tasks.snapshot(name) else { return Err(not_found(name)) };
    if let Some(error) = table::change_refusal(&task, by, yes) {
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
    let first_time = task.state != TaskState::Discarded;
    let removal = if task.shared {
        // The files go back to the workshop, which is never removed: nothing else is cleaned up.
        if let Some(path) = task.worktree.clone().filter(|path| Path::new(path).exists()) {
            let put_back = match workshop::files_of(&path, &task.scope).await {
                Ok(files) => workshop::restore(&path, &files).await,
                Err(e) => Err(e),
            };
            if let Err(e) = put_back {
                return Err(RpcError::new(
                    "unavailable",
                    format!("{name}'s files could not be put back in the workshop: {}", e.message),
                    format!("try again: kom task discard {name}"),
                ));
            }
        }
        git::Removal { removed: true, left: Vec::new() }
    } else {
        git::remove_worktree(&task.repo, task.worktree.as_deref(), Some(&task.task_branch)).await
    };
    if removal.removed {
        task.worktree = None;
    }
    task.state = TaskState::Discarded;
    if review.is_some() {
        task.review = review;
    }
    if first_time && let Err(words) = ledger::record(&core.paths.state, &task, "discarded", now_ms()) {
        task.warnings.push(words);
    }
    save(&core.paths.state, &task);
    write_result(&core.paths.state, &task, &worker_list(core));
    replace(core, task);
    core.changed.notify_waiters();
    Ok((TaskState::Discarded, removal.removed, removal.left))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pass_answers_each_worker_prefix_once() {
        let config = crate::config::Config {
            workers: Some(crate::config::WorkersConfig {
                opencode: Some(vec![std::env::current_exe().unwrap().display().to_string()]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let pass = Pass::new(&[], Ok(config));
        let worker = Worker { key: "go".into(), label: "go".into(), cli: Cli::Opencode, route: "r".into() };
        assert!(pass.installed(&worker));
        assert_eq!(pass.prefix_calls.load(Ordering::SeqCst), 1);
        assert!(pass.installed(&worker));
        assert_eq!(pass.prefix_calls.load(Ordering::SeqCst), 1, "the second answer comes from the pass's cache");
    }

    #[test]
    fn a_pass_selects_each_state_from_its_one_snapshot() {
        let tasks = vec![
            Task { name: "q".into(), state: TaskState::Queued, ..Default::default() },
            Task { name: "r".into(), state: TaskState::Running, ..Default::default() },
        ];
        let pass = Pass::new(&tasks, Ok(crate::config::Config::default()));
        let names = |state| pass.tasks_in(state).into_iter().map(|task| task.name).collect::<Vec<_>>();
        assert_eq!(names(TaskState::Queued), ["q"]);
        assert_eq!(names(TaskState::Running), ["r"]);
        assert!(names(TaskState::Stranded).is_empty());
    }

    #[tokio::test]
    async fn a_panic_reading_a_turn_is_reported_as_the_core_failing_to_read_it() {
        let error = match tokio::task::spawn_blocking(|| panic!("the record vanished mid-read")).await {
            Err(error) => error,
            Ok(_) => panic!("the read was expected to panic"),
        };
        let words = super::turn_end::unreadable_turn(error);
        assert_eq!(words, "the core could not read this turn's output: the record vanished mid-read");
        assert!(
            !words.contains("worker produced no final text"),
            "the worker was blamed for the core's own failure: {words}"
        );
    }
}
