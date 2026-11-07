//! `lc task …`: the worker loop's commands, each one call to the core.

use super::*;
use clap::Subcommand;
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

#[derive(Subcommand)]
pub(super) enum TaskCmd {
    /// Start a task: a worktree, a contained terminal and a worker.
    Start(StartArgs),
    /// List tasks (--all includes discarded).
    Ls(LsArgs),
    /// Show one task: its record, progress, changes and report.
    Show(NameArg),
    /// Wait for tasks; prints one JSON line each and a summary, at the end.
    Wait(WaitArgs),
    /// Cancel a queued or running task.
    Cancel(NameArg),
    /// Remove a finished task's worktree and branch, and write its ledger row.
    Discard(DiscardArgs),
    /// The whole review of a task in one read: checks, scope, warns, Gaps, report and the hunks they point at.
    Review(ReviewArgs),
    /// Bring a finished task's changes into its repository and commit only those files.
    Accept(AcceptArgs),
    /// Send a finished task back with a follow-up brief; its checks run again.
    Redo(RedoArgs),
    /// Remove what old tasks left; their ledger rows stay.
    Sweep(SweepArgs),
}

#[derive(clap::Args)]
pub(super) struct StartArgs {
    /// The task's name (1-32 of a-z 0-9 - _ ., starting with a letter or digit).
    pub name: String,
    /// The brief's file, or - for stdin.
    #[arg(long, value_name = "FILE")]
    pub brief: String,
    /// The folder in the repository (default: where lc runs).
    #[arg(long = "in", value_name = "FOLDER")]
    pub folder: Option<String>,
    /// Paths the task may change (repeatable); none means the whole repository.
    #[arg(long = "scope", value_name = "GLOB")]
    pub scope: Vec<String>,
    /// Which workers to use, in order: auto is Command Code then opencode Go, free is opencode free then both. Default: what lc workers shows
    #[arg(long, value_parser = ["auto", "cc", "go", "free"])]
    pub worker: Option<String>,
    /// Seconds without output or CPU before the turn is cut (default 900).
    #[arg(long)]
    pub idle: Option<u64>,
    /// Seconds the turn may run (default 3600).
    #[arg(long)]
    pub timeout: Option<u64>,
    /// The test command the checks run when the worker ends, replacing the repository's.
    #[arg(long, value_name = "COMMAND")]
    pub test: Option<String>,
    /// A regex, or glob::regex, no added line may match (repeatable).
    #[arg(long, value_name = "PATTERN")]
    pub forbid: Vec<String>,
    /// A glob::regex some line of the matching files must match after the run (repeatable).
    #[arg(long, value_name = "PATTERN")]
    pub require: Vec<String>,
    /// A regex, or glob::regex, whose hits are flagged for the review (repeatable).
    #[arg(long, value_name = "PATTERN")]
    pub warn: Vec<String>,
    /// How many times failed checks go back to the worker, 0 to 3 (default: the repository's, else 1).
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u8).range(0..=3))]
    pub qa_rounds: Option<u8>,
    /// Let the worker use the web.
    #[arg(long)]
    pub web: bool,
}

#[derive(clap::Args)]
pub(super) struct LsArgs {
    /// Include discarded tasks.
    #[arg(long)]
    pub all: bool,
}

#[derive(clap::Args)]
pub(super) struct NameArg {
    /// The task's name.
    pub task: String,
}

#[derive(clap::Args)]
pub(super) struct WaitArgs {
    /// The tasks to wait for.
    #[arg(required = true)]
    pub names: Vec<String>,
    /// Return at the first task that ends, however it ended.
    #[arg(long)]
    pub first: bool,
    /// Keep waiting while every worker is out; the task goes on by itself when one comes back
    #[arg(long)]
    pub while_stranded: bool,
    /// Seconds to wait (default: as long as it takes).
    #[arg(long)]
    pub timeout: Option<f64>,
    /// Seconds a name not known yet stays pending, for a start still on its way (default 30).
    #[arg(long, default_value_t = 30.0)]
    pub grace: f64,
}

