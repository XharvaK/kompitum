//! Claude Code's stream (`--output-format stream-json --verbose`): one JSON
//! object per line. A system init names the session, each assistant message is
//! one step with the prompt tokens it sent, and a result line closes the turn
//! with the turn's own totals.

use crate::task::decide::{TurnEnd, session_end};
use crate::task::quota;
use crate::task::types::{Task, TaskState, cut};
use crate::task::worker::{
    Progress, REASONING_TAIL, TurnOutcome, end_bytes, part_text, push_list, push_read, push_reasoning, push_tool,
    text_at, whole,
};
use serde_json::{Map, Value};

/// The model name Claude Code gives the messages it writes itself, such as a usage limit.
const SYNTHETIC: &str = "<synthetic>";

fn add_session(out: &mut TurnOutcome, id: &str) {
    if !out.sessions.iter().any(|seen| seen == id) {
        out.sessions.push(id.to_string());
    }
    if out.session.is_none() {
        out.session = Some(id.to_string());
    }
}

fn tokens(usage: Option<&Map<String, Value>>, key: &str) -> u64 {
    whole(usage.and_then(|usage| usage.get(key)))
}

/// Everything one Claude Code turn's stream says.
pub(crate) fn read(lines: impl IntoIterator<Item = String>) -> TurnOutcome {
    let mut out = TurnOutcome::default();
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            out.parse_errors += 1;
            continue;
        };
        if !value.is_object() {
            out.parse_errors += 1;
            continue;
        }
        match text_at(&value, "type").unwrap_or("?") {
            "system" if text_at(&value, "subtype") == Some("init") => {
                if let Some(id) = text_at(&value, "session_id") {
                    add_session(&mut out, id);
                }
            }
            "assistant" => assistant(&mut out, &value),
            "result" => result(&mut out, &value),
            _ => {}
        }
    }
    out.reasoning_tail = end_bytes(&out.reasoning_tail, REASONING_TAIL);
    out
}

fn assistant(out: &mut TurnOutcome, value: &Value) {
    let Some(message) = value.get("message").and_then(Value::as_object) else { return };
    out.usage.steps += 1;
    let usage = message.get("usage").and_then(Value::as_object);
    let prompt = tokens(usage, "input_tokens")
        .saturating_add(tokens(usage, "cache_read_input_tokens"))
        .saturating_add(tokens(usage, "cache_creation_input_tokens"));
    out.usage.largest_request = out.usage.largest_request.max(prompt);
    let synthetic = message.get("model").and_then(Value::as_str) == Some(SYNTHETIC);
    for block in message.get("content").and_then(Value::as_array).into_iter().flatten() {
        let Some(part) = block.as_object() else { continue };
        match part_text(part, "type") {
            Some("text") => {
                if let Some(text) = part_text(part, "text") {
                    out.text = text.to_string();
                    if synthetic {
                        out.errors.push(cut(text.trim(), 300));
                    }
                }
            }
            Some("tool_use") => tool_use(out, part),
            Some("thinking") => {
                push_reasoning(&mut out.reasoning_tail, part_text(part, "thinking").unwrap_or_default())
            }
            _ => {}
        }
    }
}

/// A tool's name, and for the Read tool the file it read.
fn tool_use(out: &mut TurnOutcome, part: &Map<String, Value>) {
    let name = part_text(part, "name").unwrap_or("tool");
    push_list(&mut out.tools, cut(name, 120));
    if name == "Read"
        && let Some(path) = part.get("input").and_then(Value::as_object).and_then(|input| part_text(input, "file_path"))
    {
        push_read(&mut out.files_read, path);
    }
}

/// The closing result: the final text, the turn's totals, and whether it failed.
fn result(out: &mut TurnOutcome, value: &Value) {
    if let Some(id) = text_at(value, "session_id") {
        add_session(out, id);
    }
    if let Some(text) = text_at(value, "result") {
        out.text = text.to_string();
    }
    let usage = value.get("usage").and_then(Value::as_object);
    out.usage.input = tokens(usage, "input_tokens");
    out.usage.output = tokens(usage, "output_tokens");
    out.usage.cache_read = tokens(usage, "cache_read_input_tokens");
    out.usage.cache_write = tokens(usage, "cache_creation_input_tokens");
    out.usage.cost_usd = value.get("total_cost_usd").and_then(Value::as_f64).unwrap_or(0.0);
    if value.get("is_error").and_then(Value::as_bool) == Some(true) {
        let words = text_at(value, "result").or_else(|| text_at(value, "subtype")).unwrap_or("error");
        out.errors.push(cut(words.trim(), 300));
    }
}

