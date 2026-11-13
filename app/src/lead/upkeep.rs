//! What the lead's status line says, when the lead may be compacted, and when its program is lost:
//! the deciding code behind the window's upkeep of the lead.

use super::{AUTO_COMPACT_WINDOW, Reveal, Session, State, TerminalVis, is_compact_command, is_lead_record};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::Path;

/// The model and usage a status line JSON says, as `Session` keeps them; `seen_ms` is when it arrived.
/// Each field is read defensively: a missing one is null, never an error.
pub fn status_line_facts(input: &Value, seen_ms: u64) -> (Option<Value>, Value) {
    let model = input["model"]["display_name"]
        .as_str()
        .filter(|name| !name.trim().is_empty())
        .map(|name| json!({ "id": input["model"]["id"].as_str(), "name": name }));
    let window = &input["context_window"];
    let size = whole(&window["context_window_size"]);
    // Claude says 0 before its first answer and after a compaction: the size isn't known then.
    let context_tokens = whole(&window["total_input_tokens"]).filter(|tokens| *tokens > 0).or_else(|| {
        let percent = window["used_percentage"].as_f64()?;
        Some((percent * window["context_window_size"].as_f64()? / 100.0).round() as u64).filter(|tokens| *tokens > 0)
    });
    let usage = json!({
        "context_tokens": context_tokens,
        "context_window": size,
        "compact_at": AUTO_COMPACT_WINDOW,
        "cache_ttl": input["prompt_cache"]["ttl"].as_str(),
        "cache_expires_at": whole(&input["prompt_cache"]["expires_at"]),
        "five_hour": limit_facts(&input["rate_limits"]["five_hour"]),
        "seven_day": limit_facts(&input["rate_limits"]["seven_day"]),
        "session_id": input["session_id"].as_str(),
        "seen_ms": seen_ms,
    });
    (model, usage)
}

/// One of Claude's limit windows as `{ used_percentage, resets_at }`, or null when the window is missing.
fn limit_facts(window: &Value) -> Value {
    if !window.is_object() {
        return Value::Null;
    }
    json!({
        "used_percentage": window["used_percentage"].as_f64(),
        "resets_at": whole(&window["resets_at"]),
    })
}

/// A count or a time that JSON may write as `271000` or `271000.0`.
fn whole(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_f64().filter(|n| n.is_finite() && *n >= 0.0).map(|n| n.round() as u64))
}

/// The one text the app itself sends the lead: Claude's own command to compact its conversation
/// (the Owner's one exception to "only the Owner's input reaches the lead", 2026-11-12).
pub const COMPACT_COMMAND: &str = "/compact";

/// Why the lead can't be told to compact now, or None when it can: it has to be waiting for the
/// Owner, with no card open and no compaction running.
pub fn compact_refusal(session: Option<&Session>) -> Option<&'static str> {
    let Some(session) = session else {
        return Some("the lead hasn't started yet");
    };
    if session.compacting {
        return Some("Claude is already tidying up");
    }
    if session.conversation.reveal == Some(Reveal::Auto) {
        return Some("Claude is showing a menu in its terminal: answer it first");
    }
    if session.ask.is_some() {
        return Some("Claude is asking something: answer it first");
    }
    match session.state {
        Some(State::Waiting) => None,
        Some(State::Asking) => Some("Claude is asking something: answer it first"),
        Some(State::Working) => Some("Claude is working: it can tidy up once it has finished"),
        None => Some("the lead hasn't started yet"),
    }
}

/// Why the lead can't be restarted by hand right now: a restart ends whatever it is doing.
pub fn restart_refusal(session: Option<&Session>) -> Option<&'static str> {
    let session = session?;
    if session.ask.is_some() {
        return Some("The lead is asking something: answer it first");
    }
    if session.compacting {
        return Some("The lead is tidying up: restart it when it's done");
    }
    (session.state == Some(State::Working)).then_some("The lead is working: restart it when it's done")
}

