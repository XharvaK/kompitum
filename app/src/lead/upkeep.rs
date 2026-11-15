//! What the lead's status line says, when the lead may be compacted, and when its program is lost:
//! the deciding code behind the window's upkeep of the lead.

use super::{AUTO_COMPACT_WINDOW, Reveal, Session, State, TerminalVis, is_compact_command, is_lead_record};
use crate::strings;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

/// A core that doesn't answer its status is wedged; the lead's next look may try again.
const STATUS_LIMIT: Duration = Duration::from_secs(30);

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

/// A refusal's words, kept for the `&'static str` its callers and tests use.
fn word(text: String) -> &'static str {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};
    static WORDS: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let mut words = WORDS.get_or_init(|| Mutex::new(HashSet::new())).lock().unwrap();
    if let Some(word) = words.get(text.as_str()) {
        return word;
    }
    let word: &'static str = Box::leak(text.into_boxed_str());
    words.insert(word);
    word
}

/// Why the lead can't take a change now, or None when it can: it has to be waiting for the
/// Owner, with no card open and no compaction running. `compacting` is the text that differs.
fn refusal(session: Option<&Session>, compacting: &str) -> Option<&'static str> {
    let Some(session) = session else {
        return Some(word(strings::text("lead.not-started")));
    };
    if session.compacting {
        return Some(word(compacting.to_owned()));
    }
    if session.conversation.reveal == Some(Reveal::Auto) {
        return Some(word(strings::text("lead.menu-shown")));
    }
    if session.ask.is_some() {
        return Some(word(strings::text("lead.asking")));
    }
    match session.state {
        Some(State::Waiting) => None,
        Some(State::Asking) => Some(word(strings::text("lead.asking"))),
        Some(State::Working) => Some(word(strings::text("lead.working-compact"))),
        None => Some(word(strings::text("lead.not-started"))),
    }
}

/// Why the lead can't be told to compact now, or None when it can: it has to be waiting for the
/// Owner, with no card open and no compaction running.
pub fn compact_refusal(session: Option<&Session>) -> Option<&'static str> {
    refusal(session, &strings::text("lead.compacting"))
}

/// Why the lead can't take a model or reasoning change now, or None when it can: the same
/// conditions as a compaction, but a tidy-up only delays the change.
pub fn apply_refusal(session: Option<&Session>) -> Option<&'static str> {
    refusal(session, &strings::text("lead.tidying-change"))
}

/// The commands a picked model and reasoning need: Claude Code's `/model` and `/effort`, each only
/// when the saved value differs from what the lead runs.
pub fn apply_commands(
    running_model: Option<&str>,
    running_effort: Option<&str>,
    saved_model: Option<&str>,
    saved_effort: Option<&str>,
) -> Vec<String> {
    let mut commands = Vec::new();
    if let Some(model) = saved_model.filter(|model| Some(*model) != running_model) {
        commands.push(format!("/model {model}"));
    }
    if let Some(effort) = saved_effort.filter(|effort| Some(*effort) != running_effort) {
        commands.push(format!("/effort {effort}"));
    }
    commands
}

/// Why the lead can't be restarted by hand right now: a restart ends whatever it is doing.
pub fn restart_refusal(session: Option<&Session>) -> Option<&'static str> {
    let session = session?;
    if session.ask.is_some() {
        return Some(word(strings::text("lead.restart-asking")));
    }
    if session.compacting {
        return Some(word(strings::text("lead.restart-tidying")));
    }
    (session.state == Some(State::Working)).then(|| word(strings::text("lead.restart-working")))
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

/// The mode a lead's screen shows, from the last 8 non-empty lines: Claude Code prints its current
/// permission mode there, and a newer line wins over an older one above it.
pub fn mode_from_screen(text: &str) -> Option<&'static str> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .rev()
        .take(8)
        .find_map(|line| {
            if line.contains("manual mode on") {
                Some("default")
            } else if line.contains("accept edits on") {
                Some("acceptEdits")
            } else if line.contains("plan mode on") {
                Some("plan")
            } else if line.contains("auto mode on") {
                Some("auto")
            } else if line.contains("bypass permissions on") {
                Some("bypassPermissions")
            } else {
                None
            }
        })
}

