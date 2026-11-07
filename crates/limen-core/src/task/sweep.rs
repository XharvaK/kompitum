//! What old tasks leave behind goes; their ledger rows stay.

use crate::api::{TaskSweepParams, TaskSweepResult};
use crate::rpc::RpcError;
use crate::runtime::{Core, now_ms};
use crate::task::types::TaskState;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Days a task keeps its files when `task.sweep` is not told otherwise.
pub const DEFAULT_DAYS: u64 = 14;
const DAY_MS: u64 = 86_400_000;

pub async fn sweep(core: &Arc<Core>, p: TaskSweepParams) -> Result<TaskSweepResult, RpcError> {
    let days = p.days.unwrap_or(DEFAULT_DAYS);
    let age = days.saturating_mul(DAY_MS);
    let now = now_ms();
    let mut result = TaskSweepResult {
        removed: Vec::new(),
        discarded: Vec::new(),
        freed_bytes: 0,
        left: Vec::new(),
        dry_run: p.dry_run,
    };
    for task in core.tasks.all() {
        let past = now.saturating_sub(task.ended_ms.unwrap_or(task.created_ms)) >= age;
        match task.state {
            TaskState::Accepted | TaskState::Discarded => {
                if !p.dry_run
                    && let Some(worktree) = task.worktree.clone()
                {
                    let removal =
                        crate::task::git::remove_worktree(&task.repo, Some(&worktree), Some(&task.task_branch)).await;
                    if removal.removed {
                        forget_worktree(core, &task.name).await;
                    }
                    result.left.extend(removal.left);
                }
                if past {
                    result.removed.push(task.name.clone());
                    if p.dry_run {
                        let state = core.paths.state.clone();
                        let name = task.name.clone();
                        let bytes =
                            tokio::task::spawn_blocking(move || count_files(&state, &name)).await.unwrap_or_default();
                        result.freed_bytes = result.freed_bytes.saturating_add(bytes);
                    } else {
                        remove_record(core, &task.name).await;
                        let state = core.paths.state.clone();
                        let name = task.name.clone();
                        let (bytes, left) =
                            tokio::task::spawn_blocking(move || remove_files(&state, &name)).await.unwrap_or_default();
                        result.freed_bytes = result.freed_bytes.saturating_add(bytes);
                        result.left.extend(left);
                    }
                }
            }
            TaskState::Failed | TaskState::TimedOut | TaskState::Cancelled => {
                if !past {
                    continue;
                }
                if p.dry_run {
                    result.discarded.push(task.name.clone());
                } else {
                    let why = nobody_came(days);
                    match crate::task::discard(core, &task.name, Some(why)).await {
                        Ok(_) => result.discarded.push(task.name.clone()),
                        Err(error) => result.left.push(format!("{}: {}", task.name, error.message)),
                    }
                }
            }
            _ => {}
        }
    }
    // A folder under worktrees no task owns is named, never removed: it may be a person's.
    let state = core.paths.state.clone();
    let owned: Vec<PathBuf> =
        core.tasks.all().iter().filter_map(|task| task.worktree.as_deref().map(PathBuf::from)).collect();
    let left = tokio::task::spawn_blocking(move || unowned(&state, &owned)).await.unwrap_or_default();
    result.left.extend(left);
    result.left.sort();
    result.left.dedup();
    Ok(result)
}

/// The sweep the core runs by itself: at its start, then once a day.
pub async fn when_due(core: &Arc<Core>, last: &mut Option<u64>) {
    let now = now_ms();
    if !due(*last, now) {
        return;
    }
    *last = Some(now);
    let core = core.clone();
    // Deleting a big folder takes long; it never holds the pump.
    tokio::spawn(async move {
        match sweep(&core, TaskSweepParams { days: None, dry_run: false }).await {
            Ok(result) => {
                for word in result.left {
                    eprintln!("core: sweep: {word}");
                }
            }
            // The store's `problems` is read-only behind the shared core, so a background
            // failure is said the way the core's other background failures are.
            Err(error) => eprintln!("core: sweep: {}", error.message),
        }
    });
}

/// True at the core's start, and after a day has passed since the last sweep.
pub fn due(last: Option<u64>, now_ms: u64) -> bool {
    last.is_none_or(|last| now_ms.saturating_sub(last) >= DAY_MS)
}

