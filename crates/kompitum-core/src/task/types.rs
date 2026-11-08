//! A task's record, the one-line wake it produces, and the runner's beat.
//! Pure data: `task.rs` writes it and `decide.rs` judges it.

use crate::task::checks::{CHECKS_PATH, Hit, WarnHit};
use crate::task::worker::{Cli, Worker};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    #[default]
    Queued,
    Running,
    // It wants a worker and every one of its chain is out. A plain comment: a doc
    // comment on one variant turns the schema's enum into a `oneOf`.
    Stranded,
    // The worker's turn is over and its checks are running: it still holds its slot and scope.
    Checking,
    Done,
    Failed,
    TimedOut,
    Cancelled,
    Discarded,
    // Its changes are in the repository.
    Accepted,
}

impl TaskState {
    /// Over: queued, running, stranded and checking are the states that haven't ended.
    pub fn ended(self) -> bool {
        !matches!(self, TaskState::Queued | TaskState::Running | TaskState::Stranded | TaskState::Checking)
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
            TaskState::Checking => "checking",
            TaskState::Done => "done",
            TaskState::Failed => "failed",
            TaskState::TimedOut => "timed_out",
            TaskState::Cancelled => "cancelled",
            TaskState::Discarded => "discarded",
            TaskState::Accepted => "accepted",
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
        // A damaged ledger line can hold any number; the core's thread must not panic on it.
        self.steps = self.steps.saturating_add(other.steps);
        self.input = self.input.saturating_add(other.input);
        self.output = self.output.saturating_add(other.output);
        self.reasoning = self.reasoning.saturating_add(other.reasoning);
        self.cache_read = self.cache_read.saturating_add(other.cache_read);
        self.cache_write = self.cache_write.saturating_add(other.cache_write);
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
    /// The CLI this turn ran, fixed at its start; null in records from before this.
    #[serde(default)]
    pub cli: Option<Cli>,
    pub terminal: Option<String>,
    /// The terminal's `KOMPITUM_SCOPE` token: everything the turn started carries it.
    pub scope_token: Option<String>,
    pub started_ms: u64,
    pub ended_ms: Option<u64>,
    pub exit_code: Option<i32>,
    /// The session the worker reported, once it did.
    pub session: Option<String>,
    /// The session this turn was started to continue; None starts fresh.
    #[serde(default)]
    pub resumes: Option<String>,
    /// What this turn was for: worker, qa when the checks sent the work back, redo when a person did.
    #[serde(default)]
    pub role: String,
    /// The folder put first on PATH for this turn, so a check finds the same programs.
    #[serde(default)]
    pub path_first: Option<String>,
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

impl Turn {
    /// The CLI the turn ran: recorded at its start, or looked up for an old record.
    pub fn cli_or_lookup(&self, workers: &[Worker]) -> Cli {
        self.cli
            .or_else(|| workers.iter().find(|worker| worker.key == self.worker).map(|worker| worker.cli))
            .unwrap_or(Cli::Opencode)
    }
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
    /// Who started it: opus, manager or owner; null for a record from before this.
    #[serde(default)]
    pub by: Option<String>,
    /// The Owner's words that let a manager accept it.
    #[serde(default)]
    pub owner_said: Option<String>,
    pub warnings: Vec<String>,
    /// Why it is still queued, in words.
    pub waiting: Option<String>,
    pub turns: Vec<Turn>,
    /// Why the loop is ending the current turn.
    pub ending: Option<Ending>,
    pub error: Option<String>,
    pub changed: Vec<ChangedFile>,
    pub review: Option<String>,
    /// What `task.start` asked of the checks and the worker.
    pub asked: Asked,
    /// The repository's committed checks asked for one builder at a time.
    #[serde(default)]
    pub one_at_a_time: bool,
    /// One report per checked turn, in order.
    pub checks: Vec<CheckReport>,
    /// The test a check has running.
    pub check: Option<CheckRun>,
    /// How many times the checks have sent this work back.
    pub qa_used: u32,
    /// What the next follow-up is: qa or redo.
    pub followup_role: Option<String>,
    pub created_ms: u64,
    pub started_ms: Option<u64>,
    pub ended_ms: Option<u64>,
}

/// What `task.start` asked of the checks and the worker, on top of the repository's committed file.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Asked {
    /// The test command, replacing the repository's.
    pub test: Option<String>,
    pub forbid: Vec<String>,
    pub require: Vec<String>,
    pub warn: Vec<String>,
    /// How many times failed checks go back to the worker; null takes the repository's.
    pub qa_rounds: Option<u8>,
    /// The worker may use the web.
    pub web: bool,
}

/// One run of a task's test command.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct TestRun {
    pub cmd: String,
    pub exit_code: Option<i32>,
    pub ok: bool,
    pub timed_out: bool,
    pub seconds: f64,
    /// Its verdict lines.
    pub summary: Vec<String>,
    /// The names that failed.
    pub failed: Vec<String>,
    /// The end of its output.
    pub tail: String,
    /// The file holding its whole output.
    pub log: Option<String>,
    /// Why it wasn't run again: this result is an earlier round's.
    pub reused: Option<String>,
    /// What each changed and untracked file held when this test ran, so a later round can tell
    /// what changed since; null for a file that is gone.
    pub fingerprint: Option<BTreeMap<String, Option<String>>>,
}

