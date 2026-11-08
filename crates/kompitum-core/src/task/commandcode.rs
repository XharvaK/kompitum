//! Command Code's stream: every line is `{"type":"event","event":{…}}` or the
//! event itself, and the last is `{"type":"result",…}`. A real turn is mostly
//! `thinking_delta`, so only a tool, a denial and a finished request show.

use crate::task::worker::{
    Progress, REASONING_TAIL, Reported, TurnOutcome, end_bytes, part_text, push_list, push_read, push_reasoning,
    push_tool, target_in, text_at, whole,
};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Split a Command Code route into (model, effort): `commandcode/<model>#<effort>`.
pub fn route(route: &str) -> (String, String) {
    let rest = route.strip_prefix("commandcode/").unwrap_or(route);
    let (model, effort) = rest.split_once('#').unwrap_or((rest, ""));
    (model.to_string(), if effort.is_empty() { "max".to_string() } else { effort.to_string() })
}

fn event_of(outer: &serde_json::Map<String, Value>) -> &serde_json::Map<String, Value> {
    outer.get("event").and_then(Value::as_object).unwrap_or(outer)
}

fn session_id(event: &serde_json::Map<String, Value>) -> Option<&str> {
    event.get("sessionId").and_then(Value::as_str).filter(|id| !id.is_empty())
}

fn add_session(out: &mut TurnOutcome, id: &str) {
    if !out.sessions.iter().any(|seen| seen == id) {
        out.sessions.push(id.to_string());
    }
    if out.session.is_none() {
        out.session = Some(id.to_string());
    }
}

fn tool_name(event: &serde_json::Map<String, Value>) -> Option<&str> {
    event.get("toolName").or_else(|| event.get("tool")).and_then(Value::as_str).filter(|name| !name.is_empty())
}

/// Everything one Command Code turn's stream says.
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
        let Some(outer) = value.as_object() else {
            out.parse_errors += 1;
            continue;
        };
        if text_at(&value, "type") == Some("result") {
            let field = |key: &str| outer.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
            out.reported =
                Some(Reported { subtype: field("subtype"), error: field("error"), final_text: field("finalText") });
            if let Some(id) = session_id(outer) {
                add_session(&mut out, id);
            }
            continue;
        }
        let event = event_of(outer);
        match event.get("type").and_then(Value::as_str).unwrap_or("?") {
            "run_start" => {
                if let Some(id) = session_id(event) {
                    add_session(&mut out, id);
                }
            }
            "model_request_start" => {
                if let Some(model) = event.get("model").and_then(Value::as_str).filter(|model| !model.is_empty())
                    && !out.models.iter().any(|seen| seen == model)
                {
                    out.models.push(model.to_string());
                }
            }
            "model_request_end" => {
                // turn_end, run_end and result repeat these numbers: only a finished
                // request counts them.
                let usage = event.get("usage").and_then(Value::as_object);
                let input = whole(usage.and_then(|u| u.get("inputTokens")));
                let cache_read = whole(usage.and_then(|u| u.get("cacheReadTokens")));
                let cache_write = whole(usage.and_then(|u| u.get("cacheWriteTokens")));
                out.usage.steps += 1;
                out.usage.input += input.saturating_sub(cache_read.saturating_add(cache_write));
                out.usage.output += whole(usage.and_then(|u| u.get("outputTokens")));
                out.usage.cache_read += cache_read;
                out.usage.cache_write += cache_write;
                out.usage.largest_request = out.usage.largest_request.max(input);
            }
            "thinking_end" => {
                if let Some(text) = event.get("text").and_then(Value::as_str) {
                    push_reasoning(&mut out.reasoning_tail, text);
                }
            }
            "tool_queued" => {
                if tool_name(event) == Some("read_file")
                    && let Some(path) =
                        event.get("input").and_then(Value::as_object).and_then(|input| part_text(input, "file_path"))
                {
                    push_read(&mut out.files_read, path);
                }
                if let Some(shown) = tool_line(event, None) {
                    push_list(&mut out.tools, shown);
                }
            }
            "tool_denied" => {
                if let Some(name) = tool_name(event) {
                    push_list(&mut out.tools, format!("{name} denied"));
                }
            }
            _ => {}
        }
    }
    if let Some(reported) = &out.reported {
        out.text = reported.final_text.clone();
    }
    out.reasoning_tail = end_bytes(&out.reasoning_tail, REASONING_TAIL);
    out
}

fn tool_line(event: &serde_json::Map<String, Value>, root: Option<&Path>) -> Option<String> {
    let name = tool_name(event)?;
    let shown = match event.get("input").and_then(Value::as_object).and_then(|input| target_in(input, root)) {
        Some(target) => crate::task::types::cut(&format!("{name} {target}"), 120),
        None => crate::task::types::cut(name, 120),
    };
    Some(shown)
}

