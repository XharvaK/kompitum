//! What the orchestrator itself spent on one task, read from its transcript.

use crate::task::types::ParentUsage;
use serde_json::Value;
use std::collections::HashSet;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// A transcript older than the task by more than this is not opened: a resumed task never needs it.
const MTIME_SLACK_MS: u64 = 60_000;
/// The tools the Python wrote, before the CLI had names for tasks.
const RUN_TOOLS: &[&str] = &["cw_run"];
const CLOSE_TOOLS: &[&str] = &["cw_merge", "cw_discard"];
/// A Bash or PowerShell tool names the task in its command today.
const COMMAND_TOOLS: &[&str] = &["Bash", "PowerShell"];

/// The transcripts under `config_dir` that hold this task's start, newest first.
pub fn find_transcript(task: &str, since_ms: u64, config_dir: &Path) -> Option<PathBuf> {
    let folder = config_dir.join("projects");
    let mut files: Vec<(u64, PathBuf)> = Vec::new();
    for project in std::fs::read_dir(&folder).ok()?.flatten() {
        let project = project.path();
        if !project.is_dir() {
            continue;
        }
        for entry in std::fs::read_dir(&project).into_iter().flatten().flatten() {
            let path = entry.path();
            if !path.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
                continue;
            }
            let modified = modified_ms(&path);
            if modified.saturating_add(MTIME_SLACK_MS) < since_ms {
                continue;
            }
            files.push((modified, path));
        }
    }
    files.sort_by_key(|b| std::cmp::Reverse(b.0));
    files.into_iter().map(|(_, path)| path).find(|path| holds_start(path, task))
}

/// The parent window for one task: requests and token sums from its start to its close.
pub fn usage(path: &Path, task: &str) -> Option<ParentUsage> {
    let mut result = ParentUsage { transcript: path.display().to_string(), ..Default::default() };
    let mut seen = HashSet::new();
    let mut started = false;
    for_each_line(path, |line| {
        // The cheap filter the Python uses, so a large transcript is mostly not parsed.
        if !started && !line.contains(task) {
            return true;
        }
        let Some(data) = parse(line) else { return true };
        let mut closing = false;
        if !started {
            if !(is_assistant(&data) && has_start(&data, task)) {
                return true;
            }
            started = true;
        } else {
            closing = is_assistant(&data) && has_close(&data, task);
        }
        sum_message(&mut result, &data, &mut seen);
        if closing {
            result.closed = true;
            return false;
        }
        true
    })?;
    Some(result)
}

fn holds_start(path: &Path, task: &str) -> bool {
    let mut found = false;
    for_each_line(path, |line| {
        if !line.contains(task) {
            return true;
        }
        let Some(data) = parse(line) else { return true };
        if is_assistant(&data) && has_start(&data, task) {
            found = true;
            return false;
        }
        true
    });
    found
}

/// Calls `visit` for each line, read a chunk at a time as the Python streams them; `visit`
/// returns false to stop early. None when the file cannot be opened or read.
fn for_each_line(path: &Path, mut visit: impl FnMut(&str) -> bool) -> Option<()> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader = std::io::BufReader::new(file);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) => return Some(()),
            Ok(_) => {
                if !visit(&String::from_utf8_lossy(&buf)) {
                    return Some(());
                }
            }
            Err(_) => return None,
        }
    }
}

fn modified_ms(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |age| age.as_millis() as u64)
}

fn parse(line: &str) -> Option<Value> {
    serde_json::from_str::<Value>(line).ok().filter(Value::is_object)
}

fn message(data: &Value) -> Option<&serde_json::Map<String, Value>> {
    data.get("message").and_then(Value::as_object)
}

fn is_assistant(data: &Value) -> bool {
    match data.get("type") {
        Some(Value::String(kind)) => kind == "assistant",
        Some(Value::Null) | None => {
            message(data).and_then(|message| message.get("role")).and_then(Value::as_str) == Some("assistant")
        }
        _ => false,
    }
}

