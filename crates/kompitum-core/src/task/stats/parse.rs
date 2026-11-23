//! The pure parsers behind `kom task stats`.

use super::{AgentCall, Call, Parsed, Step};
use crate::task::types::cut;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

fn whole(value: Option<&Value>) -> u64 {
    value.and_then(Value::as_u64).unwrap_or(0)
}

fn text_at<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str).filter(|text| !text.is_empty())
}

fn object_at<'a>(value: &'a Value, key: &str) -> Option<&'a serde_json::Map<String, Value>> {
    value.get(key).and_then(Value::as_object)
}

fn part_text<'a>(part: &'a serde_json::Map<String, Value>, key: &str) -> Option<&'a str> {
    part.get(key).and_then(Value::as_str).filter(|text| !text.is_empty())
}

fn step_of(tokens: Option<&Value>) -> Step {
    let tokens = tokens.and_then(Value::as_object);
    let cache = tokens.and_then(|tokens| tokens.get("cache")).and_then(Value::as_object);
    let input = whole(tokens.and_then(|tokens| tokens.get("input")));
    let cache_read = whole(cache.and_then(|cache| cache.get("read")));
    let cache_write = whole(cache.and_then(|cache| cache.get("write")));
    Step {
        context: input.saturating_add(cache_read).saturating_add(cache_write),
        input,
        output: whole(tokens.and_then(|tokens| tokens.get("output"))),
        reasoning: whole(tokens.and_then(|tokens| tokens.get("reasoning"))),
        cache_read,
        cache_write,
    }
}

/// The brief's target rule for an OpenCode tool: filePath, then path, then command, then pattern.
fn target_of(input: Option<&serde_json::Map<String, Value>>) -> Option<String> {
    target_in(input, &["filePath", "path", "command", "pattern"])
}

/// Other CLIs spell a file target differently; the first line is what fits a table.
fn target_any(input: Option<&serde_json::Map<String, Value>>) -> Option<String> {
    target_in(input, &["filePath", "file_path", "path", "command", "pattern"])
}

fn target_in(input: Option<&serde_json::Map<String, Value>>, keys: &[&str]) -> Option<String> {
    let input = input?;
    let text = keys.iter().find_map(|key| input.get(*key).and_then(Value::as_str).filter(|text| !text.is_empty()))?;
    Some(cut(text, 90))
}

fn child_of(name: &str, state: Option<&serde_json::Map<String, Value>>) -> Option<String> {
    if name != "subagent" {
        return None;
    }
    let id = state
        .and_then(|state| state.get("metadata"))
        .and_then(Value::as_object)
        .and_then(|metadata| metadata.get("sessionID"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())?;
    Some(id.to_string())
}

fn push_child(out: &mut Parsed, id: String) {
    if !out.children.contains(&id) {
        out.children.push(id);
    }
}

fn note_parse_errors(out: &mut Parsed) {
    if out.parse_errors > 0 {
        out.note = Some(format!("{} unparseable lines", out.parse_errors));
    }
}

/// An OpenCode stream: one step-finish ends a step, and tool parts belong to
/// the step that is open when they first complete.
pub(crate) fn parse_opencode(lines: impl IntoIterator<Item = String>) -> Parsed {
    let mut out = Parsed::default();
    let mut seen = BTreeSet::new();
    let mut open = Vec::new();
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            out.parse_errors += 1;
            continue;
        };
        let Some(part) = object_at(&value, "part") else { continue };
        match part_text(part, "type") {
            Some("step-finish") => {
                out.steps.push(step_of(part.get("tokens")));
                out.calls.push(std::mem::take(&mut open));
            }
            Some("tool") => {
                let state = part.get("state").and_then(Value::as_object);
                if state.and_then(|state| state.get("status")).and_then(Value::as_str) != Some("completed") {
                    continue;
                }
                let id =
                    part.get("id").or_else(|| part.get("partID")).and_then(Value::as_str).filter(|id| !id.is_empty());
                if let Some(id) = id
                    && !seen.insert(id.to_string())
                {
                    continue;
                }
                let name = part_text(part, "tool").unwrap_or("tool");
                let input = state.and_then(|state| state.get("input")).and_then(Value::as_object);
                if let Some(child) = child_of(name, state) {
                    push_child(&mut out, child);
                }
                open.push(Call { name: name.into(), target: target_of(input) });
            }
            _ => {}
        }
    }
    note_parse_errors(&mut out);
    out
}

