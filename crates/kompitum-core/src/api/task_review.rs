//! `task.review`, `task.accept`, `task.redo`, `task.sweep`, `task.stats` and `ledger.show`:
//! their params and results, and handlers that only call `crate::task`.

use super::*;
use crate::task::parts::PartReview;
use crate::task::stats::{SessionStats, StatsTotals};
use crate::task::types::{LedgerRow, ParentUsage, TaskState, Usage};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, JsonSchema)]
pub struct TaskReviewParams {
    /// The task's name.
    pub task: String,
    /// Add the whole patch; one that does not fit goes to a file the review names.
    #[serde(default)]
    pub full: bool,
    /// The review's size in bytes (default 20000).
    #[serde(default)]
    pub max_bytes: Option<u64>,
}

#[derive(Serialize, JsonSchema)]
pub struct TaskReviewResult {
    /// The task's name.
    pub task: String,
    /// Its state now.
    pub state: TaskState,
    /// The review, in Markdown.
    pub text: String,
    /// Where the whole patch went, when it was asked for and did not fit.
    pub patch_file: Option<String>,
    /// The review was cut at `max_bytes`.
    pub truncated: bool,
    /// Each part's state, files and require results; empty for an ordinary task.
    pub parts: Vec<PartReview>,
}

#[derive(Deserialize, JsonSchema)]
pub struct TaskAcceptParams {
    /// The task's name.
    pub task: String,
    /// Accept only this part, committing only its files and keeping its worktree.
    #[serde(default)]
    pub part: Option<String>,
    /// What the review decided, kept for the ledger.
    #[serde(default)]
    pub review: Option<String>,
    /// The commit message (default: the review, else a line naming the task).
    #[serde(default)]
    pub message: Option<String>,
    /// Put the changes in the working tree and make no commit.
    #[serde(default)]
    pub no_commit: bool,
    /// Accept a task that failed, timed out or was cancelled, or whose last checks failed.
    #[serde(default)]
    pub accept_unfinished: bool,
    /// Accept files changed outside the task's scope.
    #[serde(default)]
    pub accept_scope_violation: bool,
    /// Who is asking: opus, manager or owner (default: owner).
    #[serde(default)]
    pub by: Option<String>,
    /// The Owner's words that let a manager accept; kept in the record and the ledger row.
    #[serde(default)]
    pub owner_said: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct TaskAcceptResult {
    /// The task's name.
    pub task: String,
    /// Its state now: accepted.
    pub state: TaskState,
    /// The commit made, or null with `no_commit`.
    pub commit: Option<String>,
    /// The files that came in.
    pub files: Vec<String>,
    /// The patch, kept until the sweep takes it.
    pub patch: String,
    /// Whether its worktree and branch were removed.
    pub removed: bool,
    /// What could not be removed, in words.
    pub left: Vec<String>,
    /// What the accept did in its own words, like a part with no changes.
    pub note: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct TaskRedoParams {
    /// The task's name.
    pub task: String,
    /// What to do now, at most 200000 characters.
    pub brief: String,
    /// Who is asking: opus, manager or owner (default: owner).
    #[serde(default)]
    pub by: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct TaskRedoResult {
    /// The task's name.
    pub task: String,
    /// Its state right after the pump looked at it.
    pub state: TaskState,
    /// The number of the turn the follow-up goes to.
    pub turn: u32,
    /// Why it is still queued, in words.
    pub waiting: Option<String>,
    /// The command that waits for it.
    pub next: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct TaskSweepParams {
    /// Days after its end that a task keeps its files (default 14).
    #[serde(default)]
    pub days: Option<u64>,
    /// Say what would go, and remove nothing.
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Serialize, JsonSchema)]
pub struct TaskSweepResult {
    /// Accepted or discarded tasks whose folder and record went; their ledger rows stay.
    pub removed: Vec<String>,
    /// Failed, timed-out or cancelled tasks nobody came for: discarded, worktree and all.
    pub discarded: Vec<String>,
    /// The bytes those folders held.
    pub freed_bytes: u64,
    /// What could not be removed, and worktree folders no task owns, in words.
    pub left: Vec<String>,
    /// Nothing was removed: this says what would go.
    pub dry_run: bool,
}

#[derive(Deserialize, JsonSchema)]
pub struct TaskStatsParams {
    /// The task's name.
    #[serde(default)]
    pub task: Option<String>,
    /// An opencode session to read instead of a task.
    #[serde(default)]
    pub opencode: Option<String>,
    /// A Command Code session id or the path to its transcript.
    #[serde(default)]
    pub commandcode: Option<String>,
    /// The folder to run `opencode session export` in, or to resolve a transcript path (default: where kom runs).
    #[serde(default)]
    pub folder: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct TaskStatsResult {
    /// The task's name or the session id.
    pub subject: String,
    /// One entry per turn or session, then the subagent sessions.
    pub sessions: Vec<SessionStats>,
    /// What every entry adds up to.
    pub totals: StatsTotals,
    /// The brief's jobs: lines that start `## Job ` or `### Job `; one when none.
    pub jobs: Option<u64>,
    /// Additions plus deletions in the task's patch; null when unknown.
    pub changed_lines: Option<u64>,
    /// Resent tokens per job; null when there are none.
    pub resent_per_job: Option<u64>,
    /// Resent tokens per changed line; null when there are none or unknown.
    pub resent_per_changed_line: Option<u64>,
    /// Words for each file or export that could not be read.
    pub problems: Vec<String>,
    /// The numbers as a Markdown table.
    pub table: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct LedgerParams {
    /// Only this task's row, with the orchestrator's own usage for it.
    #[serde(default)]
    pub task: Option<String>,
    /// The newest rows to show (default 20).
    #[serde(default)]
    pub last: Option<usize>,
}

#[derive(Serialize, JsonSchema)]
pub struct LedgerResult {
    /// The rows, oldest first.
    pub rows: Vec<LedgerRow>,
    /// The rows as a Markdown table, with their totals and any parent usage.
    pub table: String,
    /// What the rows add up to.
    pub totals: Usage,
    /// The orchestrator's own requests for `task`, read from its transcript.
    pub parent: Option<ParentUsage>,
    /// Lines of the ledger that could not be read.
    pub problems: Vec<String>,
}

pub(super) async fn task_review(core: Ctx<'_>, p: TaskReviewParams) -> Result<TaskReviewResult, RpcError> {
    crate::task::review::review(core.core, p).await
}

pub(super) async fn task_accept(core: Ctx<'_>, p: TaskAcceptParams) -> Result<TaskAcceptResult, RpcError> {
    crate::task::accept::accept(core.core, p).await
}

pub(super) async fn task_redo(core: Ctx<'_>, p: TaskRedoParams) -> Result<TaskRedoResult, RpcError> {
    crate::task::accept::redo(core.core, p).await
}

pub(super) async fn task_sweep(core: Ctx<'_>, p: TaskSweepParams) -> Result<TaskSweepResult, RpcError> {
    crate::task::sweep::sweep(core.core, p).await
}

pub(super) async fn task_stats(core: Ctx<'_>, p: TaskStatsParams) -> Result<TaskStatsResult, RpcError> {
    crate::task::stats::stats(core.core, p).await
}

pub(super) async fn ledger_show(core: Ctx<'_>, p: LedgerParams) -> Result<LedgerResult, RpcError> {
    crate::task::ledger::show(core.core, p).await
}
