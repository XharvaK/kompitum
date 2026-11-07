//! What old tasks leave behind goes; their ledger rows stay.

use crate::api::{TaskSweepParams, TaskSweepResult};
use crate::rpc::RpcError;
use crate::runtime::Core;
use std::sync::Arc;

pub async fn sweep(_core: &Arc<Core>, _p: TaskSweepParams) -> Result<TaskSweepResult, RpcError> {
    Err(super::unavailable("task.sweep isn't built yet".into()))
}

/// The sweep the core runs by itself: at its start, then once a day.
pub async fn when_due(_core: &Arc<Core>, _last: &mut Option<u64>) {}
