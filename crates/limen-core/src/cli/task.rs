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
    /// Remove a finished task's worktree and branch.
    Discard(DiscardArgs),
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
    /// The worker: go or free.
    #[arg(long, value_parser = ["go", "free"])]
    pub worker: Option<String>,
    /// Seconds without output or CPU before the turn is cut (default 900).
    #[arg(long)]
    pub idle: Option<u64>,
    /// Seconds the turn may run (default 3600).
    #[arg(long)]
    pub timeout: Option<u64>,
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
    /// Seconds to wait (default: as long as it takes).
    #[arg(long)]
    pub timeout: Option<f64>,
    /// Seconds a name not known yet stays pending, for a start still on its way (default 30).
    #[arg(long, default_value_t = 30.0)]
    pub grace: f64,
}

#[derive(clap::Args)]
pub(super) struct DiscardArgs {
    /// The task's name.
    pub task: String,
    /// What the review decided, kept with the record.
    #[arg(long)]
    pub review: Option<String>,
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
    matches!(state, "done" | "failed" | "timed_out" | "cancelled" | "discarded")
}

fn ended_badly(state: &str) -> bool {
    matches!(state, "failed" | "timed_out" | "cancelled")
}

pub(super) fn task(
    _paths: &Paths,
    action: TaskCmd,
    _json: bool,
    call: impl Fn(&str, Value) -> Result<Value, RpcError>,
) -> Result<Option<Out>, RpcError> {
    Ok(Some(match action {
        TaskCmd::Start(a) => {
            let brief = if a.brief == "-" {
                let mut text = String::new();
                std::io::stdin()
                    .read_to_string(&mut text)
                    .map_err(|e| RpcError::new("usage", format!("cannot read stdin: {e}"), ""))?;
                text
            } else {
                std::fs::read_to_string(&a.brief).map_err(|e| {
                    RpcError::new("usage", format!("cannot read {}: {e}", a.brief), "give a brief file, or - for stdin")
                })?
            };
            let folder = from_here(a.folder);
            let r = call(
                "task.start",
                json!({
                    "task": a.name, "brief": brief, "folder": folder, "scope": a.scope,
                    "worker": a.worker, "idle": a.idle, "timeout": a.timeout, "caller": caller(),
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
                task["turns"].as_array().and_then(|turns| turns.first()).and_then(|turn| turn["terminal"].as_str())
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
    }))
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
        let params = json!({"tasks": names, "first": a.first, "timeout": checkin});
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
            let problem_names: Vec<String> =
                problems.iter().filter_map(|line| line["task"].as_str()).map(String::from).collect();
            let unfinished_names: Vec<String> =
                unfinished.iter().filter_map(|line| line["task"].as_str()).map(String::from).collect();
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
                "unknown": unknown,
            }});
            println!("{summary}");
            let code = if timed_out {
                2
            } else if !problems.is_empty()
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