#[derive(clap::Args)]
pub(super) struct WorkersArgs {
    /// Which workers new tasks use, in order: auto is Command Code then opencode Go, free is opencode free then both
    #[arg(long, value_parser = ["auto", "cc", "go", "free"])]
    pub mode: Option<String>,
    /// Mark this worker as out
    #[arg(long, value_parser = ["cc", "go", "free"], requires = "until")]
    pub block: Option<String>,
    /// When it is back: 2026-11-23, 2026-11-23 15:36, or a span such as 3h or 2d
    #[arg(long, requires = "block")]
    pub until: Option<String>,
    /// Why, in your words
    #[arg(long, requires = "block")]
    pub reason: Option<String>,
    /// Say this worker is back now
    #[arg(long, value_parser = ["cc", "go", "free"])]
    pub clear: Option<String>,
}

#[derive(clap::Args)]
pub(super) struct DiscardArgs {
    /// The task's name.
    pub task: String,
    /// What the review decided, kept with the record.
    #[arg(long)]
    pub review: Option<String>,
}

#[derive(clap::Args)]
pub(super) struct ReviewArgs {
    /// The task's name.
    pub task: String,
    /// Add the whole patch; one that does not fit goes to a file the review names.
    #[arg(long)]
    pub full: bool,
    /// The review's size in bytes (default 20000).
    #[arg(long, value_name = "N")]
    pub max_bytes: Option<u64>,
}

#[derive(clap::Args)]
pub(super) struct AcceptArgs {
    /// The task's name.
    pub task: String,
    /// What the review decided, kept for the ledger.
    #[arg(long)]
    pub review: Option<String>,
    /// The commit message (default: the review, else a line naming the task).
    #[arg(long)]
    pub message: Option<String>,
    /// Put the changes in the working tree and make no commit.
    #[arg(long)]
    pub no_commit: bool,
    /// Accept a task that failed, timed out or was cancelled, or whose last checks failed.
    #[arg(long)]
    pub accept_unfinished: bool,
    /// Accept files changed outside the task's scope.
    #[arg(long)]
    pub accept_scope_violation: bool,
}

#[derive(clap::Args)]
pub(super) struct RedoArgs {
    /// The task's name.
    pub task: String,
    /// The follow-up's file, or - for stdin.
    #[arg(long, value_name = "FILE")]
    pub brief: String,
}

