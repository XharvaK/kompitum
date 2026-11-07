//! `lc task review`: the whole review of a task in one read.

use crate::api::{TaskReviewParams, TaskReviewResult};
use crate::rpc::RpcError;
use crate::runtime::Core;
use std::sync::Arc;

pub async fn review(_core: &Arc<Core>, _p: TaskReviewParams) -> Result<TaskReviewResult, RpcError> {
    Err(super::unavailable("task.review isn't built yet".into()))
}
