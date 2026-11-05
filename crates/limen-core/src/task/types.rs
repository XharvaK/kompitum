//! A task's record, the one-line wake it produces, and the runner's beat.
//! Pure data: `task.rs` writes it and `decide.rs` judges it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    #[default]
    Queued,
    Running,
    /// It wants a worker and every one of its chain is out.
    Stranded,
    Done,
    Failed,
    TimedOut,
    Cancelled,
    Discarded,
}

impl TaskState {
    /// No longer running: queued and stranded are the states that haven't ended.
    pub fn ended(self) -> bool {
        !matches!(self, TaskState::Queued | TaskState::Running | TaskState::Stranded)
    }

    /// An end nobody may accept: a failure, a cut-off, or a cancel.
    pub fn ended_badly(self) -> bool {
        matches!(self, TaskState::Failed | TaskState::TimedOut | TaskState::Cancelled)
    }

    pub fn word(self) -> &'static str {
        match self {
            TaskState::Queued => "queued",
            TaskState::Running => "running",
            TaskState::Stranded => "stranded",
            TaskState::Done => "done",
            TaskState::Failed => "failed",
            TaskState::TimedOut => "timed_out",
            TaskState::Cancelled => "cancelled",
            TaskState::Discarded => "discarded",
        }
    }
}

/// Why the loop is ending the current turn.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Ending {
    Cancel,
    Idle,
    Wall,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Usage {
    pub steps: u64,
    pub input: u64,
    pub output: u64,
    pub reasoning: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cost_usd: f64,
    pub largest_request: u64,
}

