//! The ledger: one row per task, written when it is accepted or discarded and kept for good.

use crate::api::{LedgerParams, LedgerResult};
use crate::rpc::RpcError;
use crate::runtime::Core;
use crate::task::parent_usage;
use crate::task::types::{CheckReport, LedgerRow, LedgerTurn, ParentUsage, Task, Usage};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// How much of the last report a row keeps; cut on a character, never a byte.
const REPORT_LIMIT: usize = 20_000;
/// How many rows `show` reads when no count is asked for.
const DEFAULT_LAST: usize = 20;

/// Append this task's row. `outcome` is accepted or discarded.
pub fn record(state: &Path, task: &Task, outcome: &str, now_ms: u64) -> Result<(), String> {
    let mut line =
        serde_json::to_string(&row(task, outcome, now_ms)).map_err(|e| format!("cannot write the ledger row: {e}"))?;
    line.push('\n');
    let path = state.join("ledger.jsonl");
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut file| file.write_all(line.as_bytes()))
        .map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// One task's row, decided from its record alone.
pub fn row(task: &Task, outcome: &str, now_ms: u64) -> LedgerRow {
    let mut usage = Usage::default();
    for turn in &task.turns {
        usage.add(&turn.usage);
    }
    let seconds = match (task.started_ms, task.ended_ms) {
        (Some(started), Some(ended)) => ended.saturating_sub(started) as f64 / 1000.0,
        _ => 0.0,
    };
    let report = task.turns.last().map(|turn| turn.text.as_str()).unwrap_or_default();
    LedgerRow {
        at_ms: now_ms,
        created_ms: task.created_ms,
        task: task.name.clone(),
        repo: task.repo.clone(),
        outcome: outcome.into(),
        role: "worker".into(),
        workers: task.turns.iter().map(|turn| turn.worker.clone()).collect(),
        turns: task
            .turns
            .iter()
            .map(|turn| LedgerTurn {
                n: turn.n,
                worker: turn.worker.clone(),
                role: if turn.role.is_empty() { "worker".into() } else { turn.role.clone() },
                seconds: turn
                    .ended_ms
                    .map(|ended| ended.saturating_sub(turn.started_ms) as f64 / 1000.0)
                    .unwrap_or(0.0),
                usage: turn.usage.clone(),
                refused: turn.refused.clone(),
            })
            .collect(),
        qa_rounds: task.qa_used,
        handoffs: task.handoffs.clone(),
        seconds,
        usage,
        changed: u32::try_from(task.changed.len()).unwrap_or(u32::MAX),
        out_of_scope: u32::try_from(task.changed.iter().filter(|file| !file.in_scope).count()).unwrap_or(u32::MAX),
        checks: task.checks.last().map(CheckReport::words),
        review: task.review.clone(),
        report: crate::task::types::cut(report, REPORT_LIMIT),
    }
}

/// What the rows add up to.
pub fn totals(rows: &[LedgerRow]) -> Usage {
    let mut usage = Usage::default();
    for row in rows {
        usage.add(&row.usage);
    }
    usage
}

/// The rows as Markdown: one line per task, then their totals, then the parent's line.
pub fn table(rows: &[LedgerRow], parent: Option<&ParentUsage>) -> String {
    let mut lines = vec![
        "| When | Task | Outcome | Workers | Turns | QA | Time | Steps | Cost | Cache | Largest | Review |".to_string(),
        "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |".to_string(),
    ];
    lines.extend(rows.iter().map(table_row));
    lines.push(totals_line(rows));
    if let Some(parent) = parent {
        lines.push(parent_line(parent));
    }
    lines.join("\n")
}