/// An OpenCode export: assistant messages with a `tokens` object are steps.
pub(crate) fn parse_export(text: &str) -> Result<Parsed, String> {
    let value: Value = serde_json::from_str(text).map_err(|e| format!("export was not JSON: {e}"))?;
    let messages =
        value.get("messages").and_then(Value::as_array).ok_or_else(|| "export has no messages".to_string())?;
    let mut out = Parsed::default();
    for message in messages {
        if text_at(message, "type") != Some("assistant") {
            continue;
        }
        let Some(tokens) = message.get("tokens").filter(|tokens| tokens.is_object()) else { continue };
        let step = step_of(Some(tokens));
        let mut calls = Vec::new();
        for part in message.get("content").and_then(Value::as_array).into_iter().flatten() {
            if text_at(part, "type") != Some("tool") {
                continue;
            }
            let Some(name) = text_at(part, "name") else { continue };
            let state = part.get("state").and_then(Value::as_object);
            let input = state.and_then(|state| state.get("input")).and_then(Value::as_object);
            if let Some(child) = child_of(name, state) {
                push_child(&mut out, child);
            }
            calls.push(Call { name: name.into(), target: target_of(input) });
        }
        out.steps.push(step);
        out.calls.push(calls);
    }
    Ok(out)
}

/// Claude Code's stream: every assistant message is one step with its prompt tokens.
pub(super) fn parse_claude(lines: impl IntoIterator<Item = String>) -> Parsed {
    let mut out = Parsed::default();
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            out.parse_errors += 1;
            continue;
        };
        if text_at(&value, "type") != Some("assistant") {
            continue;
        }
        let Some(message) = object_at(&value, "message") else { continue };
        let usage = message.get("usage").and_then(Value::as_object);
        let input = whole(usage.and_then(|usage| usage.get("input_tokens")));
        let cache_read = whole(usage.and_then(|usage| usage.get("cache_read_input_tokens")));
        let cache_write = whole(usage.and_then(|usage| usage.get("cache_creation_input_tokens")));
        let mut calls = Vec::new();
        for block in message.get("content").and_then(Value::as_array).into_iter().flatten() {
            if text_at(block, "type") != Some("tool_use") {
                continue;
            }
            let name = text_at(block, "name").unwrap_or("tool");
            let input = block.get("input").and_then(Value::as_object);
            calls.push(Call { name: name.into(), target: target_any(input) });
        }
        out.steps.push(Step {
            context: input.saturating_add(cache_read).saturating_add(cache_write),
            input,
            output: whole(usage.and_then(|usage| usage.get("output_tokens"))),
            reasoning: whole(
                usage
                    .and_then(|usage| usage.get("reasoning_output_tokens"))
                    .or_else(|| usage.and_then(|usage| usage.get("reasoning_tokens"))),
            ),
            cache_read,
            cache_write,
        });
        out.calls.push(calls);
    }
    note_parse_errors(&mut out);
    out
}

/// Command Code's stream: each finished model request is one step, and the
/// tools it queued belong to it.
pub(super) fn parse_commandcode(lines: impl IntoIterator<Item = String>) -> Parsed {
    let mut out = Parsed::default();
    let mut open = Vec::new();
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
            continue;
        }
        let event = outer.get("event").and_then(Value::as_object).unwrap_or(outer);
        match event.get("type").and_then(Value::as_str) {
            Some("model_request_end") => {
                let usage = event.get("usage").and_then(Value::as_object);
                let total = whole(usage.and_then(|usage| usage.get("inputTokens")));
                let cache_read = whole(usage.and_then(|usage| usage.get("cacheReadTokens")));
                let cache_write = whole(usage.and_then(|usage| usage.get("cacheWriteTokens")));
                let input = total.saturating_sub(cache_read.saturating_add(cache_write));
                out.steps.push(Step {
                    context: total,
                    input,
                    output: whole(usage.and_then(|usage| usage.get("outputTokens"))),
                    reasoning: whole(usage.and_then(|usage| usage.get("reasoningTokens"))),
                    cache_read,
                    cache_write,
                });
                out.calls.push(std::mem::take(&mut open));
            }
            Some("tool_queued") => {
                let name = event
                    .get("toolName")
                    .or_else(|| event.get("tool"))
                    .and_then(Value::as_str)
                    .filter(|name| !name.is_empty());
                let Some(name) = name else { continue };
                let input = event.get("input").and_then(Value::as_object);
                open.push(Call { name: name.into(), target: target_any(input) });
            }
            _ => {}
        }
    }
    note_parse_errors(&mut out);
    out
}

