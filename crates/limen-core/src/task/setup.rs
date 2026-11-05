//! What surrounds a worker's turn: its environment, the commit guard, denied tools and Python.

use crate::rpc::RpcError;
use crate::runtime::Core;
use crate::task::types::Task;
use crate::task::worker::Worker;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

/// What a turn gets from `prepare`.
#[derive(Debug, Default)]
pub struct Prepared {
    /// Set in the turn's terminal, on top of what the turn's command sets.
    pub env: BTreeMap<String, String>,
    /// A folder the runner puts first on the worker's PATH.
    pub path_first: Option<String>,
    /// Worth a line each in the task's warnings.
    pub warnings: Vec<String>,
}

/// Everything a turn needs around it, made just before its terminal starts.
pub async fn prepare(
    _core: &Arc<Core>,
    _task: &Task,
    _worker: Worker,
    _worktree: &str,
    _n: u32,
) -> Result<Prepared, RpcError> {
    Ok(Prepared::default())
}

/// Puts back what `prepare` put in the worktree, however the turn ended and after a core restart too.
pub fn after_turn(_folder: &Path, _worktree: &str, _worker: Worker, _n: u32) -> Result<(), String> {
    Ok(())
}
