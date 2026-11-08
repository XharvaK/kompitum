//! The worker loop, part A: a task is a record on top of a contained terminal.
//! One pump changes tasks one at a time; the terminal runs the runner, which
//! runs the worker, and the process's exit is what ends a turn.

pub mod accept;
pub mod brief;
pub mod checking;
pub mod checks;
pub mod clock;
pub mod commandcode;
pub mod decide;
pub mod git;
pub mod handoff;
pub mod ledger;
pub mod opencode;
pub mod parent_usage;
pub mod prompt;
pub mod quota;
pub mod review;
pub mod runner;
pub mod setup;
pub mod sweep;
pub mod table;
pub mod types;
pub mod updates;
pub mod worker;
pub mod workers;

use crate::api::{Label, RunParams};
use crate::rpc::RpcError;
use crate::runtime::{Core, now_ms};
use crate::task::decide::{Step, TerminalNow};
use crate::task::opencode::RouteCheck;
use crate::task::types::{Beat, ChangedFile, Ending, Task, TaskLine, TaskState, Turn};
use crate::task::worker::{Cli, TurnOutcome, Worker};
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
    let checking: Vec<Task> = core.tasks.all().into_iter().filter(|task| task.state == TaskState::Checking).collect();
    for task in checking {
        checking::pump_checking(&core, task).await;
        if stopping(&core) {
            return;
        }
    }
    let mut stranded: Vec<Task> =
        core.tasks.all().into_iter().filter(|task| task.state == TaskState::Stranded).collect();
    stranded.sort_by_key(|task| task.seq);
    for task in stranded {
        pump_stranded(&core, task).await;
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

/// A stranded task asks again on every pass: a worker whose block lapsed, or
/// whose block a person cleared, puts it back in the queue.
async fn pump_stranded(core: &Arc<Core>, task: Task) {
    if stopping(core) {
        return;
    }
    let Ok(config) = crate::config::load(&core.paths.config) else { return };
    let file = core.tasks.workers();
    let installed = |worker: &Worker| worker::prefix(worker, &config).is_ok();
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
            let now = now_ms();
            let config = match crate::config::load(&core.paths.config) {
                Ok(config) => config,
                Err(error) => return fail_start(core, &task, error).await,
            };
            let file = core.tasks.workers();
            let installed = |worker: &Worker| worker::prefix(worker, &config).is_ok();
            let chain: Vec<Worker> = task.chain.iter().filter_map(|key| workers::find(&file, key).ok()).collect();
            let chain = if chain.is_empty() { workers::chain_for(&file, "auto").unwrap_or_default() } else { chain };
            // A follow-up stays with the worker that did the work, while it can.
            let followup =
                task_folder(&core.paths.state, &task.name).join(format!("turn-{}.followup.md", next_turn(&task)));
            let chain = match task.turns.last() {
                Some(last) if followup.is_file() => workers::stay_with(chain, &last.worker),
                _ => chain,
            };
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
            let last = *core.tasks.last_start_ms.lock().unwrap_or_else(|p| p.into_inner());
            if let Err(waiting) = decide::start_slot(last, now) {
                if task.waiting.as_deref() != Some(waiting.as_str()) {
                    update(core, &task.name, |task| task.waiting = Some(waiting));
                    core.changed.notify_waiters();
                }
                return;
            }
            match start_task(core, &task, worker).await {
                Ok(()) => *core.tasks.last_start_ms.lock().unwrap_or_else(|p| p.into_inner()) = Some(now_ms()),
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

/// The last bytes of a turn's stderr, which is where a CLI puts its limits.
fn read_tail(path: &Path, max: usize) -> String {
    let Ok(bytes) = std::fs::read(path) else { return String::new() };
    let start = bytes.len().saturating_sub(max);
    String::from_utf8_lossy(&bytes[start..]).into_owned()
}

/// The turn ended: read what it wrote, judge it, keep the diff and the result.
async fn finish_turn(core: &Arc<Core>, name: &str, n: u32, terminal: Option<&TerminalNow>) {
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
    let earlier: Vec<(Option<String>, f64)> =
        task.turns[..index].iter().map(|turn| (turn.session.clone(), turn.usage.cost_usd)).collect();
    let (outcome, warning, route, stderr_tail) = tokio::task::spawn_blocking(move || {
        let mut outcome = match std::fs::read(&path) {
            Ok(bytes) => TurnOutcome::from_bytes(cli, &bytes),
            Err(_) => TurnOutcome::default(),
        };
        let mut warning = None;
        if cli == Cli::CommandCode {
            match outcome.session.as_deref().and_then(crate::task::commandcode::session_cost) {
                Some(cost) => {
                    // The transcript holds every turn of a resumed session, so only
                    // what this turn added since the earlier ones counts.
                    let same: f64 = earlier
                        .iter()
                        .filter(|(session, _)| session.as_deref() == outcome.session.as_deref())
                        .map(|(_, cost)| *cost)
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
        (outcome, warning, route, read_tail(&err_path, 8 * 1024))
    })
    .await
    .unwrap_or_default();
    let code = terminal.and_then(|terminal| terminal.exit_code).or_else(|| read_exit(&core.paths.state, name, n));
    let ended = now_ms();
    let offset = crate::platform::utc_offset_minutes();
    {
        let turn = &mut task.turns[index];
        turn.session = outcome.session.clone();
        turn.text = outcome.text.clone();
        turn.usage = outcome.usage.clone();
        turn.exit_code = code;
        turn.ended_ms = Some(ended);
    }
    if let Some(warning) = warning {
        task.warnings.push(warning);
    }
    let end =
        decide::turn_end(&task, &all_workers, terminal, code, &outcome, route.as_ref(), &stderr_tail, ended, offset);
    let (done_state, question) = decide::after_turn(end.state, &outcome.text);
    task.turns[index].refused = end.refusal.as_ref().map(|refusal| refusal.window.clone());
    let mut error = end.error;
    if let Some(refusal) = &end.refusal {
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
    let died = end.refusal.is_none() && handoff::died_at_start(&task, terminal, code, &outcome, ended);
    let mut restarting = false;
    if end.refusal.is_some() {
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
    task.state = if restarting { TaskState::Queued } else { done_state };
    task.question = if task.state == TaskState::Asking { question } else { None };
    task.error = if restarting { None } else { error };
    task.waiting = if restarting {
        if end.refusal.is_some() {
            Some(format!("handing off: {refused_words}"))
        } else {
            Some("starting again: it died at its start".into())
        }
    } else {
        None
    };
    task.changed = changed;
    task.ended_ms = if restarting || end.state == TaskState::Done { None } else { Some(ended) };
    save(&core.paths.state, &task);
    if !restarting && end.state != TaskState::Done {
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
async fn handoff_note(
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

fn finish_with_error(core: &Arc<Core>, task: &Task, message: &str) {
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

async fn fail_start(core: &Arc<Core>, task: &Task, error: RpcError) {
    if stopping(core) {
        return;
    }
    let mut task = core.tasks.snapshot(&task.name).unwrap_or_else(|| task.clone());
    // A later turn's failed start keeps the first worker's work.
    if task.turns.is_empty()
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

/// The number the next turn gets.
fn next_turn(task: &Task) -> u32 {
    u32::try_from(task.turns.len()).unwrap_or(u32::MAX - 1).saturating_add(1)
}

/// A turn's note or follow-up, when it has one.
fn read_turn_file(folder: &Path, n: u32, kind: &str) -> Result<Option<Vec<u8>>, RpcError> {
    let path = folder.join(format!("turn-{n}.{kind}.md"));
    match std::fs::read(&path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(unavailable(format!("cannot read {}: {e}", path.display()))),
    }
}

async fn start_task(core: &Arc<Core>, task: &Task, worker: Worker) -> Result<(), RpcError> {
    let mut task = task.clone();
    let n = next_turn(&task);
    let all_workers = worker_list(core);
    let config = crate::config::load(&core.paths.config)?;
    let prefix = worker::prefix(&worker, &config)?;
    // The version check is OpenCode's alone.
    if worker.cli != Cli::CommandCode {
        worker::check_version(&prefix).await?;
    }
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
    let prompt_path = folder.join(format!("turn-{n}.prompt.md"));
    crate::paths::write_atomic(&prompt_path, &built.text)
        .map_err(|e| unavailable(format!("cannot write {}: {e}", prompt_path.display())))?;
    if task.worktree.is_none() {
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
        worker::turn_command(&worker, &prefix, &task.name, &prompt_path.display().to_string(), resume.as_deref());
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
    if worker.cli != Cli::CommandCode {
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
    let removal = git::remove_worktree(&task.repo, task.worktree.as_deref(), Some(&task.task_branch)).await;
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
