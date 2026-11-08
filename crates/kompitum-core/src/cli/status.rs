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
            "asking" | "done" | "failed" | "timed_out" | "cancelled" => review.push(status_task(task)),
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
    let human = status_human(&result, crate::runtime::now_ms());
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
        "question": task["question"],
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
    let mut limits = serde_json::Map::new();
    for (key, from) in [("five_hour_resets_at", "five_hour_resets_at"), ("seven_day_resets_at", "seven_day_resets_at")]
    {
        limits.insert(key.into(), value.get(from).cloned().unwrap_or(Value::Null));
    }
    for (key, from) in [
        ("five_hour_used_percentage", "five_hour_used_percentage"),
        ("seven_day_used_percentage", "seven_day_used_percentage"),
    ] {
        if let Some(number) = value.get(from).and_then(Value::as_f64).filter(|number| number.is_finite()) {
            limits.insert(key.into(), json!(number));
        }
    }
    limits.insert("seen_at".into(), value.get("seen_at").cloned().unwrap_or(Value::Null));
    Value::Object(limits)
}

fn hhmm(ms: u64) -> String {
    let at = crate::task::clock::civil(ms, crate::platform::utc_offset_minutes());
    format!("{:02}:{:02}", at.hour, at.minute)
}

/// The limit windows Claude sent, with a percentage only when it sent one.
fn limits_object(value: &Value) -> Option<serde_json::Map<String, Value>> {
    let (five, seven) = (reset_at(value, "five_hour"), reset_at(value, "seven_day"));
    let (five_used, seven_used) = (used_percentage(value, "five_hour"), used_percentage(value, "seven_day"));
    if five.is_none() && seven.is_none() && five_used.is_none() && seven_used.is_none() {
        return None;
    }
    let mut limits = serde_json::Map::new();
    limits.insert("five_hour_resets_at".into(), five.map_or(Value::Null, |at| json!(at)));
    limits.insert("seven_day_resets_at".into(), seven.map_or(Value::Null, |at| json!(at)));
    if let Some(used) = five_used {
        limits.insert("five_hour_used_percentage".into(), json!(used));
    }
    if let Some(used) = seven_used {
        limits.insert("seven_day_used_percentage".into(), json!(used));
    }
    Some(limits)
}

/// The line the Owner set: `claude: 5H 42% · resets 18:40 · Week 61% · resets Sat`.
fn claude_line(value: &Value, now_ms: u64, offset_min: i32) -> Option<String> {
    let now = crate::task::clock::civil(now_ms, offset_min);
    let mut parts = Vec::new();
    for (label, window, day_is_clock) in [("5H", "five_hour", true), ("Week", "seven_day", false)] {
        let Some(seconds) = value.get(format!("{window}_resets_at")).and_then(as_u64) else { continue };
        let at = crate::task::clock::civil(seconds.saturating_mul(1000), offset_min);
        let today = (at.year, at.month, at.day) == (now.year, now.month, now.day);
        let when = if day_is_clock || today {
            format!("{:02}:{:02}", at.hour, at.minute)
        } else {
            ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"][at.weekday as usize % 7].to_string()
        };
        let used = value.get(format!("{window}_used_percentage")).and_then(Value::as_f64);
        let head = match used {
            Some(used) if used >= 100.0 => format!("{label} exhausted"),
            Some(used) => format!("{label} {}%", (used.round().max(0.0) as u64).min(99)),
            None => label.to_string(),
        };
        parts.push(if used.is_some() { format!("{head} · resets {when}") } else { format!("{head} resets {when}") });
    }
    (!parts.is_empty()).then(|| format!("claude: {}", parts.join(" · ")))
}

/// A window's field, when Claude sent one.
fn window_value<'a>(value: &'a Value, window: &str, key: &str) -> Option<&'a Value> {
    value.get("rate_limits")?.get(window)?.get(key)
}

fn as_u64(number: &Value) -> Option<u64> {
    number.as_u64().or_else(|| number.as_f64().filter(|s| s.is_finite() && *s >= 0.0).map(|s| s as u64))
}

/// A window's `used_percentage`, when Claude sent one.
fn used_percentage(value: &Value, window: &str) -> Option<f64> {
    window_value(value, window, "used_percentage")?.as_f64().filter(|percent| percent.is_finite() && *percent >= 0.0)
}