fn table_row(row: &LedgerRow) -> String {
    let mut workers: Vec<&str> = Vec::new();
    for worker in &row.workers {
        if workers.last().copied() != Some(worker.as_str()) {
            workers.push(worker);
        }
    }
    format!(
        "| {} | {} | {} | {} | {} | {} | {} | {} | ${:.3} | {:.1}% | {} | {} |",
        when_text(row.at_ms),
        cell(&row.task),
        cell(&row.outcome),
        workers.join("→"),
        row.turns.len(),
        row.qa_rounds,
        time_text(row.seconds),
        row.usage.steps,
        row.usage.cost_usd,
        cache_share(&row.usage),
        token_text(row.usage.largest_request),
        cell(row.review.as_deref().unwrap_or_default()),
    )
}

/// A table cell cannot hold a pipe or a line break, or the row would split.
fn cell(text: &str) -> String {
    text.replace('|', "/").replace(['\r', '\n'], " ")
}

fn when_text(ms: u64) -> String {
    let at = crate::task::clock::civil(ms, crate::platform::utc_offset_minutes());
    format!("{:04}-{:02}-{:02} {:02}:{:02}", at.year, at.month, at.day, at.hour, at.minute)
}

/// Seconds under a minute, minutes after it, as `_markdown_rows` rounds them.
fn time_text(seconds: f64) -> String {
    if seconds < 60.0 { format!("{seconds:.0} s") } else { format!("{} min", (seconds / 60.0).round() as u64) }
}

fn cache_share(usage: &Usage) -> f64 {
    let total = usage.input.saturating_add(usage.cache_read);
    if total == 0 { 0.0 } else { usage.cache_read as f64 * 100.0 / total as f64 }
}

/// A token count the way the Python's ledger writes them: 45K, 2.4M.
fn token_text(value: u64) -> String {
    if value >= 1_000_000 {
        let text = format!("{:.1}", value as f64 / 1_000_000.0);
        format!("{}M", text.trim_end_matches('0').trim_end_matches('.'))
    } else {
        format!("{}K", (value as f64 / 1000.0).round() as u64)
    }
}

fn count(n: usize, one: &str, many: &str) -> String {
    if n == 1 { format!("{n} {one}") } else { format!("{n} {many}") }
}

fn totals_line(rows: &[LedgerRow]) -> String {
    let usage = totals(rows);
    let seconds: f64 = rows.iter().map(|row| row.seconds).sum();
    let turns: usize = rows.iter().map(|row| row.turns.len()).sum();
    let qa: u32 = rows.iter().map(|row| row.qa_rounds).sum();
    let qa = if qa == 1 { "1 QA round".to_string() } else { format!("{qa} QA rounds") };
    format!(
        "Totals: {}, {}, {}, {}, {} steps, ${:.3}, {:.1}% cached, largest {}.",
        count(rows.len(), "row", "rows"),
        count(turns, "turn", "turns"),
        qa,
        time_text(seconds),
        usage.steps,
        usage.cost_usd,
        cache_share(&usage),
        token_text(usage.largest_request),
    )
}

/// The orchestrator's own window, in `runtime.py`'s words.
fn parent_line(parent: &ParentUsage) -> String {
    format!(
        "Parent: {} requests, {} cache reads, {} output, peak {}.",
        parent.requests,
        token_text(parent.cache_read),
        token_text(parent.output),
        token_text(parent.peak),
    )
}

/// `<state>/ledger.jsonl`, oldest row first; a damaged line is named, never skipped silently.
fn read_rows(state: &Path) -> (Vec<LedgerRow>, Vec<String>) {
    let path = state.join("ledger.jsonl");
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Vec::new(), Vec::new()),
        Err(e) => return (Vec::new(), vec![format!("cannot read {}: {e}", path.display())]),
    };
    let text = String::from_utf8_lossy(&bytes);
    let mut rows = Vec::new();
    let mut problems = Vec::new();
    for (at, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<LedgerRow>(line) {
            Ok(row) => rows.push(row),
            Err(e) => problems.push(format!("ledger.jsonl line {}: {e}", at + 1)),
        }
    }
    (rows, problems)
}