/// Folds a hook's `permission_mode` into the session. Returns whether what mods see changed.
pub fn apply_permission_mode(session: &mut Session, input: &Value) -> bool {
    let Some(mode) = input.get("permission_mode").and_then(Value::as_str) else {
        return false;
    };
    session.modes_seen.insert(mode.to_owned());
    let changed = session.permission_mode.as_deref() != Some(mode) || session.mode_guessed;
    session.permission_mode = Some(mode.to_owned());
    session.mode_guessed = false;
    changed
}

/// The mode Claude Code shows after one Shift+Tab from `current`. Optional modes count only once seen.
pub fn next_mode(current: Option<&str>, seen: &BTreeSet<String>) -> &'static str {
    match current {
        None | Some("default") => "acceptEdits",
        Some("acceptEdits") => "plan",
        Some("plan") if seen.contains("bypassPermissions") => "bypassPermissions",
        Some("plan") if seen.contains("auto") => "auto",
        Some("bypassPermissions") if seen.contains("auto") => "auto",
        _ => "default",
    }
}

/// Why the mode can't be switched now; None when it can (working is fine).
pub fn mode_refusal(session: Option<&Session>) -> Option<&'static str> {
    let Some(session) = session else {
        return Some("the lead hasn't started yet");
    };
    if !session.started {
        return Some("the lead hasn't started yet");
    }
    if session.ask.is_some() {
        return Some("Claude is asking something: answer it first");
    }
    if session.conversation.reveal == Some(Reveal::Auto) {
        return Some("Claude is showing a menu in its terminal: answer it first");
    }
    if session.compacting {
        return Some("Claude is tidying up: switch when it's done");
    }
    None
}

/// The app compacts a waiting lead by itself from this many tokens, once it has waited `COMPACT_IDLE_MS`.
pub const COMPACT_HIGH: u64 = 333_000;
/// From this many tokens it compacts just before Claude's prompt cache goes cold.
pub const COMPACT_LOW: u64 = 222_000;
pub const COMPACT_IDLE_MS: u64 = 10 * 60 * 1000;
/// How long before the cache's expiry the app compacts, in seconds.
pub const COMPACT_CACHE_MARGIN_S: u64 = 5 * 60;
/// The wait that stands in for the cache's expiry when Claude hasn't said it.
pub const COMPACT_IDLE_NO_CACHE_MS: u64 = 55 * 60 * 1000;
/// A compaction with no end after this long is taken as over.
pub const COMPACTING_LIMIT_MS: u64 = 10 * 60 * 1000;

/// Which rule makes a waiting lead due for `/compact` now, or None: "high" (rule A, from
/// `COMPACT_HIGH` tokens) or "cache" (rule B, from `COMPACT_LOW` tokens just before the cache expires,
/// or after the idle wait when Claude names no expiry). `draft` is true while the Owner has text or a
/// file waiting in the composer.
pub fn auto_compact_rule(session: &Session, now_ms: u64, draft: bool) -> Option<&'static str> {
    if !session.started
        || session.state != Some(State::Waiting)
        || session.ask.is_some()
        || session.compacting
        || session.auto_compact_tried
        || session.conversation.terminal == TerminalVis::Shown
        || draft
    {
        return None;
    }
    let usage = session.usage.as_ref()?;
    let tokens = usage["context_tokens"].as_u64()?;
    let waited = now_ms.saturating_sub(session.waiting_since_ms?);
    if waited < COMPACT_IDLE_MS {
        return None;
    }
    if tokens >= COMPACT_HIGH {
        return Some("high");
    }
    if tokens < COMPACT_LOW {
        return None;
    }
    match usage["cache_expires_at"].as_u64() {
        Some(expires) => {
            let now_s = now_ms / 1000;
            (now_s + COMPACT_CACHE_MARGIN_S >= expires && now_s < expires).then_some("cache")
        }
        None => (waited >= COMPACT_IDLE_NO_CACHE_MS).then_some("cache"),
    }
}

/// Whether the app should send `/compact` to a waiting lead now (see `auto_compact_rule`).
pub fn auto_compact_due(session: &Session, now_ms: u64, draft: bool) -> bool {
    auto_compact_rule(session, now_ms, draft).is_some()
}