/// What the checks found after one turn.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct CheckReport {
    pub turn: u32,
    pub ok: bool,
    pub seconds: f64,
    pub test: Option<TestRun>,
    /// How many forbid entries were checked.
    pub forbid_checked: u32,
    pub forbid_hits: Vec<Hit>,
    /// How many require entries were checked.
    pub require_checked: u32,
    pub require_missing: Vec<String>,
    /// The worker changed the repository's checks file, which fails the task.
    pub checks_file_changed: bool,
    /// Lines flagged for the review; these never fail a task.
    pub warn_hits: Vec<WarnHit>,
    /// What could not be checked, in words.
    pub errors: Vec<String>,
}

/// How many failed test names a report shows.
pub const FAILED_LIMIT: usize = 20;
/// What the review and a QA round say when no report exists.
pub const NO_CHECKS: &str = "No checks ran.";

impl CheckReport {
    /// No test ran and no pattern was asked for. Warns aren't checks: they run on every task.
    pub fn nothing_checked(&self) -> bool {
        self.test.is_none() && self.forbid_checked == 0 && self.require_checked == 0
    }

    /// What a wake says of it: "passed", "none ran", or what failed in a few words.
    pub fn words(&self) -> String {
        if self.ok {
            return if self.nothing_checked() { "none ran" } else { "passed" }.into();
        }
        let many = |n: usize, one: &str, more: &str| if n == 1 { format!("1 {one}") } else { format!("{n} {more}") };
        let mut parts: Vec<String> = Vec::new();
        if let Some(test) = self.test.as_ref().filter(|test| !test.ok) {
            parts.push(if test.timed_out {
                "the test timed out".into()
            } else if test.failed.is_empty() {
                "the test failed".into()
            } else {
                many(test.failed.len(), "test failed", "tests failed")
            });
        }
        if !self.forbid_hits.is_empty() {
            parts.push(many(self.forbid_hits.len(), "forbidden line", "forbidden lines"));
        }
        if !self.require_missing.is_empty() {
            parts.push(many(self.require_missing.len(), "required pattern missing", "required patterns missing"));
        }
        if self.checks_file_changed {
            parts.push(format!("{CHECKS_PATH} changed"));
        }
        if !self.errors.is_empty() {
            parts.push(many(self.errors.len(), "error", "errors"));
        }
        if parts.is_empty() {
            return "failed".into();
        }
        format!("failed: {}", parts.join(", "))
    }

    /// The review's Checks section, which is also what a QA round is told.
    pub fn body(&self) -> String {
        if self.ok && self.nothing_checked() {
            return NO_CHECKS.into();
        }
        let verdict = if self.ok { "ok" } else { "FAILED" };
        let mut lines = vec![format!("{verdict} · {:.1} s", self.seconds)];
        if let Some(test) = &self.test {
            lines.extend(test.summary.iter().map(|line| format!("- {line}")));
            if let Some(reused) = test.reused.as_deref().filter(|reason| !reason.is_empty()) {
                lines.push(format!("- tests not rerun: {reused}"));
            }
            if test.timed_out {
                lines.push(format!("- the test timed out after {:.0} s", test.seconds));
            } else if !test.ok
                && let Some(code) = test.exit_code
            {
                lines.push(format!("- the test exited with code {code}"));
            }
            lines.extend(test.failed.iter().take(FAILED_LIMIT).map(|name| format!("- failed: {name}")));
        }
        lines.extend(self.forbid_hits.iter().map(|hit| format!("- forbid {}:{} `{}`", hit.file, hit.line, hit.text)));
        lines.extend(self.require_missing.iter().map(|entry| format!("- missing {entry}")));
        if self.checks_file_changed {
            lines.push(format!("- {CHECKS_PATH} changed"));
        }
        lines.extend(self.errors.iter().map(|error| format!("- error: {error}")));
        lines.join("\n")
    }
}