fn config_dir() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::platform::home_dir().join(".claude"))
}

pub async fn show(core: &Arc<Core>, p: LedgerParams) -> Result<LedgerResult, RpcError> {
    let state = core.paths.state.clone();
    let (mut rows, problems) = tokio::task::spawn_blocking(move || read_rows(&state))
        .await
        .map_err(|e| RpcError::new("unavailable", format!("cannot read the ledger: {e}"), "try again"))?;
    if let Some(task) = &p.task {
        rows.retain(|row| &row.task == task);
    } else {
        let last = p.last.unwrap_or(DEFAULT_LAST).max(1);
        if rows.len() > last {
            rows.drain(..rows.len() - last);
        }
    }
    let parent = match &p.task {
        None => None,
        Some(task) => {
            // A swept task has no record, so its row says when it started; with neither,
            // there is nothing to search for and no window to show.
            let since =
                rows.last().map(|row| row.created_ms).or_else(|| core.tasks.snapshot(task).map(|task| task.created_ms));
            match since {
                None => None,
                Some(since) => {
                    let name = task.clone();
                    tokio::task::spawn_blocking(move || {
                        let path = parent_usage::find_transcript(&name, since, &config_dir())?;
                        parent_usage::usage(&path, &name)
                    })
                    .await
                    .unwrap_or(None)
                }
            }
        }
    };
    let totals = totals(&rows);
    let table = table(&rows, parent.as_ref());
    Ok(LedgerResult { rows, table, totals, parent, problems })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::types::{ChangedFile, CheckReport, TaskState, Turn};

    fn usage(steps: u64, cost: f64, largest: u64) -> Usage {
        Usage {
            steps,
            input: 100,
            output: 20,
            reasoning: 10,
            cache_read: 50,
            cache_write: 5,
            cost_usd: cost,
            largest_request: largest,
        }
    }

    #[test]
    fn a_row_carries_every_turn_and_cuts_the_report_on_a_character() {
        let report = format!("{}tail", "é".repeat(REPORT_LIMIT));
        let task = Task {
            name: "fix-1".into(),
            repo: "/repo".into(),
            state: TaskState::Accepted,
            created_ms: 500,
            started_ms: Some(1_000),
            ended_ms: Some(11_000),
            turns: vec![
                Turn {
                    n: 1,
                    worker: "go".into(),
                    role: String::new(),
                    started_ms: 1_000,
                    ended_ms: Some(3_000),
                    usage: usage(1, 0.25, 150),
                    refused: Some("quota window".into()),
                    text: "first".into(),
                    ..Default::default()
                },
                Turn {
                    n: 2,
                    worker: "free".into(),
                    role: "qa".into(),
                    started_ms: 4_000,
                    ended_ms: Some(11_000),
                    usage: usage(2, 0.5, 900),
                    text: report,
                    ..Default::default()
                },
            ],
            qa_used: 1,
            handoffs: vec!["go stopped".into()],
            changed: vec![
                ChangedFile { path: "src/a.rs".into(), in_scope: true, ..Default::default() },
                ChangedFile { path: "outside.txt".into(), in_scope: false, ..Default::default() },
            ],
            checks: vec![CheckReport { ok: true, forbid_checked: 1, ..Default::default() }],
            review: Some("sound".into()),
            ..Default::default()
        };
        let row = row(&task, "accepted", 20_000);
        assert_eq!(row.at_ms, 20_000);
        assert_eq!(row.created_ms, 500);
        assert_eq!(row.task, "fix-1");
        assert_eq!(row.repo, "/repo");
        assert_eq!(row.outcome, "accepted");
        assert_eq!(row.role, "worker");
        assert_eq!(row.workers, ["go", "free"]);
        assert_eq!(row.turns.len(), 2);
        assert_eq!(row.turns[0].role, "worker", "an empty role reads as worker");
        assert_eq!(row.turns[1].role, "qa");
        assert_eq!(row.turns[0].seconds, 2.0);
        assert_eq!(row.turns[1].seconds, 7.0);
        assert_eq!(row.turns[0].refused.as_deref(), Some("quota window"));
        assert_eq!(row.qa_rounds, 1);
        assert_eq!(row.handoffs, ["go stopped"]);
        assert_eq!(row.seconds, 10.0);
        assert_eq!(row.usage.steps, 3);
        assert!((row.usage.cost_usd - 0.75).abs() < 1e-9);
        assert_eq!(row.usage.largest_request, 900);
        assert_eq!(row.changed, 2);
        assert_eq!(row.out_of_scope, 1);
        assert_eq!(row.checks.as_deref(), Some("passed"));
        assert_eq!(row.review.as_deref(), Some("sound"));
        assert_eq!(row.report.chars().count(), REPORT_LIMIT, "the report is cut on a character");
        assert_eq!(row.report, "é".repeat(REPORT_LIMIT));
    }

    #[test]
    fn the_table_holds_a_pipe_and_a_newline_and_adds_its_totals() {
        let first = LedgerRow {
            at_ms: 0,
            task: "fix-1".into(),
            outcome: "discarded".into(),
            workers: vec!["go".into(), "go".into(), "free".into(), "go".into()],
            turns: vec![
                LedgerTurn { n: 1, worker: "go".into(), role: "worker".into(), seconds: 30.0, ..Default::default() },
                LedgerTurn { n: 2, worker: "free".into(), role: "worker".into(), seconds: 60.0, ..Default::default() },
            ],
            qa_rounds: 1,
            seconds: 90.0,
            usage: usage(3, 0.25, 1_500),
            review: Some("good | bad\nsecond line".into()),
            ..Default::default()
        };
        let second = LedgerRow {
            at_ms: 0,
            task: "fix-2".into(),
            outcome: "accepted".into(),
            workers: vec!["free".into()],
            turns: vec![LedgerTurn {
                n: 1,
                worker: "free".into(),
                role: "worker".into(),
                seconds: 5.0,
                ..Default::default()
            }],
            seconds: 5.0,
            usage: usage(2, 0.5, 2_400_000),
            ..Default::default()
        };
        let parent = ParentUsage {
            transcript: "t.jsonl".into(),
            requests: 3,
            fresh: 10,
            cache_write: 0,
            cache_read: 1_234_567,
            output: 1_500,
            peak: 2_400_000,
            closed: true,
        };
        let text = table(&[first, second], Some(&parent));
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines[0],
            "| When | Task | Outcome | Workers | Turns | QA | Time | Steps | Cost | Cache | Largest | Review |"
        );
        assert_eq!(lines[2].matches('|').count(), 13, "one pipe per column edge: {}", lines[2]);
        assert!(
            lines[2].contains("go→free→go"),
            "a hand-off and back is worth seeing, while only repeats in a row fold: {}",
            lines[2]
        );
        assert!(!lines[2].contains("go→go"), "a repeat in a row folds: {}", lines[2]);
        assert!(lines[2].contains("2 min"), "90 seconds reads in minutes: {}", lines[2]);
        assert!(lines[2].contains("33.3%"), "a third of the input was cached: {}", lines[2]);
        assert!(lines[2].contains("2K"), "the largest request in the row: {}", lines[2]);
        assert!(lines[2].contains("good / bad second line"), "the review stays on one line: {}", lines[2]);
        assert!(lines[3].contains("fix-2"), "{}", lines[3]);
        assert!(lines[4].starts_with("Totals: 2 rows"), "{}", lines[4]);
        assert!(lines[4].contains("$0.750"), "{}", lines[4]);
        assert!(lines[4].contains("5 steps"), "{}", lines[4]);
        assert_eq!(
            lines[5], "Parent: 3 requests, 1.2M cache reads, 2K output, peak 2.4M.",
            "the parent line is the Python's"
        );
    }
}