/// Whether the core's `status --json` shows no running, queued or review task. What it can't read
/// (no `tasks` object, or not JSON at all) counts as busy: the app does not compact on a guess.
pub fn tasks_busy(status: &Value) -> bool {
    let Some(tasks) = status.get("tasks").and_then(Value::as_object) else {
        return true;
    };
    ["running", "queued", "review"].iter().any(|key| match tasks.get(*key).and_then(Value::as_array) {
        Some(list) => !list.is_empty(),
        None => true,
    })
}

/// A compaction that has run longer than `COMPACTING_LIMIT_MS` with no end: its flag is stuck.
pub fn compaction_overdue(session: &Session, now_ms: u64) -> bool {
    session.compacting
        && session.compacting_since_ms.is_some_and(|since| now_ms.saturating_sub(since) > COMPACTING_LIMIT_MS)
}

/// Whether a terminal record is the lead's but its program is gone: the core restarted and started a
/// plain shell in the lead's terminal, so neither what it started with nor what runs in it now names
/// the program's run word. A program the Owner started again by hand in that shell is not lost.
pub fn lead_program_lost(record: &Value, program: &str, run_word: &str) -> bool {
    if run_word.is_empty() || !is_lead_record(record, program) || !record["pid"].is_number() {
        return false;
    }
    let Some(argv) = record.get("argv").and_then(Value::as_array).filter(|argv| !argv.is_empty()) else {
        return false;
    };
    let word = run_word.to_lowercase();
    let names = |value: &Value| value.as_str().is_some_and(|text| text.to_lowercase().contains(&word));
    let running = &record["foreground"];
    !argv.iter().any(names)
        && !names(&running["name"])
        && !running["argv"].as_array().is_some_and(|argv| argv.iter().any(names))
}

/// The lead's state after a hook event, or None when the event doesn't change it. `/compact` is not
/// a turn, and a compaction the Owner or the app asked for ends with Claude waiting again.
pub fn state_after_hook(event: &str, input: &Value) -> Option<State> {
    match event {
        "UserPromptSubmit" if input["prompt"].as_str().is_some_and(is_compact_command) => None,
        "UserPromptSubmit" => Some(State::Working),
        "Stop" | "Notification" => Some(State::Waiting),
        "PostCompact" if input["trigger"] == "manual" => Some(State::Waiting),
        _ => None,
    }
}

/// Folds a status line into the session. Returns whether what mods see changed: the model, or the
/// usage without its arrival time (`seen_ms` is kept fresh either way).
pub fn apply_status_line(session: &mut Session, input: &Value, seen_ms: u64) -> bool {
    let (model, usage) = status_line_facts(input, seen_ms);
    let same_usage = session
        .usage
        .as_ref()
        .is_some_and(|old| without_seen(old) == without_seen(&usage));
    let changed = session.model != model || !same_usage;
    session.model = model;
    session.usage = Some(usage);
    changed
}

fn without_seen(usage: &Value) -> Value {
    let mut usage = usage.clone();
    if let Some(map) = usage.as_object_mut() {
        map.remove("seen_ms");
    }
    usage
}

/// Claude's compaction is over: its context size is unknown until its next answer.
pub fn compaction_done(session: &mut Session) {
    session.set_compacting(false, 0);
    if let Some(usage) = session.usage.as_mut() {
        usage["context_tokens"] = Value::Null;
    }
}

/// Hands Claude's status JSON to the core, so `kom status` keeps its limits. It runs headless on its own
/// thread; a failure is ignored, as the status line itself must never fail.
pub fn send_status_to_core(input: &Value) {
    let text = input.to_string();
    let core = crate::core_bin::path();
    std::thread::spawn(move || {
        let Ok(mut child) = crate::spawn::command(core)
            .args(["status", "--line"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        else {
            return;
        };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = std::io::Write::write_all(&mut stdin, text.as_bytes());
        }
        let _ = child.wait();
    });
}

/// `kom status --json`, run headless: its stdout, or None when the core could not run or failed.
pub fn core_status_text(core: &Path) -> Option<String> {
    let output = crate::spawn::command(core)
        .args(["status", "--json"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
#[path = "upkeep/tests.rs"]
mod tests;
