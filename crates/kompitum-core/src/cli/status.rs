//! `kom status`: everything at one glance, for the Owner, Opus and Manager.
//! `kom status --line` is Claude Code's status line: it keeps the limit resets
//! in a file and prints one short line, without a core.

use super::*;
use std::io::Read;
use std::path::Path;

#[derive(clap::Args)]
pub(super) struct StatusArgs {
    /// Read Claude Code's status line JSON on stdin: keep its limit resets and print one line.
    #[arg(long)]
    pub line: bool,
}

pub(super) fn status_cmd(
    a: StatusArgs,
    _asked: bool,
    call: impl Fn(&str, Value) -> Result<Value, RpcError>,
) -> Result<Out, RpcError> {
    if a.line {
        // Claude's status line gets its one line whatever happens here.
        match Paths::from_env() {
            Ok(paths) => status_line(&paths.state),
            Err(_) => say(""),
        }
        std::process::exit(0);
    }
    let paths = Paths::from_env()?;
    let terminals = call("terminal.list", json!({}))?;
    let tasks = call("task.list", json!({"all": true}))?;
    let workers = call("workers.show", json!({}))?;
    let mut agents = Vec::new();
    for terminal in terminals["terminals"].as_array().into_iter().flatten() {
        if terminal["state"] != "live" {
            continue;
        }
        let Some(agent) = terminal["status"]["app"].as_str().filter(|app| !app.is_empty()) else { continue };
        agents.push(json!({
            "terminal": terminal["address"],
            "name": terminal["name"],
            "agent": agent,
            "state": terminal["status"]["state"],
            "cwd": terminal["foreground"]["cwd"].as_str()
                .or_else(|| terminal["cwd"].as_str()).unwrap_or_default(),
        }));
    }
    let mut review = Vec::new();
    let mut running = Vec::new();
    let mut queued = Vec::new();
    let mut recent = Vec::new();
    for task in tasks["tasks"].as_array().into_iter().flatten() {
        match task["state"].as_str().unwrap_or_default() {
            "running" | "checking" => running.push(status_task(task)),
            "queued" | "stranded" => queued.push(status_task(task)),
            "accepted" | "discarded" => recent.push(status_task(task)),
            "done" | "failed" | "timed_out" | "cancelled" => review.push(status_task(task)),
            _ => {}
        }
    }
    // `kom task ls` goes oldest first, so the last ten are the newest.
    if recent.len() > 10 {
        recent.drain(..recent.len() - 10);
    }
    let blocked = agents.iter().filter(|agent| agent["state"] == "blocked").count();
    let review_count = review.len();
    let result = json!({
        "agents": agents,
        "tasks": {"review": review, "running": running, "queued": queued, "recent": recent},
        "workers": workers,
        "claude": claude_limits(&paths.state),
        "waiting": {"review": review_count, "blocked": blocked},
    });
    let human = status_human(&result);
    Ok(Out { json: result, human })
}

/// Each task in the one shape the Board reads.
fn status_task(task: &Value) -> Value {
    json!({
        "name": task["task"],
        "by": task["by"],
        "repo": task["repo"],
        "worker": task["worker"],
        "state": task["state"],
        "seconds": task["seconds"],
        "cost_usd": task["cost_usd"],
        "checks": task["checks"],
    })
}

/// What `kom status --line` last kept, or null.
fn claude_limits(state: &Path) -> Value {
    let Ok(bytes) = std::fs::read(state.join("claude-limits.json")) else { return Value::Null };
    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else { return Value::Null };
    if !value.is_object() {
        return Value::Null;
    }
    json!({
        "five_hour_resets_at": value.get("five_hour_resets_at").cloned().unwrap_or(Value::Null),
        "seven_day_resets_at": value.get("seven_day_resets_at").cloned().unwrap_or(Value::Null),
        "seen_at": value.get("seen_at").cloned().unwrap_or(Value::Null),
    })
}

fn hhmm(ms: u64) -> String {
    let at = crate::task::clock::civil(ms, crate::platform::utc_offset_minutes());
    format!("{:02}:{:02}", at.hour, at.minute)
}