/// A Command Code transcript: each line with a `usage` object is one request,
/// and a `compaction` line marks the step its summary precedes.
pub(super) fn parse_commandcode_transcript(lines: impl IntoIterator<Item = String>) -> Parsed {
    let mut out = Parsed::default();
    let mut agents: BTreeMap<String, usize> = BTreeMap::new();
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            out.parse_errors += 1;
            continue;
        };
        if text_at(&value, "type") == Some("compaction") {
            out.marked.push(out.steps.len());
            continue;
        }
        let mut calls = Vec::new();
        for part in value
            .get("message")
            .and_then(|message| message.get("content"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            match text_at(part, "type") {
                Some("tool_use") => {
                    let Some(name) = text_at(part, "name") else { continue };
                    let input = part.get("input").and_then(Value::as_object);
                    calls.push(Call { name: name.into(), target: target_any(input) });
                    if name == "agent"
                        && let Some(id) = text_at(part, "id")
                    {
                        agents.insert(id.to_string(), out.agents.len());
                        out.agents.push(AgentCall { id: id.into(), candidates: Vec::new() });
                    }
                }
                Some("tool_result") => {
                    let Some(id) = text_at(part, "tool_use_id") else { continue };
                    let Some(&at) = agents.get(id) else { continue };
                    for block in part.get("content").and_then(Value::as_array).into_iter().flatten() {
                        if let Some(text) = text_at(block, "text") {
                            for candidate in candidates_in(text) {
                                if !out.agents[at].candidates.contains(&candidate) {
                                    out.agents[at].candidates.push(candidate);
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        let Some(usage) = value.get("usage").and_then(Value::as_object) else { continue };
        let input = whole(usage.get("inputTokens"));
        let cache_read = whole(usage.get("cacheReadTokens"));
        let cache_write = whole(usage.get("cacheWriteTokens"));
        out.steps.push(Step {
            context: input,
            input: input.saturating_sub(cache_read.saturating_add(cache_write)),
            output: whole(usage.get("outputTokens")),
            reasoning: whole(usage.get("reasoningTokens")),
            cache_read,
            cache_write,
        });
        out.calls.push(calls);
    }
    if let Some(last) = out.steps.len().checked_sub(1) {
        for at in &mut out.marked {
            if *at > last {
                *at = last;
            }
        }
    }
    note_parse_errors(&mut out);
    out
}

/// Transcript paths and session ids a tool result's text names.
fn candidates_in(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for token in text.split(|c: char| c.is_whitespace() || "\"'`()[]{}<>,;".contains(c)) {
        let token = token.trim_matches(|c: char| ".:".contains(c));
        if token.ends_with(".jsonl") || is_session_id(token) {
            out.push(token.to_string());
        }
    }
    out
}

pub(super) fn is_session_id(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(at, byte)| match at {
            8 | 13 | 18 | 23 => *byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
}

/// Command Code's project folder for a working folder: lowercased, with each
/// run of characters outside a-z and 0-9 turned into one dash.
pub fn project_folder(path: &str) -> String {
    let mut out = String::new();
    for c in path.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

pub(super) fn one_candidate(candidates: &mut Vec<PathBuf>) -> Option<PathBuf> {
    candidates.sort();
    candidates.dedup();
    if candidates.len() == 1 { candidates.pop() } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::stats::{KindStats, Numbers, Usage, numbers, recorded, table, totals};
    use serde_json::json;
    use std::path::PathBuf;

    fn kind<'a>(numbers: &'a Numbers, name: &str) -> &'a KindStats {
        numbers
            .kinds
            .iter()
            .find(|kind| kind.kind == name)
            .unwrap_or_else(|| panic!("no {name} in {:?}", numbers.kinds))
    }

    #[test]
    fn a_stream_counts_each_tool_once_and_finds_the_child() {
        let child = format!("ses_{}", "child");
        let id = format!("call_{}", "one");
        let tool = |status: &str| {
            json!({"part": {"id": id, "type": "tool", "tool": "read",
                "state": {"status": status, "input": {"filePath": "a.rs"}}}})
            .to_string()
        };
        let finish = |input: u64, read: u64| {
            json!({"part": {"type": "step-finish",
                "tokens": {"input": input, "output": 1, "reasoning": 0, "cache": {"read": read, "write": 0}}}})
            .to_string()
        };
        let subagent = json!({"part": {"id": format!("call_{}", "child"), "type": "tool", "tool": "subagent",
            "state": {"status": "completed", "input": {}, "metadata": {"sessionID": child}}}})
        .to_string();
        let lines =
            vec![tool("running"), tool("completed"), tool("completed"), finish(10, 5), subagent, finish(20, 10)];
        let parsed = parse_opencode(lines);
        assert_eq!(parsed.steps.len(), 2);
        assert_eq!(parsed.calls[0].len(), 1, "the repeated part is one tool");
        assert_eq!(parsed.calls[0][0].target.as_deref(), Some("a.rs"));
        assert_eq!(parsed.children, [child]);
    }

    #[test]
    fn opencode_tools_name_their_path() {
        let read = json!({"part": {"type": "tool", "tool": "read",
            "state": {"status": "completed", "input": {"path": "a.rs"}}}})
        .to_string();
        let finish = json!({"part": {"type": "step-finish", "tokens": {"input": 1}}}).to_string();
        let parsed = parse_opencode(vec![read, finish]);
        assert_eq!(parsed.calls[0][0].target.as_deref(), Some("a.rs"));
    }

    #[test]
    fn an_export_counts_assistant_messages_and_finds_the_child() {
        let child = format!("ses_{}", "child");
        let text = json!({
            "info": {"id": format!("ses_{}", "root")},
            "messages": [
                {"type": "user", "content": [{"type": "text", "text": "hello"}]},
                {"type": "assistant", "tokens": {"input": 100, "output": 5, "reasoning": 1,
                    "cache": {"read": 50, "write": 0}},
                 "content": [{"type": "tool", "name": "read",
                    "state": {"input": {"filePath": "a.rs"}}}]},
                {"type": "assistant", "tokens": {"input": 200, "output": 6, "reasoning": 0,
                    "cache": {"read": 100, "write": 0}},
                 "content": [{"type": "tool", "name": "subagent",
                    "state": {"input": {}, "metadata": {"sessionID": child}}}]},
                {"type": "compaction"}
            ]
        })
        .to_string();
        let parsed = parse_export(&text).unwrap();
        assert_eq!(parsed.steps.len(), 2);
        assert_eq!(parsed.calls[0].len(), 1);
        assert_eq!(parsed.calls[0][0].target.as_deref(), Some("a.rs"));
        assert_eq!(parsed.children, [child]);
    }

    #[test]
    fn claude_and_command_code_steps_carry_their_context() {
        let claude = vec![
            json!({"type": "assistant", "message": {"usage": {"input_tokens": 3,
                "cache_read_input_tokens": 900, "cache_creation_input_tokens": 100, "output_tokens": 20},
                "content": [{"type": "tool_use", "name": "Read", "input": {"file_path": "a.rs"}}]}})
            .to_string(),
            json!({"type": "assistant", "message": {"usage": {"input_tokens": 4,
                "cache_read_input_tokens": 1000, "cache_creation_input_tokens": 0, "output_tokens": 5},
                "content": []}})
            .to_string(),
        ];
        let parsed = parse_claude(claude);
        assert_eq!(parsed.steps.iter().map(|step| step.context).collect::<Vec<_>>(), [1003, 1004]);
        assert_eq!(parsed.calls[0][0].target.as_deref(), Some("a.rs"));

        let commandcode = vec![
            json!({"event": {"type": "tool_queued", "toolName": "read_file", "input": {"file_path": "b.rs"}}})
                .to_string(),
            json!({"event": {"type": "model_request_end", "usage": {"inputTokens": 1000,
                "cacheReadTokens": 900, "cacheWriteTokens": 50, "outputTokens": 7}}})
            .to_string(),
            json!({"event": {"type": "model_request_end", "usage": {"inputTokens": 2000,
                "cacheReadTokens": 0, "cacheWriteTokens": 0, "outputTokens": 8}}})
            .to_string(),
        ];
        let parsed = parse_commandcode(commandcode);
        assert_eq!(parsed.steps.iter().map(|step| step.context).collect::<Vec<_>>(), [1000, 2000]);
        assert_eq!((parsed.steps[0].input, parsed.steps[0].cache_read), (50, 900));
        assert_eq!(parsed.calls[0][0].target.as_deref(), Some("b.rs"));
        let usage = Usage { input: 50, cache_read: 900, cache_write: 50, ..Default::default() };
        assert_eq!(recorded(&usage).resent, 1000, "recorded totals add up like a step's context");
    }

    #[test]
    fn a_command_code_transcript_counts_requests_tools_and_a_marked_compaction() {
        let request = |context: u64, tools: Value| {
            json!({"type": "message", "usage": {"inputTokens": context, "outputTokens": 2,
                "cacheReadTokens": 0, "cacheWriteTokens": 0}, "message": {"content": tools}})
            .to_string()
        };
        let lines = vec![
            request(1000, json!([{"type": "tool_use", "name": "read_file", "input": {"file_path": "a.rs"}}])),
            request(1800, json!([])),
            request(1800, json!([])),
            json!({"type": "compaction"}).to_string(),
            "not json".into(),
        ];
        let parsed = parse_commandcode_transcript(lines);
        assert_eq!(parsed.steps.iter().map(|step| step.context).collect::<Vec<_>>(), [1000, 1800, 1800]);
        assert_eq!(parsed.calls[0][0].target.as_deref(), Some("a.rs"));
        assert_eq!(parsed.marked, [2]);
        assert_eq!(parsed.parse_errors, 1);
        let numbers = numbers(&parsed.steps, &parsed.calls, &parsed.marked);
        assert_eq!(numbers.steps, 3);
        assert_eq!(numbers.compactions, 1);
        assert_eq!(kind(&numbers, "read_file").growth, 800);
    }

    #[test]
    fn command_codes_project_folder_and_one_candidate_are_pure() {
        assert_eq!(project_folder("C:\\Users\\x\\Projects\\Ashley"), "c-users-x-projects-ashley");
        assert_eq!(project_folder("C:/Users/x/Projects/Ashley/"), "c-users-x-projects-ashley");
        let one = PathBuf::from("one.jsonl");
        assert_eq!(one_candidate(&mut vec![one.clone()]), Some(one));
        assert_eq!(one_candidate(&mut Vec::new()), None);
        assert_eq!(one_candidate(&mut vec![PathBuf::from("a"), PathBuf::from("b")]), None);
    }

    #[test]
    #[ignore]
    fn a_sample_stream_prints_its_table() {
        let Ok(path) = std::env::var("KOMPITUM_STATS_SAMPLE") else {
            eprintln!("KOMPITUM_STATS_SAMPLE is not set");
            return;
        };
        let text = std::fs::read_to_string(&path).unwrap();
        let parsed = parse_opencode(text.lines().map(str::to_string));
        let numbers = numbers(&parsed.steps, &parsed.calls, &[]);
        assert_eq!(numbers.steps, 401);
        assert_eq!(numbers.resent, 126_280_580);
        let sessions = vec![numbers.session("sample".into(), 0, None)];
        let totals = totals(std::slice::from_ref(&numbers));
        println!("{}", table("sample", &sessions, &totals, None, None));
    }

    #[test]
    #[ignore]
    fn a_sample_command_code_transcript_prints_its_table() {
        let Ok(path) = std::env::var("KOMPITUM_STATS_SAMPLE_CC") else {
            eprintln!("KOMPITUM_STATS_SAMPLE_CC is not set");
            return;
        };
        let text = std::fs::read_to_string(&path).unwrap();
        let parsed = parse_commandcode_transcript(text.lines().map(str::to_string));
        let numbers = numbers(&parsed.steps, &parsed.calls, &parsed.marked);
        let sessions = vec![numbers.session("sample".into(), 0, parsed.note)];
        let totals = totals(std::slice::from_ref(&numbers));
        println!("{}", table("sample", &sessions, &totals, None, None));
    }
}