/// A check whose test is running, kept on the record so a restarted core can finish it.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct CheckRun {
    pub turn: u32,
    /// The test's terminal.
    pub terminal: Option<String>,
    pub cmd: String,
    pub started_ms: u64,
    pub timeout_s: u64,
    /// The report so far: the patterns' half.
    pub report: CheckReport,
}

/// One turn in a ledger row.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct LedgerTurn {
    pub n: u32,
    pub worker: String,
    /// worker, qa or redo.
    pub role: String,
    pub seconds: f64,
    pub usage: Usage,
    /// The window of the refusal that ended it.
    pub refused: Option<String>,
}

/// One task's row, written when it is accepted or discarded and kept for good.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct LedgerRow {
    pub at_ms: u64,
    pub created_ms: u64,
    pub task: String,
    /// Who started it: opus, manager or owner; null for a record from before this.
    #[serde(default)]
    pub by: Option<String>,
    pub repo: String,
    /// accepted or discarded.
    pub outcome: String,
    /// What the row was for: worker, reader, scout or suite.
    pub role: String,
    /// Every turn's worker key, in order.
    pub workers: Vec<String>,
    pub turns: Vec<LedgerTurn>,
    pub qa_rounds: u32,
    pub handoffs: Vec<String>,
    pub seconds: f64,
    pub usage: Usage,
    pub changed: u32,
    pub out_of_scope: u32,
    /// The last checks in a few words.
    pub checks: Option<String>,
    /// What the review decided.
    pub review: Option<String>,
    /// The worker's last report.
    pub report: String,
}

/// What the orchestrator itself spent on one task, read from its transcript.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct ParentUsage {
    pub transcript: String,
    pub requests: u64,
    pub fresh: u64,
    pub cache_write: u64,
    pub cache_read: u64,
    pub output: u64,
    pub peak: u64,
    /// The transcript also holds the task's accept or discard, so the window is whole.
    pub closed: bool,
}

/// The Gaps section's body, when the report has one, and the report without it.
pub fn gaps_and_report(text: &str) -> (Option<String>, String) {
    let lines: Vec<&str> = text.lines().collect();
    let Some((start, level)) = lines.iter().enumerate().find_map(|(at, line)| Some((at, gaps_heading(line)?))) else {
        return (None, text.trim().to_string());
    };
    let end = (start + 1..lines.len())
        .find(|at| heading(lines[*at]).is_some_and(|other| other <= level))
        .unwrap_or(lines.len());
    let body = lines[start + 1..end].join("\n").trim().to_string();
    let rest: Vec<&str> = lines[..start].iter().chain(&lines[end..]).copied().collect();
    (Some(body), rest.join("\n").trim().to_string())
}

fn gaps_heading(line: &str) -> Option<usize> {
    let level = line.bytes().take_while(|byte| *byte == b'#').count();
    if !(1..=3).contains(&level) {
        return None;
    }
    let rest = line[level..].trim_start();
    let word = rest.get(..4)?;
    let after = rest[4..].chars().next();
    (word.eq_ignore_ascii_case("gaps") && !after.is_some_and(|c| c.is_alphanumeric() || c == '_')).then_some(level)
}

fn heading(line: &str) -> Option<usize> {
    let level = line.bytes().take_while(|byte| *byte == b'#').count();
    ((1..=6).contains(&level) && line[level..].chars().next().is_none_or(char::is_whitespace)).then_some(level)
}

/// The wake and the result: one task, small enough to read in one line of tools.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct TaskLine {
    pub task: String,
    /// Who started it: opus, manager or owner; null for a record from before this.
    #[serde(default)]
    pub by: Option<String>,
    /// git's toplevel.
    #[serde(default)]
    pub repo: String,
    pub state: TaskState,
    /// Done or accepted, nothing changed outside the scope, and the last checks passed.
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
    /// The last checks in a few words: "passed", "none ran", or what failed; null before any ran.
    pub checks: Option<String>,
    /// The last report's Gaps section, cut like the text; null when it has none.
    pub gaps: Option<String>,
}

