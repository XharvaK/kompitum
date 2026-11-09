//! Codex's stream: one JSON object per line, a thread id at the start, one
//! item per step, and the thread's running usage at the end.

use crate::task::types::Usage;
use crate::task::worker::{
    Progress, REASONING_TAIL, TurnOutcome, end_bytes, push_list, push_reasoning, push_tool, text_at, whole,
};
use serde_json::Value;

fn add_session(out: &mut TurnOutcome, id: &str) {
    if !out.sessions.iter().any(|seen| seen == id) {
        out.sessions.push(id.to_string());
    }
    if out.session.is_none() {
        out.session = Some(id.to_string());
    }
}

fn item_of(value: &Value) -> Option<&serde_json::Map<String, Value>> {
    value.get("item").and_then(Value::as_object)
}

fn item_type(item: &serde_json::Map<String, Value>) -> Option<&str> {
    item.get("type").and_then(Value::as_str).filter(|kind| !kind.is_empty())
}

fn error_message(value: &Value) -> Option<&str> {
    value
        .get("error")
        .and_then(Value::as_object)
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .filter(|message| !message.is_empty())
}

/// Everything one Codex turn's stream says.
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
            "thread.started" => {
                if let Some(id) = text_at(&value, "thread_id") {
                    add_session(&mut out, id);
                }
            }
            "item.completed" => {
                let Some(item) = item_of(&value) else { continue };
                out.usage.steps += 1;
                match item_type(item) {
                    Some("agent_message") => {
                        if let Some(text) = item.get("text").and_then(Value::as_str).filter(|text| !text.is_empty()) {
                            out.text = text.to_string();
                        }
                    }
                    Some("reasoning") => {
                        if let Some(text) = item.get("text").and_then(Value::as_str) {
                            push_reasoning(&mut out.reasoning_tail, text);
                        }
                    }
                    // An item of type error is a warning, not an error.
                    Some("error") => {}
                    Some(kind @ ("command_execution" | "file_change" | "mcp_tool_call" | "web_search")) => {
                        push_list(&mut out.tools, crate::task::types::cut(kind, 120));
                    }
                    _ => {}
                }
            }
            "turn.completed" => {
                let usage = value.get("usage").and_then(Value::as_object);
                let input = whole(usage.and_then(|usage| usage.get("input_tokens")));
                let cache_read = whole(usage.and_then(|usage| usage.get("cached_input_tokens")));
                let cache_write = whole(usage.and_then(|usage| usage.get("cache_write_input_tokens")));
                out.usage.input = input.saturating_sub(cache_read);
                out.usage.cache_read = cache_read;
                out.usage.cache_write = cache_write;
                out.usage.output = whole(usage.and_then(|usage| usage.get("output_tokens")));
                out.usage.reasoning = whole(usage.and_then(|usage| usage.get("reasoning_output_tokens")));
                out.usage.largest_request = input.saturating_add(cache_write);
                out.usage.cost_usd = 0.0;
            }
            "turn.failed" => {
                if let Some(message) = error_message(&value) {
                    out.errors.push(crate::task::types::cut(message.trim(), 300));
                }
            }
            "error" => {
                if let Some(message) = text_at(&value, "message") {
                    out.errors.push(crate::task::types::cut(message.trim(), 300));
                }
            }
            _ => {}
        }
    }
    out.reasoning_tail = end_bytes(&out.reasoning_tail, REASONING_TAIL);
    out
}

/// One output line in, one line for the screen out (or nothing to show).
pub(crate) fn progress(progress: &mut Progress, line: &str) -> Option<String> {
    let value: Value = serde_json::from_str(line.trim()).ok()?;
    if text_at(&value, "type") != Some("item.completed") {
        return None;
    }
    let item = item_of(&value)?;
    progress.steps += 1;
    match item_type(item)? {
        kind @ ("command_execution" | "file_change") => {
            let shown = crate::task::types::cut(kind, 120);
            push_tool(progress, shown.clone());
            Some(shown)
        }
        _ => None,
    }
}

