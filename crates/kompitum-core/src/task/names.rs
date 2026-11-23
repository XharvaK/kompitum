//! The name a discarded task's record moves to, and the move itself.

use crate::rpc::RpcError;
use crate::runtime::Core;
use crate::task::types::TaskState;
use std::sync::Arc;

/// The first `<name>.N`, from 1 up, that `taken` says is free.
pub fn moved_name(name: &str, taken: impl Fn(&str) -> bool) -> String {
    let mut n = 1u64;
    loop {
        let candidate = format!("{name}.{n}");
        if !taken(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// Move a discarded record's folder and name aside, to `<name>.N`, so its name
/// can start a new task. A record that is not discarded keeps its name.
pub fn move_aside(core: &Arc<Core>, name: &str) -> Result<(), RpcError> {
    let Some(mut task) = core.tasks.snapshot(name) else { return Ok(()) };
    if task.state != TaskState::Discarded {
        return Ok(());
    }
    let state = &core.paths.state;
    let taken = |candidate: &str| {
        core.tasks.snapshot(candidate).is_some()
            || super::task_folder(state, candidate).exists()
            || core.lock().reg.terminals.values().any(|t| t.is_live() && t.name.as_deref() == Some(candidate))
    };
    let moved = moved_name(name, taken);
    let from = super::task_folder(state, name);
    let to = super::task_folder(state, &moved);
    std::fs::rename(&from, &to)
        .map_err(|e| super::unavailable(format!("cannot move {} aside to {}: {e}", from.display(), to.display())))?;
    task.name = moved;
    super::save(state, &task);
    core.tasks.tasks.lock().unwrap_or_else(|p| p.into_inner()).remove(name);
    super::replace(core, task);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_free_suffix_is_chosen() {
        let taken = |candidate: &str| matches!(candidate, "fix.1" | "fix.3");
        assert_eq!(moved_name("fix", taken), "fix.2");
    }

    #[test]
    fn a_moved_name_may_pass_the_limit_by_its_suffix() {
        let long = "a".repeat(32);
        assert_eq!(moved_name(&long, |_| false), format!("{long}.1"));
    }
}