fn tool_uses(data: &Value) -> Vec<&serde_json::Map<String, Value>> {
    message(data)
        .and_then(|message| message.get("content"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_use"))
        .collect()
}

fn has_named_tool(data: &Value, names: &[&str], task: &str) -> bool {
    tool_uses(data).into_iter().any(|block| {
        let Some(name) = block.get("name").and_then(Value::as_str) else { return false };
        if !names.iter().any(|wanted| name.ends_with(wanted)) {
            return false;
        }
        block.get("input").and_then(Value::as_object).and_then(|input| input.get("task")).and_then(Value::as_str)
            == Some(task)
    })
}

fn has_command_tool(data: &Value, subcommand: &str, task: &str) -> bool {
    tool_uses(data).into_iter().any(|block| {
        let name = block.get("name").and_then(Value::as_str).unwrap_or_default();
        if !COMMAND_TOOLS.contains(&name) {
            return false;
        }
        let command = block
            .get("input")
            .and_then(Value::as_object)
            .and_then(|input| input.get("command"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        command_has_task(command, subcommand, task)
    })
}

fn has_start(data: &Value, task: &str) -> bool {
    has_named_tool(data, RUN_TOOLS, task) || has_command_tool(data, "start", task)
}

fn has_close(data: &Value, task: &str) -> bool {
    has_named_tool(data, CLOSE_TOOLS, task)
        || has_command_tool(data, "accept", task)
        || has_command_tool(data, "discard", task)
}

/// `kom task <subcommand> <task>` in a command, with the name exact: `fix-1` is not `fix-10`.
fn command_has_task(command: &str, subcommand: &str, task: &str) -> bool {
    let needle = format!("kom task {subcommand} ");
    let mut rest = command;
    while let Some(at) = rest.find(&needle) {
        let boundary = rest[..at].chars().next_back().is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
        if boundary {
            let after = &rest[at + needle.len()..];
            let after = after.strip_prefix('"').or_else(|| after.strip_prefix('\'')).unwrap_or(after);
            let end = after
                .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
                .unwrap_or(after.len());
            if &after[..end] == task {
                return true;
            }
        }
        rest = &rest[at + needle.len()..];
    }
    false
}

fn sum_message(result: &mut ParentUsage, data: &Value, seen: &mut HashSet<String>) {
    if !is_assistant(data) {
        return;
    }
    let message = message(data);
    if let Some(id) = message.and_then(|message| message.get("id")).and_then(Value::as_str).filter(|id| !id.is_empty())
        && !seen.insert(id.to_string())
    {
        return;
    }
    let usage = message.and_then(|message| message.get("usage")).and_then(Value::as_object);
    let fresh = int_field(usage, "input_tokens");
    let cache_write = int_field(usage, "cache_creation_input_tokens");
    let cache_read = int_field(usage, "cache_read_input_tokens");
    let output = int_field(usage, "output_tokens");
    result.requests = result.requests.saturating_add(1);
    result.fresh = result.fresh.saturating_add(fresh);
    result.cache_write = result.cache_write.saturating_add(cache_write);
    result.cache_read = result.cache_read.saturating_add(cache_read);
    result.output = result.output.saturating_add(output);
    let request = fresh.saturating_add(cache_write).saturating_add(cache_read);
    result.peak = result.peak.max(request);
}

/// `int(value or 0)`: a number, or a string that names one; anything else counts zero.
fn int_field(usage: Option<&serde_json::Map<String, Value>>, key: &str) -> u64 {
    let Some(value) = usage.and_then(|usage| usage.get(key)) else { return 0 };
    match value {
        Value::Number(number) => number
            .as_u64()
            .or_else(|| number.as_i64().filter(|value| *value >= 0).map(|value| value as u64))
            .or_else(|| number.as_f64().filter(|value| *value >= 0.0).map(|value| value.trunc() as u64))
            .unwrap_or(0),
        Value::String(text) => text
            .trim()
            .parse::<u64>()
            .or_else(|_| text.trim().parse::<f64>().map(|value| value.trunc().max(0.0) as u64))
            .unwrap_or(0),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kom-parent-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn transcript(dir: &Path, name: &str, lines: &[Value]) -> PathBuf {
        let folder = dir.join("projects").join("repo");
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join(name);
        let text: String = lines.iter().map(|line| format!("{line}\n")).collect();
        std::fs::write(&path, text).unwrap();
        path
    }

    fn assistant(id: &str, content: Value, usage: Value) -> Value {
        json!({"type": "assistant", "message": {"id": id, "role": "assistant", "content": content, "usage": usage}})
    }

    fn command(name: &str, command: &str) -> Value {
        json!({"type": "tool_use", "name": name, "input": {"command": command}})
    }

    #[test]
    fn the_python_tools_open_and_close_the_window() {
        let dir = scratch("python-tools");
        let path = transcript(
            &dir,
            "parent.jsonl",
            &[
                json!({"type": "user", "message": {"role": "user", "content": "fix-1 please"}}),
                assistant(
                    "m1",
                    json!([{"type": "tool_use", "name": "mcp__kompitum__cw_run", "input": {"task": "fix-1"}}]),
                    json!({"input_tokens": 10, "cache_creation_input_tokens": 5, "cache_read_input_tokens": 100, "output_tokens": 20}),
                ),
                assistant("m1", json!([]), json!({"input_tokens": 999, "output_tokens": 999})),
                json!({"type": "user", "message": {"role": "user", "content": "go on"}}),
                assistant(
                    "m2",
                    json!([{"type": "tool_use", "name": "mcp__kompitum__cw_merge", "input": {"task": "fix-1"}}]),
                    json!({"input_tokens": 1, "cache_read_input_tokens": 200, "output_tokens": 30}),
                ),
                assistant("m3", json!([]), json!({"input_tokens": 1000, "output_tokens": 1000})),
            ],
        );
        let got = usage(&path, "fix-1").unwrap();
        assert_eq!(got.requests, 2, "a duplicate message id counts once, and the close is the last");
        assert_eq!((got.fresh, got.cache_write, got.cache_read, got.output), (11, 5, 300, 50));
        assert_eq!(got.peak, 201);
        assert!(got.closed);
        assert!(find_transcript("fix-1", 0, &dir).is_some());

        let discarded = transcript(
            &dir,
            "discarded.jsonl",
            &[
                assistant(
                    "d1",
                    json!([{"type": "tool_use", "name": "mcp__kompitum__cw_run", "input": {"task": "fix-1"}}]),
                    json!({"input_tokens": 4}),
                ),
                assistant(
                    "d2",
                    json!([{"type": "tool_use", "name": "mcp__kompitum__cw_discard", "input": {"task": "fix-1"}}]),
                    json!({"output_tokens": 6}),
                ),
            ],
        );
        let got = usage(&discarded, "fix-1").unwrap();
        assert_eq!(got.requests, 2);
        assert_eq!((got.fresh, got.output), (4, 6));
        assert!(got.closed, "cw_discard closes the window");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn kom_task_start_and_accept_open_and_close_the_window() {
        let dir = scratch("command-tools");
        let path = transcript(
            &dir,
            "parent.jsonl",
            &[
                assistant(
                    "m1",
                    json!([command("Bash", "cd repo && kom task start fix-1 --brief b.md")]),
                    json!({"input_tokens": 7}),
                ),
                assistant("m2", json!([command("PowerShell", "kom task accept fix-1")]), json!({"output_tokens": 3})),
            ],
        );
        let got = usage(&path, "fix-1").unwrap();
        assert_eq!(got.requests, 2);
        assert_eq!((got.fresh, got.output), (7, 3));
        assert!(got.closed);

        let discarded = transcript(
            &dir,
            "discarded.jsonl",
            &[
                assistant(
                    "d1",
                    json!([command("Bash", "kom task start fix-1 --brief b.md")]),
                    json!({"input_tokens": 8}),
                ),
                assistant("d2", json!([command("PowerShell", "kom task discard fix-1")]), json!({"output_tokens": 9})),
            ],
        );
        let got = usage(&discarded, "fix-1").unwrap();
        assert_eq!(got.requests, 2);
        assert_eq!((got.fresh, got.output), (8, 9));
        assert!(got.closed, "kom task discard closes the window");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn fix_10_is_not_fix_1_and_an_unclosed_window_says_so() {
        let dir = scratch("exact-name");
        let path = transcript(
            &dir,
            "parent.jsonl",
            &[
                assistant("m1", json!([command("Bash", "kom task start fix-10")]), json!({"input_tokens": 5})),
                json!({"type": "user", "message": {"role": "user", "content": "a damaged line follows"}}),
                json!({"type": "assistant", "message": {"id": "m2", "content": []}}),
                assistant("m3", json!([]), json!({"input_tokens": 9})),
            ],
        );
        assert!(find_transcript("fix-1", 0, &dir).is_none(), "fix-10 is not fix-1");
        assert!(find_transcript("fix-10", 0, &dir).is_some());
        let got = usage(&path, "fix-1").unwrap();
        assert_eq!(got.requests, 0, "no window opens for another task");
        assert!(!got.closed);

        // The same file for its own task: the damaged line is skipped and the window stays open.
        let text = std::fs::read_to_string(&path).unwrap();
        let broken = format!("{text}not json at all\n");
        std::fs::write(&path, broken).unwrap();
        let got = usage(&path, "fix-10").unwrap();
        assert_eq!(got.requests, 3, "the damaged line adds nothing");
        assert_eq!(got.fresh, 14);
        assert!(!got.closed, "nothing closed it");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_transcript_older_than_the_task_is_not_searched() {
        let dir = scratch("since");
        let path = transcript(
            &dir,
            "parent.jsonl",
            &[assistant("m1", json!([command("Bash", "kom task start fix-1")]), json!({"input_tokens": 5}))],
        );
        let modified = modified_ms(&path);
        assert!(find_transcript("fix-1", 0, &dir).is_some(), "the file is within the window");
        let later = modified.saturating_add(MTIME_SLACK_MS).saturating_add(1);
        assert!(find_transcript("fix-1", later, &dir).is_none(), "past the slack, the file is not opened");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_unreadable_transcript_is_no_usage() {
        let dir = scratch("missing");
        assert_eq!(usage(&dir.join("gone.jsonl"), "fix-1"), None);
        assert_eq!(find_transcript("fix-1", 0, &dir), None);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