/// One output line in, one line for the screen out (or nothing to show).
pub(crate) fn progress(progress: &mut Progress, line: &str) -> Option<String> {
    let value: Value = serde_json::from_str(line.trim()).ok()?;
    let outer = value.as_object()?;
    if text_at(&value, "type") == Some("result") {
        return None;
    }
    let event = event_of(outer);
    match event.get("type").and_then(Value::as_str)? {
        "tool_queued" => {
            let shown = tool_line(event, progress.root.as_deref())?;
            push_tool(progress, shown.clone());
            Some(shown)
        }
        "tool_denied" => {
            let name = tool_name(event)?;
            let shown = crate::task::types::cut(&format!("{name} denied"), 120);
            push_tool(progress, shown.clone());
            Some(shown)
        }
        "model_request_end" => {
            let usage = event.get("usage").and_then(Value::as_object);
            let input = whole(usage.and_then(|u| u.get("inputTokens")));
            let output = whole(usage.and_then(|u| u.get("outputTokens")));
            progress.steps += 1;
            progress.peak = progress.peak.max(input);
            Some(format!("step {}: {input} in, {output} out", progress.steps))
        }
        _ => None,
    }
}

/// The folder Command Code keeps its transcripts in.
fn projects() -> PathBuf {
    match std::env::var_os("KOMPITUM_CC_PROJECTS") {
        Some(root) => {
            let path = PathBuf::from(root);
            match path.to_str().and_then(|text| text.strip_prefix("~/")) {
                Some(rest) => crate::platform::home_dir().join(rest),
                None => path,
            }
        }
        None => crate::platform::home_dir().join(".commandcode").join("projects"),
    }
}