impl Usage {
    pub fn add(&mut self, other: &Usage) {
        self.steps += other.steps;
        self.input += other.input;
        self.output += other.output;
        self.reasoning += other.reasoning;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
        self.cost_usd += other.cost_usd;
        self.largest_request = self.largest_request.max(other.largest_request);
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct ChangedFile {
    pub path: String,
    pub added: u64,
    pub removed: u64,
    pub in_scope: bool,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Turn {
    pub n: u32,
    /// The worker key (`go`, `free`).
    pub worker: String,
    pub terminal: Option<String>,
    /// The terminal's `LIMEN_SCOPE` token: everything the turn started carries it.
    pub scope_token: Option<String>,
    pub started_ms: u64,
    pub ended_ms: Option<u64>,
    pub exit_code: Option<i32>,
    /// The session the worker reported, once it did.
    pub session: Option<String>,
    /// The session this turn was started to continue; None starts fresh.
    #[serde(default)]
    pub resumes: Option<String>,
    /// The window of the refusal that ended this turn, kept for the hand-off.
    #[serde(default)]
    pub refused: Option<String>,
    /// The worker exited with an error before it began; it was started again.
    #[serde(default)]
    pub died_at_start: bool,
    /// The last report it wrote, whole; the wake cuts it.
    pub text: String,
    pub usage: Usage,
    /// Processes carrying the scope token that outlived the kill.
    pub survivors: u32,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Task {
    pub name: String,
    /// The queue's order: lower starts first.
    pub seq: u64,
    pub state: TaskState,
    /// git's toplevel.
    pub repo: String,
    /// The branch it was cut from, "HEAD" when detached.
    pub branch: String,
    /// The commit it was cut from.
    pub base: String,
    pub task_branch: String,
    pub worktree: Option<String>,
    /// Empty: the whole repo.
    pub scope: Vec<String>,
    /// The key of the current or last turn's worker, empty until one is picked.
    pub worker: String,
    /// The worker keys `task.start` fixed, in the order to try them.
    pub chain: Vec<String>,
    /// One sentence per hand-off, in the order they happened.
    #[serde(default)]
    pub handoffs: Vec<String>,
    pub idle_s: u64,
    pub wall_s: u64,
    /// The caller's terminal.
    pub started_by: Option<String>,
    pub warnings: Vec<String>,
    /// Why it is still queued, in words.
    pub waiting: Option<String>,
    pub turns: Vec<Turn>,
    /// Why the loop is ending the current turn.
    pub ending: Option<Ending>,
    pub error: Option<String>,
    pub changed: Vec<ChangedFile>,
    pub review: Option<String>,
    pub created_ms: u64,
    pub started_ms: Option<u64>,
    pub ended_ms: Option<u64>,
}

/// The wake and the result: one task, small enough to read in one line of tools.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct TaskLine {
    pub task: String,
    pub state: TaskState,
    /// Done, and nothing changed outside the scope.
    pub ok: bool,
    /// The label of the worker that really did the last turn: "opencode Go".
    pub worker: Option<String>,
    /// Every turn's worker key, in order.
    pub workers: Vec<String>,
    /// What each hand-off did, one sentence each.
    #[serde(default)]
    pub handoffs: Vec<String>,
    pub turns: u32,
    pub seconds: f64,
    pub cost_usd: f64,
    pub usage: Usage,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
    pub changed: u32,
    pub out_of_scope: Vec<String>,
    /// The last turn's report; the wait cuts it to 1500 characters.
    pub text: String,
    pub warnings: Vec<String>,
}

impl TaskLine {
    /// The task as a wake, with its report cut to `text_limit` characters.
    pub fn of(task: &Task, now_ms: u64, text_limit: usize) -> TaskLine {
        let mut usage = Usage::default();
        for turn in &task.turns {
            usage.add(&turn.usage);
        }
        let out_of_scope: Vec<String> = task.changed.iter().filter(|f| !f.in_scope).map(|f| f.path.clone()).collect();
        let last = task.turns.last();
        let seconds = task
            .started_ms
            .map(|started| (task.ended_ms.unwrap_or(now_ms).saturating_sub(started)) as f64 / 1000.0)
            .unwrap_or(0.0);
        TaskLine {
            task: task.name.clone(),
            state: task.state,
            ok: task.state == TaskState::Done && out_of_scope.is_empty(),
            worker: last.map(|t| crate::task::worker::label_of(&t.worker)),
            workers: task.turns.iter().map(|t| t.worker.clone()).collect(),
            handoffs: task.handoffs.clone(),
            turns: task.turns.len() as u32,
            seconds,
            cost_usd: usage.cost_usd,
            usage,
            exit_code: last.and_then(|t| t.exit_code),
            error: task.error.clone(),
            changed: task.changed.len() as u32,
            out_of_scope,
            text: cut(last.map(|t| t.text.as_str()).unwrap_or_default(), text_limit),
            warnings: task.warnings.clone(),
        }
    }
}

/// At most `limit` characters (not bytes: a report may hold anything).
pub fn cut(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    text.chars().take(limit).collect()
}

/// The runner's heartbeat, written once a second beside its output.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Beat {
    pub at_ms: u64,
    /// When output or CPU last moved.
    pub active_ms: u64,
    pub cpu_ms: u64,
    pub out_bytes: u64,
    pub steps: u64,
    pub tools: Vec<String>,
    pub cost_usd: f64,
    /// input + cache read + cache write of the largest request.
    pub peak: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(worker: &str, text: &str, cost: f64) -> Turn {
        Turn {
            worker: worker.into(),
            text: text.into(),
            usage: Usage { steps: 1, cost_usd: cost, ..Default::default() },
            exit_code: Some(0),
            ..Default::default()
        }
    }

    #[test]
    fn a_line_sums_its_turns_and_is_ok_only_when_nothing_left_the_scope() {
        let task = Task {
            name: "fix-1".into(),
            state: TaskState::Done,
            started_ms: Some(1_000),
            ended_ms: Some(4_500),
            turns: vec![turn("go", "first", 0.25), turn("free", "second", 0.5)],
            changed: vec![
                ChangedFile { path: "src/a.rs".into(), added: 2, removed: 0, in_scope: true },
                ChangedFile { path: "outside.txt".into(), added: 1, removed: 1, in_scope: false },
            ],
            ..Default::default()
        };
        let line = TaskLine::of(&task, 9_999, 1500);
        assert_eq!(line.seconds, 3.5);
        assert!((line.cost_usd - 0.75).abs() < 1e-9);
        assert_eq!(line.usage.steps, 2);
        assert_eq!(line.worker.as_deref(), Some("opencode free"));
        assert_eq!(line.workers, ["go", "free"]);
        assert_eq!(line.text, "second");
        assert_eq!(line.out_of_scope, ["outside.txt"]);
        assert!(!line.ok, "a file outside the scope is not ok");
        let mut clean = task;
        clean.changed.truncate(1);
        assert!(TaskLine::of(&clean, 9_999, 1500).ok);
    }

    #[test]
    fn a_running_task_counts_to_now_and_a_report_is_cut_by_characters() {
        let task = Task {
            name: "fix-2".into(),
            state: TaskState::Running,
            started_ms: Some(1_000),
            turns: vec![turn("go", "éééé", 0.0)],
            ..Default::default()
        };
        let line = TaskLine::of(&task, 3_000, 3);
        assert_eq!(line.seconds, 2.0);
        assert_eq!(line.text, "ééé");
        assert!(!line.ok);
        assert_eq!(cut("ab", 3), "ab");
    }

    #[test]
    fn state_words_are_what_the_api_and_files_use() {
        assert_eq!(TaskState::TimedOut.word(), "timed_out");
        assert_eq!(TaskState::Stranded.word(), "stranded");
        assert!(!TaskState::Stranded.ended() && !TaskState::Stranded.ended_badly());
        assert!(TaskState::Cancelled.ended_badly() && TaskState::Done.ended() && !TaskState::Running.ended());
    }
}