/// A few short lines, starting with what waits for the person reading them.
fn status_human(r: &Value, now_ms: u64) -> String {
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
    if let Some(line) = claude_line(&r["claude"], now_ms, crate::platform::utc_offset_minutes()) {
        lines.push(line);
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
    if let Some(mut limits) = limits_object(&value) {
        limits.insert("seen_at".into(), json!(crate::runtime::now_ms()));
        if std::fs::create_dir_all(state).is_ok()
            && let Ok(bytes) = serde_json::to_vec(&Value::Object(limits))
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
        let text = status_human(&r, 1_762_550_000_000);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "1 to review: fix-1; 1 blocked: pane");
        assert!(text.contains("agents: pane claude-code blocked"), "{text}");
        assert!(text.contains("workers: next opencode Go"), "{text}");
        assert!(text.contains("claude: 5H resets "), "{text}");
        let none = status_human(
            &json!({
                "agents": [], "tasks": {"review": [], "running": [], "queued": [], "recent": []},
                "workers": {"workers": [], "next": null, "all_out": "every worker is out"}, "claude": null,
                "waiting": {"review": 0, "blocked": 0}
            }),
            1_762_550_000_000,
        );
        assert_eq!(none.lines().next(), Some("Nothing waits for you"));
        assert!(!none.contains("claude:"), "no limits, no line: {none}");
    }

    fn at(day: u32, hour: u32, minute: u32) -> u64 {
        crate::task::clock::to_ms(
            &crate::task::clock::Civil { year: 2026, month: 11, day, hour, minute, second: 0, weekday: 0 },
            0,
        ) / 1000
    }

    #[test]
    fn claude_percentages_are_kept_and_shown() {
        let value = json!({"rate_limits": {
            "five_hour": {"resets_at": at(5, 18, 40), "used_percentage": 42.4},
            "seven_day": {"resets_at": at(7, 12, 0), "used_percentage": 61.0}
        }});
        let limits = limits_object(&value).unwrap();
        assert_eq!(limits["five_hour_used_percentage"], json!(42.4));
        assert_eq!(limits["seven_day_used_percentage"], json!(61.0));
        let line = claude_line(&json!(limits), at(5, 9, 0) * 1000, 0).unwrap();
        assert_eq!(line, "claude: 5H 42% · resets 18:40 · Week 61% · resets Sat");
    }

    #[test]
    fn a_window_at_a_hundred_percent_is_exhausted() {
        let value = json!({"rate_limits": {
            "five_hour": {"resets_at": at(5, 18, 40), "used_percentage": 100.0},
            "seven_day": {"resets_at": at(7, 12, 0), "used_percentage": 99.6}
        }});
        let limits = limits_object(&value).unwrap();
        let line = claude_line(&json!(limits), at(5, 9, 0) * 1000, 0).unwrap();
        assert_eq!(line, "claude: 5H exhausted · resets 18:40 · Week 99% · resets Sat");
    }

    #[test]
    fn a_window_without_a_percentage_says_only_its_reset() {
        let value = json!({"five_hour_resets_at": at(5, 18, 40), "seven_day_resets_at": null});
        assert_eq!(claude_line(&value, at(5, 9, 0) * 1000, 0).unwrap(), "claude: 5H resets 18:40");
        assert_eq!(claude_line(&json!({}), at(5, 9, 0) * 1000, 0), None, "no limits, no line");
    }

    #[test]
    fn the_saved_percentages_reach_the_claude_object() {
        let state = std::env::temp_dir().join(format!("kom-status-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&state);
        std::fs::create_dir_all(&state).unwrap();
        std::fs::write(
            state.join("claude-limits.json"),
            json!({"five_hour_resets_at": at(5, 18, 40), "seven_day_resets_at": null,
                   "five_hour_used_percentage": 42.4, "seen_at": 7})
            .to_string(),
        )
        .unwrap();
        let limits = claude_limits(&state);
        assert_eq!(limits["five_hour_used_percentage"], json!(42.4));
        assert!(limits["seven_day_used_percentage"].is_null(), "left out when Claude sent none: {limits}");
        assert_eq!(limits["five_hour_resets_at"], json!(at(5, 18, 40)));
        let _ = std::fs::remove_dir_all(&state);
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