/// The word the interface gives a permission mode.
pub fn mode_word(mode: &str) -> String {
    match mode {
        "default" => strings::text("composer.mode-ask-first"),
        "acceptEdits" => strings::text("composer.mode-accept-edits"),
        "plan" => strings::text("composer.mode-plan-first"),
        "auto" => strings::text("composer.mode-auto"),
        "bypassPermissions" => strings::text("composer.mode-full-access"),
        "dontAsk" => strings::text("composer.mode-dont-ask"),
        _ => mode.to_owned(),
    }
}

/// The error toast a mode switch that ran out of presses shows.
pub fn mode_not_taken(target: &str, current: Option<&str>) -> Value {
    let current = current.filter(|mode| !mode.is_empty()).unwrap_or(target);
    json!({
        "title": strings::text_with("lead.mode-not-taken", &[("mode", &mode_word(target))]),
        "body": strings::text_with("lead.mode-stays", &[("current", &mode_word(current))]),
        "level": "error",
    })
}

/// The most presses `lead.set_mode` spends getting to its target.
pub const MODE_PRESSES: usize = 6;
/// How long Claude Code is given to show its new mode line after Shift+Tab.
pub const MODE_WAIT_MS: u64 = 300;
/// How long after its SessionStart the lead's first screen is given to draw its mode line.
pub const START_MODE_WAIT_MS: u64 = 1500;

/// What a mode switch read and whether it reached its target.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModeSwitch {
    /// The mode read last, or the mode it started from when no read answered.
    pub mode: Option<String>,
    /// Whether the last read (or the start) was the target.
    pub reached: bool,
    /// Whether any read answered at all.
    pub read_any: bool,
}

/// Presses Shift+Tab and reads the mode after each press until `target` is read, at most
/// `MODE_PRESSES` times. An already-current target presses nothing. A read with no answer keeps
/// the mode it had.
pub fn switch_mode(
    target: &str,
    current: Option<String>,
    mut press_and_read: impl FnMut() -> Option<String>,
) -> ModeSwitch {
    let mut switch = ModeSwitch {
        mode: current,
        reached: false,
        read_any: false,
    };
    if switch.mode.as_deref() == Some(target) {
        switch.reached = true;
        return switch;
    }
    for _ in 0..MODE_PRESSES {
        if let Some(mode) = press_and_read() {
            switch.mode = Some(mode);
            switch.read_any = true;
        }
        if switch.mode.as_deref() == Some(target) {
            switch.reached = true;
            return switch;
        }
    }
    switch
}

/// Why the mode can't be switched now; None when it can (working is fine).
pub fn mode_refusal(session: Option<&Session>) -> Option<&'static str> {
    let Some(session) = session else {
        return Some(word(strings::text("lead.not-started")));
    };
    if !session.started {
        return Some(word(strings::text("lead.not-started")));
    }
    if session.ask.is_some() {
        return Some(word(strings::text("lead.asking")));
    }
    if session.conversation.reveal == Some(Reveal::Auto) {
        return Some(word(strings::text("lead.menu-shown")));
    }
    if session.compacting {
        return Some(word(strings::text("lead.mode-tidying")));
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
        // Code Mode's background output is not a turn the Owner started.
        "UserPromptSubmit" if input["source"] == "poll_event" => None,
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
    let mut command = crate::spawn::command(core);
    command.args(["status", "--json"]);
    let (output, timed_out) = crate::spawn::output_within(command, STATUS_LIMIT).ok()?;
    if timed_out {
        tracing::warn!(
            "kom status --json took longer than {} s and was stopped",
            STATUS_LIMIT.as_secs()
        );
        return None;
    }
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

// ---- measuring a lead turn ------------------------------------------------

/// A lead turn from its Owner item to its `Stop` or `StopFailure`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Turn {
    pub started_ms: u64,
    /// Tool calls the turn made, not counting `REPL`.
    pub tool_calls: u64,
    /// `REPL` calls: each ran code that made its own tool calls.
    pub code_runs: u64,
    /// Tool ids already counted, so a repeated `PreToolUse` never counts twice.
    seen: BTreeSet<String>,
}

impl Turn {
    pub fn new(started_ms: u64) -> Self {
        Self {
            started_ms,
            ..Self::default()
        }
    }

    /// One tool call: `REPL` is a code run, everything else a tool call.
    pub fn note_tool(&mut self, id: &str, name: &str) {
        if !id.is_empty() && !self.seen.insert(id.to_owned()) {
            return;
        }
        if name == "REPL" {
            self.code_runs += 1;
        } else {
            self.tool_calls += 1;
        }
    }
}

/// The tokens one turn's assistant messages reported.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TurnTokens {
    pub input: Option<u64>,
    pub output: Option<u64>,
    pub cache_read: Option<u64>,
    pub cache_write: Option<u64>,
}