/// One output line in, one line for the screen out (or nothing to show).
pub(crate) fn progress(progress: &mut Progress, line: &str) -> Option<String> {
    let value: Value = serde_json::from_str(line.trim()).ok()?;
    if text_at(&value, "type") != Some("assistant") {
        return None;
    }
    progress.steps += 1;
    let mut shown = Vec::new();
    for block in value.get("message")?.get("content")?.as_array()? {
        let Some(part) = block.as_object() else { continue };
        if part_text(part, "type") != Some("tool_use") {
            continue;
        }
        let name = cut(part_text(part, "name").unwrap_or("tool"), 120);
        push_tool(progress, name.clone());
        shown.push(name);
    }
    (!shown.is_empty()).then(|| cut(&shown.join(", "), 120))
}

fn failed(message: impl Into<String>) -> TurnEnd {
    TurnEnd { state: TaskState::Failed, error: Some(message.into()), refusal: None }
}

/// How a Claude Code turn ended: the order `codex_end` uses, with Claude's own refusal words.
pub(crate) fn end(task: &Task, code: Option<i32>, outcome: &TurnOutcome, now_ms: u64, offset_min: i32) -> TurnEnd {
    // Only a turn that went wrong can be a refusal: a limit named in a reply must not
    // block a worker that finished its work.
    let went_wrong = !outcome.errors.is_empty() || code != Some(0) || outcome.text.is_empty();
    if went_wrong && let Some(refusal) = quota::claude_refusal(&outcome.errors, now_ms, offset_min) {
        return TurnEnd { state: TaskState::Failed, error: Some(refusal.message.clone()), refusal: Some(refusal) };
    }
    if !outcome.errors.is_empty() {
        return failed(format!("worker reported an error event: {}", outcome.errors[0]));
    }
    if code != Some(0) {
        let words = match code {
            Some(code) => format!("worker exited with code {code}"),
            None => "worker exited without an exit code".into(),
        };
        return failed(words);
    }
    if outcome.parse_errors > 0 {
        return failed(format!("worker stream contained {} unparseable lines", outcome.parse_errors));
    }
    if let Some(end) = session_end(task, outcome) {
        return end;
    }
    if outcome.text.is_empty() {
        return failed("worker produced no final text");
    }
    TurnEnd { state: TaskState::Done, error: None, refusal: None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::types::Turn;
    use crate::task::worker::Cli;
    use serde_json::json;

    const SESSION: &str = "c3d9a1e2-5f7b-4e8a-9d2c-1a2b3c4d5e6f";
    /// Two hours before the reset (1793000000 s) that the usage-limit fixture names.
    const NOW: u64 = 1_792_992_800_000;

    fn line(value: Value) -> String {
        value.to_string()
    }

    /// A whole turn: init, a step that reads a file and says half of it, a step
    /// that says the rest, and the success result with the turn's totals.
    fn turn() -> Vec<String> {
        vec![
            line(json!({"type": "system", "subtype": "init", "session_id": SESSION, "model": "claude-haiku-5-5"})),
            line(json!({"type": "assistant", "session_id": SESSION, "message": {"model": "claude-haiku-5-5",
                "content": [
                    {"type": "thinking", "thinking": "Reading the brief"},
                    {"type": "tool_use", "id": "t1", "name": "Read", "input": {"file_path": "src/lib.rs"}},
                    {"type": "text", "text": "Half way"}],
                "usage": {"input_tokens": 3, "cache_read_input_tokens": 900, "cache_creation_input_tokens": 100,
                    "output_tokens": 20}}})),
            line(json!({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": "t1",
                "content": "ok"}]}})),
            line(json!({"type": "assistant", "session_id": SESSION, "message": {"model": "claude-haiku-5-5",
                "content": [{"type": "text", "text": "All done"}],
                "usage": {"input_tokens": 4, "cache_read_input_tokens": 1000, "cache_creation_input_tokens": 0,
                    "output_tokens": 5}}})),
            line(json!({"type": "result", "subtype": "success", "is_error": false, "result": "All done.",
                "session_id": SESSION, "total_cost_usd": 0.0123, "num_turns": 2,
                "usage": {"input_tokens": 7, "output_tokens": 25, "cache_read_input_tokens": 1900,
                    "cache_creation_input_tokens": 100}})),
        ]
    }

    fn limit_message(text: &str) -> String {
        line(json!({"type": "assistant", "session_id": SESSION, "message": {"model": SYNTHETIC,
            "content": [{"type": "text", "text": text}], "usage": {"input_tokens": 0}}}))
    }

    #[test]
    fn a_full_turn_reads_as_one_session_with_its_text_steps_tools_and_totals() {
        let outcome = read(turn());
        assert_eq!(outcome.session.as_deref(), Some(SESSION));
        assert_eq!(outcome.sessions, [SESSION], "the init and the result name the same session");
        assert_eq!(outcome.text, "All done.", "the result's text is the final text");
        assert_eq!(outcome.usage.steps, 2, "one per assistant message");
        assert_eq!(outcome.tools, ["Read"]);
        assert_eq!(outcome.files_read, ["src/lib.rs"]);
        assert_eq!(outcome.reasoning_tail, "Reading the brief");
        assert_eq!(outcome.usage.input, 7);
        assert_eq!(outcome.usage.output, 25);
        assert_eq!(outcome.usage.cache_read, 1900);
        assert_eq!(outcome.usage.cache_write, 100);
        assert_eq!(outcome.usage.cost_usd, 0.0123);
        assert_eq!(outcome.usage.largest_request, 1004, "the biggest prompt any step sent");
        assert!(outcome.errors.is_empty());
        assert_eq!(outcome.parse_errors, 0);
    }

    #[test]
    fn an_error_result_lands_in_errors_and_a_result_without_text_names_its_subtype() {
        let failed = line(json!({"type": "result", "subtype": "error_during_execution", "is_error": true,
            "result": "Prompt is too long", "session_id": SESSION}));
        assert_eq!(read(vec![failed]).errors, ["Prompt is too long"]);
        let silent = line(json!({"type": "result", "subtype": "error_max_turns", "is_error": true,
            "result": "", "session_id": SESSION}));
        assert_eq!(read(vec![silent]).errors, ["error_max_turns"]);
    }

    #[test]
    fn claudes_own_limit_message_lands_in_errors() {
        let outcome = read(vec![limit_message("Claude AI usage limit reached|1793000000")]);
        assert_eq!(outcome.errors, ["Claude AI usage limit reached|1793000000"]);
        assert_eq!(outcome.text, "Claude AI usage limit reached|1793000000");
    }

    #[test]
    fn a_junk_line_counts_as_a_parse_error() {
        let outcome = read(vec!["not json".to_string(), "[1, 2]".to_string(), String::new()]);
        assert_eq!(outcome.parse_errors, 2, "a non-object line counts; a blank one does not");
    }

    #[test]
    fn a_step_shows_its_tools_and_a_text_only_step_shows_nothing() {
        let mut progress = Progress::new(Cli::Claude, None);
        let shown: Vec<_> = turn().iter().filter_map(|line| progress.feed(line)).collect();
        assert_eq!(shown, ["Read"]);
        assert_eq!(progress.steps, 2);
        assert_eq!(progress.tools, ["Read"]);
    }

    fn haiku_task() -> Task {
        Task {
            turns: vec![Turn { worker: "haiku".into(), cli: Some(Cli::Claude), ..Default::default() }],
            ..Default::default()
        }
    }

    #[test]
    fn a_clean_turn_with_text_and_a_session_is_done() {
        let end = end(&haiku_task(), Some(0), &read(turn()), NOW, 0);
        assert_eq!((end.state, end.error, end.refusal), (TaskState::Done, None, None));
    }

    #[test]
    fn a_usage_limit_error_is_a_refusal_that_names_its_reset() {
        let outcome = read(vec![limit_message("Claude AI usage limit reached|1793000000")]);
        let end = end(&haiku_task(), Some(1), &outcome, NOW, 0);
        assert_eq!(end.state, TaskState::Failed);
        let refusal = end.refusal.expect("a refusal");
        assert_eq!((refusal.until_ms, refusal.source), (1_793_000_000_000, quota::Source::Named));
        assert_eq!(refusal.window, "5-hour", "the reset is two hours away");
    }
}
