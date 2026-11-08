//! OpenCode's stream: one JSON object per line, a step-finish part carrying
//! the tokens and cost, and a text part carrying the report. Its own reader,
//! so `worker.rs` only has to know which CLI it is talking to.

use crate::task::types::Usage;
use crate::task::worker::{
    Progress, REASONING_TAIL, TurnOutcome, end_bytes, part_text, push_list, push_read, push_reasoning, push_tool,
    target_in, text_at, whole,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One step's numbers, from its `step-finish` part.
fn step_usage(part: &serde_json::Map<String, Value>) -> Usage {
    let tokens = part.get("tokens").and_then(Value::as_object);
    let cache = tokens.and_then(|t| t.get("cache")).and_then(Value::as_object);
    let input = whole(tokens.and_then(|t| t.get("input")));
    let cache_read = whole(cache.and_then(|c| c.get("read")));
    let cache_write = whole(cache.and_then(|c| c.get("write")));
    Usage {
        steps: 1,
        input,
        output: whole(tokens.and_then(|t| t.get("output"))),
        reasoning: whole(tokens.and_then(|t| t.get("reasoning"))),
        cache_read,
        cache_write,
        largest_request: input + cache_read + cache_write,
        cost_usd: part.get("cost").and_then(Value::as_f64).unwrap_or(0.0),
    }
}

/// `summarize_tool`: name and status, never the tool's input or output.
fn summarize_tool(event: &Value) -> Option<String> {
    let part = event.get("part").and_then(Value::as_object);
    let tool = part.and_then(|p| p.get("tool")).or_else(|| event.get("tool"))?;
    let (name, status) = match tool.as_object() {
        Some(tool) => {
            (tool.get("name").or_else(|| tool.get("tool")).and_then(Value::as_str), part_text(tool, "status"))
        }
        None => {
            let state = part.and_then(|p| p.get("state")).and_then(Value::as_object);
            (tool.as_str(), state.and_then(|s| part_text(s, "status")))
        }
    };
    let name = name?;
    let line = match status {
        Some(status) => format!("{name}:{status}"),
        None => name.to_string(),
    };
    Some(crate::task::types::cut(&line, 120))
}

fn tool_target(event: &Value, root: Option<&std::path::Path>) -> Option<String> {
    let input = event.get("part")?.get("state")?.get("input")?.as_object()?;
    target_in(input, root)
}

/// One output line in, one line for the screen out (or nothing to show).
pub(crate) fn progress(progress: &mut Progress, line: &str) -> Option<String> {
    let value: Value = serde_json::from_str(line.trim()).ok()?;
    let event = value.as_object()?;
    let part = event.get("part").and_then(Value::as_object);
    let part_type = part.and_then(|p| part_text(p, "type"));
    if part_type == Some("step-finish") {
        let step = step_usage(part?);
        progress.steps += 1;
        progress.peak = progress.peak.max(step.largest_request);
        progress.cost_usd += step.cost_usd;
        return Some(format!(
            "step {}: {} in, {} out, ${:.4}",
            progress.steps,
            step.input + step.cache_read,
            step.output + step.reasoning,
            progress.cost_usd
        ));
    }
    if text_at(&value, "type") == Some("tool_use") || part_type == Some("tool") {
        let part = part?;
        let id = part_text(part, "partID").or_else(|| part_text(part, "id"));
        if let Some(id) = id
            && !progress.seen.insert(id.to_string())
        {
            return None;
        }
        let tool = summarize_tool(&value)?;
        // "completed" on nearly every line says nothing to someone watching.
        let tool = tool.strip_suffix(":completed").unwrap_or(&tool);
        let shown = match tool_target(&value, progress.root.as_deref()) {
            Some(target) => crate::task::types::cut(&format!("{tool} {target}"), 120),
            None => tool.to_string(),
        };
        push_tool(progress, shown.clone());
        return Some(shown);
    }
    None
}

fn summarize_error(raw: &Value) -> String {
    let Some(object) = raw.as_object() else { return "error".into() };
    let data = object.get("data").and_then(Value::as_object);
    let name = object
        .get("name")
        .or_else(|| object.get("type"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("error");
    let code = data
        .and_then(|d| d.get("code"))
        .or_else(|| object.get("code"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    let message = data
        .and_then(|d| d.get("message"))
        .or_else(|| object.get("message"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    let mut out = name.to_string();
    if let Some(code) = code {
        out.push_str(&format!(" ({code})"));
    }
    if let Some(message) = message {
        out.push_str(&format!(": {message}"));
    }
    crate::task::types::cut(&out, 300)
}

/// Everything one OpenCode turn's stream says.
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
        let Some(event) = value.as_object() else {
            out.parse_errors += 1;
            continue;
        };
        let mut candidates = vec![event.get("sessionID"), event.get("session_id")];
        if let Some(part) = event.get("part") {
            candidates.push(part.get("sessionID"));
        }
        if let Some(properties) = event.get("properties") {
            candidates.push(properties.get("sessionID"));
        }
        for id in candidates.into_iter().flatten().filter_map(Value::as_str).filter(|s| !s.is_empty()) {
            if !out.sessions.iter().any(|seen| seen == id) {
                out.sessions.push(id.to_string());
            }
            if out.session.is_none() {
                out.session = Some(id.to_string());
            }
        }
        let kind = text_at(&value, "type").unwrap_or("?");
        let part = event.get("part").and_then(Value::as_object);
        if kind == "text"
            && let Some(text) = part.and_then(|p| p.get("text")).and_then(Value::as_str)
        {
            out.text = text.to_string();
        }
        if kind == "reasoning"
            && let Some(text) = part.and_then(|p| p.get("text")).and_then(Value::as_str)
        {
            push_reasoning(&mut out.reasoning_tail, text);
        }
        if kind == "tool_use"
            && part.and_then(|p| p.get("tool")).and_then(Value::as_str) == Some("read")
            && let Some(input) = part
                .and_then(|p| p.get("state"))
                .and_then(Value::as_object)
                .and_then(|state| state.get("input"))
                .and_then(Value::as_object)
            && let Some(path) = part_text(input, "path").or_else(|| part_text(input, "filePath"))
        {
            push_read(&mut out.files_read, path);
        }
        if let Some(part) = part.filter(|p| part_text(p, "type") == Some("step-finish")) {
            let step = step_usage(part);
            out.usage.steps += step.steps;
            out.usage.input += step.input;
            out.usage.output += step.output;
            out.usage.reasoning += step.reasoning;
            out.usage.cache_read += step.cache_read;
            out.usage.cache_write += step.cache_write;
            out.usage.largest_request = out.usage.largest_request.max(step.largest_request);
            out.usage.cost_usd += step.cost_usd;
        }
        if kind == "error" {
            let raw = event.get("error").cloned().unwrap_or_else(|| value.clone());
            out.errors.push(summarize_error(&raw));
            if out.raw_errors.len() < 5 {
                out.raw_errors.push(raw);
            }
        }
        if (kind == "tool_use" || part.and_then(|p| part_text(p, "type")) == Some("tool"))
            && let Some(tool) = summarize_tool(&value)
        {
            push_list(&mut out.tools, tool);
        }
    }
    out.reasoning_tail = end_bytes(&out.reasoning_tail, REASONING_TAIL);
    out
}

/// One route's shape in an export's sample, as the Python records it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RouteSample {
    #[serde(rename = "providerID")]
    pub provider_id: String,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
}

/// What a fresh export says about the route a turn was asked to use.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct RouteCheck {
    pub status: String,
    pub expected: String,
    pub message: Option<String>,
    pub sample: Vec<RouteSample>,
    pub attempts: u32,
}

impl RouteCheck {
    fn error(expected: &str, message: impl Into<String>) -> RouteCheck {
        RouteCheck {
            status: "error".into(),
            expected: expected.into(),
            message: Some(message.into()),
            sample: Vec::new(),
            attempts: 0,
        }
    }

    fn unverifiable(expected: &str, message: impl Into<String>) -> RouteCheck {
        RouteCheck { status: "unverifiable".into(), ..RouteCheck::error(expected, message) }
    }

    /// An export worth fetching again: it was cut off or incomplete.
    pub fn retryable(&self) -> bool {
        matches!(self.status.as_str(), "error" | "unverifiable")
    }
}

/// The provider, model and variant a route names.
fn split_route(route: &str) -> (String, String, String) {
    let (provider, rest) = route.split_once('/').unwrap_or(("", route));
    let (model, variant) = rest.split_once('#').unwrap_or((rest, ""));
    (provider.to_string(), model.to_string(), variant.to_string())
}

/// Judge one export against the route the turn asked for, as `verify_model` does.
/// A structurally incomplete export is `error` or `unverifiable` and may be
/// fetched again; a complete one that names another route is `mismatch`.
pub fn route_check(export_text: &str, route: &str) -> RouteCheck {
    let Ok(value) = serde_json::from_str::<Value>(export_text) else {
        return RouteCheck::error(route, "export was not JSON");
    };
    let Some(object) = value.as_object() else {
        return RouteCheck::error(route, "export was not a JSON object");
    };
    let Some(messages) = object.get("messages") else {
        return RouteCheck::unverifiable(route, "export messages were malformed");
    };
    let Some(messages) = messages.as_array() else {
        return RouteCheck::unverifiable(route, "export messages were malformed");
    };
    if messages.iter().any(|message| !message.is_object()) {
        return RouteCheck::unverifiable(route, "export messages were malformed");
    }
    let mut routes = Vec::new();
    for message in messages {
        let Some(model) = message.get("model").filter(|model| !model.is_null()) else { continue };
        let Some(model) = model.as_object() else {
            return RouteCheck::unverifiable(route, "export recorded an incomplete model route");
        };
        let field = |key: &str| model.get(key).and_then(Value::as_str).filter(|text| !text.trim().is_empty());
        let (Some(provider_id), Some(id)) = (field("providerID"), field("id")) else {
            return RouteCheck::unverifiable(route, "export recorded an incomplete model route");
        };
        routes.push(RouteSample {
            provider_id: provider_id.to_string(),
            id: id.to_string(),
            variant: model.get("variant").and_then(Value::as_str).map(String::from),
        });
    }
    let (provider, model, variant) = split_route(route);
    let differs = |seen: &RouteSample| {
        seen.id != model
            || seen.provider_id != provider
            || (!variant.is_empty() && seen.variant.as_deref() != Some(variant.as_str()))
    };
    let sample: Vec<RouteSample> = routes.iter().take(3).cloned().collect();
    if routes.is_empty() {
        return RouteCheck {
            status: "mismatch".into(),
            expected: route.into(),
            message: Some("export recorded no model route".into()),
            sample,
            attempts: 0,
        };
    }
    if let Some(seen) = routes.iter().find(|seen| differs(seen)) {
        let shown = match &seen.variant {
            Some(variant) if !variant.is_empty() => format!("{}/{}#{variant}", seen.provider_id, seen.id),
            _ => format!("{}/{}", seen.provider_id, seen.id),
        };
        return RouteCheck {
            status: "mismatch".into(),
            expected: route.into(),
            message: Some(format!("asked for {route}, the export says {shown}")),
            sample,
            attempts: 0,
        };
    }
    RouteCheck { status: "verified".into(), expected: route.into(), message: None, sample, attempts: 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SESSION: &str = "ses_5glfJCtnAmRo1vsz02KjZlOl0B";

    fn line(text: &str) -> String {
        text.to_string()
    }

    fn stream() -> Vec<String> {
        vec![
            line(&format!(
                r#"{{"type":"step_start","timestamp":1793798531666,"sessionID":"{SESSION}","part":{{"id":"prt_1","type":"step-start"}}}}"#
            )),
            line(&format!(
                r#"{{"type":"text","timestamp":1793798532015,"sessionID":"{SESSION}","part":{{"id":"prt_2","type":"text","text":"I'll start by reading the task brief."}}}}"#
            )),
            line(&format!(
                r#"{{"type":"tool_use","timestamp":1793798532046,"sessionID":"{SESSION}","part":{{"partID":"prt_3","type":"tool","id":"call_1","tool":"read","state":{{"status":"completed","input":{{"path":"src/lib.rs"}},"output":"Read file"}}}}}}"#
            )),
            line(&format!(
                r#"{{"type":"step_finish","timestamp":1793798532155,"sessionID":"{SESSION}","part":{{"id":"prt_4","type":"step-finish","cost":0.001723308,"tokens":{{"input":11142,"output":79,"reasoning":2,"cache":{{"read":1536,"write":8}}}}}}}}"#
            )),
            line(&format!(
                r#"{{"type":"text","timestamp":1793798532300,"sessionID":"{SESSION}","part":{{"id":"prt_5","type":"text","text":"All done."}}}}"#
            )),
        ]
    }

    #[test]
    fn a_real_stream_gives_its_session_text_usage_and_no_errors() {
        let outcome = read(stream());
        assert_eq!(outcome.session.as_deref(), Some(SESSION));
        assert_eq!(outcome.sessions, [SESSION]);
        assert_eq!(outcome.text, "All done.");
        assert_eq!(outcome.usage.steps, 1);
        assert_eq!(outcome.usage.input, 11142);
        assert_eq!(outcome.usage.output, 79);
        assert_eq!(outcome.usage.reasoning, 2);
        assert_eq!(outcome.usage.cache_read, 1536);
        assert_eq!(outcome.usage.cache_write, 8);
        assert_eq!(outcome.usage.largest_request, 11142 + 1536 + 8);
        assert!((outcome.usage.cost_usd - 0.001723308).abs() < 1e-12);
        assert!(outcome.errors.is_empty() && outcome.parse_errors == 0);
    }

    #[test]
    fn error_events_and_unparseable_lines_are_kept_as_their_own_facts() {
        let lines = vec![
            line(
                r#"{"type":"error","sessionID":"s1","error":{"name":"ProviderError","data":{"code":"429","message":"quota"}}}"#,
            ),
            line("not json"),
            line("[1,2]"),
            line(""),
        ];
        let outcome = read(lines);
        assert_eq!(outcome.errors, ["ProviderError (429): quota"]);
        assert_eq!(outcome.parse_errors, 2);
        assert_eq!(outcome.session.as_deref(), Some("s1"));
        assert_eq!(outcome.text, "");
        assert_eq!(outcome.raw_errors.len(), 1);
    }

    #[test]
    fn a_turn_remembers_what_it_read_once_each() {
        let read_tool = |path: &str, field: &str| {
            let mut input = serde_json::Map::new();
            input.insert(field.into(), Value::String(path.into()));
            serde_json::json!({"type": "tool_use", "part": {"type": "tool", "tool": "read",
                "state": {"status": "completed", "input": input}}})
            .to_string()
        };
        let lines = vec![
            read_tool("src/a.rs", "path"),
            read_tool("src/b.rs", "filePath"),
            read_tool("src/a.rs", "path"),
            serde_json::json!({"type": "tool_use", "part": {"type": "tool", "tool": "write",
                "state": {"status": "completed", "input": {"filePath": "made.txt"}}}})
            .to_string(),
        ];
        let outcome = read(lines);
        assert_eq!(outcome.files_read, ["src/a.rs", "src/b.rs"]);
    }

    #[test]
    fn a_turn_keeps_only_the_end_of_its_reasoning() {
        let head = "a".repeat(100);
        let crossing = format!("€{}", "b".repeat(8190));
        let reasoning = |text: &str| {
            serde_json::json!({"type": "reasoning", "part": {"type": "reasoning", "text": text}}).to_string()
        };
        let outcome = read(vec![reasoning(&head), reasoning(&crossing)]);
        assert_eq!(outcome.reasoning_tail, "b".repeat(8190));

        let kept = read(vec![reasoning(&"x".repeat(20 * 1024))]);
        assert_eq!(kept.reasoning_tail.len(), 8192);
    }

    #[test]
    fn progress_counts_steps_once_and_keeps_the_last_five_tools() {
        let mut progress = Progress::new(crate::task::worker::Cli::Opencode, None);
        let feed: Vec<_> = stream().iter().filter_map(|l| progress.feed(l)).collect();
        assert_eq!(progress.steps, 1);
        assert_eq!(progress.peak, 11142 + 1536 + 8);
        assert!((progress.cost_usd - 0.001723308).abs() < 1e-12);
        assert_eq!(feed, ["read src/lib.rs", "step 1: 12678 in, 81 out, $0.0017"]);
        // The same part twice is one tool.
        assert_eq!(progress.feed(&stream()[2]), None);
        for n in 0..6 {
            let tool = format!(
                r#"{{"type":"tool_use","part":{{"partID":"p{n}","type":"tool","tool":"bash","state":{{"status":"running"}}}}}}"#
            );
            assert_eq!(progress.feed(&tool).as_deref(), Some("bash:running"));
        }
        assert_eq!(progress.tools.len(), 5);
        assert_eq!(progress.tools[0], "bash:running");
    }

    #[test]
    fn a_tool_line_says_what_the_tool_works_on_relative_to_the_worktree() {
        let root = std::env::temp_dir().join("worktree");
        let inside = root.join("src").join("lib.rs").display().to_string();
        let mut progress = Progress::new(crate::task::worker::Cli::Opencode, Some(root));
        let mut feed = |tool: &str, status: &str, input: Value| {
            let id = format!("{tool}-{status}");
            let event = serde_json::json!({"type": "tool_use", "part": {
                "partID": id, "type": "tool", "tool": tool, "state": {"status": status, "input": input}}});
            progress.feed(&event.to_string())
        };
        let read = feed("read", "completed", serde_json::json!({"path": inside}));
        assert_eq!(read.as_deref(), Some("read src/lib.rs"));
        let bash = feed("bash", "running", serde_json::json!({"command": "cargo test\nnext line"}));
        assert_eq!(bash.as_deref(), Some("bash:running cargo test"));
        let outside = feed("grep", "error", serde_json::json!({"pattern": "x"}));
        assert_eq!(outside.as_deref(), Some("grep:error x"));
        let nothing = feed("list", "completed", serde_json::json!({}));
        assert_eq!(nothing.as_deref(), Some("list"));
    }

    fn export(messages: Value) -> String {
        serde_json::json!({"messages": messages}).to_string()
    }

    #[test]
    fn an_export_with_the_asked_route_is_verified() {
        let route = "opencode-go/deepseek-v4.1-flash#max";
        let text = export(serde_json::json!([
            {"role": "user", "content": [{"type": "text", "text": "hi"}]},
            {"role": "assistant", "model": {"providerID": "opencode-go", "id": "deepseek-v4.1-flash", "variant": "max"}}
        ]));
        let check = route_check(&text, route);
        assert_eq!(check.status, "verified", "{check:?}");
        assert_eq!(check.expected, route);
        assert!(!check.retryable());
        assert_eq!(check.sample.len(), 1);
    }

    #[test]
    fn an_export_with_another_route_or_none_is_a_mismatch() {
        let route = "opencode-go/deepseek-v4.1-flash#max";
        let other = export(serde_json::json!([
            {"model": {"providerID": "opencode", "id": "deepseek-v4.1-flash-free", "variant": "max"}}
        ]));
        let check = route_check(&other, route);
        assert_eq!(check.status, "mismatch");
        assert!(check.message.as_ref().unwrap().contains("opencode/deepseek-v4.1-flash-free"));
        assert!(!check.retryable());
        let none = route_check(&export(serde_json::json!([{"role": "user"}])), route);
        assert_eq!(none.status, "mismatch");
        let incomplete = export(serde_json::json!([{"model": {"providerID": "opencode-go"}}]));
        assert_eq!(route_check(&incomplete, route).status, "unverifiable");
        let wrong_variant = export(serde_json::json!([
            {"model": {"providerID": "opencode-go", "id": "deepseek-v4.1-flash", "variant": "high"}}
        ]));
        assert_eq!(route_check(&wrong_variant, route).status, "mismatch");
    }

    #[test]
    fn a_cut_off_export_asks_again_and_then_is_unverifiable_or_an_error() {
        let route = "opencode-go/deepseek-v4.1-flash#max";
        let cut = route_check("{\"messages\": [", route);
        assert_eq!(cut.status, "error");
        assert!(cut.retryable());
        assert!(route_check("[]", route).retryable(), "not an object");
        assert!(route_check("{\"messages\": \"x\"}", route).retryable(), "malformed messages");
        let complete = route_check(&export(serde_json::json!([])), route);
        assert_eq!(complete.status, "mismatch", "a complete export with no route does not gain one by retrying");
        assert!(!complete.retryable());
    }
}