/// Sums the usage of the transcript's last turn: the assistant messages after its last
/// Owner prompt. A transcript with no usage at all is `None` in every field.
pub fn turn_tokens(text: &str) -> TurnTokens {
    let mut total = TurnTokens::default();
    let mut saw = false;
    let mut seen = BTreeSet::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .or_else(|| value.pointer("/message/role").and_then(Value::as_str));
        match kind {
            Some("user") if is_prompt(&value) => {
                total = TurnTokens::default();
                saw = false;
                seen.clear();
            }
            Some("assistant") => {
                let Some(message) = value.get("message") else {
                    continue;
                };
                let Some(usage) = message.get("usage").and_then(Value::as_object) else {
                    continue;
                };
                if let Some(id) = message.get("id").and_then(Value::as_str).filter(|id| !id.is_empty())
                    && !seen.insert(id.to_owned())
                {
                    continue;
                }
                saw = true;
                for (key, slot) in [
                    ("input_tokens", &mut total.input),
                    ("output_tokens", &mut total.output),
                    ("cache_read_input_tokens", &mut total.cache_read),
                    ("cache_creation_input_tokens", &mut total.cache_write),
                ] {
                    let n = token_count(usage.get(key)).unwrap_or(0);
                    *slot = Some(slot.unwrap_or(0).saturating_add(n));
                }
            }
            _ => {}
        }
    }
    if !saw {
        return TurnTokens::default();
    }
    total
}

fn is_prompt(value: &Value) -> bool {
    if value.get("isCompactSummary") == Some(&json!(true)) || value.get("isMeta") == Some(&json!(true)) {
        return false;
    }
    let content = value
        .pointer("/message/content")
        .or_else(|| value.get("content"))
        .unwrap_or(&Value::Null);
    match content {
        Value::String(text) => !text.is_empty(),
        Value::Array(parts) => parts.iter().any(|part| {
            part.get("type").and_then(Value::as_str) == Some("text")
                && part.get("text").and_then(Value::as_str).is_some_and(|text| !text.is_empty())
        }),
        _ => false,
    }
}

fn token_count(value: Option<&Value>) -> Option<u64> {
    match value? {
        Value::Number(number) => number
            .as_u64()
            .or_else(|| number.as_i64().filter(|n| *n >= 0).map(|n| n as u64))
            .or_else(|| number.as_f64().filter(|n| *n >= 0.0).map(|n| n.round() as u64)),
        Value::String(text) => text.trim().parse::<u64>().ok().or_else(|| {
            text.trim()
                .parse::<f64>()
                .ok()
                .filter(|n| *n >= 0.0)
                .map(|n| n.round() as u64)
        }),
        _ => None,
    }
}

/// The one line a finished lead turn appends.
#[allow(clippy::too_many_arguments)]
pub fn turn_record(
    turn: &Turn,
    ended_ms: u64,
    session: &str,
    code_mode: bool,
    model: &str,
    ended_by: &str,
    tokens: TurnTokens,
) -> Value {
    json!({
        "ended": rfc3339(ended_ms),
        "session": session,
        "code_mode": code_mode,
        "model": model,
        "seconds": ended_ms.saturating_sub(turn.started_ms) as f64 / 1000.0,
        "tool_calls": turn.tool_calls,
        "code_runs": turn.code_runs,
        "input_tokens": tokens.input,
        "output_tokens": tokens.output,
        "cache_read_tokens": tokens.cache_read,
        "cache_write_tokens": tokens.cache_write,
        "ended_by": ended_by,
    })
}

/// Appends one turn's line to `lead-turns.jsonl` in `dir` (the app's home folder).
pub fn append_turn(dir: &Path, record: &Value) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    let path = dir.join("lead-turns.jsonl");
    let text = serde_json::to_string(record).map_err(|error| error.to_string())?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    writeln!(file, "{text}").map_err(|error| format!("{}: {error}", path.display()))
}

/// Milliseconds since the Unix epoch as RFC 3339 in UTC (`2026-11-15T12:34:56Z`).
pub fn rfc3339(ms: u64) -> String {
    let secs = ms / 1000;
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Howard Hinnant's `civil_from_days`: the date `days` after 1970-01-01.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (if month <= 2 { year + 1 } else { year }, month as u32, day as u32)
}

#[cfg(test)]
#[path = "upkeep/tests.rs"]
mod tests;