#[derive(clap::Args)]
pub(super) struct SweepArgs {
    /// Days after its end that a task keeps its files (default 14).
    #[arg(long, value_name = "N")]
    pub days: Option<u64>,
    /// Say what would go, and remove nothing.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(clap::Args)]
pub(super) struct LedgerArgs {
    /// Only this task's row, with the orchestrator's own usage for it.
    #[arg(long)]
    pub task: Option<String>,
    /// The newest rows to show (default 20).
    #[arg(long, value_name = "N")]
    pub last: Option<usize>,
}

fn human_seconds(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    match seconds {
        0..=59 => format!("{seconds}s"),
        60..=3599 => format!("{}m{:02}s", seconds / 60, seconds % 60),
        _ => format!("{}h{:02}m", seconds / 3600, seconds % 3600 / 60),
    }
}

fn human_cost(cost: f64) -> String {
    format!("${cost:.4}")
}

fn state(value: &Value) -> &str {
    value["state"].as_str().unwrap_or("unknown")
}

fn ended(state: &str) -> bool {
    matches!(state, "done" | "failed" | "timed_out" | "cancelled" | "discarded" | "accepted")
}

fn ended_badly(state: &str) -> bool {
    matches!(state, "failed" | "timed_out" | "cancelled")
}

fn read_brief(path: &str) -> Result<String, RpcError> {
    if path == "-" {
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .map_err(|e| RpcError::new("usage", format!("cannot read stdin: {e}"), ""))?;
        return Ok(text);
    }
    std::fs::read_to_string(path)
        .map_err(|e| RpcError::new("usage", format!("cannot read {path}: {e}"), "give a brief file, or - for stdin"))
}

/// `asked`: `--json` itself. The review and the ledger print Markdown even when piped, since
/// what reads them is a caller's context.
pub(super) fn task(
    _paths: &Paths,
    action: TaskCmd,
    asked: bool,
    call: impl Fn(&str, Value) -> Result<Value, RpcError>,
) -> Result<Option<Out>, RpcError> {
    Ok(Some(match action {
        TaskCmd::Start(a) => {
            let brief = read_brief(&a.brief)?;
            let folder = from_here(a.folder);
            let r = call(
                "task.start",
                json!({
                    "task": a.name, "brief": brief, "folder": folder, "scope": a.scope,
                    "worker": a.worker, "idle": a.idle, "timeout": a.timeout, "caller": caller(),
                    "test": a.test, "forbid": a.forbid, "require": a.require, "warn": a.warn,
                    "qa_rounds": a.qa_rounds, "web": a.web,
                }),
            )?;
            let state = state(&r);
            let mut human = if state == "running" {
                format!(
                    "{} is running on {} in {}",
                    a.name,
                    r["worker"].as_str().unwrap_or("?"),
                    r["terminal"].as_str().unwrap_or("?")
                )
            } else if state == "queued" {
                format!("{} queued: {}", a.name, r["waiting"].as_str().unwrap_or("waiting for the pump"))
            } else {
                format!("{} {state}: {}", a.name, r["error"].as_str().unwrap_or("no reason given"))
            };
            for warning in r["warnings"].as_array().into_iter().flatten() {
                human.push('\n');
                human.push_str(warning.as_str().unwrap_or_default());
            }
            if let Some(next) = r["next"].as_str() {
                human.push_str(&format!("\nnext: {next}"));
            }
            Out { json: r, human }
        }
        TaskCmd::Ls(a) => {
            let r = call("task.list", json!({"all": a.all}))?;
            let rows: Vec<String> = r["tasks"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|task| {
                    format!(
                        "{:<16} {:<9} {:<14} {:>4} {:>8} {:>9} {:>6}",
                        task["task"].as_str().unwrap_or("?"),
                        state(task),
                        task["worker"].as_str().unwrap_or("-"),
                        task["turns"].as_u64().unwrap_or(0),
                        human_seconds(task["seconds"].as_f64().unwrap_or(0.0)),
                        human_cost(task["cost_usd"].as_f64().unwrap_or(0.0)),
                        task["usage"]["steps"].as_u64().unwrap_or(0),
                    )
                })
                .collect();
            let mut human = if rows.is_empty() { "no tasks".into() } else { rows.join("\n") };
            for problem in r["problems"].as_array().into_iter().flatten() {
                human.push_str(&format!("\nproblem: {}", problem.as_str().unwrap_or_default()));
            }
            Out { json: r, human }
        }
        TaskCmd::Show(a) => {
            let r = call("task.get", json!({"task": a.task}))?;
            let task = &r["task"];
            let line = &r["line"];
            let name = task["name"].as_str().unwrap_or(&a.task);
            let mut human = format!(
                "{name} {}\nworker {}, turn {}, {}, {}, steps {}",
                state(line),
                line["worker"].as_str().unwrap_or("-"),
                line["turns"].as_u64().unwrap_or(0),
                human_seconds(line["seconds"].as_f64().unwrap_or(0.0)),
                human_cost(line["cost_usd"].as_f64().unwrap_or(0.0)),
                line["usage"]["steps"].as_u64().unwrap_or(0),
            );
            if let Some(worktree) = task["worktree"].as_str() {
                human.push_str(&format!("\nworktree {worktree}"));
            }
            if let Some(terminal) =
                task["turns"].as_array().and_then(|turns| turns.last()).and_then(|turn| turn["terminal"].as_str())
            {
                human.push_str(&format!("\nterminal {terminal}: lc read {name}"));
            }
            if let Some(tools) = r["progress"]["tools"].as_array() {
                let tools: Vec<&str> = tools.iter().filter_map(Value::as_str).collect();
                if !tools.is_empty() {
                    human.push_str(&format!("\nlast tools: {}", tools.join(", ")));
                }
            }
            if let Some(error) = task["error"].as_str() {
                human.push_str(&format!("\nerror: {error}"));
            }
            if let Some(checks) = line["checks"].as_str() {
                human.push_str(&format!("\nchecks: {checks}"));
            }
            for sentence in line["handoffs"].as_array().into_iter().flatten() {
                human.push_str(&format!("\n{}", sentence.as_str().unwrap_or_default()));
            }
            if let Some(changed) = task["changed"].as_array() {
                if !changed.is_empty() {
                    human.push_str("\nchanged:");
                }
                for file in changed {
                    let mark = if file["in_scope"] == true { " " } else { "!" };
                    human.push_str(&format!(
                        "\n {mark} {} +{} -{}",
                        file["path"].as_str().unwrap_or("?"),
                        file["added"].as_u64().unwrap_or(0),
                        file["removed"].as_u64().unwrap_or(0)
                    ));
                }
            }
            let text = line["text"].as_str().unwrap_or_default();
            if !text.is_empty() {
                human.push_str(&format!("\nreport:\n{text}"));
            }
            Out { json: r, human }
        }
        TaskCmd::Wait(a) => {
            wait(&a, &call)?;
            return Ok(None);
        }
        TaskCmd::Cancel(a) => {
            let r = call("task.cancel", json!({"task": a.task}))?;
            let human = format!("{} {} ({} survivors)", a.task, state(&r), r["survivors"].as_u64().unwrap_or(0));
            Out { json: r, human }
        }
        TaskCmd::Discard(a) => {
            let r = call("task.discard", json!({"task": a.task, "review": a.review}))?;
            let mut human = format!("{} discarded", a.task);
            if r["removed"] != true {
                human.push_str("; its worktree stayed");
            }
            for left in r["left"].as_array().into_iter().flatten() {
                human.push_str(&format!("\nleft: {}", left.as_str().unwrap_or_default()));
            }
            Out { json: r, human }
        }
        TaskCmd::Review(a) => {
            let r = call("task.review", json!({"task": a.task, "full": a.full, "max_bytes": a.max_bytes}))?;
            if !asked {
                println!("{}", r["text"].as_str().unwrap_or_default());
                return Ok(None);
            }
            Out { json: r, human: String::new() }
        }
        TaskCmd::Accept(a) => {
            let r = call(
                "task.accept",
                json!({
                    "task": a.task, "review": a.review, "message": a.message, "no_commit": a.no_commit,
                    "accept_unfinished": a.accept_unfinished, "accept_scope_violation": a.accept_scope_violation,
                }),
            )?;
            let files = r["files"].as_array().map_or(0, Vec::len);
            let mut human = match r["commit"].as_str() {
                Some(commit) => format!("{} accepted: {files} files in {commit}", a.task),
                None => format!("{} accepted: {files} files left uncommitted", a.task),
            };
            for left in r["left"].as_array().into_iter().flatten() {
                human.push_str(&format!("\nleft: {}", left.as_str().unwrap_or_default()));
            }
            Out { json: r, human }
        }
        TaskCmd::Redo(a) => {
            let brief = read_brief(&a.brief)?;
            let r = call("task.redo", json!({"task": a.task, "brief": brief}))?;
            let mut human = format!("{} {} for turn {}", a.task, state(&r), r["turn"].as_u64().unwrap_or(0));
            if let Some(waiting) = r["waiting"].as_str() {
                human.push_str(&format!(": {waiting}"));
            }
            if let Some(next) = r["next"].as_str() {
                human.push_str(&format!("\nnext: {next}"));
            }
            Out { json: r, human }
        }
        TaskCmd::Sweep(a) => {
            let r = call("task.sweep", json!({"days": a.days, "dry_run": a.dry_run}))?;
            let names = |key: &str| -> Vec<String> {
                r[key].as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from).collect()
            };
            let (removed, discarded, left) = (names("removed"), names("discarded"), names("left"));
            let mut lines = Vec::new();
            if !removed.is_empty() {
                lines.push(format!(
                    "removed: {} ({} bytes)",
                    removed.join(", "),
                    r["freed_bytes"].as_u64().unwrap_or(0)
                ));
            }
            if !discarded.is_empty() {
                lines.push(format!("discarded: {}", discarded.join(", ")));
            }
            lines.extend(left.iter().map(|left| format!("left: {left}")));
            if lines.is_empty() {
                lines.push("nothing to sweep".into());
            }
            if r["dry_run"] == true {
                lines.insert(0, "a dry run, so nothing was removed".into());
            }
            Out { human: lines.join("\n"), json: r }
        }
    }))
}