/// Codex reports a thread's running totals. A resumed turn keeps only what it
/// added: each reported field minus the earlier turns' recorded value. A field
/// that would go below zero keeps the number the turn reported. `steps` counts
/// this stream's completed items, not the thread's, so it stays as it is.
pub fn since_earlier(reported: &Usage, earlier: &Usage) -> Usage {
    fn after(reported: u64, earlier: u64) -> u64 {
        reported.checked_sub(earlier).unwrap_or(reported)
    }
    Usage {
        steps: reported.steps,
        input: after(reported.input, earlier.input),
        output: after(reported.output, earlier.output),
        reasoning: after(reported.reasoning, earlier.reasoning),
        cache_read: after(reported.cache_read, earlier.cache_read),
        cache_write: after(reported.cache_write, earlier.cache_write),
        cost_usd: if reported.cost_usd >= earlier.cost_usd {
            reported.cost_usd - earlier.cost_usd
        } else {
            reported.cost_usd
        },
        largest_request: after(reported.largest_request, earlier.largest_request),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const THREAD: &str = "0199a213-81c0-7800-8aa1-bbab2a035a53";

    fn line(value: Value) -> String {
        value.to_string()
    }

    /// The events from the brief, with one completed item of each tool kind.
    fn stream() -> Vec<String> {
        vec![
            line(serde_json::json!({"type": "thread.started", "thread_id": THREAD})),
            line(serde_json::json!({"type": "turn.started"})),
            line(serde_json::json!({"type": "item.started", "item": {"id": "item_1",
                "type": "command_execution", "command": "cargo test", "aggregated_output": "",
                "exit_code": null, "status": "in_progress"}})),
            line(serde_json::json!({"type": "item.completed", "item": {"id": "item_1",
                "type": "command_execution", "command": "cargo test", "aggregated_output": "ok",
                "exit_code": 0, "status": "completed"}})),
            line(serde_json::json!({"type": "item.completed", "item": {"id": "item_2",
                "type": "reasoning", "text": "Looking at the tests"}})),
            line(serde_json::json!({"type": "item.completed", "item": {"id": "item_3",
                "type": "agent_message", "text": "Repo contains docs, sdk, and examples directories."}})),
            line(serde_json::json!({"type": "item.completed", "item": {"id": "item_4",
                "type": "error", "message": "a warning, not fatal"}})),
            line(serde_json::json!({"type": "item.completed", "item": {"id": "item_5",
                "type": "file_change", "changes": [{"path": "src/a.rs", "kind": "update"}],
                "status": "completed"}})),
            line(serde_json::json!({"type": "item.completed", "item": {"id": "item_6",
                "type": "mcp_tool_call"}})),
            line(serde_json::json!({"type": "item.completed", "item": {"id": "item_7",
                "type": "web_search"}})),
            line(serde_json::json!({"type": "turn.completed", "usage": {"input_tokens": 24763,
                "cached_input_tokens": 24448, "cache_write_input_tokens": 0, "output_tokens": 122,
                "reasoning_output_tokens": 0}})),
            line(serde_json::json!({"type": "error", "message": "top-level warning"})),
            line(serde_json::json!({"type": "turn.failed", "error": {"message": "fatal"}})),
        ]
    }

    #[test]
    fn a_codex_turn_reads_its_thread_text_reasoning_tools_and_usage() {
        let outcome = read(stream());
        assert_eq!(outcome.session.as_deref(), Some(THREAD));
        assert_eq!(outcome.sessions, [THREAD]);
        assert_eq!(outcome.text, "Repo contains docs, sdk, and examples directories.");
        assert_eq!(outcome.reasoning_tail, "Looking at the tests");
        assert_eq!(outcome.tools, ["command_execution", "file_change", "mcp_tool_call", "web_search"]);
        assert_eq!(outcome.usage.steps, 7, "one per completed item, not the started one");
        assert_eq!(outcome.usage.input, 24763 - 24448);
        assert_eq!(outcome.usage.cache_read, 24448);
        assert_eq!(outcome.usage.cache_write, 0);
        assert_eq!(outcome.usage.output, 122);
        assert_eq!(outcome.usage.reasoning, 0);
        assert_eq!(outcome.usage.largest_request, 24763);
        assert_eq!(outcome.usage.cost_usd, 0.0);
        assert_eq!(outcome.models, Vec::<String>::new(), "no model name in the stream");
        assert_eq!(outcome.parse_errors, 0);
    }

    #[test]
    fn a_warning_item_is_not_an_error_but_the_top_level_ones_are() {
        let outcome = read(stream());
        assert_eq!(outcome.errors, ["top-level warning", "fatal"]);
    }

    #[test]
    fn a_step_shows_only_for_commands_and_file_changes() {
        let mut progress = Progress::new(crate::task::worker::Cli::Codex, None);
        let shown: Vec<_> = stream().iter().filter_map(|line| progress.feed(line)).collect();
        assert_eq!(shown, ["command_execution", "file_change"]);
        assert_eq!(progress.steps, 7);
        assert_eq!(progress.tools, ["command_execution", "file_change"]);
    }

    #[test]
    fn a_resumed_turn_counts_only_what_it_added() {
        let earlier = Usage {
            steps: 2,
            input: 100,
            output: 50,
            reasoning: 10,
            cache_read: 20,
            cache_write: 5,
            cost_usd: 0.0,
            largest_request: 105,
        };
        let reported = Usage {
            steps: 5,
            input: 160,
            output: 70,
            reasoning: 12,
            cache_read: 30,
            cache_write: 7,
            cost_usd: 0.0,
            largest_request: 167,
        };
        assert_eq!(
            since_earlier(&reported, &earlier),
            Usage {
                steps: 5,
                input: 60,
                output: 20,
                reasoning: 2,
                cache_read: 10,
                cache_write: 2,
                cost_usd: 0.0,
                largest_request: 62,
            }
        );

        let low = Usage { steps: 1, input: 10, output: 1, ..Default::default() };
        let kept = since_earlier(&low, &earlier);
        assert_eq!(kept.steps, 1, "a field below zero keeps what the turn reported");
        assert_eq!(kept.input, 10);
        assert_eq!(kept.output, 1);
        assert_eq!(kept.cache_read, 0);
        assert_eq!(kept.largest_request, 0);
    }
}