/// Sum `usage.costUsd` over Command Code's transcript for one session; None
/// when no file exists.
pub fn session_cost(session: &str) -> Option<f64> {
    // The id comes from the worker's stream: it names a file here, never a path.
    if session.contains(['/', '\\']) || session.contains("..") {
        return None;
    }
    let root = projects();
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&root).ok()?.flatten() {
        let project = entry.path();
        let candidate = project.join(format!("{session}.jsonl"));
        if project.is_dir() && candidate.is_file() {
            files.push(candidate);
        }
    }
    if files.is_empty() {
        return None;
    }
    let mut total = 0.0;
    for path in files {
        let Ok(bytes) = std::fs::read(&path) else { continue };
        for line in String::from_utf8_lossy(&bytes).lines() {
            if !line.contains("costUsd") {
                continue;
            }
            if let Ok(value) = serde_json::from_str::<Value>(line)
                && let Some(cost) = value.get("usage").and_then(|u| u.get("costUsd")).and_then(Value::as_f64)
            {
                total += cost;
            }
        }
    }
    Some((total * 1_000_000.0).round() / 1_000_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::worker::Worker;

    const SESSION: &str = "11142727-ca4c-4fe9-876a-3c0190ce95b7";
    const MODEL: &str = "deepseek/deepseek-v4.1-flash";

    fn event(value: Value) -> String {
        serde_json::json!({"type": "event", "event": value}).to_string()
    }

    /// A turn in the shape of a real one: two requests, a tool, then the result.
    fn stream() -> Vec<String> {
        vec![
            event(serde_json::json!({"type": "run_start", "sessionId": SESSION})),
            event(serde_json::json!({"type": "turn_start", "turnNumber": 1})),
            event(serde_json::json!({"type": "message_start"})),
            event(serde_json::json!({"type": "model_request_start", "model": MODEL})),
            event(serde_json::json!({"type": "thinking_delta", "delta": "thinking"})),
            event(serde_json::json!({"type": "tool_queued", "toolCallId": "call_1", "toolName": "write_file",
                "input": {"file_path": "made.txt", "content": "x"}})),
            event(serde_json::json!({"type": "tool_running", "toolCallId": "call_1", "toolName": "write_file"})),
            event(serde_json::json!({"type": "tool_completed", "toolCallId": "call_1", "toolName": "write_file"})),
            event(serde_json::json!({"type": "model_request_end", "model": MODEL,
                "usage": {"inputTokens": 17639, "outputTokens": 88, "cacheReadTokens": 8192, "cacheWriteTokens": 0},
                "stopReason": "tool_calls", "effort": "max"})),
            event(serde_json::json!({"type": "turn_end", "turnNumber": 1, "hadToolCalls": true,
                "usage": {"inputTokens": 17639, "outputTokens": 88, "cacheReadTokens": 8192, "cacheWriteTokens": 0}})),
            event(serde_json::json!({"type": "turn_start", "turnNumber": 2})),
            event(serde_json::json!({"type": "model_request_start", "model": MODEL})),
            event(serde_json::json!({"type": "text_delta", "delta": "D"})),
            event(serde_json::json!({"type": "text_delta", "delta": "ONE"})),
            event(serde_json::json!({"type": "model_request_end", "model": MODEL,
                "usage": {"inputTokens": 17775, "outputTokens": 3, "cacheReadTokens": 17536, "cacheWriteTokens": 0},
                "stopReason": "stop", "effort": "max"})),
            event(serde_json::json!({"type": "turn_end", "turnNumber": 2, "hadToolCalls": false,
                "usage": {"inputTokens": 17775, "outputTokens": 3, "cacheReadTokens": 17536, "cacheWriteTokens": 0}})),
            event(serde_json::json!({"type": "run_end", "result": {"finalText": "DONE", "stopReason": "end_turn",
                "turnCount": 2, "usage": {"inputTokens": 35414, "outputTokens": 91, "cacheReadTokens": 25728,
                "cacheWriteTokens": 0}}})),
            serde_json::json!({"type": "result", "subtype": "success", "sessionId": SESSION,
                "stopReason": "end_turn", "usage": {"inputTokens": 35414, "outputTokens": 91,
                "cacheReadTokens": 25728, "cacheWriteTokens": 0}, "durationMs": 4845, "finalText": "DONE"})
            .to_string(),
        ]
    }

    #[test]
    fn a_command_code_turn_counts_each_request_once() {
        let outcome = read(stream());
        assert_eq!(outcome.session.as_deref(), Some(SESSION));
        assert_eq!(outcome.sessions, [SESSION]);
        assert_eq!(outcome.text, "DONE");
        assert_eq!(outcome.usage.steps, 2, "turn_end, run_end and result must not count again");
        assert_eq!(outcome.usage.input, (17639 - 8192) + (17775 - 17536));
        assert_eq!(outcome.usage.output, 88 + 3);
        assert_eq!(outcome.usage.cache_read, 8192 + 17536);
        assert_eq!(outcome.usage.cache_write, 0);
        assert_eq!(outcome.usage.reasoning, 0);
        assert_eq!(outcome.usage.largest_request, 17775);
        assert_eq!(outcome.usage.cost_usd, 0.0, "the stream has no cost");
        assert_eq!(outcome.models, [MODEL]);
        assert_eq!(outcome.reported.as_ref().unwrap().subtype, "success");
        assert_eq!(outcome.reported.as_ref().unwrap().final_text, "DONE");
        assert_eq!(outcome.tools, ["write_file made.txt"]);
        assert_eq!(outcome.parse_errors, 0);
    }

    #[test]
    fn a_thinking_stream_shows_no_lines_and_a_tool_shows_its_target() {
        let mut progress = Progress::new(Worker::Cc, None);
        let shown: Vec<_> = stream().iter().filter_map(|line| progress.feed(line)).collect();
        assert_eq!(shown, ["write_file made.txt", "step 1: 17639 in, 88 out", "step 2: 17775 in, 3 out"]);
        assert_eq!(progress.steps, 2);
        assert_eq!(progress.peak, 17775);
        assert_eq!(progress.cost_usd, 0.0);
        let denied = event(serde_json::json!({"type": "tool_denied", "toolName": "read_file"}));
        assert_eq!(progress.feed(&denied).as_deref(), Some("read_file denied"));
        assert_eq!(progress.tools.last().map(String::as_str), Some("read_file denied"));
        let thinking = event(serde_json::json!({"type": "thinking_delta", "delta": "x".repeat(1000)}));
        assert_eq!(progress.feed(&thinking), None);
    }

    #[test]
    fn a_session_id_never_names_a_path() {
        for id in ["../secret", "a/b", "a\\b", ".."] {
            assert_eq!(session_cost(id), None, "{id}");
        }
    }

    #[test]
    fn a_turn_remembers_what_it_read_once_each() {
        let lines = vec![
            event(serde_json::json!({"type": "tool_queued", "toolName": "read_file",
                "input": {"file_path": "src/a.rs"}})),
            event(serde_json::json!({"type": "tool_queued", "toolName": "read_file",
                "input": {"file_path": "src/b.rs"}})),
            event(serde_json::json!({"type": "tool_queued", "toolName": "read_file",
                "input": {"file_path": "src/a.rs"}})),
            event(serde_json::json!({"type": "tool_queued", "toolName": "write_file",
                "input": {"file_path": "made.txt", "content": "x"}})),
        ];
        assert_eq!(read(lines).files_read, ["src/a.rs", "src/b.rs"]);
    }

    #[test]
    fn a_turn_keeps_only_the_end_of_its_reasoning() {
        // 100 + 3 + 8190 bytes: the 8 KB cut lands inside the three-byte character.
        let head = "a".repeat(100);
        let crossing = format!("€{}", "b".repeat(8190));
        let outcome = read(vec![
            event(serde_json::json!({"type": "thinking_end", "text": head})),
            event(serde_json::json!({"type": "thinking_end", "text": crossing})),
        ]);
        assert_eq!(outcome.reasoning_tail, "b".repeat(8190));

        let long = event(serde_json::json!({"type": "thinking_end", "text": "x".repeat(20 * 1024)}));
        let kept = read(vec![long]);
        assert_eq!(kept.reasoning_tail.len(), 8192);
    }

    #[test]
    fn a_denied_tool_and_unparseable_lines_are_kept() {
        let lines = vec![
            event(serde_json::json!({"type": "tool_denied", "toolName": "read_file"})),
            "not json".to_string(),
            "[1]".to_string(),
        ];
        let outcome = read(lines);
        assert_eq!(outcome.parse_errors, 2);
        assert_eq!(outcome.tools, ["read_file denied"]);
    }
}