pub(super) fn ledger(
    a: LedgerArgs,
    asked: bool,
    call: impl Fn(&str, Value) -> Result<Value, RpcError>,
) -> Result<Option<Out>, RpcError> {
    let r = call("ledger.show", json!({"task": a.task, "last": a.last}))?;
    if !asked {
        println!("{}", r["table"].as_str().unwrap_or_default());
        return Ok(None);
    }
    Ok(Some(Out { json: r, human: String::new() }))
}

fn money(value: &Value) -> String {
    format!("${:.2}", value.as_f64().unwrap_or(0.0))
}

fn workers_human(r: &Value) -> String {
    let workers = r["workers"].as_array().cloned().unwrap_or_default();
    let by_key = |key: &str| workers.iter().find(|worker| worker["key"].as_str() == Some(key)).cloned();
    let labels: Vec<String> = r["chain"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(&by_key)
        .filter_map(|worker| worker["label"].as_str().map(String::from))
        .collect();
    let mut out = format!("Mode {}: {}. ", r["mode"].as_str().unwrap_or("auto"), labels.join(", then "));
    match r["next"].as_str().and_then(&by_key).and_then(|worker| worker["label"].as_str().map(String::from)) {
        Some(label) => out.push_str(&format!("The next task goes to {label}.")),
        None => out.push_str(r["all_out"].as_str().unwrap_or("No worker can take a task.")),
    }
    out.push('\n');
    for worker in &workers {
        let label = worker["label"].as_str().unwrap_or("?");
        let tail = match worker["state"].as_str() {
            Some("out") => {
                let sentence = worker["sentence"].as_str().unwrap_or_default();
                let rest = sentence.strip_prefix(label).map(str::trim_start).unwrap_or_default();
                rest.strip_prefix("is ").unwrap_or(rest)
            }
            Some("not_installed") => "not installed",
            _ => "ready",
        };
        out.push_str(&format!("\n{label:<14} {tail}"));
    }
    out.push_str(&format!("\n\n{:<14} {:<11}{:<11}{}", "Spent", "5 hours", "week", "month"));
    for worker in &workers {
        let spend = &worker["spend"];
        out.push_str(&format!(
            "\n{:<14} {:<11}{:<11}{}",
            worker["label"].as_str().unwrap_or("?"),
            money(&spend["five_hours"]["cost_usd"]),
            money(&spend["week"]["cost_usd"]),
            money(&spend["month"]["cost_usd"])
        ));
    }
    for warning in r["warnings"].as_array().into_iter().flatten() {
        out.push_str(&format!("\nproblem: {}", warning.as_str().unwrap_or_default()));
    }
    out
}

/// `lc workers`: what can take a task, or change the order and the blocks.
pub(super) fn workers(a: WorkersArgs, call: impl Fn(&str, Value) -> Result<Value, RpcError>) -> Result<Out, RpcError> {
    let setting = a.mode.is_some() || a.block.is_some() || a.clear.is_some();
    let r = if setting {
        let block = match (&a.block, &a.until) {
            (Some(worker), Some(until)) => Some(json!({"worker": worker, "until": until, "reason": a.reason})),
            _ => None,
        };
        call("workers.set", json!({"mode": a.mode, "block": block, "clear": a.clear}))?
    } else {
        call("workers.show", json!({}))?
    };
    Ok(Out { json: r.clone(), human: workers_human(&r) })
}

/// The wait's contract: nothing until the end, one JSON line per task, then
/// a summary. Exit 0 when all are ok, 1 on any problem, 2 on its own timeout.
fn wait(a: &WaitArgs, call: &impl Fn(&str, Value) -> Result<Value, RpcError>) -> Result<(), RpcError> {
    let started = Instant::now();
    let grace = Duration::from_secs_f64(a.grace.max(0.0));
    let deadline = a.timeout.map(|seconds| started + Duration::from_secs_f64(seconds.max(0.0)));
    let names: Vec<String> = a.names.clone();
    loop {
        // While a name may still be on its way, check in before its grace ends.
        let grace_left = grace.saturating_sub(started.elapsed()).as_secs_f64();
        let checkin = if grace_left > 0.0 { grace_left.clamp(0.05, 60.0) } else { 60.0 };
        let checkin = match deadline {
            Some(deadline) => checkin.min(deadline.saturating_duration_since(Instant::now()).as_secs_f64()),
            None => checkin,
        };
        let params = json!({"tasks": names, "first": a.first, "while_stranded": a.while_stranded, "timeout": checkin});
        let asked = Instant::now();
        let result = loop {
            match call("task.wait", params.clone()) {
                Ok(result) => break result,
                Err(e) if e.kind == "unavailable" && asked.elapsed() < Duration::from_secs(60) => {
                    // A core restart is normal: wait for the new one to answer.
                    std::thread::sleep(Duration::from_millis(500));
                }
                Err(e) => return Err(e),
            }
        };
        let lines = result["lines"].as_array().cloned().unwrap_or_default();
        let unknown: Vec<String> =
            result["unknown"].as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from).collect();
        let any_ended = lines.iter().any(|line| ended(state(line)));
        let bad = lines.iter().any(|line| ended_badly(state(line)));
        let grace_over = started.elapsed() >= grace;
        let done = result["over"] == true || bad || (a.first && any_ended) || (grace_over && !unknown.is_empty());
        let timed_out = deadline.is_some_and(|deadline| Instant::now() >= deadline);
        if done || timed_out {
            for name in &names {
                match lines.iter().find(|line| line["task"] == name.as_str()) {
                    Some(line) => println!("{line}"),
                    None => println!("{}", json!({"task": name, "state": "unknown"})),
                }
            }
            let problems: Vec<&serde_json::Value> = lines
                .iter()
                .filter(|line| ended_badly(state(line)) || (ended(state(line)) && line["ok"] == false))
                .collect();
            let unfinished: Vec<&serde_json::Value> = lines.iter().filter(|line| !ended(state(line))).collect();
            let stranded: Vec<&serde_json::Value> = lines.iter().filter(|line| state(line) == "stranded").collect();
            let problem_names: Vec<String> =
                problems.iter().filter_map(|line| line["task"].as_str()).map(String::from).collect();
            let unfinished_names: Vec<String> =
                unfinished.iter().filter_map(|line| line["task"].as_str()).map(String::from).collect();
            let stranded_names: Vec<String> =
                stranded.iter().filter_map(|line| line["task"].as_str()).map(String::from).collect();
            let mut bad_names = BTreeSet::new();
            bad_names.extend(problem_names.iter().cloned());
            bad_names.extend(unfinished_names.iter().cloned());
            bad_names.extend(unknown.iter().cloned());
            let total = names.len();
            let summary = json!({"summary": {
                "total": total,
                "ok": total.saturating_sub(bad_names.len()),
                "problems": problem_names,
                "unfinished": unfinished_names,
                "stranded": stranded_names,
                "unknown": unknown,
            }});
            println!("{summary}");
            let code = if timed_out {
                2
            } else if !problems.is_empty()
                || !stranded_names.is_empty()
                || (!a.first && !unfinished.is_empty())
                || !(unknown.is_empty() || (a.first && any_ended))
            {
                1
            } else {
                0
            };
            std::process::exit(code);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_workers_table_is_the_documented_shape() {
        let r = json!({
            "mode": "auto", "chain": ["cc", "go"], "next": "go", "all_out": null, "warnings": [],
            "workers": [
                {"key": "cc", "label": "Command Code", "state": "out", "until_ms": 1, "source": "person",
                 "window": "", "message": "monthly credits dead", "refusals": 0,
                 "sentence": "Command Code is out by a person's word (monthly credits dead), back on Mon 23 Nov at 00:00.",
                 "spend": {"five_hours": {"runs": 0, "cost_usd": 0.0}, "week": {"runs": 0, "cost_usd": 0.0},
                           "month": {"runs": 0, "cost_usd": 1.2}}},
                {"key": "go", "label": "opencode Go", "state": "ready", "until_ms": 0, "source": null,
                 "window": "", "message": "", "sentence": "", "refusals": 0,
                 "spend": {"five_hours": {"runs": 1, "cost_usd": 0.39}, "week": {"runs": 2, "cost_usd": 1.96},
                           "month": {"runs": 3, "cost_usd": 3.1}}},
                {"key": "free", "label": "opencode free", "state": "not_installed", "until_ms": 0, "source": null,
                 "window": "", "message": "", "sentence": "", "refusals": 0,
                 "spend": {"five_hours": {"runs": 0, "cost_usd": 0.0}, "week": {"runs": 0, "cost_usd": 0.0},
                           "month": {"runs": 0, "cost_usd": 0.0}}}
            ]
        });
        assert_eq!(
            workers_human(&r),
            "Mode auto: Command Code, then opencode Go. The next task goes to opencode Go.\n\
             \n\
             Command Code   out by a person's word (monthly credits dead), back on Mon 23 Nov at 00:00.\n\
             opencode Go    ready\n\
             opencode free  not installed\n\
             \n\
             Spent          5 hours    week       month\n\
             Command Code   $0.00      $0.00      $1.20\n\
             opencode Go    $0.39      $1.96      $3.10\n\
             opencode free  $0.00      $0.00      $0.00"
        );
    }
}