impl TaskLine {
    /// The task as a wake, with its report cut to `text_limit` characters.
    pub fn of(task: &Task, workers: &[crate::task::worker::Worker], now_ms: u64, text_limit: usize) -> TaskLine {
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
            by: task.by.clone(),
            repo: task.repo.clone(),
            state: task.state,
            ok: matches!(task.state, TaskState::Done | TaskState::Accepted)
                && out_of_scope.is_empty()
                && task.checks.last().is_none_or(|report| report.ok),
            worker: last.map(|t| crate::task::worker::label_of(workers, &t.worker)),
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
            checks: task.checks.last().map(CheckReport::words),
            gaps: last.and_then(|t| gaps_and_report(&t.text).0).map(|gaps| cut(&gaps, text_limit)),
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

    fn workers() -> Vec<crate::task::worker::Worker> {
        crate::task::worker::builtin()
    }

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
    fn a_turns_own_cli_survives_a_removed_worker_and_old_records_look_it_up() {
        let recorded = turn("gone", "", 0.0);
        let mut recorded = recorded;
        recorded.cli = Some(Cli::CommandCode);
        assert_eq!(recorded.cli_or_lookup(&[]), Cli::CommandCode, "its own CLI stays");
        let old = turn("go", "", 0.0);
        assert_eq!(old.cli_or_lookup(&workers()), Cli::Opencode, "an old record looks the key up");
        assert_eq!(turn("gone", "", 0.0).cli_or_lookup(&[]), Cli::Opencode, "an unknown old key is OpenCode");
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
        let line = TaskLine::of(&task, &workers(), 9_999, 1500);
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
        assert!(TaskLine::of(&clean, &workers(), 9_999, 1500).ok);
    }

    #[test]
    fn usage_adds_saturate_instead_of_panicking() {
        let mut usage = Usage {
            steps: u64::MAX,
            input: u64::MAX,
            output: u64::MAX,
            reasoning: u64::MAX,
            cache_read: u64::MAX,
            cache_write: u64::MAX,
            cost_usd: 1.0,
            largest_request: 10,
        };
        usage.add(&Usage {
            steps: 1,
            input: 1,
            output: 1,
            reasoning: 1,
            cache_read: 1,
            cache_write: 1,
            cost_usd: 0.5,
            largest_request: 20,
        });
        assert_eq!(
            (usage.steps, usage.input, usage.output, usage.reasoning, usage.cache_read, usage.cache_write),
            (u64::MAX, u64::MAX, u64::MAX, u64::MAX, u64::MAX, u64::MAX)
        );
        assert!((usage.cost_usd - 1.5).abs() < 1e-9);
        assert_eq!(usage.largest_request, 20);
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
        let line = TaskLine::of(&task, &workers(), 3_000, 3);
        assert_eq!(line.seconds, 2.0);
        assert_eq!(line.text, "ééé");
        assert!(!line.ok);
        assert_eq!(cut("ab", 3), "ab");
    }

    fn failed_report() -> CheckReport {
        CheckReport {
            turn: 1,
            seconds: 12.34,
            test: Some(TestRun {
                exit_code: Some(1),
                summary: vec!["Ran 3 tests in 1s".into(), "FAILED (failures=2)".into()],
                failed: vec!["a_test".into(), "b_test".into()],
                ..Default::default()
            }),
            forbid_hits: vec![Hit { pattern: "x".into(), file: "src/a.rs".into(), line: 7, text: "x = 1".into() }],
            require_missing: vec!["src/**::name".into()],
            checks_file_changed: true,
            errors: vec!["a bad pattern".into()],
            ..Default::default()
        }
    }

    #[test]
    fn a_report_reads_as_the_review_shows_it() {
        let report = failed_report();
        assert_eq!(
            report.body(),
            "FAILED · 12.3 s\n- Ran 3 tests in 1s\n- FAILED (failures=2)\n- the test exited with code 1\n\
             - failed: a_test\n- failed: b_test\n- forbid src/a.rs:7 `x = 1`\n- missing src/**::name\n\
             - .kompitum/checks.json changed\n- error: a bad pattern"
        );
        assert_eq!(
            report.words(),
            "failed: 2 tests failed, 1 forbidden line, 1 required pattern missing, .kompitum/checks.json changed, 1 error"
        );
        let good = CheckReport { ok: true, seconds: 1.0, forbid_checked: 1, ..Default::default() };
        assert_eq!(good.body(), "ok · 1.0 s");
        assert_eq!(good.words(), "passed");
        let none = CheckReport { ok: true, seconds: 1.0, ..Default::default() };
        assert_eq!(none.body(), NO_CHECKS, "no check was asked for, so nothing passed");
        assert_eq!(none.words(), "none ran");
        let timed = CheckReport {
            test: Some(TestRun { timed_out: true, seconds: 90.0, ..Default::default() }),
            ..Default::default()
        };
        assert!(timed.body().contains("- the test timed out after 90 s"), "{}", timed.body());
        assert_eq!(timed.words(), "failed: the test timed out");
        let reused = CheckReport {
            ok: true,
            test: Some(TestRun { ok: true, reused: Some("only *.md changed".into()), ..Default::default() }),
            ..Default::default()
        };
        assert!(reused.body().contains("- tests not rerun: only *.md changed"));
    }

    #[test]
    fn failed_checks_make_a_finished_task_not_ok_and_the_line_says_so() {
        let mut task = Task {
            name: "fix-3".into(),
            state: TaskState::Done,
            turns: vec![turn("go", "done\n\n## Gaps\nThe docs.\n", 0.0)],
            checks: vec![failed_report()],
            ..Default::default()
        };
        let line = TaskLine::of(&task, &workers(), 0, 1500);
        assert!(!line.ok);
        assert_eq!(line.checks.as_deref().map(|words| words.starts_with("failed: ")), Some(true));
        assert_eq!(line.gaps.as_deref(), Some("The docs."));
        task.checks.push(CheckReport { ok: true, forbid_checked: 1, ..Default::default() });
        let line = TaskLine::of(&task, &workers(), 0, 1500);
        assert!(line.ok, "the last report decides");
        assert_eq!(line.checks.as_deref(), Some("passed"));
        task.state = TaskState::Accepted;
        assert!(TaskLine::of(&task, &workers(), 0, 1500).ok);
    }

    #[test]
    fn the_gaps_section_comes_out_of_a_report() {
        let (gaps, rest) = gaps_and_report("# Report\n\n## What changed\nA.\n\n## Gaps\nNone.\n\n## After\nB.\n");
        assert_eq!(gaps.as_deref(), Some("None."));
        assert_eq!(rest, "# Report\n\n## What changed\nA.\n\n## After\nB.");
        let (gaps, rest) = gaps_and_report("##gaps and more\n### a subheading stays in\nx\n# Next\ny");
        assert_eq!(gaps.as_deref(), Some("### a subheading stays in\nx"));
        assert_eq!(rest, "# Next\ny");
        assert_eq!(gaps_and_report("## Gapsless\nx").0, None);
        assert_eq!(gaps_and_report("#### Gaps\nx").0, None, "a heading past level 3 isn't the section");
        assert_eq!(gaps_and_report("no headings at all").1, "no headings at all");
        assert_eq!(gaps_and_report("## Gapé\nx").0, None, "a word cut inside a character is not a panic");
    }

    #[test]
    fn a_task_being_checked_holds_its_slot_and_its_scope() {
        let checking = Task { name: "a".into(), state: TaskState::Checking, repo: "r".into(), ..Default::default() };
        let next = Task { name: "b".into(), state: TaskState::Queued, repo: "r".into(), seq: 1, ..Default::default() };
        let one = crate::task::decide::may_start(&next, &[&checking], 1).unwrap_err();
        assert!(one.contains("1 of 1 running"), "{one}");
        let two = crate::task::decide::may_start(&next, &[&checking], 2).unwrap_err();
        assert!(two.contains("waiting for a: its scope overlaps"), "{two}");
    }

    #[test]
    fn checking_has_not_ended_and_accepted_has() {
        assert_eq!(TaskState::Checking.word(), "checking");
        assert_eq!(TaskState::Accepted.word(), "accepted");
        assert!(!TaskState::Checking.ended() && !TaskState::Checking.ended_badly());
        assert!(TaskState::Accepted.ended() && !TaskState::Accepted.ended_badly());
    }

    #[test]
    fn state_words_are_what_the_api_and_files_use() {
        assert_eq!(TaskState::TimedOut.word(), "timed_out");
        assert_eq!(TaskState::Stranded.word(), "stranded");
        assert!(!TaskState::Stranded.ended() && !TaskState::Stranded.ended_badly());
        assert!(TaskState::Cancelled.ended_badly() && TaskState::Done.ended() && !TaskState::Running.ended());
    }
}
