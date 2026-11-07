//! `lc task accept` and `lc task redo`: bring a task's changes in, or send it back.

use crate::api::{TaskAcceptParams, TaskAcceptResult, TaskRedoParams, TaskRedoResult};
use crate::rpc::RpcError;
use crate::runtime::Core;
use std::sync::Arc;

pub async fn accept(_core: &Arc<Core>, _p: TaskAcceptParams) -> Result<TaskAcceptResult, RpcError> {
    Err(super::unavailable("task.accept isn't built yet".into()))
}

pub async fn redo(_core: &Arc<Core>, _p: TaskRedoParams) -> Result<TaskRedoResult, RpcError> {
    Err(super::unavailable("task.redo isn't built yet".into()))
}