async fn remove_record(core: &Arc<Core>, name: &str) {
    let _serial = core.tasks.lock.lock().await;
    core.tasks.tasks.lock().unwrap_or_else(|p| p.into_inner()).remove(name);
}

/// The worktree went, so the record says so; a later sweep does not try it again.
async fn forget_worktree(core: &Arc<Core>, name: &str) {
    let _serial = core.tasks.lock.lock().await;
    super::update(core, name, |task| task.worktree = None);
}

fn nobody_came(days: u64) -> String {
    if days == 1 { "nobody came for it after 1 day".into() } else { format!("nobody came for it after {days} days") }
}

fn count_files(state: &Path, name: &str) -> u64 {
    let mut total = folder_bytes(&super::task_folder(state, name));
    for path in patch_files(state, name) {
        total = total.saturating_add(file_bytes(&path));
    }
    total
}

fn remove_files(state: &Path, name: &str) -> (u64, Vec<String>) {
    let mut total = 0u64;
    let mut left = Vec::new();
    let folder = super::task_folder(state, name);
    total = total.saturating_add(folder_bytes(&folder));
    if let Err(e) = std::fs::remove_dir_all(&folder)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        left.push(format!("folder {}: {e}", folder.display()));
    }
    for path in patch_files(state, name) {
        total = total.saturating_add(file_bytes(&path));
        if let Err(e) = std::fs::remove_file(&path) {
            left.push(format!("patch {}: {e}", path.display()));
        }
    }
    (total, left)
}

/// `<state>/patches/<task>.patch` and `<task>.diff`, the only names a task has there.
fn patch_files(state: &Path, name: &str) -> Vec<PathBuf> {
    let folder = state.join("patches");
    [format!("{name}.patch"), format!("{name}.diff")]
        .into_iter()
        .map(|file| folder.join(file))
        .filter(|path| path.is_file())
        .collect()
}

fn file_bytes(path: &Path) -> u64 {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => folder_bytes(path),
        Ok(meta) => meta.len(),
        Err(_) => 0,
    }
}

fn folder_bytes(path: &Path) -> u64 {
    let mut total = 0u64;
    let Ok(entries) = std::fs::read_dir(path) else { return 0 };
    for entry in entries.flatten() {
        total = total.saturating_add(file_bytes(&entry.path()));
    }
    total
}

fn unowned(state: &Path, owned: &[PathBuf]) -> Vec<String> {
    let folder = state.join("worktrees");
    let Ok(entries) = std::fs::read_dir(&folder) else { return Vec::new() };
    let mut left = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if !owned.iter().any(|owned| same_path(owned, &path)) {
            left.push(format!("worktree folder {}: no task owns it", path.display()));
        }
    }
    left
}

fn same_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sweep_is_due_at_the_start_and_then_once_a_day() {
        assert!(due(None, 0), "there was no sweep yet");
        assert!(!due(Some(1_000), 1_000 + DAY_MS - 1), "23 hours is not a day");
        assert!(due(Some(1_000), 1_000 + DAY_MS), "a day has passed");
        assert!(!due(Some(5_000), 1_000), "a clock that went back is not due");
    }

    #[test]
    fn a_patch_named_for_a_longer_task_stays() {
        let dir = std::env::temp_dir().join(format!("lc-sweep-patches-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let patches = dir.join("patches");
        std::fs::create_dir_all(&patches).unwrap();
        std::fs::write(patches.join("a.patch"), "one").unwrap();
        std::fs::write(patches.join("a.diff"), "two").unwrap();
        std::fs::write(patches.join("a.b.patch"), "another task's").unwrap();

        let (_, left) = remove_files(&dir, "a");
        assert!(left.is_empty(), "{left:?}");
        assert!(!patches.join("a.patch").exists());
        assert!(!patches.join("a.diff").exists());
        assert!(patches.join("a.b.patch").is_file(), "a.b.patch belongs to a.b, not a");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_sentence_says_how_long_nobody_came() {
        assert_eq!(nobody_came(1), "nobody came for it after 1 day");
        assert_eq!(nobody_came(14), "nobody came for it after 14 days");
    }
}
