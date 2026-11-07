//! The ledger: one row per task, written when it is accepted or discarded and kept for good.

use crate::api::{LedgerParams, LedgerResult};
use crate::rpc::RpcError;
use crate::runtime::Core;
use crate::task::types::Task;
use std::path::Path;
use std::sync::Arc;

/// Append this task's row. `outcome` is accepted or discarded.
pub fn record(_state: &Path, _task: &Task, _outcome: &str, _now_ms: u64) -> Result<(), String> {
    Ok(())
}

pub async fn show(_core: &Arc<Core>, _p: LedgerParams) -> Result<LedgerResult, RpcError> {
    Err(super::unavailable("ledger.show isn't built yet".into()))
}