/// A few short lines, starting with what waits for the person reading them.
fn status_human(r: &Value) -> String {
    let names = |list: &Value| -> Vec<String> {
        list.as_array().into_iter().flatten().filter_map(|item| item["name"].as_str()).map(String::from).collect()
    };
    let review = names(&r["tasks"]["review"]);
    let blocked: Vec<String> = r["agents"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|agent| agent["state"] == "blocked")
        .map(|agent| {
            agent["name"]
                .as_str()
                .filter(|name| !name.is_empty())
                .or_else(|| agent["terminal"].as_str())
                .unwrap_or("?")
                .to_string()
        })
        .collect();
    let mut waiting = Vec::new();
    if !review.is_empty() {
        waiting.push(format!("{} to review: {}", review.len(), review.join(", ")));
    }
    if !blocked.is_empty() {
        waiting.push(format!("{} blocked: {}", blocked.len(), blocked.join(", ")));
    }
    let mut lines = vec![if waiting.is_empty() { "Nothing waits for you".into() } else { waiting.join("; ") }];
    let agents: Vec<String> = r["agents"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|agent| {
            let name = agent["name"]
                .as_str()
                .filter(|name| !name.is_empty())
                .or_else(|| agent["terminal"].as_str())
                .unwrap_or("?");
            format!(
                "{name} {} {}",
                agent["agent"].as_str().unwrap_or("?"),
                agent["state"].as_str().unwrap_or("no status")
            )
        })
        .collect();
    lines.push(format!("agents: {}", if agents.is_empty() { "none".to_string() } else { agents.join(", ") }));
    let count = |key: &str| r["tasks"][key].as_array().map_or(0, Vec::len);
    lines.push(format!("tasks: {} running, {} queued, {} recent", count("running"), count("queued"), count("recent")));
    let workers = &r["workers"];
    let label = |key: &str| {
        workers["workers"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|worker| worker["key"] == key)
            .and_then(|worker| worker["label"].as_str())
            .map(String::from)
    };
    lines.push(match workers["next"].as_str().and_then(label) {
        Some(label) => format!("workers: next {label}"),
        None => format!("workers: {}", workers["all_out"].as_str().unwrap_or("none ready")),
    });
    if let Some(seconds) = r["claude"]["five_hour_resets_at"].as_u64() {
        lines.push(format!("Claude's 5h limit resets at {}", hhmm(seconds.saturating_mul(1000))));
    }
    lines.join("\n")
}

/// Claude Code's status line: keep the reset times it reports, then say one line.
fn status_line(state: &Path) {
    let mut text = String::new();
    if std::io::stdin().read_to_string(&mut text).is_err() {
        say("");
        return;
    }
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        say("");
        return;
    };
    let five = reset_at(&value, "five_hour");
    let seven = reset_at(&value, "seven_day");
    if five.is_some() || seven.is_some() {
        let limits = json!({
            "five_hour_resets_at": five,
            "seven_day_resets_at": seven,
            "seen_at": crate::runtime::now_ms(),
        });
        if std::fs::create_dir_all(state).is_ok()
            && let Ok(bytes) = serde_json::to_vec(&limits)
        {
            let _ = crate::paths::write_atomic(&state.join("claude-limits.json"), &bytes);
        }
    }
    let mut parts = Vec::new();
    if let Some(name) = value["model"]["display_name"].as_str().filter(|name| !name.trim().is_empty()) {
        parts.push(name.trim().to_string());
    }
    if let Some(seconds) = five {
        parts.push(format!("5h resets {}", hhmm(seconds.saturating_mul(1000))));
    }
    say(&parts.join(" · "));
}

/// A window's `resets_at`, in Unix seconds.
fn reset_at(value: &Value, window: &str) -> Option<u64> {
    let number = value.get("rate_limits")?.get(window)?.get("resets_at")?;
    number.as_u64().or_else(|| number.as_f64().filter(|s| s.is_finite() && *s >= 0.0).map(|s| s as u64))
}

fn say(line: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stdout(), "{line}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_glance_starts_with_what_waits_for_the_person() {
        let r = json!({
            "agents": [{"terminal": "t1", "name": "pane", "agent": "claude-code", "state": "blocked", "cwd": "C:/x"}],
            "tasks": {"review": [{"name": "fix-1"}], "running": [], "queued": [], "recent": []},
            "workers": {"mode": "auto", "next": "go", "all_out": null, "workers": [{"key": "go", "label": "opencode Go"}]},
            "claude": {"five_hour_resets_at": 1762560000u64, "seven_day_resets_at": null, "seen_at": 0},
            "waiting": {"review": 1, "blocked": 1}
        });
        let text = status_human(&r);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "1 to review: fix-1; 1 blocked: pane");
        assert!(text.contains("agents: pane claude-code blocked"), "{text}");
        assert!(text.contains("workers: next opencode Go"), "{text}");
        assert!(text.contains("Claude's 5h limit resets at "), "{text}");
        let none = status_human(&json!({
            "agents": [], "tasks": {"review": [], "running": [], "queued": [], "recent": []},
            "workers": {"workers": [], "next": null, "all_out": "every worker is out"}, "claude": null,
            "waiting": {"review": 0, "blocked": 0}
        }));
        assert_eq!(none.lines().next(), Some("Nothing waits for you"));
    }

    #[test]
    fn the_status_line_keeps_only_what_it_was_given() {
        assert_eq!(reset_at(&json!({}), "five_hour"), None);
        assert_eq!(reset_at(&json!({"rate_limits": {}}), "five_hour"), None);
        assert_eq!(
            reset_at(&json!({"rate_limits": {"five_hour": {"resets_at": 1762560000u64}}}), "five_hour"),
            Some(1762560000)
        );
    }
}
